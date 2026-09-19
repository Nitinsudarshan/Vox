//! What each decode cost, kept rather than printed.
//!
//! The meeting worker already measured all of this — queue wait, lock wait,
//! model load, decode, post-processing, persistence, and the voiced-time
//! profile behind the hallucination screen. It printed the numbers to stdout
//! and dropped them. That is enough to watch one run and not enough to answer
//! any question about behaviour over time, which is what every performance
//! question actually is: *is this model too slow on this machine*, *did the
//! backlog grow*, *how often does the screen reject real speech*.
//!
//! `capture::decode_history` made the same argument for dictation. This is the
//! meeting counterpart.
//!
//! ## Two files, two shapes
//!
//! ```text
//! <meeting>/diagnostics.jsonl   one line per decoded segment, append-only
//! <meeting>/diagnostics.json    the meeting's rollup, written once on stop
//! ```
//!
//! Segments are a log, so they are appended a line at a time: a crash keeps
//! everything written before it, and the cost of recording the thousandth
//! segment is the same as the first. The rollup is a document, so it is
//! written atomically like every other document in the vault.
//!
//! ## What is deliberately not here
//!
//! - **Transcript text.** A diagnostics record carries `text_chars`, not the
//!   text. The transcript already holds it, and a second copy of meeting
//!   content is a second thing to leak and a second thing to delete.
//! - **Audio.** Same reason, more so.
//! - **Speaker attribution.** Speakers are assigned long after the decode, by a
//!   separate run, and `transcript.json` already carries `speaker_id` against
//!   the same `sequence` these records use. Copying it here would create two
//!   places for one fact to disagree — which is the defect `model.rs` calls out
//!   in Meetily's never-written `speaker` column.
//! - **Confidence.** There is no confidence field, because Whisper does not
//!   report one. What it reports is `no_speech_prob`, and that is what the
//!   field is called. See `docs/speech-decision-log.md` D-006.

use serde::{Deserialize, Serialize};

/// What a record written before turn reasons existed says about its end.
fn default_end_reason() -> super::speech_state::TurnEnd {
    super::speech_state::TurnEnd::Silence
}

/// Schema version for both files. A reader that does not recognise it skips
/// the record rather than interpreting old numbers under new rules.
pub const DIAGNOSTICS_VERSION: u32 = 1;

/// What happened to one segment.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SegmentStatus {
    /// Decoded, screened, and written to the transcript.
    Kept,
    /// Decoded and discarded. `rejection` says why.
    Discarded,
    /// The decoder returned an error. The audio is still in the recording.
    Failed,
}

/// One segment's journey from the segmenter to the transcript.
///
/// Every duration is milliseconds and every one of them is separate on
/// purpose. A single total cannot tell "the model is slow" from "something
/// else held the model" from "the queue was behind", and those need three
/// different fixes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SegmentDiagnostics {
    pub version: u32,
    /// Joins to `TranscriptSegment::sequence`, including for segments that
    /// never produced one — which is how a gap in the transcript is explained.
    pub sequence: u64,
    pub start_seconds: f64,
    pub end_seconds: f64,
    /// Which capture channel dominated the span. Measured, not inferred.
    pub channel: super::model::SegmentChannel,
    /// Whether the segmenter cut this at its ceiling rather than at a silence,
    /// so the next segment continues the same sentence.
    pub forced_split: bool,
    /// Why the turn ended.
    #[serde(default = "default_end_reason")]
    pub end_reason: super::speech_state::TurnEnd,
    /// Quiet the segmenter waited through before deciding the turn was over.
    ///
    /// The part of a line's finalization latency that no decoder can remove.
    /// Recorded separately from `decode_ms` because the two are fixed by
    /// entirely different things, and confusing them is how "transcription is
    /// slow" gets answered with a smaller model when the answer was a shorter
    /// hangover.
    #[serde(default)]
    pub hangover_ms: u64,

    // --- the speech evidence the screen ran on ---
    /// Seconds of the span that cleared the voiced threshold, measured at 20 ms
    /// resolution against the span's own noise floor.
    pub voiced_seconds: f64,
    pub total_seconds: f64,
    /// Whisper's own probability that this span is not speech. `None` for an
    /// engine that does not report one — never a stand-in.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub no_speech_prob: Option<f32>,

    // --- where the time went ---
    pub queue_wait_ms: u128,
    /// Waiting for the single loaded-model slot, which dictation also uses.
    pub lock_wait_ms: u128,
    pub model_load_ms: u128,
    /// Whether that load evicted another model rather than filling an empty
    /// slot — the signature of two surfaces fighting over one engine.
    pub model_reloaded: bool,
    pub decode_ms: u128,
    /// Hallucination screen plus text normalisation.
    pub post_ms: u128,
    pub persist_ms: u128,

    // --- what produced it ---
    pub model: String,
    /// The language the decode was pinned to, or `None` for auto-detection.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub language: Option<String>,
    /// Whether this segment used the cheaper profile kept for scripts Whisper
    /// writes expensively.
    pub expensive_script_profile: bool,
    /// The encoder clamp, or `None` for Whisper's full thirty-second window.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub audio_ctx: Option<i32>,

    // --- outcome ---
    pub status: SegmentStatus,
    /// The `speech_health` reason key, for a discarded segment.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rejection: Option<String>,
    /// The decoder's error, for a failed one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// Length of the kept text. The text itself lives in the transcript.
    pub text_chars: usize,
    /// Queue depth immediately after this segment completed.
    pub queue_depth_after: u64,
}

