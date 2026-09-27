# Vox Public Speech Corpora Benchmark Suite

This directory contains the reproducible public speech dataset evaluation layer for Vox.
It decouples **core model capability** and **Vox transcription pipeline behavior** from local hardware and microphone acoustic conditions.

## Supported Datasets

### 1. Mozilla Common Voice
- **Version:** `cv-corpus-17.0-2024-03-15`
- **Source:** [Mozilla Common Voice](https://commonvoice.mozilla.org/en/datasets)
- **License:** CC0-1.0 Public Domain
- **Evaluation Splits:**
  - `manifests/commonvoice-en-v1.jsonl`: Clean read English utterances across diverse validated speakers.
  - `manifests/commonvoice-en-spontaneous-v1.jsonl`: Spontaneous and conversational English utterances with high speaker diversity, varied accents, and conversational cadences.

### 2. Google FLEURS (Few-shot Learning Evaluation of Universal Representations of Speech)
- **Version:** `fleurs-v1.0`
- **Source:** [Hugging Face google/fleurs](https://huggingface.co/datasets/google/fleurs)
- **License:** CC-BY-4.0
- **Supported Languages (10 Indian Languages):**
  - `fleurs-hi-v1.jsonl`: Hindi (`hi`)
  - `fleurs-ta-v1.jsonl`: Tamil (`ta`)
  - `fleurs-te-v1.jsonl`: Telugu (`te`)
  - `fleurs-kn-v1.jsonl`: Kannada (`kn`)
  - `fleurs-ml-v1.jsonl`: Malayalam (`ml`)
  - `fleurs-mr-v1.jsonl`: Marathi (`mr`)
  - `fleurs-bn-v1.jsonl`: Bengali (`bn`)
  - `fleurs-gu-v1.jsonl`: Gujarati (`gu`)
  - `fleurs-pa-v1.jsonl`: Punjabi (`pa`)
  - `fleurs-ur-v1.jsonl`: Urdu (`ur`)

---

## Dataset Abstraction & Normalized Manifest Schema

Public datasets are represented via the `PublicSpeechSample` and `PublicSpeechDataset` abstractions in `tests/transcription/public/schema.py`. Each line in a `.jsonl` manifest conforms to the following schema:

```json
{
  "sample_id": "cv_en_3829104",
  "dataset_name": "common_voice",
  "dataset_version": "cv-corpus-17.0-2024-03-15",
  "language": "en",
  "split": "test",
  "audio_path": "audio/common_voice/en/cv_en_3829104.wav",
  "reference_transcript": "The architectural committee approved the distributed storage roadmap yesterday afternoon.",
  "source_url": "https://commonvoice.mozilla.org/en/datasets",
  "license": "CC0-1.0",
  "speaker_id": "spk_cv_en_01",
  "duration_seconds": 5.328,
  "sample_rate": 16000,
  "audio_checksum_sha256": "ea1513f7b2b7462fef5d0ae34eae7ab2583617613df266cdba8bdcb74de10c52",
  "metadata": {
    "accent": "United States English",
    "gender": "female",
    "age": "thirties",
    "up_votes": 4,
    "down_votes": 0,
    "domain": "general_technical"
  }
}
```

---

## Audio Storage & Git Policy

> [!IMPORTANT]
> Raw audio files are **NOT** committed to the Git repository.
> The directory `tests/transcription/public/audio/` is included in `.gitignore`.
> All manifests (`.jsonl`), preparation scripts, checksums, and documentation are tracked.

---

## Reproducing the Benchmark

### Step 1: Prepare Datasets & Audio Clips
Run the dataset preparation script to generate the fixed manifests and fetch/synthesize evaluation audio:
```bash
python tests/transcription/public/prepare_datasets.py
```

### Step 2: Run Public Benchmark Suite
To evaluate all public manifests through the Vox pipeline:
```bash
# Evaluate using Whisper Small
python tests/transcription/runner/run_public_benchmark.py --recognizer whisper-small

# Evaluate using Whisper Large-v3-Turbo
python tests/transcription/runner/run_public_benchmark.py --recognizer whisper-large-v3-turbo

# Evaluate a specific language or manifest
python tests/transcription/runner/run_public_benchmark.py --manifest fleurs-hi-v1.jsonl
```

### Step 3: Inspect Reports
Results are exported to:
- Markdown report: `docs/benchmarks/TRANSCRIPTION_BENCHMARK_PUBLIC_V1.md`
- Documentation report: `docs/public-speech-benchmark.md`
- Machine-readable JSON: `tests/transcription/reports/public/benchmark_report.json`
- Tabular CSV: `tests/transcription/reports/public/benchmark_report.csv`
