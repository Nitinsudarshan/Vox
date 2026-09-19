//! Deciding whether someone is speaking, independently of what they said.
//!
//! ## Why this is its own subsystem
//!
//! Whether a turn has ended is an acoustic question, and Vox was answering it
//! inside [`crate::meetings::segmenter`] as a side effect of buffering audio.
//! The algorithm was right; having no name for its states meant three things
//! were impossible. The state was not observable, so "how much of the wait
//! before a transcript line appears is the hangover rather than the decoder"
//! had no answer. It could not be reused, so a future voice surface would
//! have grown a second copy. And it looked like a property of transcription,
//! which invites the idea that a better model would finalize sooner — it would
//! not. A 400 ms hangover is the floor on how soon *any* word can be decoded,
//! and no decoder changes it.
//!
//! So the decision lives here and the buffering stays there. Same constants,
//! same comparisons, same transitions: this is an extraction, not a rewrite,
//! and the segmenter's existing tests are what hold it to that.
//!
//! ## No model, no network, no ASR
//!
//! Energy against an adaptive noise floor, frame continuity, and two
//! hysteresis counters. It runs when no speech model is installed, when the
//! decoder has failed, and when the queue is full — which is what makes it
//! usable later by things that are not transcription at all: a live
//! transcript's turn boundaries, barge-in, cancelling playback when someone
//! starts talking over it.
//!
//! An LLM is not used to decide that a sentence has ended, and should not be.
//! It is an acoustic control problem answered in milliseconds by arithmetic;
//! routing it through a model adds a network round trip and a cost to the one
//! decision in the pipeline that has to be instant.
//!
//! ## The states
//!
//! ```text
//!                    ┌──────────────────────────────────────┐
//!                    ▼                                      │
//!   ┌─────────┐  frame ≥ onset   ┌────────────────┐         │
//!   │ Silence │ ───────────────▶ │ PossibleSpeech │         │
//!   └─────────┘                  └───────┬────────┘         │
//!        ▲                    ONSET_FRAMES│ in a row        │
//!        │                                ▼                 │
//!        │                        ┌──────────────┐          │
//!        │                        │   Speaking   │ ◀────────┤ frame ≥ hold
//!        │                        └──────┬───────┘          │  (redeemed)
//!        │                 frame < hold  │                  │
//!        │                               ▼                  │
//!        │                       ┌───────────────┐          │
//!        │                       │  ProbableEnd  │ ─────────┘
//!        │                       └───────┬───────┘
//!        │      REDEMPTION_FRAMES below  │
//!        │                               ▼
//!        │                        ┌─────────────┐
//!        └─────────────────────── │  Finalized  │
//!                                 └─────────────┘
//! ```
//!
//! `Finalized` is reported for exactly the frame on which a turn closes and is
//! never observed afterwards — the next frame reads `Silence`. It is a
//! transition, not a resting place, and modelling it as a state is what lets a
//! consumer act on the edge without diffing two observations.

use serde::{Deserialize, Serialize};

/// Analysis frame. 20 ms is short enough to place a boundary tightly and long
/// enough that one plosive does not read as speech.
pub const FRAME_MS: usize = 20;

/// Consecutive above-threshold frames needed to open a turn.
pub const ONSET_FRAMES: usize = 3;

/// Silence tolerated inside a turn before it is considered finished.
pub const REDEMPTION_MS: usize = 400;
pub const REDEMPTION_FRAMES: usize = REDEMPTION_MS / FRAME_MS;

/// How many recent frames the adaptive noise floor is estimated over.
const NOISE_WINDOW_FRAMES: usize = 150; // 3 seconds

/// Fraction of the noise window treated as "the quiet part of the room".
const NOISE_PERCENTILE: f32 = 0.2;

/// How far above the floor a frame must sit to open a turn.
const ONSET_MARGIN: f32 = 0.008;

/// How far above the floor a frame must sit to keep one open. Lower than the
/// onset margin on purpose: it is much easier to stay in speech than to enter
/// it, which is what stops a trailing-off sentence from being clipped.
const HOLD_MARGIN: f32 = 0.0048;

/// Absolute floor under which nothing counts as speech regardless of how quiet
/// the room is. Without it, a perfectly silent input makes its own noise floor
/// zero and every rounding artefact becomes a turn.
const MIN_ONSET_ENERGY: f32 = 0.006;

