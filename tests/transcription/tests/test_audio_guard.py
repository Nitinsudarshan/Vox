"""The checks that would have caught corpus-v1 before it was committed.

Run with `python3 -m unittest discover -s tests/transcription/tests` from the
repository root.
"""

import math
import sys
import unittest
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(REPO_ROOT / "tests" / "transcription" / "generator"))

from audio_guard import (  # noqa: E402
    MAX_NEAR_CLIPPING_PERCENT,
    assert_usable_audio,
    audio_concerns,
    to_float_samples,
)


class FakeDecoded:
    """Stands in for `miniaudio.DecodedSoundFile`, which is not installed here."""

    class _Fmt:
        def __init__(self, name):
            self.name = name

    def __init__(self, name, samples):
        self.sample_format = self._Fmt(name)
        self.samples = samples


def sine(seconds=1.0, rate=16000, amplitude=0.35, freq=220.0):
    n = int(seconds * rate)
    return [amplitude * math.sin(i * freq * 2 * math.pi / rate) for i in range(n)]


class ToFloatSamples(unittest.TestCase):
    """The exact defect: integer PCM read as if it were already normalized."""

    def test_signed16_is_scaled_not_clamped(self):
        # These are ordinary speech-level 16-bit samples. Before the fix, each
        # one was clamped to +/-1.0 and then multiplied by 32767, turning the
        # waveform into its own sign.
        decoded = FakeDecoded("SIGNED16", [0, 5000, -21000, 32767, -32768])
        out = to_float_samples(decoded)

        self.assertAlmostEqual(out[0], 0.0)
        self.assertAlmostEqual(out[1], 5000 / 32768.0, places=6)
        self.assertAlmostEqual(out[2], -21000 / 32768.0, places=6)
        self.assertTrue(all(-1.0001 <= s <= 1.0001 for s in out))
        self.assertNotEqual(
            [abs(s) for s in out[1:4]],
            [1.0, 1.0, 1.0],
            "a mid-level sample must not come out at full scale",
        )

    def test_float32_passes_through(self):
        decoded = FakeDecoded("FLOAT32", [0.0, 0.25, -0.5])
        self.assertEqual(to_float_samples(decoded), [0.0, 0.25, -0.5])

    def test_unsigned8_is_centred(self):
        decoded = FakeDecoded("UNSIGNED8", [128, 255, 0])
        out = to_float_samples(decoded)
        self.assertAlmostEqual(out[0], 0.0)
        self.assertGreater(out[1], 0.9)
        self.assertLess(out[2], -0.9)

    def test_an_unknown_format_is_refused_rather_than_assumed(self):
        with self.assertRaises(RuntimeError) as ctx:
            to_float_samples(FakeDecoded("SIGNED24", [1, 2, 3]))
        self.assertIn("SIGNED24", str(ctx.exception))


class AudioConcerns(unittest.TestCase):
    def test_ordinary_speech_level_audio_raises_nothing(self):
        self.assertEqual(audio_concerns(sine()), [])

    def test_a_square_wave_is_reported_as_clipped(self):
        # What corpus-v1 actually contains: the sign of the waveform.
        square = [1.0 if s >= 0 else -1.0 for s in sine()]
        concerns = audio_concerns(square)
        self.assertTrue(any("full scale" in c for c in concerns), concerns)

    def test_silence_is_not_mistaken_for_a_clean_recording(self):
        concerns = audio_concerns([0.0] * 16000)
        self.assertTrue(any("silence" in c for c in concerns), concerns)

    def test_samples_past_full_scale_name_the_cause(self):
        concerns = audio_concerns([0.0, 5000.0, -21000.0])
        self.assertTrue(any("not normalized" in c for c in concerns), concerns)

    def test_empty_audio_is_a_concern_and_not_a_pass(self):
        self.assertTrue(audio_concerns([]))

    def test_the_threshold_is_low_enough_to_catch_heavy_limiting(self):
        # A percent of the file at the clipping threshold is already a gain
        # stage pinned to its ceiling; raising this past a few percent would
        # let the corpus-v1 class of failure through again.
        self.assertLessEqual(MAX_NEAR_CLIPPING_PERCENT, 5.0)


class AssertUsableAudio(unittest.TestCase):
    def test_a_usable_case_is_written(self):
        assert_usable_audio(sine(), "en_clean_sprint")

    def test_a_broken_case_refuses_by_name(self):
        square = [1.0 if s >= 0 else -1.0 for s in sine()]
        with self.assertRaises(RuntimeError) as ctx:
            assert_usable_audio(square, "en_clean_sprint")
        self.assertIn("en_clean_sprint", str(ctx.exception))


if __name__ == "__main__":
    unittest.main()
