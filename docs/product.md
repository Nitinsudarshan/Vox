# Vox — Product Specification

## Overview
Vox is a hybrid (local + cloud) AI voice and memory assistant that converts captured speech into structured, actionable system state — text dictated straight into any app, voice notes, todos, scribbles in a knowledge graph, and meeting transcripts with reports — eliminating manual data re-entry.

## Target User
The primary user is a builder or power user with a meeting-heavy and task-heavy workflow who needs instant voice capture, automated task extraction, audio scribble structuring, and configurable voice shortcuts without relying on cloud subscriptions or intrusive meeting bots.

## Core Value Proposition & Competitive Differentiators
1. **Bot-Free Voice Capture**: Operates strictly via push-to-talk (PTT) and local system audio, avoiding the legal, platform, and privacy issues of third-party meeting bots.
2. **Transcript-to-Kanban**: Automatically parses meeting transcripts into actionable, structured task cards formatted for a Kanban workflow, rather than generating wall-of-text summaries. *(Not built. Deferred by `docs/decisions.md` Decision 33; meeting action-item extraction is `maybe_later.md` §16. Todos come from typing, from speaking on the TODOs surface, and from web captures.)*
3. **Voice Scribble to Structured Output**: Transforms rambling audio notes into polished markdown templates (executive summaries, key decisions, action points). *(Built but unreachable: `PipelineEngine::process_scribble` does this, and no surface starts the `"scribble"` capture mode that calls it. A voice note promoted to a Scribble is enriched — title, summary, topics, entities — instead.)*
4. **User-Customizable Trigger Phrases**: Allows users to configure arbitrary phrase-to-action mappings (e.g., "Schedule quick sync" -> Calendar MCP call; "Remind me in 2 hours" -> Local OS notification). *(Not built. Deferred by `docs/decisions.md` Decision 35; the unused engine and its settings tab were removed by Decision 75. Spoken snippets — a phrase that expands into text — are a different feature and do ship.)*
5. **Local-First with Zero Recurring Cost**: Runs 100% locally by default using local STT (Whisper, via `whisper-rs`), Ollama, and grounded retrieval over an Obsidian-style markdown vault.
6. **Companion Extension & Native Desktop**: Windows native app for local capture, meetings, and processing, with a companion browser extension for structured web and conversation capture into the vault.
7. **Universal Dictation, Not Just In-App Voice**: A global push-to-talk hotkey puts transcribed speech directly into whatever app or field had OS focus (Slack, email, code editors) — pasted by default, or typed key by key — not confined to Vox's own window, with a non-intrusive "listening" pill. The text arrives as transcribed; an AI cleanup pass is opt-in. A separate global hotkey shows/hides Vox from anywhere in the OS.
8. **Voice Chat Grounded in Your Own Notes**: Ask a question out loud inside Vox; it retrieves relevant vault notes, answers grounded in them with sources shown, and can speak the answer back via local TTS. *(Deferred for the current desktop-first MVP phase — see `docs/decisions.md` Decision 34. The code behind it is no longer in the tree: Decision 69.)*

## In Scope for MVP
- Push-to-talk capture with floating overlay widget & global hotkey.
- Local Speech-to-Text (Whisper via `whisper-rs`, or Parakeet TDT for dictation) & local Ollama LLM provider toggle (with cloud LLM option).
- Meeting transcript -> Kanban list parser (list-to-board rendering). *(Deferred and not built — see below.)*
- Audio scribble -> structured prompt engine (templates). *(Built, but no surface starts it — see differentiator 3.)*
- Obsidian-style Markdown Vault storage + keyword-ranked note retrieval (embedded LanceDB vector search is decided but not yet built — see `docs/roadmap.md`).
- Global show/hide hotkey and push-to-talk universal dictation (types into whatever app/field has OS focus) with a listening indicator.

## Out of Scope for MVP
- Drag-and-drop Kanban card persistence (list-to-board only).
- GraphRAG / Knowledge graph retrieval.
- Third-party meeting-bot joiners.
- Mobile native application.
- Multi-user / team shared vaults.
- Neural speaker diarization. Meetings label each line with its capture channel (You / Others) and propose acoustic speaker groups for the user to name (`meetings::voiceprint`), but there is no trained speaker model and no voice is remembered across meetings (`maybe_later.md` §11).
- Live external connectors. Vox reads Google Calendar (read-only) and writes to nothing outside itself; Notion/Drive push was decided architecturally (Decision 8), but there is no MCP client.
- Continuous/always-on background capture (structurally excluded — see `docs/decisions.md` Decision 5).

## Deferred for Current Phase
A desktop-first scope reduction (in progress — see `docs/decisions.md` Decision 32 onward) is narrowing the *active* MVP to: global PTT + click-to-talk + local STT + text injection through one Dictation Pill. Items below were taken off the active product surface for this phase. When they were deferred their code was kept in the repository; most of it has since been removed, and each item says what is left. This list grows only as each further scope-reduction step is explicitly approved and completed — `docs/decisions.md` is the authoritative, dated record of what changed and what didn't.

- **Web / Hybrid Dashboard** (Decision 32, Decision 64): the former Next.js web client (`web/`) was retired and removed in favor of a focused, native-first desktop application.
- **Kanban Board** (Decision 33): the meeting-transcript-to-task-board UI and its `process_meeting` pipeline. Neither `KanbanBoard.tsx` nor `process_meeting` is in the tree. The `KanbanCard` type and its storage in `vault/kanban/` remain, and back the TODOs surface, which lists cards as todos.
- **Voice Chat & TTS** (Decision 34): the vault-grounded voice Q&A tab and its optional local "speak back" (Piper). `ChatPanel.tsx`, `process_chat` and `TtsEngine` are no longer in the tree; Talkback, which succeeded them, was removed too (Decision 69), and `tts/` is an interface with no provider behind it. The shared LLM provider client remains.
- **Triggers & MCP** (Decision 35): the configurable trigger-phrase engine and its MCP action dispatch (calendar/reminders). Decision 35 removed both nav entries and the inline dispatch call inside the core capture path — the latter because it ran automatically on every non-chat capture, not only when explicitly configured. Decision 75 then removed `TriggerEngine`, `triggers.json`, the trigger config commands, a settings tab that had reappeared, and the MCP action stubs; nothing had run them since. `McpRouter`'s context and action entry points remain, with no caller.
