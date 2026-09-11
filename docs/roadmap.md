# Vox — Roadmap & Competitive Gap Backlog

This tracks what's real vs. stubbed today, and what should come next. It
exists so "not implemented yet" is written down explicitly instead of silently
implied by absence — see `rules/version-and-changelog.md`'s spirit of keeping
the living spec honest.

## Shipped this round (real, not mocked)
- Real microphone capture (`cpal`, resampled to 16kHz mono) — previously wrote an empty WAV placeholder.
- Real local transcription (`whisper-rs` / whisper.cpp) — previously returned a hardcoded fake transcript string.
- Global show/hide hotkey and push-to-talk universal dictation with OS-wide text injection (`enigo`) and a listening indicator window.
- Persisted settings (provider/STT/hotkeys) — previously the settings UI didn't call the backend at all.
- Keyword-ranked note retrieval (`VaultManager::search_notes`) as real (if simple) grounding for voice chat.

## Still stubbed / not real yet — prioritized backlog

1. **Embedded vector RAG (LanceDB)** — `docs/decisions.md` Decision 6 already
   commits to this; today's `search_notes` is plain keyword/term-overlap
   scoring, not embeddings. This is the highest-priority follow-up since
   voice chat and future semantic search depend on retrieval quality.
   Competitive research flags LightRAG as the cheaper alternative to full
   GraphRAG if/when note volume grows past ~1K documents.

2. **Real MCP client wiring** — `McpRouter::dispatch_action` (`native/src-tauri/src/mcp/mod.rs`)
   returns hardcoded success strings for every action type; no MCP server is
   actually called. Decision 8 names the three servers to reuse as-is
   (`nspady/google-calendar-mcp`, `makenotion/notion-mcp-server`,
   `isaacphi/mcp-gdrive`) — none are wired up yet. This is the "live external
   connectors" gap relative to Onyx-style tools; Calendar is the MVP target,
   Notion/Drive are explicitly Post-MVP per `docs/product.md`.

3. **Telling two remote participants apart** — meetings label the capture
   channel (`You` / `Others`), which is a measurement rather than an
   inference, and is all Decision 68 claims. Two people on the far end of a
   call are both `Others`. The acoustic diarization `meetings_v2` shipped in
   0.31.0 was removed with the rest of that subsystem and has not been
   restored; the voiceprint library above it (`maybe_later.md` item 11) and
   calendar attendees (item 12) remain deferred, as does calendar matching and
   meeting reminders.

4. **Multi-user / team features** — explicitly flagged in `docs/decisions.md`
   as "noted for later, not decided," and the IDE Build Prompt calls scope
   creep toward this a named product risk. Decision 12's Supabase-based real
   auth is a reasonable foundation to extend into this later, but no sharing
   model, permissions, or shared-vault schema exists.

5. **Continuous background capture** — structurally excluded on purpose
   (Decision 5 — PTT/on-screen-triggered capture only, no meeting-bot or
   always-on recording). Not a gap to close; listed here only so it isn't
   mistaken for an oversight.

## Explicitly not gaps (already real)
- Global hotkey and universal dictation — shipped, not stubs.
- Meeting recording, transcription and reports — real. Dual-stream capture,
  streaming segmentation, durable checkpointing, crash recovery, templated
  summarization, audio import and re-transcription. See `docs/meetings.md`.
- Hallucination screening on transcripts — real. Every meeting segment is
  measured for voiced time before decoding and screened for decoder loops and
  subtitle filler after (`capture::speech_health`); a rejected span is
  discarded with its reason recorded.
- Configurable trigger-phrase matching (`TriggerEngine`) — real, just its
  downstream MCP dispatch (item 2 above) is the stub.
