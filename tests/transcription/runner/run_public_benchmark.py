"""
Public Speech Corpora Benchmark Runner for Vox.

Executes reproducible public speech datasets (Mozilla Common Voice & Google FLEURS)
through the exact production Vox transcription pipeline (benchmark.exe).
Evaluates accuracy (WER, CER, S/D/I), latency (RTF, decode ms), and failure taxonomy
across Whisper Small, Whisper Large-v3-Turbo, and Parakeet.
"""

from __future__ import annotations

import argparse
import csv
import datetime
import json
import os
import subprocess
import sys
from pathlib import Path
from typing import Any, Dict, List, Optional

# Add paths
SCRIPT_DIR = Path(__file__).resolve().parent
REPO_ROOT = SCRIPT_DIR.parents[2]
PUBLIC_DIR = REPO_ROOT / "tests" / "transcription" / "public"
MANIFESTS_DIR = PUBLIC_DIR / "manifests"
REPORTS_DIR = REPO_ROOT / "tests" / "transcription" / "reports" / "public"

sys.path.insert(0, str(PUBLIC_DIR))
sys.path.insert(0, str(REPO_ROOT / "tests" / "transcription" / "evaluator"))

from schema import PublicSpeechDataset, PublicSpeechSample
from metrics import (
    calculate_cer,
    calculate_codeswitch_accuracy,
    calculate_hallucination_rate,
    calculate_wer,
    classify_transcription_failure,
)

MODEL_MAP = {
    "whisper-small": REPO_ROOT / "native" / "src-tauri" / ".vox" / "config" / "models" / "ggml-small.bin",
    "whisper-large-v3-turbo": REPO_ROOT / "native" / "src-tauri" / ".vox" / "config" / "models" / "ggml-large-v3-turbo-q5_0.bin",
    "parakeet": REPO_ROOT / "native" / "src-tauri" / ".vox" / "config" / "models" / "parakeet" / "model.onnx",
}

def find_benchmark_binary() -> Path:
    candidates = [
        REPO_ROOT / "native" / "src-tauri" / "target" / "release" / "benchmark.exe",
        REPO_ROOT / "native" / "src-tauri" / "target" / "release" / "benchmark",
        REPO_ROOT / "native" / "src-tauri" / "target" / "debug" / "benchmark.exe",
        REPO_ROOT / "native" / "src-tauri" / "target" / "debug" / "benchmark",
    ]
    for c in candidates:
        if c.is_file():
            return c
    raise FileNotFoundError("Benchmark binary not found. Please run 'cargo build --bin benchmark' first.")


def run_sample_through_pipeline(
    bin_path: Path,
    model_path: Path,
    sample: PublicSpeechSample,
    audio_file: Path
) -> Dict[str, Any]:
    temp_out = REPORTS_DIR / f"temp_{sample.sample_id}.json"
    temp_out.parent.mkdir(parents=True, exist_ok=True)

    whisper_lang = sample.language if sample.language != "auto" else "auto"
    title = f"{sample.dataset_name}_{sample.sample_id}"

    cmd = [
        str(bin_path),
        "--audio", str(audio_file),
        "--model", str(model_path),
        "--lang", whisper_lang,
        "--meeting-title", title,
        "--output", str(temp_out)
    ]

    start_t = datetime.datetime.now()
    proc = subprocess.run(cmd, capture_output=True, text=True, encoding="utf-8")
    elapsed_sec = (datetime.datetime.now() - start_t).total_seconds()

    if proc.returncode != 0:
        raise RuntimeError(f"Pipeline execution failed on {sample.sample_id}: {proc.stderr}")

    with open(temp_out, "r", encoding="utf-8") as f:
        pipeline_data = json.load(f)

    temp_out.unlink(missing_ok=True)

    hyp_text = pipeline_data.get("transcript", "").strip()
    ref_text = sample.reference_transcript.strip()

    wer_data = calculate_wer(ref_text, hyp_text)
    cer_data = calculate_cer(ref_text, hyp_text)

    telemetry_list = [s.get("telemetry") for s in pipeline_data.get("segments", []) if s.get("telemetry")]
    hallucination_data = calculate_hallucination_rate(ref_text, hyp_text, telemetry_list)

    case_meta = {
        "id": sample.sample_id,
        "language": sample.language,
        "split": sample.split,
        "dataset_name": sample.dataset_name,
        "metadata": sample.metadata,
        "category": sample.dataset_name
    }
    failure = classify_transcription_failure(case_meta, wer_data, cer_data, telemetry_list)

    decode_rtf = pipeline_data.get("decode_real_time_factor", 0.0)
    pipeline_rtf = pipeline_data.get("pipeline_real_time_factor", 0.0)
    inference_ms = pipeline_data.get("total_inference_ms", 0)

    return {
        "sample_id": sample.sample_id,
        "dataset_name": sample.dataset_name,
        "dataset_version": sample.dataset_version,
        "language": sample.language,
        "split": sample.split,
        "speaker_id": sample.speaker_id,
        "duration_seconds": pipeline_data.get("total_audio_duration_seconds", sample.duration_seconds),
        "speech_seconds": pipeline_data.get("total_speech_seconds", sample.duration_seconds),
        "inference_ms": inference_ms,
        "decode_rtf": decode_rtf,
        "pipeline_rtf": pipeline_rtf,
        "reference": ref_text,
        "hypothesis": hyp_text,
        "wer": wer_data,
        "cer": cer_data,
        "hallucination": hallucination_data,
        "failure": failure,
        "segments_decoded": pipeline_data.get("segments_decoded_count", 0),
        "segments_kept": pipeline_data.get("segments_kept_count", 0),
        "segments_discarded": pipeline_data.get("segments_discarded_count", 0),
    }


