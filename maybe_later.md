# Maybe Later (Deferred Features & Architecture Backlog)

Deferred features, removed half-features, and architecture ideas set aside for
later — each with enough context to pick it back up.

> [!NOTE]
> When a feature, visual affordance, or architectural idea is speculative,
> half-implemented, or deferred, it **must** be documented here rather than
> left as ghost UI or dead code. Deferred *code* inside a working feature gets
> a `TODO(context):` comment next to it instead. See
> [`rules/maybe-later.md`](rules/maybe-later.md).

Item numbers are stable identifiers: decision records and other documents
cite them (`maybe_later.md` §11), so an item keeps its number when others are
merged or removed, and new items are appended.

---

## Backlog Items

### 1. Talkback: Turn Detection, a Second Voice, Echo Cancellation, Offline Install

- **Status**: Deferred — the code was removed; the research stands
- **Area**: None in the tree. Would return as `native/src-tauri/src/talkback/`
  and `native/src-tauri/src/tts/` (removed; `docs/decisions.md` Decision 69).
- **Original Context**:
  - Talkback was a spoken back-and-forth with the assistant: energy-based turn
    detection with a 700 ms hangover, Piper for speech, and a downloaded voice
    installation. It was removed whole, and Decisions 47–56 describe code that
    is no longer present. What survives is the research below, which was done
    against real constraints and is worth not repeating.
- **Concept & Implementation Blueprint**:
  - **Turn detection and wake word.** Energy plus a hangover cannot tell a
    thinking pause from the end of a thought. **Silero VAD** (MIT; ONNX; ~1 ms
    per 30 ms frame on one CPU thread; Rust ports `silero-vad-rs`,
    `voice_activity_detector`) for speech/non-speech, **Pipecat Smart Turn v3**
    (open weights, ~8M parameters, <60 ms CPU) for "has this person finished",
    and **openWakeWord** / **microWakeWord** (Apache-2.0, via their ONNX
    exports) if wake-word activation is ever wanted — never on by default.
    All three are blocked on the `ort`-on-Windows packaging proof
    (`docs/spikes/onnx-windows.md`), which is written and has never been run.
  - **A second local voice.** **Kokoro-82M** leads: Apache-2.0 code *and*
    weights (redistributable with an installer), ~350 MB, eight languages
    including Hindi, several Rust ONNX ports with chunked streaming.
    **Chatterbox is not a candidate** despite its quality: it needs a PyTorch
    runtime. Piper upstream moved to `OHF-Voice/piper1-gpl` (GPL-3.0,
    compatible with Vox's AGPL-3.0 and irrelevant anyway for a separate
    process). Benchmark on real Windows hardware before choosing:
    time-to-first-audio, total synthesis, CPU, RAM, Hindi and mixed-language
    speech, startup, package size, interruptibility.
  - **Echo cancellation for barge-in.** With speakers and an open microphone
    the assistant hears itself. The old mitigation multiplied the speech
    threshold by 2.5× while speaking. Real AEC needs the playback signal as a
    reference, so playback must move into Rust — blocked by the `rodio`/`cpal`
    0.17 conflict in Decision 49. `webrtc-audio-processing` is the usual
    answer; a cheaper step is having the frontend report playback amplitude so
    the guard scales with the actual output level.
  - **Offline and ARM64 voice installation.** ARM64 needs only a manifest
    entry once an upstream `aarch64` asset exists. Offline is a packaging
    variant that ships the engine and one voice as bundled resources. A third,
    cheaper option: let setup import a folder copied from another machine,
    verified against the same manifest checksums.

### 2. A Second Local TTS Voice (Kokoro)

- **Status**: Merged into item 1, which carries its research. The number is
  kept because other documents cite it.

### 3. Acoustic Echo Cancellation for Barge-In

- **Status**: Merged into item 1, which carries its research. The number is
  kept because other documents cite it.

### 4. Offer to Move a Legacy Data Folder

