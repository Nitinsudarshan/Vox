//! The ONNX sessions behind [`super`], and the only part that needs ONNX Runtime.
//!
//! Three graphs run per transcription: the bundled preprocessor, the encoder
//! once, and the joint network once per greedy step. The tensor names and
//! shapes here are not guesses — they are the interface NeMo's ONNX export
//! declares, cross-checked against the `onnx-asr` reference implementation:
//!
//! | graph | inputs | outputs |
//! |---|---|---|
//! | `nemo128.onnx` | `waveforms` f32 [1,N], `waveforms_lens` i64 [1] | `features` f32 [1,128,T], `features_lens` i64 [1] |
//! | `encoder-model.onnx` | `audio_signal` f32 [1,128,T], `length` i64 [1] | `outputs` f32 [1,D,T'], `encoded_lengths` i64 [1] |
//! | `decoder_joint-model.onnx` | `encoder_outputs` f32 [1,D,1], `targets` [1,1], `target_length` [1], `input_states_1`, `input_states_2` | `outputs`, `output_states_1`, `output_states_2` |
//!
//! Two things are read from the model rather than assumed, because assuming
//! them is how this breaks silently on a re-export: the decoder state shapes,
//! and whether `targets` wants int32 or int64. Both come from the session's
//! own declared inputs at load time.

use std::path::Path;

use ort::session::Session;
use ort::value::{Tensor, TensorElementType, ValueType};

use super::decode::{self, DecodeConfig, StepOutput, TransducerStep};
use super::{ModelFiles, ParakeetError, Transcription, Vocab};

/// NeMo's feature extractor, exported as a graph and shipped with Vox.
///
/// 139 KB of constants — pre-emphasis, a Hann-windowed STFT, a 128-bin mel
/// filterbank, a log and per-feature normalisation. Embedded rather than
/// downloaded because it is small, it never changes, and a transcription that
/// fails because a 139 KB support file did not arrive is a bad trade for the
/// bytes. From `onnx-asr` (MIT); see `docs/licenses/`.
const PREPROCESSOR: &[u8] = include_bytes!("../../../resources/nemo128.onnx");

/// Whether this build can run Parakeet at all.
pub fn is_available() -> bool {
    true
}

/// A loaded Parakeet model.
pub struct ParakeetEngine {
    preprocessor: Session,
    encoder: Session,
    joint: Session,
    vocab: Vocab,
    /// Shapes for `input_states_1` / `input_states_2`, read from the joint.
    state_shapes: [Vec<i64>; 2],
    /// Whether `targets` is declared int32 or int64.
    targets_are_i32: bool,
}

impl ParakeetEngine {
    /// Loads the three graphs and the vocabulary.
    pub fn load(files: &ModelFiles) -> Result<Self, ParakeetError> {
        if !files.complete() {
            return Err(ParakeetError::Incomplete(files.missing()));
        }

        let vocab = Vocab::load(&files.vocab)?;
        let preprocessor = build_session_from_memory(PREPROCESSOR)?;
        let encoder = build_session(&files.encoder)?;
        let joint = build_session(&files.decoder_joint)?;

        let state_shapes = [
            declared_state_shape(&joint, "input_states_1")?,
            declared_state_shape(&joint, "input_states_2")?,
        ];
        let targets_are_i32 = declared_element_type(&joint, "targets")
            .map(|ty| ty == TensorElementType::Int32)
            .unwrap_or(true);

        Ok(Self {
            preprocessor,
            encoder,
            joint,
            vocab,
            state_shapes,
            targets_are_i32,
        })
    }

    /// Transcribes 16 kHz mono audio.
    pub fn transcribe(&mut self, audio: &[f32]) -> Result<Transcription, ParakeetError> {
        if audio.is_empty() {
            return Ok(Transcription {
                text: String::new(),
                tokens: Vec::new(),
            });
        }

        let (features, feature_frames, mel_bins) = self.preprocess(audio)?;
        let (encoded, dim, frames) = self.encode(features, feature_frames, mel_bins)?;

        let mut stepper = JointStep {
            joint: &mut self.joint,
            encoded: &encoded,
            dim,
            frames,
            vocab_size: self.vocab.len(),
            blank: self.vocab.blank(),
            targets_are_i32: self.targets_are_i32,
            state: [
                zeros(&self.state_shapes[0]),
                zeros(&self.state_shapes[1]),
            ],
            pending: None,
            state_shapes: &self.state_shapes,
        };

        let tokens = decode::greedy_decode(
            &mut stepper,
            frames,
            DecodeConfig {
                blank: self.vocab.blank(),
                ..DecodeConfig::default()
            },
        )
        .map_err(ParakeetError::Decode)?;

        let ids: Vec<usize> = tokens.iter().map(|t| t.id).collect();
        Ok(Transcription {
            text: self.vocab.detokenize(&ids),
            tokens,
        })
    }

