"""The Vox ASR Shootout: several engines over one corpus, one evaluator.

The question this exists to answer is not "is Vox's transcription good". It is
"*which* of model, inference engine, segmentation, context or audio is costing
us", and that question cannot be answered by changing one of them and looking
at the transcript.

    corpus-v1 ──┬──▶ vox / whisper.cpp / ggml-small   ──┐
                ├──▶ vox / whisper.cpp / ggml-base    ──┤
                ├──▶ vox / whole-file                 ──┼──▶ ONE EVALUATOR ──▶ report
                ├──▶ faster-whisper / CTranslate2     ──┤
                └──▶ …                                ──┘

Three rules hold the thing together, and each one exists because the obvious
shortcut past it produces a number that looks like a result:

1. **One evaluator.** Every adapter returns a transcript and timings in the
   same shape, and `metrics.py` scores all of them. An engine that brought its
   own word error rate would be graded on its own normalization.
2. **Every run states its configuration.** Model, engine, quantization,
   language mode, context mode, segmentation mode, host. A run that does not
   say what it was is not comparable to anything, including itself next month.
3. **The audio is checked before the models are.** A case whose recording is
   clipped cannot separate two models, so it is refused rather than scored.
   This is not hypothetical: it is how corpus-v1 shipped.

An engine that cannot run — not installed, no model file, a feature not
compiled — is reported as `skipped` with the reason. It is never silently
dropped, because a comparison missing its fastest entrant reads as a
comparison.
"""

import argparse
import datetime
import json
import platform
import subprocess
import sys
from pathlib import Path

SCRIPT_DIR = Path(__file__).resolve().parent
REPO_ROOT = SCRIPT_DIR.parents[2]
sys.path.insert(0, str(REPO_ROOT / "tests" / "transcription" / "evaluator"))

from metrics import (  # noqa: E402
    calculate_cer,
    calculate_codeswitch_accuracy,
    calculate_hallucination_rate,
    calculate_wer,
)

DEFAULT_MANIFEST = REPO_ROOT / "tests" / "transcription" / "corpus-v1" / "manifest.json"
DEFAULT_ENGINES = REPO_ROOT / "tests" / "transcription" / "shootout" / "engines.json"
REPORTS_DIR = REPO_ROOT / "tests" / "transcription" / "reports"
REFERENCES_DIR = REPO_ROOT / "tests" / "transcription" / "references"


# --- host -------------------------------------------------------------------


def host_stamp() -> dict:
    """What machine this ran on.

    §20: a real-time factor without the processor that produced it is not a
    measurement, it is an anecdote. `platform` is stdlib and cross-platform;
    the richer fields are filled from `psutil` when it happens to be installed
    and reported as unmeasured when it is not, rather than guessed.
    """
    stamp = {
        "os": platform.system(),
        "os_release": platform.release(),
        "machine": platform.machine(),
        "processor": platform.processor() or "unreported",
        "python": platform.python_version(),
        "logical_cores": None,
        "total_ram_gb": None,
    }
    try:
        import os

        stamp["logical_cores"] = os.cpu_count()
    except Exception:
        pass
    try:
        import psutil

        stamp["total_ram_gb"] = round(psutil.virtual_memory().total / 1024**3, 1)
    except Exception:
        stamp["total_ram_gb"] = None
    return stamp


def vox_commit() -> str:
    """The commit the Vox side was built from, or `unknown`.

    Stage 1 of the reassessment is "freeze the current baseline", and a frozen
    baseline that cannot name its commit is not frozen.
    """
    try:
        out = subprocess.run(
            ["git", "-C", str(REPO_ROOT), "rev-parse", "--short", "HEAD"],
            capture_output=True,
            text=True,
            timeout=10,
        )
        if out.returncode == 0:
            dirty = subprocess.run(
                ["git", "-C", str(REPO_ROOT), "status", "--porcelain"],
                capture_output=True,
                text=True,
                timeout=10,
            )
            suffix = "-dirty" if dirty.stdout.strip() else ""
            return out.stdout.strip() + suffix
    except Exception:
        pass
    return "unknown"