- **Status**: Deferred — the unsafe default is fixed; the migration is not
- **Area**: Native backend (`native/src-tauri/src/lib.rs` —
  `choose_base_dir`)
- **Original Context**:
  - Release builds used to fall back to `current_dir()/.vox`, and a packaged
    app's working directory is whatever launched it: `C:\Windows\System32`
    from autostart, the install folder from a Start Menu shortcut. Settings
    could fail to save, or land somewhere different on each launch.
  - Fixed for new installs: with no existing data folder, a release build now
    uses `%APPDATA%\com.vox.app` (the same folder Tauri's `app_data_dir()`
    names), and `VOX_HOME` overrides everything.
  - Deliberately *not* done: an existing `.vox` or `.relay` beside the
    executable or in the working-directory tree is still used where it is.
    Moving someone's vault, notes and settings as a side effect of an upgrade
    is the kind of change that loses data.
- **Concept & Implementation Blueprint**:
  - On startup, when the resolved folder is one of those legacy locations and
    the app-data folder is empty, offer to copy it there — explicitly, with the
    old copy left in place, matching the "never move, migrate or delete"
    promise the Vault Directory Location setting makes (Decision 38).
  - The Vault Directory Location setting already overrides the vault, so the
    copy mainly concerns `config/`: settings, models, the STT cache. Provider
    API keys live in the OS credential store and are unaffected.

### 5. Offline and ARM64 Voice Installation

- **Status**: Merged into item 1, which carries its research. The number is
  kept because other documents cite it.

### 6. Capturing a Web Page From Vox's Own Hotkey

- **Status**: Deferred — blocked by the browser permission model, not by missing code
- **Area**: Native backend (`native/src-tauri/src/capture/web/bridge.rs`, `hotkeys/mod.rs`), browser extension (`native/src/webcapture/background.ts`)
- **Original Context**:
  - Capture was asked for as an OS-level Vox hotkey that reads whatever page
    the user is looking at. It ships the other way round: the browser owns the
    trigger, and Vox's `capture_hotkey` opens the Captures surface
    (`docs/decisions.md` Decision 57).
  - The blocker is `activeTab`. Chrome grants it only for a gesture made inside
    the browser — the extension's action, a context-menu item, a `commands`
    shortcut, an omnibox suggestion. A global desktop hotkey is none of those.
- **Why it is deferred rather than done**:
  - The only way to make it work today is `<all_urls>` (or
    `optional_host_permissions` granted once), which trades the whole
    least-privilege story for one shortcut.
  - An MV3 service worker is terminated when idle, so Vox cannot simply push a
    request to it; keeping one alive in practice means native messaging.
- **Concept & Implementation Blueprint**:
  - Move the transport to native messaging (`runtime.connectNative`), which
    keeps the worker alive for the life of the port and gives Vox a channel it
    can initiate on — the same migration Decision 58 names as the exit from the
    loopback bridge, so the two would land together.
  - Add `optional_host_permissions: ["<all_urls>"]` and request it from the
    options page with the trade stated plainly.
  - Keep today's behaviour as the default and the fallback.

### 7. Screenshot and OCR Fallback for Unreadable Pages

- **Status**: Deferred — deliberately not a rung on the capture ladder
- **Area**: Native backend (`native/src-tauri/src/capture/web/`), browser extension
- **Original Context**:
  - The capture ladder ends at `text_only` and then refuses. A canvas-rendered
    app or a scanned document produces nothing, and Vox says so rather than
    saving an empty artifact.
  - Screenshot plus OCR was rejected for v1: the product principle is
    structured acquisition, and the cases it would serve are already labelled
    honestly rather than silently mis-captured.
