# Vox — Data Model Specification

Paths below are relative to two roots, both resolved at startup
(`docs/architecture.md`, Data Access): `<vault>` is `<base>/vault` unless the
user chose a Vault Directory Location, and `<config>` is `<base>/config`.
`<base>` is `VOX_HOME` if set, an existing `.vox` (or legacy `.relay`) folder
if one is found, and otherwise — for a fresh release install — the per-user
application-data folder (`%APPDATA%\com.vox.app` on Windows).

## 1. Vault Markdown Note Schema
Voice notes — and the notes the scribble pipeline writes — are stored in
`<vault>/notes/<id>.md`. The frontmatter is written by hand
(`vault::VaultManager::save_note`), and its optional fields use Rust's `Debug`
form — `None`, `Some("…")` — rather than YAML's `null`; the reader parses that
form back:

```markdown
---
id: "note_3f2b6c1e-9a0d-4c55-8d2e-6b1f0a7c9e21"
title: "Remind me to send the numbers to Priya before Friday."
type: "voice_note"
created_at: "2026-09-25T09:14:03.112000000+00:00"
updated_at: "2026-09-25T09:14:03.112000000+00:00"
tags: []
source_audio: None
raw_content: Some("um remind me to send the numbers to Priya before Friday")
cleanup_style: Some("faithful")
merged_from: None
---

Remind me to send the numbers to Priya before Friday.
```

`type` is `voice_note` for every dictation, from the hotkey or the pill.
Notes of type `scribble` were written by a retired capture mode and still
read. Scribbles in
the knowledge layer are separate files, `<vault>/scribbles/<id>.md`, with
their own frontmatter (`vault::scribble`).

`raw_content` and `cleanup_style` are set only when an opt-in cleanup style
changed the text: `raw_content` holds it as it was before the rewrite (after
the deterministic pass), and `cleanup_style` names the style. With the default,
`raw`, both are `None` and `content` is the transcript as the deterministic
pass left it. `merged_from` lists the notes a merge combined; their originals
are kept in `<vault>/merged_sources/` so the merge can be undone.
`source_audio` is part of the schema, but no current writer sets it.

## 2. Kanban Card Schema (the TODOs surface)
Todos are Kanban cards, stored as `<vault>/kanban/<id>.md`
(`vault::kanban::KanbanCard::format_markdown`). A todo spoken on the TODOs
page:

```markdown
---
id: "card_8c1d0e2f-4b7a-4f3e-9d6c-2a5b7e9f1c30"
title: "Send the Q3 numbers to Priya before Friday."
assignee: ""
status: "todo"
priority: "medium"
due_date: ""
created_at: "2026-09-25T09:14:03.112000000+00:00"
source_note_id: "note_5a7c2e90-1d3b-4f6a-8e2c-9b0d4f1a6e73"
source_kind: "voice_note"
captured_at: "2026-09-25T09:14:03.112000000+00:00"
source_ref: {"id":"note_5a7c2e90-1d3b-4f6a-8e2c-9b0d4f1a6e73","label":"Spoken on the TODOs page"}
---

Send the Q3 numbers to Priya before Friday.
```

`status` is `todo | in_progress | done`. `source_kind` is `manual`,
`voice_note`, `meeting`, `scribble`, `web_capture` or `talkback`; `source_ref`
points at the originating object, with a `turn_ordinal` where the source has
turns. `source_kind`, `para`, `captured_at` and `source_ref` are written only
when set, so a card from an older version still reads. The body is the card's
description; a spoken todo keeps the full transcript there, while its title is
cut to 120 characters.

## 3. Trigger Phrase Configuration (removed)
Nothing reads or writes `triggers.json` any more: the trigger engine, its
settings tab and its commands were removed (`docs/decisions.md` Decision 75).
A file left in `<config>` by an earlier version is ignored.

## 4. App Settings Configuration Schema (`settings.json`)
Stored at `<config>/settings.json`. The Rust `AppSettings` struct
(`native/src-tauri/src/settings/mod.rs`) is the authority: every section
defaults when absent, unknown keys are ignored (and dropped on the next save),
and a corrupt file falls back to defaults rather than stopping the app. The
sections dictation reads, at their defaults:

