# Vox — System Architecture

## Architecture Overview

Vox is a native-first Windows desktop assistant built with a Rust backend and Tauri/React frontend, paired with a companion browser extension for structured web and conversation capture.

```
┌────────────────────────────────────────────────────────────────────────┐
│                        Native Desktop (native/)                        │
│                                                                        │
│  ┌──────────────────────────────────────────────────────────────────┐  │
│  │                     React UI (native/src/)                       │  │
│  │   Home | Voice Notes | Meetings | Scribbles | TODOs | Graph      │  │
│  │   Files | Captures | Tests | Diagnostics | Settings              │  │
│  │   + own windows: dictation pill, meeting pill, reminder card     │  │
│  └─────────────────────────────────┬────────────────────────────────┘  │
│                                    │ Tauri IPC (invoke commands)       │
│  ┌─────────────────────────────────▼────────────────────────────────┐  │
│  │                 Rust Backend (native/src-tauri/)                 │  │
│  │                                                                  │  │
│  │  ┌──────────────┐  ┌──────────────┐  ┌────────────────────────┐  │  │
│  │  │   capture    │  │   pipeline   │  │        meetings        │  │  │
│  │  │(cpal/whisper/│  │(scribble,    │  │ (dual-stream capture,  │  │  │
│  │  │ dictation)   │  │ analysis)    │  │  transcript, reports)  │  │  │
│  │  └──────────────┘  └──────────────┘  └────────────────────────┘  │  │
│  │  ┌──────────────┐  ┌──────────────┐  ┌────────────────────────┐  │  │
│  │  │  providers   │  │    vault     │  │        calendar        │  │  │
│  │  │(Ollama/Cloud)│  │(Vault/search)│  │ (Google, reminders)    │  │  │
│  │  └──────────────┘  └──────────────┘  └────────────────────────┘  │  │
│  │  ┌──────────────┐                    ┌────────────────────────┐  │  │
│  │  │   hotkeys    │                    │       settings         │  │  │
│  │  │(global+inject│                    │  (settings.json I/O)   │  │  │
│  │  └──────────────┘                    └────────────────────────┘  │  │
│  │  ┌──────────────────────────────────────────────────────────────┐│  │
│  │  │ capture::web  ◀── loopback (127.0.0.1) ── browser extension  ││  │
│  │  │ (detect source → sanitize → normalize → Vault artifact)      ││  │
│  │  └──────────────────────────────────────────────────────────────┘│  │
│  └─────────────────────────────────┬────────────────────────────────┘  │
└────────────────────────────────────┼───────────────────────────────────┘
                                     │ Heartbeat, diagnostics & release checks
                                     ▼
                      ┌──────────────────────────────┐
                      │   Supabase Cloud Services    │
                      └──────────────────────────────┘
```

## Rust Backend Module Design (`native/src-tauri/src/`)

- `capture`: Manages audio device input via `cpal` on a dedicated thread (resampled to 16kHz mono), writes WAV files, and (`capture::stt`) transcribes via a local Whisper model (`whisper-rs`) — or, for dictation, Parakeet TDT (`capture::parakeet`) when it is selected and installed.
  - `dictation.rs`: The one dictation pipeline, called by the hotkey and the pill alike: `transcribe`, then `finish_text` (deterministic normalisation with learned corrections, snippets, the output script; an empty result is never typed or saved), then `clean_up` — the opt-in rewrite in `rewrite.rs`, which is `Raw` (no model call) by default and capped at five seconds when a style is chosen. See `docs/decisions.md` Decisions 71 and 72.
- `capture::web`: Web capture. Content arrives from the Vox browser extension over a loopback listener (`bridge.rs`) rather than through Tauri IPC — the browser is the only place a rendered page exists, and the only place a browser will grant access to one. Vox derives provenance from the URL (`source.rs`), sanitizes and normalizes the payload into markdown (`normalize.rs`), and persists it as a Vault artifact before any AI runs. See `docs/capture.md`.
- `pipeline`: Prompt engines over captured content.
  - `mod.rs` / `PipelineEngine::process_scribble`: Turns a transcript into a structured Markdown note. Reached through `start_capture` in `"scribble"` mode, which no surface currently starts.
  - `enrichment.rs`: Gives a Scribble its title, summary, topics and entities.
  - `source_boundary.rs`: Frames external content as data, never instructions, before any model reads it (`docs/capture.md` §9).