- **Concept & Implementation Blueprint**:
  - `chrome.tabs.captureVisibleTab` needs only the `activeTab` grant Vox
    already has, but captures the viewport, not the page — scroll-and-stitch
    would be needed too.
  - OCR is the real cost: Tesseract bindings or an ONNX model is a large native
    dependency with a Windows build story, for output below every other rung.
  - If built, it must be a *supplementary* artifact (`ScribbleAttachment`
    exists), never a substitute that lets `coverage` claim more than the text
    extraction earned.

### 8. Firefox Support for the Capture Extension

- **Status**: Deferred — structurally compatible, not validated
- **Area**: `native/browser-extension/manifest.json`, `native/src/webcapture/background.ts`
- **Original Context**:
  - The extension targets Chrome and Edge. Firefox supports
    `scripting.executeScript` under MV3 from Firefox 101 and grants
    `activeTab` on the same gestures, so the extraction layer is not
    Chrome-specific — but no Firefox testing was done, so it is not claimed
    (`docs/capture.md` §13).
- **Concept & Implementation Blueprint**:
  - Add `browser_specific_settings.gecko.id`, and on older Firefox a
    `background.scripts` event page; the build already emits an ES module and
    an IIFE.
  - Firefox treats `host_permissions` as optional, so pairing must handle
    "granted later" — **Save and test** on the options page is the place to
    call `permissions.request`.
  - The bridge already accepts `moz-extension://` origins, with a test.

### 9. Capturing File Bytes and Image Data

- **Status**: Deferred — technically possible, deliberately refused for v2
- **Area**: `native/src/webcapture/dom.ts`, `native/src/webcapture/types.ts`, `native/src-tauri/src/capture/web/`
- **Original Context**:
  - Files and images are recorded as metadata plus a reference, with
    `content_captured: false`. A content script's `fetch` would resolve
    authenticated asset URLs with the page's cookies; it was refused for four
    reasons recorded in `docs/capture/RESEARCH.md` §6.1 — the gesture was
    "capture this page", the payload contract is text-only (which keeps
    normalization a total function over untrusted input), fetching
    authenticated URLs makes Vox a client of the site's API, and
    metadata-now/bytes-later loses nothing.
- **Concept & Implementation Blueprint**:
  - An explicit, off-by-default setting with its own consent copy.
  - Fetch in the **service worker**, not the content script, and store bytes
    beside the artifact (`vault/captures/<id>/files/`) so the text-only payload
    contract survives; `content_captured` becomes a real measurement.
  - Per-file and per-capture ceilings, a type allowlist, a same-origin rule,
    and a note on every file a limit rejected. `sandbox:/mnt/data/…`
    references stay metadata-only.

### 10. A Configurable Traversal Budget

- **Status**: Deferred — the budget is a constant per source, and hitting it is reported
- **Area**: `native/src/webcapture/traversal/budget.ts`, `native/src-tauri/src/settings/`, `native/src/components/settings/CaptureSettingsView.tsx`
- **Original Context**:
  - The reveal pass stops after 10 s on a conversation (6 s on a document) and
    reports `time_budget` with `partial` coverage — roughly 450–500 turns
    (`docs/capture/BENCHMARKS.md`). The user cannot say "take longer".
  - Left constant because the extension cannot read Vox's settings without
    another bridge round-trip on the capture path.
- **Concept & Implementation Blueprint**:
  - Carry the budget on `/v1/health`, which pairing already calls, and cache it
    in `chrome.storage.local`.
  - Offer a choice ("quick", "thorough", "as long as it takes") mapping to
    `maxMs`/`maxSteps`, not a millisecond field.
  - A longer budget may change how much is read, never what a capture is
    allowed to claim.

### 11. Neural Speaker Embeddings and a Voice Library

- **Status**: Backlog (acoustic speaker grouping ships; embeddings do not)
- **Area**: Native backend (`native/src-tauri/src/meetings/voiceprint.rs`, `native/src-tauri/src/meetings/speakers.rs`)
- **Original Context**:
  - Vox groups a meeting's turns by speaker from 26 MFCC statistics per turn,
    cosine-normalised and clustered with average linkage, and presents the
    grouping as a *proposal* with a playable span per speaker.
  - A neural embedding (x-vector, ECAPA) is better, and means an ONNX runtime,
    a model download with its own licence question, and a consent flow — an
    embedding stored across meetings **is** biometric data, which the classical
    path never creates. `voiceprint.rs`'s module docs list what the classical
    features cannot do.
