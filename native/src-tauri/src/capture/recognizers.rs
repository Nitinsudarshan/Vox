//! The engines Vox ships, behind [`SpeechRecognizer`].
//!
//! Each adapter is thin by design: name what it is, hand the audio to the
//! engine, and report what came back — including what did *not* come back.
//! Everything a comparison depends on (segmentation, screening, normalization)
//! lives above this layer and is identical whichever engine is chosen, so a
//! difference between two runs is a difference between two engines.

use std::path::{Path, PathBuf};

use super::recognizer::{
    Recognition, RecognitionError, RecognitionRequest, RecognitionTiming, RecognizedSpan,
    RecognizerDescriptor, RecognizerKind, SpeechRecognizer,
};
use super::stt::{
    join_utterance_text, SttEngine, SttLanguageConfig, WhisperDecodingConfig,
};

/// The model's filename, for a descriptor that will be read on another
/// machine. Never the path: a benchmark report and a diagnostics file both get
/// shared, and a path names a machine and usually a person.
fn file_name(path: &Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().to_string())
        .unwrap_or_else(|| path.to_string_lossy().to_string())
}

/// Whisper, through the same call the meeting worker makes.
pub struct WhisperRecognizer {
    engine: SttEngine,
    model_path: PathBuf,
    /// The decode profile. Passed in rather than derived here so two profiles
    /// can be compared over the same audio — which is what "is the careful
    /// profile worth what it costs" reduces to.
    decoding: WhisperDecodingConfig,
}

impl WhisperRecognizer {
    pub fn new(
        engine: SttEngine,
        model_path: impl Into<PathBuf>,
        decoding: WhisperDecodingConfig,
    ) -> Self {
        Self {
            engine,
            model_path: model_path.into(),
            decoding,
        }
    }
}

impl SpeechRecognizer for WhisperRecognizer {
    fn descriptor(&self) -> RecognizerDescriptor {
        RecognizerDescriptor {
            id: RecognizerKind::Whisper.id().to_string(),
            display_name: RecognizerKind::Whisper.display_name().to_string(),
            model: file_name(&self.model_path),
            capabilities: RecognizerKind::Whisper.capabilities(),
        }
    }

    fn transcribe(&self, request: RecognitionRequest<'_>) -> Result<Recognition, RecognitionError> {
        if !self.model_path.is_file() {
            return Err(RecognitionError::ModelMissing {
                engine: RecognizerKind::Whisper.id().to_string(),
            });
        }
        let language = SttLanguageConfig {
            whisper_language: request.language.clone(),
            translate: request.translate,
        };
        let (utterances, diagnostics) = self
            .engine
            .transcribe_utterances_with_config(
                Some(&self.model_path.to_string_lossy()),
                request.samples,
                &language,
                &self.decoding,
            )
            .map_err(|err| RecognitionError::Failed(err.to_string()))?;

        let text = join_utterance_text(&utterances);
        Ok(Recognition {
            spans: utterances
                .iter()
                .map(|utterance| RecognizedSpan {
                    text: utterance.text.clone(),
                    start_seconds: None,
                    end_seconds: None,
                    no_speech_prob: Some(utterance.no_speech_prob),
                })
                .collect(),
            text,
            language: request.language,
            timing: RecognitionTiming {
                decode_ms: diagnostics.transcription_latency_ms,
                lock_wait_ms: diagnostics.lock_wait_ms,
                model_load_ms: diagnostics.model_load_ms,
                model_reloaded: diagnostics.model_reloaded,
            },
        })
    }
}

/// Parakeet TDT.
///
/// English only, and it reports no per-decode no-speech probability. Both are
/// declared in [`RecognizerKind::capabilities`] rather than discovered from a
/// report that looks comparable and is not, and this adapter returns `None`
/// for the probability rather than a stand-in.
pub struct ParakeetRecognizer {
    engine: SttEngine,
    model_dir: PathBuf,
}

impl ParakeetRecognizer {
    pub fn new(engine: SttEngine, model_dir: impl Into<PathBuf>) -> Self {
        Self {
            engine,
            model_dir: model_dir.into(),
        }
    }
}

impl SpeechRecognizer for ParakeetRecognizer {
    fn descriptor(&self) -> RecognizerDescriptor {
        RecognizerDescriptor {
            id: RecognizerKind::Parakeet.id().to_string(),
            display_name: RecognizerKind::Parakeet.display_name().to_string(),
            model: "parakeet-tdt-0.6b-v3".to_string(),
            capabilities: RecognizerKind::Parakeet.capabilities(),
        }
    }

