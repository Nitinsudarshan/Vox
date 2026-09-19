//! What a meeting *is*, on disk and over the IPC boundary.
//!
//! One set of types, serialized straight to JSON in the vault and straight to
//! the frontend. Meetily keeps three parallel shapes for this — a SQLite
//! schema, a Rust struct, and a TypeScript interface that drifts from both —
//! and its `speaker` column is the visible cost: migrated into existence and
//! never written by any query. Here the store writes these structs verbatim,
//! so a field that nothing sets is a field that is visibly absent.

use serde::{Deserialize, Serialize};

/// Where a meeting's audio came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MeetingSource {
    /// Captured live from this machine's microphone and system audio.
    Recorded,
    /// Decoded from an audio file the user pointed at.
    Imported,
}

/// Where a meeting is in its lifecycle.
///
/// `Recording` and `Paused` are the only states a crash can leave behind, and
/// [`crate::meetings::store::MeetingStore::recover_interrupted`] is what turns
/// them into `Completed` on the next launch.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MeetingState {
    Recording,
    Paused,
    /// Capture has stopped; queued audio is still being decoded.
    Transcribing,
    Completed,
    /// Capture failed outright. `Meeting::error` says why.
    Failed,
}

impl MeetingState {
    /// Whether a crash in this state left work to finish on next launch.
    pub fn is_interrupted(self) -> bool {
        matches!(self, MeetingState::Recording | MeetingState::Paused)
    }
}

/// Which side of the conversation a transcript segment came from.
///
/// Not diarization, and deliberately not named as if it were: this is the
/// capture channel, known for free because the microphone and the system
/// loopback are two separate streams. Meetily sums those streams into one mono
/// buffer before its VAD ever runs (`audio/pipeline.rs:154`), which destroys
/// the distinction by construction and leaves its `speaker` column permanently
/// empty. Keeping per-channel energy alongside the mixed window costs one
/// extra accumulator and answers the question users actually ask of a meeting
/// transcript — "was that me or them?" — without claiming to tell two remote
/// participants apart, which this cannot do.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SegmentChannel {
    /// Dominated by the local microphone.
    Microphone,
    /// Dominated by system/loopback audio — the far end of the call.
    System,
    /// Both channels carried comparable energy, or only one stream existed.
    Mixed,
}

impl SegmentChannel {
    /// The label shown in a transcript and handed to the summarizer.
    pub fn label(self) -> &'static str {
        match self {
            SegmentChannel::Microphone => "You",
            SegmentChannel::System => "Others",
            SegmentChannel::Mixed => "Speaker",
        }
    }
}

/// One decoded span of speech.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TranscriptSegment {
    /// Monotonic per-meeting ordering key. The transcription worker assigns
    /// these before decoding, so a slow segment can never reorder the
    /// transcript relative to a fast one that followed it.
    pub sequence: u64,
    pub text: String,
    /// Seconds from the start of the recording, not wall-clock — so a
    /// transcript still lines up with its audio after a pause.
    pub start_seconds: f64,
    pub end_seconds: f64,
    pub channel: SegmentChannel,
    /// Whisper's own probability that this span is not speech. A real model
    /// output, unlike meetily's `(text.len() / 100.0).min(0.9) + 0.1`
    /// (`whisper_engine.rs:601`), which is a function of string length shown
    /// to users as if it were a confidence score.
    pub no_speech_prob: f32,
    pub recorded_at: String,
    /// Whether the segmenter cut this span at its ceiling rather than at a
    /// silence — so the *next* segment continues the same sentence.
    ///
    /// The segmenter knew this and threw it away, which meant nothing
    /// downstream could tell a sentence split across two lines from two
    /// sentences. It is what lets the assembler put them back together.
    #[serde(default)]
    pub cut_at_ceiling: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub original_text: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub romanized_text: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub translated_text: Option<String>,
    /// What the glossary changed in this line, and why.
    ///
    /// Empty for almost every line. Where it is not, applying these backwards
    /// reconstructs exactly what the decoder said — which is how the raw ASR
    /// text is preserved without keeping a second copy of every line, and how
    /// a wrong correction stays visible instead of reading as a correct
    /// transcription.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub corrections: Vec<crate::capture::glossary::TermCorrection>,
}

