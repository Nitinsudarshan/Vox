# VOX PUBLIC SPEECH CORPORA BENCHMARK REPORT (V1)
**Date:** 2026-09-19T18:42:11.594966+00:00
**Recognizer:** `whisper-small` (`ggml-small.bin`)
**Evaluation Scope:** Mozilla Common Voice (English Standard & Spontaneous) + Google FLEURS (10 Indian Languages)
**Pipeline Mode:** Production Meeting Pipeline (Segmenter + VAD/Queue + Whisper + Quality Gate + Recovery Pass)

> [!IMPORTANT]
> Per benchmark fairness specifications, each public dataset and language is reported individually.
> Multilingual results are **NOT** collapsed into a single misleading aggregate number.

## 1. Mozilla Common Voice Results
| Split / Profile | Manifest | Lang | Samples | Ref Words | WER (%) | CER (%) | Avg RTF |
|---|---|---|---|---|---|---|---|
| `commonvoice-en-spontaneous-v1.jsonl` | `commonvoice-en-spontaneous-v1.jsonl` | `en` | 1 | 20 | **10.00%** | 10.19% | 2.507 |
| `commonvoice-en-v1.jsonl` | `commonvoice-en-v1.jsonl` | `en` | 1 | 10 | **0.00%** | 0.00% | 5.171 |

## 2. Google FLEURS Results (10 Indian Languages)
| Language | Code | Manifest | Samples | Ref Words | WER (%) | CER (%) | Avg RTF |
|---|---|---|---|---|---|---|---|
| BN | `bn` | `fleurs-bn-v1.jsonl` | 1 | 12 | **125.00%** | 108.22% | 7.539 |
| GU | `gu` | `fleurs-gu-v1.jsonl` | 1 | 13 | **100.00%** | 113.64% | 6.677 |
| HI | `hi` | `fleurs-hi-v1.jsonl` | 1 | 16 | **100.00%** | 105.97% | 4.808 |
| KN | `kn` | `fleurs-kn-v1.jsonl` | 1 | 11 | **100.00%** | 119.48% | 9.592 |
| ML | `ml` | `fleurs-ml-v1.jsonl` | 1 | 12 | **125.00%** | 102.97% | 7.567 |
| MR | `mr` | `fleurs-mr-v1.jsonl` | 1 | 12 | **133.33%** | 117.86% | 6.724 |
| PA | `pa` | `fleurs-pa-v1.jsonl` | 1 | 18 | **105.56%** | 115.07% | 6.413 |
| TA | `ta` | `fleurs-ta-v1.jsonl` | 1 | 11 | **63.64%** | 15.91% | 15.288 |
| TE | `te` | `fleurs-te-v1.jsonl` | 1 | 11 | **163.64%** | 112.90% | 9.151 |
| UR | `ur` | `fleurs-ur-v1.jsonl` | 1 | 17 | **5.88%** | 1.49% | 7.880 |

## 3. Dataset-Level Aggregates (Reported Separately)
| Dataset | Total Samples | Total Words | Mean WER (%) | Mean CER (%) | Mean RTF |
|---|---|---|---|---|---|
| **Mozilla Common Voice** | 2 | 30 | **5.00%** | 5.09% | 3.839 |
| **Google FLEURS (10 Languages)** | 10 | 133 | **102.20%** | 91.35% | 8.164 |

## 4. Error Taxonomy & Systematic Failure Analysis
| Sample ID | Dataset | Lang | Primary Failure | Reasons |
|---|---|---|---|---|
| `fleurs_bn_01` | `fleurs` | `bn` | **LANGUAGE** | High error rate on language 'bn' indicates limited multilingual acoustic representation or tokenization gap. |
| `fleurs_gu_01` | `fleurs` | `gu` | **LANGUAGE** | High error rate on language 'gu' indicates limited multilingual acoustic representation or tokenization gap. |
| `fleurs_hi_01` | `fleurs` | `hi` | **LANGUAGE** | High error rate on language 'hi' indicates limited multilingual acoustic representation or tokenization gap. |
| `fleurs_kn_01` | `fleurs` | `kn` | **LANGUAGE** | High error rate on language 'kn' indicates limited multilingual acoustic representation or tokenization gap. |
| `fleurs_ml_01` | `fleurs` | `ml` | **LANGUAGE** | High error rate on language 'ml' indicates limited multilingual acoustic representation or tokenization gap. |
| `fleurs_mr_01` | `fleurs` | `mr` | **HALLUCINATION** | Excessive insertions detected indicating hallucination loops or phantom tokens. |
| `fleurs_pa_01` | `fleurs` | `pa` | **LANGUAGE** | High error rate on language 'pa' indicates limited multilingual acoustic representation or tokenization gap. |
| `fleurs_ta_01` | `fleurs` | `ta` | **LANGUAGE** | High error rate on language 'ta' indicates limited multilingual acoustic representation or tokenization gap. |
| `fleurs_te_01` | `fleurs` | `te` | **HALLUCINATION** | Excessive insertions detected indicating hallucination loops or phantom tokens. |
