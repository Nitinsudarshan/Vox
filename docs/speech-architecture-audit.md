# Vox Speech Architecture — Audit

Traced from **v0.1.0**, commit `b4d82ab`, branch `main`. Read this as history:
it records what the speech stack was on that commit, not what it must become.
Where a later change contradicts something here, the code is right and this
file is stale.

This is a **deep record** in the sense of [README.md](README.md) — a long-form
analysis against a specific version, like [capture/RESEARCH.md](capture/RESEARCH.md).
It proposes nothing that is not anchored to a named module, and it decides
nothing: decisions that follow from it live in
[speech-decision-log.md](speech-decision-log.md).

Scope: the path from a microphone sample to a meeting report, plus the
speech-adjacent subsystems that path depends on — dictation's STT engine, the
hallucination screen, the evaluation harness, the speaker pipeline, and the
TTS layer the decision log says exists.

---

## 1. Current architecture

Two speech paths share one engine.

```text
                         ┌──────────────────────────────┐
  PTT / dictation ──────▶│ capture::AudioRecorder       │──┐
  (hotkeys/mod.rs)       │ one stream, whole-buffer VAD │  │
                         └──────────────────────────────┘  │
                                                           ▼
                         ┌──────────────────────────────┐ ┌───────────────────┐
  microphone ──┐         │ meetings::capture            │ │ capture::stt      │
               ├────────▶│ DualCapture: 2 cpal streams, │ │ SttEngine         │
  system audio ┘         │ lockstep drain, soft mix,    │ │ ONE model slot,   │
  (loopback)             │ per-channel energy kept      │ │ behind a Mutex    │
                         └──────────────┬───────────────┘ └─────────▲─────────┘
                                        │ MixedAudio (mixed/mic/sys)│
                                        ▼                           │
                         ┌──────────────────────────────┐           │
                         │ meetings::engine::run_pump   │           │
                         │ ONE thread, two consumers    │           │
                         └───┬──────────────────────┬───┘           │
                             │                      │               │
                             ▼                      ▼               │
              ┌──────────────────────┐  ┌────────────────────────┐  │
              │ meetings::segmenter  │  │ meetings::checkpoint   │  │
              │ streaming energy VAD │  │ 30 s WAV chunks        │  │
              └──────────┬───────────┘  └───────────┬────────────┘  │
                         │ SpeechSegment            │               │
                         ▼                          ▼               │
              ┌──────────────────────┐   audio/chunk_NNNNNN.wav     │
              │ TranscriptionQueue   │   ──▶ audio/audio.wav        │
              │ sync_channel(64),    │                              │
              │ try_send, counters   │                              │
              └──────────┬───────────┘                              │
                         │ DecodeJob                                │
                         ▼                                          │
              ┌──────────────────────────────────────────┐          │
              │ meetings::transcription::run_worker      │──────────┘
              │ ONE serial thread                        │
              │  decode → speech_health → text_normalize │
              │  → romanize → store.append_segments      │
              └──────────┬───────────────────────────────┘
                         │ TranscriptSegment
                         ▼
                 transcript.json  ◀── also written by: speaker detection,
                         │              English track, romanization,
                         │              re-transcription
                         ▼
              ┌──────────────────────────────────────────┐
              │ meetings::summary::service                │
              │ render_transcript_with_speakers → LLM     │
              └──────────┬───────────────────────────────┘
                         ▼
                   summary.json ──▶ promote_meeting_to_scribble ──▶ vault
                                                                      │
                                                     entities / memory / relationships
```

The important structural facts, each anchored:

| Fact | Where |
|---|---|
| Microphone and loopback are drained in temporal lockstep, not "whatever is available" | `meetings/capture.rs:plan_drain` |
| Per-channel energy survives the mix, so a line can say **You** / **Others** | `meetings/capture.rs` (`MixedAudio.mic` / `.sys`), `meetings/segmenter.rs:classify_channel` |
| Segmentation is streaming, with 300 ms pre-roll, 400 ms redemption, 25 s ceiling | `meetings/segmenter.rs` |
| The decode queue is bounded at 64 and **non-blocking** (`try_send`) | `meetings/transcription.rs:64`, `TranscriptionQueue::submit` |
| Decoding is strictly serial, so transcript order is structural | `meetings/transcription.rs:run_worker` |
| Sequence numbers are assigned at submit, so a drop leaves a visible gap | `meetings/transcription.rs:TranscriptionQueue::submit` |
| Every decode passes a hallucination screen before it reaches the transcript | `capture/speech_health.rs:screen_decode` |
| Confidence is Whisper's own `no_speech_prob`, never invented | `meetings/model.rs:TranscriptSegment` |
| Audio is checkpointed every 30 s and merged on stop | `meetings/checkpoint.rs` |
| Every meaningful write is temp-file-plus-rename | `meetings/store.rs:541` (`write_atomic`) |
| A crash is finalized on next launch, from the checkpoints that reached disk | `meetings/store.rs:recover_interrupted`, `meetings/engine.rs:recover_interrupted` |

