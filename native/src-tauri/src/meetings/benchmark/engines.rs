//! The engines a corpus can be run against, behind one interface.
//!
//! Each adapter is thin on purpose: it names what it is, hands the segmenter's
//! audio to a real decoder, and reports what came back. Everything that makes
//! a measurement comparable — segmentation, screening, normalization, the
//! queue — lives in [`super`] and is identical whichever engine is chosen.
//!
//! An engine that reports no per-decode no-speech probability returns `None`
//! rather than a stand-in, so a screen tuned for Whisper does not silently
//! grade another engine on a number it never produced.

use std::path::{Path, PathBuf};

use crate::capture::stt::{
    join_utterance_text, SttEngine, SttLanguageConfig, WhisperDecodingConfig,
};

use super::{BenchmarkDecoder, BenchmarkError, DecodeOutput, EngineDescriptor};

/// Whisper, decoded through the same call the meeting worker makes.
pub struct WhisperDecoder {
    engine: SttEngine,
    model_path: String,
    language: SttLanguageConfig,
    decoding: WhisperDecodingConfig,
    profile: String,
}

impl WhisperDecoder {
    /// `decoding` is passed in rather than derived here so a benchmark can
    /// compare two profiles over the same audio — which is the question
    /// "is the careful profile worth what it costs" reduces to.
    pub fn new(
        engine: SttEngine,
        model_path: impl Into<String>,
        language: SttLanguageConfig,
        decoding: WhisperDecodingConfig,
    ) -> Self {
        let profile = format!("{:?}", decoding.strategy);
        Self {
            engine,
            model_path: model_path.into(),
            language,
            decoding,
            profile,
        }
    }
}

impl BenchmarkDecoder for WhisperDecoder {
    fn descriptor(&self) -> EngineDescriptor {
        EngineDescriptor {
            engine: "whisper".to_string(),
            model: file_name(&self.model_path),
            language: self.language.whisper_language.clone(),
            profile: self.profile.clone(),
        }
    }

    fn decode(&mut self, samples: &[f32]) -> Result<DecodeOutput, String> {
        let (utterances, _diagnostics) = self
            .engine
            .transcribe_utterances_with_config(
                Some(&self.model_path),
                samples,
                &self.language,
                &self.decoding,
            )
            .map_err(|err| err.to_string())?;

        let mean_no_speech_prob = if utterances.is_empty() {
            None
        } else {
            Some(
                utterances.iter().map(|u| u.no_speech_prob).sum::<f32>()
                    / utterances.len() as f32,
            )
        };
        Ok(DecodeOutput {
            text: join_utterance_text(&utterances),
            mean_no_speech_prob,
        })
    }
}

/// Parakeet TDT, through the same engine handle.
///
/// English only, and it reports no no-speech probability — both stated here
/// rather than discovered from a report that looks comparable and is not.
pub struct ParakeetDecoder {
    engine: SttEngine,
    model_dir: PathBuf,
}

impl ParakeetDecoder {
    pub fn new(engine: SttEngine, model_dir: impl Into<PathBuf>) -> Self {
        Self {
            engine,
            model_dir: model_dir.into(),
        }
    }
}

impl BenchmarkDecoder for ParakeetDecoder {
    fn descriptor(&self) -> EngineDescriptor {
        EngineDescriptor {
            engine: "parakeet".to_string(),
            model: "parakeet-tdt-0.6b-v3".to_string(),
            language: Some("en".to_string()),
            profile: "tdt-greedy".to_string(),
        }
    }

    fn decode(&mut self, samples: &[f32]) -> Result<DecodeOutput, String> {
        let (text, _diagnostics) = self
            .engine
            .transcribe_parakeet(&self.model_dir, samples)
            .map_err(|err| err.to_string())?;
        Ok(DecodeOutput {
            text,
            mean_no_speech_prob: None,
        })
    }
}

/// Which engine a run should use.
///
/// A closed set rather than a string the caller invents: an engine name that
/// does not resolve is a benchmark that silently measures the default one and
/// labels the result with something else.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case", tag = "engine")]
pub enum EngineChoice {
    Whisper {
        /// Absolute path to a `.bin` model file.
        model_path: String,
    },
    Parakeet {
        /// Directory holding the Parakeet ONNX files.
        model_dir: String,
    },
}

impl EngineChoice {
    /// Builds the decoder, failing before any audio is read when the model is
    /// not actually installed.
    pub fn build(
        &self,
        engine: SttEngine,
        language: SttLanguageConfig,
        decoding: WhisperDecodingConfig,
    ) -> Result<Box<dyn BenchmarkDecoder>, BenchmarkError> {
        match self {
            Self::Whisper { model_path } => {
                if !Path::new(model_path).is_file() {
                    return Err(BenchmarkError::Invalid(format!(
                        "no speech model at {model_path}"
                    )));
                }
                Ok(Box::new(WhisperDecoder::new(
                    engine, model_path, language, decoding,
                )))
            }
            Self::Parakeet { model_dir } => {
                let dir = PathBuf::from(model_dir);
                if !crate::capture::parakeet::ModelFiles::is_installed_in(&dir) {
                    return Err(BenchmarkError::Invalid(format!(
                        "the Parakeet model files are not installed in {model_dir}"
                    )));
                }
                Ok(Box::new(ParakeetDecoder::new(engine, dir)))
            }
        }
    }
}

fn file_name(path: &str) -> String {
    Path::new(path)
        .file_name()
        .map(|name| name.to_string_lossy().to_string())
        .unwrap_or_else(|| path.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_descriptor_names_the_model_file_and_never_its_path() {
        // A report is shared, and a path names a machine and usually a person.
        let decoder = WhisperDecoder::new(
            SttEngine::new(),
            "/home/someone/.Vox/config/models/ggml-small.bin",
            SttLanguageConfig {
                whisper_language: Some("en".into()),
                translate: false,
            },
            WhisperDecodingConfig::default(),
        );
        let descriptor = decoder.descriptor();
        assert_eq!(descriptor.model, "ggml-small.bin");
        assert!(!descriptor.model.contains('/'));
        assert_eq!(descriptor.engine, "whisper");
    }

    #[test]
    fn parakeet_declares_that_it_reports_no_no_speech_probability() {
        // Stated in the adapter rather than inferred from a run, so a
        // comparison against whisper cannot quietly grade it on a number it
        // does not produce.
        let decoder = ParakeetDecoder::new(SttEngine::new(), "/models/parakeet");
        assert_eq!(decoder.descriptor().language.as_deref(), Some("en"));
    }

    #[test]
    fn a_missing_model_fails_before_any_audio_is_read() {
        let choice = EngineChoice::Whisper {
            model_path: "/definitely/not/here/ggml-small.bin".into(),
        };
        let built = choice.build(
            SttEngine::new(),
            SttLanguageConfig {
                whisper_language: None,
                translate: false,
            },
            WhisperDecodingConfig::default(),
        );
        assert!(built.is_err(), "a benchmark must not start without its model");
    }

    #[test]
    fn an_engine_choice_round_trips_through_json() {
        let choice = EngineChoice::Parakeet {
            model_dir: "models/parakeet".into(),
        };
        let json = serde_json::to_string(&choice).expect("serialize");
        let back: EngineChoice = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(choice, back);
    }
}