    fn transcribe(&self, request: RecognitionRequest<'_>) -> Result<Recognition, RecognitionError> {
        if !RecognizerKind::Parakeet.compiled_in() {
            return Err(RecognitionError::Unavailable {
                engine: RecognizerKind::Parakeet.id().to_string(),
            });
        }
        if !crate::capture::parakeet::ModelFiles::is_installed_in(&self.model_dir) {
            return Err(RecognitionError::ModelMissing {
                engine: RecognizerKind::Parakeet.id().to_string(),
            });
        }
        let (text, diagnostics) = self
            .engine
            .transcribe_parakeet(&self.model_dir, request.samples)
            .map_err(|err| RecognitionError::Failed(err.to_string()))?;

        Ok(Recognition {
            spans: vec![RecognizedSpan {
                text: text.clone(),
                start_seconds: None,
                end_seconds: None,
                // Declared, not omitted by accident: this engine has no
                // no-speech probability to report.
                no_speech_prob: None,
            }],
            text,
            language: Some("en".to_string()),
            timing: RecognitionTiming {
                decode_ms: diagnostics.transcription_latency_ms,
                ..RecognitionTiming::default()
            },
        })
    }
}

/// Which engine to build, and where its model is.
///
/// A closed set rather than a string the caller invents: an engine name that
/// does not resolve is a run that silently measures the default one and labels
/// the result with something else.
///
/// There is no variant carrying a credential. A cloud adapter, when there is
/// one, reads its key from the OS credential store (`providers::secrets`).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case", tag = "engine")]
pub enum RecognizerChoice {
    Whisper {
        /// Absolute path to a `.bin` model file.
        model_path: String,
    },
    Parakeet {
        /// Directory holding the Parakeet ONNX files.
        model_dir: String,
    },
}

impl RecognizerChoice {
    pub fn kind(&self) -> RecognizerKind {
        match self {
            Self::Whisper { .. } => RecognizerKind::Whisper,
            Self::Parakeet { .. } => RecognizerKind::Parakeet,
        }
    }

    /// Builds the recognizer, failing before any audio is read when the engine
    /// is not in this build or its model is not installed.
    pub fn build(
        &self,
        engine: SttEngine,
        decoding: WhisperDecodingConfig,
    ) -> Result<Box<dyn SpeechRecognizer>, RecognitionError> {
        let kind = self.kind();
        if !kind.compiled_in() {
            return Err(RecognitionError::Unavailable {
                engine: kind.id().to_string(),
            });
        }
        match self {
            Self::Whisper { model_path } => {
                if !Path::new(model_path).is_file() {
                    return Err(RecognitionError::ModelMissing {
                        engine: kind.id().to_string(),
                    });
                }
                Ok(Box::new(WhisperRecognizer::new(
                    engine, model_path, decoding,
                )))
            }
            Self::Parakeet { model_dir } => {
                let dir = PathBuf::from(model_dir);
                if !crate::capture::parakeet::ModelFiles::is_installed_in(&dir) {
                    return Err(RecognitionError::ModelMissing {
                        engine: kind.id().to_string(),
                    });
                }
                Ok(Box::new(ParakeetRecognizer::new(engine, dir)))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_descriptor_names_the_model_file_and_never_its_path() {
        let recognizer = WhisperRecognizer::new(
            SttEngine::new(),
            "/home/someone/.Vox/config/models/ggml-small.bin",
            WhisperDecodingConfig::default(),
        );
        let descriptor = recognizer.descriptor();
        assert_eq!(descriptor.model, "ggml-small.bin");
        assert!(!descriptor.model.contains('/'));
        assert!(!descriptor.model.contains("someone"));
        assert_eq!(descriptor.id, "whisper");
    }

    #[test]
    fn parakeet_declares_its_limits_in_its_descriptor() {
        let recognizer = ParakeetRecognizer::new(SttEngine::new(), "/models/parakeet");
        let capabilities = recognizer.descriptor().capabilities;
        assert!(!capabilities.no_speech_evidence);
        assert!(!capabilities.language.can_pin());
    }

    #[test]
    fn a_missing_model_fails_before_any_audio_is_read() {
        let choice = RecognizerChoice::Whisper {
            model_path: "/definitely/not/here/ggml-small.bin".into(),
        };
        let built = choice.build(SttEngine::new(), WhisperDecodingConfig::default());
        assert!(
            matches!(built, Err(RecognitionError::ModelMissing { .. })),
            "a run must not start without its model"
        );
    }

    #[test]
    fn transcribing_with_no_model_is_an_error_rather_than_empty_text() {
        // Empty text is indistinguishable from silence, and a benchmark would
        // score it as a total deletion rather than as a failed run.
        let recognizer = WhisperRecognizer::new(
            SttEngine::new(),
            "/definitely/not/here/ggml-small.bin",
            WhisperDecodingConfig::default(),
        );
        let result = recognizer.transcribe(RecognitionRequest {
            samples: &[0.0; 1_600],
            language: None,
            translate: false,
        });
        assert!(matches!(
            result,
            Err(RecognitionError::ModelMissing { .. })
        ));
    }

    #[test]
    fn a_choice_round_trips_through_json_without_room_for_a_secret() {
        let choice = RecognizerChoice::Parakeet {
            model_dir: "models/parakeet".into(),
        };
        let json = serde_json::to_string(&choice).expect("serialize");
        assert!(!json.contains("key"));
        assert!(!json.contains("token"));
        let back: RecognizerChoice = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(choice, back);
    }
}