def evaluate_manifest(
    manifest_path: Path,
    bin_path: Path,
    model_path: Path,
    recognizer_name: str,
    max_samples: Optional[int] = None
) -> Dict[str, Any]:
    ds = PublicSpeechDataset.from_jsonl(manifest_path)
    print(f"\nEvaluating manifest: {manifest_path.name} ({len(ds.samples)} samples, lang={ds.language})...")

    results: List[Dict[str, Any]] = []
    samples_to_run = ds.samples[:max_samples] if max_samples else ds.samples

    for s in samples_to_run:
        rel_audio = Path(s.audio_path)
        audio_file = PUBLIC_DIR / rel_audio
        if not audio_file.is_file():
            print(f"  [SKIP] Audio file not found: {audio_file}")
            continue

        print(f"  -> [{s.sample_id}] Running {s.language} ({s.duration_seconds:.1f}s)...", end=" ", flush=True)
        res = run_sample_through_pipeline(bin_path, model_path, s, audio_file)
        results.append(res)
        print(f"WER: {res['wer']['wer_percent']:.1f}% | CER: {res['cer']['cer_percent']:.1f}% | RTF: {res['decode_rtf']:.2f}")

    if not results:
        return {
            "manifest": manifest_path.name,
            "dataset_name": ds.dataset_name,
            "language": ds.language,
            "samples_evaluated": 0,
            "avg_wer": 0.0,
            "avg_cer": 0.0,
            "avg_rtf": 0.0,
            "results": []
        }

    total_ref_words = sum(r["wer"]["ref_words"] for r in results)
    total_errors = sum(r["wer"]["total_errors"] for r in results)
    total_ref_chars = sum(r["cer"]["ref_chars"] for r in results)
    total_char_errors = sum(r["cer"]["total_errors"] for r in results)
    avg_rtf = sum(r["decode_rtf"] for r in results) / len(results)

    manifest_wer = (total_errors / total_ref_words * 100.0) if total_ref_words > 0 else 0.0
    manifest_cer = (total_char_errors / total_ref_chars * 100.0) if total_ref_chars > 0 else 0.0

    return {
        "manifest": manifest_path.name,
        "dataset_name": ds.dataset_name,
        "language": ds.language,
        "samples_evaluated": len(results),
        "total_ref_words": total_ref_words,
        "total_ref_chars": total_ref_chars,
        "avg_wer": round(manifest_wer, 2),
        "avg_cer": round(manifest_cer, 2),
        "avg_rtf": round(avg_rtf, 3),
        "results": results
    }


