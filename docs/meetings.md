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
| Import | `import.rs` | Decoding an existing recording, and re-transcribing one Vox already has. |

## On disk

```text
<vault>/meetings/<meeting_id>/
├── meeting.json      metadata — the record the list reads
├── transcript.json   segments, ordered by sequence
├── summary.json      the report, its cache fingerprint, and the English original
├── notes.md          whatever the user typed
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

Transcript lines are labelled **You** or **Others**, and that is the capture
channel rather than diarization. The microphone and the system loopback are
two separate streams, so which one carried a span is known for free; the
mixer keeps per-channel energy alongside the mixed window and a span is
attributed to a channel only when that channel is three times louder than the
other. Anything closer reads as **Speaker**.

Two remote participants are both **Others** and cannot be told apart. Vox says
so in the prompt rather than letting a model invent names for them.

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

## Failure and recovery

| What happens | What the user gets |
|---|---|
| Vox is killed mid-recording | Audio up to the last 30 s checkpoint, the transcript flushed so far, and the meeting marked complete on next launch with a note saying it was interrupted. |
| System audio will not open | The recording proceeds with the microphone alone, and says so — in the recorder, on the list row, and in the summarization prompt. |
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

- **Diarization.** Two people on the far end of a call are not separated. See
  Decision 68.
- **Meeting reminders and calendar matching.** Removed with `meetings_v2` and
  not restored; `maybe_later.md` holds the deferral.
- **A meeting overlay window.** Recording is controlled from the Meetings
  surface and the tray.
