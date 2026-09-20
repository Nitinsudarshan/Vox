"""Whether generated audio can support the comparison it was made for.

Separated from `generate_corpus.py` so it can be tested without `edge_tts` or
`miniaudio` installed — the two imports that make that module unrunnable
anywhere the synthesizer is not reachable. The checks here are the reason the
split is worth it: they are what would have caught corpus-v1 before it was
committed.
"""

import math

# A generated file whose samples sit at full scale is not "loud" — it is
# destroyed. See `assert_usable_audio`.
MAX_NEAR_CLIPPING_PERCENT = 1.0

# Below this the synthesizer produced silence, whatever it returned.
MIN_RMS = 0.01


def to_float_samples(decoded) -> list[float]:
    """miniaudio's samples as floats in [-1.0, 1.0], whatever it decoded to.

    This function exists because of a bug it now cannot reproduce. The
    generator asked `miniaudio.decode` for 16 kHz mono and took
    `decoded.samples` as if they were floats. They are **signed 16-bit
    integers** by default, so every non-zero sample — 5000, -21000, anything —
    was clamped to ±1.0 by the writer's `max(-1.0, min(1.0, s))` and then
    multiplied by 32767. The corpus that came out was the *sign* of the
    waveform and nothing else: a square wave at full scale, 91% to 99% of its
    samples at the clipping threshold.

    Nothing in the pipeline objected. The files were valid 16 kHz mono WAVs of
    the right length, they played as recognisable speech, and Whisper
    transcribed them at 15% word error rate — which reads as a model result
    and was a measurement of 1-bit distortion.

    So: convert explicitly, from whatever format was actually decoded, and let
    `assert_usable_audio` refuse the output if this is ever wrong again.
    """
    fmt = getattr(decoded, "sample_format", None)
    fmt_name = getattr(fmt, "name", str(fmt))
    samples = decoded.samples

    if fmt_name == "FLOAT32":
        return [float(s) for s in samples]
    if fmt_name == "SIGNED16":
        return [s / 32768.0 for s in samples]
    if fmt_name == "SIGNED32":
        return [s / 2147483648.0 for s in samples]
    if fmt_name == "UNSIGNED8":
        return [(s - 128) / 128.0 for s in samples]
    raise RuntimeError(
        f"miniaudio decoded to {fmt_name}, which this generator does not know how to "
        "normalize. Add it here rather than assuming the samples are already floats — "
        "assuming that is what produced a corpus of square waves."
    )


def audio_concerns(samples: list[float]) -> list[str]:
    """Why this audio cannot be used as ground truth, if it cannot.

    The same three questions `benchmark --audio-stats-only` asks of a recording
    in `native/src-tauri/src/bin/benchmark.rs`, asked here at generation time,
    because a corpus is worth checking before it is committed rather than after
    it has produced a report.
    """
    if not samples:
        return ["no samples were produced"]
    peak = max(abs(s) for s in samples)
    rms = math.sqrt(sum(s * s for s in samples) / len(samples))
    near_clipping = 100.0 * sum(1 for s in samples if abs(s) >= 0.98) / len(samples)

    concerns = []
    if peak > 1.0001:
        concerns.append(
            f"samples reach {peak:.3f}, past full scale — these are not normalized floats"
        )
    if near_clipping >= MAX_NEAR_CLIPPING_PERCENT:
        concerns.append(
            f"{near_clipping:.1f}% of samples are at or beyond 98% of full scale; "
            "speech does not look like this and a model cannot be judged on it"
        )
    if rms < MIN_RMS:
        concerns.append(f"RMS {rms:.4f} is silence, not speech")
    return concerns


def assert_usable_audio(samples: list[float], case_id: str) -> None:
    """Refuses to write a case that cannot support the comparison it is for."""
    concerns = audio_concerns(samples)
    if concerns:
        raise RuntimeError(
            f"{case_id}: refusing to write this case — "
            + "; ".join(concerns)
            + ". A corpus that ships broken audio produces word error rates that look "
            "like model results."
        )


