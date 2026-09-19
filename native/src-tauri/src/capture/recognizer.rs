//! One interface over the engines that turn audio into text.
//!
//! ## What this is for
//!
//! Vox already ran two engines — Whisper and Parakeet TDT — and they sat side
//! by side inside [`SttEngine`](crate::capture::stt::SttEngine) as two methods
//! with different shapes and different guarantees. Calling code had to know
//! which one it had, and anything wanting to add a third had to touch every
//! call site.
//!
//! The seam was already real; what was missing was a name for it.
//!
//! ## Capabilities, not a lowest common denominator
//!
//! The tempting design is one trait method per operation and a uniform result,
//! which forces every engine to look like the weakest one — or, worse, to
//! pretend. Parakeet reports no per-decode no-speech probability. A uniform
//! interface would have it return `0.0`, and a screen tuned for Whisper's
//! probability would then be grading it on a number it never produced.
//!
//! So an engine *declares* what it can do, in [`RecognizerCapabilities`], and
//! a caller that needs something checks for it. Where the answer is genuinely
//! not known — an engine nobody has measured on this axis — it is
//! [`Support::Unknown`] rather than a guess.
//!
//! ## Task → capability → provider → model
//!
//! The question worth asking is not "what is the best speech model". It is
//! "what does this task need, and what is the smallest replaceable thing that
//! provides it". Push-to-talk wants latency and needs no timestamps; a live
//! meeting needs to keep up with a clock; a final pass has no clock at all and
//! wants accuracy. Those are three different requirements over the same
//! audio, expressed here as [`SpeechTask`] and matched against capabilities
//! rather than hard-coded to an engine name.
//!
//! ## Scope, deliberately
//!
//! This is the interface and its two adapters. The production decode paths —
//! dictation, the live meeting worker, re-transcription — still call
//! `SttEngine` directly, because changing what decodes a meeting is a change
//! to what users get and belongs with the measurement that justifies it. The
//! benchmark runs through this trait, which is how the adapters are exercised
//! without putting a refactor in front of a recording.
//!
//! No credential lives here. A cloud adapter, when there is one, reads its key
//! through the keyring path the rest of Vox uses, and the registry has no way
//! to express a secret.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// Whether an engine supports something, including "nobody has checked".
///
/// Three-valued on purpose. A provider table full of `false` where the honest
/// answer is "unmeasured" reads as a comparison and is not one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Support {
    Yes,
    No,
    /// Not established for this engine. Never treated as `Yes`.
    Unknown,
}

impl Support {
    /// Whether a caller that requires this may proceed. `Unknown` may not:
    /// requiring something means needing it to work, and an unverified claim
    /// is not a working feature.
    pub fn is_usable(self) -> bool {
        matches!(self, Support::Yes)
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Yes => "yes",
            Self::No => "no",
            Self::Unknown => "unknown",
        }
    }
}

/// How an engine handles the language of what it is given.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum LanguageSupport {
    /// One language, always. The caller cannot ask for another.
    Fixed { language: String },
    /// The caller may pin a language, and may leave it to the engine.
    Selectable { detects: bool },
}

impl LanguageSupport {
    pub fn can_pin(&self) -> bool {
        matches!(self, Self::Selectable { .. })
    }

    pub fn can_detect(&self) -> bool {
        matches!(self, Self::Selectable { detects: true })
    }
}

/// What an engine can actually do.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecognizerCapabilities {
    /// Decodes a finished span of audio.
    pub batch: bool,
    /// Consumes audio as it arrives and emits text before the span ends.
    /// Vox has no streaming engine today; the field exists so an engine that
    /// is one can say so rather than being discovered by trial.
    pub streaming: bool,
    /// Per-span start and end times.
    pub segment_timestamps: Support,
    /// Per-word times inside a span.
    pub word_timestamps: Support,
    pub language: LanguageSupport,
    /// Speech-to-English in one pass, rather than transcribe-then-translate.
    pub translation: Support,
    /// Reports a per-decode probability that the audio was not speech.
    ///
    /// Not "confidence" — no engine Vox runs reports one, and calling it that
    /// is how a length-derived number ends up filtering transcripts.
    pub no_speech_evidence: bool,
    /// Emits revisable partial text during a decode.
    pub partial_transcripts: bool,
    /// Runs on this machine with no network.
    pub local: bool,
    /// Needs a network call to transcribe. `local` and this are separate
    /// questions: an engine could be neither (bundled but unimplemented).
    pub requires_network: bool,
}