# --- the Vox binary ---------------------------------------------------------


def find_vox_binary() -> Path | None:
    candidates = [
        REPO_ROOT / "native" / "src-tauri" / "target" / "release" / "benchmark.exe",
        REPO_ROOT / "native" / "src-tauri" / "target" / "release" / "benchmark",
        REPO_ROOT / "native" / "src-tauri" / "target" / "debug" / "benchmark.exe",
        REPO_ROOT / "native" / "src-tauri" / "target" / "debug" / "benchmark",
    ]
    for candidate in candidates:
        if candidate.is_file():
            return candidate
    return None


def binary_is_release(path: Path) -> bool:
    return "release" in path.parts


# --- audio gate -------------------------------------------------------------


def audit_case_audio(binary: Path, wav: Path) -> dict:
    """The recording's own measurements, from Vox's own `AudioStats`.

    Through the binary rather than reimplemented here, so "near clipping"
    means the same thing to the corpus audit, to the benchmark and to a live
    meeting's per-segment diagnostics. Three definitions of one threshold is
    how they drift.
    """
    out = subprocess.run(
        [str(binary), "--audio", str(wav), "--audio-stats-only"],
        capture_output=True,
        text=True,
    )
    if out.returncode != 0:
        return {"error": out.stderr.strip()[-400:], "concerns": ["audio could not be measured"]}
    return json.loads(out.stdout)


# --- engine adapters --------------------------------------------------------


class EngineUnavailable(Exception):
    """Raised with a reason a human can act on, never swallowed."""


def run_vox(spec: dict, case: dict, wav: Path, binary: Path | None) -> dict:
    """Vox's own pipeline, or Vox's model without Vox's pipeline.

    Both go through the same binary, which is the point: `--segmentation
    whole-file` changes exactly one variable, so the difference between the two
    rows is the segmenter and not also the engine.
    """
    if binary is None:
        raise EngineUnavailable(
            "the benchmark binary is not built — run "
            "`cargo build --release --bin benchmark` in native/src-tauri"
        )
    # A relative path in the matrix means "relative to the repository", not to
    # whatever directory the runner happened to be started from.
    model = Path(spec["model"]).expanduser()
    if not model.is_absolute():
        model = REPO_ROOT / model
    if not model.is_file():
        raise EngineUnavailable(f"model file not found: {model}")

    cmd = [
        str(binary),
        "--audio", str(wav),
        "--model", str(model),
        "--lang", spec.get("language", case.get("whisper_language", "auto")),
        "--preset", spec.get("preset", "balanced"),
        "--decode-path", spec.get("decode_path", "live"),
        "--segmentation", spec.get("segmentation", "vox"),
        "--context", spec.get("context", "previous"),
        "--trim-audio-ctx", "on" if spec.get("trim_audio_ctx", True) else "off",
    ]
    if spec.get("quantization"):
        cmd += ["--quantization", spec["quantization"]]
    if case.get("title"):
        cmd += ["--meeting-title", case["title"]]

    started = datetime.datetime.now()
    out = subprocess.run(cmd, capture_output=True, text=True)
    wall = (datetime.datetime.now() - started).total_seconds()
    if out.returncode != 0:
        raise EngineUnavailable(f"benchmark binary failed: {out.stderr.strip()[-400:]}")

    payload = json.loads(out.stdout)
    return {
        "transcript": payload["transcript"],
        "segments": payload.get("segments", []),
        "audio_seconds": payload["total_audio_duration_seconds"],
        "speech_seconds": payload["total_speech_seconds"],
        "decode_ms": payload["total_inference_ms"],
        "decode_rtf": payload["decode_real_time_factor"],
        "pipeline_rtf": payload["pipeline_real_time_factor"],
        "wall_seconds": wall,
        "segments_kept": payload.get("segments_kept_count", 0),
        "segments_discarded": payload.get("segments_discarded_count", 0),
        "segments_dropped": payload.get("segments_dropped_count", 0),
        "peak_queue_depth": payload.get("peak_queue_depth", 0),
        "stamp": payload["run"],
    }