```json
{
  "provider": {
    "active_provider": "ollama",
    "ollama_host": "http://localhost:11434",
    "ollama_model": "llama3.2:latest",
    "cloud_api_key": null,
    "cloud_model": "gpt-4o-mini",
    "context_tokens": 8192,
    "provider_keys": {},
    "custom_openai_endpoint": null,
    "custom_openai_model": null
  },
  "stt": {
    "whisper_model_path": null,
    "dictation_quality": "fast",
    "dictation_threads": null,
    "enable_initial_prompt": false,
    "custom_initial_prompt": null,
    "preset": "",
    "cleanup_style": "",
    "meeting_model_id": null,
    "dictation_engine": null,
    "meeting_trim_audio_context": true
  },
  "hotkeys": {
    "show_hide_hotkey": "Ctrl+Shift+Space",
    "dictation_hotkey": "Ctrl+Space",
    "toggle_to_talk": false,
    "capture_hotkey": "Ctrl+Shift+C"
  },
  "clipboard": {
    "auto_paste": true,
    "copy_to_clipboard": true,
    "injection_method": "clipboard_paste"
  }
}
```

- `active_provider` is one of `"ollama" | "cloud_openai" | "cloud_gemini" |
  "cloud_anthropic" | "groq" | "openrouter" | "custom_openai"`.
- **API keys are not kept here.** `provider_keys` (one per provider) and the
  legacy single `cloud_api_key` are written to the OS credential store —
  service `com.vox.app.providers`, account `provider_keys` — and appear in the
  file empty. Only on a machine with no credential store do they stay in the
  file. `get_settings` returns each stored key as the placeholder
  `__vox_stored_key__`, and `save_settings` reads that placeholder, sent back
  unchanged, as "keep the stored key" (`docs/decisions.md` Decision 73).
- `stt.cleanup_style`: `""` — the default — and any unrecognised value mean
  `raw`, and no model runs. `faithful`, `clean`, `polished` (or
  `professional`) and `concise` opt into the rewrite, which may hold the text
  for at most five seconds (Decision 71). There is no `text_transform` and no
  `dictation_streaming_shadow`; either key in an older file is ignored.
- `stt.whisper_model_path` overrides the dictation model. Unset,
  `dictation_quality` picks `ggml-base.bin` (`fast`, falling back to an
  installed `ggml-small.bin`) or `ggml-small.bin` (`accurate`) from
  `<config>/models/`, downloading it on first use. `dictation_engine:
  "parakeet"` switches dictation to Parakeet once its model is installed.
  Meetings use `meeting_model_id`, or whatever model is installed.
- `stt.enable_initial_prompt` governs `custom_initial_prompt` only; the
  `dictionary` words always prime Whisper.
- `clipboard.injection_method` is `clipboard_paste` (clipboard plus Ctrl+V,
  the default) or `keystrokes` (typed through `enigo`).

The remaining top-level sections: `ui` (pill position), `vault` (`directory`:
the chosen Vault Directory Location, `null` until one is chosen), `language`
(dictation languages, note language, output script), `diagnostics` (anonymous
diagnostics consent — off by default — and first-run state), `cloud` (Supabase
URL and anon key), `sound` (`dictation_sounds`), `capture` (§8), `startup`
(launch at login, start minimized), `audio_input` (microphone and keep-warm),
`meetings` (camelCase keys: system audio, template, languages, devices,
reminders, the meeting pill), `dictionary`, `snippets` (a spoken phrase
expanded into text) and `vocabulary_corrections` (§9). There is no `tts`
section.

## 5. LanceDB Vector Record Schema (planned — not built)
Decision 6 commits to embedded vector search, and nothing in the tree
implements it yet: no embeddings are generated, no LanceDB database is
created, and retrieval ranks by keyword overlap (`docs/roadmap.md`). The table
below is the planned shape, `note_embeddings`, not a description of anything on
disk:

| Field | Type | Description |
|---|---|---|
| `id` | String (PK) | Note or card ID |
| `vector` | FixedSizeList<Float32, 384> | Dense embedding vector |
| `path` | String | File path in markdown vault |
| `content_snippet` | String | First 500 characters of content |
| `created_at` | String | ISO timestamp |