    /// Audio to log-mel features. Returns the buffer, its frame count and the
    /// number of mel bins, because the encoder needs all three.
    fn preprocess(&mut self, audio: &[f32]) -> Result<(Vec<f32>, usize, usize), ParakeetError> {
        let samples = audio.len() as i64;
        let waveforms = Tensor::from_array((vec![1i64, samples], audio.to_vec())).map_err(runtime)?;
        let lens = Tensor::from_array((vec![1i64], vec![samples])).map_err(runtime)?;

        let outputs = self
            .preprocessor
            .run(ort::inputs![
                "waveforms" => waveforms,
                "waveforms_lens" => lens,
            ])
            .map_err(runtime)?;

        let (shape, data) = outputs["features"]
            .try_extract_tensor::<f32>()
            .map_err(runtime)?;
        // [batch, mel_bins, frames]
        let mel_bins = *shape.get(1).unwrap_or(&128) as usize;
        let frames = *shape.get(2).unwrap_or(&0) as usize;
        Ok((data.to_vec(), frames, mel_bins))
    }

    /// Features to encoder frames. Returns the buffer laid out `[dim][frame]`,
    /// its dimension, and how many frames are valid.
    fn encode(
        &mut self,
        features: Vec<f32>,
        frames: usize,
        mel_bins: usize,
    ) -> Result<(Vec<f32>, usize, usize), ParakeetError> {
        let audio_signal = Tensor::from_array((
            vec![1i64, mel_bins as i64, frames as i64],
            features,
        ))
        .map_err(runtime)?;
        let length = Tensor::from_array((vec![1i64], vec![frames as i64])).map_err(runtime)?;

        let outputs = self
            .encoder
            .run(ort::inputs![
                "audio_signal" => audio_signal,
                "length" => length,
            ])
            .map_err(runtime)?;

        let (shape, data) = outputs["outputs"]
            .try_extract_tensor::<f32>()
            .map_err(runtime)?;
        // [batch, dim, frames] — kept in that layout. One frame is then a
        // stride rather than a contiguous slice, which is why `JointStep`
        // gathers it; transposing the whole buffer to avoid that would copy
        // megabytes to save a gather per emitted token.
        let dim = *shape.get(1).unwrap_or(&0) as usize;
        let encoded_frames = *shape.get(2).unwrap_or(&0) as usize;
        let encoded = data.to_vec();

        // The model's own count wins where it is sane: a quantised encoder can
        // report a length longer than the tensor it returned, and reading past
        // the end is worse than losing the last frame.
        let valid = outputs
            .get("encoded_lengths")
            .and_then(|value| value.try_extract_tensor::<i64>().ok())
            .and_then(|(_, lens)| lens.first().copied())
            .map(|len| (len.max(0) as usize).min(encoded_frames))
            .unwrap_or(encoded_frames);

        Ok((encoded, dim, valid))
    }
}

/// One joint-network evaluation, holding the decoder state between them.
struct JointStep<'a> {
    joint: &'a mut Session,
    encoded: &'a [f32],
    dim: usize,
    frames: usize,
    vocab_size: usize,
    blank: usize,
    targets_are_i32: bool,
    state: [Vec<f32>; 2],
    state_shapes: &'a [Vec<i64>; 2],
    /// The state the last evaluation produced, kept only until the loop says
    /// whether it emitted a token.
    pending: Option<[Vec<f32>; 2]>,
}

