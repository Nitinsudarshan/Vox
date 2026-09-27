# Vox Long-Form Meeting Transcription Benchmark

This document defines the methodology, evaluation metrics, failure taxonomy, and test procedures for the Vox Automated Meeting Transcription Benchmark.

---

## 1. Objectives and Principles

The benchmark evaluates the end-to-end performance of Vox's meeting transcription pipeline under reproducible, deterministic conditions.

### Core Operating Rules
1. **Model Invariance During Pipeline Iteration:**
   - The production speech model (`ggml-small.bin`) remains constant during pipeline improvements so optimizations reflect genuine pipeline stabilization rather than model capacity changes.
2. **Fixed, Versioned Corpus (`corpus-v1`):**
   - Test audio and reference texts are frozen. The corpus is never dynamically modified or regenerated after pipeline changes.
3. **Statistical Significance via Long-Form Audio:**
   - Short phrases (5–20 words) introduce extreme statistical noise where a single misrecognized token skews error rates by 10%.
   - Every benchmark case in `corpus-v1` contains **700 to 1,000 words** (~800–900 words target), representing 4 to 6 minutes of continuous multi-utterance meeting speech.

---

## 2. Test Corpus Structure (`corpus-v1`)

The benchmark corpus consists of 8 standardized, realistic meeting scenarios spanning English, Hindi, and Hinglish:

| Test ID | Language | Category | Word Count | Duration | Primary Voice | Description |
|---|---|---|---|---|---|---|
| `en_clean_sprint` | English | A (Clean) | 793 words | ~341s | `en-US-JennyNeural` | Sprint review, architecture migration, memory footprint, cloud sync. |
| `en_fast_standup` | English | C (Fast) | 729 words | ~290s | `en-IN-PrabhatNeural` | Rapid standup (+25% speed), CI/CD failures, buffer underruns, hiring. |
| `en_tech_domain` | English | G (Domain) | 831 words | ~404s | `en-US-JennyNeural` | Domain vocabulary: NavGurukul, SOSC, NGConnect, GHAR, Sama Saathi, Ares, Macquarie, Kishore, Abhishek, Rust, Tauri, Supabase. |
| `hi_clean_review` | Hindi | A (Clean) | 865 words | ~407s | `hi-IN-MadhurNeural` | Devanagari clean review: institutional planning, educational budgets, lab upgrades. |
| `hi_meeting_dialogue`| Hindi | D (Meeting) | 814 words | ~371s | `hi-IN-SwaraNeural` | Conversational Devanagari meeting: operational challenges, solar power, logistics, community councils. |
| `hinglish_clean_sync` | Hinglish | F (Code-switch)| 806 words | ~392s | `hi-IN-MadhurNeural` | Intra-sentence code-switching (~45% Hindi, 55% English), audio synchronization, SQLite indexing. |
| `hinglish_tech_names` | Hinglish | G (Domain) | 741 words | ~371s | `hi-IN-SwaraNeural` | Technical Hinglish with NavGurukul, SOSC, GHAR, Kishore, Abhishek, Macquarie, database migration, React, Rust. |
| `hinglish_noisy_ops` | Hinglish | E (Noisy) | 740 words | ~377s | `hi-IN-MadhurNeural` | Realistic meeting scenario with added moderate acoustic room noise (SNR=22 dB), field logistics. |

**Total Corpus Scope:** 8 test cases, 6,319 reference words, 49.2 minutes of 16 kHz mono 16-bit PCM WAV audio.

---

## 3. Evaluation Metrics

### 1. Word Error Rate (WER)
Levenshtein edit distance on normalized word tokens:
$$\text{WER} = \frac{\text{Substitutions} + \text{Deletions} + \text{Insertions}}{\text{Reference Words}}$$
Text normalization converts text to lowercase, applies Unicode NFKC normalization, removes punctuation, and standardizes whitespace while preserving Devanagari and Latin character sets.

### 2. Character Error Rate (CER)
Levenshtein edit distance on individual characters:
$$\text{CER} = \frac{\text{Char Substitutions} + \text{Char Deletions} + \text{Char Insertions}}{\text{Reference Characters}}$$
CER is critical for Hindi (Devanagari) where word segmentation and conjunct ligature variations can artificially inflate word-level error rates despite high semantic fidelity.

### 3. Hinglish Code-Switching Accuracy
- **Hindi Word Retention (%):** Proportion of Hindi words in reference successfully recognized in hypothesis.
- **English Word Retention (%):** Proportion of English words in reference successfully recognized in hypothesis.
- **Domain Keyword Accuracy (%):** Recognition rate of monitored technical and proper noun terms (`NavGurukul`, `SOSC`, `NGConnect`, `GHAR`, `Kishore`, `Abhishek`, etc.).

### 4. Hallucination Rate
- **Insertion Proxy Rate (%):** Proportion of hypothesis words representing unaligned insertions.
- **Suspicious / Discarded Segments:** Count of segments flagged by the quality gate for high compression ratio or phantom decoding during silence.

### 5. Latency & Real-Time Factor (RTF)
- **Decode RTF:** $\frac{\text{Total Inference Duration (seconds)}}{\text{Total Speech Audio Duration (seconds)}}$
- **Pipeline RTF:** $\frac{\text{Total Wall Duration (seconds)}}{\text{Total Recording Audio Duration (seconds)}}$
An RTF $< 1.0$ indicates faster-than-real-time performance.

---

## 4. Failure Categorization Taxonomy

Every test case failure is systematically classified into one of the following root-cause categories:

1. **`AUDIO`:** Acoustic clipping, microphone gain anomalies, severe noise corruption.
2. **`VAD`:** Premature silence truncation or unclosed utterances resulting in high deletions.
3. **`SEGMENTATION`:** Hard cuts through words or syntax fragmentation caused by forced splits.
4. **`LANGUAGE`:** Language mismatch (e.g. Hindi decoded into English phonetic gibberish).
5. **`DECODING`:** Divergent beam search or temperature fallback instability.
6. **`HALLUCINATION`:** Excessive insertions, repetitive loops, compression ratio $\ge 2.4$.
7. **`VOCABULARY`:** Domain keywords and proper nouns phonetically substituted.
8. **`CONTEXT`:** Boundary errors across split segments due to missing prompt context.
9. **`MODEL`:** Acoustic model capacity limitation (high substitutions despite clean audio).
10. **`NONE`:** Successful transcription within acceptable tolerance.

---

## 5. Running the Benchmark

### Prerequisites
- Rust backend compiled with benchmark binary: `cargo build --release --bin benchmark` in `native/src-tauri`.
- Python 3.10+ with `edge-tts` and `miniaudio`.

### Executing Benchmark Suite
```bash
# Run complete corpus-v1 benchmark suite
python scripts/benchmark_transcription.py

# Or via Windows CMD / Bash wrappers
scripts/benchmark_transcription.cmd
./scripts/benchmark_transcription

# Run a specific subset of test cases
python scripts/benchmark_transcription.py --cases en_clean_sprint,hinglish_clean_sync

# Evaluate an alternative Whisper model
python scripts/benchmark_transcription.py --model path/to/ggml-large-v3-turbo-q5_0.bin
```

### Generated Reports
Reports are automatically written to:
- `docs/benchmarks/TRANSCRIPTION_BENCHMARK_V1.md` (the committed human-readable report; the same file is also written to `tests/transcription/reports/`)
- `tests/transcription/reports/benchmark_report.json` (machine-readable metrics)
- `tests/transcription/reports/benchmark_report.csv` (tabular metrics for charting)