## 6. Web Capture Schema

A capture is a `VaultFile` (`native/src-tauri/src/vault/file.rs`) with the
optional `capture` field populated. Captures live in their own directory so
the Files surface — which lists `<vault>/files/` only — is unaffected:

```text
<vault>/captures/<capture_id>/
  metadata.json                     # the VaultFile record
  original/<Sanitized-Title>.json   # the raw structured payload, written once
```

`file_type` is `"webcapture"`, `mime_type` is `"application/json"`,
`content` is the normalized markdown, `vault_path` points at the raw payload,
and `last_known_source_path` is the captured URL. `content_hash` is a SHA-256
over the *captured content* (title, url, structured content) rather than the
rendered markdown, which carries the capture timestamp and would otherwise
make every re-capture look like a change.

`capture` carries provenance only. Semantic fields (`summary`, `tags`,
`topics`, `entities`, `ai_metadata`) are produced later by analysis and are
deliberately kept out of it:

```json
{
  "source_type": "web",
  "capture_type": "conversation",
  "application": "ChatGPT",
  "domain": "chatgpt.com",
  "url": "https://chatgpt.com/c/2b1f0e3a",
  "page_title": "Designing Vox Capture",
  "captured_at": "2026-02-14T09:30:12.884Z",
  "browser_captured_at": "2026-02-14T09:30:12.104Z",
  "browser": "Mozilla/5.0 ...",
  "extractor_id": "chatgpt",
  "extractor_version": 1,
  "fidelity": "structured",
  "coverage": "rendered_dom",
  "notes": ["Only the turns the page had rendered were captured. ..."],
  "message_count": 24,
  "block_count": 58,
  "skipped_block_count": 0,
  "truncated": false,
  "canonical_url": null,
  "author": null,
  "published_at": null,
  "language": "en",
  "version": 1,
  "previous_capture_id": null,
  "recapture_count": 0
}
```

`capture_type` is one of `conversation | article | repository | issue |
pull_request | discussion | code | page`; `fidelity` is `structured |
generic | text_only`; `coverage` is `full_document | rendered_dom | partial |
failed | unknown`. See `docs/capture.md` §7 for what each coverage value is
allowed to claim.

The raw payload under `original/` is the wire format shared with the browser
extension (`native/src/webcapture/types.ts` ↔
`native/src-tauri/src/capture/web/mod.rs`). It is text-only by construction —
no field in it can carry HTML — and is never rewritten, which is what lets
`renormalize_capture` rebuild the markdown from it after normalization
improves.

## 7. Derived Data Schema

Analysis output is stored **beside** a source artifact, never inside it. The
source record (`metadata.json`) is what Vox captured; derived data is what
Vox concluded, and re-analysing must never be able to rewrite the former.

```text
<vault>/captures/<source_id>/        (or <vault>/files/<source_id>/)
  metadata.json                        # the VaultFile record — source truth
  original/<Sanitized-Title>.json      # raw payload, written once
  context.json                         # SourceContext, the read path for the UI
  derived/
    summary.json                       # DerivedData { derived_type: "summary" }
    context.json                        # DerivedData { derived_type: "context" }
    enrichment.json                    # DerivedData { derived_type: "enrichment" }
```

Every derived record names the source it came from and how it was produced
(`native/src-tauri/src/pipeline/analysis/derived.rs`):

```json
{
  "id": "cap_2b1f::context",
  "source_id": "cap_2b1f",
  "derived_type": "context",
  "version": 2,
  "created_at": "2026-09-03T10:04:11Z",
  "updated_at": "2026-09-04T08:15:00Z",
  "analysis": {
    "analysis_type": "context",
    "status": "succeeded",
    "prompt_id": "repository.context",
    "prompt_version": 1,
    "provider": "ollama",
    "model": "llama3.2:latest",
    "deterministic": false,
    "source_coverage": "partial",
    "generated_at": "2026-09-04T08:15:00Z",
    "prompt_tokens": 4210,
    "completion_tokens": 812
  },
  "payload_kind": "structured",
  "payload": { "...": "the SourceContext, or summary text for a summary" }
}
```

