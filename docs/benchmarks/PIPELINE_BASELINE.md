# Vox Meeting Transcription Pipeline Baseline (`PIPELINE_BASELINE.md`)

This document records the architecture, configuration, diagnostics, and baseline performance of the stabilized Vox meeting transcription pipeline. It acts as the immutable Phase 1 engineering baseline against which future pipeline refinements and alternative speech models are evaluated.

---

## 1. End-to-End Pipeline Architecture

The Vox meeting transcription pipeline operates through an asynchronous streaming topology designed to balance real-time responsiveness, temporal lockstep, code-switching support, and resilience against sequence-model hallucinations:

```
┌────────────────────────┐      ┌────────────────────────┐
│ Local Microphone (CPAL)│      │  WASAPI Loopback (Sys) │
└───────────┬────────────┘      └───────────┬────────────┘
            │                               │
            └───────────────┬───────────────┘
                            ▼
           ┌─────────────────────────────────┐
           │ Audio Capture & Lockstep Mixer  │
           │  - 16 kHz Mono Resampling       │
           │  - Per-channel Energy Tracking  │
           │  - Audio Health Diagnostics     │
           └────────────────┬────────────────┘
                            ▼
           ┌─────────────────────────────────┐
           │ Streaming VAD & Segmentation    │
           │  - 20 ms Adaptive Energy Window │
           │  - 300 ms Pre-Roll / 400 ms Tail│
           │  - Forced-Split Boundary Cut    │
           └────────────────┬────────────────┘
                            ▼
           ┌─────────────────────────────────┐
           │ Context Construction & Priming  │
           │  - Multi-tier Domain Vocabulary │
           │  - Clean Preceding Context Tail │
           │  - Forced-Split Continuation    │
           └────────────────┬────────────────┘
                            ▼
           ┌─────────────────────────────────┐
           │   ASR Decoding (Whisper ASR)    │
           │  - Explicit Parameter Control   │
           │  - Multilingual Auto-Detection  │
           │  - Dual Profile (Careful/Fast)  │
           └────────────────┬────────────────┘
                            ▼
           ┌─────────────────────────────────┐
           │ Structured Quality Gate & Retry │
           │  - Compression Ratio (Zlib)     │
           │  - Repetition Loop Detection    │
           │  - Alternate Recovery Attempt   │
           └────────────────┬────────────────┘
                            ▼
           ┌─────────────────────────────────┐
           │ Canonical Transcript & Variants │
           │  - Provenance & Telemetry       │
           │  - Devanagari Romanization FSM  │
           │  - Structured Evidence Extractor│
           └─────────────────────────────────┘
```

### Stage 1: Audio Capture & Synchronization (`meetings/capture.rs`)
- **Streams**: Opens local microphone and default loopback audio simultaneously via CPAL.
- **Lockstep Mixing**: Wakes every 20 ms (`MIXER_TICK`), consuming equal sample counts from microphone and system buffers. Prevents acoustic time drift using a 250 ms max stream lag ceiling (`MAX_STREAM_LAG_SAMPLES`).
- **Channel Attribution**: Maintains per-channel energy accumulators (`mic_sum_sq`, `sys_sum_sq`) alongside mixed audio to classify segments into `Microphone` ("You"), `System` ("Others"), or `Mixed` ("Speaker") without destructive early downmixing.

### Stage 2: Streaming Adaptive Segmentation (`meetings/segmenter.rs`)
- **Sample Rate**: 16,000 Hz mono PCM.
- **Analysis Window**: 20 ms frames (320 samples).
- **Pre-roll**: 300 ms (15 frames) buffered in a FIFO ring to preserve initial consonants and plosives.
- **Redemption**: 400 ms (20 frames) silence tolerance before closing an active speech turn to prevent sentence chopping.
- **Ceiling Split**: When a turn reaches 25 seconds (`MAX_SEGMENT_SECONDS`), the segmenter splits at the quietest frame in the tail half (`quietest_frame_in_tail`), preserving remaining frames for the next segment and setting `forced_split: true`.
- **Signal Health**: Every segment calculates `AudioStats` (RMS, peak amplitude, near-clipping percentage, and non-finite sample sanity).

### Stage 3: Domain Vocabulary & Context Construction (`capture/vocabulary.rs`)
- **Multi-Tier Vocabulary**:
  - Global domain vocabulary: curated organization and technical entities (`NavGurukul`, `SOSC`, `NGConnect`, `GHAR`, `Sama Saathi`, `Ares`, `Macquarie`, `Kishore`, `Abhishek`, `Tauri`, `Whisper`, `Rust`, `Supabase`, etc.).
  - Meeting-specific keywords (from meeting title, agenda, and participants).
  - User custom vocabulary (persisted dictionary).
- **Dynamic Context Prompt**: Priming budget is clamped to ~220 characters. For continuation segments (`forced_split`), the clean tail of the preceding segment is prioritized. Hallucinated or rejected segments are strictly prohibited from contributing to subsequent context prompts.

