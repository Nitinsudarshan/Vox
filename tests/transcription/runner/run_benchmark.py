"""
Automated Benchmark Runner for Vox Meeting Transcription Pipeline.
Runs test cases from corpus-v1 through the production meeting pipeline, evaluates against references,
computes WER, CER, RTF, Code-switch accuracy, Hallucination rate, classifies failures, and outputs
Markdown, JSON, and CSV reports.
"""

import argparse
import csv
import datetime
import json
import os
import subprocess
import sys
from pathlib import Path

# Add project roots to path
SCRIPT_DIR = Path(__file__).resolve().parent
REPO_ROOT = SCRIPT_DIR.parents[2]
sys.path.insert(0, str(REPO_ROOT / "tests" / "transcription" / "evaluator"))

from metrics import (
    calculate_cer,
    calculate_codeswitch_accuracy,
    calculate_hallucination_rate,
    calculate_wer,
    classify_transcription_failure,
)

DEFAULT_CORPUS_MANIFEST = REPO_ROOT / "tests" / "transcription" / "corpus-v1" / "manifest.json"
DEFAULT_MODEL_PATH = REPO_ROOT / "native" / "src-tauri" / ".vox" / "config" / "models" / "ggml-small.bin"
REPORTS_DIR = REPO_ROOT / "tests" / "transcription" / "reports"

def find_benchmark_binary() -> Path:
    release_bin = REPO_ROOT / "native" / "src-tauri" / "target" / "release" / "benchmark.exe"
    debug_bin = REPO_ROOT / "native" / "src-tauri" / "target" / "debug" / "benchmark.exe"
    linux_release = REPO_ROOT / "native" / "src-tauri" / "target" / "release" / "benchmark"
    linux_debug = REPO_ROOT / "native" / "src-tauri" / "target" / "debug" / "benchmark"

    for candidate in [release_bin, linux_release, debug_bin, linux_debug]:
        if candidate.is_file():
            return candidate
    raise FileNotFoundError(
        f"Benchmark binary not found in target directories. Please run 'cargo build --bin benchmark' first."
    )

def run_single_case(
    bin_path: Path,
    model_path: Path,
    case: dict,
    audio_path: Path,
    ref_path: Path
) -> dict:
    case_id = case["id"]
    lang = case["language"]
    title = case.get("title", "")
    
    # Read ground truth reference
    with open(ref_path, "r", encoding="utf-8") as f:
        reference_text = f.read().strip()

    # Map language to whisper language setting
    whisper_lang = "en" if lang == "en" else ("hi" if lang == "hi" else "auto")
    
    temp_out = REPO_ROOT / "tests" / "transcription" / "reports" / f"temp_{case_id}.json"
    temp_out.parent.mkdir(parents=True, exist_ok=True)

    cmd = [
        str(bin_path),
        "--audio", str(audio_path),
        "--model", str(model_path),
        "--lang", whisper_lang,
        "--meeting-title", title,
        "--output", str(temp_out)
    ]

    print(f"\n[{case_id}] Running pipeline: lang={lang} (whisper={whisper_lang}) audio={audio_path.name}...")
    start_time = datetime.datetime.now()
    
    proc = subprocess.run(cmd, capture_output=True, text=True, encoding="utf-8")
    elapsed = (datetime.datetime.now() - start_time).total_seconds()
    
    if proc.returncode != 0:
        print(f"Error running benchmark on {case_id}: {proc.stderr}", file=sys.stderr)
        raise RuntimeError(f"Benchmark binary failed on {case_id}: {proc.stderr}")

    with open(temp_out, "r", encoding="utf-8") as f:
        pipeline_data = json.load(f)

    # Remove temp file
    try:
        temp_out.unlink()
    except Exception:
        pass

    hypothesis_text = pipeline_data.get("transcript", "").strip()
    telemetry_list = [s.get("telemetry") for s in pipeline_data.get("segments", []) if s.get("telemetry")]

    # Compute metrics
    wer_data = calculate_wer(reference_text, hypothesis_text)
    cer_data = calculate_cer(reference_text, hypothesis_text)
    codeswitch_data = calculate_codeswitch_accuracy(reference_text, hypothesis_text)
    hallucination_data = calculate_hallucination_rate(reference_text, hypothesis_text, telemetry_list)
    failure_diag = classify_transcription_failure(case, wer_data, cer_data, telemetry_list)

    print(f"  WER: {wer_data['wer_percent']}% | CER: {cer_data['cer_percent']}% | "
          f"RTF: {pipeline_data['decode_real_time_factor']:.3f} | Wall: {elapsed:.1f}s | "
          f"Failure: {failure_diag['primary_category']}")

    return {
        "id": case_id,
        "language": lang,
        "category": case.get("category", ""),
        "title": title,
        "audio_file": str(audio_path),
        "duration_seconds": pipeline_data["total_audio_duration_seconds"],
        "speech_seconds": pipeline_data["total_speech_seconds"],
        "inference_ms": pipeline_data["total_inference_ms"],
        "decode_rtf": pipeline_data["decode_real_time_factor"],
        "pipeline_rtf": pipeline_data["pipeline_real_time_factor"],
        "segments_decoded": pipeline_data["segments_decoded_count"],
        "segments_kept": pipeline_data["segments_kept_count"],
        "segments_discarded": pipeline_data["segments_discarded_count"],
        "reference": reference_text,
        "hypothesis": hypothesis_text,
        "wer": wer_data,
        "cer": cer_data,
        "codeswitch": codeswitch_data,
        "hallucination": hallucination_data,
        "failure_diagnosis": failure_diag,
        "segments": pipeline_data.get("segments", [])
    }

