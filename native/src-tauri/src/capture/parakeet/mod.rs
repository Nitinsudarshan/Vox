//! Parakeet TDT — NVIDIA's transducer ASR, run locally through ONNX Runtime.
//!
//! Meetily's default engine, and the reason its transcription keeps up with a
//! meeting on a laptop that Whisper's larger models cannot: a 0.6B transducer
//! at int8 decodes a great deal faster than `large-v3-turbo`, and its duration
//! head lets it skip most of the encoder frames rather than stepping through
//! them.
//!
//! ## The shape of a transcription
//!
//! ```text
//! 16 kHz mono f32
//!        │
//!        ▼  nemo128.onnx        waveforms → features [1, 128, T]
//!   preprocessor                (log-mel, per-feature normalised)
//!        │
//!        ▼  encoder-model.onnx  audio_signal → outputs [1, D, T'], encoded_lengths
//!    encoder
//!        │
//!        ▼  decoder_joint-model.onnx, once per emitted token
//!  greedy TDT  ──────────────▶ token ids ──▶ vocab.txt ──▶ text
//! ```
//!
//! ## What is proven, and what is not
//!
//! [`decode`] and [`vocab`] are pure and compile in every build, tested
//! against scripted logits and hand-written vocabularies. They hold the
//! algorithm — blank handling, duration skipping, the per-frame token budget,
//! SentencePiece detokenisation — which is where transducer decoding actually
//! goes wrong.
//!
//! The session code below is behind the `parakeet` feature because ONNX
//! Runtime is not in a default build. It was written against a reference
//! implementation's tensor names and shapes rather than guessed, and the
//! preprocessor is exercised for real in tests. What has *not* been run is
//! this pipeline against NVIDIA's published weights: they are on Hugging Face,
//! which the container this was written in cannot reach. Whoever runs it first
//! should expect the interface to be right and should still check the first
//! transcript against Whisper's before trusting it.
//!
//! ## The preprocessor is not a model
//!
//! `nemo128.onnx` is bundled with Vox — 139 KB, MIT-licensed, from the
//! `onnx-asr` project. It is NeMo's feature extraction exported as a graph:
//! pre-emphasis, a Hann-windowed 512-point STFT at a 160-sample hop, a 128-bin
//! mel filterbank, a log, and per-feature normalisation. Constants, not
//! learned parameters. Bundling it rather than reimplementing it is the whole
//! reason this module has no FFT in it, and the mel front end is exactly where
//! a hand-written ASR pipeline produces plausible-looking garbage.

pub mod decode;
pub mod vocab;

use std::path::{Path, PathBuf};

pub use decode::{DecodeConfig, DecodedToken, StepOutput, TransducerStep};
pub use vocab::Vocab;

/// Encoder frames per second of audio.
///
/// The preprocessor emits one feature frame per 10 ms and the encoder
/// subsamples by 8, so a frame index times this is a timestamp. Used to place
/// segment boundaries, never to claim word-level timing.
pub const ENCODER_FRAMES_PER_SECOND: f64 = 12.5;

/// The audio rate every part of this expects.
pub const SAMPLE_RATE: u32 = 16_000;

/// The files one installed Parakeet model is made of.
///
/// Named here rather than at each call site because the model is three files
/// and a missing one fails inside ONNX Runtime with a message about a path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelFiles {
    pub encoder: PathBuf,
    pub decoder_joint: PathBuf,
    pub vocab: PathBuf,
}

impl ModelFiles {
    /// Where the three files sit inside a model directory.
    ///
    /// `quantized` selects the `.int8` variants, which is what makes Parakeet
    /// worth running on a laptop at all.
    pub fn in_dir(dir: &Path, quantized: bool) -> Self {
        let suffix = if quantized { ".int8" } else { "" };
        Self {
            encoder: dir.join(format!("encoder-model{suffix}.onnx")),
            decoder_joint: dir.join(format!("decoder_joint-model{suffix}.onnx")),
            vocab: dir.join("vocab.txt"),
        }
    }

    /// Every file, for checking what is installed.
    pub fn all(&self) -> [&Path; 3] {
        [&self.encoder, &self.decoder_joint, &self.vocab]
    }

    /// Whether all three are present.
    pub fn complete(&self) -> bool {
        self.all().iter().all(|path| path.is_file())
    }

