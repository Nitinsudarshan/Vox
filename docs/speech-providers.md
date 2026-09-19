# Vox — Speech recognition providers

What each engine can do, what it cannot, and what nobody has established.
Written against **v0.1.0**. The capability columns are generated from
`capture::recognizer::RecognizerKind::capabilities`, so if this table and the
code disagree, the code is right and this file is a bug.

## The shape of the question

Not "what is the best speech model for Vox". That question has no stable
answer — the models change every few months — and answering it once bakes the
answer into the architecture.

The question is: **what does this task need, and what is the smallest
replaceable thing that provides it?**

```text
SPEECH TASK  ──▶  REQUIRED CAPABILITY  ──▶  PROVIDER  ──▶  MODEL
```

`capture::recognizer::SpeechTask` holds the first two columns as code:

| Task | Must have | Why |
|---|---|---|
| `PushToTalk` | batch, offline | One short utterance with someone waiting on it. Offline because this is the surface people use on a plane, and because Vox's baseline is a zero-cost local one. |
| `LiveMeeting` | batch, offline, language pinning | Audio keeps arriving, so the decoder has to average faster than real time or a backlog grows. |
| `FinalPass` | batch, language pinning | No clock to race. Pinning matters most here, because a re-transcription exists precisely because the first attempt got the language wrong. |
| `Benchmark` | batch | Requires whatever an engine offers and nothing more, so a new engine can be measured before it is integrated. |

Everything not listed is a *preference*, and a preference belongs in a
measured comparison rather than in a requirement that silently excludes an
engine.

## The providers

`Support::Unknown` means **nobody has established this**, and is never treated
as a yes. A table of `no` where the honest answer is "unmeasured" reads as a
comparison and is not one.

| | Whisper | Parakeet TDT | A cloud provider (none integrated) |
|---|---|---|---|
| Id | `whisper` | `parakeet` | — |
| In the default build | yes | yes (`parakeet` feature) | — |
| Runs locally | yes | yes | no |
| Needs the network | no | no | yes |
| Batch | yes | yes | typically yes |
| Streaming | no | no | typically yes |
| Segment timestamps | yes | **unknown** | typically yes |
| Word timestamps | no | no | often yes |
| Language | selectable, with detection | **fixed: English** | varies |
| Speech-to-English in one pass | yes | no | varies |
| Reports no-speech evidence | yes (`no_speech_prob`) | **no** | varies |
| Partial transcripts | no | no | typically yes |
| Typical latency | model-dependent; measure it | lower than Whisper on the same audio, unmeasured here | network round trip plus decode |
| Typical accuracy | model-dependent; measure it | unmeasured here | unmeasured here |

Two entries are worth reading twice, because they are the reason the interface
declares capabilities instead of assuming them:

- **Parakeet reports no no-speech probability.** A uniform interface would have
  it return `0.0`, and Vox's hallucination screen — tuned against Whisper's
  probability — would then be grading it on a number it never produced. The
  adapter returns `None`, the capability says `no_speech_evidence: false`, and
  the benchmark records that in the report so a comparison cannot quietly
  mislead.
- **Parakeet's language is fixed at English.** A caller asking for Hindi has to
  be told no. Handing Hindi audio to an English-only model does not fail — it
  returns fluent nonsense in English phonology, which is worse than an error
  because nothing about it looks broken.

"Typical latency" and "typical accuracy" are left largely blank on purpose.
Vox has a benchmark (`docs/testing.md` §1b) and no corpus, so any number here
would be a recollection rather than a measurement. Fill them from a real run.

## Adding a provider

1. Implement `capture::recognizer::SpeechRecognizer` in
   `capture::recognizers`.
2. Declare what it can do in `RecognizerKind::capabilities`, using
   `Support::Unknown` for anything you have not verified. Do not guess upward.
3. Add a variant to `RecognizerChoice`. It carries a model path or directory —
   **there is deliberately no variant that can hold a credential**, so a secret
   cannot reach a settings file or a log by being expressible there. A cloud
   adapter reads its key through the keyring path the rest of Vox uses.
4. Benchmark it against an existing engine on the same corpus before changing
   any default. `docs/speech-decision-log.md` D-023 is the rule; the benchmark
   is how it is satisfied.

## What a cloud provider would and would not be

Optional acceleration, never a dependency. `decisions.md` Decision 4 and
NFR-1/NFR-3 make a zero-cost offline baseline a constraint rather than a
preference, and `SpeechTask::requires` encodes that: `PushToTalk` and
`LiveMeeting` demand `offline`, so a networked engine is structurally excluded
from them. `FinalPass` does not, because a final pass has no clock and a user
who wants to spend money on one meeting's accuracy is making a reasonable
choice.

Deepgram, or any other cloud STT, is therefore a **benchmark entry and an
optional provider**, not an architectural component. Nothing about the meeting
pipeline should require one to exist.

## Where the abstraction is, and is not, used yet

The benchmark runs through `SpeechRecognizer`. The production decode paths —
dictation, the live meeting worker, re-transcription — still call `SttEngine`
directly.

That is deliberate. Changing what decodes a meeting changes what users get, so
it belongs with the measurement that justifies it rather than arriving as a
refactor. The interface exists, both engines implement it, and the benchmark
exercises it; adopting it in the live path is Stage 6's business, where the
live/final split gives it a reason.
