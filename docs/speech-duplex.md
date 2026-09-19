# Speaking and listening at the same time

*What exists today, what it decides, and what each future that needs it would
still have to build.*

Written for `native/src-tauri/src/conversation/`, which landed at Stage 12 of
the speech architecture plan. Read
[speech-architecture-audit.md](speech-architecture-audit.md) first for how the
speech stack got here, and [speech-decision-log.md](speech-decision-log.md)
D-040 and D-041 for the two decisions this records.

## Status

| | |
|---|---|
| **Exists** | A state machine and an event model. `ConversationMachine`, `ConversationState`, `ConversationEvent`, `ConversationAction`, `BargeInPolicy`. Pure — events in, actions out. 16 tests. |
| **Does not exist** | Audio. A model. A network call. A Tauri command. Anything that calls it. |
| **Deliberately absent** | A duplex audio path, acoustic echo cancellation, a streaming recognizer, backchannel detection, and any product policy about when Vox should stop talking. |

Nothing in the shipped app changes. This is a seam, placed before there is
pressure to ship through it.

## Why now, and not later

The two inputs a conversation needs already exist, and they arrived for other
reasons:

- **A turn detector.** `meetings::speech_state` decides speaking from
  acoustics alone — no ASR, no model, no network — so it answers in
  milliseconds and keeps answering with the speech model uninstalled or the
  decode queue full. Stage 4 extracted it from the segmenter for the meeting
  path. A conversation needs exactly that and nothing more.
- **A voice that can be stopped mid-sentence.** `tts::SpeechQueue` cancels
  through a shared token rather than a flag checked between utterances
  (D-039). That property cannot be retrofitted — a queue that stops at the
  next item boundary makes the longest thing the system ever says its worst
  case.

What was missing was the piece between them that decides what those facts
*mean*: the user is making a sound while Vox is talking — is that an
interruption, agreement, or Vox's own voice coming back through the speakers?

That decision is cheap to make now and expensive to make under a deadline,
because getting it wrong produces a bug that looks like faulty hardware rather
than like a policy error. So the state model is written and tested now; the
audio path is not built at all.

## The state model

```text
           ┌──────── user speaks ─────────┐
           ▼                              │
  Idle ─▶ Listening ─▶ UserSpeaking ─▶ Thinking ─▶ Speaking
           ▲              │                           │
           │        (they stop)                user speaks over it
           │                                          ▼
           └────────────── stop speaking ──────── Interrupted
```

| State | Microphone | Turn owner | Meaning |
|---|---|---|---|
| `Idle` | closed | — | Not in a conversation. The microphone is not Vox's to hold. |
| `Listening` | open | — | In a conversation, nobody speaking. |
| `UserSpeaking` | open | User | |
| `Thinking` | open | Vox | The user stopped; the model is working. |
| `Speaking` | **open** | Vox | Vox has the turn *and the microphone is still open*. |
| `Interrupted` | open | Vox | A transition, not a resting place: the caller has just been told to stop. |

Three of those entries carry most of the design.

**`Speaking` keeps the microphone open.** That is the whole of what "full
duplex" means, and it is the one property that cannot be added afterwards: a
system whose microphone closes during playback has a recorder and a speaker,
not a conversation, and every layer built above that assumption has to be
redesigned to change it. `ConversationState::microphone_open` returns true in
every state but `Idle`, and a test pins it.

**`Thinking` belongs to Vox**, even though Vox is making no sound. The user
has stopped and is waiting, so a token arriving from the model is a
continuation rather than an interruption. But the microphone stays open, and
speech during `Thinking` returns the turn to the user immediately and abandons
the answer in flight — somebody who adds a sentence while Vox thinks has not
finished, whatever the acoustics decided 200 ms ago.

**`Interrupted` is a state rather than an inline action** because stopping is
not instantaneous. The caller has been handed `CancelSpeech`; the audio device
still has buffered samples; the user is mid-word. Collapsing that into
`UserSpeaking` would mean the machine claims the user has the turn while Vox
is still audible, and every consumer of the state would have to re-derive the
difference.

## Events in, actions out

| Event | Source |
|---|---|
| `Started` / `Stopped` | The user opening or closing a conversation. |
| `Speech(SpeechState)` | `meetings::speech_state`, one per audio frame. **The only audio-derived input.** |
| `ResponseStarted` / `ResponseFinished` | The model and the speech queue. |
| `CancelRequested` | The user stopping Vox by any means that is not their voice — a key, a button, a hotkey. |

| Action | What the caller does |
|---|---|
| `OpenMicrophone` / `CloseMicrophone` | |
| `BeginResponse` | The user's turn ended; ask the model. |
| `CancelSpeech` | `tts::SpeechQueue::cancel`. |

Actions are **returned, not performed**. The machine has no handles to a
microphone, a queue or a provider, so the whole of its behaviour is testable
by feeding it a sequence of verdicts, and the things it drives stay separately
testable and swappable. It returns at most one action per event: a machine
that returns a list invites a caller to perform them in the wrong order, and
no case has yet needed two.

`Speech` is the only audio-derived input on purpose. A conversation that needs
a transcript to know somebody is talking cannot react before the transcript
exists, which on the local Whisper path is hundreds of milliseconds after the
person started — far too late to stop Vox politely.

## The problem that makes naive barge-in fail

On a laptop with no headphones, the microphone hears the speaker. A detector
that treats any speech during playback as an interruption interrupts Vox with
Vox, every time. The user sees speech that cuts itself off a syllable in, and
reports it as a flaky microphone.

The real fix is acoustic echo cancellation, which needs the playback signal as
a reference. Vox does not have that: `meetings::capture` takes the default
output device in loopback for capturing *other people* in a meeting, and
nothing in the tree lines that up sample-for-sample against what is being
played for subtraction.

