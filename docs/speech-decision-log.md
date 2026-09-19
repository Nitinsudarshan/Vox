# Vox — Speech Architecture Decision Log

Append-only, and scoped to the speech stack: capture, segmentation, turn
state, STT, the transcript layer, speakers, TTS, and anything downstream that
consumes speech. Same rules as [decisions.md](decisions.md) — an entry is
never edited to match later reality; a reversal gets a new entry saying so.

Ids are `D-0NN` and are **not** related to `decisions.md`'s numbering. Where a
decision here restates one already recorded there, the entry says so rather
than claiming it as new.

A decision earns a place here only once the repository can justify it — either
because the code already works that way and the reasoning should be written
down, or because a measurement in the repo supports it. Everything else lives
in [speech-architecture-audit.md](speech-architecture-audit.md) §13 as a
proposal, and in §"Reserved, not yet decided" below as a reserved id.

---

### D-001 — Raw audio is the ultimate source of truth

- **Context**: A meeting produces audio, a transcript, a speaker map and a
  report. Three of those four are derived, and only one cannot be regenerated.
- **Decision**: The durable recording is authoritative. Every other meeting
  artifact is a derivation that may be rebuilt from it, and no derivation may
  be allowed to cost audio.
- **Already true in the code**: `meetings::checkpoint` writes a 30 s WAV chunk
  throughout the recording; `store::recover_interrupted` re-derives a crashed
  meeting's duration from the chunks that reached disk rather than from the
  clock the dead process was keeping; `import::retranscribe` regenerates a
  whole transcript from `audio.wav`; `import::fingerprint_segments` re-derives
  speaker evidence from it too.
- **Consequence**: Every later stage may treat re-decoding as cheap and
  re-recording as impossible.
- **Restates**: Nothing in `decisions.md`; this was implicit in Decision 67's
  durable-checkpoint design and is now explicit.

---

### D-002 — Speech recognition is a derived artifact, and a failing decoder may not stop a recording

- **Context**: A decoder slower than real time, a missing model, a panicking
  worker — none of these are reasons to stop capturing audio.
- **Decision**: The transcription path is downstream of, and subordinate to,
  the recording path. It may drop work; it may not apply back-pressure to the
  recorder.
- **Already true in the code**: `TranscriptionQueue::submit`
  (`meetings/transcription.rs`) uses `try_send` on a `sync_channel`, so a full
  queue refuses a segment rather than blocking the caller — and its caller is
  `run_pump`, the same thread that writes checkpoints. A blocking send there
  would make audio durability a function of decode speed.
- **Consequence**: Independent failure domains. The counterpart obligation is
  D-004.
- **Known gap on this commit**: the reverse direction is not yet safe. The
  mixer→pump channel (`meetings/capture.rs:171`) is an *unbounded*
  `std_mpsc::channel`, so a stalled pump grows memory without limit. See the
  audit's §7.2.

---

### D-003 — A meeting that could not install a speech model is refused before anything is recorded

- **Context**: Recording audio that can never become a transcript is a failure
  the user only discovers when the meeting is over.
- **Decision**: `MeetingEngine::start` resolves the model path first and
  returns `NoSpeechModel` before opening a device, deleting the half-created
  meeting directory on the way out.
- **Already true in the code**: `meetings/engine.rs:start`, with the test
  `starting_without_a_speech_model_is_refused_before_anything_is_recorded`.
- **Tension worth recording**: this is in direct tension with D-001 — it is the
  one place where an STT prerequisite is allowed to stop a recording. It is
  accepted because it fails *before* any audio exists, so nothing is lost.

---

### D-004 — Nothing is dropped silently

- **Context**: A bounded queue must shed load. A bounded queue that sheds load
  invisibly is worse than an unbounded one, because the user reads a complete
  transcript that is missing sentences.
- **Decision**: Every loss is counted, surfaced at the time, and persisted on
  the meeting record.
- **Already true in the code**: `MAX_QUEUED_SEGMENTS = 64`; a refused segment
  increments `dropped`, logs, and emits `meeting-transcription-warning`; a
  failed decode does the same with a `decode` kind; `Meeting::dropped_segments`
  is written on stop and rendered in `MeetingView.tsx:199`. Sequence numbers
  are assigned at submit so a drop leaves a *visible gap* in the transcript's
  numbering rather than a renumbering that hides it.
- **Known gaps on this commit**, both recorded in the audit §4 rather than
  fixed here: checkpoint failure (`engine.rs:527`, `:545`) logs and does not
  warn, and a microphone that fails while system audio opens produces no
  warning at all. Both are audio-side losses, which makes them the *more*
  serious half of this decision, not the less.

---

### D-005 — Transcript order is structural, not sorted

- **Context**: Whisper decode time varies by an order of magnitude with
  segment length, so any parallel decoder reorders the transcript relative to
  the conversation.
- **Decision**: One serial decoder thread, with the sequence number assigned
  at submit rather than at completion. Ordering is a property of the pipeline;
  the sort on read is a backstop, not the mechanism.
- **Already true in the code**: `transcription::run_worker` is a single
  thread over `Receiver<DecodeJob>`; `store::load_transcript` sorts and dedups
  by sequence on read so even a torn write reads back ordered.
- **Consequence**: Throughput is explicitly not the optimization target on the
  live path. A faster pipeline comes from a faster model or a second pass, not
  from parallel decoding.

---

