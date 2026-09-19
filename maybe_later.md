# Maybe Later (Deferred Features & Architecture Backlog)

This document tracks deferred features, rejected/postponed UI patterns, and architecture concepts that have been set aside for future evaluation. 

> [!NOTE]
> When a feature, visual affordance, or architectural idea is identified as speculative, half-implemented, or deferred to keep the current surface clean and reliable, it **must** be documented here rather than left as ghost UI or commented-out code. See [`rules/maybe-later.md`](rules/maybe-later.md).

---

## Backlog Items

### 1. Neural Turn Detection and Wake Word for Talkback

> [!IMPORTANT]
> **The code this item points at is not in the tree.** `talkback/` and `tts/`
> were removed, and `docs/decisions.md` Decision 69 records that. The research
> and the blueprint below still stand; the claims about what already exists do
> not. Read "the seam is ready" as "here is the seam to build".


- **Status**: Deferred (V2) — architecture ready, models not shipped
- **Area**: Native backend (`native/src-tauri/src/talkback/turn.rs::TurnDetector::push`, `talkback::ActivationMode`)
- **Original Context**:
  - Talkback V1 detects turns with energy plus an adaptive noise floor and a 700 ms hangover. It works, and it is the same shape as the meeting live clock's speech flag, but it cannot tell a thinking pause from the end of a thought — so the hangover is a compromise between cutting people off and feeling sluggish.
  - `ActivationMode::WakeWord` is a real settings value that the engine refuses with a clear message. No always-on listener ships, deliberately.
- **Concept & Implementation Blueprint**:
  - **Silero VAD** (MIT, code and weights; ONNX; ~1 ms per 30 ms frame on one CPU thread; mature Rust ports in `silero-vad-rs` and `voice_activity_detector`) replaces the energy gate for speech/non-speech.
  - **Pipecat Smart Turn v3** (open weights, open training data, ~8M parameters, <60 ms CPU) answers the harder question — has this person *finished* — from the waveform rather than the transcript. That is what removes the hangover tradeoff.
  - **openWakeWord** / **microWakeWord** (both Apache-2.0) for `wake_word` activation. Both are Python/MCU-targeted today, so the realistic path is their ONNX exports, not the frameworks.
  - All three land through the same seam: `TurnDetector::push` takes a frame and returns a `TurnEvent`. Same signature, better decision.
- **Blocked on**: the `ort`-on-Windows packaging proof. That spike is now written — `docs/spikes/onnx-windows.md`, behind the `onnx-spike` cargo feature — and has never been run; it needs a Windows machine and half an hour. One run unblocks this item and G6 denoise together.

### 2. A Second Local TTS Provider for Talkback (Kokoro)

> [!IMPORTANT]
> **The code this item points at is not in the tree.** `talkback/` and `tts/`
> were removed, and `docs/decisions.md` Decision 69 records that. The research
> and the blueprint below still stand; the claims about what already exists do
> not. Read "the seam is ready" as "here is the seam to build".