impl SegmentDiagnostics {
    /// Audio seconds this span covered.
    pub fn audio_seconds(&self) -> f64 {
        (self.end_seconds - self.start_seconds).max(0.0)
    }

    /// Decode time against the audio decoded — the model's own speed, with no
    /// queueing in it.
    pub fn decode_rtf(&self) -> f64 {
        div(self.decode_ms as f64 / 1000.0, self.audio_seconds())
    }

    /// From the segmenter closing this span to its text existing.
    pub fn finalization_ms(&self) -> u128 {
        self.queue_wait_ms + self.lock_wait_ms + self.model_load_ms + self.decode_ms + self.post_ms
    }

    /// Share of the span that was actually voice.
    pub fn voiced_ratio(&self) -> f64 {
        div(self.voiced_seconds, self.total_seconds)
    }
}

/// How the recording itself went, as distinct from how transcription went.
///
/// Separate from [`TranscriptionHealth`] rather than merged into one status,
/// because they fail for different reasons and are fixed by different actions:
/// a microphone that heard nothing is a device problem, and a backlog is a
/// model problem. See `docs/speech-decision-log.md` D-010.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct CaptureHealth {
    pub recording_seconds: f64,
    pub microphone_opened: bool,
    pub system_audio_opened: bool,
    /// Whether anything above the silence floor ever arrived on each channel.
    /// `opened && !heard` is the signature of the wrong device.
    pub microphone_heard: bool,
    pub system_audio_heard: bool,
    /// Whether durable audio was written for the whole recording.
    pub audio_checkpoints_written: bool,
    /// Checkpoint writes that failed. Non-zero means the recording has gaps.
    pub checkpoint_failures: u64,
    /// Audio that was captured and never reached the writer, because the
    /// bounded channel to the pump was full or a device FIFO overran.
    ///
    /// Should be 0.0 on every ordinary recording. Non-zero is a hole in the
    /// recording itself, which is worse than anything on the transcription
    /// side, because nothing can regenerate it.
    #[serde(default)]
    pub audio_lost_seconds: f64,
}

/// How transcription went.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct TranscriptionHealth {
    pub segments_emitted: u64,
    pub segments_kept: u64,
    pub segments_discarded: u64,
    pub segments_failed: u64,
    /// Refused because the bounded queue was full. Speech that exists in the
    /// recording and not in the transcript.
    pub segments_dropped: u64,
    pub peak_queue_depth: u64,

    /// Audio the segmenter called speech.
    pub speech_seconds: f64,
    /// Audio behind transcript text that survived screening.
    pub transcribed_seconds: f64,

    /// Decode time against speech — the model's raw speed.
    pub mean_decode_rtf: f64,
    /// Worst single segment, and which one. The mean hides a temperature
    /// fallback that re-decoded one span six times.
    pub worst_decode_rtf: f64,
    pub worst_decode_sequence: u64,
    /// Decode time against wall-clock recording length. Above 1.0 the backlog
    /// grows; below it, nothing accumulates. This is the number that decides
    /// whether a meeting can be transcribed live at all.
    pub pipeline_rtf: f64,

    pub finalization_p50_ms: u128,
    pub finalization_p95_ms: u128,
    pub finalization_max_ms: u128,
    /// Median quiet waited through before a turn was judged over.
    ///
    /// Reported beside the finalization percentiles because together they
    /// answer the question that decides what to tune: of the wait before a
    /// line appears, how much is the decoder and how much is the hangover. No
    /// model makes the second number smaller.
    #[serde(default)]
    pub hangover_p50_ms: u64,
    /// Turns cut at the segmenter's ceiling rather than at a silence. Each one
    /// is a sentence split across two transcript lines.
    #[serde(default)]
    pub segments_forced_split: u64,

    /// Total time spent waiting on the shared model slot.
    pub lock_wait_ms_total: u128,
    pub model_load_ms_total: u128,
    pub model_reloads: u64,

    /// What the user waited out after pressing stop.
    pub drain_seconds: f64,
    /// Whether the drain finished, or gave up on a backlog it could not clear.
    pub drain_completed: bool,

    /// Discarded segments by `speech_health` reason key.
    #[serde(default)]
    pub rejections: std::collections::BTreeMap<String, u64>,
}

