# Vox Meeting Transcription Pipeline Architecture

This document specifies the authoritative, deterministic architecture of the Vox long-form meeting transcription pipeline.

---

## 1. High-Level Pipeline Overview

The meeting transcription pipeline processes incoming microphone and system loopback streams in real time, segments speech adaptively, manages dynamic domain context, decodes utterances serially, screens output through a structured quality gate, executes deterministic recovery for suspicious segments, and produces an auditable canonical transcript with comprehensive diagnostic telemetry.

```
Microphone Stream (CPAL)
         +
System Loopback Stream (CPAL)
         │
         ▼
[ 1. Capture & Synchronization ]
  • Sample rate conversion to 16 kHz mono float32
  • Inter-channel drift compensation (< 10 ms)
  • Acoustic clipping detection & RMS energy computation
         │
         ▼
[ 2. Adaptive Speech Segmentation (Segmenter) ]
  • Sliding frame energy analysis (20 ms window)
  • Adaptive noise floor tracking (3-second percentile buffer)
  • Pre-roll preservation (200 ms) & redemption tail
  • Adaptive silence threshold (700 ms natural pause close)
  • Max duration ceiling (25.0s) with `forced_split = true`
         │
         ▼
[ 3. Bounded Work Queue (TranscriptionQueue) ]
  • Capacity: 64 segments (~100 MB / ~25 min audio headroom)
  • Sequence numbering assigned at submission (auditable drop detection)
  • Queue backlog latency telemetry
         │
         ▼
[ 4. Dynamic Context & Vocabulary Conditioning (DomainVocabulary) ]
  • Multi-tier vocabulary: Global domain terms + Meeting metadata + User custom dictionary
  • Syntactic tail extraction from previous valid segment (`prev_kept_text`)
  • Hallucination quarantine: context cleared on rejected/discarded segments
         │
         ▼
[ 5. Serial ASR Decoding (Whisper Engine) ]
  • Model: ggml-small.bin (production baseline)
  • Explicit decoding configuration: greedy / temperature fallback, entropy & logprob thresholds
  • Dual-profile decoding: careful baseline vs expensive-script profile for non-Latin scripts
         │
         ▼
[ 6. Transcript Quality Gate & Telemetry ]
  • Compression ratio calculation via zlib deflate (< 2.4 threshold)
  • No-speech probability & average log-probability evaluation
  • Repetition detection & hallucination reason classification
  • Quality status categorization: Good, Suspicious, Rejected
         │
         ▼
[ 7. Deterministic Recovery (For Suspicious Segments) ]
  • Strip conditioning prompt & reset temperature to 0.0
  • Alternate greedy decode attempt
  • Re-screen through quality gate; accept if recovered, discard if still defective
         │
         ▼
[ 8. Canonical Transcript & Persistence (MeetingStore) ]
  • Immutable transcript.json with full SegmentTelemetry
  • Decoupled Romanization & Translation pipelines
  • Structured summary & evidence extraction
```

---

## 2. Audio Capture, Synchronization, and Channel Preservation

1. **Format Standardization:**
   - All capture streams are resampled to 16,000 Hz, 1-channel (mono), 32-bit floating point PCM via high-quality sinc interpolation.
2. **Channel Attribution:**
   - Both microphone and system loopback audio streams are tracked. During frame processing, relative energy levels are compared (`CHANNEL_DOMINANCE_RATIO = 3.0`):
     - `SegmentChannel::Mic`: Local speaker.
     - `SegmentChannel::System`: Remote meeting participants.
     - `SegmentChannel::Mixed`: Concurrent speech or acoustic crosstalk.
3. **Signal Health Diagnostics:**
   - Each closed segment computes `AudioStats`:
     - `rms`: Root-mean-square amplitude proxy.
     - `peak_amplitude`: Absolute maximum sample value.
     - `near_clipping_percent`: Percentage of samples exceeding `|s| >= 0.99`.
     - `has_non_finite`: Guard against NaN or Inf values produced by faulty audio drivers.

---

## 3. Adaptive Speech Segmentation (`Segmenter`)

