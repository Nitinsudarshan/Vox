//! The engines behind [`TextToSpeech`], of which there are currently none.
//!
//! `NullTts` is not a placeholder to be replaced by a real provider — it is
//! the correct answer to "what should Vox do when no voice is set up", which
//! is the state of every fresh install and will remain the common one. It says
//! [`TtsError::NotConfigured`], every caller degrades to text, and nothing
//! pretends.
//!
//! Adding a real one means implementing [`TextToSpeech`] here, declaring what
//! it can do in its descriptor, and nothing else: the queue, the phrase
//! splitter and the cancellation path are provider-agnostic and already
//! tested. `docs/decisions.md` Decisions 51–56 hold the research from the last
//! time a local voice was set up — a managed install location, a pinned
//! release with a checksum, an atomic install that cannot damage a working
//! one, and a self-test through the production provider before reporting
//! ready. That reasoning survived the code; a second attempt should start
//! from it rather than from scratch.

use super::{
    SynthesizedAudio, TextToSpeech, TtsCapabilities, TtsDescriptor, TtsError, Utterance,
};

/// No voice is set up.
///
/// The ordinary state, reported as a state rather than as a fault.
#[derive(Debug, Clone, Copy, Default)]
pub struct NullTts;

impl TextToSpeech for NullTts {
    fn descriptor(&self) -> TtsDescriptor {
        TtsDescriptor {
            id: "null".to_string(),
            display_name: "No voice".to_string(),
            capabilities: TtsCapabilities::none(),
            voices: Vec::new(),
        }
    }

    fn is_available(&self) -> bool {
        false
    }

    fn synthesize(&self, _utterance: &Utterance) -> Result<SynthesizedAudio, TtsError> {
        Err(TtsError::NotConfigured)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_unconfigured_vox_says_so_rather_than_failing_obscurely() {
        let provider = NullTts;
        assert!(!provider.is_available());

        let error = provider
            .synthesize(&Utterance::new("anything"))
            .expect_err("there is no voice");
        assert!(
            error.is_unconfigured(),
            "a fresh install is not a malfunction"
        );
    }

    #[test]
    fn it_claims_no_capability_it_does_not_have() {
        let capabilities = NullTts.descriptor().capabilities;
        assert!(!capabilities.synthesize);
        assert!(!capabilities.streaming);
        assert!(!capabilities.voice_selection);
        assert!(NullTts.descriptor().voices.is_empty());
    }
}
