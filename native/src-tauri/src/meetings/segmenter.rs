//! Turns a continuous mixed audio stream into the spans of speech worth
//! decoding.
//!
//! This is the piece that decides what Whisper ever sees. Meetily's version
//! runs Silero over 600 ms windows and forwards every completed speech segment
//! (`audio/pipeline.rs`); this one is an energy segmenter, because that is the
//! VAD Vox already has, tuned and shipped, in [`crate::capture::VadConfig`] —
//! adding an ONNX model here would add a 
//! runtime dependency to a pipeline that does not otherwise need one.
//!
//! What matters is not which VAD: it is that the segmenter is *streaming*.
//! Vox's existing VAD trims the leading and trailing silence off a finished
//! recording — one decision, made once, at the end. A meeting needs the
//! opposite shape: speech boundaries found continuously, as audio arrives, so
//! a decode can start on the first sentence while the second is still being
//! spoken.
//!
//! Three things make the difference between a usable transcript and a
//! shredded one:
//!
//! - **Pre-roll.** Onset is only detectable after it has happened, so the
//!   segmenter always holds the last [`PRE_ROLL_MS`] and prepends it. Without
//!   it every segment loses its first syllable.
//! - **Redemption.** A gap shorter than [`REDEMPTION_MS`] is a breath, not a
//!   turn. Closing on the first quiet frame cuts sentences in half mid-clause,
//!   and Whisper decodes half a clause far worse than a whole one.
//! - **A ceiling.** Someone who talks for four minutes without a real pause
//!   must not become one four-minute decode, so a segment that reaches
//!   [`MAX_SEGMENT_SECONDS`] is split at its quietest frame rather than at an
//!   arbitrary sample.

use super::model::SegmentChannel;

/// Rate the segmenter (and everything downstream of it) works at.
pub const SEGMENT_SAMPLE_RATE: u32 = 16_000;

/// Analysis frame. 20 ms is short enough to place a boundary tightly and long
/// enough that one plosive does not read as speech.
const FRAME_MS: usize = 20;
const FRAME_SAMPLES: usize = (SEGMENT_SAMPLE_RATE as usize * FRAME_MS) / 1000;

/// Audio prepended to every segment, so onset detection latency does not eat
/// the first syllable.
const PRE_ROLL_MS: usize = 300;
const PRE_ROLL_FRAMES: usize = PRE_ROLL_MS / FRAME_MS;

/// Silence tolerated inside a segment before it is considered finished.
const REDEMPTION_MS: usize = 400;
const REDEMPTION_FRAMES: usize = REDEMPTION_MS / FRAME_MS;

/// Speech shorter than this is a cough, a click, or a door.
const MIN_SPEECH_MS: usize = 250;
const MIN_SPEECH_FRAMES: usize = MIN_SPEECH_MS / FRAME_MS;

/// Consecutive above-threshold frames needed to open a segment.
const ONSET_FRAMES: usize = 3;

/// Longest segment handed to a decoder in one piece.
///
/// The same ceiling meetily uses for its batch path (`audio/import.rs:525`),
/// and for the same reason: Whisper's own window is 30 s, and a decode that
/// long blocks the single serial worker for its whole duration.
pub const MAX_SEGMENT_SECONDS: f64 = 25.0;
const MAX_SEGMENT_FRAMES: usize = (MAX_SEGMENT_SECONDS as usize * 1000) / FRAME_MS;

/// How many recent frames the adaptive noise floor is estimated over.
const NOISE_WINDOW_FRAMES: usize = 150; // 3 seconds

/// Fraction of the noise window treated as "the quiet part of the room".
const NOISE_PERCENTILE: f32 = 0.2;

/// How far above the floor a frame must sit to open a segment.
const ONSET_MARGIN: f32 = 0.008;

