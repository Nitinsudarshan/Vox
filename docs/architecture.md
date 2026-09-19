# Vox — System Architecture

## Architecture Overview

Vox is a native-first Windows desktop assistant built with a Rust backend and Tauri/React frontend, paired with a companion browser extension for structured web and conversation capture.

```
┌────────────────────────────────────────────────────────────────────────┐
│                        Native Desktop (native/)                        │
│                                                                        │
│  ┌──────────────────────────────────────────────────────────────────┐  │
│  │                     React UI (native/src/)                       │  │
│  │   Home | PTT Dictation Pill | Scribbles | Graph                  │  │
│  │   Files | Captures | Diagnostics | Settings                      │  │
│  └─────────────────────────────────┬────────────────────────────────┘  │
│                                    │ Tauri IPC (invoke commands)       │
│  ┌─────────────────────────────────▼────────────────────────────────┐  │
│  │                 Rust Backend (native/src-tauri/)                 │  │
│  │                                                                  │  │
│  │  ┌──────────────┐  ┌──────────────┐  ┌────────────────────────┐  │  │
│  │  │   capture    │  │   pipeline   │  │        triggers        │  │  │
│  │  │(cpal+whisper)│  │(Kanban/Scrib/│  │ (Intent/MCP Router)    │  │  │
│  │  │              │  │ Chat/RAG)    │  │                        │  │  │
│  │  └──────────────┘  └──────────────┘  └────────────────────────┘  │  │
│  │  ┌──────────────┐  ┌──────────────┐  ┌────────────────────────┐  │  │
│  │  │  providers   │  │    vault     │  │          mcp           │  │  │
│  │  │(Ollama/Cloud)│  │(Vault/search)│  │(Calendar/Notion/Drive) │  │  │
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

- `capture`: Manages audio device input via `cpal` on a dedicated thread (resampled to 16kHz mono), writes WAV files, and (`capture::stt`) transcribes via a local Whisper model (`whisper-rs`).
- `capture::web`: Web capture. Content arrives from the Vox browser extension over a loopback listener (`bridge.rs`) rather than through Tauri IPC — the browser is the only place a rendered page exists, and the only place a browser will grant access to one. Vox derives provenance from the URL (`source.rs`), sanitizes and normalizes the payload into markdown (`normalize.rs`), and persists it as a Vault artifact before any AI runs. See `docs/capture.md`.
- `pipeline`: Core prompt engines (all in `mod.rs` except `chat.rs`).
  - `process_scribble`: Generates structured Markdown documents from unstructured voice scribbles.
  - `chat.rs` / `process_chat`: Voice chat — retrieves grounding notes from the vault and asks the LLM provider.
- `pipeline::analysis`: The shared foundation every analysis runs on, so that adding one does not mean building another capture → normalize → prompt → LLM → parse → persist → provenance pipeline.
  - `source.rs`: `SourceDescriptor` — a borrowed view over a `VaultFile` answering "what is this, and how much of it does Vox have?". Classification comes from the `capture_type` that `capture::web::source` derived from the URL, not from re-matching the URL later.
  - `content.rs`: `CanonicalContent` — the analysis-facing contract every source-specific normalizer produces. Preserves turn ordinals and code artifacts, which flat markdown destroyed.
  - `contract.rs`: `AnalysisRequest` / `AnalysisResult` / `AnalysisStatus`. Keeps "the evidence was not there" apart from "the analysis failed".
  - `prompts.rs`: The prompt registry — stable ids, versions recorded on every result, and an applicability rule so a repository prompt cannot be run against a conversation.
  - `service.rs`: `AnalysisService` — the one place a prompt is resolved, the source boundary applied, the provider called, and structured output parsed and validated.
  - `derived.rs`: `DerivedData` — one record type with typed payloads, keyed by source id. See `docs/data-model.md` §7.
- `meetings`: Meeting recording, transcription and reports. Two `cpal` streams — the microphone and the default output device in loopback — drained in temporal lockstep and soft-mixed, with per-channel energy kept so a transcript line can say "You" or "Others" (`capture.rs`). A streaming energy VAD emits speech spans with pre-roll and redemption (`segmenter.rs`), one serial decoder turns them into ordered transcript lines screened through `capture::speech_health` (`transcription.rs`), a WAV checkpoint lands every 30 s so a crash costs seconds (`checkpoint.rs`), and JSON templates plus `providers::LLMClient` produce the report (`summary/`). One directory per meeting under `<vault>/meetings/` (`store.rs`). Modelled on Meetily; `docs/meetings.md` lists every deliberate divergence.
- `triggers`: Dynamic phrase matching and classification against `triggers.json`. Extracts parameters and dispatches to tool handlers. Skipped for chat mode.
- `providers`: Unified `LLMClient`.
  - Ollama: Connects to `http://localhost:11434`.
  - Cloud: Connects to OpenAI / Gemini / Anthropic OpenAI-compatible endpoints.