## 2. Data flow

### 2.1 Live recording

1. `DualCapture::start` opens the microphone and the default output device in
   loopback on a dedicated thread. If only one opens, recording proceeds and
   `CaptureBinding` says which. If neither opens, the start fails and the
   half-created meeting directory is deleted (`engine.rs:start`).
2. Each `cpal` callback downmixes to mono, resamples to 16 kHz and pushes into
   a per-device `VecDeque` FIFO. Nothing else happens on the audio thread.
3. Every 20 ms the mixer thread drains both FIFOs in lockstep, soft-mixes, and
   sends one `MixedAudio` block carrying `mixed`, `mic` and `sys` for the same
   span.
4. `run_pump` reads those blocks on its own thread and does two things per
   block, in this order: feed the segmenter and submit whatever segments
   completed; then push the mixed samples to the checkpoint writer.
5. `Segmenter::push` accumulates 20 ms frames, tracks an adaptive noise floor
   over the last 3 s of non-speech frames, and emits a `SpeechSegment` on
   redemption timeout or at the 25 s ceiling (split at the quietest frame in
   the tail).
6. `TranscriptionQueue::submit` assigns a sequence number and `try_send`s. Full
   queue ⇒ counted, logged, and emitted as a `meeting-transcription-warning`.
7. The worker decodes one segment at a time, screens it, normalizes it,
   romanizes Devanagari, appends it to `transcript.json` and emits
   `meeting-transcript-segment`.
8. On stop: capture stops → pump flushes the segmenter and merges audio →
   the queue drains (up to `DRAIN_TIMEOUT` = 600 s) → the worker is joined →
   `meeting.json` is finalized.

### 2.2 Import and re-transcription

`import::run_batch` streams the file through `symphonia` packet by packet into
the **same** `Segmenter`, decodes each segment with `BatchConfig` (one decode
profile, no cheap-script fallback, encoder clamp off) and appends. Peak memory
is a function of segment length, not file length.

### 2.3 Derived views

| View | Produced by | Writes to |
|---|---|---|
| Romanized | `capture::romanize`, offline state machine | `segment.romanized_text` (on read in `get_meeting`, persisted by `romanize_meeting_transcript`) |
| English | `import::generate_english_track` — a second Whisper pass in translate mode, aligned by **time** | `segment.translated_text` |
| Speakers | `import::fingerprint_segments` → `voiceprint` → `speakers::assign_speakers` | `segment.speaker_id` **and** `speakers.json` |
| Report | `summary::service` over `render_transcript_with_speakers` | `summary.json` |
| Vault / KG | `promote_meeting_to_scribble` — summary markdown, or the rendered transcript if no summary | a Scribble, then `entities` / `memory` / `relationships` |

## 3. Source-of-truth files

| Artifact | Path | Written by | Authority |
|---|---|---|---|
| Durable audio | `<vault>/meetings/<id>/audio/chunk_*.wav`, then `audio.wav` | `meetings::checkpoint` | **Highest.** Everything else can be rebuilt from it. |
| Raw + derived transcript | `.../transcript.json` | worker, retranscribe, English pass, romanize, speaker detection | Mixed. See §7. |
| Speakers | `.../speakers.json` | `speakers::assign_speakers`, `rename_meeting_speaker` | User names are authoritative; groupings are proposals. |
| Report | `.../summary.json` | `summary::service` | Derived; carries `fingerprint` for cache validity. |
| Meeting record | `.../meeting.json` | `store::update_meeting` | Derived counts (`segment_count`, `dropped_segments`, `duration_seconds`). |
| Notes | `.../notes.md` | the user | Authoritative, never machine-written. |

The vault layout is already the right shape for a two-pass architecture: audio
is separable from transcript, and transcript is separable from report.

## 4. Failure modes

Ranked by what they cost.