/// How far above the floor a frame must sit to keep one open. Lower than the
/// onset margin on purpose: it is much easier to stay in speech than to enter
/// it, which is what stops a trailing-off sentence from being clipped.
const HOLD_MARGIN: f32 = 0.0048;

/// Absolute floor under which nothing counts as speech regardless of how quiet
/// the room is. Without it, a perfectly silent input makes its own noise floor
/// zero and every rounding artefact becomes a segment.
const MIN_ONSET_ENERGY: f32 = 0.006;

/// How much louder one capture channel must be than the other before a segment
/// is attributed to it rather than to both.
const CHANNEL_DOMINANCE_RATIO: f32 = 3.0;

/// One span of speech, ready to decode.
#[derive(Debug, Clone, PartialEq)]
pub struct SpeechSegment {
    /// 16 kHz mono samples, pre-roll included.
    pub samples: Vec<f32>,
    /// Seconds from the start of the recording.
    pub start_seconds: f64,
    pub end_seconds: f64,
    /// Which capture channel dominated this span.
    pub channel: SegmentChannel,
    /// Whether the segmenter cut this segment at [`MAX_SEGMENT_SECONDS`]
    /// rather than at a real silence. The next segment continues the same
    /// sentence, which is worth knowing when joining transcript text.
    pub forced_split: bool,
}

impl SpeechSegment {
    pub fn duration_seconds(&self) -> f64 {
        self.end_seconds - self.start_seconds
    }
}

/// One frame's worth of mixed audio plus where its energy came from.
#[derive(Debug, Clone, Copy, Default)]
struct FrameEnergy {
    mixed_rms: f32,
    mic_sum_sq: f32,
    sys_sum_sq: f32,
    samples: usize,
}

/// A frame held in the pre-roll ring or in an open segment.
#[derive(Debug, Clone)]
struct Frame {
    samples: Vec<f32>,
    energy: FrameEnergy,
}

/// Streaming speech segmenter.
///
/// Feed it mixed 16 kHz mono audio with [`Segmenter::push`]; it returns each
/// segment as soon as that segment is complete. Call [`Segmenter::flush`] when
/// capture stops to close whatever is still open.
pub struct Segmenter {
    /// Samples not yet grouped into a whole frame.
    pending: Vec<f32>,
    pending_mic_sum_sq: f32,
    pending_sys_sum_sq: f32,
    /// The last [`PRE_ROLL_FRAMES`] frames, kept while idle.
    pre_roll: std::collections::VecDeque<Frame>,
    /// Frames belonging to the segment currently open.
    open: Vec<Frame>,
    /// Recent frame energies, for the adaptive noise floor.
    noise_window: std::collections::VecDeque<f32>,
    in_speech: bool,
    consecutive_above: usize,
    consecutive_below: usize,
    /// Frames of real speech seen in the open segment — trailing redemption
    /// silence does not count towards [`MIN_SPEECH_FRAMES`].
    speech_frames: usize,
    /// Index into `open` of the last frame that was actually speech, so the
    /// redemption tail can be trimmed when the segment closes.
    last_speech_frame: usize,
    /// Frames consumed since the recording started — the clock every timestamp
    /// is derived from.
    frames_emitted: u64,
    /// Frame index where the open segment's audio begins (pre-roll included).
    segment_start_frame: u64,
}

impl Default for Segmenter {
    fn default() -> Self {
        Self::new()
    }
}

impl Segmenter {
    pub fn new() -> Self {
        Self {
            pending: Vec::with_capacity(FRAME_SAMPLES),
            pending_mic_sum_sq: 0.0,
            pending_sys_sum_sq: 0.0,
            pre_roll: std::collections::VecDeque::with_capacity(PRE_ROLL_FRAMES + 1),
            open: Vec::new(),
            noise_window: std::collections::VecDeque::with_capacity(NOISE_WINDOW_FRAMES),
            in_speech: false,
            consecutive_above: 0,
            consecutive_below: 0,
            speech_frames: 0,
            last_speech_frame: 0,
            frames_emitted: 0,
            segment_start_frame: 0,
        }
    }