Vox rejects fixed 30-second chunking in favor of active-speech-based adaptive segmentation:
- **Frame Duration:** 20 ms (`FRAME_SAMPLES = 320` at 16 kHz).
- **Noise Floor Estimation:** Continuous rolling 3-second window (`NOISE_WINDOW_FRAMES = 150`), tracking the 20th percentile of frame energy as the ambient acoustic floor.
- **Onset Detection:** 3 consecutive frames (`ONSET_FRAMES = 3`) exceeding `floor + ONSET_MARGIN (0.008)` opens a segment.
- **Pre-Roll Window:** The last 10 frames (200 ms) are held in a circular buffer and prepended to opened segments, preserving the initial consonants of words.
- **Natural Pause Close:** Utterances remain open through short thinking pauses, closing only when silence exceeds 700 ms.
- **Forced Split Protection:** If speech is continuous without a natural pause, the segment is cleanly split at `MAX_SEGMENT_SECONDS = 25.0s`. Crucially, metadata `forced_split = true` is set, signalling downstream components that the utterance continues across the boundary.

---

## 4. Multi-Tier Dynamic Context Conditioning (`DomainVocabulary`)

Whisper's decoder is heavily influenced by preceding prompt tokens. The `DomainVocabulary` engine synthesizes dynamic prompts:
1. **Tier 1 — Global Domain Terms:**
   - Invariant institutional keywords: `NavGurukul`, `SOSC`, `NGConnect`, `GHAR`, `Sama Saathi`, `Ares`, `Macquarie`, `Kishore`, `Abhishek`.
   - Core systems terms: `Tauri`, `Rust`, `Whisper`, `Supabase`, `whisper.cpp`, `CPAL`.
2. **Tier 2 — Meeting Metadata:**
   - Ingested dynamically from calendar invitations, meeting titles, agendas, and participant rosters.
3. **Tier 3 — User Custom Dictionary:**
   - Maintained by individual users for specialized project code-names.
4. **Context Propagation & Hallucination Quarantine:**
   - On clean segments (`quality_status = Good`), the clean tail of the transcribed text is stored as `prev_kept_text` and prepended to the prompt for the next segment.
   - If a segment is flagged as `Suspicious`, `Rejected`, or `Discarded`, `prev_kept_text` is immediately cleared to prevent hallucination loops from infecting subsequent speech.

---

## 5. Structured Quality Gate & Telemetry

Never assume Whisper returned valid text. Every decoded segment is screened through `evaluate_segment_quality`:
- **High Compression Ratio:** Text with repetitive patterns is compressed using zlib deflate. If `len(raw) / len(compressed) >= 2.4`, it is flagged as `HighCompressionRatio`.
- **No-Speech Probability:** Whisper internal probability > 0.85 on short utterances flags phantom text on background silence.
- **Output-to-Audio Disproportion:** Extreme text length on very short audio (< 1.5s speech producing > 100 characters).
- **Quality Status Categories:**
  - `Good`: Accepted and emitted to UI and store.
  - `Suspicious`: Dispatched to deterministic recovery.
  - `Rejected`: Discarded from canonical transcript; reason logged to telemetry.

### Telemetry Schema per Segment
Every `TranscriptSegment` contains:
```json
{
  "sequence": 14,
  "start_seconds": 45.2,
  "end_seconds": 58.7,
  "duration": 13.5,
  "speech_duration": 12.8,
  "channel": "Microphone",
  "detected_language": "hi",
  "decode_language": "hi",
  "model": "ggml-small.bin",
  "decode_profile": "expensive_script",
  "decode_ms": 1420,
  "real_time_factor": 0.111,
  "queue_wait_ms": 45,
  "no_speech_prob": 0.02,
  "compression_ratio": 1.34,
  "quality_status": "good",
  "retry_count": 0,
  "forced_split": false,
  "audio_rms": 0.082,
  "audio_peak": 0.65,
  "audio_clipping_percent": 0.0
}
```

---

## 6. Deterministic Recovery Strategy

When a segment is flagged as `Suspicious`:
1. Strips conditioning prompts and historical context that may have primed the model into a hallucination loop.
2. Clamps decoding temperature to `0.0` (pure greedy search).
3. Re-runs inference.
4. If the recovery output passes the quality gate and achieves lower no-speech probability, it is accepted with `quality_status = "recovered"`. Otherwise, it is discarded with `quality_status = "rejected"`.

---

## 7. Canonical Transcript and Downstream Intelligence

1. **Canonical Source:** The primary transcript is stored in original spoken script/language (English in Latin script, Hindi in Devanagari script, Hinglish code-switched verbatim).
2. **Transformations are Separate Concerns:**
   - Romanization is a downstream phonetic transformation, not a separate ASR decode pass.
   - Translation into English is an independent operation operating on canonical text.
   - Summarization operates on evidence extracted from the canonical transcript.
