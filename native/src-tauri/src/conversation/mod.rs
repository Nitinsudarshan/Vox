//! Who is talking, and who is allowed to interrupt whom.
//!
//! ## What this is
//!
//! A state machine and an event model. No audio, no model, no network, and
//! nothing wired to it. It exists so that the day Vox speaks and listens at
//! the same time, the hard parts — turn ownership, interruption, and the fact
//! that a laptop's microphone can hear its own speaker — are decisions already
//! made and tested rather than discovered under a deadline.
//!
//! Every input it takes already exists. [`crate::meetings::speech_state`]
//! says whether a person is speaking, from acoustics alone, with no ASR.
//! [`crate::tts::SpeechQueue`] can be cancelled mid-sentence. What was missing
//! was the thing between them that decides what those facts mean.
//!
//! ## What this is not
//!
//! Not a voice assistant, and not a step toward replacing the meeting
//! pipeline with a speech-to-speech model. A meeting transcript's value is
//! that every claim traces back to a span of audio
//! ([`crate::meetings::provenance`]); an end-to-end speech model has no text
//! turn to attach that to. The two are different products and this is the
//! first of them, not a migration away from it.
//!
//! No policy is implemented here either. Whether Vox should stop talking when
//! somebody says "mm-hm" is a product question with a real answer, and
//! [`BargeInPolicy`] is where it will go — deliberately a parameter rather
//! than a hard-coded threshold, because the answer differs per surface.
//!
//! ## The shape
//!
//! ```text
//!            ┌──────── user speaks ─────────┐
//!            ▼                              │
//!   Idle ─▶ Listening ─▶ UserSpeaking ─▶ Thinking ─▶ Speaking
//!            ▲              │                           │
//!            │        (they stop)                user speaks over it
//!            │                                          ▼
//!            └────────────── stop speaking ──────── Interrupted
//! ```
//!
//! ## The problem that makes naive barge-in fail
//!
//! On a laptop with no headphones, the microphone hears the speaker. A
//! detector that treats any speech during playback as an interruption will
//! interrupt Vox with Vox, every time, and the failure looks exactly like a
//! flaky microphone.
//!
//! Acoustic echo cancellation is the real fix and needs the playback signal as
//! a reference, which Vox does not have. So the guard is in the state model
//! instead: while Vox is speaking, an interruption must clear a higher bar and
//! hold for longer. That is honest about being a mitigation — it makes
//! talking over Vox harder, and on speakers it is the difference between
//! usable and unusable.

use serde::{Deserialize, Serialize};

use crate::meetings::speech_state::SpeechState;

/// Where a conversation is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConversationState {
    /// Not in a conversation. The microphone is not Vox's to hold.
    Idle,
    /// Listening, and nobody is speaking.
    Listening,
    /// The user has the turn.
    UserSpeaking,
    /// The user has finished and Vox is working out what to say. The
    /// microphone stays open: somebody who adds a sentence while Vox thinks
    /// has not finished, whatever the acoustics decided.
    Thinking,
    /// Vox has the turn.
    Speaking,
    /// The user started speaking while Vox had the turn. A transition rather
    /// than a resting place — the very next thing that happens is Vox stops.
    Interrupted,
}

impl ConversationState {
    pub fn key(self) -> &'static str {
        match self {
            Self::Idle => "idle",
            Self::Listening => "listening",
            Self::UserSpeaking => "user_speaking",
            Self::Thinking => "thinking",
            Self::Speaking => "speaking",
            Self::Interrupted => "interrupted",
        }
    }

    /// Whether the microphone should be open.
    ///
    /// True in every state but `Idle`, **including while Vox is speaking** —
    /// which is the whole of what "full duplex" means and the one property
    /// that cannot be added later without redesigning everything above it.
    pub fn microphone_open(self) -> bool {
        !matches!(self, Self::Idle)
    }

    /// Who owns the turn, if anybody.
    pub fn turn_owner(self) -> Option<TurnOwner> {
        match self {
            Self::Idle | Self::Listening => None,
            Self::UserSpeaking => Some(TurnOwner::User),
            Self::Thinking | Self::Speaking | Self::Interrupted => Some(TurnOwner::Vox),
        }
    }
}