    /// Seconds of audio consumed so far.
    pub fn position_seconds(&self) -> f64 {
        frame_to_seconds(self.frames_emitted)
    }

    /// Whether a segment is currently open.
    pub fn is_in_speech(&self) -> bool {
        self.in_speech
    }

    /// Feeds mixed audio in, returning any segments that completed.
    ///
    /// `mic` and `sys` are the pre-mix per-channel samples for the same span,
    /// used only to attribute the segment to a channel. Either may be empty
    /// when that source is not being captured.
    pub fn push(&mut self, mixed: &[f32], mic: &[f32], sys: &[f32]) -> Vec<SpeechSegment> {
        let mut done = Vec::new();
        for (i, &sample) in mixed.iter().enumerate() {
            self.pending.push(sample);
            let m = mic.get(i).copied().unwrap_or(0.0);
            let s = sys.get(i).copied().unwrap_or(0.0);
            self.pending_mic_sum_sq += m * m;
            self.pending_sys_sum_sq += s * s;

            if self.pending.len() == FRAME_SAMPLES {
                if let Some(segment) = self.consume_frame() {
                    done.push(segment);
                }
            }
        }
        done
    }

    /// Closes whatever is open, returning the final segment if it qualifies.
    ///
    /// Called when capture stops. A partial frame at the end is padded rather
    /// than dropped — the last word of a meeting is exactly where a lost frame
    /// is most noticeable.
    pub fn flush(&mut self) -> Vec<SpeechSegment> {
        let mut done = Vec::new();
        if !self.pending.is_empty() {
            self.pending.resize(FRAME_SAMPLES, 0.0);
            if let Some(segment) = self.consume_frame() {
                done.push(segment);
            }
        }
        if self.in_speech {
            if let Some(segment) = self.close_segment(false) {
                done.push(segment);
            }
        }
        self.in_speech = false;
        self.open.clear();
        self.pre_roll.clear();
        done
    }

    /// Processes one whole frame out of `pending`.
    fn consume_frame(&mut self) -> Option<SpeechSegment> {
        let samples: Vec<f32> = std::mem::replace(&mut self.pending, Vec::with_capacity(FRAME_SAMPLES));
        let energy = FrameEnergy {
            mixed_rms: rms(&samples),
            mic_sum_sq: std::mem::take(&mut self.pending_mic_sum_sq),
            sys_sum_sq: std::mem::take(&mut self.pending_sys_sum_sq),
            samples: samples.len(),
        };
        self.frames_emitted += 1;

        let (onset, hold) = self.thresholds();
        // The floor is made only of frames the segmenter judged to be
        // non-speech. Recording every frame is what made a steady voice raise
        // its own threshold: after a few seconds of talking the "noise"
        // estimate *is* the voice, the hold threshold climbs above it, and the
        // segment closes mid-sentence and cannot re-open.
        //
        // The very first frame is recorded regardless, to seed the estimate. A
        // room with real background noise clears the absolute floor from its
        // opening frame, so with nothing seeded nothing would ever be recorded
        // and the whole recording would read as one unbroken segment.
        if !self.in_speech && (self.noise_window.is_empty() || energy.mixed_rms < onset) {
            self.record_noise(energy.mixed_rms);
        }

        let frame = Frame { samples, energy };

        if !self.in_speech {
            self.push_pre_roll(frame);
            if energy.mixed_rms >= onset {
                self.consecutive_above += 1;
                if self.consecutive_above >= ONSET_FRAMES {
                    self.open_segment();
                }
            } else {
                self.consecutive_above = 0;
            }
            return None;
        }

        // Inside a segment.
        self.open.push(frame);
        if energy.mixed_rms >= hold {
            self.last_speech_frame = self.open.len() - 1;
            self.speech_frames += 1;
            self.consecutive_below = 0;
        } else {
            self.consecutive_below += 1;
            if self.consecutive_below >= REDEMPTION_FRAMES {
                return self.close_segment(false);
            }
        }

        if self.open.len() >= MAX_SEGMENT_FRAMES {
            return self.close_segment(true);
        }
        None
    }