So the guard is in the state model instead, and is honest about being a
mitigation:

```rust
pub struct BargeInPolicy {
    pub frames_to_interrupt: usize,  // default 6 — 120 ms of held speech
    pub enabled: bool,
}
```

While Vox has the turn, interruption requires `frames_to_interrupt`
**consecutive** `Speaking` observations. Consecutive, not cumulative: echo
arrives as bursts that track Vox's own syllables, with gaps between them, so a
cumulative counter would eventually trip on any long enough answer. A test
feeds the machine exactly that pattern — speech, silence, speech, silence —
and asserts the run never accumulates.

The cost is real and worth stating: this makes talking over Vox harder for
everyone, including people on headphones who have no echo at all. 120 ms is a
guess at the balance, which is why it is a parameter on a policy struct and
not a constant. `BargeInPolicy::disabled()` turns barge-in off entirely, for
surfaces where Vox should finish its sentence.

**No product policy is implemented here.** Whether "mm-hm" should stop Vox
mid-sentence is a real question with a real answer that differs per surface,
and it belongs in the policy, not in the machine.

## What each future would still have to build

The state model is the part that is hard to change later. Everything below is
the part that is hard to *build*, and none of it is started. Listing it here
is the point of the document: a seam that implies more than it delivers is
worse than no seam.

### Full duplex

**Decided:** the microphone stays open while Vox speaks; turn ownership is
explicit; interruption has a policy with a default that survives laptop
speakers.

**Still missing:** a playback path (there is no provider behind
`TextToSpeech` at all — see `docs/decisions.md` Decision 69), a duplex device
configuration that does not fight the meeting recorder for the same devices,
and echo cancellation if barge-in is ever to be reliable on speakers. The
frame cadence the policy assumes (20 ms) has to be whatever the conversation's
capture actually produces; today that number is an assumption written in a
doc-comment, not a measured property of a running path.

### Backchannels

"Mm-hm", "right", "yeah" — agreement noises that a person makes *while* you
talk and does not intend as an interruption. Treating them as barge-in makes
Vox stop constantly for an engaged listener, which is the opposite of the
intent.

**Decided:** nothing, deliberately. `BargeInPolicy` is where the answer goes.

**Still missing:** everything. A backchannel is short, low-energy and
lexically tiny, and separating it from a real interruption probably needs
either a fast partial recognizer on the interrupting audio or a duration and
energy profile measured against real recordings. Vox has neither, and a
threshold invented without recordings would be a guess wearing a number's
clothing. The honest first version is the one shipped here — hold-time only —
with the measurement done before the second.

### Interruption

**Decided:** the full path. `Speech` while `Speaking` → run counting →
`Interrupted` → `CancelSpeech` → `UserSpeaking`. Cancellation reaches the
provider mid-utterance because `SpeechQueue` was built that way (D-039), and
audio arriving for a cancelled turn is discarded rather than played late.

**Still missing:** what happens to the abandoned answer. The machine says the
turn moved; it says nothing about whether the half-spoken response is kept in
the conversation history, marked as interrupted, or dropped. That is a
transcript question, and it wants the same provenance discipline as the
meeting path (`meetings::provenance`) rather than an invention.

### A real-time voice assistant

**Decided:** the state model, and the latency-shaped pieces beneath it —
`tts::phrases` so time to first audio is *first phrase plus one synthesis*
rather than *whole generation plus synthesis*, and a recognizer abstraction
(`capture::recognizer`) whose capabilities are declared rather than assumed,
so a task that needs streaming can ask instead of discovering by trial.

**Still missing:** the numbers. Nothing in the loop has been measured
end-to-end, because the loop does not exist. `meetings::benchmark` measures
the meeting path and `capture::evaluation` the dictation path; a conversation
needs its own budget — time to first response token, time to first audio,
time from a user's barge-in to silence — and the honest position is that none
of the three has a figure today. A real-time assistant is a latency product,
and shipping one against unmeasured latency is how it becomes an unusable one.

### STS and SpeechLM experimentation

Speech-to-speech models take audio and return audio, with no text turn in the
middle.

**Decided, and worth being explicit about:** this is not a migration path away
from the meeting pipeline, and the module says so. A meeting transcript's
value in Vox is that every claim traces back to a span of audio — claim →
canonical segment → raw ASR sequence → sample range (`meetings::canonical`,
`meetings::provenance`). An end-to-end speech model has no text turn to attach
that chain to, so adopting one for meetings would trade the property that
makes the output trustworthy for latency that meetings do not need.

Where it does fit is here: a live conversation, where latency is the product
and provenance is not the point. `ConversationMachine` is indifferent to what
produces the audio — it consumes `SpeechState` and emits `BeginResponse`, and
an STS provider satisfying that contract would need no change to the state
model.

**Still missing:** a provider interface for "audio in, audio out" (neither
`SpeechRecognizer` nor `TextToSpeech` is that shape), and a decision about
what such a turn writes to the vault, since a conversation with no transcript
leaves nothing behind. Both are real design work, and both are cheaper to do
against a state model that already exists than alongside one being invented.

## What is deliberately not here

- **No wiring.** No Tauri command, no frontend state, no `AppState` field.
  Exposing a machine nothing drives would make it look shipped.
- **No speculative behaviour.** No backchannel classifier, no adaptive
  threshold, no "assistant mode" setting.
- **No replacement of the meeting pipeline.** They are different products;
  see above.
- **No echo cancellation pretending to be echo cancellation.** The hold-time
  guard is a mitigation and is labelled as one, in the code and here.