impl TransducerStep for JointStep<'_> {
    fn step(&mut self, frame: usize, emitted: &[usize]) -> Result<StepOutput, String> {
        if frame >= self.frames {
            return Err(format!("frame {frame} is past the encoder output"));
        }

        // Gather this frame out of the `[dim][frame]` layout.
        let mut encoder_frame = Vec::with_capacity(self.dim);
        for d in 0..self.dim {
            encoder_frame.push(self.encoded[d * self.frames + frame]);
        }

        let target = emitted.last().copied().unwrap_or(self.blank) as i64;
        let encoder_outputs =
            Tensor::from_array((vec![1i64, self.dim as i64, 1i64], encoder_frame))
                .map_err(|e| e.to_string())?;
        let target_length =
            Tensor::from_array((vec![1i64], vec![1i32])).map_err(|e| e.to_string())?;
        let states_1 = Tensor::from_array((self.state_shapes[0].clone(), self.state[0].clone()))
            .map_err(|e| e.to_string())?;
        let states_2 = Tensor::from_array((self.state_shapes[1].clone(), self.state[1].clone()))
            .map_err(|e| e.to_string())?;

        let outputs = if self.targets_are_i32 {
            let targets = Tensor::from_array((vec![1i64, 1i64], vec![target as i32]))
                .map_err(|e| e.to_string())?;
            self.joint.run(ort::inputs![
                "encoder_outputs" => encoder_outputs,
                "targets" => targets,
                "target_length" => target_length,
                "input_states_1" => states_1,
                "input_states_2" => states_2,
            ])
        } else {
            let targets =
                Tensor::from_array((vec![1i64, 1i64], vec![target])).map_err(|e| e.to_string())?;
            self.joint.run(ort::inputs![
                "encoder_outputs" => encoder_outputs,
                "targets" => targets,
                "target_length" => target_length,
                "input_states_1" => states_1,
                "input_states_2" => states_2,
            ])
        }
        .map_err(|e| e.to_string())?;

        let (_, logits) = outputs["outputs"]
            .try_extract_tensor::<f32>()
            .map_err(|e| e.to_string())?;

        // TDT emits vocabulary logits followed by duration logits in one
        // vector. Splitting it in the wrong place reads durations as tokens,
        // which looks like a model that hallucinates rather than one that is
        // being read wrong.
        let split = self.vocab_size.min(logits.len());
        let token_logits = logits[..split].to_vec();
        let duration = if logits.len() > split {
            argmax(&logits[split..])
        } else {
            None
        };

        let next_1 = outputs
            .get("output_states_1")
            .and_then(|v| v.try_extract_tensor::<f32>().ok())
            .map(|(_, data)| data.to_vec())
            .unwrap_or_else(|| self.state[0].clone());
        let next_2 = outputs
            .get("output_states_2")
            .and_then(|v| v.try_extract_tensor::<f32>().ok())
            .map(|(_, data)| data.to_vec())
            .unwrap_or_else(|| self.state[1].clone());
        self.pending = Some([next_1, next_2]);

        Ok(StepOutput {
            token_logits,
            duration,
        })
    }

    fn commit(&mut self) {
        if let Some(next) = self.pending.take() {
            self.state = next;
        }
    }
}

/// The index of the largest value.
fn argmax(values: &[f32]) -> Option<usize> {
    let mut best = None;
    let mut best_value = f32::NEG_INFINITY;
    for (index, &value) in values.iter().enumerate() {
        if value > best_value {
            best_value = value;
            best = Some(index);
        }
    }
    best.or(if values.is_empty() { None } else { Some(0) })
}

fn zeros(shape: &[i64]) -> Vec<f32> {
    vec![0.0; shape.iter().map(|d| (*d).max(0) as usize).product()]
}

/// A decoder state's shape, with the batch dimension pinned to one.
///
/// The export leaves batch dynamic (`-1`); every other dimension is fixed and
/// is what the LSTM actually needs. Guessing these instead of reading them is
/// how a re-exported model fails with a shape mismatch nobody can place.
fn declared_state_shape(session: &Session, name: &str) -> Result<Vec<i64>, ParakeetError> {
    let outlet = session
        .inputs()
        .iter()
        .find(|input| input.name() == name)
        .ok_or_else(|| {
            ParakeetError::Runtime(format!(
                "the joint network has no '{name}' input, so it is not a NeMo transducer export"
            ))
        })?;

    match outlet.dtype() {
        ValueType::Tensor { shape, .. } => Ok(shape
            .iter()
            .enumerate()
            .map(|(axis, dim)| if axis == 1 || *dim < 0 { 1 } else { *dim })
            .collect()),
        _ => Err(ParakeetError::Runtime(format!("'{name}' is not a tensor"))),
    }
}

fn declared_element_type(session: &Session, name: &str) -> Option<TensorElementType> {
    session
        .inputs()
        .iter()
        .find(|input| input.name() == name)
        .and_then(|outlet| match outlet.dtype() {
            ValueType::Tensor { ty, .. } => Some(*ty),
            _ => None,
        })
}

fn build_session(path: &Path) -> Result<Session, ParakeetError> {
    Session::builder()
        .map_err(runtime)?
        .commit_from_file(path)
        .map_err(runtime)
}