- **Concept & Implementation Blueprint**:
  - The ONNX runtime behind a feature flag (as `parakeet` is) and an
    off-by-default setting.
  - Swap the vector `voiceprint::voiceprint` returns, keeping `TurnPrint` so
    `voiceprint::cluster` and `speakers::assign_speakers` are untouched;
    recalibrate `DEFAULT_SPLIT_DISTANCE` for the new scale.
  - Only then a voice library: enrolment, a "Remember voices across meetings"
    toggle that never defaults on, and list/rename/merge/delete.
  - Separately, keeping the microphone and loopback streams apart on disk would
    take the local user's voice out of the clustering problem entirely.

### 12. Calendar Attendees as a Speaker Hint

- **Status**: Backlog (calendar sync ships; the speaker hint does not)
- **Area**: Native backend (`native/src-tauri/src/calendar/agenda.rs`, `native/src-tauri/src/meetings/speakers.rs`)
- **Original Context**:
  - Each matched calendar event carries its attendee list, and nothing uses it:
    a three-person call whose voices cluster into five groups asks the user to
    name five.
- **Concept & Implementation Blueprint**:
  - An `expected_speakers: Option<usize>` in `DetectionSettings`, defaulted from
    the matched event's non-resource attendee count, with a user setting
    winning over it.
  - A *stopping hint*, not a target: keep merging while the cluster count
    exceeds it, never split to reach it — the calendar says who was invited,
    not who spoke.
  - Never auto-assign names; offer the attendee list as autocomplete in
    `SpeakerPanel`'s rename field. Calendar text is external data, never an
    instruction (`rules/security.md`).

### 13. Auto-Learning Dictionary Words From Corrections Made in Other Apps

- **Status**: Removed — the setting existed, the mechanism could not
- **Area**: Native backend (`native/src-tauri/src/settings/mod.rs`), Settings › Engine
- **Original Context**:
  - `AudioInputSettings::auto_learn_words` shipped as a toggle, on by default,
    read by nothing. It promised to learn from corrections made "in the target
    app", which needs accessibility text APIs — reading the contents of
    somebody else's window. `hotkeys::injection` can put text into a field and
    has no way to read one back, by design.
  - What does exist is the version that needs no such access: correcting a
    phrase in a Voice Note with **Teach Vox this correction** ticked records
    the mapping (`AppSettings::learn_correction`), and the recognizer is primed
    with it from then on.
- **Concept & Implementation Blueprint**:
  - Only with a decision to take an accessibility-API dependency (UI
    Automation on Windows) and the privacy posture that goes with it — Vox
    would be reading the user's other applications, a different promise from
    recording their microphone when they press a key.

### 14. A Summary Across a Whole Series

- **Status**: Backlog — the series object it needs exists; this does not
- **Area**: Native backend (`native/src-tauri/src/meetings/summary/processor.rs`, `native/src-tauri/src/meetings/series.rs`)
- **Original Context**:
  - Recurring meetings have an identity (`meetings::series`), which answers
    "which meetings are the standup" but not "what has changed in this standup
    over six weeks".
- **Concept & Implementation Blueprint**:
  - A change-over-time template: decisions made then reversed, items carried
    forward without moving, commitments and whether they landed.
  - Map-reduce, not concatenation: summarise each occurrence (most already have
    a report), then reduce the reports.
  - Its own progress shape and cancellation — a local model over six meetings
    takes minutes — and a cache keyed on the set of meeting ids, so a seventh
    occurrence visibly invalidates it.

### 15. Retire the Knowledge Graph's Node-Position Cache