/// Whose turn it is.
///
/// `Thinking` belongs to Vox even though it makes no sound: the user has
/// stopped and is waiting, so something arriving from the model is a
/// continuation rather than an interruption.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TurnOwner {
    User,
    Vox,
}

/// Something that happened.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConversationEvent {
    /// The user opened the conversation.
    Started,
    /// The user closed it.
    Stopped,
    /// A frame's worth of acoustic verdict, from
    /// [`crate::meetings::speech_state`]. The only audio-derived input, and
    /// deliberately the only one: a conversation that needs a transcript to
    /// know somebody is talking cannot react before the transcript exists.
    Speech(SpeechState),
    /// The model began producing an answer.
    ResponseStarted,
    /// The answer is finished and has been spoken.
    ResponseFinished,
    /// The user asked Vox to stop, by whatever means is not their voice.
    CancelRequested,
}

/// What the caller should do about it.
///
/// Returned rather than performed, so the machine stays a pure function and
/// the things it drives — a microphone, a synthesis queue, a model — are
/// testable separately and swappable.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConversationAction {
    /// Open the microphone.
    OpenMicrophone,
    /// Close it.
    CloseMicrophone,
    /// The user's turn ended; ask the model.
    BeginResponse,
    /// Stop speaking, now. [`crate::tts::SpeechQueue::cancel`].
    CancelSpeech,
}

/// When speech over Vox counts as an interruption.
///
/// A policy rather than a constant because the right answer differs by
/// surface and by hardware, and because it is a product question: whether
/// "mm-hm" should stop Vox mid-sentence has a real answer, and it is not this
/// module's to give.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BargeInPolicy {
    /// Consecutive `Speaking` observations required while Vox has the turn.
    ///
    /// Higher than one on purpose. Without acoustic echo cancellation the
    /// microphone hears Vox's own voice on laptop speakers, and a detector
    /// that fires on the first frame interrupts Vox with Vox — a failure that
    /// looks exactly like a flaky microphone.
    pub frames_to_interrupt: usize,
    /// Whether the user may interrupt at all. `false` makes Vox finish its
    /// sentence, which some surfaces want and a conversation does not.
    pub enabled: bool,
}

impl Default for BargeInPolicy {
    fn default() -> Self {
        Self {
            // Six 20 ms frames — 120 ms of held speech. Long enough that a
            // syllable of echo does not trigger it, short enough that a person
            // talking over Vox is not left waiting.
            frames_to_interrupt: 6,
            enabled: true,
        }
    }
}

impl BargeInPolicy {
    /// Barge-in off. What a surface wants when Vox must finish its sentence.
    pub fn disabled() -> Self {
        Self {
            enabled: false,
            ..Self::default()
        }
    }
}

/// The conversation's state, and the rules for changing it.
///
/// Pure: events in, actions out, no audio and no I/O. Everything it decides
/// can be tested by feeding it a sequence of verdicts.
#[derive(Debug)]
pub struct ConversationMachine {
    state: ConversationState,
    policy: BargeInPolicy,
    /// Consecutive `Speaking` frames observed while Vox had the turn.
    interrupt_run: usize,
}

impl Default for ConversationMachine {
    fn default() -> Self {
        Self::new(BargeInPolicy::default())
    }
}

impl ConversationMachine {
    pub fn new(policy: BargeInPolicy) -> Self {
        Self {
            state: ConversationState::Idle,
            policy,
            interrupt_run: 0,
        }
    }

    pub fn state(&self) -> ConversationState {
        self.state
    }

    pub fn policy(&self) -> BargeInPolicy {
        self.policy
    }