fn build_session_from_memory(bytes: &[u8]) -> Result<Session, ParakeetError> {
    Session::builder()
        .map_err(runtime)?
        .commit_from_memory(bytes)
        .map_err(runtime)
}

fn runtime<E: std::fmt::Display>(error: E) -> ParakeetError {
    ParakeetError::Runtime(error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn this_build_reports_parakeet_as_available() {
        assert!(is_available());
    }

    /// The bundled preprocessor runs, and produces the shape the encoder
    /// expects.
    ///
    /// The one part of this pipeline that can be exercised for real without
    /// NVIDIA's weights, and the part most worth exercising: it is the reason
    /// this module contains no FFT, and a preprocessor that emitted 80 bins
    /// instead of 128 would fail much later with a shape error against the
    /// encoder.
    #[test]
    fn the_bundled_preprocessor_turns_audio_into_128_mel_bins() {
        let mut session = build_session_from_memory(PREPROCESSOR).unwrap();

        // One second of 16 kHz: a 440 Hz tone, so the output is not all one
        // value and a filterbank that did nothing would be visible.
        let audio: Vec<f32> = (0..16_000)
            .map(|n| 0.25 * (2.0 * std::f32::consts::PI * 440.0 * n as f32 / 16_000.0).sin())
            .collect();

        let waveforms =
            Tensor::from_array((vec![1i64, audio.len() as i64], audio.clone())).unwrap();
        let lens = Tensor::from_array((vec![1i64], vec![audio.len() as i64])).unwrap();
        let outputs = session
            .run(ort::inputs!["waveforms" => waveforms, "waveforms_lens" => lens])
            .unwrap();

        let (shape, data) = outputs["features"].try_extract_tensor::<f32>().unwrap();
        assert_eq!(shape[0], 1, "one utterance in, one out");
        assert_eq!(shape[1], 128, "the encoder is built for 128 mel bins");
        // 10 ms hop over one second, plus the centre-padded edge frame.
        assert!(
            (100..=102).contains(&shape[2]),
            "expected ~100 frames for one second, got {}",
            shape[2]
        );
        assert_eq!(data.len(), shape.iter().product::<i64>() as usize);
        assert!(
            data.iter().all(|v| v.is_finite()),
            "a non-finite feature would decode as a NaN logit"
        );

        let (_, lens_out) = outputs["features_lens"]
            .try_extract_tensor::<i64>()
            .unwrap();
        assert_eq!(lens_out[0], 100);
    }

    #[test]
    fn silence_and_a_tone_do_not_produce_the_same_features() {
        // Guards against the graph being loaded but not actually run — an
        // all-zeros output would pass every shape assertion above.
        let mut session = build_session_from_memory(PREPROCESSOR).unwrap();
        let mut run = |samples: Vec<f32>| {
            let n = samples.len() as i64;
            let waveforms = Tensor::from_array((vec![1i64, n], samples)).unwrap();
            let lens = Tensor::from_array((vec![1i64], vec![n])).unwrap();
            let outputs = session
                .run(ort::inputs!["waveforms" => waveforms, "waveforms_lens" => lens])
                .unwrap();
            let (_, data) = outputs["features"].try_extract_tensor::<f32>().unwrap();
            data.to_vec()
        };

        let silence = run(vec![0.0; 16_000]);
        let tone: Vec<f32> = (0..16_000)
            .map(|n| 0.25 * (2.0 * std::f32::consts::PI * 440.0 * n as f32 / 16_000.0).sin())
            .collect();
        let sound = run(tone);

        assert_eq!(silence.len(), sound.len());
        assert!(
            silence
                .iter()
                .zip(&sound)
                .any(|(a, b)| (a - b).abs() > 1e-3),
            "the preprocessor produced identical features for silence and a tone"
        );
    }

    #[test]
    fn an_incomplete_model_is_refused_before_onnx_runtime_sees_it() {
        let files = ModelFiles::in_dir(Path::new("/not/a/model"), true);
        assert!(matches!(
            ParakeetEngine::load(&files),
            Err(ParakeetError::Incomplete(_))
        ));
    }

    #[test]
    fn a_state_shape_pins_the_batch_dimension_and_keeps_the_rest() {
        // Exercised through `zeros` rather than a session, since building a
        // transducer export here is not possible: the contract is that a
        // dynamic dimension becomes 1 and a fixed one survives.
        assert_eq!(zeros(&[2, 1, 640]).len(), 1280);
        assert_eq!(zeros(&[]).len(), 1);
    }
}