`status` is the field that carries Vox's trust model, and its three
meaningful values are not interchangeable:

| Status | Meaning |
|---|---|
| `succeeded` | A model answered and the answer validated against the contract. |
| `insufficient_evidence` | The analysis ran correctly and the source did not carry what was asked for. A successful, honest outcome — including when a deterministic fallback produced the payload. |
| `failed` | No usable answer: no model responded, or the response could not be validated. |

`deterministic: true` means no model read the source and a pattern-matching
fallback produced the payload. Such a record never names a `model`, because
none answered.

`prompt_id` and `prompt_version` come from the registry in
`pipeline/analysis/prompts.rs`. They are recorded so that derived data
produced under an older prompt cannot silently claim to be the output of a
newer one.

### Regeneration policy

Re-analysis **replaces** the record for a given `(source_id, derived_type)`
pair and increments its `version`. Vox keeps the latest derived
representation, not a history — the same behaviour `context.json` always had,
now stated and applied consistently. The source is never modified, so any
analysis can be run again.

### Backward compatibility

`context.json` remains the read path for the Captures UI and is written for
every capture, including vaults that predate this layout. A `context.json` in
the pre-v0.28.5 shape (a bare `ConversationContext` with no `kind` envelope)
still deserializes, and gets `analysis: null` — no metadata is invented for a
context produced before the contract existed. The semantic fields on
`VaultFile` (`summary`, `tags`, `topics`, `entities`, `ai_metadata`) are still
written and read; `derived/` is populated alongside them rather than instead
of them.

## 8. Capture Settings

Part of `settings.json` (§4), under `capture`:

```json
{
  "capture": {
    "bridge_enabled": false,
    "bridge_port": 8765,
    "pairing_token": null,
    "analyze_on_capture": true
  }
}
```

Off by default: capture needs a paired browser extension before it can do
anything, so a fresh install opens no listening socket. `pairing_token` is a
64-character hex secret — two random v4 UUIDs, 244 random bits — generated
when capture is first enabled. `hotkeys` also
gains `capture_hotkey` (default `Ctrl+Shift+C`), which opens the Captures
surface — reading a page is triggered from inside the browser, for the reason
given in `docs/capture.md` §1.

## 9. Voice Note Corrections

Two separate records, because they answer different questions.

### Correction history (`<vault>/corrections/<note_id>.json`)

Every phrase correction applied to one Voice Note, oldest first. The same
sidecar shape as `merged_sources/<note_id>.json`, and for the same reason: a
note's undoable history belongs beside the vault rather than inside the note's
own Markdown, where it would be text the user has to look at.

```json
[
  {
    "id": "corr_7f3c…",
    "note_id": "note_123456789",
    "original": "super base",
    "replacement": "Supabase",
    "start": 14,
    "corrected_at": "2026-09-09T17:24:00Z",
    "learned": true
  }
]
```

`start` is a **character** offset into the note content at the time of the
correction — not bytes, and not UTF-16 code units. It is what makes undo a
reversal of the range (put `original` back where `replacement` now sits) rather
than a stored copy of the note: no versioning system, and a full-editor change
made in between is refused instead of being thrown away. The file is deleted
when the stack empties.

`learned` records whether the user ticked "Teach Vox this correction". It is
not recoverable from the note afterwards, and it is the difference between an
ordinary edit and a standing rule.

### Learned corrections (`settings.json` › `vocabulary_corrections`)

```json
{
  "dictionary": ["Vox", "Whisper", "Supabase"],
  "vocabulary_corrections": [
    {
      "source": "super base",
      "replacement": "Supabase",
      "enabled": true,
      "created_at": "2026-09-09T17:24:00Z"
    }
  ]
}
```

Distinct from `dictionary`, and deliberately not merged into it. `dictionary`
is canonical words used to prime the recognizer *before* it guesses (via
`build_stt_prompt`) and to correct near-misses by edit distance. A correction
repairs a form the recognizer keeps producing *after* the fact, applied in
`capture::text_normalize` ahead of the glossary so the user's specific rule
wins over a fuzzy token match. Putting `"super base"` in `dictionary` would
prime Vox to produce the very phrase being corrected.

