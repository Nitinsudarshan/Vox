# Vox ASR Shootout

How Vox decides which recognition stack to use, and why the decision is a
measurement rather than a preference.

Written against **v0.1.0**. Where this and the code disagree, the code is right
and this file is a bug.

---

## 1. The question

Not "which Whisper model should Vox use". That question has an answer that
expires, and answering it once bakes the answer into the architecture.

The question is **which of these is costing us**:

```text
audio quality  →  segmentation  →  context  →  model  →  inference engine  →  hardware
```

Every one of them can produce a bad transcript, they produce *different* bad
transcripts, and they are fixed in different places. A larger model does not
fix clipped audio. A faster inference engine does not fix a segmenter that cuts
mid-clause. Swapping Whisper Small for Large-v3-Turbo improves accuracy and
makes throughput worse, and whether that is a good trade depends entirely on
which of the two is currently the binding constraint.

So the shootout runs several configurations over one corpus and reports, per
row, which variable the result points at.

## 2. The three rules

Each exists because the shortcut past it produces a number that looks like a
result.

1. **One evaluator.** Every engine adapter returns a transcript and timings in
   the same shape; `tests/transcription/evaluator/metrics.py` scores all of
   them. An engine that brought its own word error rate would be graded on its
   own text normalization, and the comparison would be between normalizers.
2. **Every run states its configuration.** Model, engine, quantization, decode
   preset, language mode, context mode, segmentation mode, Vox commit, host.
   A run that does not say what it was cannot be compared with anything —
   including itself next month. This is why
   `native/src-tauri/src/bin/benchmark.rs` takes flags instead of constants.
3. **The audio is checked before the models are.** A case whose recording is
   clipped cannot separate two models, so it is refused rather than scored.
   Not hypothetical: see §6.

An engine that cannot run is reported as `skipped` with an actionable reason —
the model file is missing, the package is not installed, the feature is not
compiled. It is never silently dropped, because a comparison missing its
fastest entrant still reads as a comparison.

## 3. The pieces

| Piece | Where | What it is |
|---|---|---|
| Corpus | `tests/transcription/corpus-v1/` | Eight synthesized cases: English, Hindi, Hinglish; clean, fast, noisy, technical. See §6. |
| Generator | `tests/transcription/generator/` | Rebuilds the corpus from `corpus_definitions.py` through edge-tts. |
| Audio guard | `tests/transcription/generator/audio_guard.py` | Refuses to write a case whose audio cannot support a comparison. |
| Vox adapter | `native/src-tauri/src/bin/benchmark.rs` | One engine configuration over one file. Also `--audio-stats-only`. |
| Orchestrator | `tests/transcription/runner/shootout.py` | Runs the matrix, scores it through one evaluator, classifies each row. |
| Matrix | `tests/transcription/shootout/engines.json` | The configurations. Edit this, not the runner. |
| Evaluator | `tests/transcription/evaluator/metrics.py` | WER, CER, code-switch retention, domain terms, hallucination rate. |
| Tests | `tests/transcription/tests/` | The audio guard and the comparative taxonomy. No model needed; runs in CI. |

There is a second, older harness — `meetings::benchmark`, driven by the
`run_speech_benchmark` Tauri command — which scores the meeting pipeline from
inside the app and declares each engine's capabilities. It is not redundant
with this one and it does not know about it. Reconciling them is open work.

## 4. Running it

```bash
# once, and in release: a debug build makes everything around whisper.cpp slow
cd native/src-tauri && cargo build --release --bin benchmark

# the audio gate alone — no model required
python3 tests/transcription/runner/shootout.py --audit-only

# the matrix
python3 tests/transcription/runner/shootout.py
```

Models are not bundled. Download the ggml files named in `engines.json` from
<https://huggingface.co/ggerganov/whisper.cpp> and put them where it points, or
edit the paths. An entry whose model is missing costs one skipped row.

Reports land in `tests/transcription/reports/` as `ASR_SHOOTOUT.md` and
`shootout.json`.

## 5. The experiments, and what each one isolates

The matrix in `engines.json` is ordered so that consecutive rows differ in one
thing. That is the whole design: a difference is only attributable when
everything else is held.

