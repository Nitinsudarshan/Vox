"""The shootout's comparative layer: what it concludes, and what it refuses to.

Most of the error taxonomy is only visible across rows — "word error rate is
high" means nothing until another row holds everything but one variable fixed.
These tests build those pairs directly, because a real run needs models nobody
checks into a repository.

Run with `python3 -m unittest discover -s tests/transcription/tests` from the
repository root.
"""

import sys
import unittest
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(REPO_ROOT / "tests" / "transcription" / "runner"))

from shootout import classify, model_family, render_markdown  # noqa: E402


def row(**overrides):
    """A scored row with production-shaped defaults, adjusted per test."""
    stamp = {
        "vox_version": "0.1.0",
        "engine": "whisper.cpp",
        "model": "ggml-small.bin",
        "quantization": "f16",
        "decode_path": "live",
        "preset": "balanced",
        "strategy": "BeamSearch",
        "temperature_inc": 0.2,
        "trim_audio_context": True,
        "n_threads": None,
        "language_mode": "en",
        "context_mode": "previous",
        "segmentation_mode": "vox",
    }
    stamp.update(overrides.pop("stamp", {}))
    base = {
        "case_id": "en_clean_sprint",
        "language": "en",
        "engine_id": "vox-small-baseline",
        "status": "scored",
        "audio_concerns": [],
        "wer_percent": 6.0,
        "cer_percent": 3.0,
        "decode_rtf": 0.4,
        "pipeline_rtf": 0.35,
        "segments_dropped": 0,
        "hallucination_segments": 0,
        "stamp": stamp,
    }
    base.update(overrides)
    return base


class Taxonomy(unittest.TestCase):
    def test_a_healthy_row_is_diagnosed_as_nothing(self):
        entry = row()
        self.assertEqual(classify(entry, [entry])["categories"], ["NONE"])

    def test_clipped_audio_outranks_every_model_conclusion(self):
        entry = row(audio_concerns=["92% of samples at full scale"], wer_percent=40.0)
        verdict = classify(entry, [entry])
        self.assertIn("AUDIO", verdict["categories"])
        self.assertTrue(
            any("nothing below it can be attributed" in r for r in verdict["reasons"]),
            verdict,
        )

    def test_dropped_segments_are_throughput_not_accuracy(self):
        entry = row(segments_dropped=4, wer_percent=22.0)
        verdict = classify(entry, [entry])
        self.assertIn("THROUGHPUT", verdict["categories"])
        self.assertTrue(any("never saw" in r for r in verdict["reasons"]), verdict)

    def test_a_decoder_slower_than_real_time_is_flagged_even_when_accurate(self):
        entry = row(decode_rtf=3.03, wer_percent=4.0)
        self.assertIn("THROUGHPUT", classify(entry, [entry])["categories"])

    def test_the_same_model_under_two_engines_names_the_engine(self):
        slow = row(engine_id="vox-small-whole-file", decode_rtf=3.0,
                   stamp={"segmentation_mode": "whole-file"})
        fast = row(engine_id="faster-whisper-small", decode_rtf=0.4,
                   stamp={"engine": "faster-whisper/ctranslate2",
                          "model": "small",
                          "segmentation_mode": "whole-file"})
        verdict = classify(slow, [slow, fast])
        self.assertIn("INFERENCE_ENGINE", verdict["categories"])

    def test_segmentation_is_named_only_when_it_is_the_only_difference(self):
        segmented = row(wer_percent=18.0)
        whole = row(engine_id="vox-small-whole-file", wer_percent=7.0,
                    stamp={"segmentation_mode": "whole-file"})
        verdict = classify(segmented, [segmented, whole])
        self.assertIn("SEGMENTATION", verdict["categories"])

    def test_a_different_model_does_not_count_as_a_segmentation_comparison(self):
        segmented = row(wer_percent=18.0)
        other_model = row(
            engine_id="vox-base",
            wer_percent=7.0,
            stamp={"model": "ggml-base.bin", "segmentation_mode": "whole-file"},
        )
        verdict = classify(segmented, [segmented, other_model])
        self.assertNotIn("SEGMENTATION", verdict["categories"])

    def test_prompt_conditioning_is_named_when_it_is_the_only_difference(self):
        primed = row(wer_percent=15.0)
        bare = row(engine_id="vox-small-no-context", wer_percent=8.0,
                   stamp={"context_mode": "none"})
        verdict = classify(primed, [primed, bare])
        self.assertIn("CONTEXT", verdict["categories"])

    def test_model_capacity_is_the_conclusion_of_last_resort(self):
        # Nothing else in the matrix isolates a cause, so the model's own
        # capacity is what is left — and only then.
        entry = row(wer_percent=24.0)
        verdict = classify(entry, [entry])
        self.assertEqual(verdict["categories"], ["MODEL"])


class ModelFamily(unittest.TestCase):
    """Two adapters naming one model differently must not read as two models."""

    def test_a_ggml_filename_and_a_hub_id_are_the_same_model(self):
        self.assertEqual(model_family("ggml-small.bin"), model_family("small"))
        self.assertEqual(
            model_family("ggml-small-q5_0.bin"),
            model_family("Systran/faster-whisper-small"),
        )

    def test_large_v3_turbo_is_not_large(self):
        self.assertNotEqual(model_family("ggml-large-v3-turbo.bin"), model_family("ggml-large.bin"))
        self.assertEqual(model_family("ggml-large-v3-turbo.bin"), "large-v3-turbo")

    def test_an_english_only_variant_is_a_different_model(self):
        self.assertNotEqual(model_family("ggml-small.en.bin"), model_family("ggml-small.bin"))


class Reporting(unittest.TestCase):
    def payload(self, results, audit=None):
        return {
            "generated_at": "2026-09-20T00:00:00Z",
            "corpus_version": "corpus-v1",
            "vox_commit": "abc1234",
            "host": {
                "os": "Windows",
                "os_release": "11",
                "machine": "AMD64",
                "processor": "Intel",
                "logical_cores": 8,
                "total_ram_gb": 16.0,
            },
            "binary": "benchmark.exe",
            "binary_profile": "release",
            "audio_audit": audit or {},
            "results": results,
        }

    def test_a_run_that_scored_nothing_says_so(self):
        md = render_markdown(
            self.payload(
                [{"case_id": "en_clean_sprint", "engine_id": "vox-small-baseline",
                  "status": "refused", "reason": "clipped"}]
            )
        )
        self.assertIn("No row was scored", md)
        self.assertIn("clipped", md)

    def test_skipped_engines_appear_with_their_reason(self):
        md = render_markdown(
            self.payload(
                [{"case_id": "en_clean_sprint", "engine_id": "faster-whisper-small",
                  "status": "skipped", "reason": "faster-whisper is not installed"}]
            )
        )
        self.assertIn("faster-whisper is not installed", md)

    def test_a_debug_build_warns_before_any_timing_is_read(self):
        payload = self.payload([])
        payload["binary_profile"] = "debug"
        md = render_markdown(payload)
        self.assertIn("debug build", md)
        self.assertLess(md.index("debug build"), md.index("## Audio gate"))

    def test_every_scored_row_carries_the_configuration_that_produced_it(self):
        entry = row()
        entry["diagnosis"] = classify(entry, [entry])
        md = render_markdown(self.payload([entry]))
        for expected in ("whisper.cpp", "ggml-small.bin", "f16", "vox", "previous"):
            self.assertIn(expected, md)


if __name__ == "__main__":
    unittest.main()