/// Where the speaker is, as far as the acoustics can tell.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SpeechState {
    /// Nothing above the room.
    Silence,
    /// Something crossed the onset threshold but has not held long enough to
    /// be speech. A door, a cough and the first syllable of a sentence all
    /// look like this, which is why it is a state and not a decision.
    PossibleSpeech,
    /// A turn is open and the current frame is voiced.
    Speaking,
    /// A turn is open and has gone quiet. It may still be a breath — that is
    /// what [`REDEMPTION_FRAMES`] is for — so nothing is finalized yet.
    ProbableEnd,
    /// Reported for the one frame on which a turn closed.
    Finalized,
}

impl SpeechState {
    /// Whether a turn is currently open. `Finalized` is not open: the turn it
    /// describes has just ended.
    pub fn is_speaking(self) -> bool {
        matches!(self, SpeechState::Speaking | SpeechState::ProbableEnd)
    }

    pub fn key(self) -> &'static str {
        match self {
            Self::Silence => "silence",
            Self::PossibleSpeech => "possible_speech",
            Self::Speaking => "speaking",
            Self::ProbableEnd => "probable_end",
            Self::Finalized => "finalized",
        }
    }
}

/// Why a turn ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TurnEnd {
    /// Long enough quiet that the speaker had finished.
    Silence,
    /// The ceiling was reached and the turn was cut mid-speech. Whatever comes
    /// next continues the same sentence.
    Ceiling,
    /// Capture stopped with a turn still open.
    Flush,
}

/// What the current frame means for the turn.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrameOutcome {
    /// Still silence, or still only a possibility. Nothing is open.
    Waiting,
    /// This frame confirmed an onset: a turn is now open, and the
    /// [`ONSET_FRAMES`] frames that led here belong to it.
    Opened,
    /// Inside a turn, and this frame was voiced.
    Voiced,
    /// Inside a turn, and this frame was not — the hangover is running.
    Unvoiced,
    /// The hangover expired. The turn is over.
    Ended,
}

/// The acoustic turn detector.
///
/// Frame in, verdict out. Holds no audio: the caller owns the samples and
/// decides what to do with them, which is what lets the same detector serve a
/// meeting segmenter, a live transcript and, later, a voice surface that has
/// no transcript at all.
#[derive(Debug)]
pub struct SpeechStateMachine {
    state: SpeechState,
    consecutive_above: usize,
    consecutive_below: usize,
    noise_window: std::collections::VecDeque<f32>,
    /// Frames in the currently open turn, for the ceiling.
    frames_open: usize,
    /// Frames spent in `ProbableEnd` since the last voiced one — the hangover
    /// this turn has accumulated so far.
    unvoiced_run: usize,
}

impl Default for SpeechStateMachine {
    fn default() -> Self {
        Self::new()
    }
}

impl SpeechStateMachine {
    pub fn new() -> Self {
        Self {
            state: SpeechState::Silence,
            consecutive_above: 0,
            consecutive_below: 0,
            noise_window: std::collections::VecDeque::with_capacity(NOISE_WINDOW_FRAMES),
            frames_open: 0,
            unvoiced_run: 0,
        }
    }

    /// Where the speaker is, as of the last frame observed.
    pub fn state(&self) -> SpeechState {
        self.state
    }

    /// Frames in the open turn, including its hangover.
    pub fn frames_open(&self) -> usize {
        self.frames_open
    }

    /// Milliseconds of quiet accumulated since the last voiced frame.
    ///
    /// This is the part of a turn's finalization latency that no decoder can
    /// remove: it is time spent waiting to be sure the speaker has stopped.
    pub fn hangover_ms(&self) -> u64 {
        (self.unvoiced_run * FRAME_MS) as u64
    }

