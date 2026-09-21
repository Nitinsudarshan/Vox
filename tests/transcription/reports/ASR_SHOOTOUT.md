# VOX ASR SHOOTOUT

**Date:** 2026-09-20T22:03:59.956214+00:00
**Corpus:** corpus-v1
**Vox commit:** `1305bac-dirty`
**Host:** Linux 6.18.44-fc-v37 · x86_64 · x86_64 · 4 logical cores · ? GB RAM
**Binary:** benchmark (debug)

> **These timings are from a debug build.** Cargo builds whisper.cpp with optimizations either way, but everything around it — the segmenter, the resampler, the quality gate — is unoptimized. Rebuild with `--release` before comparing throughput.

## Audio gate

Checked before any model, because a clipped recording cannot separate two of them.

| Case | Duration | RMS | Peak | Near clipping | Voiced | SNR est. | Verdict |
|---|---|---|---|---|---|---|---|
| `en_clean_sprint` | 341s | 0.9473 | 1.000 | 89.75% | 0.0% | 1.3 dB | **2 concern(s)** |
| `en_fast_standup` | 290s | 0.9676 | 1.000 | 93.62% | 0.0% | 0.0 dB | **2 concern(s)** |
| `en_tech_domain` | 404s | 0.9641 | 1.000 | 92.96% | 0.0% | 0.4 dB | **2 concern(s)** |
| `hi_clean_review` | 407s | 0.9627 | 1.000 | 92.69% | 0.0% | 0.5 dB | **2 concern(s)** |
| `hi_meeting_dialogue` | 371s | 0.9419 | 1.000 | 88.72% | 0.0% | 3.4 dB | **2 concern(s)** |
| `hinglish_clean_sync` | 392s | 0.9629 | 1.000 | 92.73% | 0.0% | 0.3 dB | **2 concern(s)** |
| `hinglish_tech_names` | 371s | 0.9465 | 1.000 | 89.60% | 0.0% | 2.7 dB | **2 concern(s)** |
| `hinglish_noisy_ops` | 377s | 0.9997 | 1.000 | 99.92% | 0.0% | 0.0 dB | **2 concern(s)** |

- `en_clean_sprint`: 89.8% of samples are at or beyond 98% of full scale — the recording is clipped, and distorted phonemes are not a model's fault
- `en_clean_sprint`: estimated SNR 1.3 dB leaves little room between speech and background
- `en_fast_standup`: 93.6% of samples are at or beyond 98% of full scale — the recording is clipped, and distorted phonemes are not a model's fault
- `en_fast_standup`: estimated SNR 0.0 dB leaves little room between speech and background
- `en_tech_domain`: 93.0% of samples are at or beyond 98% of full scale — the recording is clipped, and distorted phonemes are not a model's fault
- `en_tech_domain`: estimated SNR 0.4 dB leaves little room between speech and background
- `hi_clean_review`: 92.7% of samples are at or beyond 98% of full scale — the recording is clipped, and distorted phonemes are not a model's fault
- `hi_clean_review`: estimated SNR 0.5 dB leaves little room between speech and background
- `hi_meeting_dialogue`: 88.7% of samples are at or beyond 98% of full scale — the recording is clipped, and distorted phonemes are not a model's fault
- `hi_meeting_dialogue`: estimated SNR 3.4 dB leaves little room between speech and background
- `hinglish_clean_sync`: 92.7% of samples are at or beyond 98% of full scale — the recording is clipped, and distorted phonemes are not a model's fault
- `hinglish_clean_sync`: estimated SNR 0.3 dB leaves little room between speech and background
- `hinglish_tech_names`: 89.6% of samples are at or beyond 98% of full scale — the recording is clipped, and distorted phonemes are not a model's fault
- `hinglish_tech_names`: estimated SNR 2.7 dB leaves little room between speech and background
- `hinglish_noisy_ops`: 99.9% of samples are at or beyond 98% of full scale — the recording is clipped, and distorted phonemes are not a model's fault
- `hinglish_noisy_ops`: estimated SNR 0.0 dB leaves little room between speech and background

## Results

No row was scored. Every entry below says why.

## Not run

| Case | Engine | Status | Reason |
|---|---|---|---|
| `en_clean_sprint` | - | refused | 89.8% of samples are at or beyond 98% of full scale — the recording is clipped, and distorted phonemes are not a model's fault; estimated SNR 1.3 dB leaves little room between speech and background |
| `en_fast_standup` | - | refused | 93.6% of samples are at or beyond 98% of full scale — the recording is clipped, and distorted phonemes are not a model's fault; estimated SNR 0.0 dB leaves little room between speech and background |
| `en_tech_domain` | - | refused | 93.0% of samples are at or beyond 98% of full scale — the recording is clipped, and distorted phonemes are not a model's fault; estimated SNR 0.4 dB leaves little room between speech and background |
| `hi_clean_review` | - | refused | 92.7% of samples are at or beyond 98% of full scale — the recording is clipped, and distorted phonemes are not a model's fault; estimated SNR 0.5 dB leaves little room between speech and background |
| `hi_meeting_dialogue` | - | refused | 88.7% of samples are at or beyond 98% of full scale — the recording is clipped, and distorted phonemes are not a model's fault; estimated SNR 3.4 dB leaves little room between speech and background |
| `hinglish_clean_sync` | - | refused | 92.7% of samples are at or beyond 98% of full scale — the recording is clipped, and distorted phonemes are not a model's fault; estimated SNR 0.3 dB leaves little room between speech and background |
| `hinglish_tech_names` | - | refused | 89.6% of samples are at or beyond 98% of full scale — the recording is clipped, and distorted phonemes are not a model's fault; estimated SNR 2.7 dB leaves little room between speech and background |
| `hinglish_noisy_ops` | - | refused | 99.9% of samples are at or beyond 98% of full scale — the recording is clipped, and distorted phonemes are not a model's fault; estimated SNR 0.0 dB leaves little room between speech and background |

## Diagnosis detail

Nothing was scored, so nothing is diagnosed.