Empty by default: every entry was put there by the user ticking the box on a
correction made inside Vox. Nothing is learned by watching what they type in
other applications — see `maybe_later.md` §13.

`enabled: false` keeps an entry visible and stops it being applied, so a
correction that turns out to be wrong can be silenced without losing the record
of having made it. Both are managed in Settings › Dictionary.

---

## 10. Meeting Schema (`<vault>/meetings/<meeting_id>/`)

One directory per meeting. The directory *is* the meeting: deleting it deletes
the transcript, the report and the audio together.

```text
<vault>/meetings/meeting-<uuid>/
├── meeting.json          metadata
├── transcript.json       Vec<TranscriptSegment>, ordered by `sequence`
├── transcript.live.json  the live pass's transcript, kept once a final pass replaces it
├── speakers.json         the voices speaker detection proposed, and the names given them
├── attribution.json      which transcript line belongs to which of them
├── summary.json          the report, its fingerprint, and the English original
├── notes.md              free text the user typed
├── diagnostics.jsonl     one line per decoded segment, appended as it goes
├── diagnostics.json      the run's rollup, written once on stop
└── audio/
    ├── chunk_000000.wav   durable 30 s checkpoints, present only while recording
    └── audio.wav          the merged recording, 16 kHz mono 16-bit PCM
```

Every file above is written to a temporary path in the same directory and
renamed into place, so an interrupted write never leaves a half-file where a
complete one was.

### `meeting.json`

| Field | Type | Notes |
|---|---|---|
| `id` | string | `meeting-<uuid simple>` |
| `title` | string | `Meeting YYYY-MM-DD HH:MM` until a report renames it — and only while it still matches that shape |
| `state` | enum | `recording` / `paused` / `transcribing` / `completed` / `failed` |
| `source` | enum | `recorded` / `imported` |
| `duration_seconds` | number | Measured from the audio that reached disk, not from a wall clock |
| `created_at` / `updated_at` | string | RFC 3339 |
| `audio_path` | string? | Absolute path to `audio.wav` |
| `transcript` | object? | What produced the transcript on disk: `pass` (`live` / `final`), `engine`, `model` (a filename, never a path), `language`, `profile`, `completed_at`. `None` while recording and for a meeting transcribed before this was recorded; it replaced an earlier `transcript_model` field |
| `language` | string? | |
| `mic_device` | string? | |
| `system_audio_captured` | bool | `false` means only the microphone was recorded, which changes how the transcript should be read |
| `segment_count` | number | |
| `dropped_segments` | number | Segments queued but never decoded. Surfaced, not swallowed |
| `error` | string? | Set by crash recovery and by a failed import |
| `tags` | string[] | |
| `series_id` | string? | The recurring meeting this recording belongs to, written during a calendar sync; series records live in `<vault>/meetings/series.json` |

### `transcript.json`

Each segment: `sequence` (monotonic, assigned before decoding so a slow span
cannot reorder the transcript), `text`, `start_seconds` / `end_seconds`
(recording-relative, so a transcript still lines up with its audio after a
pause), `channel` (`microphone` / `system` / `mixed` — see Decision 68),
`no_speech_prob` (Whisper's own, not a derived number), and `recorded_at`.
Optional, and omitted when unset: `cut_at_ceiling` (the segmenter cut this span
at its ceiling, so the next line continues the sentence), `original_text`,
`romanized_text` and `translated_text` (the script and language variants),
`corrections` (what the glossary changed, so the decoder's own words can be
reconstructed), and `telemetry` (how a live decode went).

### `summary.json`

`status`, `template_id`, `markdown` (as the user should see it),
`english_markdown` (the original, kept even when `markdown` is a translation),
`previous_markdown` (held only during a regeneration, restored on failure),
`fingerprint` (hash of transcript + template content + instructions + model +
context window), `provider`, `model`, `language`, `chunk_count`,
`processing_ms`.

### Templates (`<config>/meeting-templates/<id>.json`)

`{ id, name, description, sections: [{ heading, instruction, style, required }] }`
where `style` is `bullets` / `checklist` / `prose`. An `id` matching a bundled
template replaces it; any other adds one. Ids are validated before they touch
the filesystem.