def run_faster_whisper(spec: dict, case: dict, wav: Path, _binary) -> dict:
    """The same Whisper weights through CTranslate2 instead of whisper.cpp.

    §6 of the reassessment, and the single most informative row in the matrix:
    if one model runs at three times real time under one engine and a third of
    it under another, the bottleneck was never the model.

    Nothing about Vox's pipeline is involved — no segmenter, no queue, no
    quality gate — so this row is comparable to a Vox `whole-file` row and not
    to a Vox `vox` row. The report keeps them in separate columns for exactly
    that reason.
    """
    try:
        from faster_whisper import WhisperModel
    except ImportError as exc:
        raise EngineUnavailable(
            "faster-whisper is not installed — `pip install faster-whisper` "
            f"({exc})"
        ) from exc

    import time

    model_id = spec.get("model", "small")
    compute_type = spec.get("quantization", "int8")
    language = spec.get("language", case.get("whisper_language", "auto"))
    language = None if language in ("auto", "", None) else language

    try:
        model = WhisperModel(model_id, device=spec.get("device", "cpu"), compute_type=compute_type)
    except Exception as exc:
        raise EngineUnavailable(f"faster-whisper could not load '{model_id}': {exc}") from exc

    started = time.monotonic()
    segments, info = model.transcribe(
        str(wav),
        language=language,
        beam_size=spec.get("beam_size", 5),
        vad_filter=spec.get("vad_filter", False),
    )
    collected = list(segments)
    wall = time.monotonic() - started

    transcript = " ".join(s.text.strip() for s in collected if s.text.strip())
    audio_seconds = float(getattr(info, "duration", 0.0)) or 0.0
    decode_ms = int(wall * 1000)

    return {
        "transcript": transcript,
        # No Vox telemetry: this engine never entered Vox's pipeline, and a
        # synthesized `telemetry` block would make the rows look interchangeable.
        "segments": [],
        "audio_seconds": audio_seconds,
        "speech_seconds": audio_seconds,
        "decode_ms": decode_ms,
        "decode_rtf": (decode_ms / 1000.0) / audio_seconds if audio_seconds else 0.0,
        "pipeline_rtf": (decode_ms / 1000.0) / audio_seconds if audio_seconds else 0.0,
        "wall_seconds": wall,
        "segments_kept": len(collected),
        "segments_discarded": 0,
        "segments_dropped": 0,
        "peak_queue_depth": 0,
        "stamp": {
            "vox_version": "n/a",
            "engine": "faster-whisper/ctranslate2",
            "model": model_id,
            "quantization": compute_type,
            "decode_path": "batch",
            "preset": f"beam_size={spec.get('beam_size', 5)}",
            "strategy": "BeamSearch",
            "temperature_inc": 0.0,
            "trim_audio_context": False,
            "n_threads": None,
            "language_mode": language or "auto",
            "context_mode": "none",
            "segmentation_mode": "whole-file",
        },
    }


# TODO(asr-shootout): a Parakeet adapter. `capture::recognizers::RecognizerChoice`
# already builds one and `meetings::benchmark::CaseRunner` already drives it, but
# neither is reachable from a command line — the only entry point is the
# `run_speech_benchmark` Tauri command. Giving the benchmark binary a
# `--engine parakeet` that goes through `CaseRunner` rather than `spawn_worker`
# is the work, and it also needs the `parakeet` feature, whose ONNX Runtime
# download is what stops it being built in some environments.
ADAPTERS = {
    "vox": run_vox,
    "faster-whisper": run_faster_whisper,
}


# --- running ----------------------------------------------------------------