    /// The files that are absent, for an error that names them.
    pub fn missing(&self) -> Vec<String> {
        self.all()
            .iter()
            .filter(|path| !path.is_file())
            .map(|path| {
                path.file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_default()
            })
            .collect()
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ParakeetError {
    #[error("the Parakeet model is incomplete — missing {}", .0.join(", "))]
    Incomplete(Vec<String>),

    #[error(transparent)]
    Vocab(#[from] vocab::VocabError),

    #[error("ONNX Runtime: {0}")]
    Runtime(String),

    #[error("decoding failed: {0}")]
    Decode(String),

    #[error("this build has no Parakeet support; rebuild with `--features parakeet`")]
    NotCompiledIn,
}

/// What one transcription produced.
#[derive(Debug, Clone, PartialEq)]
pub struct Transcription {
    pub text: String,
    pub tokens: Vec<DecodedToken>,
}

impl Transcription {
    /// Seconds into the audio that the first token was heard.
    pub fn first_token_seconds(&self) -> Option<f64> {
        self.tokens
            .first()
            .map(|t| t.frame as f64 / ENCODER_FRAMES_PER_SECOND)
    }
}

#[cfg(not(feature = "parakeet"))]
mod engine {
    use super::*;

    /// The stand-in for a build without ONNX Runtime.
    ///
    /// A type that exists and refuses, rather than a module that is absent:
    /// the call sites then compile in every configuration and the error a user
    /// sees names the build rather than failing to resolve a symbol.
    #[derive(Debug)]
    pub struct ParakeetEngine;

    impl ParakeetEngine {
        pub fn load(_files: &ModelFiles) -> Result<Self, ParakeetError> {
            Err(ParakeetError::NotCompiledIn)
        }

        pub fn transcribe(&mut self, _audio: &[f32]) -> Result<Transcription, ParakeetError> {
            Err(ParakeetError::NotCompiledIn)
        }
    }

    /// Whether this build can run Parakeet at all.
    pub fn is_available() -> bool {
        false
    }
}

#[cfg(feature = "parakeet")]
mod engine;

pub use engine::{is_available, ParakeetEngine};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_model_directory_is_three_files() {
        let files = ModelFiles::in_dir(Path::new("/models/parakeet"), false);
        assert!(files.encoder.ends_with("encoder-model.onnx"));
        assert!(files.decoder_joint.ends_with("decoder_joint-model.onnx"));
        assert!(files.vocab.ends_with("vocab.txt"));
    }

    #[test]
    fn the_quantised_variant_changes_the_graphs_and_not_the_vocabulary() {
        let files = ModelFiles::in_dir(Path::new("/models/parakeet"), true);
        assert!(files.encoder.ends_with("encoder-model.int8.onnx"));
        assert!(files.decoder_joint.ends_with("decoder_joint-model.int8.onnx"));
        // One vocabulary serves both: quantisation changes the weights, not
        // the token set.
        assert!(files.vocab.ends_with("vocab.txt"));
    }

    #[test]
    fn an_incomplete_model_names_what_is_absent() {
        let files = ModelFiles::in_dir(Path::new("/definitely/not/here"), false);
        assert!(!files.complete());
        assert_eq!(
            files.missing(),
            vec!["encoder-model.onnx", "decoder_joint-model.onnx", "vocab.txt"]
        );
    }

    #[test]
    fn a_frame_index_converts_to_seconds_at_the_encoder_rate() {
        // 10 ms per feature frame, subsampled by 8 — so 12.5 frames a second.
        let transcription = Transcription {
            text: "hello".into(),
            tokens: vec![DecodedToken { id: 1, frame: 25 }],
        };
        assert_eq!(transcription.first_token_seconds(), Some(2.0));
    }

    #[test]
    fn nothing_decoded_has_no_first_token() {
        let transcription = Transcription {
            text: String::new(),
            tokens: Vec::new(),
        };
        assert_eq!(transcription.first_token_seconds(), None);
    }

    #[test]
    #[cfg(not(feature = "parakeet"))]
    fn a_build_without_the_feature_says_so_rather_than_failing_obscurely() {
        assert!(!is_available());
        let files = ModelFiles::in_dir(Path::new("/models/parakeet"), true);
        let error = ParakeetEngine::load(&files).unwrap_err();
        assert!(matches!(error, ParakeetError::NotCompiledIn));
        assert!(error.to_string().contains("--features parakeet"));
    }
}
