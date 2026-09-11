//! Greedy TDT decoding.
//!
//! Parakeet is a Token-and-Duration Transducer. An ordinary RNN-T walks the
//! encoder output one frame at a time and emits a token or a blank at each;
//! TDT adds a second head that predicts *how many frames to skip* after the
//! emission. That is where its speed comes from — most of a recording is
//! skipped rather than stepped through — and it is also the part that is easy
//! to get wrong, because a duration of zero and a blank advance the clock for
//! different reasons.
//!
//! So the loop lives here, pure, over a [`TransducerStep`] the caller
//! implements. The ONNX session is one implementation of that trait; the tests
//! below are another, handing the loop scripted logits. Everything that can be
//! wrong about the algorithm is therefore wrong in a test rather than in a
//! transcript, which matters for a model whose weights this repository cannot
//! reach to try.

/// One joint-network evaluation.
///
/// The implementor holds the decoder state; the loop tells it which frame and
/// which previous tokens, and gets back the vocabulary logits and how many
/// frames to advance.
pub trait TransducerStep {
    /// Evaluates the joint network at `frame`, given everything emitted so far.
    ///
    /// Returns the vocabulary logits and the predicted duration. A duration of
    /// `None` means the model has no duration head — an ordinary RNN-T — and
    /// the loop falls back to advancing one frame at a time.
    fn step(&mut self, frame: usize, emitted: &[usize]) -> Result<StepOutput, String>;

    /// Keeps the state produced by the last [`step`](Self::step).
    ///
    /// Called only when that step emitted a real token. A blank leaves the
    /// decoder state where it was: the prediction network is conditioned on
    /// the token history, and a blank adds nothing to that history.
    fn commit(&mut self);
}

/// What one joint evaluation produced.
#[derive(Debug, Clone, PartialEq)]
pub struct StepOutput {
    /// Logits over the vocabulary, including the blank.
    pub token_logits: Vec<f32>,
    /// Frames to advance, from the duration head.
    pub duration: Option<usize>,
}

/// One decoded token and where it was heard.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DecodedToken {
    pub id: usize,
    /// Encoder frame index. Multiply by the frame duration for seconds.
    pub frame: usize,
}

/// How the loop is bounded and what it treats as "emit nothing".
#[derive(Debug, Clone, Copy)]
pub struct DecodeConfig {
    pub blank: usize,
    /// Tokens the model may emit at a single frame before the loop advances
    /// anyway.
    ///
    /// The escape hatch for the one way greedy transducer decoding fails to
    /// terminate: a model that emits a real token, leaves the frame where it
    /// is, and is then asked again in a state that produces the same token.
    /// NeMo's own default is 10.
    pub max_tokens_per_frame: usize,
}

impl Default for DecodeConfig {
    fn default() -> Self {
        Self {
            blank: 0,
            max_tokens_per_frame: 10,
        }
    }
}

/// Walks `frames` encoder frames, emitting tokens greedily.
pub fn greedy_decode<S: TransducerStep>(
    step: &mut S,
    frames: usize,
    config: DecodeConfig,
) -> Result<Vec<DecodedToken>, String> {
    let mut emitted: Vec<DecodedToken> = Vec::new();
    let mut ids: Vec<usize> = Vec::new();
    let mut frame = 0usize;
    let mut emitted_here = 0usize;

    while frame < frames {
        let output = step.step(frame, &ids)?;
        let token = argmax(&output.token_logits)
            .ok_or_else(|| "the joint network returned no logits".to_string())?;

        if token != config.blank {
            // Only a real token advances the decoder state. Committing on a
            // blank would condition the prediction network on a token that was
            // never emitted, and the error compounds over the rest of the
            // utterance rather than showing up as one wrong word.
            step.commit();
            ids.push(token);
            emitted.push(DecodedToken { id: token, frame });
            emitted_here += 1;
        }

        match output.duration {
            // The duration head spoke: honour it, and the frame's token budget
            // starts again wherever we land.
            Some(skip) if skip > 0 => {
                frame += skip;
                emitted_here = 0;
            }
            // A zero duration means "stay here", which is legitimate — it is
            // how several tokens are emitted from one frame — but only until
            // the budget runs out.
            _ if token == config.blank || emitted_here >= config.max_tokens_per_frame => {
                frame += 1;
                emitted_here = 0;
            }
            _ => {}
        }
    }

    Ok(emitted)
}

