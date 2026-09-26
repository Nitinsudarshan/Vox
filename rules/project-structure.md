---
trigger: always_on
description: Folder structure — where new files should be created
---

# Project Folder Structure

Vox is a native-first Rust + Tauri desktop application for Windows with a
companion browser extension for web capture. The layout below is what exists
today; put new files where it says rather than starting a parallel tree.

```
Vox/
  native/                    Tauri desktop app (Windows, local mode)
    browser-extension/        Companion browser extension for web capture
    src-tauri/                Rust backend
      src/
        capture/               Push-to-talk, local Whisper/Parakeet STT, web capture wiring
        meetings/              Meeting recording, segmentation, transcription, reports
        pipeline/              Scribble enrichment, analysis, external-source framing
        calendar/              Google Calendar sync and meeting reminders
        providers/             Local / cloud LLM providers
        vault/                 Local markdown vault (and LanceDB, once built)
        identity/              Native telemetry and anonymous identity
        updates/               Native update checks
        commands/              #[tauri::command] entry points by surface (thin, see rust-backend.md)
    src/                       React frontend rendered inside the Tauri window
      components/
        ui/                    shadcn/ui primitives (generated, low-touch)
        shared/                Reusable components
      hooks/
      lib/
      types/
  docs/                      Living spec — see docs/README.md for the index
```

## Rules

- Reusable UI primitives (buttons, inputs, dialogs) go in `native/src/components/ui`
  — prefer generating via the shadcn CLI over hand-writing them.
- Shared, non-primitive components go in `native/src/components/shared` or
  feature-specific component directories.
- Rust modules under `src-tauri/src/` are organized by domain
  (`capture/`, `meetings/`, `calendar/`, `pipeline/`, `providers/`, `vault/`, `mcp/`), not
  by technical layer — a feature's parsing, validation, and persistence logic
  live together in its own module, not scattered across generic `services/`
  or `utils/` folders.
- `commands/` (one module per surface) and the `commands.rs` of a domain that
  owns its surface (`meetings/`, `calendar/`), all registered via Tauri, are
  the **only** places `#[tauri::command]` functions live — they call into
  domain modules, they don't contain business logic themselves (see
  `rust-backend.md`).
- `native/src/` never talks to Supabase directly — only the Rust backend does
  (see `security.md`).
- TypeScript types go in `native/src/types`.
- `docs/` is the living spec — keep it current, it's not optional
  documentation. `docs/README.md` says what each file is for and what does
  not warrant a new one.