### Stage 4: Explicit Whisper Decoding Configuration (`capture/stt.rs`)
- Eliminates hidden configuration discrepancies across streaming, meeting, dictation, and batch paths.
- Parameters explicitly mapped to `FullParams`:
  - `sampling_strategy`: Greedy with `best_of = 1` or Beam Search with configurable beam size.
  - `temperature`: 0.0 with increment 0.2 (set to 0.0 on fast non-Latin profile).
  - `suppress_blank`: true; `print_special`: false; `print_timestamps`: false.
  - `no_speech_thold`: 0.60; `entropy_thold`: 2.40; `logprob_thold`: -1.0.
  - `single_segment`: enforced per window requirements.
  - `no_context`: configured to preserve cross-boundary conditioning where desired.
  - `audio_ctx`: dynamic encoder clamping via `audio_ctx_for_seconds` to avoid paying for empty silence on short utterances.
  - `n_threads`: clamped to available physical/logical cores.

### Stage 5: Structured Quality Gate & Deterministic Recovery (`capture/speech_health.rs`, `meetings/transcription.rs`)
- **Evaluation Criteria**:
  - `compression_ratio`: evaluated via `flate2::write::ZlibEncoder`. Ratios > 2.45 over >= 12 words trigger immediate hallucination rejection.
  - `repetition_loop`: dominant adjacent repeats >= 3 covering >= 85% of words.
  - `filler_over_silence`: boilerplate phrases over < 1.0s of voiced audio.
  - `implausible_rate`: word rate exceeding 8.0 words per voiced second.
  - `suspicious`: compression ratio >= 1.85, moderate no-speech probability (0.50–0.85), word rate > 5.5, or presence of Unicode replacement characters (`\u{FFFD}`).
- **Deterministic Recovery**:
  - If a segment is classified as `Suspicious`, the worker immediately executes an alternate recovery decode pass using greedy search (`temperature = 0.0`), zero temperature increment, and a cleared initial prompt to break repetitive conditioning.
  - If the recovery pass yields a `Good` quality result or achieves lower no-speech probability, it is accepted (`quality_status = "recovered"`).

### Stage 6: Canonical Transcript & Projections (`meetings/model.rs`, `variants.rs`)
- **Canonical Model (`TranscriptSegment`)**:
  - Primary text, start/end timestamps, channel provenance, no-speech confidence, and full `SegmentTelemetry`.
- **Derived Projections**:
  - `romanized_text`: Deterministic Latin projection of Devanagari via a zero-cost finite-state transliterator (`capture/romanize.rs`). Never outsourced to an LLM.
  - `translated_text`: English translation generated via batched background passes.

---

## 2. Telemetry & Observability Schema

Every decoded segment captures the following diagnostic telemetry in `SegmentTelemetry`:

| Field | Type | Description |
|---|---|---|
| `sequence` | `u64` | Monotonic segment index in the meeting transcript |
| `start_seconds` | `f64` | Timestamp relative to recording start |
| `end_seconds` | `f64` | Timestamp relative to recording start |
| `duration_seconds`| `f64` | Total segment duration (including pre-roll) |
| `speech_seconds` | `f64` | Voiced audio duration measured by VAD |
| `channel` | `SegmentChannel` | `Microphone`, `System`, or `Mixed` |
| `model` | `String` | Path or identifier of active speech model |
| `decode_profile` | `String` | `"careful"` (beam search) or `"fast"` (greedy) |
| `decode_ms` | `u128` | Inference execution time |
| `rtf` | `f32` | Real-Time Factor (`decode_time / duration`) |
| `queue_wait_ms` | `u128` | Time spent waiting in the decode queue |
| `no_speech_probability` | `f32` | Whisper no-speech probability |
| `compression_ratio` | `f32` | Zlib compression ratio of transcript text |
| `quality_status` | `String` | `"good"`, `"recovered"`, `"suspicious"`, or `"rejected"` |
| `retry_count` | `usize` | Number of recovery attempts made (0 or 1) |
| `forced_split` | `bool` | True if cut at 25s ceiling |
| `rms` | `f32` | Root Mean Square energy of segment samples |
| `peak_amplitude` | `f32` | Peak sample absolute value |
| `near_clipping_percent` | `f32` | Percentage of samples >= 0.98 |

---

## 3. Known Limitations (Phase 1 Baseline)

1. **Tokenizer Overhead in Devanagari**:
   - Whisper's byte-pair tokenizer requires 2–4 tokens per Devanagari character compared to ~0.3 tokens per Latin character.
   - Non-Latin decoding runs with the fast profile (greedy search, zero fallback increment) to maintain an RTF < 1.0 on local CPU.
2. **Multi-Speaker Diarization on Single Channel**:
   - Audio from the system loopback channel ("Others") captures all remote participants as one mixed stream. Fine-grained multi-speaker distinction on the remote side requires voiceprint clustering rather than acoustic hardware separation.
3. **Severe Acoustic Noise vs. Weak Speech**:
   - Rooms with sustained background noise exceeding -30 dBFS elevate the adaptive noise floor, which can occasionally truncate quiet trailing utterances.

---

## 4. Phase 1 Verification Status

- **Unit & Integration Suite**: 995 passed, 0 failed, 0 ignored (`cargo test --lib`).
- **Linter & Static Analysis**: Zero warnings (`cargo clippy --all-targets -- -D warnings`).
- **Production Speech Model**: `ggml-small.bin` (244M parameters) remains the unchanged default model throughout Phase 1.