impl TranscriptionHealth {
    /// Audio time covered by transcript text, as a fraction of the recording.
    ///
    /// Well under 1.0 for any real meeting, because a meeting is mostly
    /// silence. Read against another run over the same recording, not against
    /// an absolute target.
    pub fn transcript_coverage(&self, recording_seconds: f64) -> f64 {
        div(self.transcribed_seconds, recording_seconds)
    }

    /// Of the audio the segmenter called speech, how much produced text.
    /// This one should approach 1.0, and the gap is speech that was lost.
    pub fn speech_coverage(&self) -> f64 {
        div(self.transcribed_seconds, self.speech_seconds)
    }
}

/// One meeting's diagnostics rollup.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MeetingDiagnostics {
    pub version: u32,
    pub meeting_id: String,
    pub completed_at: String,
    /// Model filename, never its path — a diagnostics file gets shared, and a
    /// path names a machine and usually a person.
    pub model: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub language: Option<String>,
    pub strategy: String,
    pub threads: String,
    /// Every decode-profile change, as (first segment on the new profile,
    /// whether that profile is the cheaper one).
    #[serde(default)]
    pub profile_switches: Vec<(u64, bool)>,
    pub segments_on_cheap_profile: u64,

    pub capture: CaptureHealth,
    pub transcription: TranscriptionHealth,
}

impl MeetingDiagnostics {
    /// Builds the rollup from the segment log plus what only the stop path
    /// knows: how long the recording ran and how long the drain took.
    pub fn summarize(
        meeting_id: &str,
        segments: &[SegmentDiagnostics],
        capture: CaptureHealth,
        stop: StopFacts,
    ) -> Self {
        let mut transcription = TranscriptionHealth {
            segments_emitted: stop.segments_emitted,
            segments_dropped: stop.segments_dropped,
            peak_queue_depth: stop.peak_queue_depth,
            drain_seconds: stop.drain_seconds,
            drain_completed: stop.drain_completed,
            model_reloads: segments.iter().filter(|s| s.model_reloaded).count() as u64,
            ..TranscriptionHealth::default()
        };

        let mut decode_ms_total = 0u128;
        let mut finalizations: Vec<u128> = Vec::with_capacity(segments.len());
        let mut hangovers: Vec<u64> = Vec::with_capacity(segments.len());
        for segment in segments {
            hangovers.push(segment.hangover_ms);
            if segment.forced_split {
                transcription.segments_forced_split += 1;
            }
            decode_ms_total += segment.decode_ms;
            transcription.lock_wait_ms_total += segment.lock_wait_ms;
            transcription.model_load_ms_total += segment.model_load_ms;
            transcription.speech_seconds += segment.audio_seconds();
            finalizations.push(segment.finalization_ms());

            let rtf = segment.decode_rtf();
            if rtf > transcription.worst_decode_rtf {
                transcription.worst_decode_rtf = rtf;
                transcription.worst_decode_sequence = segment.sequence;
            }

            match segment.status {
                SegmentStatus::Kept => {
                    transcription.segments_kept += 1;
                    transcription.transcribed_seconds += segment.audio_seconds();
                }
                SegmentStatus::Discarded => {
                    transcription.segments_discarded += 1;
                    if let Some(reason) = &segment.rejection {
                        *transcription.rejections.entry(reason.clone()).or_insert(0) += 1;
                    }
                }
                SegmentStatus::Failed => transcription.segments_failed += 1,
            }
        }

        transcription.mean_decode_rtf = div(
            decode_ms_total as f64 / 1000.0,
            transcription.speech_seconds,
        );
        transcription.pipeline_rtf = div(
            decode_ms_total as f64 / 1000.0,
            capture.recording_seconds,
        );
        hangovers.sort_unstable();
        transcription.hangover_p50_ms = percentile_u64(&hangovers, 0.50);
        finalizations.sort_unstable();
        transcription.finalization_p50_ms = percentile(&finalizations, 0.50);
        transcription.finalization_p95_ms = percentile(&finalizations, 0.95);
        transcription.finalization_max_ms = finalizations.last().copied().unwrap_or(0);

        Self {
            version: DIAGNOSTICS_VERSION,
            meeting_id: meeting_id.to_string(),
            completed_at: chrono::Utc::now().to_rfc3339(),
            model: stop.model,
            language: stop.language,
            strategy: stop.strategy,
            threads: stop.threads,
            profile_switches: stop.profile_switches,
            segments_on_cheap_profile: stop.segments_on_cheap_profile,
            capture,
            transcription,
        }
    }