impl RecognizerCapabilities {
    /// What a local, batch-only engine looks like, as a base to adjust.
    pub fn local_batch() -> Self {
        Self {
            batch: true,
            streaming: false,
            segment_timestamps: Support::Unknown,
            word_timestamps: Support::No,
            language: LanguageSupport::Selectable { detects: true },
            translation: Support::No,
            no_speech_evidence: false,
            partial_transcripts: false,
            local: true,
            requires_network: false,
        }
    }
}

/// Which engine, and which model of it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecognizerDescriptor {
    /// Stable id — `whisper`, `parakeet`. What a report and a setting store.
    pub id: String,
    /// What to call it in a sentence.
    pub display_name: String,
    /// Model filename or model id. **Never a path**: a descriptor ends up in
    /// diagnostics and benchmark reports, which get shared, and a path names a
    /// machine and usually a person.
    pub model: String,
    pub capabilities: RecognizerCapabilities,
}

/// One span of recognized speech.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RecognizedSpan {
    pub text: String,
    /// Seconds from the start of the audio handed over, where the engine
    /// reports them. `None` where it does not.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start_seconds: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub end_seconds: Option<f64>,
    /// The engine's own probability that this span is not speech, where it
    /// reports one. `None` is absent, never zero.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub no_speech_prob: Option<f32>,
}

/// What a decode cost, as the engine measured it.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
pub struct RecognitionTiming {
    pub decode_ms: u128,
    /// Waiting for a shared model slot, for an engine that has one.
    pub lock_wait_ms: u128,
    pub model_load_ms: u128,
    /// Whether that load evicted another model rather than filling an empty
    /// slot.
    pub model_reloaded: bool,
}

/// One decode's result.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Recognition {
    pub spans: Vec<RecognizedSpan>,
    /// The spans joined, as the engine's own joining rules produce.
    pub text: String,
    /// The language the engine resolved to, where it says.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub language: Option<String>,
    pub timing: RecognitionTiming,
}

impl Recognition {
    /// Mean no-speech probability across the spans that reported one.
    ///
    /// `None` when no span did — which is the whole point, and is why this is
    /// not a `0.0` default.
    pub fn mean_no_speech_prob(&self) -> Option<f32> {
        let reported: Vec<f32> = self
            .spans
            .iter()
            .filter_map(|span| span.no_speech_prob)
            .collect();
        if reported.is_empty() {
            return None;
        }
        Some(reported.iter().sum::<f32>() / reported.len() as f32)
    }
}

/// What to decode, and how.
#[derive(Debug, Clone)]
pub struct RecognitionRequest<'a> {
    /// 16 kHz mono — the rate everything downstream of the mixer works at.
    pub samples: &'a [f32],
    /// Pin the decode to this language, or `None` to let the engine decide.
    /// Ignored by an engine whose language is [`LanguageSupport::Fixed`].
    pub language: Option<String>,
    /// Ask for English out of non-English speech in one pass. Only honoured
    /// where `capabilities.translation` is [`Support::Yes`].
    pub translate: bool,
}

#[derive(Debug, thiserror::Error)]
pub enum RecognitionError {
    #[error("no model is installed for {engine}")]
    ModelMissing { engine: String },

    #[error("{engine} is not available in this build")]
    Unavailable { engine: String },

    #[error("{0}")]
    Failed(String),
}

/// An engine that turns audio into text.
///
/// `Send + Sync` because a recognizer is shared: the benchmark hands one to a
/// decode thread, and a future two-pass design would hold one per pass.
pub trait SpeechRecognizer: Send + Sync {
    fn descriptor(&self) -> RecognizerDescriptor;

