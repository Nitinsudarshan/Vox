# Production Provider Migration Audit: `SttEngine` Call Sites & Seams

## 1. Executive Summary

Vox has introduced the [`SpeechRecognizer`](../native/src-tauri/src/capture/recognizer.rs) trait and associated adapters ([`WhisperRecognizer`, `ParakeetRecognizer`](../native/src-tauri/src/capture/recognizers.rs)) to abstract audio-to-text engines behind a declared capability model. However, production paths—dictation hotkeys, the live meeting worker, audio import, re-transcription, and Tauri commands—still couple directly to the concrete [`SttEngine`](../native/src-tauri/src/capture/stt.rs) type.

This audit maps every direct `SttEngine` call site across:
- `native/src-tauri/src/capture/recognizer.rs`
- `native/src-tauri/src/capture/stt.rs`
- `native/src-tauri/src/meetings/transcription.rs`
- `native/src-tauri/src/meetings/import.rs`
- `native/src-tauri/src/meetings/engine.rs`

It evaluates migration risks, details adapter gaps, enumerates required tests, and provides an ordered migration strategy. **No production code is migrated during Stage 13.**

---

## 2. Direct `SttEngine` Call Sites & Surface Inventory

### A. Live Meeting Transcription Worker (`meetings/transcription.rs`)
| Call Site | Code / Signature | Current Behavior |
|---|---|---|
| Line 353 | `WorkerConfig { engine: SttEngine, ... }` | Concrete `SttEngine` stored in configuration struct. |
| Line 389 | `spawn_worker(..., engine: SttEngine)` | Takes owned `SttEngine` to move into the background thread. |
| Line 846 | `decode_segment(engine: &SttEngine, ...)` | Worker loop calls `decode_segment` for each queued audio chunk. |
| Line 880 | `engine.transcribe_utterances_with_config(model, samples, &language, &decoding)` | Directly invokes internal whisper.cpp state machine with dynamically injected initial prompt. |

### B. Meeting Engine Orchestration (`meetings/engine.rs`)
| Call Site | Code / Signature | Current Behavior |
|---|---|---|
| Line 71 | `MeetingEngine { pub stt: SttEngine, ... }` | Direct field on the central meeting state machine. |
| Line 229 | `spawn_worker(..., self.stt.clone(), ...)` | Clones `SttEngine` handle (Arc-wrapped whisper model context). |
| Line 510 | `retranscribe(..., &self.stt, ...)` | Delegates background re-transcription of saved audio to `import.rs`. |

### C. Offline Audio Import & Re-transcription (`meetings/import.rs`)
| Call Site | Code / Signature | Current Behavior |
|---|---|---|
| Line 176 | `import_audio(..., engine: &SttEngine, ...)` | Batch decodes imported audio files (WAV, MP3, M4A, AAC, FLAC). |
| Line 239 | `retranscribe(..., engine: &SttEngine, ...)` | Re-runs decoding on an existing meeting with modified parameters/model. |
| Line 314, 591, 721 | `decode_file_segments(..., engine: &SttEngine, ...)` | Loops over segments, directly calling `engine.transcribe_utterances_with_config`. |

### D. Dictation & Tauri Commands (`commands.rs` & `hotkeys/mod.rs`)
| Call Site | Code / Signature | Current Behavior |
|---|---|---|
| `commands.rs:86` | `AppState { pub stt: SttEngine, ... }` | Held as Tauri application state. |
| `commands.rs:587` | `stt.transcribe_parakeet(...)` | Dictation fallback branching: manual check between `whisper` and `parakeet`. |
| `hotkeys/mod.rs:562` | `stt.transcribe_parakeet(...)` / `stt.transcribe_with_config(...)` | Push-to-talk hotkey thread invokes `SttEngine` directly. |

---

## 3. Critical Adapter Gaps in `SpeechRecognizer`

