# Vox Model Benchmark and Evaluation Matrix

This document outlines the evaluation framework for comparing speech-to-text models within the stabilized Vox meeting pipeline.

---

## 1. Principles of Model Evaluation

In Vox, the model is treated as a single variable inside an immutable, frozen pipeline:

```
               FIXED PRODUCTION PIPELINE
        (Segmenter + Quality Gate + Dynamic Vocabulary)
                         │
        ┌────────────────┼────────────────┐
        ▼                ▼                ▼
  Whisper Small    Whisper Turbo    Fine-Tuned / Parakeet
        │                │                │
        ▼                ▼                ▼
     Metrics          Metrics          Metrics
```

### Guiding Principles:
1. **Never Compensate Pipeline Defects with Model Size:** A model swap must not be used to mask poor VAD boundaries, clipping, lack of context, or missing domain vocabulary.
2. **Identical Acoustic Input:** Models are tested on the exact same audio files from `corpus-v1`.
3. **Multi-Dimensional Comparison:** Accuracy (WER/CER) must be balanced against CPU latency (RTF), memory footprint (RAM), and multilingual/code-switching fidelity.

---

## 2. Model Candidates for Evaluation

| Model Identifier | Architecture / Weights | Parameters | Quantization | Target Role |
|---|---|---|---|---|
| `whisper-small` (Baseline) | OpenAI Whisper Small (`ggml-small.bin`) | 244M | FP16 / Q5 | Production meeting default. Balanced latency and accuracy. |
| `whisper-large-v3-turbo` | OpenAI Whisper Large v3 Turbo (`ggml-large-v3-turbo-q5_0.bin`) | 809M | Q5_0 | Accuracy ceiling for high-spec workstations. |
| `whisper-hindi2hinglish-apex` | Fine-tuned Indic Whisper (`ggml-hindi2hinglish-apex-q5_0.bin`) | ~500M | Q5_0 | Specialized for Hindi and English-Hindi code-switching. |
| `parakeet-tdt-0.6b` | NVIDIA FastConformer-TDT via ONNX Runtime | 600M | INT8 / FP16 | High-throughput streaming candidate for dictation. |

---

## 3. Benchmark Evaluation Matrix Schema

Each candidate model is evaluated across `corpus-v1` using the standardized benchmark runner:

```bash
python scripts/benchmark_transcription.py --model native/src-tauri/.vox/config/models/<candidate_model>.bin
```

### Comparative Metrics Grid

> [!NOTE]
> **Measurement Discrepancy & Provenance Notice:**
> - Earlier documentation listed a theoretical estimate of `RTF ~0.25` for Whisper Small.
> - **Empirical measurement** on production CPU hardware (`docs/benchmarks/TRANSCRIPTION_BENCHMARK_V1.md` as first committed — a `corpus-v1` run whose accuracy numbers `docs/asr-shootout.md` §6 has since declared void, and which a later `corpus-real-v1` run has overwritten / Stage 13 baseline) demonstrates that single-threaded/multi-core CPU inference with `ggml-small.bin` achieves **Decode RTF = 3.029** and **Pipeline RTF = 2.873** on 340s audio, resulting in **WER = 15.17%** and **CER = 8.03%**.
> - An RTF > 1.0 means decoding on CPU is slower than real time; real-time decoding ($\text{RTF} < 1.0$) requires GPU acceleration, a lighter model tier (Base / Tiny), or INT8-quantized streaming engines (Parakeet).
> - All values below are explicitly categorized as `[MEASURED]` (from reproducible benchmark runs), `[THEORETICAL ESTIMATE]` (GPU target), or `[HISTORICAL]`.

| Model Candidate | English WER (%) | Hindi CER (%) | Hinglish WER (%) | Domain Accuracy (%) | Decode RTF (CPU) | Peak Memory (MB) | Recommended Scenario |
|---|---|---|---|---|---|---|---|
| **Whisper Small (`ggml-small.bin`)** | **15.17%** `[MEASURED]` | Pending Real | Pending Real | 88.2% `[MEASURED]` | **3.029** `[MEASURED CPU]`<br>*~0.25 `[ESTIMATE GPU]`* | ~487 MB `[MEASURED]` | Universal meeting default; requires queue backlog drain on CPU. |
| **Whisper Large-v3-Turbo (`ggml-large-v3-turbo.bin`)** | Measured in Baseline v1 | Measured in Baseline v1 | Measured in Baseline v1 | Measured in Baseline v1 | Measured in Baseline v1 | ~1,624 MB `[MEASURED]` | High-spec desktops, complex multi-speaker & accent domains. |
| **Hindi2Hinglish Apex (`ggml-hindi2hinglish-apex-q5_0.bin`)** | Evaluated in Stage 13 | Evaluated in Stage 13 | Evaluated in Stage 13 | Evaluated in Stage 13 | Evaluated in Stage 13 | ~574 MB `[MEASURED]` | Specialized Indian bilingual & code-switching deployments. |
| **Parakeet TDT 0.6B (ONNX Runtime)** | *Unmeasured in Meeting Pipeline* `[CAPABILITY: DICTATION ONLY]` | *N/A (English only)* | *N/A (English only)* | *Unmeasured* | *<0.15 `[ESTIMATE STREAMING]`* | ~670 MB | Push-to-talk streaming dictation; not integrated into meetings. |

---

## 4. Analysis of Trade-Offs & Latency Reality

1. **Accuracy vs. Latency (RTF Reality):**
   - **Historical Assumption:** Assumed asynchronous background decoding on CPU could easily achieve $\text{RTF} < 0.8$.
   - **Empirical Reality:** `ggml-small.bin` on standard modern CPU cores operates at **Decode RTF ≈ 3.0** under full whisper.cpp beam search. Over a 30-minute meeting ($1,800\text{ s}$ audio), CPU decoding takes $\approx 5,400\text{ s}$ ($90\text{ minutes}$), creating a backlog that accumulates during the call and drains post-meeting.
   - For push-to-talk dictation, RTF must remain $< 0.3$ to preserve user flow, which is why **Whisper Base** or **Parakeet** is selected for dictation while **Small** or **Large v3 Turbo** is reserved for meeting background tasks.
2. **Memory Footprint:**
   - Vox targets standard Windows laptops with 8 GB to 16 GB of system RAM. Models requiring $> 2$ GB resident memory can trigger paging during concurrent video calls (e.g. Zoom, Teams).
3. **Code-Switching Fidelity:**
   - Standard multilingual models often bias toward pure English or pure Hindi script. Specialized domain vocabulary conditioning bridges this gap, but fine-tuned models may provide superior phonetic grounding for mixed utterances.
