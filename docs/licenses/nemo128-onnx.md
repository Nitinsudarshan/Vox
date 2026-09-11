# `nemo128.onnx` — bundled third-party file

**Path**: `native/src-tauri/resources/nemo128.onnx`
**Size**: 139 KB
**Source**: [`onnx-asr`](https://pypi.org/project/onnx-asr/) — `onnx_asr/preprocessors/data/nemo128.onnx`
**Licence**: MIT

## What it is

NeMo's audio feature extraction, exported as an ONNX graph. It takes a 16 kHz
mono waveform and produces the 128-bin log-mel features Parakeet's encoder
expects:

```
waveforms      f32 [batch, N]     ─┐
waveforms_lens i64 [batch]        ─┴─▶  features      f32 [batch, 128, T]
                                        features_lens i64 [batch]
```

Inside it: pre-emphasis at 0.97, a Hann-windowed 512-point STFT at a
400-sample window and 160-sample hop, a 128-bin mel filterbank, a log with a
zero guard, and per-feature normalisation.

## Why it is in the repository

It is not a model. There are no learned parameters in it — the constants are a
window function and a mel filterbank, both of which are determined by the
sample rate and the bin count. Vox still bundles no model weights.

The alternative was writing the same DSP in Rust, which means an FFT
dependency and matching NeMo's exact preprocessing conventions by hand. The
mel front end is where a hand-written ASR pipeline produces output that looks
plausible and transcribes badly, and the failure is silent: nothing errors,
the words are just wrong. 139 KB is a good price for removing that.

Downloading it at runtime was the other option. It is small enough that
shipping it is cheaper than the code to fetch, cache and verify it, and a
transcription that fails because a 139 KB support file did not arrive is a bad
trade.

## Keeping it honest

`capture::parakeet::engine` embeds it with `include_bytes!` and two tests run
it for real —
`the_bundled_preprocessor_turns_audio_into_128_mel_bins` checks the contract
the encoder depends on, and `silence_and_a_tone_do_not_produce_the_same_features`
checks the graph is actually executing rather than returning zeros.

To update it, take the file from a current `onnx-asr` wheel and run those two
tests:

```bash
pip download onnx-asr --no-deps -d /tmp/ref && cd /tmp/ref && unzip -q *.whl
cp onnx_asr/preprocessors/data/nemo128.onnx <repo>/native/src-tauri/resources/
```