def generate_reports(
    eval_summaries: List[Dict[str, Any]],
    recognizer_name: str,
    model_path: Path,
    out_dir: Path
) -> None:
    out_dir.mkdir(parents=True, exist_ok=True)
    timestamp = datetime.datetime.now(datetime.timezone.utc).isoformat()

    # 1. JSON Report
    json_path = out_dir / "benchmark_report.json"
    full_data = {
        "timestamp": timestamp,
        "recognizer": recognizer_name,
        "model_path": str(model_path.relative_to(REPO_ROOT) if model_path.is_relative_to(REPO_ROOT) else model_path),
        "manifests_count": len(eval_summaries),
        "manifest_summaries": eval_summaries
    }
    with open(json_path, "w", encoding="utf-8") as f:
        json.dump(full_data, f, indent=2, ensure_ascii=False)
    print(f"\n[REPORT] JSON written to {json_path}")

    # 2. CSV Report
    csv_path = out_dir / "benchmark_report.csv"
    with open(csv_path, "w", newline="", encoding="utf-8") as f:
        writer = csv.writer(f)
        writer.writerow([
            "Manifest", "Dataset", "Language", "Sample ID", "Ref Words", "Hyp Words",
            "WER (%)", "CER (%)", "Substitutions", "Deletions", "Insertions",
            "Decode RTF", "Pipeline RTF", "Inference (ms)", "Primary Failure"
        ])
        for summary in eval_summaries:
            for r in summary["results"]:
                writer.writerow([
                    summary["manifest"],
                    r["dataset_name"],
                    r["language"],
                    r["sample_id"],
                    r["wer"]["ref_words"],
                    r["wer"]["hyp_words"],
                    f"{r['wer']['wer_percent']:.2f}",
                    f"{r['cer']['cer_percent']:.2f}",
                    r["wer"]["substitutions"],
                    r["wer"]["deletions"],
                    r["wer"]["insertions"],
                    f"{r['decode_rtf']:.3f}",
                    f"{r['pipeline_rtf']:.3f}",
                    r["inference_ms"],
                    r["failure"]["primary_category"]
                ])
    print(f"[REPORT] CSV written to {csv_path}")

    # 3. Markdown Report: docs/benchmarks/TRANSCRIPTION_BENCHMARK_PUBLIC_V1.md —
    # under docs/ rather than the repository root, where AGENTS.md asks for no
    # new markdown files.
    md_path = REPO_ROOT / "docs" / "benchmarks" / "TRANSCRIPTION_BENCHMARK_PUBLIC_V1.md"
    md_path.parent.mkdir(parents=True, exist_ok=True)
    
    # Compute aggregates
    cv_summaries = [s for s in eval_summaries if s["dataset_name"] == "common_voice"]
    fleurs_summaries = [s for s in eval_summaries if s["dataset_name"] == "fleurs"]

    lines = []
    lines.append("# VOX PUBLIC SPEECH CORPORA BENCHMARK REPORT (V1)")
    lines.append(f"**Date:** {timestamp}")
    lines.append(f"**Recognizer:** `{recognizer_name}` (`{model_path.name}`)")
    lines.append(f"**Evaluation Scope:** Mozilla Common Voice (English Standard & Spontaneous) + Google FLEURS (10 Indian Languages)")
    lines.append(f"**Pipeline Mode:** Production Meeting Pipeline (Segmenter + VAD/Queue + Whisper + Quality Gate + Recovery Pass)")
    lines.append("")
    lines.append("> [!IMPORTANT]")
    lines.append("> Per benchmark fairness specifications, each public dataset and language is reported individually.")
    lines.append("> Multilingual results are **NOT** collapsed into a single misleading aggregate number.")
    lines.append("")
    lines.append("## 1. Mozilla Common Voice Results")
    lines.append("| Split / Profile | Manifest | Lang | Samples | Ref Words | WER (%) | CER (%) | Avg RTF |")
    lines.append("|---|---|---|---|---|---|---|---|")
    for s in cv_summaries:
        lines.append(f"| `{s['manifest']}` | `{s['manifest']}` | `{s['language']}` | {s['samples_evaluated']} | {s['total_ref_words']} | **{s['avg_wer']:.2f}%** | {s['avg_cer']:.2f}% | {s['avg_rtf']:.3f} |")
    lines.append("")
    lines.append("## 2. Google FLEURS Results (10 Indian Languages)")
    lines.append("| Language | Code | Manifest | Samples | Ref Words | WER (%) | CER (%) | Avg RTF |")
    lines.append("|---|---|---|---|---|---|---|---|")
    for s in fleurs_summaries:
        lines.append(f"| {s['language'].upper()} | `{s['language']}` | `{s['manifest']}` | {s['samples_evaluated']} | {s['total_ref_words']} | **{s['avg_wer']:.2f}%** | {s['avg_cer']:.2f}% | {s['avg_rtf']:.3f} |")
    lines.append("")
    
    # Aggregates section
    lines.append("## 3. Dataset-Level Aggregates (Reported Separately)")
    lines.append("| Dataset | Total Samples | Total Words | Mean WER (%) | Mean CER (%) | Mean RTF |")
    lines.append("|---|---|---|---|---|---|")
    if cv_summaries:
        cv_words = sum(s["total_ref_words"] for s in cv_summaries)
        cv_samples = sum(s["samples_evaluated"] for s in cv_summaries)
        cv_mean_wer = sum(s["avg_wer"] for s in cv_summaries) / len(cv_summaries)
        cv_mean_cer = sum(s["avg_cer"] for s in cv_summaries) / len(cv_summaries)
        cv_mean_rtf = sum(s["avg_rtf"] for s in cv_summaries) / len(cv_summaries)
        lines.append(f"| **Mozilla Common Voice** | {cv_samples} | {cv_words} | **{cv_mean_wer:.2f}%** | {cv_mean_cer:.2f}% | {cv_mean_rtf:.3f} |")
    if fleurs_summaries:
        fl_words = sum(s["total_ref_words"] for s in fleurs_summaries)
        fl_samples = sum(s["samples_evaluated"] for s in fleurs_summaries)
        fl_mean_wer = sum(s["avg_wer"] for s in fleurs_summaries) / len(fleurs_summaries)
        fl_mean_cer = sum(s["avg_cer"] for s in fleurs_summaries) / len(fleurs_summaries)
        fl_mean_rtf = sum(s["avg_rtf"] for s in fleurs_summaries) / len(fleurs_summaries)
        lines.append(f"| **Google FLEURS (10 Languages)** | {fl_samples} | {fl_words} | **{fl_mean_wer:.2f}%** | {fl_mean_cer:.2f}% | {fl_mean_rtf:.3f} |")
    lines.append("")

    lines.append("## 4. Error Taxonomy & Systematic Failure Analysis")
    lines.append("| Sample ID | Dataset | Lang | Primary Failure | Reasons |")
    lines.append("|---|---|---|---|---|")
    for summary in eval_summaries:
        for r in summary["results"]:
            if r["failure"]["primary_category"] != "NONE":
                reasons_str = "; ".join(r["failure"]["reasons"])
                lines.append(f"| `{r['sample_id']}` | `{r['dataset_name']}` | `{r['language']}` | **{r['failure']['primary_category']}** | {reasons_str} |")
    lines.append("")

    with open(md_path, "w", encoding="utf-8") as f:
        f.write("\n".join(lines))
    print(f"[REPORT] Markdown report written to {md_path}")