- `vault`: Reads and writes markdown note files with frontmatter headers. `search_notes`/`list_notes` provide keyword-ranked retrieval over vault notes — a placeholder for the embedded LanceDB vector search Decision 6 commits to (see `docs/roadmap.md`).
- `mcp`: Interface for dispatching trigger actions (`google-calendar`, `notion`, `gdrive`, OS notifications) — currently returns stubbed success results; real MCP client wiring is tracked as backlog (`docs/roadmap.md`).
- `hotkeys`: Registers the show/hide and universal-dictation global OS hotkeys (`tauri-plugin-global-shortcut`), manages the always-on-top listening indicator window, and (`hotkeys::injection`) types transcribed text into whatever field has OS focus via `enigo`.
- `tts`: Speech synthesis, as an interface with no provider behind it. `TextToSpeech` plus declared `TtsCapabilities`; `tts::phrases` splits a streaming answer into speakable phrases so time to first audio is *first phrase plus one synthesis* rather than *whole generation plus synthesis*; `tts::queue::SpeechQueue` is a bounded, single-consumer, cancellable synthesis queue whose ordering is structural. `NullTts` reports "no voice is set up", which is the ordinary state of a fresh install rather than a fault. The Talkback subsystem this replaces was removed — see `docs/decisions.md` Decision 69.
- `conversation`: Who is talking, and who may interrupt whom. A pure state machine — `ConversationMachine` takes `ConversationEvent`s (chiefly a frame's verdict from `meetings::speech_state`) and returns `ConversationAction`s, with no audio, no model and nothing wired to it. The microphone stays open while Vox speaks, which is the one property full duplex cannot acquire later; barge-in requires consecutive held speech under a `BargeInPolicy`, because without acoustic echo cancellation a laptop's microphone hears its own speaker and a first-frame detector interrupts Vox with Vox. See `docs/speech-duplex.md`.
- `settings`: Loads/saves `AppSettings` (provider, STT, hotkey config) at `.Vox/config/settings.json`.
- `commands.rs`: Exposes thin `#[tauri::command]` functions returning `Result<T, CommandError>`.

## Window Architecture vs Native OS Notifications

- **Persistent Custom Tauri Windows**: Reserved strictly for UI surfaces that genuinely require custom interactive windows (`"main"` application window and `"dictation-pill"` overlay). Meetings are recorded and controlled from the main window and the tray — there is no meeting overlay window.
- **Transient Notifications**: Native OS Toast Notifications (`tauri_plugin_notification`), presented directly without any React WebView or Tauri window of their own.

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
- **Local-Only Mode**: Default operating mode. No authentication required. Notes, scribbles, audio, and captures saved in `.Vox/vault`. Vector indices stored in `.Vox/lancedb`. Zero network transmission of user notes or audio.
- **Supabase Cloud Services**: Native Rust integration (`identity/supabase.rs`, `updates/mod.rs`) for anonymous installation registration, heartbeat telemetry, diagnostics ingestion, and app release checking. Never transmits vault content, audio, or notes.