def score(reference: str, run: dict) -> dict:
    """One evaluator, over every engine's output. See rule 1."""
    hypothesis = run["transcript"].strip()
    telemetry = [s.get("telemetry") for s in run.get("segments", []) if s.get("telemetry")]
    return {
        "wer": calculate_wer(reference, hypothesis),
        "cer": calculate_cer(reference, hypothesis),
        "codeswitch": calculate_codeswitch_accuracy(reference, hypothesis),
        "hallucination": calculate_hallucination_rate(reference, hypothesis, telemetry),
    }


# Longest first, so `large-v3-turbo` is not matched as `large`.
MODEL_FAMILIES = [
    "large-v3-turbo",
    "large-v3",
    "large-v2",
    "large-v1",
    "large",
    "medium",
    "small",
    "base",
    "tiny",
]


def model_family(name: str) -> str:
    """The model behind a filename or a hub id.

    `ggml-small.bin`, `small`, `Systran/faster-whisper-small` and
    `ggml-small-q5_0.bin` are the same weights, and the whole
    engine-versus-model experiment depends on noticing that. An `.en` variant
    is *not* the same model as its multilingual sibling and keeps its suffix,
    because comparing them would be comparing two models.
    """
    n = name.lower().replace("_", "-")
    english_only = ".en" in n or "-en-" in n or n.endswith("-en")
    for family in MODEL_FAMILIES:
        if family in n:
            return f"{family}.en" if english_only else family
    return n


def classify(entry: dict, peers: list[dict]) -> dict:
    """Where this result's problem most likely lives (§18).

    Comparative by construction, because most of the taxonomy is only visible
    across rows. "Word error rate is high" says nothing on its own; "high here
    and low for the same model under another engine" says INFERENCE_ENGINE, and
    "high here and low for the same engine without our segmenter" says
    SEGMENTATION. A single run can only ever reach the first two verdicts.
    """
    reasons = []
    categories = []

    if entry.get("audio_concerns"):
        categories.append("AUDIO")
        reasons.append(
            "the recording itself failed the audio gate, so nothing below it can be attributed "
            "to a model"
        )

    if entry.get("segments_dropped", 0) > 0:
        categories.append("THROUGHPUT")
        reasons.append(
            f"{entry['segments_dropped']} segment(s) were refused by a full queue — this row's "
            "word error rate includes speech the decoder never saw"
        )
    elif entry.get("decode_rtf", 0.0) > 1.0:
        categories.append("THROUGHPUT")
        reasons.append(
            f"decode RTF {entry['decode_rtf']:.2f} is above real time; a live meeting on this "
            "configuration accumulates backlog"
        )

    wer = entry.get("wer_percent")
    if wer is None:
        return {"categories": categories or ["NONE"], "reasons": reasons}

    def same(other, *keys):
        for key in keys:
            if key == "model":
                if model_family(other["stamp"].get("model", "")) != model_family(
                    entry["stamp"].get("model", "")
                ):
                    return False
            elif other["stamp"].get(key) != entry["stamp"].get(key):
                return False
        return True

    # Same model and same pipeline shape, different engine.
    engine_peers = [
        p
        for p in peers
        if p is not entry
        and p.get("wer_percent") is not None
        and model_family(p["stamp"].get("model", "")) == model_family(entry["stamp"].get("model", ""))
        and p["stamp"].get("engine") != entry["stamp"].get("engine")
    ]
    for peer in engine_peers:
        if entry["decode_rtf"] > 1.5 * max(peer["decode_rtf"], 1e-6):
            categories.append("INFERENCE_ENGINE")
            reasons.append(
                f"the same model runs {entry['decode_rtf'] / max(peer['decode_rtf'], 1e-6):.1f}x "
                f"slower under {entry['stamp'].get('engine')} than under {peer['stamp'].get('engine')}"
            )
            break

    # Same engine and model, segmentation the only difference.
    seg_peers = [
        p
        for p in peers
        if p is not entry
        and p.get("wer_percent") is not None
        and same(p, "engine", "model", "preset", "language_mode")
        and p["stamp"].get("segmentation_mode") != entry["stamp"].get("segmentation_mode")
    ]
    for peer in seg_peers:
        if wer > peer["wer_percent"] + 3.0:
            categories.append("SEGMENTATION")
            reasons.append(
                f"{entry['stamp'].get('segmentation_mode')} scores {wer:.1f}% against "
                f"{peer['wer_percent']:.1f}% for {peer['stamp'].get('segmentation_mode')} on the "
                "same engine and model"
            )
            break

    # Same everything but the prompt.
    ctx_peers = [
        p
        for p in peers
        if p is not entry
        and p.get("wer_percent") is not None
        and same(p, "engine", "model", "preset", "segmentation_mode")
        and p["stamp"].get("context_mode") != entry["stamp"].get("context_mode")
    ]
    for peer in ctx_peers:
        if wer > peer["wer_percent"] + 3.0:
            categories.append("CONTEXT")
            reasons.append(
                f"context={entry['stamp'].get('context_mode')} scores {wer:.1f}% against "
                f"{peer['wer_percent']:.1f}% for context={peer['stamp'].get('context_mode')}"
            )
            break

    if entry.get("hallucination_segments", 0) > 0:
        categories.append("HALLUCINATION")
        reasons.append(
            f"{entry['hallucination_segments']} segment(s) tripped the quality gate"
        )

    if not categories and wer > 10.0:
        categories.append("MODEL")
        reasons.append(
            "no peer row isolates engine, segmentation or context as the cause, which leaves the "
            "model's own capacity"
        )

    return {"categories": categories or ["NONE"], "reasons": reasons}