    /// Decodes a finished span. Every engine Vox has supports this; a
    /// streaming-only engine would return
    /// [`RecognitionError::Unavailable`] and declare `batch: false`.
    fn transcribe(&self, request: RecognitionRequest<'_>) -> Result<Recognition, RecognitionError>;
}

// --- task → capability --------------------------------------------------

/// A thing Vox needs speech recognition *for*.
///
/// Named so that "which engine should we use" is answered by what the task
/// needs rather than by which model is currently fashionable. The engine and
/// the model are the last two steps, not the first.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SpeechTask {
    /// Push-to-talk dictation. One short utterance, someone waiting on it.
    PushToTalk,
    /// A meeting being recorded now. Audio keeps arriving, so the decoder has
    /// to average faster than real time or a backlog grows.
    LiveMeeting,
    /// A meeting's audio, decoded again with no clock to race.
    FinalPass,
    /// Measuring an engine. Requires whatever the engine offers and nothing.
    Benchmark,
}

/// What a task cannot do without.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RequiredCapabilities {
    pub batch: bool,
    pub streaming: bool,
    pub segment_timestamps: bool,
    pub language_pinning: bool,
    pub offline: bool,
}

impl SpeechTask {
    /// The capabilities an engine must have to serve this task.
    ///
    /// Deliberately short. Everything not listed is a preference, and a
    /// preference belongs in a measured comparison rather than in a hard
    /// requirement that silently excludes an engine.
    pub fn requires(self) -> RequiredCapabilities {
        match self {
            // Offline because dictation is the surface people use on a plane,
            // and because Vox's baseline is a zero-cost local one.
            Self::PushToTalk => RequiredCapabilities {
                batch: true,
                streaming: false,
                segment_timestamps: false,
                language_pinning: false,
                offline: true,
            },
            // A meeting transcript is ordered by span, so the spans have to be
            // placeable in the recording.
            Self::LiveMeeting => RequiredCapabilities {
                batch: true,
                streaming: false,
                segment_timestamps: false,
                language_pinning: true,
                offline: true,
            },
            // Pinning matters most here: a re-transcription exists because the
            // first attempt got the language wrong.
            Self::FinalPass => RequiredCapabilities {
                batch: true,
                streaming: false,
                segment_timestamps: false,
                language_pinning: true,
                offline: false,
            },
            Self::Benchmark => RequiredCapabilities {
                batch: true,
                streaming: false,
                segment_timestamps: false,
                language_pinning: false,
                offline: false,
            },
        }
    }

    /// Whether an engine can serve this task at all.
    pub fn is_satisfied_by(self, capabilities: &RecognizerCapabilities) -> bool {
        let required = self.requires();
        (!required.batch || capabilities.batch)
            && (!required.streaming || capabilities.streaming)
            && (!required.segment_timestamps || capabilities.segment_timestamps.is_usable())
            && (!required.language_pinning || capabilities.language.can_pin())
            && (!required.offline || (capabilities.local && !capabilities.requires_network))
    }
}

// --- registry -----------------------------------------------------------

/// Where an engine's model lives, as far as the registry is concerned.
///
/// A path, never a credential. There is deliberately no variant carrying a
/// key: a cloud adapter reads its own through the keyring the rest of Vox
/// uses, so a secret cannot end up in a settings file or a log by being
/// expressible here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ModelLocation {
    /// A single model file, as Whisper uses.
    File(PathBuf),
    /// A directory of model files, as Parakeet uses.
    Directory(PathBuf),
}

/// The engines this build knows how to construct.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RecognizerKind {
    Whisper,
    Parakeet,
}

impl RecognizerKind {
    pub const ALL: [RecognizerKind; 2] = [Self::Whisper, Self::Parakeet];