    /// Feeds one frame's RMS in and returns what it means.
    ///
    /// The caller is responsible for the audio; this only judges it.
    pub fn observe(&mut self, rms: f32) -> FrameOutcome {
        let (onset, hold) = self.thresholds();

        // The floor is made only of frames judged to be non-speech. Recording
        // every frame is what made a steady voice raise its own threshold:
        // after a few seconds of talking the "noise" estimate *is* the voice,
        // the hold threshold climbs above it, and the turn closes mid-sentence
        // and cannot re-open.
        //
        // The very first frame is recorded regardless, to seed the estimate. A
        // room with real background noise clears the absolute floor from its
        // opening frame, so with nothing seeded nothing would ever be recorded
        // and the whole recording would read as one unbroken turn.
        if !self.state.is_speaking() && (self.noise_window.is_empty() || rms < onset) {
            self.record_noise(rms);
        }

        if !self.state.is_speaking() {
            if rms >= onset {
                self.consecutive_above += 1;
                if self.consecutive_above >= ONSET_FRAMES {
                    self.open();
                    return FrameOutcome::Opened;
                }
                self.state = SpeechState::PossibleSpeech;
            } else {
                self.consecutive_above = 0;
                self.state = SpeechState::Silence;
            }
            return FrameOutcome::Waiting;
        }

        self.frames_open += 1;
        if rms >= hold {
            self.consecutive_below = 0;
            self.unvoiced_run = 0;
            self.state = SpeechState::Speaking;
            FrameOutcome::Voiced
        } else {
            self.consecutive_below += 1;
            self.unvoiced_run += 1;
            if self.consecutive_below >= REDEMPTION_FRAMES {
                self.state = SpeechState::Finalized;
                return FrameOutcome::Ended;
            }
            self.state = SpeechState::ProbableEnd;
            FrameOutcome::Unvoiced
        }
    }

    /// Whether the open turn has reached `max_frames` and must be cut.
    ///
    /// Asked by the caller rather than folded into [`Self::observe`] because
    /// the ceiling is about how long a decode may be, which is the caller's
    /// concern, not the speaker's.
    pub fn at_ceiling(&self, max_frames: usize) -> bool {
        self.state.is_speaking() && self.frames_open >= max_frames
    }

    /// Returns to silence after a turn was closed normally or flushed.
    pub fn close(&mut self) {
        self.state = SpeechState::Silence;
        self.consecutive_above = 0;
        self.consecutive_below = 0;
        self.frames_open = 0;
        self.unvoiced_run = 0;
    }

    /// Stays in the turn after a ceiling cut, carrying over the frames the
    /// caller kept back.
    ///
    /// The speaker has not stopped, so returning to `Silence` here would make
    /// the next syllable look like a new onset and cost it the three frames an
    /// onset takes to confirm.
    pub fn continue_after_ceiling(&mut self, frames_retained: usize) {
        self.state = SpeechState::Speaking;
        self.consecutive_above = 0;
        self.consecutive_below = 0;
        self.frames_open = frames_retained;
        self.unvoiced_run = 0;
    }

    fn open(&mut self) {
        self.state = SpeechState::Speaking;
        self.consecutive_above = 0;
        self.consecutive_below = 0;
        self.unvoiced_run = 0;
        // The frames that confirmed the onset are real speech and the caller
        // has them in its pre-roll, so the turn starts already that long.
        self.frames_open = ONSET_FRAMES;
    }

    fn record_noise(&mut self, rms: f32) {
        if self.noise_window.len() == NOISE_WINDOW_FRAMES {
            self.noise_window.pop_front();
        }
        self.noise_window.push_back(rms);
    }

    /// Onset and hold thresholds for the current room.
    pub fn thresholds(&self) -> (f32, f32) {
        let floor = self.noise_floor();
        (
            (floor + ONSET_MARGIN).max(MIN_ONSET_ENERGY),
            (floor + HOLD_MARGIN).max(MIN_ONSET_ENERGY * 0.75),
        )
    }