| # | Failure | Current behaviour | Cost |
|---|---|---|---|
| F1 | `CheckpointWriter::new` fails (permissions, full disk) | `tracing::error!` at `engine.rs:527`, `writer = None`, **recording continues with no audio saved**. No event, no UI. | Total, silent audio loss for the whole meeting. The user learns at the end. |
| F2 | A `writer.push` fails mid-meeting | `tracing::error!` at `engine.rs:545`, loop continues | Silent gap in the recording, and `duration_seconds` still counts the samples. |
| F3 | Decode queue saturates | Counted, logged, `meeting-transcription-warning` emitted, `dropped_segments` persisted, shown in `MeetingView.tsx:199` | Loud, and correct. The audio still has the speech. |
| F4 | A segment fails to decode | Counted, warning emitted once per failure | Loud. |
| F5 | Drain exceeds 600 s after stop | `cancel` is set, the remaining backlog is abandoned, counted into `lost` | Loud, but the number is a lump: "`queued - completed`". |
| F6 | System audio will not open | `CaptureBinding.system_audio = false`, a warning string on the status and on `meeting.json` | Correct and visible. |
| F7 | Microphone will not open, system audio does | Recording proceeds. **No warning is constructed** — `engine.rs:start` only builds one for the system-audio case. | The inverse of F6, and silent. |
| F8 | Crash mid-recording | `recover_interrupted` marks the meeting complete, re-derives duration from the chunks, merges them | Costs at most 30 s. Genuinely good. |
| F9 | Crash mid-re-transcription | `run_batch` truncates `transcript.json` **before** decoding (`import.rs`), and the in-memory `previous` restore only runs on `Err` | A crash — not a cancel — leaves a partial transcript. Audio is intact, so it is recoverable by re-running. |
| F10 | Transcript write fails | `tracing::error!` plus a `storage` warning event | Loud. |
| F11 | The pump thread panics | `engine.rs:stop` catches the join error and reports duration 0, audio path `None` | Audio chunks are on disk but the record says there is none. |
| F12 | Both `SttEngine` slots contended (dictation during a meeting) | `lock_wait_ms` accumulates; a different model path evicts the loaded one (`model_reloaded`) | Measured, printed to stdout, never persisted. |

**The pattern worth naming**: transcript loss is loud (F3, F4, F10) and audio
loss is silent (F1, F2, F7, F11). That is backwards. Audio is the only thing
that cannot be regenerated.

## 5. Latency bottlenecks

Numbers below are structural, not measured — measuring them is §14.

| Stage | Known cost | Bound by |
|---|---|---|
| Capture → mixer | ≤ 20 ms (`MIXER_TICK`) plus device buffer | `capture.rs` |
| Lockstep stall on a silent loopback | up to 250 ms (`MAX_STREAM_LAG_SAMPLES`) | `plan_drain` |
| Onset detection | 60 ms (`ONSET_FRAMES` × 20 ms), recovered by the 300 ms pre-roll | `segmenter.rs` |
| **Finalization** | **400 ms of silence minimum**, and unbounded inside a sentence up to 25 s | `REDEMPTION_MS`, `MAX_SEGMENT_SECONDS` |
| Queue wait | 0 → minutes, depends entirely on `pipeline_rtf` | `transcription.rs` |
| Model load | 0, or a full model load when the single `SttEngine` slot was holding another model | `stt.rs:1232` |
| Decode | the dominant term; `decode_rtf` is measured per segment | `SttEngine::transcribe_utterances_with_config` |
| Persist | one whole-file rewrite of `transcript.json` **per segment** | `store.rs:247` |
| Post-stop drain | `(pipeline_rtf − 1) × meeting_length`, capped at 600 s | `engine.rs:78`, `projected_drain` |

Two of these are the ones worth attacking, and they are different problems:

- **Finalization latency is a segmentation decision, not a model decision.**
  A 400 ms hangover is the floor on how soon any word can be decoded, and no
  faster model changes it. This is the whole argument for treating turn
  detection as its own subsystem.
- **Queue wait is the only unbounded term.** It is a function of
  `pipeline_rtf`, which the code already computes and prints
  (`print_meeting_summary`) and never stores.

## 6. Accuracy bottlenecks