| Pair | Isolates | Reads as |
|---|---|---|
| `vox-small-baseline` vs `vox-small-whole-file` | Vox's segmenter | `SEGMENTATION` |
| `vox-small-baseline` vs `vox-small-no-context` | prompt conditioning | `CONTEXT` |
| `vox-small-no-context` vs `vox-small-static-context` | vocabulary alone, without continuity | `CONTEXT` |
| `vox-small-baseline` vs `vox-base` | model capacity against decode cost | `MODEL` |
| `vox-small-baseline` vs `vox-large-v3-turbo` | the accuracy ceiling | `MODEL` |
| `vox-small-whole-file` vs `faster-whisper-small` | the inference engine | `INFERENCE_ENGINE` |

The last pair is the one most likely to be surprising and the one most often
skipped: the same weights under whisper.cpp and under CTranslate2 are the same
model and a different program. If one runs at three times real time and the
other at a third of it, the bottleneck was never the model.

`faster-whisper` rows are compared against `whole-file` rows and not against
`vox` rows, because that adapter runs no segmenter, no bounded queue and no
quality gate. Putting it beside a full-pipeline row would attribute Vox's
pipeline to CTranslate2.

## 6. What the audio gate found

The gate was written to guard future corpora. The first thing it did was
invalidate the existing one.

Every case in `corpus-v1` is between 89% and 99% *clipped* — RMS around 0.95
against a full scale of 1.0, peak exactly 1.0, and the adaptive VAD measuring
0% voiced because the noise floor and the signal are the same value. The cause
is one line in the generator: `miniaudio.decode` returns **signed 16-bit
integers** by default, and the writer treated them as floats in `[-1.0, 1.0]`,
so `max(-1.0, min(1.0, s))` clamped every non-zero sample to ±1 before
multiplying by 32767. The corpus is the *sign* of the waveform: a square wave.

Nothing objected. The files are valid 16 kHz mono WAVs of the right duration,
they play as recognisable speech, and Whisper transcribed them at 15.17% word
error rate — which reads as a model result and is a measurement of 1-bit
distortion.

Consequences, stated plainly:

- **Every accuracy number published from `corpus-v1` is void**, including the
  15.17% WER and 8.03% CER in `TRANSCRIPTION_BENCHMARK_V1.md`.
- **The throughput numbers are suspect rather than void.** Decode cost tracks
  transcript length and segment count, both of which this distortion moves, so
  3.03 RTF is a real measurement of a run that should not have happened.
- The generator is fixed and now refuses to write a case that fails the gate.
  Regenerating needs network access to the TTS endpoint.

The gate's thresholds are in `benchmark.rs`: 1% of samples at or beyond 98% of
full scale is called clipped, RMS under 0.01 is called silence, and an
estimated SNR under 10 dB is called out. A percent is deliberately strict —
unlimited speech reaches full scale on a handful of vowel peaks at most.

## 7. What is not here yet

- **A Parakeet adapter.** `capture::recognizers::RecognizerChoice` builds one
  and `meetings::benchmark::CaseRunner` drives it, but neither is reachable
  from a command line. The marker is in `shootout.py` beside the adapter table.
- **Peak memory per run.** `psutil` gives it cross-platform when installed; the
  runner reports it as unmeasured otherwise rather than guessing.
- **First-token latency.** `meetings::benchmark::LatencyReport` already
  measures `first_transcript_ms` and finalization percentiles under
  `Pacing::Realtime`; the shootout's Vox adapter drives `spawn_worker` in batch
  and does not. Pacing the adapter to wall clock is what connects the two.
- **A recorded corpus.** Everything above is synthesized speech: clean
  articulation, no overlap, no room, no accent variation. A word error rate
  from it is a floor, not an estimate.

## 8. Definition of done

The ASR work is not done when the transcript looks better. It is done when, on
a named machine and a named commit, the shootout can show:

| | Current | Target |
|---|---|---|
| WER, per language | measured | improved |
| Hinglish retention | measured | improved |
| Domain terms | measured | improved |
| decode RTF | measured | < 1.0 sustained |
| Dropped segments | measured | zero |
| Queue growth | measured | stable |
| Hallucination rate | measured | controlled |

and can answer "why is this version better than the last" with a row of
numbers rather than an impression.