    /// How long the backlog would take to clear after a meeting of `minutes`,
    /// at this run's pipeline RTF.
    ///
    /// Audio arrives at wall-clock rate and is consumed at `1 / rtf` of it, so
    /// the backlog reaches `L × (rtf − 1)` and takes exactly that long to
    /// drain. At or below 1.0 nothing accumulates — which is why the point of
    /// tuning is to cross 1.0, not to chase a large multiple of real time.
    pub fn projected_drain_minutes(&self, minutes: f64) -> f64 {
        let rtf = self.transcription.pipeline_rtf;
        if rtf > 1.0 {
            (rtf - 1.0) * minutes
        } else {
            0.0
        }
    }
}

/// The facts only the stop path holds.
#[derive(Debug, Clone, Default)]
pub struct StopFacts {
    pub model: String,
    pub language: Option<String>,
    pub strategy: String,
    pub threads: String,
    pub profile_switches: Vec<(u64, bool)>,
    pub segments_on_cheap_profile: u64,
    pub segments_emitted: u64,
    pub segments_dropped: u64,
    pub peak_queue_depth: u64,
    pub drain_seconds: f64,
    pub drain_completed: bool,
}

fn percentile_u64(sorted: &[u64], fraction: f64) -> u64 {
    if sorted.is_empty() {
        return 0;
    }
    let index = ((sorted.len() as f64 - 1.0) * fraction).round() as usize;
    sorted[index.min(sorted.len() - 1)]
}

fn percentile(sorted: &[u128], fraction: f64) -> u128 {
    if sorted.is_empty() {
        return 0;
    }
    let index = ((sorted.len() as f64 - 1.0) * fraction).round() as usize;
    sorted[index.min(sorted.len() - 1)]
}