    /// Applies an event, returning what the caller should do.
    ///
    /// At most one action per event: a machine that returns a list invites a
    /// caller to perform them in the wrong order, and there has been no case
    /// yet where two are needed at once.
    pub fn handle(&mut self, event: ConversationEvent) -> Option<ConversationAction> {
        use ConversationEvent as E;
        use ConversationState as S;

        match (self.state, event) {
            // --- opening and closing ---
            (S::Idle, E::Started) => {
                self.state = S::Listening;
                Some(ConversationAction::OpenMicrophone)
            }
            (_, E::Stopped) => {
                let was_speaking = matches!(self.state, S::Speaking | S::Thinking);
                self.state = S::Idle;
                self.interrupt_run = 0;
                // Closing while Vox is mid-sentence has to stop the sentence,
                // or audio outlives the conversation it belongs to.
                Some(if was_speaking {
                    ConversationAction::CancelSpeech
                } else {
                    ConversationAction::CloseMicrophone
                })
            }
            (S::Idle, _) => None,

            // --- the user's turn ---
            (S::Listening, E::Speech(speech)) if speech.is_speaking() => {
                self.state = S::UserSpeaking;
                None
            }
            (S::UserSpeaking, E::Speech(SpeechState::Finalized)) => {
                self.state = S::Thinking;
                Some(ConversationAction::BeginResponse)
            }
            // A pause inside a sentence is not the end of a turn. The
            // acoustic detector already made that judgement, and second
            // guessing it here would put the hangover in two places.
            (S::UserSpeaking, E::Speech(_)) => None,

            // --- Vox's turn ---
            (S::Thinking, E::ResponseStarted) => {
                self.state = S::Speaking;
                None
            }
            (S::Thinking | S::Speaking, E::CancelRequested) => {
                self.state = S::Listening;
                self.interrupt_run = 0;
                Some(ConversationAction::CancelSpeech)
            }
            (S::Speaking, E::ResponseFinished) => {
                self.state = S::Listening;
                self.interrupt_run = 0;
                None
            }
            // Somebody adding a sentence while Vox thinks has not finished,
            // whatever the acoustics decided a moment ago. The turn goes back
            // to them and the answer in flight is abandoned.
            (S::Thinking, E::Speech(speech)) if speech.is_speaking() => {
                self.state = S::UserSpeaking;
                Some(ConversationAction::CancelSpeech)
            }

            // --- barge-in ---
            (S::Speaking, E::Speech(speech)) => {
                if !self.policy.enabled {
                    return None;
                }
                if speech.is_speaking() {
                    self.interrupt_run += 1;
                    if self.interrupt_run >= self.policy.frames_to_interrupt {
                        self.state = S::Interrupted;
                        self.interrupt_run = 0;
                        return Some(ConversationAction::CancelSpeech);
                    }
                } else {
                    // Not held. Almost always Vox's own voice arriving
                    // through the speakers, which is why the run has to be
                    // consecutive rather than cumulative.
                    self.interrupt_run = 0;
                }
                None
            }

            // `Interrupted` is a transition: the caller has been told to stop,
            // and the next thing is the user's turn.
            (S::Interrupted, E::Speech(speech)) if speech.is_speaking() => {
                self.state = S::UserSpeaking;
                None
            }
            (S::Interrupted, _) => {
                self.state = S::Listening;
                None
            }

            // Anything else is a message for a state that does not care.
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ConversationAction as A;
    use ConversationEvent as E;
    use ConversationState as S;

    fn started() -> ConversationMachine {
        let mut machine = ConversationMachine::default();
        assert_eq!(machine.handle(E::Started), Some(A::OpenMicrophone));
        machine
    }

    /// Drives the machine to `Speaking`, the state barge-in matters in.
    fn speaking() -> ConversationMachine {
        let mut machine = started();
        machine.handle(E::Speech(SpeechState::Speaking));
        machine.handle(E::Speech(SpeechState::Finalized));
        machine.handle(E::ResponseStarted);
        assert_eq!(machine.state(), S::Speaking);
        machine
    }

    #[test]
    fn a_whole_exchange_goes_round_and_comes_back_to_listening() {
        let mut machine = started();
        assert_eq!(machine.state(), S::Listening);

        machine.handle(E::Speech(SpeechState::Speaking));
        assert_eq!(machine.state(), S::UserSpeaking);

        assert_eq!(
            machine.handle(E::Speech(SpeechState::Finalized)),
            Some(A::BeginResponse)
        );
        assert_eq!(machine.state(), S::Thinking);

        machine.handle(E::ResponseStarted);
        assert_eq!(machine.state(), S::Speaking);

        machine.handle(E::ResponseFinished);
        assert_eq!(machine.state(), S::Listening);
    }

    #[test]
    fn the_microphone_stays_open_while_vox_speaks() {
        // This is the whole of what full duplex means, and the one property
        // that cannot be added later without redesigning everything above it.
        assert!(S::Speaking.microphone_open());
        assert!(S::Thinking.microphone_open());
        assert!(S::Interrupted.microphone_open());
        assert!(!S::Idle.microphone_open());
    }

    #[test]
    fn a_pause_inside_a_sentence_does_not_end_the_users_turn() {
        // The acoustic detector already made that judgement. Second guessing
        // it here would put the hangover in two places.
        let mut machine = started();
        machine.handle(E::Speech(SpeechState::Speaking));
        assert_eq!(machine.handle(E::Speech(SpeechState::ProbableEnd)), None);
        assert_eq!(machine.state(), S::UserSpeaking);
    }

    #[test]
    fn one_frame_of_speech_over_vox_is_not_an_interruption() {
        // Without echo cancellation the microphone hears Vox on laptop
        // speakers. A detector that fires on the first frame interrupts Vox
        // with Vox, and the failure looks like a flaky microphone.
        let mut machine = speaking();
        assert_eq!(machine.handle(E::Speech(SpeechState::Speaking)), None);
        assert_eq!(machine.state(), S::Speaking, "still Vox's turn");
    }

    #[test]
    fn held_speech_over_vox_stops_it() {
        let mut machine = speaking();
        let policy = machine.policy();
        for _ in 0..policy.frames_to_interrupt - 1 {
            assert_eq!(machine.handle(E::Speech(SpeechState::Speaking)), None);
        }
        assert_eq!(
            machine.handle(E::Speech(SpeechState::Speaking)),
            Some(A::CancelSpeech)
        );
        assert_eq!(machine.state(), S::Interrupted);
    }

    #[test]
    fn the_run_has_to_be_consecutive_so_echo_never_accumulates_into_one() {
        // Vox's own voice arrives in bursts with gaps. Cumulative counting
        // would eventually reach any threshold.
        let mut machine = speaking();
        let policy = machine.policy();
        for _ in 0..policy.frames_to_interrupt * 4 {
            assert_eq!(machine.handle(E::Speech(SpeechState::Speaking)), None);
            assert_eq!(machine.handle(E::Speech(SpeechState::Silence)), None);
        }
        assert_eq!(machine.state(), S::Speaking, "echo must not add up");
    }

    #[test]
    fn barge_in_can_be_turned_off_for_a_surface_that_wants_vox_to_finish() {
        let mut machine = ConversationMachine::new(BargeInPolicy::disabled());
        machine.handle(E::Started);
        machine.handle(E::Speech(SpeechState::Speaking));
        machine.handle(E::Speech(SpeechState::Finalized));
        machine.handle(E::ResponseStarted);

        for _ in 0..100 {
            assert_eq!(machine.handle(E::Speech(SpeechState::Speaking)), None);
        }
        assert_eq!(machine.state(), S::Speaking);
    }

    #[test]
    fn an_interruption_hands_the_turn_straight_back_to_the_user() {
        // Interrupted is a transition, not a resting place: somebody who
        // talked over Vox is mid-sentence, not waiting to be asked.
        let mut machine = speaking();
        let policy = machine.policy();
        for _ in 0..policy.frames_to_interrupt {
            machine.handle(E::Speech(SpeechState::Speaking));
        }
        assert_eq!(machine.state(), S::Interrupted);

        machine.handle(E::Speech(SpeechState::Speaking));
        assert_eq!(machine.state(), S::UserSpeaking);
    }

    #[test]
    fn speaking_while_vox_is_thinking_takes_the_turn_back_immediately() {
        // No echo to guard against — nothing is playing — and somebody adding
        // a sentence has not finished, whatever the acoustics decided.
        let mut machine = started();
        machine.handle(E::Speech(SpeechState::Speaking));
        machine.handle(E::Speech(SpeechState::Finalized));
        assert_eq!(machine.state(), S::Thinking);

        assert_eq!(
            machine.handle(E::Speech(SpeechState::Speaking)),
            Some(A::CancelSpeech)
        );
        assert_eq!(machine.state(), S::UserSpeaking);
    }

    #[test]
    fn asking_vox_to_stop_works_from_thinking_as_well_as_from_speaking() {
        for state in [S::Thinking, S::Speaking] {
            let mut machine = started();
            machine.handle(E::Speech(SpeechState::Speaking));
            machine.handle(E::Speech(SpeechState::Finalized));
            if state == S::Speaking {
                machine.handle(E::ResponseStarted);
            }
            assert_eq!(machine.handle(E::CancelRequested), Some(A::CancelSpeech));
            assert_eq!(machine.state(), S::Listening);
        }
    }

    #[test]
    fn closing_the_conversation_mid_sentence_stops_the_sentence() {
        // Otherwise audio outlives the conversation it belongs to.
        let mut machine = speaking();
        assert_eq!(machine.handle(E::Stopped), Some(A::CancelSpeech));
        assert_eq!(machine.state(), S::Idle);
        assert!(!machine.state().microphone_open());
    }

    #[test]
    fn closing_while_nobody_is_speaking_just_closes_the_microphone() {
        let mut machine = started();
        assert_eq!(machine.handle(E::Stopped), Some(A::CloseMicrophone));
        assert_eq!(machine.state(), S::Idle);
    }

    #[test]
    fn an_idle_machine_ignores_everything_but_being_started() {
        let mut machine = ConversationMachine::default();
        for event in [
            E::Speech(SpeechState::Speaking),
            E::ResponseStarted,
            E::ResponseFinished,
            E::CancelRequested,
        ] {
            assert_eq!(machine.handle(event), None);
            assert_eq!(machine.state(), S::Idle);
        }
    }

    #[test]
    fn thinking_belongs_to_vox_even_though_it_makes_no_sound() {
        // The user has stopped and is waiting, so something arriving from the
        // model is a continuation rather than an interruption.
        assert_eq!(S::Thinking.turn_owner(), Some(TurnOwner::Vox));
        assert_eq!(S::UserSpeaking.turn_owner(), Some(TurnOwner::User));
        assert_eq!(S::Listening.turn_owner(), None);
        assert_eq!(S::Idle.turn_owner(), None);
    }

    #[test]
    fn an_interruption_run_does_not_survive_the_turn_it_was_counted_in() {
        // Otherwise a near-interruption in one answer interrupts the next one
        // early, which reads as Vox refusing to finish a sentence.
        let mut machine = speaking();
        machine.handle(E::Speech(SpeechState::Speaking));
        machine.handle(E::ResponseFinished);
        assert_eq!(machine.state(), S::Listening);

        machine.handle(E::Speech(SpeechState::Speaking));
        machine.handle(E::Speech(SpeechState::Finalized));
        machine.handle(E::ResponseStarted);
        assert_eq!(
            machine.handle(E::Speech(SpeechState::Speaking)),
            None,
            "the previous turn's count must not carry over"
        );
    }

    #[test]
    fn state_keys_are_stable_for_anything_that_records_or_renders_them() {
        assert_eq!(S::UserSpeaking.key(), "user_speaking");
        assert_eq!(S::Interrupted.key(), "interrupted");
        let json = serde_json::to_string(&S::Speaking).expect("serialize");
        assert_eq!(json, "\"speaking\"");
    }
}