    /// The quiet part of the recent past, as an RMS level.
    pub fn noise_floor(&self) -> f32 {
        if self.noise_window.is_empty() {
            return 0.0;
        }
        let mut sorted: Vec<f32> = self.noise_window.iter().copied().collect();
        sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        let count = ((sorted.len() as f32 * NOISE_PERCENTILE).ceil() as usize).max(1);
        sorted[..count].iter().sum::<f32>() / count as f32
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const QUIET: f32 = 0.0;
    const LOUD: f32 = 0.2;

    /// Frames of room, enough to fill the noise window.
    ///
    /// Every real recording gives the detector this before anyone speaks, and
    /// every segmenter test prepends it. Without it the machine is on its
    /// first frame, has no floor to compare against, and treats anything above
    /// the absolute minimum as a possibility — correct, and not what most of
    /// these tests are about. [`the_very_first_frame_has_no_floor_to_judge_against`]
    /// pins that case on its own.
    fn calibrated(room: f32) -> SpeechStateMachine {
        let mut machine = SpeechStateMachine::new();
        for _ in 0..NOISE_WINDOW_FRAMES {
            machine.observe(room);
        }
        machine
    }

    /// Runs frame energies through a calibrated machine and returns the state
    /// after each one.
    fn trace(room: f32, frames: &[f32]) -> Vec<SpeechState> {
        let mut machine = calibrated(room);
        frames
            .iter()
            .map(|rms| {
                let outcome = machine.observe(*rms);
                let state = machine.state();
                if outcome == FrameOutcome::Ended {
                    machine.close();
                }
                state
            })
            .collect()
    }

    #[test]
    fn the_detector_runs_with_no_model_no_network_and_no_transcript() {
        // The whole point of the separation: this is arithmetic over energy,
        // so it answers whether someone is speaking when the decoder has
        // failed, when the queue is full, and when no model is installed.
        let mut machine = SpeechStateMachine::new();
        assert_eq!(machine.state(), SpeechState::Silence);
        machine.observe(LOUD);
    }

    #[test]
    fn the_very_first_frame_has_no_floor_to_judge_against() {
        // A recording that opens mid-word calibrates against the word. The
        // absolute minimum is the only thing protecting the first frame, and
        // one frame is never enough to open a turn anyway.
        let mut machine = SpeechStateMachine::new();
        assert_eq!(machine.observe(0.009), FrameOutcome::Waiting);
        assert_eq!(machine.state(), SpeechState::PossibleSpeech);
        // By the next frame there is a floor, and steady room tone sits on it.
        machine.observe(0.009);
        assert_eq!(machine.state(), SpeechState::Silence);
    }

    #[test]
    fn silence_stays_silent() {
        let states = trace(QUIET, &[QUIET; 50]);
        assert!(states.iter().all(|s| *s == SpeechState::Silence));
    }

    #[test]
    fn a_single_loud_frame_is_only_a_possibility() {
        // A door, a cough and the first syllable of a sentence all look the
        // same for one frame, which is why the onset has to be confirmed.
        let states = trace(QUIET, &[QUIET, LOUD, QUIET, QUIET]);
        assert_eq!(states[1], SpeechState::PossibleSpeech);
        assert_eq!(states[2], SpeechState::Silence, "it did not hold");
    }

    #[test]
    fn an_onset_is_confirmed_by_holding_for_three_frames() {
        let states = trace(QUIET, &[QUIET, LOUD, LOUD, LOUD, LOUD]);
        assert_eq!(states[1], SpeechState::PossibleSpeech);
        assert_eq!(states[2], SpeechState::PossibleSpeech);
        assert_eq!(states[3], SpeechState::Speaking);
        assert_eq!(states[4], SpeechState::Speaking);
    }

    #[test]
    fn a_breath_inside_a_sentence_reads_as_probable_end_and_is_redeemed() {
        // Ten frames of quiet is 200 ms — half the hangover, so the turn must
        // survive it.
        let mut frames = vec![LOUD; 10];
        frames.extend(vec![QUIET; 10]);
        frames.extend(vec![LOUD; 10]);
        let states = trace(QUIET, &frames);

        assert_eq!(states[12], SpeechState::ProbableEnd);
        assert_eq!(states[19], SpeechState::ProbableEnd, "still inside the hangover");
        assert_eq!(states[25], SpeechState::Speaking, "and redeemed by more speech");
        assert!(!states.contains(&SpeechState::Finalized));
    }

    #[test]
    fn a_real_pause_finalizes_the_turn_exactly_when_the_hangover_expires() {
        let mut frames = vec![LOUD; 10];
        frames.extend(vec![QUIET; REDEMPTION_FRAMES + 5]);
        let states = trace(QUIET, &frames);

        // The onset takes three frames to confirm, so speech opens at index 2
        // and the quiet starts at index 10. The REDEMPTION_FRAMES-th quiet
        // frame is the one that closes it.
        let finalized = states
            .iter()
            .position(|s| *s == SpeechState::Finalized)
            .expect("the turn must end");
        assert_eq!(finalized, 10 + REDEMPTION_FRAMES - 1);
        // And it is an edge, not a resting place.
        assert_eq!(states[finalized + 1], SpeechState::Silence);
    }

    #[test]
    fn the_hangover_is_reported_so_its_cost_is_measurable() {
        // The question the audit could not answer: how much of the wait before
        // a line appears is this, rather than the decoder.
        let mut machine = calibrated(QUIET);
        for _ in 0..10 {
            machine.observe(LOUD);
        }
        assert_eq!(machine.state(), SpeechState::Speaking);
        assert_eq!(machine.hangover_ms(), 0);

        machine.observe(QUIET);
        machine.observe(QUIET);
        assert_eq!(machine.hangover_ms(), (2 * FRAME_MS) as u64);

        // Redeemed speech resets it: the wait only counts from the last thing
        // actually said.
        machine.observe(LOUD);
        assert_eq!(machine.hangover_ms(), 0);
    }

    #[test]
    fn continuous_speech_reaches_the_ceiling_and_carries_on_afterwards() {
        let mut machine = calibrated(QUIET);
        for _ in 0..100 {
            machine.observe(LOUD);
        }
        assert!(machine.at_ceiling(50));
        assert!(!machine.at_ceiling(500));

        // The speaker has not stopped, so the next syllable must not have to
        // re-confirm an onset.
        machine.continue_after_ceiling(10);
        assert_eq!(machine.state(), SpeechState::Speaking);
        assert_eq!(machine.frames_open(), 10);
        assert_eq!(machine.observe(LOUD), FrameOutcome::Voiced);
    }

    #[test]
    fn a_closed_turn_leaves_the_machine_ready_for_the_next_one() {
        let mut machine = calibrated(QUIET);
        for _ in 0..10 {
            machine.observe(LOUD);
        }
        machine.close();
        assert_eq!(machine.state(), SpeechState::Silence);
        assert_eq!(machine.frames_open(), 0);
        assert_eq!(machine.hangover_ms(), 0);
        assert!(!machine.at_ceiling(1));
    }

    #[test]
    fn steady_room_tone_calibrates_instead_of_reading_as_one_endless_turn() {
        // Constant hiss well above the absolute floor. Every frame sits at the
        // noise floor, so none of them clears the onset margin above it — and
        // the adaptive floor, not the absolute one, is what rejects this.
        let states = trace(0.009, &[0.009; 300]);
        assert!(
            states.iter().all(|s| *s == SpeechState::Silence),
            "background noise is not speech"
        );
    }

    #[test]
    fn speech_over_a_noisy_room_still_registers() {
        let states = trace(0.009, &[0.08; 30]);
        assert!(
            states.contains(&SpeechState::Speaking),
            "a raised voice over noise is still speech"
        );
    }

    #[test]
    fn a_quiet_voice_in_a_quiet_room_is_not_lost_to_the_absolute_floor() {
        // Well under conversational level, and well over silence. The absolute
        // floor exists to reject rounding artefacts, not soft speakers.
        let states = trace(QUIET, &[0.012; 30]);
        assert!(states.contains(&SpeechState::Speaking));
    }

    #[test]
    fn a_voice_does_not_raise_its_own_threshold_until_it_cuts_itself_off() {
        // The failure this guards: recording every frame into the noise floor
        // makes a steady voice the "noise", the hold threshold climbs above
        // it, and the turn closes mid-sentence and cannot re-open.
        let states = trace(QUIET, &[0.05; 400]);
        assert!(
            !states.contains(&SpeechState::Finalized),
            "a speaker who keeps talking must not be cut off by their own voice"
        );
    }

    #[test]
    fn staying_in_speech_is_easier_than_entering_it() {
        // Hysteresis: the hold threshold sits below the onset threshold, which
        // is what stops a trailing-off sentence from being clipped.
        let machine = SpeechStateMachine::new();
        let (onset, hold) = machine.thresholds();
        assert!(hold < onset, "hold {hold} must be under onset {onset}");
    }

    #[test]
    fn state_keys_are_stable_for_anything_that_records_them() {
        assert_eq!(SpeechState::Silence.key(), "silence");
        assert_eq!(SpeechState::ProbableEnd.key(), "probable_end");
        assert!(SpeechState::Speaking.is_speaking());
        assert!(SpeechState::ProbableEnd.is_speaking());
        assert!(!SpeechState::Finalized.is_speaking(), "the turn has ended");
        assert!(!SpeechState::PossibleSpeech.is_speaking());
    }
}