fn div(numerator: f64, denominator: f64) -> f64 {
    if denominator > 0.0 {
        numerator / denominator
    } else {
        0.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::meetings::model::SegmentChannel;

    fn segment(sequence: u64, status: SegmentStatus) -> SegmentDiagnostics {
        SegmentDiagnostics {
            version: DIAGNOSTICS_VERSION,
            sequence,
            start_seconds: sequence as f64,
            end_seconds: sequence as f64 + 1.0,
            channel: SegmentChannel::Microphone,
            forced_split: false,
            end_reason: crate::meetings::speech_state::TurnEnd::Silence,
            hangover_ms: 400,
            voiced_seconds: 0.8,
            total_seconds: 1.0,
            no_speech_prob: Some(0.1),
            queue_wait_ms: 10,
            lock_wait_ms: 0,
            model_load_ms: 0,
            model_reloaded: false,
            decode_ms: 500,
            post_ms: 5,
            persist_ms: 2,
            model: "ggml-small.bin".into(),
            language: Some("en".into()),
            expensive_script_profile: false,
            audio_ctx: None,
            status,
            rejection: None,
            error: None,
            text_chars: 20,
            queue_depth_after: 0,
        }
    }

    #[test]
    fn a_segments_phases_stay_separate_and_sum_to_its_finalization() {
        let mut record = segment(0, SegmentStatus::Kept);
        record.queue_wait_ms = 100;
        record.lock_wait_ms = 50;
        record.model_load_ms = 200;
        record.decode_ms = 400;
        record.post_ms = 10;
        // Persistence is deliberately outside finalization: the text exists
        // before it reaches disk, and the two fail for different reasons.
        assert_eq!(record.finalization_ms(), 760);
    }

    #[test]
    fn decode_rtf_is_against_the_audio_decoded_not_the_wall_clock() {
        let mut record = segment(0, SegmentStatus::Kept);
        record.decode_ms = 500;
        record.end_seconds = record.start_seconds + 2.0;
        assert!((record.decode_rtf() - 0.25).abs() < 1e-9);
    }

    #[test]
    fn a_zero_length_segment_reports_zero_rather_than_dividing_by_it() {
        let mut record = segment(0, SegmentStatus::Kept);
        record.end_seconds = record.start_seconds;
        assert_eq!(record.decode_rtf(), 0.0);
        record.total_seconds = 0.0;
        assert_eq!(record.voiced_ratio(), 0.0);
    }

    #[test]
    fn the_rollup_separates_the_two_coverage_questions() {
        let segments = vec![
            segment(0, SegmentStatus::Kept),
            segment(1, SegmentStatus::Discarded),
        ];
        let capture = CaptureHealth {
            recording_seconds: 10.0,
            ..CaptureHealth::default()
        };
        let rollup =
            MeetingDiagnostics::summarize("m", &segments, capture, StopFacts::default());

        // One kept second of ten recorded.
        assert!((rollup.transcription.transcript_coverage(10.0) - 0.1).abs() < 1e-9);
        // One kept second out of two the segmenter called speech: the other
        // was decoded and thrown away, and that gap is the point.
        assert!((rollup.transcription.speech_coverage() - 0.5).abs() < 1e-9);
    }

    #[test]
    fn the_rollup_counts_rejections_by_reason_so_a_pattern_is_visible() {
        let mut looped = segment(1, SegmentStatus::Discarded);
        looped.rejection = Some("phrase_loop".into());
        let mut filler = segment(2, SegmentStatus::Discarded);
        filler.rejection = Some("phrase_loop".into());
        let mut other = segment(3, SegmentStatus::Discarded);
        other.rejection = Some("subtitle_filler".into());

        let rollup = MeetingDiagnostics::summarize(
            "m",
            &[looped, filler, other],
            CaptureHealth::default(),
            StopFacts::default(),
        );
        assert_eq!(rollup.transcription.rejections["phrase_loop"], 2);
        assert_eq!(rollup.transcription.rejections["subtitle_filler"], 1);
        assert_eq!(rollup.transcription.segments_discarded, 3);
    }

    #[test]
    fn the_worst_segment_is_named_because_the_mean_hides_it() {
        let mut fast = segment(0, SegmentStatus::Kept);
        fast.decode_ms = 100;
        let mut slow = segment(7, SegmentStatus::Kept);
        slow.decode_ms = 6_000;

        let rollup = MeetingDiagnostics::summarize(
            "m",
            &[fast, slow],
            CaptureHealth::default(),
            StopFacts::default(),
        );
        assert_eq!(rollup.transcription.worst_decode_sequence, 7);
        assert!(rollup.transcription.worst_decode_rtf > rollup.transcription.mean_decode_rtf);
    }

    #[test]
    fn pipeline_rtf_measures_against_the_clock_and_decode_rtf_against_speech() {
        // Ten seconds of recording holding two seconds of speech, decoded in
        // four. The model runs at 2x real time against speech and still keeps
        // up with the meeting, and only one of those numbers says so.
        let mut a = segment(0, SegmentStatus::Kept);
        a.decode_ms = 2_000;
        let mut b = segment(1, SegmentStatus::Kept);
        b.decode_ms = 2_000;
        let capture = CaptureHealth {
            recording_seconds: 10.0,
            ..CaptureHealth::default()
        };

        let rollup = MeetingDiagnostics::summarize("m", &[a, b], capture, StopFacts::default());
        assert!((rollup.transcription.mean_decode_rtf - 2.0).abs() < 1e-9);
        assert!((rollup.transcription.pipeline_rtf - 0.4).abs() < 1e-9);
        assert_eq!(rollup.projected_drain_minutes(60.0), 0.0);
    }

    #[test]
    fn a_decoder_slower_than_the_clock_projects_the_wait_it_will_cost() {
        let mut slow = segment(0, SegmentStatus::Kept);
        slow.decode_ms = 15_000;
        let capture = CaptureHealth {
            recording_seconds: 10.0,
            ..CaptureHealth::default()
        };
        let rollup = MeetingDiagnostics::summarize("m", &[slow], capture, StopFacts::default());
        assert!((rollup.transcription.pipeline_rtf - 1.5).abs() < 1e-9);
        // Half a minute of backlog per minute of meeting.
        assert!((rollup.projected_drain_minutes(60.0) - 30.0).abs() < 1e-6);
    }

    #[test]
    fn finalization_percentiles_come_from_the_segments_not_from_a_mean() {
        let segments: Vec<SegmentDiagnostics> = (0..100)
            .map(|i| {
                let mut record = segment(i, SegmentStatus::Kept);
                record.queue_wait_ms = i as u128 * 10;
                record.decode_ms = 0;
                record.post_ms = 0;
                record
            })
            .collect();
        let rollup = MeetingDiagnostics::summarize(
            "m",
            &segments,
            CaptureHealth::default(),
            StopFacts::default(),
        );
        // 0, 10, … 990. A mean over that is 495 and describes no segment; the
        // p95 is what says a twentieth of them waited most of a second.
        assert_eq!(rollup.transcription.finalization_p50_ms, 500);
        assert_eq!(rollup.transcription.finalization_p95_ms, 940);
        assert_eq!(rollup.transcription.finalization_max_ms, 990);
    }

    #[test]
    fn lost_audio_is_a_separate_fact_from_a_failed_checkpoint() {
        // Two different holes: one where the writer refused, one where the
        // audio never reached the writer. They have different causes and the
        // rollup must not collapse them into "something went wrong".
        let capture = CaptureHealth {
            recording_seconds: 600.0,
            audio_checkpoints_written: true,
            checkpoint_failures: 0,
            audio_lost_seconds: 1.4,
            ..CaptureHealth::default()
        };
        let rollup = MeetingDiagnostics::summarize("m", &[], capture, StopFacts::default());
        assert!(rollup.capture.audio_checkpoints_written);
        assert_eq!(rollup.capture.checkpoint_failures, 0);
        assert!(rollup.capture.audio_lost_seconds > 0.0);
    }

    #[test]
    fn capture_health_and_transcription_health_are_separately_answerable() {
        // The wrong device: the stream opened and nothing was ever heard on
        // it. Nothing about transcription can express that, which is why they
        // are two structs.
        let capture = CaptureHealth {
            recording_seconds: 600.0,
            microphone_opened: true,
            microphone_heard: false,
            system_audio_opened: true,
            system_audio_heard: true,
            audio_checkpoints_written: true,
            checkpoint_failures: 0,
            audio_lost_seconds: 0.0,
        };
        let rollup =
            MeetingDiagnostics::summarize("m", &[], capture, StopFacts::default());
        assert!(rollup.capture.microphone_opened && !rollup.capture.microphone_heard);
        assert_eq!(rollup.transcription.segments_kept, 0);
    }

    #[test]
    fn there_is_no_confidence_field_to_invent_one_into() {
        // D-006, pinned as a test rather than a comment: what is recorded is
        // the model's own no-speech probability, and an engine that reports
        // none records none.
        let mut record = segment(0, SegmentStatus::Kept);
        record.no_speech_prob = None;
        let json = serde_json::to_string(&record).expect("serialize");
        assert!(!json.contains("confidence"));
        assert!(
            !json.contains("no_speech_prob"),
            "an absent probability must be absent, not zero: {json}"
        );
    }

    #[test]
    fn a_segment_record_round_trips_and_carries_no_transcript_text() {
        let record = segment(3, SegmentStatus::Kept);
        let json = serde_json::to_string(&record).expect("serialize");
        assert!(json.contains("\"text_chars\":20"));
        assert!(!json.contains("\"text\""), "diagnostics must not copy the transcript");
        let back: SegmentDiagnostics = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(record, back);
    }

    #[test]
    fn a_rollup_round_trips_through_its_own_json() {
        let rollup = MeetingDiagnostics::summarize(
            "meeting-1",
            &[segment(0, SegmentStatus::Kept)],
            CaptureHealth::default(),
            StopFacts {
                model: "ggml-small.bin".into(),
                ..StopFacts::default()
            },
        );
        let json = serde_json::to_string(&rollup).expect("serialize");
        let back: MeetingDiagnostics = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(rollup, back);
    }
}
