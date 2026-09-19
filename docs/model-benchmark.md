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

| Model Candidate | English WER (%) | Hindi CER (%) | Hinglish WER (%) | Domain Accuracy (%) | Decode RTF (CPU) | Peak Memory (MB) | Recommended Scenario |
|---|---|---|---|---|---|---|---|
| **Whisper Small (Baseline)** | Baseline | Baseline | Baseline | Baseline | Baseline (~0.25) | ~480 MB | Standard laptops, universal default |
| **Whisper Large-v3-Turbo** | Measured | Measured | Measured | Measured | Measured | ~1,200 MB | High-end desktops, complex accents |
| **Hindi2Hinglish Apex** | Measured | Measured | Measured | Measured | Measured | ~850 MB | Dedicated Indian bilingual deployments |
| **Parakeet Multilingual** | Measured | Measured | Measured | Measured | Measured | ~700 MB | Ultra-low-latency real-time dictation |

---

## 4. Analysis of Trade-Offs

1. **Accuracy vs. Latency (RTF):**
   - In background meeting transcription, RTF $< 0.8$ is acceptable because decoding runs asynchronously during the call.
   - For push-to-talk dictation, RTF must remain $< 0.3$ to preserve user flow.
2. **Memory Footprint:**
   - Vox targets standard Windows laptops with 8 GB to 16 GB of system RAM. Models requiring $> 2$ GB resident memory can trigger paging during concurrent video calls (e.g. Zoom, Teams).
3. **Code-Switching Fidelity:**
   - Standard multilingual models often bias toward pure English or pure Hindi script. Specialized domain vocabulary conditioning bridges this gap, but fine-tuned models may provide superior phonetic grounding for mixed utterances.