- **Status**: Deferred — superseded, not yet removed
- **Area**: Native frontend (`native/src/components/knowledge/graph/graphStorage.ts` — `loadNodePositions`, `saveNodePositions`, `clearNodePositions`, `GraphPositionMap`; the "Reset layout" affordance in `KnowledgeGraphView`)
- **Original Context**:
  - The force-directed layout is seeded from `Math.random`, so coordinates are
    persisted to localStorage to stop the graph rearranging between sessions.
    The Rings view is seeded and bit-reproducible (`ringsLayout.test.ts`) and
    uses none of this.
- **Concept & Implementation Blueprint**:
  - Remove once the force view adopts a seeded layout or stops being offered:
    the three functions, `POSITIONS_STORAGE_KEY`, `GraphPositionMap`, and the
    "Reset layout" flow. Leave the stored key unread rather than migrating it.

### 16. Action-Item Extraction for Meetings, Voice Notes and Scribbles

- **Status**: Deferred — the TODOs surface exists; three of its feeds do not
- **Area**: Native backend (`native/src-tauri/src/meetings/summary/`, `native/src-tauri/src/pipeline/enrichment.rs`), feeding `vault::VaultManager::record_extracted_todos`
- **Original Context**:
  - The one structured action-item extractor is `ContextActionItem` in
    `capture/web/context.rs` (LLM and deterministic implementations), serving
    browser captures and wired through to persisted todos.
  - Meetings have "Action Items" only as a prose heading in a summary template;
    nothing parses it back out. Voice notes and scribbles go through
    `pipeline::enrichment`, which derives title, summary, topics, entities,
    concepts and questions — not action items.
- **Concept & Implementation Blueprint**:
  - The write side is shared and idempotent on `(source id, title)`; each new
    extractor only produces candidates.
  - **Meetings**: a structured pass beside the summary, with
    `source_turn_ordinals` like `ContextActionItem`, rather than parsing
    generated prose back out.
  - **Voice notes and scribbles**: add `action_items` to the existing
    enrichment call, backed by a deterministic cue scanner so the feature does
    not vanish when no model is configured — Vox's zero-cost default.
  - Watch the false-positive rate: "I should probably look at that" is not a
    commitment.

### 17. Reviewing a Dictation Cleanup After It Lands

- **Status**: Removed — dead code; cleanup now runs before injection
- **Area**: Native backend (`native/src-tauri/src/commands.rs`, `hotkeys/injection.rs`, `capture/text_normalize.rs`)
- **Original Context**:
  - Three commands — `rewrite_dictation`, `get_cleanup_target`,
    `apply_dictation_cleanup` — let a surface propose a cleanup of the last
    dictation *after* it was typed, then select exactly that text and type over
    it. No surface ever called them: the pipeline had meanwhile moved cleanup
    in front of injection, so there was nothing left to review in place.
  - The implementation is in git history at `ea43f1f`, including the part
    worth keeping: `text_normalize::cursor_steps`, which counts Shift+Left
    steps by grapheme cluster so a selection over Devanagari cannot swallow
    text the user wrote before dictating, and `injection::select_previous`.
- **Concept & Implementation Blueprint**:
  - Worth reviving only as a mode — "type the raw text now, offer the cleanup
    after" — for users who want cleanup without its latency.
  - The pill shows the diff and an explicit accept; the target is consumed on
    use so a double press cannot delete twice; nothing is selected unless the
    focus context still matches the one captured at injection.

### 18. Streaming Dictation

- **Status**: Backlog — the pipeline exists as a measurement instrument; production decodes after release
- **Area**: Native backend (`native/src-tauri/src/capture/streaming_pipeline.rs`, `capture/eligibility.rs`, `meetings/segmenter.rs`, the recorder callback in `capture/mod.rs`, `hotkeys/mod.rs`)
- **Original Context**:
  - Hotkey dictation decodes the whole utterance after the key is released, so
    the wait grows with how long the user spoke. `ShadowDictationRunner`
    segments audio, decodes each segment and gates a per-segment cleanup on
    `evaluate_cleanup_eligibility` — but only the ignored benchmark
    (`capture::benchmark::run_phase4_diagnostics`) drives it.
  - An unread `dictation_streaming_shadow` setting and a recorder sink nothing
    attached to were removed, so the capture callback no longer takes a lock
    per buffer for them.
