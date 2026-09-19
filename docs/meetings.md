# Meetings

Recording a conversation on this machine, transcribing it here, and turning
the transcript into a report.

The subsystem lives in `native/src-tauri/src/meetings/` and renders through
`native/src/components/meetings/`. It is modelled on
[Meetily](https://github.com/Zackriya-Solutions/meetily), the closest
published equivalent — same Tauri/Rust/local-Whisper shape — and the places it
diverges are listed at the end, with reasons.

## The chain

```text
microphone ─┐
            ├─ capture ─ mix ─ segmenter ─ transcription ─ transcript.json
system ─────┘   │                (speech spans)   │
(loopback)      └─ checkpoint ─ audio/chunk_*.wav └─ meeting-transcript-segment events

transcript.json ─ summary::service ─ template + LLM ─ summary.json
```

| Stage | Module | What it decides |
|---|---|---|
| Capture | `capture.rs` | Opens the microphone and the default output device in loopback. Drains both in temporal lockstep and soft-mixes to one 16 kHz mono stream, keeping each channel's energy alongside it. |
| Segmentation | `segmenter.rs` | Streaming energy VAD. Emits a span when speech starts, with 300 ms of pre-roll, tolerating 400 ms gaps, splitting anything past 25 s at its quietest frame. |
| Transcription | `transcription.rs` | One serial decoder over a bounded queue. Screens every decode through `capture::speech_health` before it reaches the transcript. |
| Durability | `checkpoint.rs` | A WAV checkpoint every 30 s, merged into `audio.wav` on stop. |
| Storage | `store.rs` | One directory per meeting, atomic writes, crash recovery. |
| Reports | `summary/` | Six JSON templates, chunking against the configured context window, English-first generation with a translation pass. |
| Lifecycle | `engine.rs` | Start, pause, resume, stop, recover. |
| Import | `import.rs` | Decoding an existing recording, re-transcribing one Vox already has, and decoding a second English pass over either. |
| Script & language | `variants.rs` | Devanagari to Latin without a model; batched, validated LLM translation for everything a model is actually needed for. |
| Speakers | `voiceprint.rs`, `speakers.rs` | MFCC statistics per turn, clustered into proposed speakers with a voice sample for each. |

## On disk

```text
<vault>/meetings/<meeting_id>/
├── meeting.json      metadata — the record the list reads
├── transcript.json   segments, ordered by sequence
├── speakers.json     the voices detection proposed, and the names you gave them
├── summary.json      the report, its cache fingerprint, and the English original
├── notes.md          whatever the user typed
├── diagnostics.jsonl one line per decoded segment, appended as it goes
├── diagnostics.json  the run's rollup, written once on stop
└── audio/
    ├── chunk_000000.wav   durable 30 s checkpoints, during recording
    └── audio.wav          the merged recording, written on stop
```

Files rather than a database, matching the rest of the vault: a meeting stays
greppable and syncable without Vox running. Every write that matters goes to a
temporary file and is renamed into place.

Deleting a meeting removes the directory — transcript, report and audio
together.

## Speakers

Two layers, and the difference between them matters.

**The capture channel is measured.** Transcript lines are labelled **You** or
**Others** from which stream carried them: the microphone and the system
loopback are two separate streams, so this is known for free. The mixer keeps
per-channel energy alongside the mixed window and a span is attributed to a
channel only when that channel is three times louder than the other. Anything
closer reads as **Speaker**. This is never wrong, and it can never tell two
remote participants apart.

**Who those participants are is proposed.** `voiceprint.rs` fingerprints each
turn — 26 MFCC statistics over 32 ms frames, cosine-normalised so the same
voice at two volumes stays one voice — and `speakers.rs` clusters the
fingerprints with average-linkage agglomeration, then hands each group a span
of the recording where that voice is talking alone.

This is not neural diarization. A production system embeds turns with a
trained speaker encoder (x-vector, ECAPA); that needs an ONNX runtime and a
model file, and `maybe_later.md` §11 is where it is described. MFCC statistics
group the same speaker together far more often than chance and are beaten by a
shared microphone, by two similar voices, and by a turn short enough that one
vowel dominates it.

So the product shape follows what the technique can honestly claim: Vox
proposes groups, plays four seconds of each, and the user names them. A wrong
proposal costs one rename. Two rules keep that bearable:

- The microphone channel is never clustered — one microphone carries one
  person, so clustering it could only ever invent a second speaker for the
  same voice — and the two channels are never merged, so a quiet remote voice
  can never be attributed to the person running Vox.
- A turn shorter than 1.2 seconds gets no fingerprint and stays unattributed.
  An unattributed line reads as unknown; a wrongly attributed one reads as
  known and wrong.

Names the user types survive a re-run; Vox's own `Speaker 3` placeholders do
not. Reports are rendered with whatever names exist, which is what lets a
report say "Payal committed to sending the deck" rather than "Others did".

## Script and language

A meeting is rarely in one language. The same conversation can be almost all
English with a few Hindi sentences, almost all Hindi with a few English words,
or anything between — and a reader who cannot read Devanagari needs the same
meeting back in an alphabet they can read.

Three views, produced three different ways, deliberately:

| View | Produced by | Needs a model? |
|---|---|---|
| Original | The decode itself | — |
| Romanized | `capture::romanize`, a Devanagari state machine | No |
| English | A second Whisper pass in translate mode, then an LLM for what it missed | Yes, for the fallback only |

**Romanization is a projection**: the same words in a different alphabet,
computed offline in microseconds, faithful by construction. Nothing asks a
model to do it. Before `variants.rs`, one model call was asked for a
translation and a romanization in the same object, and a local model answered
both with the same string — so the Romanized view showed English. The
acceptance check in `variants::accept_translation` now refuses a "translation"
that is merely the romanization handed back.

**English comes from the audio first.** `import::generate_english_track`
decodes the recording a second time with Whisper's translate task. That is
better than translating the finished transcript for a specific reason: Whisper
saw far more speech-to-English data than Hindi transcription data, and a model
handed a garbled Hindi line can only produce a garbled English one. The LLM
pass fills whatever lines the translate pass left empty, in validated batches,
and reports what it could not do rather than saving the transcript unchanged.

The report pipeline ensures English exists before it summarises, because every
report is written in English and translated afterwards whatever the configured
report language.

## Decoding, and what a re-transcription changes

A live meeting decodes against a clock: audio keeps arriving, and a decoder
slower than real time eventually drops speech. So the live worker keeps a
cheaper profile for scripts Whisper writes expensively — greedy, no
temperature fallback — and switches to it when it sees one.

A batch run has no clock. `BatchConfig` therefore carries **one** decode
profile and not two, so re-transcribing Hindi audio structurally cannot be
decoded more cheaply than the same audio in English, and
`WhisperDecodingConfig::for_meeting_batch` turns off the encoder clamp and
defaults to the Quality preset.

Re-transcription is a dialog rather than a button, because of what it is for:
nobody presses it when the transcript is fine. Running it again on the
settings that produced a wrong transcript produces the wrong transcript again.
The three choices it offers are the three that change the answer — the
language to pin, the model to read with, and how hard to look.

Pinning the language matters more than it looks. Whisper re-detects the
language every thirty seconds, so a call that switches between two languages
can be detected as the wrong one partway through — and a chunk decoded under
the wrong language does not fail. It comes back as fluent nonsense in the
wrong language's phonology, which is worse than an error because nothing about
it looks broken.

## Reports

A template is JSON: a list of sections, each with a heading, an instruction in
the author's own words, and a layout (bullets, checklist, prose). Six ship
compiled into the binary — General, Standup, One-on-One, Client Call,
Interview, Technical Review. A file in `<config>/meeting-templates/<id>.json`
adds a new one or replaces a built-in with the same id, with no rebuild.

Generation:

1. The transcript is rendered with speaker labels and timestamps.
2. If it fits the model's context window, one pass produces the report.
   Otherwise it is chunked with overlap, each chunk becomes dense notes, and
   one further pass assembles the report from those notes.
3. The report is always generated in English and translated afterwards if the
   user asked for another language. Models follow a template's structure far
   more reliably in English, and the English original is cached so a second
   language costs one pass rather than two.
4. The report's `# ` title replaces the meeting's name — but only while the
   meeting still carries the generated `Meeting 2026-09-11 14:31` placeholder.
   A title the user typed is theirs.

A regeneration copies the existing report aside first and restores it if the
run fails or is cancelled.

The transcript reaches the model through `pipeline::source_boundary`: anyone
on a call can say "ignore your instructions", and a transcript is full of
imperative sentences, so it is framed as data inside an unguessable delimiter
rather than filtered.

## Diagnostics

Every decode was already measured — queue wait, lock wait, model load, decode,
post-processing, persistence, and the voiced-time profile the hallucination
screen judged it against. Those numbers were printed to a terminal and
dropped, which is enough to watch one run and not enough to answer any
question about behaviour over time. That is most questions: *is this model too
slow on this machine*, *did the backlog grow*, *how often does the screen
reject real speech*.

Two files, two shapes. `diagnostics.jsonl` is a log, appended one line per
segment, so the thousandth costs what the first did and a crash keeps
everything written before it — a torn final line is skipped on read, which
would be unacceptable for a transcript and is exactly right for telemetry
about a run that has already crashed. `diagnostics.json` is a document,
written once on stop, atomically like everything else in the vault.

What is deliberately **not** in them: transcript text (the record carries
`text_chars`; the transcript already holds the words, and a second copy is a
second thing to leak and to delete), audio, speaker attribution
(`transcript.json` carries `speaker_id` against the same `sequence`, and two
copies of one fact is how they disagree), and confidence — Whisper does not
report one, so the field is called `no_speech_prob`, and an engine that
reports nothing records nothing.

The rollup answers two questions separately, because they fail for different
reasons and are fixed by different actions:

| | Answers |
|---|---|
| `capture` | Did the devices open, did each one *hear* anything, was durable audio written, did any checkpoint fail. `opened && !heard` is the wrong device. |
| `transcription` | What the segmenter emitted and what became text, the two coverage figures, decode RTF against speech and pipeline RTF against the clock, finalization p50/p95/max, peak queue, model reloads, the wait after stop, and discarded segments broken down by the reason `speech_health` gave. |

`pipeline_rtf` is the number that decides whether a meeting can be transcribed
live at all: audio arrives at wall-clock rate, so above 1.0 the backlog grows
by `L × (rtf − 1)` over a meeting of length `L` and takes that long to clear
after stop. `decode_rtf` measures the model against speech and can look
excellent while the backlog grows, because a meeting is mostly silence.

Meeting Detail renders this once the recording has finished — not during one,
because a rollup of an unfinished run reads as a verdict on it — and offers
the slowest segments on request rather than loading a thousand records nobody
asked for.

Re-transcription and import decode through `import::run_batch` and do **not**
write diagnostics yet. They have no clock to fall behind, so backlog and drops
cannot happen there; what is missing is the accuracy-side evidence, and that
gap is real.

## Failure and recovery

Audio failures and transcript failures are reported on **separate channels**,
because they are not the same kind of problem. `meeting-transcription-warning`
means a line is missing from the transcript; `meeting-recording-warning` means
something is wrong with the recording itself — and that is the more serious of
the two, because a transcript can be regenerated from a recording and a
recording cannot be regenerated from anything. Both reach the user while the
meeting is still running, which is the only point at which they can act: move
to another device, free some disk, start again.

| What happens | What the user gets |
|---|---|
| Vox is killed mid-recording | Audio up to the last 30 s checkpoint, the transcript flushed so far, and the meeting marked complete on next launch with a note saying it was interrupted. |
| System audio will not open | The recording proceeds with the microphone alone, and says so — in the recorder, on the list row, and in the summarization prompt. |
| The microphone will not open | The recording proceeds with system audio alone, and says so. The mirror of the row above, and the more serious half: without it the person running Vox is the one missing from their own meeting. |
| Audio cannot be written at all | A recording warning during the meeting, and the diagnostics say `audio_checkpoints_written: false` afterwards. The transcript is still produced — it is most of the value — but the user is told while there is still a meeting to move. |
| A checkpoint write fails part-way | Counted, warned once (a full disk fails every write, and one warning per 30 s of audio would bury the first), and reported as gaps in the recording. |
| Audio arrives faster than it can be written | The channel to the pump holds ten seconds; past that a 20 ms block is shed, counted in `audio_lost_seconds`, and warned once. Unbounded was the previous behaviour and it is worse: memory grows until the process dies, which costs every second since the last checkpoint instead of twenty milliseconds. |
| No speech model installed | Recording is refused before anything is captured, rather than producing audio that can never become a transcript. |
| Transcription falls behind real time | The queue holds 64 segments; past that a segment is refused, counted, reported in the UI and recorded on the meeting. |
| A segment will not decode | Counted, reported once, and the meeting's `dropped_segments` says how many. |
| The model is unreachable | Each pass is retried three times with widening delays. A failure restores the previous report. |
| A translation fails | The English report is kept — a complete, useful answer. |

## Where this differs from Meetily, and why

Each of these is a defect named in Meetily's own architectural teardown, not a
matter of taste.

| Meetily | Vox | Why |
|---|---|---|
| The renderer writes the meeting to SQLite seconds after the recording stops (`recording_commands.rs:866`) | Rust writes the transcript as it arrives; the frontend only reads | A webview reload in that window loses the meeting |
| Confidence is `(text.len() / 100.0).min(0.9) + 0.1`, filtered on and shown to users | Whisper's own `no_speech_prob` | A length-derived number is not a confidence score |
| Microphone and system audio summed before the VAD, so its `speaker` column is never written | Per-channel energy kept alongside the mix | "Was that me or them" is the question users actually ask |
| Every audio channel is `unbounded_channel` | Bounded queue that reports what it dropped | A decoder slower than real time otherwise grows memory for the whole meeting |
| Deleting a meeting leaves `audio.mp4` on disk | The directory is the meeting | Deleting a sensitive meeting should delete the recording |
| No retry, and a 300 s timeout reported as 60 s | Three attempts with widening delays | A loading model or a 429 is ordinary |
| API keys in plaintext SQLite columns, returned to the webview | Vox's existing keyring path | — |
| Transcript text logged at `info`, with `RUST_LOG` forced to `info` | Transcript text never logged above `debug` | Meeting content should not reach log files by default |
| `template_id` joined into a path with no traversal check | Ids validated before they touch the filesystem | — |
| No hallucination screening on the live path | Every segment through `capture::speech_health` | Whisper invents fluent text over silence |
| FFmpeg downloaded at build time, `panic!` without network; AAC checkpoints | `hound` WAV checkpoints, `symphonia` for import | A third more disk, no external binary, and a merge that is a byte copy |
| Silero VAD via ONNX Runtime | Vox's own energy VAD, made streaming | One fewer runtime dependency; the shape that matters is streaming segmentation, not the model |

What Vox does not take from Meetily and does not have: cloud STT (there is
none — transcription is always local), and a choice of seven summary
providers (Vox's `providers/` layer already offers Ollama, OpenAI, Anthropic
and Gemini, and that is the one place provider choice belongs).

## What is not here

- **Neural diarization.** Speakers are grouped by MFCC statistics, not by a
  trained speaker encoder, and the grouping is presented as a proposal for
  exactly that reason. See `maybe_later.md` §11 and Decision 68.
- **Voices remembered across meetings.** A name given in one meeting does not
  carry to the next. That needs an embedding stable enough to store, which is
  the same gap as above.
- **A reminder for a meeting that is still being recorded after it ended.**
  The mirror of the unrecorded reminder, and the other way a recording goes
  wrong unattended.
- **A summary across a whole series.** Recurring meetings are linked
  (`meetings::series`) and their occurrences read in order, but "what has
  changed in this standup over six weeks" is still six reports read by hand.
  Designed in `maybe_later.md` §14.
- **A meeting overlay window.** Recording is controlled from the Meetings
  surface and the tray.
