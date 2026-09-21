# VOX TRANSCRIPTION BENCHMARK REPORT
**Date:** 2026-09-19T18:23:08.566446+00:00
**Corpus:** corpus-real-v1 (6.7 min audio, 857 words)
**Pipeline Version:** 0.1.0 (Production Segmenter + Dynamic Domain Vocabulary + Quality Gate + Recovery)
**Model:** ggml-small.bin (`native\src-tauri\.vox\config\models\ggml-small.bin`)

## Executive Summary
| Metric | English | Hindi | Hinglish | Overall |
|---|---|---|---|---|
| **Average WER** | 11.67% | 0.00% | 0.00% | **11.67%** |
| **Average CER** | 5.08% | 0.00% | 0.00% | **5.08%** |
| **Average RTF** | 2.067 | 0.000 | 0.000 | **2.067** |
| **Total Speech** | 391.3s | 0.0s | 0.0s | **391.3s** |

## Detailed Per-Test Results
| Test ID | Lang | Category | Words | Audio (s) | WER | CER | Subs | Dels | Inss | RTF | Status |
|---|---|---|---|---|---|---|---|---|---|---|---|
| `real_tech_domain` | en | technical_vocabulary | 857 | 403.7s | 11.67% | 5.08% | 74 | 15 | 11 | 2.067 | NONE |

## Code-Switching & Domain Vocabulary Analysis (Hinglish & Tech)
| Test ID | Hindi Retention | English Retention | Monitored Domain Terms | Domain Accuracy |
|---|---|---|---|---|

## Diagnostic Telemetry & Quality Gate Actions
| Test ID | Segments Kept | Segments Discarded | Suspicious/Recovered | Discard Rate |
|---|---|---|---|---|
| `real_tech_domain` | 32 | 0 | 0 | 0.0% |

## Failure Analysis & Hypotheses