- **Concept & Implementation Blueprint**:
  - Re-add the sink delivering **16 kHz mono**: the callback runs at the device
    rate and the segmenter's timing assumes 16 kHz, so keep a resampler state
    across chunks rather than resampling each buffer in isolation.
  - Run in shadow first — decode alongside the batch path, log per-segment
    latency, backlog and the difference from the batch transcript, never alter
    output — and switch only when the data says stitched segments are at least
    as accurate.
  - Segments cut at `MaxDuration` can split a word; plan a final reconcile at
    release. Dictation and meetings share the Whisper lock, so measure the
    backlog while a meeting records.

### 19. Voice Triggers That Run Actions

- **Status**: Removed — nothing ever executed a trigger
- **Area**: Former `native/src-tauri/src/triggers/` and `native/src/components/settings/TriggerSettings.tsx`; `native/src-tauri/src/mcp/mod.rs`; `native/src-tauri/src/actions/`
- **Original Context**:
  - Settings had an **MCP Triggers** tab that saved phrase→action mappings to
    `triggers.json`. `TriggerEngine::match_transcript` was never called outside
    its tests, and the MCP dispatch it would have fed returned hard-coded
    success strings ("Scheduled calendar event: …") without doing anything —
    while logging the transcript at info level. Decision 35 had already
    deferred Triggers/MCP; the settings tab had come back regardless.
- **Concept & Implementation Blueprint**:
  - Match against the finished dictation text (after
    `capture::dictation::finish_text`), never raw ASR output.
  - Route every action through `actions::ActionDispatcher`, so confirmation
    gating is the same as the UI's; a matched trigger says what it is about to
    do and asks first.
  - A real outbound MCP client, or native integrations. Calendar access is
    read-only today; creating events needs a new OAuth scope and its own
    consent.
  - Never log transcript text.

### 20. Per-Window Command Permissions

- **Status**: Backlog — hardening
- **Area**: `native/src-tauri/capabilities/`, `native/src-tauri/build.rs`, `native/src-tauri/src/lib.rs`
- **Original Context**:
  - Capabilities scope *plugin* permissions per window, but every app command
    in `generate_handler!` — about two hundred — is callable from every window,
    including the dictation pill, the meeting overlay and the reminder card,
    which each need a handful. A bug in an auxiliary window can reach anything
    the main window can.
- **Concept & Implementation Blueprint**:
  - Declare the commands in an app manifest
    (`tauri_build::Builder::new().app_manifest(tauri_build::AppManifest::new().commands(&[…]))`),
    which generates an `allow-<command>` permission for each; grant the main
    window a set containing all of them and each auxiliary window only its own.
  - A missing permission fails at run time, not compile time, so every window's
    flows need exercising on Windows before this ships.

### 21. Building Installers in the Release Pipeline

- **Status**: Backlog
- **Area**: `.github/workflows/release.yml`
- **Original Context**:
  - The release workflow bumps `VERSION` and writes `CHANGELOG.md`, and builds
    nothing; installers are produced by hand. CI now compiles and tests on
    Windows, but never bundles.
- **Concept & Implementation Blueprint**:
  - A `windows-latest` job on the release tag running `npm run tauri build`
    and attaching the installer to the GitHub release; code signing (a
    certificate held as a secret) decided separately.
  - The same job is the natural place to finally run the ONNX packaging spike
    (`docs/spikes/onnx-windows.md`) against an installed build, which unblocks
    item 1 and settles whether Parakeet (on by default) survives packaging.