/// Which pass produced a transcript.
///
/// The two exist because they optimize for different things over the same
/// audio. A live pass runs against a clock: audio keeps arriving, so a decoder
/// slower than real time builds a backlog and eventually drops speech. A final
/// pass has no clock at all — the recording is on disk and is not going
/// anywhere — so it can afford a wider beam and a bigger model.
///
/// Recorded on the meeting rather than inferred, because "why is this
/// transcript worse than the one I got last time" is otherwise unanswerable.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TranscriptionPass {
    /// Decoded while the meeting was being recorded.
    Live,
    /// Decoded from the durable recording afterwards.
    Final,
}

impl TranscriptionPass {
    pub fn key(self) -> &'static str {
        match self {
            Self::Live => "live",
            Self::Final => "final",
        }
    }
}

/// What produced the transcript currently on disk.
///
/// Every field is something that changes the answer, which is what makes this
/// worth storing: re-transcribing with the same model, language and profile
/// should produce the same transcript, and when it does not, this is what
/// says which of them moved.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TranscriptProvenance {
    pub pass: TranscriptionPass,
    /// Recognizer id — `whisper`, `parakeet`.
    pub engine: String,
    /// Model filename, never its path. A meeting record is exported and
    /// shared, and a path names a machine and usually a person.
    pub model: String,
    /// The language the decode was pinned to, or `None` for auto-detection.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub language: Option<String>,
    /// Decode profile, in the engine's own terms.
    pub profile: String,
    pub completed_at: String,
}

/// One person Vox believes spoke during a meeting.
///
/// The name is the user's; everything else is evidence. Vox proposes the
/// grouping and plays a sample back so the user can hear who it found — see
/// [`crate::meetings::voiceprint`] for why proposing rather than asserting is
/// the honest shape for the technique underneath.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Speaker {
    /// Stable within a meeting, and what [`TranscriptSegment::speaker_id`]
    /// points at.
    pub id: String,
    /// What to call this person. Starts as "Speaker 1"; the user renames it.
    pub label: String,
    /// Whether [`Self::label`] is the user's word or Vox's placeholder. A
    /// re-run may renumber its own placeholders and must never overwrite a
    /// name somebody typed.
    #[serde(default)]
    pub named_by_user: bool,
    /// Which capture channel this speaker was heard on, where it is
    /// consistent. `Microphone` is the person holding the laptop.
    pub channel: SegmentChannel,
    /// A span of the recording where this speaker is talking alone, so the UI
    /// can play a few seconds and let the user put a name to the voice.
    pub sample_start_seconds: f64,
    pub sample_end_seconds: f64,
    /// Transcript lines attributed to this speaker.
    pub segment_count: usize,
    /// Seconds of speech attributed to this speaker.
    pub speaking_seconds: f64,
}

impl TranscriptSegment {
    /// How long this span lasted, in seconds.
    pub fn duration_seconds(&self) -> f64 {
        (self.end_seconds - self.start_seconds).max(0.0)
    }
}

/// A meeting's metadata record — everything except its transcript and summary,
/// which are large and are stored beside it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Meeting {
    pub id: String,
    pub title: String,
    pub created_at: String,
    pub updated_at: String,
    pub state: MeetingState,
    pub source: MeetingSource,
    pub duration_seconds: f64,
    /// Absolute path to the merged recording, once one exists.
    #[serde(default)]
    pub audio_path: Option<String>,
    /// What produced the transcript currently on disk.
    ///
    /// `None` for a meeting transcribed before this was recorded, and for one
    /// still recording. Replaces an earlier `transcript_model` field that held
    /// a full filesystem path and that nothing ever read.
    #[serde(default)]
    pub transcript: Option<TranscriptProvenance>,
    #[serde(default)]
    pub language: Option<String>,
    #[serde(default)]
    pub mic_device: Option<String>,
    /// Whether system audio was successfully captured. `false` means the
    /// transcript has only the local side of the conversation, which changes
    /// how a summary should be read.
    #[serde(default)]
    pub system_audio_captured: bool,
    #[serde(default)]
    pub segment_count: usize,
    /// Segments the pipeline queued but could not decode. Surfaced rather than
    /// swallowed: meetily counts a failed chunk as complete and drops it from
    /// the transcript with no user-visible gap (`worker.rs:229`).
    #[serde(default)]
    pub dropped_segments: usize,
    #[serde(default)]
    pub error: Option<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    /// The recurring meeting this recording is one of, once it is known.
    ///
    /// Written during a sync, from the calendar event the recording matched,
    /// and never cleared by one — the event cache is a rolling window, so a
    /// membership derived fresh on every read would evaporate a month later.
    /// A typed field rather than a tag: a series is one-to-many with an
    /// identity that comes from outside Vox, and matching on tag text would
    /// split a series silently the moment somebody renamed it.
    #[serde(default)]
    pub series_id: Option<String>,
}

