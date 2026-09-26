# Vox — API Conventions & Specifications

Per `rules/api-conventions.md`, all Tauri IPC commands return consistent, typed error responses.



## 1. Tauri Commands API (`native/src-tauri/src/commands/`)

Commands live in one module per surface under `commands/` (capture, notes,
vault, models, app, diagnostics, account, web_capture, knowledge, test_lab),
plus `meetings::commands` and `calendar::commands`. All of them are registered
in the `generate_handler!` list in `lib.rs`, and `npm run verify:invoke` fails
CI when a frontend `invoke()` names a command that list does not contain, or
passes arguments it does not take.

### Common Response Shape
All Rust Tauri commands return `Result<T, CommandError>` where `CommandError` serializes to:
```typescript
interface CommandError {
  code: string;     // e.g. "CAPTURE_START_FAILED", "STT_FAILED", "VAULT_BUSY"
  message: string;  // User-facing descriptive message
}
```

### Command Signatures

#### Capture Commands
- `start_capture(mode: String) -> Result<String, CommandError>`
  Starts audio capture and returns the session id. Modes in use: `"voice_note"`
  (the dictation pill's click-to-record) and `"todo"` (press-and-hold on the
  TODOs page). `"scribble"` is still handled by `stop_capture` but no surface
  starts it.
- `stop_capture() -> Result<Option<ProcessedPipelineResult>, CommandError>`
  Stops audio capture. `AudioRecorder::stop` reports `had_audio` — whether
  sustained, above-ambient input arrived while recording — and this command
  gates on it: without it, it emits `capture-state-changed` with
  `status: "NO_SPEECH"` and returns `Ok(None)` **without invoking the STT
  engine**. Otherwise it emits `status: "TRANSCRIBING"` and runs the shared
  dictation pipeline (`capture::dictation`: transcribe, the deterministic
  pass, then the opt-in cleanup — skipped for todos). An empty result returns
  `Ok(None)`. Every non-empty transcript is saved as a Voice Note, and
  `"todo"` also writes a Kanban card. `start_capture` accepts only
  `"voice_note"` and `"todo"`. Every stop that announced `TRANSCRIBING` ends
  with `SUCCESS` or `ERROR`, so the pill never stays on "Transcribing…".

  Universal dictation (the global push-to-talk hotkey) does **not** go through
  these commands: `hotkeys::stop_dictation_session` stops the recorder, runs
  the same `capture::dictation` pipeline, and delivers the text into the field
  that had focus (`hotkeys::injection`), bypassing Tauri IPC.

#### Todo Commands (Kanban cards)
- `get_kanban_cards() -> Result<Vec<KanbanCard>, CommandError>`
  Reads every card file in `<vault>/kanban/`; the TODOs surface lists them.
- `create_manual_todo(title: String) -> Result<KanbanCard, CommandError>`
- `set_todo_status(id: String, status: String) -> Result<KanbanCard, CommandError>`
- `delete_todo(id: String) -> Result<(), CommandError>`

#### Web Capture Commands
Web capture (`docs/capture.md`) — distinct from the audio capture commands
above. Acquisition is separate from interpretation:
every command below either stores or reads a capture; none of them can lose one.

- `get_capture_bridge_status() -> Result<CaptureBridgeStatus, CommandError>`
  Whether browser capture is enabled, whether a loopback listener is actually
  bound, the port in use (which differs from the configured one when that port
  was taken), the pairing token, and the desktop capture hotkey.
- `set_capture_bridge_enabled(enabled: bool) -> Result<CaptureBridgeStatus, CommandError>`
  Starts or stops the listener and persists the choice, generating a pairing
  token the first time it is switched on. Errors with
  `CAPTURE_BRIDGE_START_FAILED` if no loopback port could be bound.
- `set_capture_bridge_port(port: u16) -> Result<CaptureBridgeStatus, CommandError>`
  Rebinds on a new port. Rejects ports below 1024 with `INVALID_PORT`.
- `regenerate_capture_pairing_token() -> Result<CaptureBridgeStatus, CommandError>`
  Issues a new token, immediately unpairing every browser.
- `set_capture_analyze_on_capture(enabled: bool) -> Result<CaptureBridgeStatus, CommandError>`
  Whether Vox analyses a capture as soon as it lands. Storage never depends
  on this.
- `get_captures() -> Result<Vec<VaultFile>, CommandError>`
  Captures only, newest first. Imported documents stay on `get_vault_files`.
- `get_capture(id: String) -> Result<VaultFile, CommandError>`
- `get_capture_payload(id: String) -> Result<WebCapturePayload, CommandError>`
  The raw structured payload as captured — written once, never rewritten.
- `renormalize_capture(id: String) -> Result<VaultFile, CommandError>`
  Rebuilds the markdown from that payload, preserving id, capture time and
  version history.
- `delete_capture(id: String) -> Result<(), CommandError>` Moves it to Trash.
- `import_web_capture(payload_json: String) -> Result<VaultFile, CommandError>`
  Ingests a payload handed over inside the app rather than over the bridge.