    /// Opens a segment, moving the pre-roll into it.
    fn open_segment(&mut self) {
        let pre_roll: Vec<Frame> = self.pre_roll.drain(..).collect();
        self.segment_start_frame = self.frames_emitted.saturating_sub(pre_roll.len() as u64);
        self.open = pre_roll;
        self.in_speech = true;
        self.consecutive_above = 0;
        self.consecutive_below = 0;
        // The frames that triggered the onset are real speech, and they are
        // already in `open` via the pre-roll.
        self.speech_frames = ONSET_FRAMES.min(self.open.len());
        self.last_speech_frame = self.open.len().saturating_sub(1);
    }

    /// Ends the open segment.
    ///
    /// Returns `None` — and throws the audio away — when what was captured was
    /// shorter than [`MIN_SPEECH_MS`], because a decode of that is noise with
    /// a transcript attached.
    fn close_segment(&mut self, forced: bool) -> Option<SpeechSegment> {
        let frames = std::mem::take(&mut self.open);
        self.in_speech = false;
        self.consecutive_above = 0;
        self.consecutive_below = 0;
        let speech_frames = std::mem::take(&mut self.speech_frames);

        if frames.is_empty() || speech_frames < MIN_SPEECH_FRAMES {
            return None;
        }

        // A forced split cuts at the quietest frame in the back half, so the
        // boundary lands in a gap between words rather than mid-vowel wherever
        // the ceiling happened to fall.
        let cut = if forced {
            quietest_frame_in_tail(&frames)
        } else {
            // Trim the redemption silence that closed the segment, keeping one
            // redemption's worth of tail so the last word is not clipped.
            (self.last_speech_frame + REDEMPTION_FRAMES.min(2) + 1).min(frames.len())
        };
        let cut = cut.clamp(1, frames.len());

        let (kept, rest) = frames.split_at(cut);
        let start_frame = self.segment_start_frame;
        let end_frame = start_frame + kept.len() as u64;

        let mut samples = Vec::with_capacity(kept.len() * FRAME_SAMPLES);
        let mut mic_sum_sq = 0.0f32;
        let mut sys_sum_sq = 0.0f32;
        let mut sample_count = 0usize;
        for frame in kept {
            samples.extend_from_slice(&frame.samples);
            mic_sum_sq += frame.energy.mic_sum_sq;
            sys_sum_sq += frame.energy.sys_sum_sq;
            sample_count += frame.energy.samples;
        }

        if forced {
            // Everything after the cut is the start of the next segment: the
            // speaker has not stopped, so it must not be dropped or replayed.
            self.open = rest.to_vec();
            self.in_speech = true;
            self.segment_start_frame = end_frame;
            self.speech_frames = self.open.len();
            self.last_speech_frame = self.open.len().saturating_sub(1);
        } else {
            self.pre_roll.clear();
        }

        Some(SpeechSegment {
            samples,
            start_seconds: frame_to_seconds(start_frame),
            end_seconds: frame_to_seconds(end_frame),
            channel: classify_channel(mic_sum_sq, sys_sum_sq, sample_count),
            forced_split: forced,
        })
    }

    fn push_pre_roll(&mut self, frame: Frame) {
        if self.pre_roll.len() == PRE_ROLL_FRAMES {
            self.pre_roll.pop_front();
        }
        self.pre_roll.push_back(frame);
    }

    fn record_noise(&mut self, rms: f32) {
        if self.noise_window.len() == NOISE_WINDOW_FRAMES {
            self.noise_window.pop_front();
        }
        self.noise_window.push_back(rms);
    }

