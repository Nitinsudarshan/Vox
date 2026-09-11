//! Long-form meeting recording: capture both sides of a call, transcribe it on
//! this machine, and turn the transcript into a report.
//!
//! ## Shape
//!
//! ```text
//! microphone ─┐
//!             ├─ capture ─ mix ─ segmenter ─ transcription ─ transcript.json
//! system   ───┘              │        (speech spans)   │
//! (loopback)                 └─ checkpoint ─ audio/    └─ meeting-transcript-segment events
//!                                (30s WAVs)
//!
//! transcript.json ─ summary::service ─ templates + LLM ─ summary.json
//! ```
//!
//! ## Why it looks like this
//!
//! The design follows Meetily's, which is the clearest published answer to
//! "record a meeting locally and summarise it": VAD-driven segmentation rather
//! than fixed windows, one serial decoder so transcript order is a property of
//! the pipeline rather than a hope, durable audio checkpoints so a crash costs
//! seconds rather than the meeting, and JSON-defined summary templates so the
//! report shape is data.
//!
//! Where it differs, it differs on purpose — every one of these is a defect
//! named in the teardown of that project, not a matter of taste:
//!
//! - **Rust owns persistence.** Meetily deliberately has its renderer write
//!   the meeting to SQLite after the recording stops
//!   (`recording_commands.rs:866`), which leaves a window where a webview
//!   reload loses the meeting. Here the backend writes the transcript as it
//!   arrives, and the frontend only ever reads.
//! - **Confidence is measured, not invented.** Meetily reports
//!   `(text.len() / 100.0).min(0.9) + 0.1` as a confidence score and filters
//!   on it. Segments here carry Whisper's own `no_speech_prob`.
//! - **The capture channel is kept.** Meetily sums microphone and system audio
//!   before its VAD, which is why its `speaker` column is never written. The
//!   mixer here tracks per-channel energy alongside the mixed window, so a
//!   line can say "You" or "Others".
//! - **Queues are bounded.** Every channel in meetily's audio path is
//!   unbounded, so a decoder slower than real time grows memory for the length
//!   of the meeting. The queue here has a ceiling and reports what it dropped.
//! - **A deleted meeting is deleted.** Meetily removes the database rows and
//!   leaves the audio on disk.

pub mod capture;
pub mod checkpoint;
pub mod engine;
pub mod import;
pub mod model;
pub mod segmenter;
pub mod store;
pub mod summary;
pub mod transcription;

pub use model::{
    Meeting, MeetingListItem, MeetingSource, MeetingState, MeetingSummary, SegmentChannel,
    SummaryStatus, TranscriptSegment,
};
pub use store::{MeetingStore, MeetingStoreError};