- `pipeline::analysis`: The shared foundation every analysis runs on, so that adding one does not mean building another capture → normalize → prompt → LLM → parse → persist → provenance pipeline.
  - `source.rs`: `SourceDescriptor` — a borrowed view over a `VaultFile` answering "what is this, and how much of it does Vox have?". Classification comes from the `capture_type` that `capture::web::source` derived from the URL, not from re-matching the URL later.
  - `content.rs`: `CanonicalContent` — the analysis-facing contract every source-specific normalizer produces. Preserves turn ordinals and code artifacts, which flat markdown destroyed.
  - `contract.rs`: `AnalysisRequest` / `AnalysisResult` / `AnalysisStatus`. Keeps "the evidence was not there" apart from "the analysis failed".
  - `prompts.rs`: The prompt registry — stable ids, versions recorded on every result, and an applicability rule so a repository prompt cannot be run against a conversation.
  - `service.rs`: `AnalysisService` — the one place a prompt is resolved, the source boundary applied, the provider called, and structured output parsed and validated.
  - `derived.rs`: `DerivedData` — one record type with typed payloads, keyed by source id. See `docs/data-model.md` §7.
- `meetings`: Meeting recording, transcription and reports. Two `cpal` streams — the microphone and the default output device in loopback — drained in temporal lockstep and soft-mixed, with per-channel energy kept so a transcript line can say "You" or "Others" (`capture.rs`). A streaming energy VAD emits speech spans with pre-roll and redemption (`segmenter.rs`), one serial decoder turns them into ordered transcript lines screened through `capture::speech_health` (`transcription.rs`), a WAV checkpoint lands every 30 s so a crash costs seconds (`checkpoint.rs`), and JSON templates plus `providers::LLMClient` produce the report (`summary/`). One directory per meeting under `<vault>/meetings/` (`store.rs`). Modelled on Meetily; `docs/meetings.md` lists every deliberate divergence.
- `calendar`: Google Calendar sync across any number of accounts (read-only), recordings matched to the events they overlap, and meeting reminders (`calendar::reminders`) raised in the `meeting-reminder` window.
- `providers`: Unified `LLMClient`.
  - Ollama: Connects to `http://localhost:11434`; Vox can install, start and pull models for it (`ollama_install.rs`, `ollama_manager.rs`).
  - Cloud: OpenAI, Gemini, Anthropic, Groq, OpenRouter, and any OpenAI-compatible server at an address the user gives (`custom_openai`).
  - `secrets.rs`: API keys live in the OS credential store rather than `settings.json`, and the webview only ever sees the placeholder `__vox_stored_key__`. With no credential store available they stay in the file (Decision 73).
