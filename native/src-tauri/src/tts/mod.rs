//! Turning text into speech, behind one interface.
//!
//! ## What this is, and what it is not
//!
//! An interface and a chunker. There is **no provider behind it** — see
//! `docs/decisions.md` Decision 69 — and the honest shape for that is a
//! [`providers::NullTts`] that says so, rather than an adapter over something
//! absent.
//!
//! It is not a voice agent. There is no conversation state here, no barge-in,
//! no interruption policy. Those need a turn detector, a microphone that stays
//! open while audio plays, and decisions about who is allowed to talk over
//! whom, and building them before anything can produce a sound would be
//! designing against an imagined constraint.
//!
//! It is not voice cloning, and will not be. The thing worth optimizing first
//! is time to first audio and reliable cancellation; a system that sounds
//! exactly like you and takes four seconds to start is worse for every use
//! Vox has.
//!
//! ## The shape that decides latency
//!
//! ```text
//! text ─▶ phrases ─▶ synthesize ─▶ audio ─▶ playback
//!          ▲                                  │
//!          └──── the whole answer never waits ┘
//! ```
//!
//! A model writes an answer over several seconds. Synthesizing after it
//! finishes means time to first audio is *generation plus synthesis*;
//! synthesizing each phrase as it completes makes it *first phrase plus one
//! synthesis*, and the gap widens with every sentence. [`phrases`] is the
//! splitter that makes the second possible, and it is pure text, so it is
//! testable with no provider, no audio device and no model.
//!
//! ## Cancellation is a requirement, not a feature
//!
//! Anything that speaks has to be able to stop mid-sentence — because the user
//! asked, because they started talking, because the answer turned out to be
//! wrong. A design where stopping means waiting for the current utterance to
//! finish cannot be made interruptible later; it has to be built that way from
//! the start, which is why [`SpeechQueue`] has a cancel token rather than a
//! stop flag checked between items.

use serde::{Deserialize, Serialize};

pub mod phrases;
pub mod providers;
pub mod queue;

pub use phrases::split_into_phrases;
pub use queue::{SpeechQueue, SpeechSink};

/// What a synthesis engine can do.
///
/// Declared per provider rather than assumed, for the same reason
/// [`crate::capture::recognizer::RecognizerCapabilities`] is: a caller that
/// needs streaming has to be able to ask, and a provider that cannot stream
/// has to be able to say so rather than being discovered by trial.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TtsCapabilities {
    /// Produces a whole utterance's audio in one call.
    pub synthesize: bool,
    /// Emits audio before the utterance is finished, so playback can start
    /// sooner than synthesis ends.
    pub streaming: bool,
    /// More than one voice to choose between.
    pub voice_selection: bool,
    /// Runs on this machine with no network.
    pub local: bool,
    pub requires_network: bool,
}

impl TtsCapabilities {
    /// Nothing. What an absent provider reports.
    pub fn none() -> Self {
        Self {
            synthesize: false,
            streaming: false,
            voice_selection: false,
            local: false,
            requires_network: false,
        }
    }
}

/// A voice a provider offers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Voice {
    pub id: String,
    pub name: String,
    /// BCP-47 where the provider says, `None` where it does not.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub language: Option<String>,
}

/// Which engine, and what it can do.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TtsDescriptor {
    /// Stable id — `null`, and later `piper`, `system`, or whatever else.
    pub id: String,
    pub display_name: String,
    pub capabilities: TtsCapabilities,
    #[serde(default)]
    pub voices: Vec<Voice>,
}

/// One utterance's audio.
///
/// Carried as bytes with a stated format rather than decoded samples: a
/// provider that shells out to a binary produces a file, and decoding it here
/// only to re-encode it for playback would be work in service of a type.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SynthesizedAudio {
    pub bytes: Vec<u8>,
    /// A media type — `audio/wav`, `audio/mpeg`.
    pub media_type: String,
    /// Where the provider reports it. `None` rather than a guess.
    pub duration_ms: Option<u64>,
}

/// What to say.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Utterance {
    pub text: String,
    /// The voice to use, or `None` for the provider's default.
    pub voice_id: Option<String>,
}

impl Utterance {
    pub fn new(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            voice_id: None,
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum TtsError {
    /// No engine is installed. The ordinary state, not a fault.
    #[error("no speech synthesis voice is set up")]
    NotConfigured,

    #[error("the voice '{0}' is not one this engine offers")]
    UnknownVoice(String),

    #[error("speech synthesis was cancelled")]
    Cancelled,

    #[error("{0}")]
    Failed(String),
}

impl TtsError {
    /// Whether this is "nothing is set up" rather than "something broke".
    ///
    /// The difference matters at every call site: the first degrades to text
    /// silently, and the second is worth telling somebody about.
    pub fn is_unconfigured(&self) -> bool {
        matches!(self, TtsError::NotConfigured)
    }
}

/// An engine that turns text into audio.
///
/// `Send + Sync` because synthesis happens on a worker thread while the thing
/// producing the text runs somewhere else — which is the entire point of the
/// queue below.
pub trait TextToSpeech: Send + Sync {
    fn descriptor(&self) -> TtsDescriptor;

    /// Whether this engine can actually speak right now.
    ///
    /// Separate from [`Self::descriptor`] because a provider can be compiled
    /// in, configured, and still have no voice file on disk. A caller deciding
    /// whether to offer a Speak button wants this, not a list of capabilities.
    fn is_available(&self) -> bool;

    fn synthesize(&self, utterance: &Utterance) -> Result<SynthesizedAudio, TtsError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_absent_provider_is_a_state_rather_than_a_fault() {
        // Vox works fine without a voice, so "not set up" must not read as an
        // error at every call site that has to decide whether to complain.
        assert!(TtsError::NotConfigured.is_unconfigured());
        assert!(!TtsError::Failed("broke".into()).is_unconfigured());
        assert!(!TtsError::Cancelled.is_unconfigured());
    }

    #[test]
    fn capabilities_are_declared_so_a_caller_can_ask_before_relying() {
        let none = TtsCapabilities::none();
        assert!(!none.synthesize);
        assert!(!none.streaming);
        let json = serde_json::to_string(&none).expect("serialize");
        let back: TtsCapabilities = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(none, back);
    }

    #[test]
    fn audio_reports_a_duration_only_where_the_provider_gives_one() {
        // No estimating from byte count: a guessed duration is the same class
        // of invention as a guessed confidence.
        let audio = SynthesizedAudio {
            bytes: vec![0; 1024],
            media_type: "audio/wav".into(),
            duration_ms: None,
        };
        assert_eq!(audio.duration_ms, None);
    }
}
