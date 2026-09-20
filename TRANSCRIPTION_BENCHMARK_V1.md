# VOX TRANSCRIPTION BENCHMARK REPORT
**Date:** 2026-09-18T11:14:11.592489+00:00
**Corpus:** corpus-v1 (5.7 min audio, 811 words)
**Pipeline Version:** 0.1.0 (Production Segmenter + Dynamic Domain Vocabulary + Quality Gate + Recovery)
**Model:** ggml-small.bin (`ggml-small.bin`)
**Cases:** 1 of 8 in the manifest — **not run:** `en_fast_standup`, `en_tech_domain`, `hi_clean_review`, `hi_meeting_dialogue`, `hinglish_clean_sync`, `hinglish_tech_names`, `hinglish_noisy_ops`

> ## ⚠ These results do not measure a model
>
> The audio gate failed on `en_clean_sprint`. A recording whose samples sit at full scale has had its formant peaks flattened, so the word error rates below are a measurement of the recording and not of the engine that transcribed it. Regenerate the corpus (`scripts/generate_transcription_corpus.py`) and run again.

## Executive Summary
| Metric | English | Hindi | Hinglish | Overall |
|---|---|---|---|---|
| **Cases** | 1 | 0 | 0 | **1** |
| **Average WER** | 15.17% | not run | not run | **15.17%** |
| **Average CER** | 8.03% | not run | not run | **8.03%** |
| **Average RTF** | 3.029 | not run | not run | **3.029** |
| **Total Speech** | 323.1s | not run | not run | **323.1s** |

## Detailed Per-Test Results
| Test ID | Lang | Decoded as | Category | Words | Audio (s) | WER | CER | Subs | Dels | Inss | RTF | Status |
|---|---|---|---|---|---|---|---|---|---|---|---|---|
| `en_clean_sprint` | en | en ×43 | A_clean | 811 | 340.7s | 15.17% | 8.03% | 86 | 26 | 11 | 3.029 | DECODING |

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