def run_matrix(manifest, engines, cases, binary, force_bad_audio):
    results = []
    audio_audit = {}

    for case in cases:
        case_id = case["id"]
        lang = case["language"]
        wav = REPO_ROOT / "tests" / "transcription" / manifest["version"] / lang / f"{case_id}.wav"
        ref = REFERENCES_DIR / f"{case_id}.txt"
        if not wav.is_file() or not ref.is_file():
            print(f"[skip] {case_id}: audio or reference missing", file=sys.stderr)
            continue

        audit = audit_case_audio(binary, wav) if binary else {"concerns": ["binary not built"]}
        audio_audit[case_id] = audit
        concerns = audit.get("concerns", [])
        if concerns and not force_bad_audio:
            print(f"[refused] {case_id}: {concerns[0]}", file=sys.stderr)
            results.append(
                {
                    "case_id": case_id,
                    "language": lang,
                    "engine_id": "-",
                    "status": "refused",
                    "reason": "; ".join(concerns),
                    "audio_concerns": concerns,
                }
            )
            continue

        reference = ref.read_text(encoding="utf-8").strip()
        for spec in engines:
            engine_id = spec["id"]
            adapter = ADAPTERS.get(spec.get("adapter", "vox"))
            if adapter is None:
                results.append(
                    {
                        "case_id": case_id,
                        "language": lang,
                        "engine_id": engine_id,
                        "status": "skipped",
                        "reason": f"no adapter named '{spec.get('adapter')}'",
                    }
                )
                continue

            print(f"[run] {case_id} x {engine_id}", file=sys.stderr)
            try:
                run = adapter(spec, case, wav, binary)
            except EngineUnavailable as exc:
                print(f"[skip] {engine_id}: {exc}", file=sys.stderr)
                results.append(
                    {
                        "case_id": case_id,
                        "language": lang,
                        "engine_id": engine_id,
                        "status": "skipped",
                        "reason": str(exc),
                    }
                )
                continue

            scored = score(reference, run)
            results.append(
                {
                    "case_id": case_id,
                    "language": lang,
                    "engine_id": engine_id,
                    "status": "scored",
                    "audio_concerns": concerns,
                    "wer_percent": scored["wer"]["wer_percent"],
                    "cer_percent": scored["cer"]["cer_percent"],
                    "reference_words": scored["wer"]["ref_words"],
                    "substitutions": scored["wer"]["substitutions"],
                    "deletions": scored["wer"]["deletions"],
                    "insertions": scored["wer"]["insertions"],
                    "hindi_retention": scored["codeswitch"]["hindi_retention_percent"],
                    "english_retention": scored["codeswitch"]["english_retention_percent"],
                    "domain_accuracy": scored["codeswitch"]["domain_accuracy_percent"],
                    "hallucination_segments": scored["hallucination"]["suspicious_segments_count"],
                    "decode_rtf": run["decode_rtf"],
                    "pipeline_rtf": run["pipeline_rtf"],
                    "wall_seconds": run["wall_seconds"],
                    "segments_kept": run["segments_kept"],
                    "segments_dropped": run["segments_dropped"],
                    "peak_queue_depth": run["peak_queue_depth"],
                    "stamp": run["stamp"],
                    "transcript": run["transcript"],
                }
            )

    scored_rows = [r for r in results if r["status"] == "scored"]
    for row in scored_rows:
        peers = [p for p in scored_rows if p["case_id"] == row["case_id"]]
        row["diagnosis"] = classify(row, peers)

    return results, audio_audit