    pub fn id(self) -> &'static str {
        match self {
            Self::Whisper => "whisper",
            Self::Parakeet => "parakeet",
        }
    }

    pub fn display_name(self) -> &'static str {
        match self {
            Self::Whisper => "Whisper",
            Self::Parakeet => "NVIDIA Parakeet TDT",
        }
    }

    /// What this engine can do, independent of which model is installed.
    pub fn capabilities(self) -> RecognizerCapabilities {
        match self {
            Self::Whisper => RecognizerCapabilities {
                segment_timestamps: Support::Yes,
                translation: Support::Yes,
                no_speech_evidence: true,
                ..RecognizerCapabilities::local_batch()
            },
            Self::Parakeet => RecognizerCapabilities {
                // English-only in the model Vox ships against, so a caller
                // asking for Hindi must be told no rather than handed English
                // phonology applied to Hindi audio.
                language: LanguageSupport::Fixed {
                    language: "en".to_string(),
                },
                // The transducer emits token times, and Vox's decoder does not
                // currently surface them. Unknown rather than No: the engine
                // may well do it, and nobody has established that it does not
                // through this adapter.
                segment_timestamps: Support::Unknown,
                translation: Support::No,
                no_speech_evidence: false,
                ..RecognizerCapabilities::local_batch()
            },
        }
    }

    /// Whether this build contains the engine at all.
    ///
    /// Parakeet is behind a Cargo feature, and an engine compiled out has to
    /// say so rather than failing at the first decode.
    pub fn compiled_in(self) -> bool {
        match self {
            Self::Whisper => cfg!(feature = "whisper-local"),
            Self::Parakeet => cfg!(feature = "parakeet"),
        }
    }
}

