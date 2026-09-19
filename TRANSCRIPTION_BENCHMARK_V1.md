# VOX TRANSCRIPTION BENCHMARK REPORT
**Date:** 2026-09-18T11:14:11.592489+00:00
**Corpus:** corpus-v1 (5.7 min audio, 811 words)
**Pipeline Version:** 0.1.0 (Production Segmenter + Dynamic Domain Vocabulary + Quality Gate + Recovery)
**Model:** ggml-small.bin (`D:\Projects\Vox\native\src-tauri\.vox\config\models\ggml-small.bin`)

## Executive Summary
| Metric | English | Hindi | Hinglish | Overall |
|---|---|---|---|---|
| **Average WER** | 15.17% | 0.00% | 0.00% | **15.17%** |
| **Average CER** | 8.03% | 0.00% | 0.00% | **8.03%** |
| **Average RTF** | 3.029 | 0.000 | 0.000 | **3.029** |
| **Total Speech** | 323.1s | 0.0s | 0.0s | **323.1s** |

## Detailed Per-Test Results
| Test ID | Lang | Category | Words | Audio (s) | WER | CER | Subs | Dels | Inss | RTF | Status |
|---|---|---|---|---|---|---|---|---|---|---|---|
| `en_clean_sprint` | en | A_clean | 811 | 340.7s | 15.17% | 8.03% | 86 | 26 | 11 | 3.029 | DECODING |

## Code-Switching & Domain Vocabulary Analysis (Hinglish & Tech)
| Test ID | Hindi Retention | English Retention | Monitored Domain Terms | Domain Accuracy |
|---|---|---|---|---|

## Diagnostic Telemetry & Quality Gate Actions
| Test ID | Segments Kept | Segments Discarded | Suspicious/Recovered | Discard Rate |
|---|---|---|---|---|
| `en_clean_sprint` | 43 | 0 | 0 | 0.0% |

## Failure Analysis & Hypotheses
### `en_clean_sprint` — Category: DECODING
- **Observed Evidence:** Sub-optimal beam search or temperature fallback during ambiguous speech intervals.
- **Expected snippet:** *"Good morning everyone, let us kick off our bi-weekly engineering sprint review. Today our primary objective is to evaluate the status of our desktop client..."*
- **Output snippet:** *"Good morning everyone, let us kick off our bivirically engineering Sprint Review. Today our primary objective is to evaluate the status of our desktop client..."*