- **Status**: Deferred (V2) — trait shipped, provider not
- **Area**: Native backend (`native/src-tauri/src/tts/`)
- **Original Context**:
  - `tts::TtsProvider` exists with Piper behind it, so a second engine is an addition rather than a refactor. What has *not* happened is the benchmark: Piper, Kokoro and Chatterbox were researched but not measured, because measuring them in a Linux CI container says nothing about a Windows laptop (`docs/talkback/BENCHMARKS.md`).
  - Piper also needs attention independently: `rhasspy/piper` was archived read-only in October 2025 and active work moved to `OHF-Voice/piper1-gpl` (GPL-3.0 — compatible with Relay's AGPL-3.0, and irrelevant anyway since Relay shells out to a separate process). The settings copy and doc comments should point at the maintained binary.
- **Concept & Implementation Blueprint**:
  - **Kokoro-82M** is the leading candidate: Apache-2.0 for both code *and* weights (so it is redistributable with a desktop installer), ~350 MB, 8 languages including Hindi, and several Rust ONNX ports (`Kokoros`, `kokoroxide`, `kokoro-en`, `tts-rs`) with chunked streaming.
  - **Chatterbox is not a candidate** despite being MIT and sounding better: it needs a PyTorch runtime, which disqualifies it as a Tauri dependency regardless of quality.
  - Benchmark on real Windows hardware before adopting: time-to-first-audio, total synthesis, CPU, RAM, English, Hindi, mixed-language speech, startup time, packaging size, and interruptibility. Piper stays the default until a measurement says otherwise — being newer is not evidence.

### 3. Acoustic Echo Cancellation for Talkback Barge-In

> [!IMPORTANT]
> **The code this item points at is not in the tree.** `talkback/` and `tts/`
> were removed, and `docs/decisions.md` Decision 69 records that. The research
> and the blueprint below still stand; the claims about what already exists do
> not. Read "the seam is ready" as "here is the seam to build".


- **Status**: Deferred — mitigated, not solved
- **Area**: Native backend (`native/src-tauri/src/talkback/turn.rs`)
- **Original Context**:
  - Talkback plays its answer through the speakers while its microphone is open, so on a laptop without headphones the microphone hears the agent and can interrupt it with its own voice.
  - The current mitigation is an echo guard: while the agent speaks, the speech threshold is multiplied (2.5×), so a barge-in must be clearly louder than the agent. This works well enough at normal volumes and is honest about its limits — the Settings copy recommends headphones.
- **Concept & Implementation Blueprint**:
  - Real AEC needs the playback signal as a reference, which means playback would have to move into Rust — currently blocked by the `rodio`/`cpal 0.17` conflict recorded in `docs/decisions.md` Decision 49.
  - `webrtc-audio-processing` bindings are the usual answer, at the cost of a C++ dependency and a Windows build story.
  - A cheaper intermediate step: have the frontend report playback amplitude back to the engine so the echo guard scales with actual output level rather than using a fixed multiplier.

### 4. Move Relay's Config and Vault Root Off `current_dir()`

- **Status**: Deferred — a migration, not a feature
- **Area**: Native backend (`native/src-tauri/src/lib.rs`)
- **Original Context**:
  - `base_dir` is `std::env::current_dir()/.relay`, and the vault, `settings.json` and the Whisper models directory all hang off it.
  - Beside a development checkout this is fine and convenient. In a packaged Windows app it is not: launched from a Start Menu shortcut `current_dir()` is typically `C:\Windows\System32`; launched from its install directory it is under `Program Files`. Neither is writable by a standard user, and the location changes depending on how Relay was started — so settings can silently fail to save, or save somewhere that is not found next launch.
  - v0.18.1 fixed this for the *voice installation only* (`docs/decisions.md` Decision 52), because that subsystem prints its folder to the user as an instruction and is new state with nothing to migrate.
- **Why it is deferred rather than done**:
  - Changing `base_dir` relocates every existing user's vault, notes, Kanban cards, meetings and settings. Getting that wrong loses data, and getting it right means a detection-and-migration path with its own testing.
  - It is not a Talkback problem, and bundling it into a Talkback change would hide a data migration inside a feature diff.
- **Concept & Implementation Blueprint**:
  - Resolve `base_dir` from the OS application-data directory. `tts::discovery::default_tts_root` used to do exactly this and was removed with the rest of `tts/` (Decision 69); the approach was right and is worth reinstating.
  - On startup, if the new location is empty and a process-relative `.relay` exists beside the executable, offer to move it — explicitly, with the old copy left in place, matching the "never move, migrate or delete" promise the Vault Directory Location setting already makes (`docs/decisions.md` Decision 38).
  - The configurable Vault Directory Location setting already overrides this for the vault, so the migration mainly concerns `config/` — settings, models, and the STT cache.

### 5. Offline and ARM64 Voice Installation

> [!IMPORTANT]
> **The code this item points at is not in the tree.** `talkback/` and `tts/`
> were removed, and `docs/decisions.md` Decision 69 records that. The research
> and the blueprint below still stand; the claims about what already exists do
> not. Read "the seam is ready" as "here is the seam to build".


- **Status**: Deferred — a packaging decision, not missing code
- **Area**: Native backend (`native/src-tauri/src/tts/{manifest,installer}.rs`, `native/src-tauri/resources/voice-manifest.json`)
- **Original Context**:
  - v0.19.0 installs the local voice by downloading it. Two users are therefore still stuck: one on a machine with no internet access, and one on Windows on ARM, for which the manifest carries no runtime and the installer reports "Automatic voice setup isn't available for aarch64 yet" instead of guessing.
  - Both are deliberate. The engine and a voice are roughly 80–100 MB per architecture; bundling them into the installer for every user so that a minority never has to download would triple the download for everyone, and shipping an x86-64 binary to an ARM machine would fail *after* setup claimed success rather than before it started.
- **Concept & Implementation Blueprint**:
  - **ARM64** is the smaller job: add a `piper-windows-aarch64` entry to the manifest once an upstream release asset exists for it. `runtime_for()` already distinguishes an unsupported architecture from an unsupported platform, and nothing else changes — the manifest is the only place a runtime is named.
  - **Offline** is a packaging change, not a code change: `discovery` already looks in Tauri's resource directory (the `Bundled` origin) before falling back to `PATH`, so an offline or enterprise build can ship the engine and the recommended voice as bundled resources and the installer will find them already present and skip the download. What is missing is the build variant and the release pipeline that produces it, not the lookup.
  - A third option, worth considering before either: let the user point setup at a folder they copied from another machine, verified against the same manifest checksums. Cheap, and it covers the air-gapped case without a second build.

### 6. Capturing a Web Page From Relay's Own Hotkey

- **Status**: Deferred — blocked by the browser permission model, not by missing code
- **Area**: Native backend (`native/src-tauri/src/capture/web/bridge.rs`, `hotkeys/mod.rs`), browser extension (`native/src/webcapture/background.ts`)
- **Original Context**:
  - The capture feature was asked for as an OS-level Relay hotkey that reads whatever page the user is looking at. It ships the other way round: the browser owns the trigger, and Relay's `capture_hotkey` opens the Captures surface (`docs/decisions.md` Decision 57).
  - The blocker is `activeTab`. Chrome grants it only in response to a gesture made inside the browser — the extension's action, a context-menu item, a `commands` shortcut, or an omnibox suggestion. A global desktop hotkey is none of those, so an extension asked to capture "from outside" has no permission to read the tab.
- **Why it is deferred rather than done**:
  - The only way to make it work today is `<all_urls>` (or `optional_host_permissions` the user grants once), which trades the entire least-privilege story — the thing that makes this feature safe to install — for the convenience of one shortcut.
  - Even with the permission, the desktop→browser direction is unreliable: an MV3 service worker is terminated when idle, so Relay cannot simply push a request to it. Keeping one alive means holding a port open, which in practice means native messaging.
- **Concept & Implementation Blueprint**:
  - Switch the transport to native messaging (`runtime.connectNative`), which keeps the service worker alive for the life of the port and gives Relay a channel it can *initiate* on. That is the same migration Decision 58 already names as the exit from the loopback bridge, so the two would land together.
  - Add `optional_host_permissions: ["<all_urls>"]` to the manifest and request it from the options page, with the trade stated plainly: "Relay can capture from its own shortcut, but the extension will be able to read any page you visit."
  - Keep the current behaviour as the default and the fallback: if the permission is not granted, the desktop hotkey opens Captures and shows the browser shortcut, exactly as it does now.

### 7. Screenshot and OCR Fallback for Unreadable Pages

- **Status**: Deferred — deliberately not a rung on the capture ladder
- **Area**: Native backend (`native/src-tauri/src/capture/web/`), browser extension
- **Original Context**:
  - The capture ladder ends at `text_only` (the page's visible text) and then refuses. A canvas-rendered app, a scanned document in a viewer, or a page that renders entirely into shadow roots produces nothing, and Relay says so rather than saving an empty artifact.
  - A screenshot plus OCR was considered as a fifth rung and rejected for v1: the product principle is structured acquisition, and the cases it would serve are already labelled honestly rather than silently mis-captured.
- **Concept & Implementation Blueprint**:
  - `chrome.tabs.captureVisibleTab` needs the `activeTab` grant Relay already has, so the browser side is small — but it captures the *viewport*, not the page, which means the artifact would be less complete than the fallback it sits below unless scroll-and-stitch is built too.
  - OCR is the real cost: a Rust OCR engine (Tesseract bindings or an ONNX model) is a large native dependency with a Windows build story, for output that is lower fidelity than every rung above it.
  - If it is built, it must be a *supplementary* artifact attached to a capture — `ScribbleAttachment` already exists — and never a substitute that lets `coverage` claim more than the text extraction earned.

### 8. Firefox Support for the Capture Extension

- **Status**: Deferred — structurally compatible, not validated
- **Area**: `native/browser-extension/manifest.json`, `native/src/webcapture/background.ts`
- **Original Context**:
  - The extension targets Chrome and Edge. Firefox supports `scripting.executeScript` under MV3 from Firefox 101 and grants `activeTab` on the same gestures, so nothing in the extraction layer is Chrome-specific — but the manifest ships Chrome shapes and no Firefox testing was done, so it is not claimed as supported (`docs/capture.md` §13).
- **Concept & Implementation Blueprint**:
  - Add `browser_specific_settings.gecko.id` and, on older Firefox, a `background.scripts` event page instead of `background.service_worker`; the build already emits both an ES module and an IIFE, so no new bundling is required.
  - Firefox treats `host_permissions` as optional by default, so pairing must handle "granted later" — the options page's **Save and test** is the natural place to call `permissions.request`.
  - The bridge already accepts `moz-extension://` origins, and there is a test asserting it.

### 9. Capturing File Bytes and Image Data

- **Status**: Deferred — technically possible, deliberately refused for v2
- **Area**: `native/src/webcapture/dom.ts`, `native/src/webcapture/types.ts`, `native/src-tauri/src/capture/web/`
- **Original Context**:
  - Capture v2 records files and images as metadata plus a reference, with `content_captured: false` and a note saying the file itself was not retrieved. It is not a limitation of the browser: a content script's `fetch` runs with the page's cookies, so an authenticated asset URL (ChatGPT's `backend-api/estuary/content`, a GitHub attachment) *would* resolve, and at least one published exporter does exactly that.
  - Four reasons it was refused rather than postponed by accident, all recorded in `docs/capture/RESEARCH.md` §6.1: the gesture was "capture this page", not "download every file it references"; the payload contract is text-only, which is what lets normalization be a total function over untrusted input, and the 8 MiB body limit would be spent by two screenshots; fetching authenticated asset URLs makes Relay a client of the site's API, a materially larger claim for a least-privilege feature; and metadata-now/bytes-later loses nothing because the reference is preserved, while shipping downloads and retracting them is not available.
- **Concept & Implementation Blueprint**:
  - Gate it on an explicit, off-by-default setting with its own consent copy — "download files referenced by captured pages" — rather than folding it into "enable capture".
  - Do it in the **service worker**, not the content script: the worker already holds the only network permission the extension has, and keeping fetches out of the page context keeps the isolated-world guarantee intact. The content script would hand over a list of references, and the worker would decide what to fetch.
  - Store bytes beside the artifact (`vault/captures/<id>/files/`) rather than in the payload, so the text-only contract survives; the block gains a `stored_path` and `content_captured` becomes a real measurement instead of a constant.
  - Needs its own limits (per-file and per-capture ceilings, an allowlist of types, a same-origin-as-the-page rule) and its own honesty: a file that was fetched and rejected by a limit must say so, exactly as `content_note` does now.
  - `sandbox:/mnt/data/…` references are not fetchable at all and would stay metadata-only regardless.

### 10. A Configurable Traversal Budget

- **Status**: Deferred — the budget is a constant per source, and hitting it is reported
- **Area**: `native/src/webcapture/traversal/budget.ts`, `native/src-tauri/src/settings/`, `native/src/components/settings/CaptureSettingsView.tsx`
- **Original Context**:
  - The reveal pass stops after 10 seconds on a conversation (6 on a document) and reports `time_budget` with `partial` coverage. Measured against the validation fixtures, that covers roughly 450–500 turns (`docs/capture/BENCHMARKS.md`), so a genuinely enormous thread is captured incompletely — honestly, but incompletely — and the user has no way to say "take longer, I'll wait".
  - It was left constant because the extension cannot read Relay's settings without another round-trip to the bridge, and adding one to the capture path for a value that is right in almost every case is a poor trade.
- **Concept & Implementation Blueprint**:
  - The bridge's `/v1/health` response is the natural carrier: the service worker already calls it during pairing, so the budget could ride along and be cached in `chrome.storage.local` rather than fetched per capture.
  - Expose it as a choice rather than a number — "quick", "thorough", "as long as it takes" — mapping to `maxMs`/`maxSteps` pairs, because a millisecond field invites a value that makes capture feel broken.
  - Whatever is chosen, `termination` and the completeness verdict must keep working the same way: a longer budget may not change what a capture is *allowed to claim*, only how much it manages to read.

### 11. Neural Speaker Embeddings and a Voice Library

- **Status**: Backlog (acoustic speaker grouping ships; embeddings do not)
- **Area**: Native backend (`native/src-tauri/src/meetings/voiceprint.rs`,
  `native/src-tauri/src/meetings/speakers.rs`)
- **Original Context**:
  - Vox groups a meeting's turns by speaker using classical features: 26 MFCC
    statistics per turn — mean and standard deviation over 32 ms frames —
    cosine-normalised and clustered with average-linkage agglomeration. The
    grouping is presented as a *proposal*, with a span of the recording per
    speaker so the user can hear each voice and name it.
  - That framing is the point, and it is a deliberate trade rather than a
    placeholder. A neural speaker embedding (x-vector, ECAPA) is materially
    better, and it means shipping an ONNX runtime, a model download with its
    own licence question, and a consent flow — because an embedding stored
    across meetings **is** biometric data, which the classical path never
    creates.
  - What the classical features do not buy is written into
    `voiceprint.rs`'s own module docs: telling two similar voices apart on one
    channel, separating speakers who share a microphone, and matching a voice
    across meetings.
- **Concept & Implementation Blueprint**:
  - Add the ONNX runtime behind the existing `parakeet`-style feature flag and
    a settings toggle that is **off** by default.
  - Replace the vector `voiceprint::voiceprint` returns with the embedding,
    keeping `TurnPrint` unchanged so `voiceprint::cluster` and
    `speakers::assign_speakers` are untouched. Recalibrate
    `DEFAULT_SPLIT_DISTANCE` against the embedding's own scale; the constant's
    doc comment records why the current value under-splits rather than
    over-splits, and the same reasoning applies at any scale.
  - Only then build the voice library: enrolment, a "Remember voices across
    meetings" toggle that must never default on, and management UI — list,
    rename, merge, delete. Nothing currently persists a voice beyond the
    meeting it was heard in, so there is no migration to do first.
  - Per-source audio tracks would help independently. `assign_speakers`
    already clusters the microphone and the loopback separately, but both are
    decoded from one mixed stream; keeping the streams apart on disk would
    remove the local user's voice from the clustering problem entirely.

### 12. Calendar Attendees as a Speaker Hint

- **Status**: Backlog (calendar sync ships; the speaker hint does not)
- **Area**: Native backend (`native/src-tauri/src/calendar/agenda.rs`,
  `native/src-tauri/src/meetings/speakers.rs`)
- **Original Context**:
  - Google Calendar sync ships: several accounts, merged into one day,
    de-duplicated, and matched to recordings by overlap
    (`calendar::agenda::match_recordings`). Each event carries its attendee
    list.
  - Nothing uses that list. `speakers::assign_speakers` finds however many
    groups the audio suggests, capped at `voiceprint::MAX_SPEAKERS`, with no
    reference to how many people were invited — so a three-person call whose
    voices cluster into five groups asks the user to name five.
  - The attendee count is the one piece of outside evidence available about
    how many voices to expect, and it is already sitting on the matched event.
- **Concept & Implementation Blueprint**:
  - Pass an `expected_speakers: Option<usize>` through `DetectionSettings`,
    defaulted from the matched event's non-resource attendee count, with an
    explicit user setting winning over it.
  - Use it as a *stopping hint* rather than a target: keep merging while the
    cluster count exceeds it, and never split to reach it. The calendar says
    who was invited, not who spoke — someone who never unmuted is on the list
    and not in the audio.
  - Do **not** auto-assign attendee names to clusters. Matching a name to a
    voice needs exactly one candidate to fit, and a calendar of five people
    offers five; a wrong name in a report is read as fact. Offer the attendee
    list as the autocomplete behind `SpeakerPanel`'s rename field instead,
    which is the same information with the user's judgement in the loop.
  - Treat the calendar as external source material, per `rules/security.md`:
    an event title or description is data, never an instruction to Vox's AI.

### 13. Auto-Learning Dictionary Words From Corrections

- **Status**: Removed — the setting existed, the mechanism could not
- **Area**: Native backend (`native/src-tauri/src/settings/mod.rs`), Settings › Engine
- **Original Context**:
  - `AudioInputSettings::auto_learn_words` shipped as a persisted setting with a
    rendered toggle, defaulting on, and was read by nothing. Its own copy
    promised: "When you correct a transcription in the target app, the
    corrected word is automatically added to your dictionary."
  - That is not a feature that was left unfinished. It describes observing what
    the user edits **in another application**, which needs accessibility text
    APIs — reading the contents of somebody else's window. Relay does not do
    that anywhere, and the capture surfaces are deliberately built the other
    way round: `hotkeys::injection` can put text into a focused field and has
    no way to read one back.
  - Left in place it was worse than absent: a toggle that is on by default and
    silently does nothing teaches the user that the setting does not matter.
- **What would have to be true first**:
  - A decision to take an accessibility-API dependency (UI Automation on
    Windows, AX on macOS) and the privacy posture to go with it — Relay would
    be reading the user's other applications, which is a different product
    promise from recording their microphone when they press a key.
  - That is the same class of decision as the ONNX runtime in item 1: a
    dependency and a stance, not an afternoon.
- **The cheaper thing that is actually available**:
  - Relay already has a user dictionary and a glossary that primes the
    recognizer, and it now has a rewrite layer (`capture::rewrite`) where the
    user explicitly accepts or rejects a proposed change to their own dictated
    text. A word the user *accepts* there is a correction Relay can observe
    without reading anybody's window. That is a real feature and a different
    one, and it should be designed as itself rather than smuggled in under this
    setting's name.

### 14. A Summary Across a Whole Series

- **Status**: Backlog — the series object it needs now exists; this does not
- **Area**: Native backend (`native/src-tauri/src/meetings/summary/processor.rs`,
  `native/src-tauri/src/meetings/series.rs`)
- **Original Context**:
  - Recurring meetings now have an identity (`meetings::series`): Google's
    `recurringEventId` names the series, membership is stamped onto each
    recording during a sync, and the occurrences can be read in order. That
    answers "which meetings are the standup".
  - It does not answer the question underneath it — *what has changed in this
    standup over six weeks*. Reading six reports in order is the manual
    version, and it is what a person does today.
- **Concept & Implementation Blueprint**:
  - A template that reasons about change over time rather than about one
    conversation: what was decided and then reversed, what has been carried
    forward without moving, who committed to what and whether it landed.
    `summary::templates` is JSON-driven, so the template itself is cheap; the
    prompt behind it is not the same prompt.
  - Map-reduce across transcripts, not a concatenation. Six hour-long meetings
    exceed any local model's context, so the shape is the existing chunked
    processor applied a level up: summarise each occurrence (most already
    have a report), then reduce the reports with the change-over-time
    template.
  - Cost and cancellation are the real design work. A local model summarising
    six meetings is minutes, not seconds, and the existing per-meeting
    progress events (`SummaryProgress`) are per-meeting — a series run needs
    its own progress shape or it looks like a hang.
  - Where it goes: the series panel already lists the occurrences, so the
    report belongs at the top of it, cached against the set of meeting ids it
    was generated from so adding a seventh occurrence visibly invalidates it
    rather than silently showing a stale answer.
- **Not blocked on anything.** It is a feature-sized piece of work, not a
  dependency problem.

---

### 15. Retire the Knowledge Graph's Node-Position Cache

- **Status**: Deferred — superseded, not yet removed
- **Area**: Native frontend (`native/src/components/knowledge/graph/graphStorage.ts` —
  `loadNodePositions`, `saveNodePositions`, `clearNodePositions`, and the
  `GraphPositionMap` type; the "Reset layout" affordance in
  `KnowledgeGraphView`)
- **Original Context**:
  - The force-directed view's layout is seeded from `Math.random` and
    settles differently on every run, so coordinates were persisted to
    localStorage to stop the graph rearranging itself between sessions. The
    cache exists to paper over an irreproducible layout.
  - The Rings view removed that need for itself: `ringsLayout` is seeded,
    runs a fixed iteration count with no convergence exit, and produces
    bit-identical coordinates for the same vault (`ringsLayout.test.ts`).
    Rings therefore reads and writes none of this.
- **Concept & Implementation Blueprint**:
  - Not removed in the same change that introduced Rings, deliberately: the
    force view still depends on the cache, and deleting it while that view
    is the fallback would regress the mode the change promised not to touch.
  - The removal is unblocked once the force view either adopts a seeded
    layout of its own or stops being offered. At that point delete the three
    functions, the `POSITIONS_STORAGE_KEY` entry, the `GraphPositionMap`
    type, and the "Reset layout" confirmation flow that exists only to clear
    it — reproducibility makes a reset button meaningless, since there is
    nothing to reset to.
  - Leave the stored key unread rather than migrating it; a stale
    localStorage entry costs nothing and nothing else reads that key.

---

### 16. Action-Item Extraction for Meetings, Voice Notes and Scribbles

- **Status**: Deferred — the TODOs surface exists, three of its four
  promised feeds do not
- **Area**: Native backend (`native/src-tauri/src/meetings/summary/`,
  `native/src-tauri/src/pipeline/enrichment.rs`,
  `native/src-tauri/src/talkback/`), feeding
  `vault::VaultManager::record_extracted_todos`
- **Original Context**:
  - An audit of every capture path found exactly one structured
    action-item extractor in the codebase: `ContextActionItem` in
    `capture/web/context.rs`, with both an LLM and a deterministic
    cue-scanning implementation. It serves **browser captures** — AI
    conversations, GitHub issues, pull requests, discussions — reachable
    from the Captures surface via `analyze_capture_context`. It is now
    wired through to persisted todos.
  - It is **not** a meetings extractor, despite being the one the TODOs
    brief expected to reuse for meetings. The remaining sources have no
    extraction at all:
    - **Meetings** — "Action Items" is a prose heading in a summary
      template (`SectionStyle::Checklist` in `summary/templates.rs`). The
      model writes checkboxes into Markdown; nothing parses them back out,
      so no meeting has ever produced a structured commitment.
    - **Voice notes** — `pipeline::enrichment` derives title, summary,
      topics, entities, concepts and questions. Action items are not among
      them.
    - **Scribbles** — the same enrichment pass, the same absence.
    - **Talkback** — no action-item concept anywhere in the module.
- **Concept & Implementation Blueprint**:
  - The write side is done and shared: `record_extracted_todos` takes
    `(title, kind, source_ref, para, captured_at)` and is idempotent on
    `(source id, title)`, so re-running an extractor over the same source
    cannot deal a second copy. Each new extractor only has to produce
    candidates.
  - **Meetings** are the highest-value and the most structural work. The
    honest fix is a structured extraction pass beside the summary rather
    than parsing checkboxes back out of generated prose — the report is a
    rendering, and reverse-engineering data from it will break the first
    time the template's wording changes. The pass wants `source_turn_ordinals`
    the way `ContextActionItem` has them, so a todo can open the meeting at
    the moment the commitment was made.
  - **Voice notes and scribbles** share `enrich_scribble`'s single LLM
    call. Adding an `action_items` array to the existing analysis contract
    is cheaper than a second pass, and a deterministic cue scanner (the
    shape `extract_deterministic_context` already uses) must back it, or
    the feature silently stops existing whenever no model is configured —
    which is Vox's zero-cost default.
  - **PARA** comes free for these two: a scribble carries a band, so the
    todo inherits it. Meetings and web captures have no band, so their
    todos arrive uncategorised until the source is filed.
  - Watch the false-positive rate. "I should probably look at that" is not
    a commitment, and a TODOs surface that fills with non-commitments is
    one the user stops opening — which is worse than one with gaps in it.
