# Vox — Roadmap & Competitive Gap Backlog

What is real today, what is not, and what should come next. It exists so "not
implemented yet" is written down instead of implied by absence. Where this and
the code disagree, the code is right and this file is the bug.

## Real today

- **Dictation.** Push-to-talk (hold, or toggle-to-talk) records the microphone
  (`cpal`, resampled to 16 kHz mono) and transcribes locally — whisper.cpp
  through `whisper-rs`, or Parakeet TDT through ONNX Runtime when selected and
  installed. The text goes into the field that had focus: clipboard plus a
  Win32 `SendInput` Ctrl+V by default, `enigo` keystrokes as the alternative,
  with a focus guard that leaves the text on the clipboard rather than type
  into a window the user has left. An AI cleanup pass is opt-in; the default
  sends nothing to a model (Decision 71). The hotkey and the pill share one
  pipeline (`capture::dictation`, Decision 72).
- **Voice notes, todos, scribbles and the knowledge graph** in a local
  Markdown vault, with keyword-ranked retrieval (`VaultManager::search_notes`,
  `retrieval::`).
- **Meetings.** Dual-stream capture (microphone plus system loopback),
  streaming segmentation, a serial decoder with hallucination screening
  (`capture::speech_health`), 30-second checkpoints and crash recovery,
  templated reports, audio import and re-transcription. Lines are labelled
  You / Others by capture channel, and turns are grouped into proposed
  speakers by MFCC statistics for the user to name (`meetings::voiceprint`,
  `meetings::speakers`). See `docs/meetings.md`.
- **Calendar.** Read-only Google Calendar sync across accounts, recordings
  matched to their events, series identity, and meeting reminders in their own
  window (`calendar::reminders`, Decision 63).
- **Web capture** through the browser extension and a loopback bridge, and
  AI-conversation import from export packages. See `docs/capture.md`.
- **Model providers.** Ollama (which Vox can install, start and pull models
  for), OpenAI, Anthropic, Gemini, Groq, OpenRouter, and any OpenAI-compatible
  server. API keys live in the OS credential store (Decision 73).
- **CI** runs clippy and the Rust tests on Linux and Windows, the frontend
  tests and build, the extension build, a frontend-to-backend command contract
  check, and a weekly dependency-advisory audit.

## Not real yet — prioritized backlog

1. **Embedded vector retrieval (LanceDB).** Decision 6 commits to it; today's
   retrieval is keyword and term overlap, not embeddings. The highest-value
   follow-up, since semantic search and any grounded Q&A depend on retrieval
   quality. LightRAG is the cheaper alternative to full GraphRAG if note volume
   grows past about a thousand documents.

2. **Proving ONNX Runtime survives Windows packaging.** Parakeet is on by
   default, and `docs/spikes/onnx-windows.md` — written, never run — is what
   would show it works in an installed build rather than only in the one that
   built it. The same run unblocks the ONNX-based items in `maybe_later.md`
   item 1. Building installers in CI (`maybe_later.md` item 21) is the natural
   place to run it.

3. **External actions.** There is no outbound MCP client and Vox writes to
   nothing outside itself; Decision 8 names the servers to reuse. The trigger
   engine that would have fed it, and stubs that reported success without
   doing anything, were removed (Decision 75). Voice-triggered actions are
   designed as a new feature in `maybe_later.md` item 19.

4. **Telling similar voices apart, and remembering them.** Acoustic grouping
   can merge two similar voices on one channel, and nothing persists a voice
   across meetings. A neural speaker embedding and an opt-in voice library are
   `maybe_later.md` item 11; calendar attendees as a speaker-count hint are
   item 12.

5. **Streaming dictation.** Dictation decodes the whole utterance after the
   key is released, so the wait grows with how long the user spoke.
   `capture::streaming_pipeline` exists as a measurement instrument only
   (`maybe_later.md` item 18).

6. **Multi-user / team features.** Noted for later, not decided. Decision 12's
   Supabase auth is a foundation to extend; no sharing model, permissions or
   shared-vault schema exists.

## Explicitly not gaps

- **Continuous background capture** is excluded on purpose (Decision 5 —
  push-to-talk and on-screen-triggered capture only, no meeting bot, no
  always-on recording). Listed so it is not mistaken for an oversight.