### D-006 — Confidence is measured or absent, never invented

- **Context**: The obvious cheap "confidence" — a function of transcript
  length — is a number that looks like evidence and is not. Meetily ships
  exactly that (`(text.len() / 100.0).min(0.9) + 0.1`) and filters on it.
- **Decision**: `TranscriptSegment` carries Whisper's own `no_speech_prob` and
  nothing else. Where a model exposes no true confidence for a field, the
  field is named for what it actually is — evidence, voiced ratio, no-speech
  probability — rather than dressed up as confidence.
- **Already true in the code**: `meetings/model.rs:TranscriptSegment`;
  `speech_health::DecodeEvidence` carries `voiced_seconds`, `total_seconds`
  and `mean_no_speech_prob`, all measured.
- **Consequence**: Any future provider abstraction must be able to say "this
  provider reports no confidence" rather than synthesizing one.

---

### D-007 — Channel attribution is measured and precedes any diarization

- **Context**: The microphone and the system loopback are two separate
  streams. Which side of a call a line came from is therefore known for free,
  at capture time, exactly.
- **Decision**: Keep per-channel energy alongside the mixed window and derive
  `SegmentChannel` from it. Never ask an acoustic clustering step to
  rediscover information the capture layer already had.
- **Already true in the code**: `MixedAudio` carries `mic` and `sys` beside
  `mixed`; `segmenter::classify_channel` applies a 3× dominance ratio;
  `speakers::assign_speakers` clusters the two channels **separately** and
  never clusters the microphone channel at all.
- **Restates**: `decisions.md` Decision 68, from the speech stack's side.

---

### D-008 — Speaker identity is a proposal until the user confirms it

- **Context**: The grouping technique is 26 MFCC statistics per turn clustered
  by average linkage — honest, and not identity-grade.
- **Decision**: Vox proposes groups, plays a sample of each, and the user
  names them. A name the user typed survives reprocessing; Vox's own
  `Speaker N` placeholders do not. A turn shorter than 1.2 s gets no
  fingerprint and stays unattributed rather than being guessed.
- **Already true in the code**: `voiceprint.rs`, `speakers.rs`,
  `Speaker::named_by_user`, `MIN_VOICEPRINT_SECONDS = 1.2`.
- **Restates**: `decisions.md` Decision 68 and `maybe_later.md` §11.

---

### D-009 — A model is never asked to do what a deterministic projection can do

- **Context**: One model call was once asked for a translation and a
  romanization in the same object, and a local model answered both with the
  same string — so the Romanized view showed English.
- **Decision**: Romanization is a Devanagari→Latin state machine, computed
  offline. English comes from a second Whisper pass over the *audio* in
  translate mode; an LLM fills only what that pass left empty, in validated
  batches, and a "translation" that is merely the romanization handed back is
  refused.
- **Already true in the code**: `capture::romanize`,
  `import::generate_english_track`, `variants::accept_translation`,
  `variants::align_english_spans` (aligned by time, not by sequence).
- **Consequence**: The same principle bounds any future glossary work — see
  the audit §13.17. A glossary that rewrites text a model did not say is the
  same failure in a different costume, and today's glossary is applied exactly
  that way, as a post-decode edit in `text_normalize::normalize_segment_text`.

---

### D-010 — Capture health and transcription health are separate questions

- **Context**: "Is this recording working?" and "is the transcript keeping
  up?" fail for different reasons, are fixed by different actions, and were
  historically reported as one status.
- **Decision**: Report them separately. The recorder reports which devices
  opened, which are still delivering callbacks, and whether each has *heard*
  anything above the silence floor. The transcription path reports queue
  depth, completions and drops.
- **Already true in the code**: `MeetingRecordingStatus` carries
  `microphone_active` / `system_audio_active` (stream alive) and
  `microphone_heard` / `system_audio_heard` (anything above
  `AUDIBLE_RMS_THRESHOLD`) *and*, separately, `segments_queued` /
  `segments_completed` / `segments_dropped` plus `devices: OpenedDevices`.
- **Consequence**: "The microphone bar never moved" becomes actionable without
  leaving the app.

---

### D-011 — Pipeline stages are timed separately, because a single total cannot name a cause

- **Context**: End-to-end latency hides its own cause. A long queue wait, a
  contended model lock, a model reload and a slow decode produce the same
  total and need four different fixes.
- **Decision**: Measure each phase of a segment's journey as its own number:
  `queue_wait_ms`, `lock_wait_ms`, `model_load_ms` (plus whether a reload
  happened), `decode_ms`, `post_ms`, `persist_ms` — and at meeting level
  `decode_rtf` (against speech) alongside `pipeline_rtf` (against wall clock),
  because only the latter decides whether a backlog grows.
- **Already true in the code**: `transcription::SegmentTiming`,
  `TranscriptionStats`, `print_segment_trace`, `print_meeting_summary`,
  `projected_drain`.
- **Known gap on this commit**: all of it is `println!` to a terminal. Nothing
  is persisted, so no question about behaviour over time can be answered — the
  same gap `capture/decode_history.rs` was built to close for dictation and
  which the meeting path has no feed into.

---

### D-012 — Cloud STT is optional acceleration, never a dependency

- **Context**: Vox is local-first by constitution, not by preference.
- **Decision**: Every core speech capability must work with no network. A
  cloud provider may be added as an alternative path; it may never become the
  path a meeting requires.