`analyze_vault_file`, `summarize_vault_file` and `create_scribble_from_vault_file`
accept a capture id as well as a file id — a capture is a Vault artifact, and
is analysed and promoted by the same code paths. `reprocess_vault_file` is a
no-op for a capture: its text was normalized from a payload, not extracted
from bytes, so re-running document extraction on it could only destroy it.

Captures also arrive over a loopback HTTP bridge rather than through Tauri
IPC (`POST http://127.0.0.1:<port>/v1/capture`, `X-Vox-Token` required).
That surface's contract, limits and threat model are in `docs/capture.md` §5.

Progress is broadcast on the `capture-progress` event with a `stage` of
`SAVING | SAVED | ANALYSING | ANALYSED | FAILED`.

#### Meeting Commands
- `start_meeting(title?: string, captureSystemAudio?: bool, devices?: MeetingDevices) -> Result<Meeting, CommandError>`
  Opens the microphone and (unless disabled) the system loopback, and begins recording. Refuses with `MEETING_NO_SPEECH_MODEL` when no Whisper model is installed, rather than recording audio that can never become a transcript, and with `MEETING_ALREADY_RECORDING` when one is already in flight.
- `pause_meeting()` / `resume_meeting() -> Result<(), CommandError>`
  Paused audio is discarded, not stored as silence, so a meeting's length matches the speech in it.
- `stop_meeting() -> Result<Meeting, CommandError>`
  Returns only once the transcript is complete: capture stops, the segmenter flushes the sentence still being spoken, the decoder drains its backlog, and the audio checkpoints are merged. Starts the report too when `settings.meetings.autoSummarize` is on.
- `get_meeting_recording_status() -> MeetingRecordingStatus`
  Elapsed time, which streams are bound, whether either has ever carried audio, and the decode counters.
- `list_meetings() -> Result<Vec<MeetingListItem>, CommandError>`
- `get_meeting(meetingId) -> Result<MeetingDetail, CommandError>`
  Metadata, transcript, report and notes in one round trip.
- `search_meetings(query, limit?) -> Result<Vec<MeetingSearchHit>, CommandError>`
- `rename_meeting(meetingId, title)` / `save_meeting_notes(meetingId, notes)` / `delete_meeting(meetingId)`
  Deleting removes the meeting's directory — transcript, report and audio together.
- `open_meeting_folder(meetingId) -> Result<(), CommandError>`
- `list_meeting_templates() -> Vec<MeetingTemplate>`
  The six bundled templates plus anything in `<config>/meeting-templates/`.
- `generate_meeting_summary(meetingId, templateId?, language?, instructions?, force?) -> Result<(), CommandError>`
  Returns as soon as the background task is spawned. Progress arrives on `meeting-summary-progress`; the result is read back with `get_meeting_summary`.
- `cancel_meeting_summary(meetingId) -> bool` — restores the previous report.
- `get_meeting_summary(meetingId)` / `save_meeting_summary(meetingId, markdown)`
  Saving the user's own edits clears the English cache, so a later regeneration cannot silently discard them.
- `promote_meeting_to_scribble(meetingId) -> Result<Scribble, CommandError>`
- `pick_meeting_audio_file() -> Result<Option<String>, CommandError>` — the OS file picker, in Rust because the window's capability set grants no dialog permission to the frontend.
- `meeting_audio_extensions() -> Vec<String>` / `import_meeting_audio(path, title?)` / `retranscribe_meeting(meetingId)` / `cancel_meeting_import(key)`

Events: `meeting-state-changed`, `meeting-audio-level`, `meeting-transcript-segment`, `meeting-transcription-progress`, `meeting-transcription-warning`, `meeting-summary-progress`, `meeting-import-progress`. Shapes are in `native/src/types/meetings.ts`; the subsystem is documented in `docs/meetings.md`.

#### Settings Commands
- `get_settings() -> Result<AppSettings, CommandError>`
  Returns the current provider/STT/meetings/hotkey configuration (see `docs/data-model.md` §4).
  Provider API keys arrive as the placeholder `__vox_stored_key__`, never as
  their values; the keys themselves live in the OS credential store
  (`providers::secrets`).
- `save_settings(settings: AppSettings) -> Result<(), CommandError>`
  Persists settings to `<config>/settings.json` and updates the running app's
  in-memory config immediately. A placeholder sent back for an API key keeps
  the stored key. Changing the vault directory is refused with `VAULT_BUSY`
  while a meeting is recording; otherwise every vault-backed store follows it.
- `update_hotkeys(hotkeys: HotkeySettings) -> Result<(), CommandError>`
  Re-registers the global hotkeys at once. Whether each one actually
  registered — another application can hold the same combination — is
  broadcast on the `hotkey-status-changed` event.

---

The Tauri IPC command layer is the primary internal API surface for Vox desktop.