- `vault`: Reads and writes markdown note files with frontmatter headers. `search_notes`/`list_notes` provide keyword-ranked retrieval over vault notes — a placeholder for the embedded LanceDB vector search Decision 6 commits to (see `docs/roadmap.md`).
- `mcp`: `McpRouter::assemble_context` and `McpRouter::execute_action` shape a context pack and a confirmation-gated universal action for an MCP caller. Nothing calls them yet, and there is no outbound MCP client: the stubs that reported success for calendar, Notion and Drive actions without doing anything were removed with the trigger engine (Decision 75).
- `hotkeys`: Registers the show/hide, dictation and capture global hotkeys (`tauri-plugin-global-shortcut`) and runs hold-to-talk and toggle-to-talk. `hotkeys::injection` delivers dictated text to the field that had OS focus when recording started: by default it puts the text on the clipboard and sends Ctrl+V through Win32 `SendInput`; the `keystrokes` method types it through `enigo` instead. If focus has moved, it waits up to 15 s for the user to return to the same window and tab, and otherwise leaves the text on the clipboard with a toast saying so. Injection and clipboard writes run on the blocking pool. The pill window itself is `overlay.rs`'s.
- `tts`: Speech synthesis, as an interface with no provider behind it. `TextToSpeech` plus declared `TtsCapabilities`; `tts::phrases` splits a streaming answer into speakable phrases so time to first audio is *first phrase plus one synthesis* rather than *whole generation plus synthesis*; `tts::queue::SpeechQueue` is a bounded, single-consumer, cancellable synthesis queue whose ordering is structural. `NullTts` reports "no voice is set up", which is the ordinary state of a fresh install rather than a fault. The Talkback subsystem this replaces was removed — see `docs/decisions.md` Decision 69.
- `conversation`: Who is talking, and who may interrupt whom. A pure state machine — `ConversationMachine` takes `ConversationEvent`s (chiefly a frame's verdict from `meetings::speech_state`) and returns `ConversationAction`s, with no audio, no model and nothing wired to it. The microphone stays open while Vox speaks, which is the one property full duplex cannot acquire later; barge-in requires consecutive held speech under a `BargeInPolicy`, because without acoustic echo cancellation a laptop's microphone hears its own speaker and a first-frame detector interrupts Vox with Vox. See `docs/speech-duplex.md`.
- `settings`: Loads/saves `AppSettings` at `<base>/config/settings.json` (`<base>` is described under Data Access below). Provider API keys are the exception — see `providers::secrets` above.
- `env_config`: Reads configuration from the environment as `VOX_*`, falling back to the pre-rename `RELAY_*` (the Google client id and secret, the Supabase URL and anon key). `VOX_HOME` is read directly, with no fallback.
- `commands.rs`: Exposes thin `#[tauri::command]` functions returning `Result<T, CommandError>`.

## Window Architecture vs Native OS Notifications

- **App-owned Tauri windows** (`overlay.rs` builds all but `main`): `"main"`, the application window; `"dictation-pill"`, the always-present push-to-talk pill (Decision 36); `"meeting-overlay"`, the pill with the elapsed time and a live waveform that `start_meeting` shows and stopping hides; and `"meeting-reminder"`, the reminder card carrying Record, Join and Snooze — created hidden at startup, never focus-stealing, and content-protected so it stays out of screen shares (Decision 63).
- **Native OS toasts** (`tauri_plugin_notification`) carry only what needs no button — today, dictation's notice that it left the text on the clipboard rather than type into a window the user had moved away from. A desktop toast cannot carry action buttons, which is why a meeting reminder is a window of its own (Decision 63).

## Rendered Markdown And Diagrams (`native/src/components/common/MarkdownView.tsx`)

Vox renders markdown it did not write — a model's meeting summary, a captured
web page — so this surface is treated as untrusted input, not as presentation.
See `docs/decisions.md` Decision 70.

- **Mermaid runs at `securityLevel: 'strict'`**, configured through an exported
  `mermaidConfig()` that a test asserts against. `'loose'`, which this used to
  be, passes HTML in a diagram label straight into the injected SVG.
- **Every SVG passes `sanitizeSvgMarkup` at the point of injection**
  (`native/src/lib/svgSafety.ts`), which removes script elements, `on*`
  handlers and executable URL schemes while leaving the `foreignObject` that
  flowchart labels need. Hand-written rather than a generic profile, because
  the standard SVG allowlists drop `foreignObject` and take every label with
  it.
- **The webview has a content security policy** (`tauri.conf.json`), whose
  load-bearing clause is `script-src 'self'`.

Diagrams render inside `DiagramViewport`
(`native/src/components/shared/DiagramViewport.tsx`) rather than being scaled
to fit. A summary's flowchart is often several times wider than the panel it
lands in, and fitting it produces a picture of a diagram with every label below
body-text size. Instead the viewport opens at a legible zoom, overflows on
purpose past that point, and is pannable — by drag, by wheel, by arrow key
(shift for a screenful), with `+`/`-` to zoom, `0` to fit and `F` for a
full-screen dialog. The arithmetic lives in `native/src/lib/diagramViewport.ts`
and is tested there: jsdom has no layout, so a test rendering the component
would measure zeroes.

## Data Access & Security Model
- **Local-Only Mode**: Default operating mode. No authentication required. Notes, scribbles, meetings and captures are saved in `<base>/vault` (or the folder the user chose as Vault Directory Location), and settings, speech models and dictation recordings (`config/audio/`) in `<base>/config`. `<base>` is chosen once at startup by `lib.rs::choose_base_dir` (Decision 74): `VOX_HOME` if set; otherwise an existing `.vox` (or legacy `.relay`) folder beside the executable or in the working directory or one of its ancestors, never moved; otherwise, for a fresh release install, the per-user application-data folder (`%APPDATA%\com.vox.app` on Windows); a debug build uses the checkout's `native/src-tauri/.vox`. There is no vector index — LanceDB is decided (Decision 6) but not built. Zero network transmission of user notes or audio unless the user selects a cloud LLM provider, which then receives the text it is asked to process.
- **Supabase Cloud Services**: Native Rust integration (`identity/supabase.rs`, `updates/mod.rs`) for anonymous installation registration, heartbeat telemetry, diagnostics ingestion, and app release checking. Never transmits vault content, audio, or notes.