- **Already true in the repo**: `decisions.md` Decision 4 ("every
  cloud-optional feature must function fully at $0 recurring cost using local
  STT"), NFR-1 and NFR-3 in `requirements.md`, and `docs/meetings.md`'s
  statement that Vox takes no cloud STT from Meetily — "transcription is
  always local".
- **Restates**: Decision 4, scoped to speech.

---

### D-013 — Whisper's output over silence is screened, and a rejection records its reason

- **Context**: Whisper has no way to say "nothing was said". Handed room tone
  it emits subtitle boilerplate, and its own output then conditions the rest
  of the window into a loop.
- **Decision**: Two separate defences. Before the decode, measure how much of
  the span is actually voiced, at 20 ms resolution against the span's own
  noise floor. After the decode, reject text no plausible speech could have
  produced. A rejection replaces the text with nothing and records what was
  discarded and why — it never rewrites it.
- **Already true in the code**: `capture/speech_health.rs`
  (`profile_speech`, `assess`, `screen_decode`, `TranscriptRejection`), applied
  to every meeting segment in `transcription::decode_segment`.
- **Known gap on this commit**: the rejection reason reaches `tracing::debug!`
  and a `discarded` counter that is printed and not stored, so hallucination
  *rate* is not measurable after the fact.

---

### D-014 — A meeting transcript is data handed to a model, never instructions

- **Context**: Anyone on a call can say "ignore your previous instructions",
  and a transcript is full of imperative sentences.
- **Decision**: The transcript reaches a model framed as data inside an
  unguessable delimiter rather than filtered for dangerous phrases.
- **Already true in the code**: `pipeline::source_boundary`, used by
  `summary::service`.
- **Restates**: `rules/untrusted-input.md` and `decisions.md` Decision 65
  ("A Tier 2 Rewrite May Never Touch a Meeting Transcript"), scoped to speech.
- **Consequence**: This holds unchanged under any canonical-transcript
  refactor — the boundary wraps whatever rendering is handed over.

---

### D-015 — Long-form recordings are the primary accuracy benchmark, and the benchmark drives the meeting path

- **Context**: Vox already had an accuracy harness, and it measured the wrong
  pipeline over the wrong material. `capture::evaluation` scores the dictation
  path — one clip, `VadConfig::process` over a whole buffer, one decode — across
  a corpus of 35 dictation-length clips. A meeting is none of those things: it
  is cut into spans by `meetings::segmenter`, those spans queue against a
  bounded channel, and one decoder works through them in order.
- **Decision**: A second harness, `meetings::benchmark`, which drives the
  production `Segmenter`, `speech_health`, `text_normalize` and a queue of the
  same depth and discipline as the live one. Its corpus is organised into
  twelve named conditions, each with a minimum duration — 30 minutes for a
  meeting, 5 for long-form speech, 3 for everything else — and a case below its
  floor is reported as `UnderLength` rather than scored.
- **Reason**: The failures worth measuring do not exist in a clip. Segmentation
  cutting a clause in half, a backlog that grows until speech is dropped,
  repetition at a forced split, a noise floor that drifts over an hour — none
  of them can occur in ten words, and all of them are what a meeting recorder
  is actually judged on. On top of that, a word error rate over a short
  reference moves several points on one misheard clause, so two engines
  separated by noise read as separated by quality.
- **Alternatives considered**: Extending `capture::evaluation` (rejected — its
  shape is one clip, one decode, and a meeting is neither; the two harnesses
  answer different questions and should not be one function with a mode flag).
  Synthesising a corpus with text-to-speech (rejected — it would measure the
  synthesiser, not a room, and its failure modes are not a meeting's).
- **Impact**: `meetings/benchmark/`, four `speech_benchmark_*` commands, and
  `docs/testing.md` §1b. No audio ships: real meetings belong to the user, and
  the manifest names what to supply rather than inventing it. Until a corpus
  exists, every accuracy claim about Vox's transcription is an impression.
- **Consequence for D-006**: the harness carries the no-fake-confidence rule
  into measurement. An engine that reports no per-decode no-speech probability
  records `None` and is not screened on a number it never produced — Parakeet
  is such an engine, and its adapter declares it rather than leaving it to be
  discovered from a report that looks comparable.

---

### D-016 — Word error rate is scored case-insensitively, because Vox adds the casing

- **Context**: `text_normalize` gives every transcript line sentence casing and
  terminal punctuation. A ground-truth reference is written by hand and has
  neither. `normalize_for_eval` stripped punctuation and preserved case, and
  `calculate_accuracy` compares words with `==`.
- **Decision**: Fold case in `normalize_for_eval`, so both harnesses score the
  same way.
- **Reason**: Every run was being charged a word error for capitalization
  nobody typed and no model got wrong. A roughly constant penalty is worse
  than a large one, because it looks like signal: it survives every
  comparison and shifts every absolute number. The technical-term check in the
  same function already lowercased both sides, so the intent was always
  case-insensitive matching and only half the function did it.
- **Impact**: Word error rates from `capture::evaluation` computed before this
  change are not comparable with ones computed after. Nothing user-facing reads
  them, and no baseline had been recorded.

---

### D-017 — Telemetry is persisted, and persisted as a log rather than a document

- **Context**: D-011 recorded that the pipeline's phases are measured
  separately. They were — and then printed to stdout with `println!` and
  dropped. Nothing about behaviour over time was answerable, which is what
  every performance question actually is.
- **Decision**: Two files per meeting. `diagnostics.jsonl` gets one appended
  line per decoded segment; `diagnostics.json` gets the rollup, written once
  on stop.
- **Reason for the split**: they are different kinds of thing. A per-segment
  record is a log entry — appending must cost the same at segment one thousand
  as at segment one, which rewriting a growing file does not, and a crash
  should keep every line written before it. A rollup is a document, so it
  goes through `write_atomic` like every other document in the vault. A torn
  final line in the log is skipped on read: for a transcript that would be
  unacceptable, and for telemetry about a run that has already crashed it is
  precisely the right trade.
- **What is excluded, and why**: transcript text (`text_chars` only — a second
  copy of meeting content is a second thing to leak and a second thing to
  delete), audio, and speaker attribution (`transcript.json` already carries
  `speaker_id` against the same `sequence`; two copies of one fact is how they
  come to disagree, which is the defect `model.rs` calls out in Meetily's
  never-written `speaker` column).
- **Impact**: `meetings/telemetry.rs`, four store methods, the worker writing
  a record per segment, the rollup written on stop, `MeetingDetail.diagnostics`
  and a panel on Meeting Detail. A diagnostics write that fails is logged and
  the meeting carries on — trading a recording for the notes about a recording
  is the wrong way round.
- **Not covered**: `import::run_batch`, so re-transcription and import write no
  diagnostics. They have no clock to fall behind, so backlog and drops cannot
  occur there; the accuracy-side evidence is genuinely missing and is recorded
  as such in `docs/meetings.md`.

---

### D-018 — A model path is never written into a diagnostics file

- **Context**: `TranscriptionStats` carried `model_path`, and the obvious thing
  to record is what the worker was configured with.
- **Decision**: Diagnostics record the model's *filename*.
- **Reason**: A diagnostics file exists to be shared — attached to a bug
  report, pasted into a thread. An absolute path names a machine and, on
  Windows, almost always a person. The filename is the whole of what a reader
  needs to know which model ran.
- **Consequence**: the same rule applies to the benchmark's `EngineDescriptor`,
  which already followed it, and to anything later that reports what produced
  a transcript.

---

### D-019 — A bounded loss beats an unbounded one, and the loss is counted

- **Context**: The decode queue was carefully bounded and the audio channel
  feeding it was not. `meetings/capture.rs` used an unbounded `std_mpsc`
  channel from the mixer to the pump, and both device FIFOs were unbounded
  `VecDeque`s. A pump that stalls — a slow disk during a checkpoint write —
  grew memory with nothing shedding load and nothing reporting depth.
- **Decision**: The channel holds ten seconds of audio and the mixer uses
  `try_send`; past that a block is shed and counted. Each device FIFO holds a
  minute and discards its oldest samples, counted. Both totals reach
  `CaptureHealth::audio_lost_seconds`.
- **Reason**: Unbounded is not the safe option, it is the option whose failure
  is worst. Against an unbounded channel a stalled pump grows memory until the
  process dies, and that costs every second recorded since the last
  checkpoint. Shedding twenty milliseconds costs twenty milliseconds. Neither
  is good; only one of them is bounded, and only one of them can be reported.
- **Why the mixer sheds rather than blocks**: blocking would push the backlog
  one level up into the device FIFOs, which the audio callbacks write to. The
  shed keeps the mixer draining, which is what keeps those FIFOs empty.
- **Why the oldest samples go**: what someone just said matters more than what
  they said a minute ago.
- **Consequence**: `audio_lost_seconds` should be 0.0 on every ordinary
  recording. A non-zero value is the most serious thing in the diagnostics: it
  is audio that is in neither the recording nor the transcript, and nothing can
  regenerate it.

---

### D-020 — Audio warnings travel on their own channel, and both directions of device failure are reported

- **Context**: The audit found the pattern that transcript loss was loud and
  audio loss was silent. A checkpoint writer that could not be created logged
  at `error` and the recording continued with nothing being saved; a failed
  checkpoint write did the same; and a microphone that failed while system
  audio opened produced no warning at all, while the reverse case had one.
- **Decision**: A `meeting-recording-warning` event, separate from
  `meeting-transcription-warning`, carrying `microphone`, `system_audio`,
  `audio_storage` or `audio_shed`. `run_pump` is given an `AppHandle` so it can
  speak. The microphone-failed case gets the warning the system-audio case
  already had.
- **Reason for the separate event**: they are different failures with different
  remedies, and the audio one is the more serious. Mixing them would put "this
  meeting is not being recorded" in the same stream as "one line is missing".
- **Reason for warning during the meeting**: it is the only point at which the
  user can do anything — change device, free disk, restart. A log line
  discovered afterwards is a post-mortem.
- **Warned once, not per failure**: a full disk fails every write, and one
  warning per 30 seconds of audio would bury the first.
- **Deliberately not done**: separate `mic.wav` / `system.wav` files. Writing
  per-channel audio triples the disk a meeting costs, and the only thing it
  buys is per-channel decoding, which the audit classified MEASURE FIRST
  (§13.18) because it doubles decode cost for a benefit nobody has measured.
  Per-channel *energy* — which is all channel attribution needs — is already
  kept.

---

### D-021 — Turn detection is a subsystem, not a side effect of buffering audio

- **Context**: `segmenter.rs` decided whether a turn had ended while it was
  deciding which samples to hand a decoder. The algorithm was correct and the
  entanglement cost three things: the state was not observable, the detector
  was not reusable, and turn timing read as a property of transcription.
- **Decision**: `meetings/speech_state.rs` owns the decision —
  `Silence → PossibleSpeech → Speaking → ProbableEnd → Finalized`, the
  thresholds, the adaptive noise floor and both hysteresis counters. The
  segmenter keeps the pre-roll, the open frames and the ceiling, and asks the
  machine what each frame means.
- **Reason it is an extraction and not a rewrite**: the algorithm was already
  right. Every constant, comparison and transition is carried over unchanged,
  and the segmenter's seventeen existing tests — breath versus real pause,
  noisy room, quiet voice, ceiling split, ragged buffers — pass untouched.
  That is the evidence, not the claim.
- **Why `Finalized` is a state**: it is reported for exactly the frame a turn
  closes on and never after, so a consumer can act on the edge without
  diffing two observations.
- **Impact**: `Segmenter::speech_state()`, `SpeechSegment::end_reason` and
  `hangover_ms`, `TranscriptionHealth::hangover_p50_ms`. Nothing built on top
  of it: no barge-in, no interruption, no full duplex.

---

### D-022 — No model decides that a sentence has ended

- **Context**: End-of-turn looks like a language problem — whether a clause is
  complete is a question about meaning — and a model could be asked.
- **Decision**: Energy against an adaptive noise floor, frame continuity and
  two hysteresis counters. No LLM, no network, no ASR.
- **Reason**: It is a control problem answered in milliseconds by arithmetic,
  and it is the one decision in the pipeline that has to be instant. Routing it
  through a model adds a round trip and a cost to the step whose whole job is
  to be immediate, and makes the pipeline stop working when the model does.
  Recent end-of-turn research points the same way — acoustic and prosodic
  evidence gives the better accuracy/latency trade, and text understanding does
  not automatically improve the decision.
- **Consequence, and the reason it is worth recording**: it puts a floor on
  latency that no model upgrade removes. A 400 ms hangover means no word can be
  decoded sooner than 400 ms after it was said, and `hangover_p50_ms` is
  reported next to the finalization percentiles so that tuning aims at the
  right number. Shortening it is a real option with a real cost — a shorter
  hangover fragments sentences at breaths — and it is now a measured trade
  rather than a guess.

---

### D-023 — An engine declares what it can do; the interface does not assume

- **Context**: Vox ran two engines and they sat side by side inside
  `SttEngine` as two methods with different shapes. The seam was real; what
  was missing was a name for it.
- **Decision**: `capture::recognizer::SpeechRecognizer`, with a
  `RecognizerCapabilities` that each engine declares — batch, streaming,
  timestamps, language handling, translation, no-speech evidence, partial
  transcripts, local, network.
- **Reason this is not one uniform result type**: the tempting design forces
  every engine to look like the weakest one, or to pretend. Parakeet reports no
  per-decode no-speech probability. A uniform interface would have it return
  `0.0`, and the hallucination screen — tuned against Whisper's probability —
  would then be grading it on a number it never produced. Its language is
  likewise fixed at English, and handing an English-only model Hindi audio does
  not fail: it returns fluent nonsense in English phonology, which is worse
  than an error because nothing about it looks broken.
- **Three-valued support**: `Yes`, `No`, `Unknown`, and `Unknown` is never
  usable. A provider table full of `no` where the honest answer is "nobody
  measured this" reads as a comparison and is not one.
- **Impact**: `capture/recognizer.rs`, `capture/recognizers.rs`, and the
  benchmark folded onto it — its private `BenchmarkDecoder` is deleted rather
  than left beside its replacement. An engine's declared limits now travel into
  the benchmark report with it, so a comparison states what one side could not
  do. `docs/speech-providers.md` is the table.
- **Deliberately not done**: the production decode paths still call
  `SttEngine`. Changing what decodes a meeting changes what users get, and
  belongs with the measurement that justifies it rather than arriving as a
  refactor. Stage 6's live/final split is where the interface gets its reason.

---

### D-024 — Task, then capability, then provider, then model

- **Context**: "What is the best speech model for Vox" has no stable answer —
  the models change every few months — and answering it once bakes that answer
  into the architecture.
- **Decision**: `SpeechTask` names what Vox needs recognition *for*, and
  `SpeechTask::requires` says what a task cannot do without. An engine is
  eligible when it satisfies those; choosing between eligible engines is a
  measured comparison, not a ranking in code.
- **Reason**: push-to-talk wants latency and needs no timestamps; a live
  meeting must average faster than a clock; a final pass has no clock and wants
  accuracy. Three different requirements over the same audio. Naming them makes
  "should we switch to X" a question with a procedure rather than an opinion.
- **What this makes structural**: `PushToTalk` and `LiveMeeting` require
  `offline`, so a networked engine is excluded from them by the type rather
  than by a convention someone has to remember. `FinalPass` does not require
  it, because a final pass has no clock and a user who wants to spend money on
  one meeting's accuracy is making a reasonable choice. This is D-012's
  local-first rule expressed in code.
- **Consequence**: Deepgram, or any other cloud STT, is a benchmark entry and
  an optional provider. Nothing about the meeting pipeline requires one to
  exist.
- **`engines_for` deliberately does not rank.** It filters and preserves the
  caller's order. Ranking engines without measuring them is exactly how "the
  best model" becomes the architecture.

---

### D-025 — A provider choice cannot express a credential

- **Context**: `RecognizerChoice` is serialized, stored in settings, and logged
  in a benchmark report. The obvious shape for a cloud provider is a variant
  holding an endpoint and an API key.
- **Decision**: The enum carries a model path or a model directory and nothing
  else. A cloud adapter reads its key through the keyring path the rest of Vox
  uses.
- **Reason**: structural rather than a review rule. A secret cannot reach a
  settings file, a log line or a shared benchmark report by being storable
  there, because there is nowhere to store it. A test asserts the serialized
  form contains no key or token field.

---

### D-026 — Live speed and final accuracy are separate targets, and which one ran is recorded

- **Context**: Vox already decoded a live meeting and a re-transcription
  differently — Balanced against Quality, the encoder clamp on against off, a
  cheap profile for expensive scripts on the live path only. Nothing recorded
  which of them had produced the transcript on screen, so "why is this worse
  than last time" had no starting point.
- **Decision**: `TranscriptionPass` and `TranscriptProvenance` on the meeting —
  pass, engine, model filename, language actually pinned, decode profile, and
  when it finished. Meeting Detail shows which pass produced what it is
  displaying.
- **Reason the two passes differ at all**: the clock. A live decode races one,
  so a decoder slower than real time builds a backlog and eventually drops
  speech. A final pass has no clock — the recording is on disk — so it can
  afford a wider beam and a bigger model.
- **Replaces `Meeting::transcript_model`**, which held a full filesystem path
  and which nothing ever read. A meeting record gets exported; a path names a
  machine and usually a person (D-018).
- **Idempotence**: the final pass truncates and decodes from sequence zero, so
  running it twice with the same inputs produces the same transcript rather
  than two copies of it.

---

### D-027 — The live transcript is kept when a final pass replaces it

- **Context**: `run_batch` truncated `transcript.json` before decoding
  anything, and `retranscribe`'s only copy of the previous transcript was in
  memory. A crash part-way through therefore left a truncated transcript and
  nothing to restore from — the audit recorded this as F9.
- **Decision**: `transcript.live.json`, written once before the first final
  pass.
- **Written once, not per pass**: a second final pass would archive the first
  final one over it, and the live transcript is the thing worth comparing
  against.
- **Never written empty**: `MeetingStore::create` writes an empty
  `transcript.json`, so archiving unconditionally would fill the once-only slot
  with nothing and hide the live transcript from every later pass. This was a
  real bug in the first implementation, caught by a test whose premise was
  wrong for an interesting reason.
- **Two jobs**: a crash mid-re-transcription now costs nothing, and "the new
  transcript is worse than the old one" is a comparison rather than a memory.

---

### D-028 — Raw ASR evidence and the canonical transcript are separate layers

- **Context**: `transcript.json` is written by five producers — the live
  worker, re-transcription, the English pass, romanization and speaker
  detection — so "what did the model actually say" and "what does Vox believe
  this means" were the same bytes.
- **Decision**: `meetings::canonical`. A `TranscriptSegment` is evidence about
  a span of audio; a `CanonicalSegment` is derived from it. The assembler is a
  pure function that borrows the raw segments and never modifies them, and a
  test pins that.
- **Reason**: every step between the two — ordering, joining, de-duplicating,
  labelling, choosing a rendering — is a decision that could be made
  differently tomorrow. Baking them into the evidence means a later change to
  any one of them cannot be distinguished from the model having said something
  different.
- **Provenance**: every canonical segment carries `sources`, the raw sequences
  behind it, so a claim in a report walks back to a span of audio. Two lines
  joined into one name both.
- **Impact**: the report path now reads canonical rather than raw.
  `TranscriptSegment` gained `cut_at_ceiling`, which the segmenter knew and
  discarded — without it nothing downstream could tell a sentence split across
  two lines from two sentences.
- **Not done here**: speaker detection still writes `speaker_id` onto the raw
  record. That is interpretation and does not belong there; moving it is Stage
  8's business, where the speaker pipeline is the subject rather than a
  dependency. The English and romanization passes stay, and that is
  deliberate — a second Whisper pass over the same samples is another decode,
  and romanization is a deterministic projection of words already present.
  Both are evidence about the audio; an attribution is not.

---

### D-029 — A gap in the transcript is stated, not closed over

- **Context**: Sequence numbers are assigned before a decode, so a dropped or
  failed segment leaves a hole in the numbering. `Meeting::dropped_segments`
  counted them and nothing recorded *where* they were — the audit's R1.
- **Decision**: The assembler reports holes as `TranscriptGap`s, with the
  lines either side and the seconds they span, and `render_transcript` marks
  each one in the text a summarizer reads.
- **Reason**: a transcript that silently omits ninety seconds reads as a
  complete record of a meeting in which nobody spoke for ninety seconds. That
  is a different and worse claim than "this part is missing", and a model
  handed the first one will summarize a conversation that did not happen.
- **Consequence**: `CanonicalTranscript::is_complete` is answerable, which is
  what a downstream consumer needs before it asserts anything about what was
  decided in a meeting.

---

### D-030 — A glossary correction is evidence, and names are exempt from guessing

- **Context**: The glossary was a find-and-replace. Any token within one edit
  of a term was rewritten in place, inside the text normalizer, before the
  segment reached disk, with nothing recording what changed or from what.
- **Decision**: `capture::glossary`. A correction records what the decoder
  said, what it became, which term did it and why, and those travel with the
  segment. Terms carry a category, and **a near miss is never corrected to a
  person's name**.
- **Reason for recording**: if the decoder heard "supabse" and the glossary
  says "Supabase", something went right. If it heard a real word one edit from
  a term, something went badly wrong — and under blind replacement the
  transcript asserts the glossary's word with nothing to indicate otherwise. A
  correction that cannot name its own evidence is indistinguishable from a
  transcription.
- **Reason names are exempt**: they are short, numerous, and collide with
  ordinary words — "Marc" and "mark", "Bill" and "bill", "Rose" and "rose".
  Getting one wrong changes who a meeting says made a commitment, which is the
  most consequential thing a meeting transcript asserts. Casing is still
  fixed, because casing changes no words.
- **Not a spell-checker, deliberately**: the rule is one edit, so "banglore" —
  four edits from "Bengaluru", obvious to a human — is left as the decoder
  said it. A rule loose enough to catch it is loose enough to rewrite words
  nobody meant, and that failure is silent. The narrower rule leaves more
  errors in and puts none in.
- **Consequence**: this satisfies "preserve the raw ASR text" as a property of
  the data rather than a second copy of every line. `uncorrect` applies the
  corrections backwards and reconstructs the decode exactly.

---

### D-031 — Speaker attribution is not part of the transcript

- **Context**: `speakers::assign_speakers` took `&mut [TranscriptSegment]` and
  wrote `speaker_id` onto each one, so `detect_meeting_speakers` rewrote
  `transcript.json`. Re-running an interpretation modified the record of the
  decode.
- **Decision**: attribution is its own record — `attribution.json`, a map from
  raw sequence to speaker id — and the assembler joins it. `assign_speakers`
  now takes the segments by shared reference, and a test asserts the
  transcript is byte-identical afterwards.
- **Keyed by sequence, not by index**: a re-transcription renumbers from zero
  and produces a different number of lines. An index would then point at a
  different sentence; a sequence points at nothing, which is correct.
- **Why it matters beyond tidiness**: it makes "a speaker rename must never
  mutate the raw transcript" structural rather than a rule someone has to
  remember, and it means a detection run that finds nothing returns nothing
  instead of leaving the previous run's guesses in place.

---

### D-032 — The speaker encoder gets a seam and no second implementation

- **Context**: MFCC statistics are the weak link in the speaker pipeline and
  the docs have always said so. A trained encoder (x-vector, ECAPA) is
  markedly better and needs an ONNX runtime and a model file, which
  `docs/spikes/onnx-windows.md` has not established can be bundled into a
  Windows build.
- **Decision**: `voiceprint::SpeakerEncoder`, with one implementation
  (`MfccEncoder`, id `mfcc-26`). No second one, no ONNX dependency, no
  abstraction over a thing that does not exist.
- **Reason**: the seam costs a trait and makes the eventual swap a slot-in
  rather than a rewrite of `assign_speakers`. Building the second
  implementation before the spike runs would mean an engine that works for
  whoever built it and crashes on an installed app — which is the same trade
  the Parakeet feature flag already records.

---

### D-033 — Long-meeting reliability is tested synthetically, and the decoder is left out

- **Context**: The pipeline's reliability claims are all claims about time.
  None can fail in a five-minute demo. Waiting two hours per test run is not a
  test anybody runs.
- **Decision**: `meetings::endurance` drives the production `Segmenter`,
  `CheckpointWriter` and `MeetingStore` over generated audio of arbitrary
  length, and leaves the decoder out.
- **Reason for leaving the decoder out**: a real decode needs a model this
  repository does not ship and cannot run in CI, and decode speed is what
  `meetings::benchmark` already measures properly. What remains — segmentation,
  checkpointing, queueing, persistence, ordering — is exactly where the
  long-duration failures live.
- **Impact**: five fast tests plus one `#[ignore]`d two-hour run.

---

### D-034 — The transcript's write cost was measured, halved, and then left alone

- **Context**: The audit classified `append_segments` rewriting the whole file
  per segment as MEASURE FIRST. Measured over a synthetic two-hour meeting
  (1,439 lines): the first hundred lines cost 40 ms to persist, the last
  hundred 2,056 ms. A **persistence** bottleneck — not capture, segmentation,
  queue, model or UI.
- **Decision**: cache the parsed transcript in the store, keyed by meeting and
  validated against the file's modification time. Last hundred: 1,458 ms, a
  29% win. Stop there.
- **Reason for stopping**: the size of that win says the remaining cost is
  serializing and writing, not parsing. Removing it means not rewriting the
  file per line — a journal plus a materialised document, the shape the
  diagnostics log already uses — and that is a vault format change. At 1,439
  lines the cost is about 15 ms per segment against a decode measured in
  hundreds of milliseconds: a few percent.
- **The ceiling, recorded so the next person does not have to re-derive it**:
  around 3,000 lines the cost reaches tens of milliseconds per segment and
  starts to take a real share of the decoder's budget. That is an eight-hour
  recording or a very talkative three-hour one. Until one exists, one
  greppable atomically-replaceable file per meeting is the better trade.
- **Why the cache validates against mtime**: the vault is explicitly meant to
  be editable without Vox running. A cache that cannot tell whether it is
  stale must not claim it is fresh, so a file whose modification time the
  platform will not report is simply not cached.

---

### D-035 — Meeting intelligence reads the canonical transcript, and nothing else

- **Context**: The report path was moved onto the assembler in Stage 7.
  Promotion into the vault — and therefore into entities, memory and the
  knowledge graph — still rendered raw segments directly.
- **Decision**: `promote_meeting_to_scribble` assembles and renders the
  canonical transcript like everything else.
- **Reason**: a graph built on a different rendering of the same meeting is a
  graph built on text no report was written from. The two would disagree about
  what was said, with nothing to say which was right.
- **Held by tests rather than by convention**: re-decoding a line changes what
  downstream reads, and renaming a speaker changes the rendering and not one
  byte of the evidence. Both are regression tests, in both directions.

---

### D-036 — A claim about a meeting carries its evidence, or says it has none

- **Context**: "Why did Vox think this was an action item?" had no answer, and
  the chain that would answer it — claim to canonical line to raw sequence to
  span of audio — already existed link by link with no type carrying it.
- **Decision**: `meetings::provenance`. `Evidence` names the meeting, the
  canonical lines, the raw sequences and the seconds. `Attributed<T>` wraps
  any claim, because a decision, an action, a question and a graph edge differ
  in what they assert and not at all in how they are evidenced.
- **It will not guess**: `locate` returns nothing when it cannot find the
  quoted text, and refuses to anchor on fewer than three words — "the release"
  occurs in half the lines of a meeting about a release. A claim with no
  evidence is reported as such, never attached to the most similar line.
- **Reason**: an approximate provenance is worse than none. It reads exactly
  like a real one, so it does not merely fail to help — it actively misleads
  the person checking, which is the one thing provenance exists to prevent.
- **An untraceable claim is kept, not dropped**: it is still something the
  model said, and hiding it would make the intelligence look better than it
  is.

---

### D-037 — A report records the transcript it was written from

- **Context**: `summary.json` carried a cache fingerprint — a hash answering
  "may this be reused". Nothing answered "what was it made from".
- **Decision**: the summary records the transcript's provenance, its line
  count, and how many segments of recorded speech were missing from it.
- **Reason**: a report generated from a transcript with a hole in it is a
  different object from one generated from a complete transcript, and the two
  should not be indistinguishable. The canonical renderer already marks gaps
  in the text a model reads; this makes the same fact legible afterwards,
  without re-deriving it.

---

### D-038 — TTS is an interface with no provider, and that is the honest shape

- **Context**: The plan for this work assumed a local TTS path existed to
  wrap. The audit found none: `talkback/` and `tts/` are not in the tree,
  while Decisions 47–56, `maybe_later.md` §§1–3 and FR-2.4 all describe them
  in detail. `docs/decisions.md` Decision 69 records the removal, because that
  log is append-only and the originals cannot be edited to match.
- **Decision**: build `tts/` as the interface — `TextToSpeech`, declared
  capabilities, a phrase splitter, a cancellable queue — with `NullTts` behind
  it.
- **Reason `NullTts` is not a placeholder**: "no voice is set up" is the state
  of every fresh install and will remain the common one. It is the correct
  answer, reported as a state rather than a fault, so every caller degrades to
  text silently and nothing pretends.
- **Reason the splitter and the queue come first**: they are what decide time
  to first audio, they are provider-agnostic, and they are testable with no
  provider, no audio device and no model. A provider added later inherits a
  tested overlap and cancellation path instead of arriving with its own.
- **What is deliberately absent**: voice cloning, conversation state,
  barge-in, interruption policy. A system that sounds exactly like you and
  takes four seconds to start is worse for every use Vox has, and the rest
  needs a turn detector and an open microphone during playback — Stage 12's
  subject, not this one's.
- **Resolves** the audit's §13.27: `LLMClient::complete_streaming` is kept,
  because `SpeechQueue` is what it was waiting for, and its doc-comment now
  points there instead of at a module nobody can open.

---

### D-039 — Stopping speech means stopping now, not after this sentence

- **Context**: The tempting design checks a stop flag between utterances.
- **Decision**: a cancel token shared with the provider and the sink.
  Everything queued is abandoned, and audio that arrives for a cancelled turn
  is discarded rather than played late.
- **Reason**: a between-items check makes the longest thing the system ever
  says its worst case, and that is exactly the case that matters — somebody
  cancels *because* the answer is long and wrong. A design that cannot stop
  mid-sentence cannot be made interruptible later; it has to be built that
  way, before anything depends on the other behaviour.
- **Consequence**: barge-in, when there is a microphone open during playback,
  needs no change here. It calls `cancel`.
- **Bounded and counted, like every other queue in the speech stack**: a model
  that runs away must not grow a backlog of audio nobody will hear, and a
  refusal that is not counted is a silent loss.

---

## Reserved, not yet decided

These ids are reserved so that the staged plan's numbering and this log's do
not diverge. Each is a **proposal** in
[speech-architecture-audit.md](speech-architecture-audit.md) §13, not a
decision, and each becomes an entry above only when the stage that implements
it lands — with the measurement that justified it.

| Id | Proposal | Blocked on |
|---|---|---|
| D-040 | Full duplex is a future layer, not a replacement for the meeting pipeline | Stage 12 — depends on D-021 (turn state) and D-038 (a cancellable voice) |