# --- reporting --------------------------------------------------------------


def render_markdown(payload: dict) -> str:
    md = ["# VOX ASR SHOOTOUT", ""]
    md.append(f"**Date:** {payload['generated_at']}")
    md.append(f"**Corpus:** {payload['corpus_version']}")
    md.append(f"**Vox commit:** `{payload['vox_commit']}`")
    host = payload["host"]
    md.append(
        f"**Host:** {host['os']} {host['os_release']} · {host['machine']} · "
        f"{host['processor']} · {host['logical_cores'] or '?'} logical cores · "
        f"{host['total_ram_gb'] or '?'} GB RAM"
    )
    md.append(f"**Binary:** {payload['binary'] or 'not built'} ({payload['binary_profile']})")
    md.append("")

    if payload["binary_profile"] == "debug":
        md.append(
            "> **These timings are from a debug build.** Cargo builds whisper.cpp with "
            "optimizations either way, but everything around it — the segmenter, the "
            "resampler, the quality gate — is unoptimized. Rebuild with `--release` "
            "before comparing throughput."
        )
        md.append("")

    md.append("## Audio gate")
    md.append("")
    md.append(
        "Checked before any model, because a clipped recording cannot separate two of them."
    )
    md.append("")
    md.append("| Case | Duration | RMS | Peak | Near clipping | Voiced | SNR est. | Verdict |")
    md.append("|---|---|---|---|---|---|---|---|")
    for case_id, audit in payload["audio_audit"].items():
        if "error" in audit:
            md.append(f"| `{case_id}` | — | — | — | — | — | — | could not measure |")
            continue
        verdict = "ok" if not audit["concerns"] else f"**{len(audit['concerns'])} concern(s)**"
        snr = audit.get("snr_db_estimate")
        md.append(
            f"| `{case_id}` | {audit['duration_seconds']:.0f}s | {audit['rms']:.4f} | "
            f"{audit['peak_amplitude']:.3f} | {audit['near_clipping_percent']:.2f}% | "
            f"{audit['voiced_ratio'] * 100:.1f}% | "
            f"{(f'{snr:.1f} dB' if snr is not None else '—')} | {verdict} |"
        )
    md.append("")
    for case_id, audit in payload["audio_audit"].items():
        for concern in audit.get("concerns", []):
            md.append(f"- `{case_id}`: {concern}")
    md.append("")

    scored = [r for r in payload["results"] if r["status"] == "scored"]
    md.append("## Results")
    md.append("")
    if not scored:
        md.append("No row was scored. Every entry below says why.")
    else:
        md.append(
            "| Case | Engine | Model | Quant | Seg | Context | WER | CER | decode RTF | "
            "Dropped | Diagnosis |"
        )
        md.append("|---|---|---|---|---|---|---|---|---|---|---|")
        for r in scored:
            s = r["stamp"]
            md.append(
                f"| `{r['case_id']}` | {s.get('engine')} | {s.get('model')} | "
                f"{s.get('quantization') or '—'} | {s.get('segmentation_mode')} | "
                f"{s.get('context_mode')} | {r['wer_percent']:.2f}% | {r['cer_percent']:.2f}% | "
                f"{r['decode_rtf']:.3f} | {r['segments_dropped']} | "
                f"{', '.join(r['diagnosis']['categories'])} |"
            )
    md.append("")

    not_run = [r for r in payload["results"] if r["status"] != "scored"]
    md.append("## Not run")
    md.append("")
    if not not_run:
        md.append("Nothing — every case ran against every engine.")
    else:
        md.append("| Case | Engine | Status | Reason |")
        md.append("|---|---|---|---|")
        for r in not_run:
            md.append(
                f"| `{r['case_id']}` | {r['engine_id']} | {r['status']} | {r.get('reason', '')} |"
            )
    md.append("")

    md.append("## Diagnosis detail")
    md.append("")
    if not scored:
        md.append("Nothing was scored, so nothing is diagnosed.")
    for r in scored:
        if r["diagnosis"]["categories"] == ["NONE"]:
            continue
        md.append(f"### `{r['case_id']}` × {r['engine_id']} — {', '.join(r['diagnosis']['categories'])}")
        for reason in r["diagnosis"]["reasons"]:
            md.append(f"- {reason}")
        md.append("")

    return "\n".join(md)