impl Meeting {
    /// A fresh meeting in `Recording` state.
    pub fn new(id: String, title: String, source: MeetingSource) -> Self {
        let now = chrono::Utc::now().to_rfc3339();
        Self {
            id,
            title,
            created_at: now.clone(),
            updated_at: now,
            state: MeetingState::Recording,
            source,
            duration_seconds: 0.0,
            audio_path: None,
            transcript: None,
            language: None,
            mic_device: None,
            system_audio_captured: false,
            segment_count: 0,
            dropped_segments: 0,
            error: None,
            tags: Vec::new(),
            series_id: None,
        }
    }

    /// Stamps `updated_at`. Every mutation goes through the store, and the
    /// store calls this, so no caller has to remember.
    pub fn touch(&mut self) {
        self.updated_at = chrono::Utc::now().to_rfc3339();
    }
}

/// How far a summary got.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SummaryStatus {
    Pending,
    Processing,
    Completed,
    Failed,
    Cancelled,
}

/// A meeting's generated report, and everything needed to decide whether it
/// can be reused instead of regenerated.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MeetingSummary {
    pub meeting_id: String,
    pub status: SummaryStatus,
    pub template_id: String,
    /// The report as the user should see it — translated, if a translation was
    /// asked for.
    #[serde(default)]
    pub markdown: Option<String>,
    /// The English original, kept even when `markdown` is a translation.
    ///
    /// This is what makes re-translating a meeting into a second language cost
    /// one pass instead of two: the expensive summarization is already done and
    /// its fingerprint still matches.
    #[serde(default)]
    pub english_markdown: Option<String>,
    /// The previous completed report, held only for the duration of a
    /// regeneration so a failure or a cancel restores what was there before.
    #[serde(default)]
    pub previous_markdown: Option<String>,
    #[serde(default)]
    pub error: Option<String>,
    #[serde(default)]
    pub provider: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
    /// The language the user asked for, as a BCP-47 code. `None` means English.
    #[serde(default)]
    pub language: Option<String>,
    /// Hash of every input that could change the English report. A regeneration
    /// whose fingerprint matches and which only changes the output language
    /// skips straight to the translation pass.
    #[serde(default)]
    pub fingerprint: Option<String>,
    /// The transcript this report was written from.
    ///
    /// Not the same question as the cache fingerprint beside it. The
    /// fingerprint answers "may this be reused"; this answers "what was it
    /// made from" — which model, which pass, how many lines, and whether any
    /// speech was missing from them. A report generated from a transcript
    /// with a hole in it is a different object from one generated from a
    /// complete transcript, and until now nothing said which it was.
    #[serde(default)]
    pub transcript_source: Option<TranscriptProvenance>,
    /// Canonical lines the report was written from.
    #[serde(default)]
    pub transcript_segments: usize,
    /// Segments of recorded speech the transcript did not contain. Non-zero
    /// means the report describes an incomplete record, and says so.
    #[serde(default)]
    pub transcript_missing_segments: usize,
    #[serde(default)]
    pub chunk_count: u32,
    #[serde(default)]
    pub processing_ms: u64,
    #[serde(default)]
    pub started_at: Option<String>,
    #[serde(default)]
    pub completed_at: Option<String>,
}

