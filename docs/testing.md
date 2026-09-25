# Vox — Testing Strategy

What is actually tested today, and how to run it. `rules/testing.md` holds the
conventions (frameworks, placement, what not to test); this file holds the
state of play. CI runs all of it on every push and pull request —
`.github/workflows/ci.yml`.

## 1. Rust backend (`native/src-tauri/`)

665 tests, `cargo test`.

```bash
cd native/src-tauri
cargo test
cargo clippy --all-targets -- -D warnings
```

Building needs a C/C++ toolchain and CMake for whisper.cpp. On Linux it also
needs the GTK/WebKit and ALSA development headers — the CI workflow's `system
dependencies` step is the authoritative package list.

The compiler version is pinned in `native/src-tauri/rust-toolchain.toml` and
installed by rustup on the first `cargo` call. That pin is what makes a local
`clippy -D warnings` mean the same thing as CI's: lints are added between Rust
releases, so a floating `stable` turns "passes locally" into a coin flip.

Where the coverage sits:

- `meetings/` — the largest concentration. `segmenter.rs` asserts the
  behaviours that decide whether a transcript is usable at all: pre-roll so a
  segment starts before onset was detected, a 200 ms breath not splitting a
  sentence while a 1.2 s pause does, a 40-second monologue split at its
  quietest frame with no audio lost across the cut, a click too short to be
  speech discarded, steady room tone never opening a segment, speech over that
  same room tone still doing so, and ragged buffer sizes segmenting
  identically to one block — because device callbacks never deliver whole
  frames. `capture.rs` covers lockstep draining, a silent loopback not
  stalling the recording, and a mix curve with no step in it (the previous one
  dropped from 1.0 to 0.5 across one sample's worth of level). `checkpoint.rs`
  covers the crash path directly: a writer dropped without finalizing, merged
  afterwards, sample-for-sample. `store.rs` covers atomic writes, id traversal
  refusal, recovery recomputing duration from the audio that reached disk, and
  a corrupt meeting not hiding the rest. `summary/` covers prompt construction
  (including that a hostile transcript stays inside the source-boundary
  envelope), chunking that loses no line and terminates with a pathological
  overlap, and the cache fingerprint — that a changed transcript, template,
  model or context window invalidates it, and that a changed *output language*
  does not. `transcription.rs` pins the shutdown deadlock: dropping the queue
  must end the worker.
- `settings/` — schema defaults, serde aliases, and backward compatibility with
  settings files written by earlier versions.
- `vault/` — frontmatter parsing, note and scribble CRUD, merge behaviour.
- `capture/evaluation.rs` — the STT benchmark matrix and VAD behaviour.
- `capture/web/` — the web capture path, and the second-largest concentration
  after meetings: URL-based source detection, sanitization (control and
  bidirectional characters, non-`http(s)` link targets, `mermaid` fence
  downgrade, fence escaping), coverage downgrade when caps are hit, refusal of
  an empty capture, the loopback bridge's token comparison / origin refusal /
  size limits over a real socket, and an end-to-end set that drives a payload
  into the vault and checks re-capture versioning, Files isolation, promotion
  provenance, a Trash round-trip and path-traversal refusal.
- `sync.rs` — mutex poison recovery, including a test that pins the failure
  mode it exists to prevent.

No test calls a live Ollama instance or a cloud API. Meeting summarization is
covered at the prompt-construction and cache-fingerprint level — both pure
functions — rather than by mocking a provider, so what is asserted is the text
Vox actually sends.

What is **not** covered: anything requiring a real audio device or a loaded
Whisper model. `DualCapture::start`, the decode inside the transcription
worker, and a full record-to-report round trip are exercised by hand on
Windows, not in CI.

## 1b. The speech benchmark (`meetings::benchmark`)

Tests tell you the pipeline still behaves. They cannot tell you whether it
*hears* well, and no assertion can: transcription quality is a measurement
against recordings, not a boolean. That measurement lives in
`native/src-tauri/src/meetings/benchmark/`.

It is not the same thing as `capture::evaluation`, and the difference matters.
`capture::evaluation` scores the **dictation** path — one clip, the
whole-buffer VAD, one decode. The benchmark scores the **meeting** path, by
driving the production `Segmenter`, `speech_health`, `text_normalize` and a
bounded queue of the same depth as the live one. Segmentation errors, backlog,
dropped speech and repetition across a forced split only exist in that
machinery, so only that machinery can measure them.

### The corpus

Twelve conditions, each with a minimum duration. A case shorter than its
category's floor runs and is reported as `UnderLength` rather than scored:
short clips produce word error rates dominated by noise, and cannot express
drift, backlog or repetition at all (`docs/speech-decision-log.md` D-015).

| Category | Floor |
|---|---|
| `long_meeting` | 30 min |
| `long_form_speech` | 5 min |
| everything else — clean/Indian English, Hinglish, technical vocabulary, proper nouns, numbers and dates, two speaker, overlapping speech, noisy, system-audio call | 3 min |

**No audio ships with Vox.** Recordings of real meetings are the user's, and a
synthetic corpus would measure a text-to-speech engine rather than a room. The
manifest names what to supply:

```jsonc
{
  "version": 1,
  "cases": [{
    "id": "standup-hinglish",
    "audio": "audio/standup.wav",          // relative to the manifest, may not escape it
    "category": "hinglish",
    "reference_file": "reference/standup.txt",
    "expect": {
      "proper_nouns": ["Payal", "Bengaluru"],
      "numbers": ["fourteen", "2026-03-04"],
      "technical_terms": ["Tauri", "Supabase"]
    }
  }]
}
```

`speech_benchmark_template` returns a starter manifest covering all twelve.

### Running it

Through the IPC surface, because the engines it compares need the app's
resolved model paths and settings:

| Command | Does |
|---|---|
| `speech_benchmark_template` | starter manifest, one case per category |
| `inspect_speech_benchmark` | validates a manifest and says which categories have no case |
| `run_speech_benchmark` | runs a corpus against one engine and writes the report |
| `cancel_speech_benchmark` | stops part-way; finished cases are still reported |

`run_speech_benchmark` takes an `engine` (`whisper` with a model path, or
`parakeet` with a model directory — a `RecognizerChoice`, which deliberately
has no variant that can hold a credential) and a `pacing`:

- **`batch`** — as fast as the decoder manages. Measures accuracy and real-time
  factor. Queue depth and drops are meaningless here, because nothing is racing.
- **`realtime`** — paced to wall clock, the way a recording actually arrives.
  The only mode in which backlog, dropped speech and time-to-first-transcript
  mean anything, and it takes as long as the audio does.

### What comes out

One `benchmark_run.json` per case per engine, plus `report.json` and a
`report.md` summary, written to `results/<timestamp>/` beside the manifest.
One file per run because a run is the unit that gets compared — diffing two
engines is diffing two directories.

Each run carries accuracy (WER, CER, and substitution/deletion/insertion as
rates against the reference length), repetition rate, hallucination rate broken
down by the reason `speech_health` gave, recall of the proper nouns, numbers
and technical terms the case expected, coverage, and the latency breakdown:
decode RTF, pipeline RTF, time to first transcript, p50/p95/max finalization,
and drain.

Three honesty constraints are built in rather than left to the reader:

- **No invented confidence.** An engine that reports no per-decode no-speech
  probability records `None`, and is not screened on a number it never
  produced. Parakeet is such an engine, and its declared capabilities travel
  into the report with it, so a comparison states what one side could not do
  rather than leaving a reader to infer it from a column of zeroes. See
  `docs/speech-providers.md`.
- **`pipeline_rtf` is the number that matters**, not `decode_rtf`. Audio
  arrives at wall-clock rate, so above 1.0 the backlog grows by
  `L × (rtf − 1)` over a meeting of length `L` and takes that long to clear
  after stop. Below 1.0 the decoder keeps up and nothing accumulates.
- **Repetition rate counts repetition, not error.** A speaker who says "no,
  no, no" raises it. It is comparable between two runs over the same audio and
  is not a defect count.

The report names the categories the corpus has no case for, so an average over
four easy conditions cannot be mistaken for coverage.

### What the benchmark's own tests cover

25 tests, run by `cargo test benchmark`, against a scripted decoder and
synthetic audio — so every metric is checked against a known answer rather
than against whatever a model said that day. They cover segmentation into
spans, screening a looping decode as a hallucination, a failed decode, the
two coverage measures, repetition, term recall, error rates, manifest
validation including path traversal and schema version, report rendering and
JSON round-trip. One test asserts the harness's queue depth equals
`transcription::MAX_QUEUED_SEGMENTS`, so the benchmark cannot drift into
measuring a pipeline that does not ship.

## 1c. Long-meeting endurance (`meetings::endurance`)

Every reliability property the meeting pipeline claims is a property over
*time*: the queue stays bounded, the transcript stays ordered, memory does not
grow, audio survives, nothing is lost quietly. None of them can fail in a
five-minute demo and all of them can fail in an hour, so a fixture drives the
production `Segmenter`, `CheckpointWriter` and `MeetingStore::append_segments`
over a synthetic recording of arbitrary length. The audio is generated, so two
hours costs seconds of CPU and no fixture file.

It leaves out the decoder on purpose. A real decode needs a model this
repository does not ship, and its speed is what `meetings::benchmark` already
measures properly. What remains is everything *around* the decode, which is
where the long-duration failures live.

```bash
cd native/src-tauri
cargo test endurance                 # the fast ones, in CI
cargo test -- --ignored endurance    # the two-hour run, before a release
```

What it asserts: a 15-minute meeting keeps every promise (audio merged,
checkpointed as it went, every span queued, nothing dropped, transcript
ordered); a decoder that cannot keep up sheds visibly and never exceeds the
queue bound while the recording stays complete; an hour holds no more memory
than five minutes; checkpoint memory is bounded by the interval rather than by
the meeting; and pause/resume neither reorders nor loses transcript.

### The one thing it found

Persisting a transcript line costs more as the transcript grows.
`append_segments` rewrites the whole file, so each append pays for everything
already written. Over a synthetic two-hour meeting (1,439 lines) the first
hundred lines cost **40 ms** to persist and the last hundred cost **2,056 ms**.

Attributed: a **persistence** bottleneck, not capture, segmentation, queue,
model or UI.

Caching the parsed transcript in the store took the last hundred to
**1,458 ms** — a 29% win, and the shape of that win says the remaining cost is
serializing and writing rather than parsing.

It was not taken further, and that is a decision rather than an omission. At
1,439 lines this is roughly 15 ms per segment on the decode thread, against a
decode measured in hundreds of milliseconds — a few percent. Removing the rest
means not rewriting the file per line, which is a vault format change, and
`docs/meetings.md` records the ceiling at which it becomes worth making. A
test pins the growth's shape so a change that makes it worse is visible.

## 2. Native frontend (`native/src/`)

370 tests, Vitest + React Testing Library, jsdom.

```bash
cd native
npm ci
npm test              # vitest run
npm run test:watch
npm run test:coverage
npm run typecheck     # tsc --noEmit
```

Tauri modules (`@tauri-apps/api/core`, `/event`, `/window`) are stubbed
globally in `src/test/setup.ts`, because they only resolve inside a Tauri
webview. A test overrides one with
`vi.mocked(invoke).mockImplementation(...)`.

Where the coverage sits:

- `lib/meetings.ts` — the formatting the surface depends on: a clock that
  grows a leading hour only when there is one and never renders negative, a
  duration that shows a dash rather than "0 sec", and a transcript rendering
  that names the speaker when the channel changes and skips blank turns.
- `components/meetings/MeetingsPage.tsx` — behaviour driven entirely through
  `invoke` responses: the list, both level meters and the live controls
  appearing while recording, the microphone-only warning, opening a meeting
  onto its transcript with the speaker label, a report offered when there is
  none, filtering without a second backend call, an empty vault saying so, and
  a failed start being reported rather than swallowed.
- `knowledge/graph/graphPhysics.ts` — layout invariants: pinned nodes never
  drift, alpha always decays to zero, coincident nodes separate rather than
  producing `NaN`.
- `knowledge/KnowledgeGraphPage.tsx` — that the surface reads the graph itself
  rather than depending on Scribbles, that it summarises what the canvas
  toolbar does not, and that an empty or unreadable graph says so instead of
  drawing a blank canvas.
- `home/homeStats.ts` — every figure Home shows: week deltas that refuse to
  count an unparseable date, transcribed-word and recording-time sums, the
  promotion and enrichment backlogs, case-insensitive topic distinctness, the
  cross-surface activity merge (an unreadable timestamp sorts last rather than
  vanishing), and each formatter's degenerate input.
- `home/HomePage.tsx` — the wiring rather than the layout: that a capture card
  opens the mode it names instead of capturing on the landing page, that a
  counter navigates to its own surface, that the dictation hotkey shown is the
  configured one, that an unconfigured capability offers the control that fixes
  it, and that one failing vault read degrades that surface without hiding the
  others.
- `lib/soundEffects.ts` — the only code path behind the `dictation_sounds`
  setting, over a stubbed Web Audio API.
- `webcapture/` — the browser extension's extraction layer, tested against
  fixture documents in jsdom: block extraction (headings, lists, code, tables,
  quotes, images), hidden content, malformed markup, deep nesting, duplicate
  text, unicode, link resolution, `<head>` metadata, every coverage verdict,
  and each site extractor's role attribution, turn ordering, fallback
  strategies and refusal to guess.
- `components/captures/` — completeness wording (a capture may only claim the
  whole page when coverage says so), filtering, the bridge-off warning, and
  deletion behind a confirmation.

### The cross-language contract

`webcapture/contract.test.ts` generates the payload fixtures under
`native/src-tauri/src/capture/web/fixtures/` from committed HTML, and
`capture::web::contract_tests` consumes the same bytes. A field renamed on
either side of the extension↔backend boundary fails one of the two suites
immediately rather than failing quietly on a user's next capture. Regenerate
with:

```bash
cd native && VOX_UPDATE_CAPTURE_FIXTURES=1 npm test
```



## Known gaps

- `cargo fmt --check` is not a CI gate. The crate predates any formatting pass
  and currently differs from rustfmt in 45 files; running `cargo fmt` once, as
  its own commit, is what unblocks adding it.
- Several large native components have no tests: `ProviderSettings.tsx` (1,874
  lines), `ScribbleDetailEditor.tsx`, `DictationPill.tsx`. The pill is the
  highest-value of these, since it owns the capture state machine.
- No end-to-end test drives a real recording through capture, STT, and
  processing. The eval fixtures cover the processing half of that.
- **`corpus-v1`'s audio is broken, and every accuracy number from it is void.**
  All eight cases are 89–99% clipped: the generator read `miniaudio`'s signed
  16-bit samples as if they were floats, so every non-zero sample was clamped
  to full scale. The corpus is the sign of the waveform. `docs/asr-shootout.md`
  §6 has the detail; the generator is fixed and now refuses to write a case
  that fails the gate, and regenerating needs network access to the TTS
  endpoint. Until it is regenerated, `TRANSCRIPTION_BENCHMARK_V1.md` carries a
  banner saying its numbers measure the recording.
- **Even once regenerated, the corpus is synthesized, not recorded.**
  Text-to-speech produces clean articulation, no overlap, no room and no accent
  variation, so a word error rate from it is a floor rather than an estimate.
  Treat a *change* in the number as evidence and the number itself as
  provisional until someone supplies recordings.
- **Two benchmark harnesses exist, measuring different things.**
  `meetings::benchmark` (§1b) runs a corpus through the production pieces
  behind Tauri commands, declares each engine's capabilities, and is the only
  one that measures first-transcript latency and queue backlog under real-time
  pacing. `src/bin/benchmark.rs` plus `tests/transcription/runner/shootout.py`
  compares engine *configurations* over one corpus through one evaluator
  (`docs/asr-shootout.md`). Neither is redundant and neither knows about the
  other; the Parakeet adapter and first-token latency are the two places where
  the shootout wants what the Rust harness already has.
- **Only one of the eight corpus cases has been run.** The committed report
  says so on its own first page, which is the fix that was available here; the
  Hindi and Hinglish rows are the ones that would actually test the
  code-switching this pipeline is built for.
- **No browser-level test drives web capture in a real browser.** The
  extraction layer is covered by jsdom fixtures and the wire format by the
  contract tests above, but a fixture cannot tell you that ChatGPT changed its
  markup last week. `docs/capture.md` §11 holds the reproducible manual
  procedure, and it is the only thing that validates the site extractors
  against live pages.
- The extension bundle build (`npm run build:extension`) is not a CI step. Its
  sources live under `native/src/webcapture/`, so they are typechecked and
  unit-tested with the rest of the frontend; what is unverified in CI is the
  bundling itself.