def generate_markdown_report(report_data: dict) -> str:
    md = []
    md.append(f"# VOX TRANSCRIPTION BENCHMARK REPORT")
    md.append(f"**Date:** {report_data['timestamp']}")
    md.append(f"**Corpus:** {report_data['corpus_version']} ({report_data['total_audio_minutes']:.1f} min audio, {report_data['total_words']} words)")
    md.append(f"**Pipeline Version:** 0.1.0 (Production Segmenter + Dynamic Domain Vocabulary + Quality Gate + Recovery)")
    md.append(f"**Model:** {report_data['model_name']} (`{report_data['model_path']}`)\n")

    md.append("## Executive Summary")
    md.append("| Metric | English | Hindi | Hinglish | Overall |")
    md.append("|---|---|---|---|---|")

    by_lang = report_data["language_summary"]
    en = by_lang.get("en", {})
    hi = by_lang.get("hi", {})
    hing = by_lang.get("hinglish", {})
    ov = report_data["overall_summary"]

    md.append(f"| **Average WER** | {en.get('avg_wer', 0.0):.2f}% | {hi.get('avg_wer', 0.0):.2f}% | {hing.get('avg_wer', 0.0):.2f}% | **{ov.get('avg_wer', 0.0):.2f}%** |")
    md.append(f"| **Average CER** | {en.get('avg_cer', 0.0):.2f}% | {hi.get('avg_cer', 0.0):.2f}% | {hing.get('avg_cer', 0.0):.2f}% | **{ov.get('avg_cer', 0.0):.2f}%** |")
    md.append(f"| **Average RTF** | {en.get('avg_rtf', 0.0):.3f} | {hi.get('avg_rtf', 0.0):.3f} | {hing.get('avg_rtf', 0.0):.3f} | **{ov.get('avg_rtf', 0.0):.3f}** |")
    md.append(f"| **Total Speech** | {en.get('total_speech_sec', 0.0):.1f}s | {hi.get('total_speech_sec', 0.0):.1f}s | {hing.get('total_speech_sec', 0.0):.1f}s | **{ov.get('total_speech_sec', 0.0):.1f}s** |\n")

    md.append("## Detailed Per-Test Results")
    md.append("| Test ID | Lang | Category | Words | Audio (s) | WER | CER | Subs | Dels | Inss | RTF | Status |")
    md.append("|---|---|---|---|---|---|---|---|---|---|---|---|")

    for c in report_data["cases"]:
        w = c["wer"]
        k = c["cer"]
        md.append(
            f"| `{c['id']}` | {c['language']} | {c['category']} | {w['ref_words']} | "
            f"{c['duration_seconds']:.1f}s | {w['wer_percent']:.2f}% | {k['cer_percent']:.2f}% | "
            f"{w['substitutions']} | {w['deletions']} | {w['insertions']} | "
            f"{c['decode_rtf']:.3f} | {c['failure_diagnosis']['primary_category']} |"
        )

    md.append("\n## Code-Switching & Domain Vocabulary Analysis (Hinglish & Tech)")
    md.append("| Test ID | Hindi Retention | English Retention | Monitored Domain Terms | Domain Accuracy |")
    md.append("|---|---|---|---|---|")
    for c in report_data["cases"]:
        if c["language"] == "hinglish" or c["category"] == "G_tech_domain":
            cs = c["codeswitch"]
            terms_str = ", ".join([f"{k}:{'✓' if v else '✗'}" for k, v in cs["domain_terms_monitored"].items()]) if cs["domain_terms_monitored"] else "N/A"
            md.append(f"| `{c['id']}` | {cs['hindi_retention_percent']:.1f}% | {cs['english_retention_percent']:.1f}% | {terms_str} | {cs['domain_accuracy_percent']:.1f}% |")

    md.append("\n## Diagnostic Telemetry & Quality Gate Actions")
    md.append("| Test ID | Segments Kept | Segments Discarded | Suspicious/Recovered | Discard Rate |")
    md.append("|---|---|---|---|---|")
    for c in report_data["cases"]:
        h = c["hallucination"]
        total = c["segments_kept"] + c["segments_discarded"]
        discard_rate = (c["segments_discarded"] / total * 100.0) if total > 0 else 0.0
        md.append(f"| `{c['id']}` | {c['segments_kept']} | {c['segments_discarded']} | {h['suspicious_segments_count']} | {discard_rate:.1f}% |")

    md.append("\n## Failure Analysis & Hypotheses")
    for c in report_data["cases"]:
        diag = c["failure_diagnosis"]
        if diag["primary_category"] != "NONE":
            md.append(f"### `{c['id']}` — Category: {diag['primary_category']}")
            for reason in diag["reasons"]:
                md.append(f"- **Observed Evidence:** {reason}")
            # Show a brief sample of diff
            ref_snippet = " ".join(c["reference"].split()[:25]) + "..."
            hyp_snippet = " ".join(c["hypothesis"].split()[:25]) + "..."
            md.append(f"- **Expected snippet:** *\"{ref_snippet}\"*")
            md.append(f"- **Output snippet:** *\"{hyp_snippet}\"*")

    return "\n".join(md)