/// The index of the largest value, or `None` for an empty slice.
///
/// Written out rather than folded with `partial_cmp().unwrap()`: logits
/// arriving as NaN is a real failure mode for a quantised model, and unwrapping
/// a comparison against NaN panics inside a transcription worker. Here NaN
/// simply never wins, and a frame of all-NaN yields index 0 — usually the
/// blank — which costs one frame instead of the process.
fn argmax(values: &[f32]) -> Option<usize> {
    if values.is_empty() {
        return None;
    }
    let mut best = 0usize;
    let mut best_value = f32::NEG_INFINITY;
    for (index, &value) in values.iter().enumerate() {
        if value > best_value {
            best_value = value;
            best = index;
        }
    }
    Some(best)
}

#[cfg(test)]
mod tests {
    use super::*;

    const BLANK: usize = 0;

    /// A joint network that reads from a script.
    ///
    /// Indexed by call, not by frame, so a test can say what happens on the
    /// second evaluation of the same frame — which is the whole of what
    /// `max_tokens_per_frame` and a zero duration are about.
    struct Scripted {
        script: Vec<(usize, Option<usize>)>,
        calls: usize,
        commits: usize,
        /// Frames the loop asked about, in order.
        frames_seen: Vec<usize>,
        /// The token history handed over at each call.
        histories: Vec<Vec<usize>>,
    }

    impl Scripted {
        fn new(script: Vec<(usize, Option<usize>)>) -> Self {
            Self {
                script,
                calls: 0,
                commits: 0,
                frames_seen: Vec::new(),
                histories: Vec::new(),
            }
        }
    }

    impl TransducerStep for Scripted {
        fn step(&mut self, frame: usize, emitted: &[usize]) -> Result<StepOutput, String> {
            self.frames_seen.push(frame);
            self.histories.push(emitted.to_vec());
            let (token, duration) = *self
                .script
                .get(self.calls)
                .unwrap_or_else(|| panic!("script ran out at call {}", self.calls));
            self.calls += 1;

            // One-hot over a five-token vocabulary: 0 is the blank.
            let mut logits = vec![0.0f32; 5];
            logits[token] = 1.0;
            Ok(StepOutput {
                token_logits: logits,
                duration,
            })
        }

        fn commit(&mut self) {
            self.commits += 1;
        }
    }

    fn config() -> DecodeConfig {
        DecodeConfig {
            blank: BLANK,
            max_tokens_per_frame: 3,
        }
    }

    #[test]
    fn a_blank_emits_nothing_and_advances_one_frame() {
        let mut step = Scripted::new(vec![(BLANK, Some(1)), (BLANK, Some(1))]);
        let decoded = greedy_decode(&mut step, 2, config()).unwrap();
        assert!(decoded.is_empty());
        assert_eq!(step.frames_seen, vec![0, 1]);
    }

    #[test]
    fn a_token_is_emitted_with_the_frame_it_was_heard_at() {
        let mut step = Scripted::new(vec![(BLANK, Some(1)), (3, Some(1)), (BLANK, Some(1))]);
        let decoded = greedy_decode(&mut step, 3, config()).unwrap();
        assert_eq!(decoded, vec![DecodedToken { id: 3, frame: 1 }]);
    }

    #[test]
    fn the_duration_head_skips_frames_rather_than_stepping_through_them() {
        // This is where TDT's speed comes from: four frames, one evaluation.
        let mut step = Scripted::new(vec![(2, Some(4))]);
        let decoded = greedy_decode(&mut step, 4, config()).unwrap();
        assert_eq!(decoded, vec![DecodedToken { id: 2, frame: 0 }]);
        assert_eq!(step.calls, 1, "a skip of 4 should not be walked frame by frame");
    }

    #[test]
    fn a_skip_past_the_end_ends_the_utterance_rather_than_reading_past_it() {
        let mut step = Scripted::new(vec![(2, Some(100))]);
        let decoded = greedy_decode(&mut step, 4, config()).unwrap();
        assert_eq!(decoded.len(), 1);
        assert_eq!(step.calls, 1);
    }

    #[test]
    fn a_zero_duration_emits_several_tokens_from_one_frame() {
        let mut step = Scripted::new(vec![
            (1, Some(0)),
            (2, Some(0)),
            (BLANK, Some(1)),
            (BLANK, Some(1)),
        ]);
        let decoded = greedy_decode(&mut step, 2, config()).unwrap();
        assert_eq!(
            decoded,
            vec![
                DecodedToken { id: 1, frame: 0 },
                DecodedToken { id: 2, frame: 0 },
            ]
        );
    }