    /// Onset and hold thresholds for the current room.
    fn thresholds(&self) -> (f32, f32) {
        let floor = self.noise_floor();
        (
            (floor + ONSET_MARGIN).max(MIN_ONSET_ENERGY),
            (floor + HOLD_MARGIN).max(MIN_ONSET_ENERGY * 0.75),
        )
    }

    /// The quiet part of the recent past, as an RMS level.
    fn noise_floor(&self) -> f32 {
        if self.noise_window.is_empty() {
            return 0.0;
        }
        let mut sorted: Vec<f32> = self.noise_window.iter().copied().collect();
        sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        let count = ((sorted.len() as f32 * NOISE_PERCENTILE).ceil() as usize).max(1);
        sorted[..count].iter().sum::<f32>() / count as f32
    }
}

fn frame_to_seconds(frames: u64) -> f64 {
    frames as f64 * FRAME_MS as f64 / 1000.0
}

fn rms(samples: &[f32]) -> f32 {
    if samples.is_empty() {
        return 0.0;
    }
    let sum_sq: f32 = samples.iter().map(|&s| s * s).sum();
    (sum_sq / samples.len() as f32).sqrt()
}

/// Index just past the quietest frame in the back half of a segment.
///
/// The back half only: cutting at the quietest frame overall would often
/// produce a one-frame segment out of the pre-roll.
fn quietest_frame_in_tail(frames: &[Frame]) -> usize {
    let start = frames.len() / 2;
    let mut best = frames.len() - 1;
    let mut best_rms = f32::MAX;
    for (offset, frame) in frames[start..].iter().enumerate() {
        if frame.energy.mixed_rms < best_rms {
            best_rms = frame.energy.mixed_rms;
            best = start + offset;
        }
    }
    best + 1
}