Before `SttEngine` can be removed from production paths, the `SpeechRecognizer` trait and `RecognitionRequest` struct must bridge three critical features currently consumed by `transcription.rs`:

1. **Dynamic Prompt & Vocabulary Conditioning:**
   - In `meetings/transcription.rs:862`, `WorkerConfig::vocabulary.build_prompt(...)` dynamically computes an `initial_prompt` per segment (combining previous segment text, domain vocabulary terms, and forced split context).
   - `RecognitionRequest` currently only accepts `samples`, `language`, and `translate`. It lacks `initial_prompt: Option<&'a str>` or a generic `prompt: Option<&'a str>` field.
2. **Detailed Segment Telemetry Diagnostics:**
   - `speech_health::screen_decode` and `MeetingDiagnostics` require low-level decoder telemetry: `no_speech_prob`, `avg_logprob`, and `transcription_latency_ms`.
   - `Recognition` provides `mean_no_speech_prob()` and `RecognitionTiming`, but does not expose token-level compression ratio or language probability arrays when available.
3. **Model Switching & Unloading Semantics:**
   - `SttEngine` manages a lazy-loaded, thread-safe cache slot (`Mutex<Option<WhisperContext>>`).
   - `SpeechRecognizer` instances currently bind to a single `model_path` at creation time. Switching models mid-session (e.g., during retranscribe) requires instantiating a new recognizer rather than repointing the engine.

---

## 4. Risk Assessment of Migration

| Component | Risk Level | Primary Failure Modes |
|---|---|---|
| **Live Meeting Worker (`transcription.rs`)** | **HIGH** | Latency regression if prompt conditioning is omitted; thread lock contention if recognizer wrapper introduces additional mutexes; dropped segments if error mapping changes. |
| **Audio Import & Re-transcription (`import.rs`)** | **MEDIUM** | Cancellation token propagation (`cancel: Arc<AtomicBool>`); progress reporting accuracy if batch chunks differ. |
| **Push-to-Talk Dictation (`hotkeys/`)** | **MEDIUM** | Audio latency penalty ($>50\text{ ms}$) on push-to-talk release; dual-engine switching bugs between Whisper and Parakeet. |
| **Diagnostics & Benchmark (`benchmark/`)** | **LOW** | Already operates on `SpeechRecognizer`. Verified in benchmark suite. |

---

## 5. Recommended Migration Sequence

When migration commences in a subsequent stage, it should strictly follow this order:

1. **Step 1: Extend `RecognitionRequest` & `RecognizerCapabilities`**
   - Add `initial_prompt: Option<&'a str>` and `temperature: Option<f32>` to `RecognitionRequest`.
   - Ensure `WhisperRecognizer` forwards `initial_prompt` to `WhisperDecodingConfig`.
   - Ensure non-prompting engines (e.g. Parakeet) declare `Support::No` for prompt conditioning and safely ignore it.
2. **Step 2: Migrate Offline Audio Import (`meetings/import.rs`)**
   - Import is offline and non-realtime, making it the safest production testbed.
   - Replace `engine: &SttEngine` with `recognizer: &dyn SpeechRecognizer`.
   - Validate with unit tests: `cargo test --lib meetings::import::tests`.
3. **Step 3: Migrate Retranscription (`meetings/import.rs` & `meetings/engine.rs`)**
   - Migrate `retranscribe` path. Verify that output transcripts and diagnostics match historical outputs.
4. **Step 4: Migrate Live Meeting Worker (`meetings/transcription.rs`)**
   - Change `WorkerConfig` to take `Arc<dyn SpeechRecognizer>`.
   - Benchmark pipeline RTF against `transcription.rs` baseline to confirm zero latency regressions.
5. **Step 5: Migrate Dictation Hotkeys & Commands (`hotkeys/` & `commands.rs`)**
   - Unify the `if use_parakeet` conditional branching into a single recognizer lookup via `RecognizerRegistry::get(task)`.