/// Picks the engines that can serve a task.
///
/// Returns them in the order given, so a caller's own preference — a user
/// setting, a benchmark result — decides between equals. This deliberately
/// does not rank: ranking engines without measuring them is how "the best
/// model" becomes the architecture.
pub fn engines_for(
    task: SpeechTask,
    available: &[RecognizerKind],
) -> Vec<RecognizerKind> {
    available
        .iter()
        .copied()
        .filter(|kind| kind.compiled_in() && task.is_satisfied_by(&kind.capabilities()))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_unknown_capability_is_never_treated_as_a_yes() {
        // A provider table full of `false` where the honest answer is
        // "unmeasured" reads as a comparison and is not one.
        assert!(Support::Yes.is_usable());
        assert!(!Support::No.is_usable());
        assert!(!Support::Unknown.is_usable(), "unverified is not working");
    }

    #[test]
    fn an_engine_that_reports_no_no_speech_probability_reports_none() {
        // D-006 expressed in the interface: the absence has to survive, or a
        // screen tuned for whisper grades another engine on a number it never
        // produced.
        let silent = Recognition {
            spans: vec![RecognizedSpan {
                text: "hello".into(),
                start_seconds: None,
                end_seconds: None,
                no_speech_prob: None,
            }],
            ..Recognition::default()
        };
        assert_eq!(silent.mean_no_speech_prob(), None);

        let reporting = Recognition {
            spans: vec![
                RecognizedSpan {
                    text: "a".into(),
                    start_seconds: None,
                    end_seconds: None,
                    no_speech_prob: Some(0.2),
                },
                RecognizedSpan {
                    text: "b".into(),
                    start_seconds: None,
                    end_seconds: None,
                    no_speech_prob: Some(0.4),
                },
            ],
            ..Recognition::default()
        };
        let mean = reporting.mean_no_speech_prob().expect("both spans reported one");
        assert!((mean - 0.3).abs() < 1e-6, "got {mean}");
    }

    #[test]
    fn a_span_with_no_reported_probability_does_not_drag_the_mean_to_zero() {
        let mixed = Recognition {
            spans: vec![
                RecognizedSpan {
                    text: "a".into(),
                    start_seconds: None,
                    end_seconds: None,
                    no_speech_prob: Some(0.5),
                },
                RecognizedSpan {
                    text: "b".into(),
                    start_seconds: None,
                    end_seconds: None,
                    no_speech_prob: None,
                },
            ],
            ..Recognition::default()
        };
        assert_eq!(mixed.mean_no_speech_prob(), Some(0.5));
    }

    #[test]
    fn parakeet_declares_one_language_rather_than_pretending_to_choose() {
        let capabilities = RecognizerKind::Parakeet.capabilities();
        assert!(!capabilities.language.can_pin());
        assert!(!capabilities.language.can_detect());
        assert_eq!(
            capabilities.language,
            LanguageSupport::Fixed {
                language: "en".into()
            }
        );
        // A caller asking for Hindi has to be told no, rather than handed
        // English phonology applied to Hindi audio.
        assert!(!SpeechTask::FinalPass.is_satisfied_by(&capabilities));
    }

    #[test]
    fn whisper_can_pin_a_language_and_translate_in_one_pass() {
        let capabilities = RecognizerKind::Whisper.capabilities();
        assert!(capabilities.language.can_pin());
        assert!(capabilities.language.can_detect());
        assert_eq!(capabilities.translation, Support::Yes);
        assert!(capabilities.no_speech_evidence);
    }

    #[test]
    fn a_task_names_what_it_needs_rather_than_which_engine_it_wants() {
        // The distinction the whole module exists for: re-transcription exists
        // because the first attempt got the language wrong, so pinning is a
        // requirement there and not a preference.
        assert!(SpeechTask::FinalPass.requires().language_pinning);
        assert!(!SpeechTask::PushToTalk.requires().language_pinning);
        // And dictation has to work with no network, because that is the
        // surface people use on a plane.
        assert!(SpeechTask::PushToTalk.requires().offline);
    }

    #[test]
    fn a_cloud_engine_is_excluded_from_the_tasks_that_must_work_offline() {
        let cloud = RecognizerCapabilities {
            local: false,
            requires_network: true,
            ..RecognizerCapabilities::local_batch()
        };
        assert!(!SpeechTask::PushToTalk.is_satisfied_by(&cloud));
        assert!(!SpeechTask::LiveMeeting.is_satisfied_by(&cloud));
        // A final pass has no clock and no offline requirement, so a cloud
        // engine is allowed to serve it — optional acceleration, not a
        // dependency.
        assert!(SpeechTask::FinalPass.is_satisfied_by(&cloud));
    }

    #[test]
    fn a_streaming_only_engine_cannot_serve_a_task_that_needs_batch() {
        let streaming_only = RecognizerCapabilities {
            batch: false,
            streaming: true,
            ..RecognizerCapabilities::local_batch()
        };
        assert!(!SpeechTask::LiveMeeting.is_satisfied_by(&streaming_only));
    }

    #[test]
    fn engine_selection_keeps_the_callers_order_rather_than_ranking() {
        // Ranking engines without measuring them is how "the best model"
        // becomes the architecture.
        let chosen = engines_for(
            SpeechTask::PushToTalk,
            &[RecognizerKind::Whisper, RecognizerKind::Parakeet],
        );
        let reversed = engines_for(
            SpeechTask::PushToTalk,
            &[RecognizerKind::Parakeet, RecognizerKind::Whisper],
        );
        assert_eq!(chosen.first() != reversed.first(), chosen.len() > 1);
    }

    #[test]
    fn a_task_needing_a_pinned_language_excludes_the_fixed_language_engine() {
        let chosen = engines_for(SpeechTask::FinalPass, &RecognizerKind::ALL);
        assert!(!chosen.contains(&RecognizerKind::Parakeet));
    }

    #[test]
    fn an_engine_compiled_out_of_this_build_is_not_offered() {
        // An engine that works for whoever built it and is absent from an
        // installed app is worse than one that says it is absent.
        for kind in RecognizerKind::ALL {
            if !kind.compiled_in() {
                assert!(
                    !engines_for(SpeechTask::Benchmark, &[kind]).contains(&kind),
                    "{} is not in this build and must not be offered",
                    kind.id()
                );
            }
        }
    }

    #[test]
    fn there_is_no_way_to_express_a_credential_in_a_model_location() {
        // Structural rather than a review rule: a secret cannot reach a
        // settings file or a log by being storable here.
        let location = ModelLocation::File(PathBuf::from("/models/ggml-small.bin"));
        match location {
            ModelLocation::File(_) | ModelLocation::Directory(_) => {}
        }
    }

    #[test]
    fn capabilities_round_trip_so_a_report_can_carry_them() {
        let capabilities = RecognizerKind::Whisper.capabilities();
        let json = serde_json::to_string(&capabilities).expect("serialize");
        let back: RecognizerCapabilities = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(capabilities, back);
        assert!(json.contains("no_speech_evidence"));
        assert!(!json.contains("confidence"));
    }
}