def main():
    parser = argparse.ArgumentParser(description="Run Vox Public Speech Corpora Benchmark.")
    parser.add_argument("--recognizer", choices=["whisper-small", "whisper-large-v3-turbo", "parakeet"], default="whisper-small")
    parser.add_argument("--model", type=str, default=None, help="Custom model path")
    parser.add_argument("--manifest", type=str, default=None, help="Specific manifest filename in manifests/ to run")
    parser.add_argument("--max-samples", type=int, default=None, help="Max samples per manifest")
    args = parser.parse_args()

    bin_path = find_benchmark_binary()
    model_path = Path(args.model) if args.model else MODEL_MAP[args.recognizer]

    if not model_path.is_file():
        if args.recognizer == "parakeet":
            print(f"[ERROR] Parakeet ONNX model not found at {model_path}.")
            print("Parakeet is currently built into Vox as a feature-gated dictation engine, but ONNX weights are not installed on this workstation.")
            sys.exit(2)
        else:
            raise FileNotFoundError(f"Model file not found: {model_path}")

    manifest_files = [MANIFESTS_DIR / args.manifest] if args.manifest else sorted(list(MANIFESTS_DIR.glob("*.jsonl")))

    eval_summaries = []
    for mf in manifest_files:
        summary = evaluate_manifest(mf, bin_path, model_path, args.recognizer, args.max_samples)
        eval_summaries.append(summary)

    generate_reports(eval_summaries, args.recognizer, model_path, REPORTS_DIR)


if __name__ == "__main__":
    main()