/// Attributes a span to a capture channel by comparing the two channels' RMS.
///
/// Deliberately conservative: anything short of one channel being
/// [`CHANNEL_DOMINANCE_RATIO`] times the other reads as `Mixed`, because a
/// wrong "You"/"Others" label on a transcript line is worse than no label.
fn classify_channel(mic_sum_sq: f32, sys_sum_sq: f32, samples: usize) -> SegmentChannel {
    if samples == 0 {
        return SegmentChannel::Mixed;
    }
    let mic = (mic_sum_sq / samples as f32).sqrt();
    let sys = (sys_sum_sq / samples as f32).sqrt();
    // Both silent: nothing to attribute. (A segment can only be here because
    // the *mixed* signal was loud, so this is the degenerate case where
    // neither per-channel buffer was supplied.)
    if mic <= f32::EPSILON && sys <= f32::EPSILON {
        return SegmentChannel::Mixed;
    }
    if mic > sys * CHANNEL_DOMINANCE_RATIO {
        SegmentChannel::Microphone
    } else if sys > mic * CHANNEL_DOMINANCE_RATIO {
        SegmentChannel::System
    } else {
        SegmentChannel::Mixed
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `seconds` of a 300 Hz tone at `amplitude`.
    fn tone(seconds: f64, amplitude: f32) -> Vec<f32> {
        let count = (seconds * SEGMENT_SAMPLE_RATE as f64) as usize;
        (0..count)
            .map(|i| {
                let t = i as f32 / SEGMENT_SAMPLE_RATE as f32;
                (t * 300.0 * std::f32::consts::TAU).sin() * amplitude
            })
            .collect()
    }

    fn silence(seconds: f64) -> Vec<f32> {
        vec![0.0; (seconds * SEGMENT_SAMPLE_RATE as f64) as usize]
    }

    /// Pushes mixed audio with no per-channel detail.
    fn push_mixed(segmenter: &mut Segmenter, samples: &[f32]) -> Vec<SpeechSegment> {
        segmenter.push(samples, &[], &[])
    }

    #[test]
    fn silence_alone_never_opens_a_segment() {
        let mut segmenter = Segmenter::new();
        let out = push_mixed(&mut segmenter, &silence(5.0));
        assert!(out.is_empty());
        assert!(segmenter.flush().is_empty());
    }

    #[test]
    fn a_room_tone_below_the_absolute_floor_is_not_speech() {
        // A constant hiss is the case an adaptive floor alone gets wrong: its
        // own noise estimate rises to meet it. MIN_ONSET_ENERGY is what stops
        // that becoming an endless segment.
        let mut segmenter = Segmenter::new();
        let hiss: Vec<f32> = (0..SEGMENT_SAMPLE_RATE as usize * 4)
            .map(|i| if i % 2 == 0 { 0.001 } else { -0.001 })
            .collect();
        let out = push_mixed(&mut segmenter, &hiss);
        assert!(out.is_empty(), "steady room tone should not open a segment");
    }

    #[test]
    fn speech_between_silences_produces_one_segment() {
        let mut segmenter = Segmenter::new();
        let mut audio = silence(1.0);
        audio.extend(tone(2.0, 0.25));
        audio.extend(silence(1.5));

        let mut segments = push_mixed(&mut segmenter, &audio);
        segments.extend(segmenter.flush());

        assert_eq!(segments.len(), 1, "expected exactly one segment");
        let segment = &segments[0];
        assert!(
            segment.duration_seconds() >= 2.0,
            "segment {:.2}s should cover the speech plus pre-roll",
            segment.duration_seconds()
        );
        assert!(!segment.forced_split);
    }

    #[test]
    fn a_segment_starts_before_the_onset_was_detected() {
        let mut segmenter = Segmenter::new();
        let mut audio = silence(1.0);
        audio.extend(tone(1.0, 0.25));
        audio.extend(silence(1.0));

        let mut segments = push_mixed(&mut segmenter, &audio);
        segments.extend(segmenter.flush());

        let segment = &segments[0];
        // Speech starts at 1.0s; the segment must start earlier or the first
        // syllable is gone.
        assert!(
            segment.start_seconds < 1.0,
            "segment started at {:.3}s, no pre-roll was prepended",
            segment.start_seconds
        );
        assert!(segment.start_seconds >= 1.0 - (PRE_ROLL_MS as f64 / 1000.0) - 0.1);
    }

    #[test]
    fn a_breath_inside_a_sentence_does_not_split_it() {
        let mut segmenter = Segmenter::new();
        let mut audio = silence(0.5);
        audio.extend(tone(1.0, 0.25));
        audio.extend(silence(0.2)); // shorter than REDEMPTION_MS
        audio.extend(tone(1.0, 0.25));
        audio.extend(silence(1.0));

        let mut segments = push_mixed(&mut segmenter, &audio);
        segments.extend(segmenter.flush());

        assert_eq!(segments.len(), 1, "a 200ms gap is a breath, not a turn");
    }

    #[test]
    fn a_real_pause_between_turns_splits_them() {
        let mut segmenter = Segmenter::new();
        let mut audio = silence(0.5);
        audio.extend(tone(1.0, 0.25));
        audio.extend(silence(1.2)); // well past REDEMPTION_MS
        audio.extend(tone(1.0, 0.25));
        audio.extend(silence(1.0));

        let mut segments = push_mixed(&mut segmenter, &audio);
        segments.extend(segmenter.flush());

        assert_eq!(segments.len(), 2);
        assert!(
            segments[1].start_seconds > segments[0].end_seconds,
            "segments must not overlap"
        );
    }

    #[test]
    fn a_click_too_short_to_be_speech_is_discarded() {
        let mut segmenter = Segmenter::new();
        let mut audio = silence(0.5);
        audio.extend(tone(0.08, 0.4)); // 80ms, under MIN_SPEECH_MS
        audio.extend(silence(1.0));

        let mut segments = push_mixed(&mut segmenter, &audio);
        segments.extend(segmenter.flush());

        assert!(segments.is_empty(), "80ms of tone is not a sentence");
    }

    #[test]
    fn continuous_speech_is_split_at_the_ceiling_without_losing_audio() {
        let mut segmenter = Segmenter::new();
        let mut audio = silence(0.5);
        audio.extend(tone(40.0, 0.25));

        let mut segments = push_mixed(&mut segmenter, &audio);
        segments.extend(segmenter.flush());

        assert!(segments.len() >= 2, "40s of speech must not be one decode");
        assert!(segments[0].forced_split);
        for segment in &segments {
            assert!(
                segment.duration_seconds() <= MAX_SEGMENT_SECONDS + 0.5,
                "segment ran {:.1}s, past the ceiling",
                segment.duration_seconds()
            );
        }
        // The split must be a cut, not a gap: the second segment starts where
        // the first ended.
        assert!((segments[1].start_seconds - segments[0].end_seconds).abs() < 1e-6);
    }

    #[test]
    fn timestamps_advance_with_the_audio_and_never_go_backwards() {
        let mut segmenter = Segmenter::new();
        // A lead-in, because every real recording has one: capture starts
        // before anyone speaks, and that is what the floor calibrates against.
        let mut audio = silence(0.5);
        for _ in 0..3 {
            audio.extend(tone(1.0, 0.25));
            audio.extend(silence(1.0));
        }

        let mut segments = push_mixed(&mut segmenter, &audio);
        segments.extend(segmenter.flush());

        assert_eq!(segments.len(), 3);
        for pair in segments.windows(2) {
            assert!(pair[0].end_seconds <= pair[1].start_seconds);
        }
        assert!((segmenter.position_seconds() - 6.5).abs() < 0.05);
    }

    #[test]
    fn a_noisy_room_calibrates_instead_of_becoming_one_endless_segment() {
        // Background noise loud enough to clear the absolute floor from the
        // first frame. Without a seeded estimate nothing would ever be
        // recorded as noise, the floor would stay at zero, and the whole
        // recording would open one segment and never close it.
        let noisy: Vec<f32> = (0..SEGMENT_SAMPLE_RATE as usize * 4)
            .map(|i| if i % 2 == 0 { 0.02 } else { -0.02 })
            .collect();

        let mut segmenter = Segmenter::new();
        let mut segments = push_mixed(&mut segmenter, &noisy);
        segments.extend(segmenter.flush());

        assert!(segments.is_empty(), "steady background noise is not speech");
    }

    #[test]
    fn speech_over_a_noisy_room_still_registers() {
        // The contrast is what matters, not the absolute level: the same room
        // noise with someone talking over it must produce a segment.
        let room = |seconds: f64| -> Vec<f32> {
            (0..(seconds * SEGMENT_SAMPLE_RATE as f64) as usize)
                .map(|i| if i % 2 == 0 { 0.02 } else { -0.02 })
                .collect()
        };
        let mut audio = room(1.5);
        let speech = tone(1.5, 0.25);
        audio.extend(speech.iter().zip(room(1.5)).map(|(s, r)| s + r));
        audio.extend(room(1.5));

        let mut segmenter = Segmenter::new();
        let mut segments = push_mixed(&mut segmenter, &audio);
        segments.extend(segmenter.flush());

        assert_eq!(segments.len(), 1, "speech over noise should be one segment");
    }

    #[test]
    fn flush_closes_a_segment_that_was_still_open() {
        let mut segmenter = Segmenter::new();
        let mut audio = silence(0.5);
        audio.extend(tone(1.5, 0.25));

        let during = push_mixed(&mut segmenter, &audio);
        assert!(during.is_empty(), "nothing closed it yet");
        assert!(segmenter.is_in_speech());

        let flushed = segmenter.flush();
        assert_eq!(flushed.len(), 1, "stopping must not lose the last sentence");
        assert!(!segmenter.is_in_speech());
    }

    #[test]
    fn a_partial_frame_at_the_end_is_not_dropped() {
        let mut segmenter = Segmenter::new();
        let mut audio = silence(0.5);
        audio.extend(tone(1.5, 0.25));
        audio.extend(vec![0.3f32; FRAME_SAMPLES / 2]); // half a frame

        push_mixed(&mut segmenter, &audio);
        let flushed = segmenter.flush();

        assert_eq!(flushed.len(), 1);
    }

    #[test]
    fn a_segment_is_attributed_to_whichever_channel_carried_it() {
        let speech = tone(1.5, 0.25);
        let quiet = vec![0.0f32; speech.len()];
        let lead_in = silence(0.5);
        let tail = silence(1.0);

        let mut mic_only = Segmenter::new();
        mic_only.push(&lead_in, &lead_in, &lead_in);
        let mut segments = mic_only.push(&speech, &speech, &quiet);
        segments.extend(mic_only.push(&tail, &tail, &tail));
        segments.extend(mic_only.flush());
        assert_eq!(segments.len(), 1);
        assert_eq!(segments[0].channel, SegmentChannel::Microphone);

        let mut sys_only = Segmenter::new();
        sys_only.push(&lead_in, &lead_in, &lead_in);
        let mut segments = sys_only.push(&speech, &quiet, &speech);
        segments.extend(sys_only.push(&tail, &tail, &tail));
        segments.extend(sys_only.flush());
        assert_eq!(segments.len(), 1);
        assert_eq!(segments[0].channel, SegmentChannel::System);
    }

    #[test]
    fn two_people_talking_at_once_is_not_attributed_to_either() {
        let speech = tone(1.5, 0.25);
        let lead_in = silence(0.5);

        let mut segmenter = Segmenter::new();
        segmenter.push(&lead_in, &lead_in, &lead_in);
        let mut segments = segmenter.push(&speech, &speech, &speech);
        segments.extend(segmenter.flush());

        assert_eq!(segments.len(), 1);
        assert_eq!(segments[0].channel, SegmentChannel::Mixed);
    }

    #[test]
    fn channel_classification_needs_real_dominance() {
        // 1000 samples of RMS 0.2 vs 0.1 — twice as loud, under the 3x bar.
        let loud = 0.2f32 * 0.2 * 1000.0;
        let quiet = 0.1f32 * 0.1 * 1000.0;
        assert_eq!(classify_channel(loud, quiet, 1000), SegmentChannel::Mixed);
        // 10x is dominance.
        let much_quieter = 0.02f32 * 0.02 * 1000.0;
        assert_eq!(
            classify_channel(loud, much_quieter, 1000),
            SegmentChannel::Microphone
        );
    }

    #[test]
    fn audio_arriving_in_ragged_buffers_segments_the_same_as_one_block() {
        // Device callbacks do not deliver whole frames, and a segmenter that
        // only works on frame-aligned input works on a test and not on a
        // meeting.
        let mut audio = silence(0.5);
        audio.extend(tone(1.5, 0.25));
        audio.extend(silence(1.0));

        let mut whole = Segmenter::new();
        let mut expected = push_mixed(&mut whole, &audio);
        expected.extend(whole.flush());

        let mut ragged = Segmenter::new();
        let mut actual = Vec::new();
        let mut offset = 0;
        for (i, size) in [37usize, 512, 91, 1024, 7].iter().cycle().enumerate() {
            if offset >= audio.len() {
                break;
            }
            let end = (offset + size).min(audio.len());
            actual.extend(ragged.push(&audio[offset..end], &[], &[]));
            offset = end;
            assert!(i < 10_000, "loop must terminate");
        }
        actual.extend(ragged.flush());

        assert_eq!(actual.len(), expected.len());
        for (a, e) in actual.iter().zip(expected.iter()) {
            assert_eq!(a.samples.len(), e.samples.len());
            assert!((a.start_seconds - e.start_seconds).abs() < 1e-9);
        }
    }
}