| Bottleneck | Evidence in the repo |
|---|---|
| Decoding the **mix**, not the channels | `decode_segment` passes `job.segment.samples` — the mixed stream. Two people talking over each other are decoded as one soft-compressed signal, even though `MixedAudio` carried both channels separately one stage earlier. |
| Language re-detection drift | `docs/meetings.md` already names it: Whisper re-detects every 30 s, and a chunk decoded under the wrong language returns fluent nonsense rather than an error. `SttLanguageConfig::from_settings(.., LongForm)` deliberately does not pin. |
| A cheap profile chosen by script, not by measurement | `ScriptTracker` switches the live worker to greedy/no-fallback the moment it sees a non-Latin script (`transcription.rs`). The cost of that switch has never been measured — which is exactly the accuracy/latency trade-off a benchmark exists to decide. |
| Segment boundaries mid-clause | A forced split at 25 s cuts at the quietest frame in the tail, and `forced_split` is recorded on `SpeechSegment` — and then **dropped**: it never reaches `TranscriptSegment`, so nothing downstream knows two lines are one sentence. |
| Glossary applied as post-hoc text edit, and disconnected from the decode's own prompt | Two independent mechanisms that never meet. `WhisperDecodingConfig::from_settings_defaulting` will set `initial_prompt` from `SttSettings::custom_initial_prompt` when `enable_initial_prompt` is on — one free-text string the user types. The **glossary** (`settings.dictionary`) is a separate list, and it is only ever applied afterwards, by `text_normalize::normalize_segment_text(&text, &config.glossary)` on the decoded string. So the words the user told Vox about cannot bias the decode; they can only repair its output. |
| No numeric / proper-noun accounting | `evaluation::AccuracyMetrics` has WER, CER, and a technical-term list hardcoded to Vox's own stack (`TRACKED_TECHNICAL_TERMS`). Numbers, dates and money are not measured at all. |
| The hallucination screen is a *filter*, not a *metric* | `speech_health::screen_decode` discards and logs at `debug`. The count reaches `TranscriptionStats::discarded`, which is printed and never stored — so "how often does this model hallucinate over silence" is unanswerable after the fact. |

## 7. Potential data-loss paths

1. **F1/F2 — checkpoint failure is silent.** The single worst one. `run_pump`
   has no `AppHandle` and therefore structurally cannot warn.
2. **Unbounded audio channel.** `capture.rs:171` is `std_mpsc::channel()` —
   unbounded. If `run_pump` stalls (a slow disk during a checkpoint write, or
   an expensive segmenter frame), blocks accumulate in memory with nothing
   shedding load and nothing reporting depth. The decode queue is carefully
   bounded; the audio channel ahead of it is not. The device FIFOs
   (`VecDeque` in `run_capture_loop`) are unbounded for the same reason.
3. **F9 — re-transcription truncates first.** `run_batch` calls
   `save_transcript(meeting_id, &[])` before it decodes anything.
4. **Segments shorter than 250 ms are discarded before decoding.**
   `close_segment` returns `None` below `MIN_SPEECH_FRAMES` and the audio is
   dropped from the segment path entirely. It is still in the recording, and
   nothing counts how often this happens.
5. **Drain timeout discards the tail.** After 600 s the remaining backlog is
   cancelled. It is counted, but the *audio* for those segments is in
   `audio.wav`, and nothing offers to finish the job later.
6. **Pause discards, deliberately.** `DualCapture::pause` drains both FIFOs, so
   paused audio is excised rather than stored as silence. That is a design
   choice, correctly documented, and it means paused time is unrecoverable.

## 8. Transcription ordering risks

Ordering is in good shape, and the reasons are structural rather than
incidental:

- One decoder thread. A slow segment cannot be overtaken.
- Sequence assigned at `submit`, before the decode.
- `load_transcript` sorts by sequence and dedups on read, so even a torn write
  reads back ordered.
- `write_atomic` means `transcript.json` is never half-written.

Three residual risks:

| Risk | Detail |
|---|---|
| R1 | A dropped segment consumes a sequence number by design, so gaps are visible — but **nothing in the UI or the data model reads the gaps**. `dropped_segments` is a count with no positions, so a reader cannot see *where* speech is missing. |
| R2 | Re-transcription renumbers from 0 (`run_batch`'s local `sequence`), while `speakers.json` and any external reference still point at old ids. `speaker_id` survives only because `assign_speakers` is re-run. |
| R3 | `append_segments` rewrites the entire file per segment: read → extend → sort → dedup → serialize → write. At ~1,000 segments for a two-hour meeting this is O(n²) in bytes written. It is correct; it is the wrong shape at long-meeting scale. |

## 9. Speaker attribution limits

Two layers, and the codebase is already honest about the difference.

**Measured — the capture channel.** `SegmentChannel` is derived from
per-channel energy with a 3× dominance ratio. It cannot be wrong, and it
cannot distinguish two remote participants. `docs/meetings.md` and Decision 68
both say exactly this.

**Proposed — the acoustic grouping.** `voiceprint.rs` computes 26 MFCC
statistics per turn, cosine-normalised; `speakers.rs` clusters with
average-linkage agglomeration at `DEFAULT_SPLIT_DISTANCE = 0.35`, capped at 8
speakers, skipping turns under `MIN_VOICEPRINT_SECONDS = 1.2`. The microphone
channel is never clustered; the channels are never merged; user-typed names
survive a re-run and Vox's own placeholders do not.

Limits, stated plainly:

- MFCC statistics are not speaker embeddings. Two similar voices on one
  loopback stream will merge; one voice recorded at two distances may split.
  `maybe_later.md` §11 and Decision 68 already record this.
- A turn under 1.2 s gets no fingerprint and stays `speaker_id: None`.
- Nothing carries a voice across meetings, by design — there is no embedding
  stable enough to store.
- **The detection run writes `speaker_id` back into `transcript.json`**
  (`commands.rs:1016`). The raw decode and a derived attribution share one
  file, so "re-run detection" and "re-derive the transcript" are not cleanly
  separable operations.

## 10. Current TTS path

**There is none.** This is the largest documentation-versus-code gap in the
repository, and it matters because the plan's Stage 11 assumes otherwise.

`docs/decisions.md` Decisions 47–56 describe a Talkback subsystem in detail:
`native/src-tauri/src/talkback/` (11 modules), `tts::TtsProvider` with
`PiperProvider` and `NullProvider`, `tts/discovery.rs`, `tts/manifest.rs`,
`tts/installer.rs`, `resources/voice-manifest.json`,
`scripts/build-voice-manifest.mjs`, a `SpeechPipeline` with a bounded
synthesis queue, `talkback::turn::TurnDetector`, and `docs/talkback/RESEARCH.md`.

On this commit, **none of those paths exist**:

```
native/src-tauri/src/     → no talkback/, no tts/
native/src-tauri/resources/ → nemo128.onnx only (Parakeet's preprocessor)
scripts/                  → no build-voice-manifest.mjs
docs/                     → no talkback/
```

What survives is residue:

| Residue | Where |
|---|---|
| `LLMClient::complete_streaming` and `parse_stream_line` | `providers/mod.rs:1143`, `:1320` — **no callers outside their own tests** |
| `KanbanSource::Talkback` | `vault/kanban.rs:42` |
| `TodoSourceKind = ... | 'talkback'` | `native/src/types/index.ts:338` |
| `ProcessResult`'s TTS audio field comment | `pipeline/mod.rs:45` |
| `maybe_later.md` §§1–3, all three pointing at `talkback/turn.rs` and `tts::TtsProvider` | `maybe_later.md` |
| Roughly a dozen doc-comments across `capture/stt.rs`, `capture/speech_health.rs`, `capture/device.rs`, `context/assembly.rs`, `pipeline/enrichment.rs` | — |

`docs/requirements.md` FR-2.4 still requires TTS ("If a local TTS engine is
configured, the answer MUST additionally be synthesized"), marked deferred under
Decision 34 — which Decision 47 says it superseded by *replacement*, and
Decision 47's replacement is itself gone.

So Stage 11 is not "wrap the existing local TTS in an interface". It is either
a from-scratch TTS layer, or a deliberate restoration — and before either, the
decision log needs an entry saying Talkback was removed and Decisions 47–56 no
longer describe the code.

## 11. Current report / knowledge-graph dependencies

```text
transcript.json
   │  render_transcript_with_speakers()   ← the only rendering used for reports
   ▼
summary::service::generate
   │  fingerprint(transcript, template, options, provider, client)
   │  one pass if it fits the context window, else chunk-with-overlap → notes → assemble
   │  always generated in English, translated afterwards
   ▼
summary.json  { markdown, english_markdown, previous_markdown, fingerprint, … }
   │
   │  promote_meeting_to_scribble()  ← summary markdown, or the *rendered transcript*
   ▼                                   when no summary exists
vault Scribble
   │
   ▼
entities::extractor → entities/ · memory/ · relationships/   (the knowledge layer)
```

Findings:

- Downstream intelligence already reads **one** rendering function. That is
  most of a canonical-transcript contract; what is missing is that the
  rendering is a `String`, so nothing downstream can point back at a segment.
- The report's provenance is a **fingerprint**, not a segment reference. It
  answers "is this report still valid?" and cannot answer "which line said
  that?".
- The knowledge layer is two hops away and loses meeting structure entirely: a
  Scribble is markdown. Every entity, decision and relationship derived from a
  meeting traces back to a Scribble, not to a timestamp.
- `pipeline::source_boundary` frames the transcript as data inside an
  unguessable delimiter before it reaches a model — correct, and it stays
  correct under any canonical-transcript refactor.
- **Provenance-sensitive handling already exists** for meetings specifically:
  Decision 65, "A Tier 2 Rewrite May Never Touch a Meeting Transcript".

## 12. What is already good and must not be rewritten

Each of these is load-bearing, has tests, and has a recorded reason.

1. **Dual-stream lockstep capture with per-channel energy retained**
   (`meetings/capture.rs`). This is the thing most comparable products get
   wrong, and it is what makes channel attribution free. `plan_drain` and
   `soft_mix` have real tests, including one asserting the mix curve has no
   step in it.
2. **The streaming segmenter's shape** (`meetings/segmenter.rs`): pre-roll,
   redemption, adaptive noise floor built only from non-speech frames, a
   ceiling split at the quietest frame in the tail. The *thresholds* are
   tunable; the shape is right.
3. **Serial decoding with sequence-at-submit** (`meetings/transcription.rs`).
   Ordering as a structural property, not a sort.
4. **The bounded, non-blocking decode queue.** `try_send` is what keeps a slow
   model from ever touching the recorder.
5. **`capture::speech_health`.** Two separate defences, before and after the
   decode, and a rejection that records its reason rather than rewriting text.
6. **Real `no_speech_prob` instead of an invented confidence.**
7. **30 s checkpoints, streamed merge, crash recovery.** `merge_chunks`
   streams rather than loading; recovery passes `remove_chunks = false` first
   so a failed merge leaves the sources.
8. **`write_atomic` everywhere, and `sort`+`dedup` on read.**
9. **The three-view language model** (`variants.rs`): romanization as an
   offline projection, English from a second *audio* pass, LLM only as the
   fallback — plus `accept_translation` refusing a romanization handed back as
   a translation.
10. **The honest speaker product shape**: propose, play a sample, let the user
    name it; never overwrite a typed name.
11. **The vault layout.** One directory per meeting, files not a database,
    deleting a meeting deletes its audio.
12. **`pipeline::source_boundary`.**

## 13. Recommended target architecture

Not a rewrite. Four seams inserted into the chain above, each of which can be
built and shipped independently.

```text
      ┌────────────── SEAM A: the recorder owns its own thread ──────────────┐
      │                                                                      │
mic ──┤                                                                      │
      ├─▶ DualCapture ─┬─▶ [bounded] ─▶ durable recorder (checkpoints)  ◀── never blocked,
sys ──┘                │                                                     always reports
                       └─▶ [bounded] ─▶ perception thread
                                          │
      ┌─────────────── SEAM B: speech state is its own subsystem ────────────┐
      │                                          │                           │
      │   SpeechState: Silence → PossibleSpeech → Speaking → ProbableEnd     │
      │                → Finalized      (acoustic only, no ASR, no LLM)      │
      └──────────────────────────────┬───────────────────────────────────────┘
                                     ▼
      ┌─────────────── SEAM C: SpeechRecognizer, with capabilities ──────────┐
      │   Whisper │ Parakeet │ (future cloud)   — capabilities declared,     │
      │   selected per task (PTT / live / final), not per app                │
      └──────────────────────────────┬───────────────────────────────────────┘
                                     ▼
                            RawAsrSegment  (immutable evidence, append-only)
                                     │
      ┌─────────────── SEAM D: TranscriptAssembler ──────────────────────────┐
      │   order · merge · dedupe · channel · speaker · language · revision   │
      └──────────────────────────────┬───────────────────────────────────────┘
                                     ▼
                          CanonicalTranscript  (the AI input contract)
                                     │
                    reports · actions · entities · KG · Q&A
```

Change classification. Every row names the module that owns the change.

| # | Change | Module | Class |
|---|---|---|---|
| 1 | Give `run_pump` an `AppHandle` and emit a warning when checkpointing fails or is unavailable | `meetings/engine.rs:515` | **CHANGE** |
| 2 | Warn when the microphone fails but system audio opens (the mirror of the existing warning) | `meetings/engine.rs:start` | **CHANGE** |
| 3 | Bound the mixer→pump audio channel and count what it sheds | `meetings/capture.rs:171` | **CHANGE** |
| 4 | Move checkpoint writing off the segmentation thread so segmentation cost cannot delay a durable write | `meetings/engine.rs:run_pump` | **MEASURE FIRST** — measure `writer.push` and `segmenter.push` cost per block before splitting a thread |
| 5 | Persist `TranscriptionStats` / `SegmentTiming` instead of `println!` | `meetings/transcription.rs:532`, `:619` | **CHANGE** |
| 6 | Carry `forced_split` through to `TranscriptSegment` | `meetings/segmenter.rs`, `meetings/model.rs` | **CHANGE** |
| 7 | Count and persist screen-rejections by reason, and sub-minimum segments | `capture/speech_health.rs`, `meetings/segmenter.rs` | **CHANGE** |
| 8 | Long-form benchmark corpus + a runner over it | `capture/evaluation.rs` (extend; `run_benchmark_matrix_on_sample` already exists and is unused) | **CHANGE** |
| 9 | Benchmark against the **meeting** path (segmenter + queue), not only `VadConfig::process` | `capture/evaluation.rs:evaluate_audio_buffer` | **CHANGE** |
| 10 | Extract `SpeechState` from `Segmenter` as an observable state machine | `meetings/segmenter.rs` | **CHANGE** (keep the algorithm; expose the states) |
| 11 | Prosodic features beyond energy for end-of-turn | `meetings/segmenter.rs` | **MEASURE FIRST** — the current 400 ms hangover's real cost is unmeasured |
| 12 | `SpeechRecognizer` trait + capability descriptor over Whisper and Parakeet | `capture/stt.rs:1232` | **CHANGE** |
| 13 | More than one loaded model at a time (a live profile and a final profile) | `capture/stt.rs:1234` (single `Mutex<Option<(String, WhisperContext)>>`) | **MEASURE FIRST** — measure `model_load_ms` and `model_reloads` on a real bilingual meeting first |
| 14 | Two-pass live/final | `meetings/transcription.rs`, `meetings/import.rs` | **CHANGE** — `retranscribe` is already 80 % of the final pass |
| 15 | Split `RawAsrSegment` from `CanonicalSegment`; stop writing derived fields into `transcript.json` | `meetings/model.rs`, `meetings/store.rs` | **CHANGE** |
| 16 | Make `append_segments` incremental rather than a full rewrite per segment | `meetings/store.rs:247` | **MEASURE FIRST** — measure `persist_ms` growth across a 2-hour meeting; it may be irrelevant next to decode cost |
| 17 | Connect the glossary to the decode's `initial_prompt` instead of leaving two unrelated mechanisms | `capture/text_normalize.rs`, `capture/stt.rs:from_settings_defaulting` | **MEASURE FIRST** — Whisper prompts can themselves induce hallucination, and `speech_health` would then be screening text the prompt suggested |
| 18 | Per-channel decoding (decode `mic` and `sys` separately, not the mix) | `meetings/transcription.rs:decode_segment` | **MEASURE FIRST** — doubles decode cost; worth it only if overlap is measurably common |
| 19 | A stronger speaker encoder behind the existing `voiceprint` interface | `meetings/voiceprint.rs` | **DEFER** — blocked on `docs/spikes/onnx-windows.md`, which is written and never run |
| 20 | Voices remembered across meetings | `meetings/speakers.rs` | **DEFER** — depends on 19 |
| 21 | Canonical-transcript provenance on every AI-derived object | `meetings/summary/`, `entities/`, `memory/`, `relationships/` | **CHANGE**, after 15 |
| 22 | TTS abstraction | *no module exists* — see §10 | **CHANGE**, but first record that Talkback was removed |
| 23 | Full-duplex state model, barge-in, cancellation | depends on 10 and 22 | **DEFER** |
| 24 | Cloud STT adapter | `capture/stt.rs` behind the trait from 12 | **DEFER** — optional acceleration only; Decision 4 and NFR-3 make local the baseline |
| 25 | Reconcile `default = ["whisper-local", "parakeet"]` with the `parakeet` feature's own comment six lines below it — "Off by default until the Windows bundling question above is answered" | `native/src-tauri/Cargo.toml:101` vs `:108` | **CHANGE** (one line of code, or one line of comment — but they cannot both stand) |
| 26 | Reconcile Decisions 47–56 / `maybe_later.md` §§1–3 / FR-2.4 with the absent Talkback code | `docs/decisions.md`, `maybe_later.md`, `docs/requirements.md` | **CHANGE** |
| 27 | Remove or claim `LLMClient::complete_streaming` | `providers/mod.rs:1143` | **MEASURE FIRST** — keep it if Stage 11 restores streaming TTS; delete it otherwise |

**KEEP**, explicitly, with no change proposed: everything in §12.

## 14. Questions that must be measured before coding

Stage 1 exists to answer these. None of them currently has a number.

**Accuracy**

1. What is meeting-path WER on clean English, Indian English and Hinglish —
   measured through `Segmenter` + the live worker, not through
   `VadConfig::process` on a whole clip?
2. How much does the `ScriptTracker` cheap profile cost in WER, and on which
   languages? It is applied today on script detection alone.
3. How often does `speech_health` reject a real utterance (false positive),
   and how often does a hallucination survive it?
4. How much speech never reaches a decoder at all — segments under
   `MIN_SPEECH_MS`, plus queue drops, as a fraction of recorded speech time?
5. Does pinning the language beat auto-detection on a real bilingual meeting,
   and by how much? `decode_history.rs` was built to answer exactly this and
   has no meeting-path feed.
6. How often are numbers, dates and proper nouns wrong? Nothing measures this.

**Latency and throughput**

7. What is `decode_rtf` and `pipeline_rtf` for each model tier
   (`base` / `small` / `large-v3-turbo`) on a representative machine?
8. At which tier does `pipeline_rtf` cross 1.0 — the only threshold that
   decides whether a backlog grows at all?
9. What is the real distribution of finalization latency, and how much of it
   is the 400 ms hangover versus queue wait?
10. What does `persist_ms` look like at segment 50 versus segment 800?
11. How often does the single `SttEngine` model slot thrash when dictation and
    a meeting overlap (`model_reloads`, `lock_wait_ms`)?

**Reliability**

12. Does a 90-minute meeting hold steady memory? The audio channel and both
    device FIFOs are unbounded.
13. What is the actual worst-case checkpoint write latency on a loaded
    machine, and can it stall the pump long enough to matter?
14. How often does overlapping speech occur in real recordings? This decides
    whether per-channel decoding (§13.18) is worth double the decode cost.

**Platform**

15. Does `ort` bundle and run in a real Windows Tauri binary
    (`docs/spikes/onnx-windows.md`)? Two deferred items (§13.19, §13.20) and
    the Parakeet default-feature question all hang on it.

---

## Appendix: environment limitations at the time of this audit

This audit was written in a Linux container, which matters for every stage
after this one because it decides what can be verified without a Windows
machine.

| Gate | Result |
|---|---|
| `npm run verify:rules` | passes |
| `npx tsc --noEmit` | passes |
| `npm test` (vitest) | passes — 46 files, 650 tests |
| `cargo clippy --all-targets --no-default-features --features whisper-local -- -D warnings` | passes, clean |
| `cargo test --no-default-features --features whisper-local` | 987 passed, 1 failed |
| `cargo clippy --all-targets` (default features) | **cannot run** |

Three things a later stage needs to know:

1. **The GTK/ALSA/WebKit development packages are not present by default.**
   `.github/workflows/ci.yml`'s "Install system dependencies" step installs
   them and is the fix; without it `gdk-sys` cannot configure and nothing
   compiles.
2. **The default feature set cannot be built without network access to
   `cdn.pyke.io`.** `default = ["whisper-local", "parakeet"]`, `parakeet`
   pulls `ort`, and `ort-sys` downloads a prebuilt ONNX Runtime at build time.
   Behind a restrictive proxy that download 403s and the build fails. This is
   the same shape as the build-time FFmpeg download `docs/meetings.md` names
   as a Meetily defect — worth noting alongside audit §13.25, which is about
   the same feature flag being on by default while its own comment says it is
   off.
3. **One test fails for environmental reasons, unrelated to speech.**
   `actions::dispatcher::tests::test_read_only_action_executes_without_confirmation`
   (`src/actions/dispatcher.rs:108`) asserts that dispatching an `OpenUrl`
   action succeeds; in a headless container there is no browser to open, so
   the `open` call fails. It is pre-existing on this commit and has nothing to
   do with the speech stack.

Any measurement in §14 has to be taken on a real machine with those
dependencies — and the latency and throughput numbers specifically have to be
taken on Windows, because that is the only platform Vox ships to.