impl MeetingSummary {
    /// An empty record in `Pending`, ready for the background task to claim.
    pub fn pending(meeting_id: &str, template_id: &str) -> Self {
        Self {
            meeting_id: meeting_id.to_string(),
            status: SummaryStatus::Pending,
            template_id: template_id.to_string(),
            markdown: None,
            english_markdown: None,
            previous_markdown: None,
            error: None,
            provider: None,
            model: None,
            language: None,
            fingerprint: None,
            transcript_source: None,
            transcript_segments: 0,
            transcript_missing_segments: 0,
            chunk_count: 0,
            processing_ms: 0,
            started_at: Some(chrono::Utc::now().to_rfc3339()),
            completed_at: None,
        }
    }

    /// Whether a summary run is still in flight.
    pub fn is_running(&self) -> bool {
        matches!(self.status, SummaryStatus::Pending | SummaryStatus::Processing)
    }
}

/// A meeting as the list surface needs it: metadata plus the couple of derived
/// numbers the UI would otherwise fetch a whole transcript to compute.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MeetingListItem {
    #[serde(flatten)]
    pub meeting: Meeting,
    pub has_summary: bool,
    pub summary_status: Option<SummaryStatus>,
    /// First ~200 characters of the transcript, for the list row.
    pub preview: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn interrupted_states_are_exactly_the_ones_a_crash_can_leave() {
        assert!(MeetingState::Recording.is_interrupted());
        assert!(MeetingState::Paused.is_interrupted());
        assert!(!MeetingState::Transcribing.is_interrupted());
        assert!(!MeetingState::Completed.is_interrupted());
        assert!(!MeetingState::Failed.is_interrupted());
    }

    #[test]
    fn channel_labels_say_who_rather_than_which_device() {
        assert_eq!(SegmentChannel::Microphone.label(), "You");
        assert_eq!(SegmentChannel::System.label(), "Others");
        assert_eq!(SegmentChannel::Mixed.label(), "Speaker");
    }

    #[test]
    fn a_segments_duration_never_goes_negative_on_reversed_bounds() {
        let segment = TranscriptSegment {
            sequence: 0,
            text: "hello".into(),
            start_seconds: 5.0,
            end_seconds: 4.0,
            channel: SegmentChannel::Mixed,
            no_speech_prob: 0.0,
            recorded_at: "2026-01-01T00:00:00Z".into(),
            cut_at_ceiling: false,
            original_text: None,
            romanized_text: None,
            translated_text: None,
            corrections: Vec::new(),
        };
        assert_eq!(segment.duration_seconds(), 0.0);
    }

    #[test]
    fn a_new_meeting_starts_recording_with_matching_timestamps() {
        let meeting = Meeting::new("meeting-1".into(), "Standup".into(), MeetingSource::Recorded);
        assert_eq!(meeting.state, MeetingState::Recording);
        assert_eq!(meeting.created_at, meeting.updated_at);
        assert!(!meeting.system_audio_captured);
    }

    #[test]
    fn a_pending_summary_reads_as_running() {
        let summary = MeetingSummary::pending("meeting-1", "default");
        assert!(summary.is_running());
        assert!(summary.markdown.is_none());
    }

    #[test]
    fn meeting_json_round_trips_through_the_store_shape() {
        let meeting = Meeting::new("meeting-1".into(), "Sync".into(), MeetingSource::Imported);
        let json = serde_json::to_string(&meeting).expect("serialize");
        let back: Meeting = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(meeting, back);
    }

    #[test]
    fn an_older_record_missing_the_newer_fields_still_loads() {
        // Every field added after the first release is `#[serde(default)]`,
        // so a meeting.json written by an earlier build must still parse.
        let json = r#"{
            "id": "meeting-1",
            "title": "Old",
            "created_at": "2026-01-01T00:00:00Z",
            "updated_at": "2026-01-01T00:00:00Z",
            "state": "completed",
            "source": "recorded",
            "duration_seconds": 12.0
        }"#;
        let meeting: Meeting = serde_json::from_str(json).expect("parse legacy record");
        assert_eq!(meeting.segment_count, 0);
        assert!(meeting.audio_path.is_none());
        assert!(meeting.tags.is_empty());
    }
}