    #[test]
    fn a_model_stuck_on_one_frame_is_moved_along_rather_than_looping_forever() {
        // The one way greedy transducer decoding fails to terminate: a real
        // token, a zero duration, and a state that produces the same token.
        let mut step = Scripted::new(vec![(1, Some(0)); 16]);
        let decoded = greedy_decode(&mut step, 2, config()).unwrap();
        // Three per frame, two frames — the budget, not the script.
        assert_eq!(decoded.len(), 6);
        assert_eq!(step.calls, 6);
    }

    #[test]
    fn the_budget_starts_again_after_a_skip() {
        let mut step = Scripted::new(vec![
            (1, Some(0)),
            (2, Some(1)),
            (1, Some(0)),
            (2, Some(1)),
        ]);
        let decoded = greedy_decode(&mut step, 2, config()).unwrap();
        assert_eq!(decoded.len(), 4, "the second frame should get a full budget");
    }

    #[test]
    fn only_a_real_token_advances_the_decoder_state() {
        // A blank adds nothing to the token history, so committing on one
        // conditions the prediction network on something never emitted — an
        // error that compounds over the utterance rather than showing up as
        // one wrong word.
        let mut step = Scripted::new(vec![(BLANK, Some(1)), (4, Some(1)), (BLANK, Some(1))]);
        greedy_decode(&mut step, 3, config()).unwrap();
        assert_eq!(step.commits, 1);
    }

    #[test]
    fn each_evaluation_sees_everything_emitted_before_it() {
        let mut step = Scripted::new(vec![(1, Some(1)), (2, Some(1)), (BLANK, Some(1))]);
        greedy_decode(&mut step, 3, config()).unwrap();
        assert_eq!(step.histories, vec![vec![], vec![1], vec![1, 2]]);
    }

    #[test]
    fn a_model_with_no_duration_head_advances_only_on_a_blank() {
        // Ordinary RNN-T. Without a duration head a real token leaves the
        // frame where it is — that is how a frame emits several tokens — and
        // only a blank moves on. Asserting one token per frame here is the
        // mistake that produces a transcript missing every word after the
        // first at each frame.
        let mut step = Scripted::new(vec![
            (1, None),
            (BLANK, None),
            (2, None),
            (BLANK, None),
        ]);
        let decoded = greedy_decode(&mut step, 2, config()).unwrap();
        assert_eq!(
            decoded,
            vec![
                DecodedToken { id: 1, frame: 0 },
                DecodedToken { id: 2, frame: 1 },
            ]
        );
    }

    #[test]
    fn without_a_duration_head_the_budget_still_ends_a_stuck_frame() {
        let mut step = Scripted::new(vec![(1, None); 16]);
        let decoded = greedy_decode(&mut step, 1, config()).unwrap();
        assert_eq!(decoded.len(), 3, "the per-frame budget, not the script");
    }

    #[test]
    fn no_frames_is_no_tokens_and_no_work() {
        let mut step = Scripted::new(vec![]);
        assert!(greedy_decode(&mut step, 0, config()).unwrap().is_empty());
        assert_eq!(step.calls, 0);
    }

    #[test]
    fn a_nan_logit_never_wins() {
        // Quantised models do produce these, and `partial_cmp().unwrap()`
        // inside a transcription worker turns one into a panic.
        assert_eq!(argmax(&[f32::NAN, 1.0, 2.0]), Some(2));
        assert_eq!(argmax(&[3.0, f32::NAN, 2.0]), Some(0));
        assert_eq!(argmax(&[f32::NAN, f32::NAN]), Some(0));
        assert_eq!(argmax(&[]), None);
    }

    #[test]
    fn the_first_of_two_equal_logits_wins_so_decoding_is_reproducible() {
        assert_eq!(argmax(&[1.0, 1.0, 1.0]), Some(0));
    }

    #[test]
    fn a_joint_that_returns_nothing_is_an_error_rather_than_a_panic() {
        struct Empty;
        impl TransducerStep for Empty {
            fn step(&mut self, _: usize, _: &[usize]) -> Result<StepOutput, String> {
                Ok(StepOutput {
                    token_logits: Vec::new(),
                    duration: None,
                })
            }
            fn commit(&mut self) {}
        }
        assert!(greedy_decode(&mut Empty, 1, config()).is_err());
    }
}