def main():
    parser = argparse.ArgumentParser(description="Vox ASR Shootout")
    parser.add_argument("--corpus", default=str(DEFAULT_MANIFEST))
    parser.add_argument("--engines", default=str(DEFAULT_ENGINES))
    parser.add_argument("--cases", default=None, help="comma-separated case ids")
    parser.add_argument("--out-dir", default=str(REPORTS_DIR))
    parser.add_argument(
        "--audit-only",
        action="store_true",
        help="run the audio gate over the corpus and stop, without loading a model",
    )
    parser.add_argument(
        "--force-bad-audio",
        action="store_true",
        help="score cases that failed the audio gate anyway. Their numbers measure the "
             "recording as much as the model, and the report says so on every row.",
    )
    args = parser.parse_args()

    manifest = json.loads(Path(args.corpus).read_text(encoding="utf-8"))
    selected = args.cases.split(",") if args.cases else None
    cases = [c for c in manifest["cases"] if selected is None or c["id"] in selected]

    binary = find_vox_binary()
    if binary is None:
        print(
            "[warn] the benchmark binary is not built; every Vox row will be skipped",
            file=sys.stderr,
        )

    if args.audit_only:
        engines = []
    else:
        engines_path = Path(args.engines)
        if not engines_path.is_file():
            print(f"Error: no engine matrix at {engines_path}", file=sys.stderr)
            sys.exit(1)
        engines = json.loads(engines_path.read_text(encoding="utf-8"))["engines"]
        engines = [e for e in engines if e.get("enabled", True)]

    results, audio_audit = run_matrix(manifest, engines, cases, binary, args.force_bad_audio)

    payload = {
        "generated_at": datetime.datetime.now(datetime.timezone.utc).isoformat(),
        "corpus_version": manifest["version"],
        "vox_commit": vox_commit(),
        "host": host_stamp(),
        "binary": binary.name if binary else None,
        "binary_profile": "release" if binary and binary_is_release(binary) else "debug",
        "audio_audit": audio_audit,
        "results": results,
    }

    out_dir = Path(args.out_dir)
    out_dir.mkdir(parents=True, exist_ok=True)
    (out_dir / "shootout.json").write_text(
        json.dumps(payload, indent=2, ensure_ascii=False), encoding="utf-8"
    )
    md_path = out_dir / "ASR_SHOOTOUT.md"
    md_path.write_text(render_markdown(payload), encoding="utf-8")
    print(f"[report] {md_path}")
    print(f"[report] {out_dir / 'shootout.json'}")


if __name__ == "__main__":
    main()