def main():
    parser = argparse.ArgumentParser(description="Vox Automated Meeting Transcription Benchmark Runner")
    parser.add_argument("--corpus", type=str, default=str(DEFAULT_CORPUS_MANIFEST), help="Path to corpus manifest.json")
    parser.add_argument("--model", type=str, default=str(DEFAULT_MODEL_PATH), help="Path to Whisper model (.bin)")
    parser.add_argument("--cases", type=str, default=None, help="Comma-separated subset of case IDs to run")
    parser.add_argument("--out-dir", type=str, default=str(REPORTS_DIR), help="Output directory for reports")
    args = parser.parse_args()

    corpus_path = Path(args.corpus)
    model_path = Path(args.model)
    out_dir = Path(args.out_dir)
    out_dir.mkdir(parents=True, exist_ok=True)

    if not corpus_path.is_file():
        print(f"Error: Corpus manifest not found at {corpus_path}", file=sys.stderr)
        sys.exit(1)
    if not model_path.is_file():
        print(f"Error: Model not found at {model_path}", file=sys.stderr)
        sys.exit(1)

    bin_path = find_benchmark_binary()
    print(f"Using benchmark binary: {bin_path}")
    print(f"Using model: {model_path.name}")
    print(f"Using corpus manifest: {corpus_path}")

    with open(corpus_path, "r", encoding="utf-8") as f:
        manifest = json.load(f)

    selected_cases = args.cases.split(",") if args.cases else None
    cases_to_run = []
    for c in manifest["cases"]:
        if selected_cases is None or c["id"] in selected_cases:
            cases_to_run.append(c)

    print(f"Running benchmark on {len(cases_to_run)} test cases...")

    results = []
    for c in cases_to_run:
        lang = c["language"]
        case_id = c["id"]
        wav_path = REPO_ROOT / "tests" / "transcription" / manifest["version"] / lang / f"{case_id}.wav"
        ref_path = REPO_ROOT / "tests" / "transcription" / "references" / f"{case_id}.txt"

        if not wav_path.is_file():
            print(f"Warning: Audio file {wav_path} missing, skipping.")
            continue
        if not ref_path.is_file():
            print(f"Warning: Reference file {ref_path} missing, skipping.")
            continue

        res = run_single_case(bin_path, model_path, c, wav_path, ref_path)
        results.append(res)

    # Compute summaries
    def compute_stats(cases_subset):
        if not cases_subset:
            return {"avg_wer": 0.0, "avg_cer": 0.0, "avg_rtf": 0.0, "total_speech_sec": 0.0}
        total_w = sum(x["wer"]["wer_percent"] for x in cases_subset)
        total_c = sum(x["cer"]["cer_percent"] for x in cases_subset)
        total_r = sum(x["decode_rtf"] for x in cases_subset)
        total_s = sum(x["speech_seconds"] for x in cases_subset)
        n = len(cases_subset)
        return {
            "avg_wer": round(total_w / n, 2),
            "avg_cer": round(total_c / n, 2),
            "avg_rtf": round(total_r / n, 3),
            "total_speech_sec": round(total_s, 2),
            "count": n
        }

    lang_summary = {}
    for lang in ["en", "hi", "hinglish"]:
        subset = [r for r in results if r["language"] == lang]
        lang_summary[lang] = compute_stats(subset)

    overall_summary = compute_stats(results)

    total_words = sum(r["wer"]["ref_words"] for r in results)
    total_audio_sec = sum(r["duration_seconds"] for r in results)

    report_payload = {
        "timestamp": datetime.datetime.now(datetime.timezone.utc).isoformat(),
        "corpus_version": manifest["version"],
        "model_name": model_path.name,
        "model_path": str(model_path),
        "total_audio_minutes": round(total_audio_sec / 60.0, 2),
        "total_words": total_words,
        "overall_summary": overall_summary,
        "language_summary": lang_summary,
        "cases": results
    }

    # 1. Write JSON Report
    json_path = out_dir / "benchmark_report.json"
    with open(json_path, "w", encoding="utf-8") as f:
        json.dump(report_payload, f, indent=2)
    print(f"\n[REPORT] JSON saved to {json_path}")

    # 2. Write CSV Report
    csv_path = out_dir / "benchmark_report.csv"
    with open(csv_path, "w", newline="", encoding="utf-8") as f:
        writer = csv.writer(f)
        writer.writerow([
            "id", "language", "category", "words", "audio_seconds", "speech_seconds",
            "wer_percent", "cer_percent", "substitutions", "deletions", "insertions",
            "decode_rtf", "pipeline_rtf", "kept_segments", "discarded_segments",
            "suspicious_segments", "failure_category"
        ])
        for r in results:
            w = r["wer"]
            k = r["cer"]
            h = r["hallucination"]
            writer.writerow([
                r["id"], r["language"], r["category"], w["ref_words"], r["duration_seconds"],
                r["speech_seconds"], w["wer_percent"], k["cer_percent"], w["substitutions"],
                w["deletions"], w["insertions"], r["decode_rtf"], r["pipeline_rtf"],
                r["segments_kept"], r["segments_discarded"], h["suspicious_segments_count"],
                r["failure_diagnosis"]["primary_category"]
            ])
    print(f"[REPORT] CSV saved to {csv_path}")

    # 3. Write Markdown Report
    md_content = generate_markdown_report(report_payload)
    md_path = out_dir / "TRANSCRIPTION_BENCHMARK_V1.md"
    root_md_path = REPO_ROOT / "TRANSCRIPTION_BENCHMARK_V1.md"
    with open(md_path, "w", encoding="utf-8") as f:
        f.write(md_content)
    with open(root_md_path, "w", encoding="utf-8") as f:
        f.write(md_content)
    print(f"[REPORT] Markdown saved to {root_md_path}")

if __name__ == "__main__":
    main()
