//! The serial decoder that turns speech segments into transcript lines.
//!
//! ## One worker, on purpose
//!
//! Segments are decoded strictly in order, by a single thread. Meetily makes
//! the same call and says why in one line — "Serial processing ensures
//! transcripts emit in chronological order" (`worker.rs:67`) — having already
//! written a parallel processor it then does not use. The reasoning holds:
//! Whisper's decode time varies by an order of magnitude with segment length,
//! so a parallel pool reorders the transcript relative to the conversation,
//! and re-ordering afterwards means buffering an unbounded number of decoded
//! segments waiting for a slow predecessor. Ordering is the product; throughput
//! is not.
//!
//! ## A bounded queue that says what it dropped
//!
//! Every channel in meetily's audio path is `unbounded_channel`, so a decoder
//! slower than real time — a large model on a CPU-only build, a laptop under
//! load — grows memory for the length of the meeting with nothing shedding
//! load and nothing warning. Here the queue holds at most
//! [`MAX_QUEUED_SEGMENTS`]; past that, a segment is refused, counted, and
//! reported on the meeting record and to the UI. Losing the tail of a
//! backlogged meeting is bad. Silently losing it is worse, and exhausting
//! memory mid-meeting is worse still.
//!
//! ## What is thrown away, and why that is not silent either
//!
//! Whisper invents fluent text over silence and noise — "Thank you for
//! watching", "[Music]", the same clause eight times. Vox already screens for
//! exactly this in [`crate::capture::speech_health`], and every meeting
//! segment goes through it. Meetily has no equivalent on its live path; its
//! only filter is the length-derived "confidence" it invents.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc as std_mpsc;
use std::sync::Arc;

use tauri::{AppHandle, Emitter};

use crate::capture::speech_health::{self, DecodeEvidence};
use crate::capture::stt::{join_utterance_text, SttEngine, SttLanguageConfig, WhisperDecodingConfig};

use super::model::{SegmentChannel, TranscriptSegment};
use super::segmenter::{SpeechSegment, SEGMENT_SAMPLE_RATE};
use super::store::MeetingStore;

/// A transcript line, as it reaches the UI.
pub const TRANSCRIPT_SEGMENT_EVENT: &str = "meeting-transcript-segment";

/// Queue depth and decode counts, for the recording surface's status line.
pub const TRANSCRIPTION_PROGRESS_EVENT: &str = "meeting-transcription-progress";

/// Something the user should know about: a dropped segment, a decode failure.
pub const TRANSCRIPTION_WARNING_EVENT: &str = "meeting-transcription-warning";

/// Longest backlog the decoder will hold.
///
/// A segment is at most [`super::segmenter::MAX_SEGMENT_SECONDS`] of 16 kHz
/// `f32`, so 64 of them is roughly 100 MB and several minutes of speech — deep
/// enough that a transient stall (a big model warming up, another app taking
/// the CPU) never costs a word, shallow enough that a decoder permanently
/// slower than real time fails visibly instead of consuming the machine.
pub const MAX_QUEUED_SEGMENTS: usize = 64;

/// One segment waiting to be decoded.
#[derive(Debug)]
pub struct DecodeJob {
    pub sequence: u64,
    pub segment: SpeechSegment,
}

/// What the worker reports as it goes.
#[derive(Debug, Clone, serde::Serialize)]
pub struct TranscriptionProgress {
    pub meeting_id: String,
    pub queued: u64,
    pub completed: u64,
    pub dropped: u64,
    pub in_queue: u64,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct TranscriptionWarning {
    pub meeting_id: String,
    pub kind: String,
    pub message: String,
}

/// Counts shared between the producers and the decoder.
///
/// Deliberately separate from [`TranscriptionQueue`], which owns the channel's
/// sending half. The worker needs the counts and must *not* hold a sender: a
/// `SyncSender` in the worker's own hands keeps the channel open forever, so
/// `for job in rx` never ends and the stop path's `join` blocks for good.
#[derive(Default)]
struct QueueCounters {
    next_sequence: AtomicU64,
    queued: AtomicU64,
    completed: AtomicU64,
    dropped: AtomicU64,
}

/// The producer end of the decoder's queue.
///
/// Cloneable so the capture loop and the stop path can both reach it; the
/// counters are shared, so both see the same totals. Dropping every clone is
/// what tells the decoder there is nothing more coming.
#[derive(Clone)]
pub struct TranscriptionQueue {
    tx: std_mpsc::SyncSender<DecodeJob>,
    counters: Arc<QueueCounters>,
    app: Option<AppHandle>,
    meeting_id: String,
}

impl TranscriptionQueue {
    /// Enqueues a segment, assigning it the next sequence number.
    ///
    /// Sequence numbers are assigned here rather than in the worker so that a
    /// refused segment still consumes one: a gap in the transcript's sequence
    /// numbers is then a visible record that something was dropped, rather
    /// than a renumbering that hides it.
    ///
    /// Returns `false` when the queue was full and the segment was dropped.
    pub fn submit(&self, segment: SpeechSegment) -> bool {
        let sequence = self.counters.next_sequence.fetch_add(1, Ordering::SeqCst);
        match self.tx.try_send(DecodeJob { sequence, segment }) {
            Ok(()) => {
                self.counters.queued.fetch_add(1, Ordering::SeqCst);
                true
            }
            Err(std_mpsc::TrySendError::Full(job)) => {
                let dropped = self.counters.dropped.fetch_add(1, Ordering::SeqCst) + 1;
                tracing::warn!(
                    "meeting {}: transcription backlog full, dropped segment {} ({:.1}s); \
                     {} dropped so far",
                    self.meeting_id,
                    job.sequence,
                    job.segment.duration_seconds(),
                    dropped
                );
                self.warn(
                    "backlog",
                    &format!(
                        "Transcription is running behind and {dropped} segment(s) of speech could \
                         not be queued. The recording itself is unaffected — a smaller speech \
                         model will keep up better."
                    ),
                );
                false
            }
            Err(std_mpsc::TrySendError::Disconnected(_)) => {
                self.counters.dropped.fetch_add(1, Ordering::SeqCst);
                false
            }
        }
    }

    /// Segments submitted, decoded, and refused.
    pub fn counts(&self) -> (u64, u64, u64) {
        counts_of(&self.counters)
    }

    /// Whether every submitted segment has been decoded.
    pub fn is_drained(&self) -> bool {
        self.counters.completed.load(Ordering::SeqCst)
            >= self.counters.queued.load(Ordering::SeqCst)
    }

    fn warn(&self, kind: &str, message: &str) {
        if let Some(app) = &self.app {
            let _ = app.emit(
                TRANSCRIPTION_WARNING_EVENT,
                TranscriptionWarning {
                    meeting_id: self.meeting_id.clone(),
                    kind: kind.to_string(),
                    message: message.to_string(),
                },
            );
        }
    }
}

/// Everything the worker needs that does not change during a meeting.
pub struct WorkerConfig {
    pub meeting_id: String,
    pub model_path: String,
    pub language: SttLanguageConfig,
    pub decoding: WhisperDecodingConfig,
    /// Words the user has told Vox about, applied to every decoded segment.
    pub glossary: Vec<String>,
}

/// Starts the decoder.
///
/// Returns the queue to submit segments on and the thread handle. Dropping
/// every clone of the queue ends the worker; joining the handle waits for the
/// backlog to finish, which is what the stop path does.
pub fn spawn_worker(
    config: WorkerConfig,
    engine: SttEngine,
    store: Arc<MeetingStore>,
    app: Option<AppHandle>,
    cancel: Arc<AtomicBool>,
) -> (TranscriptionQueue, std::thread::JoinHandle<()>) {
    let (tx, rx) = std_mpsc::sync_channel::<DecodeJob>(MAX_QUEUED_SEGMENTS);
    let counters = Arc::new(QueueCounters::default());

    let queue = TranscriptionQueue {
        tx,
        counters: Arc::clone(&counters),
        app: app.clone(),
        meeting_id: config.meeting_id.clone(),
    };

    // The worker gets the counters, never the queue: see [`QueueCounters`].
    let handle = std::thread::Builder::new()
        .name("vox-meeting-transcribe".into())
        .spawn(move || {
            run_worker(config, engine, store, app, cancel, rx, counters);
        })
        .expect("spawning the meeting transcription thread");

    (queue, handle)
}

fn counts_of(counters: &QueueCounters) -> (u64, u64, u64) {
    (
        counters.queued.load(Ordering::SeqCst),
        counters.completed.load(Ordering::SeqCst),
        counters.dropped.load(Ordering::SeqCst),
    )
}

fn run_worker(
    config: WorkerConfig,
    engine: SttEngine,
    store: Arc<MeetingStore>,
    app: Option<AppHandle>,
    cancel: Arc<AtomicBool>,
    rx: std_mpsc::Receiver<DecodeJob>,
    counters: Arc<QueueCounters>,
) {
    for job in rx {
        if cancel.load(Ordering::SeqCst) {
            tracing::info!("meeting {}: transcription cancelled", config.meeting_id);
            break;
        }

        let outcome = decode_segment(&engine, &config, &job);
        counters.completed.fetch_add(1, Ordering::SeqCst);

        match outcome {
            DecodeOutcome::Kept(segment) => {
                if let Err(err) = store.append_segments(&config.meeting_id, std::slice::from_ref(&segment)) {
                    // A transcript that cannot be written is worth saying out
                    // loud: the meeting is still recording, and the user would
                    // otherwise find out at the end.
                    tracing::error!(
                        "meeting {}: could not persist segment {}: {}",
                        config.meeting_id,
                        segment.sequence,
                        err
                    );
                    emit_warning(
                        &app,
                        &config.meeting_id,
                        "storage",
                        "A transcript line could not be saved to the vault.",
                    );
                }
                if let Some(app) = &app {
                    let _ = app.emit(TRANSCRIPT_SEGMENT_EVENT, &segment);
                }
            }
            DecodeOutcome::Discarded => {}
            DecodeOutcome::Failed(message) => {
                tracing::warn!(
                    "meeting {}: segment {} failed to decode: {}",
                    config.meeting_id,
                    job.sequence,
                    message
                );
                emit_warning(
                    &app,
                    &config.meeting_id,
                    "decode",
                    "A segment of speech could not be transcribed and is missing from the \
                     transcript. The audio recording still contains it.",
                );
            }
        }

        emit_progress(&app, &config.meeting_id, &counters);
    }
    emit_progress(&app, &config.meeting_id, &counters);
    tracing::info!(
        "meeting {}: transcription worker finished ({:?})",
        config.meeting_id,
        counts_of(&counters)
    );
}

enum DecodeOutcome {
    Kept(TranscriptSegment),
    /// Decoded, but screened out as a hallucination or as empty.
    Discarded,
    Failed(String),
}

fn decode_segment(engine: &SttEngine, config: &WorkerConfig, job: &DecodeJob) -> DecodeOutcome {
    let samples = &job.segment.samples;
    let profile = speech_health::profile_speech(samples, SEGMENT_SAMPLE_RATE);

    // The segmenter already decided this was speech; this second measurement
    // is about *how much* of the span is voice, which is what the
    // hallucination screen needs to judge the decode against.
    let result = engine.transcribe_utterances_with_config(
        Some(&config.model_path),
        samples,
        &config.language,
        &config.decoding,
    );

    let utterances = match result {
        Ok((utterances, _diagnostics)) => utterances,
        Err(err) => return DecodeOutcome::Failed(err.to_string()),
    };

    let text = join_utterance_text(&utterances);
    if text.trim().is_empty() {
        return DecodeOutcome::Discarded;
    }

    let mean_no_speech_prob = if utterances.is_empty() {
        1.0
    } else {
        utterances.iter().map(|u| u.no_speech_prob).sum::<f32>() / utterances.len() as f32
    };

    let evidence = DecodeEvidence {
        voiced_seconds: profile.voiced_seconds,
        total_seconds: profile.total_seconds,
        mean_no_speech_prob,
    };
    if let Some(reason) = speech_health::screen_decode(
        "meeting-segment",
        config.language.whisper_language.as_deref(),
        &text,
        evidence,
    ) {
        tracing::debug!(
            "meeting {}: discarded segment {} — {}",
            config.meeting_id,
            job.sequence,
            reason.describe()
        );
        return DecodeOutcome::Discarded;
    }

    let normalized = crate::capture::text_normalize::normalize_segment_text(&text, &config.glossary);

    DecodeOutcome::Kept(TranscriptSegment {
        sequence: job.sequence,
        text: normalized.text,
        start_seconds: job.segment.start_seconds,
        end_seconds: job.segment.end_seconds,
        channel: job.segment.channel,
        no_speech_prob: mean_no_speech_prob,
        recorded_at: chrono::Utc::now().to_rfc3339(),
    })
}

fn emit_progress(app: &Option<AppHandle>, meeting_id: &str, counters: &QueueCounters) {
    let Some(app) = app else { return };
    let (queued, completed, dropped) = counts_of(counters);
    let _ = app.emit(
        TRANSCRIPTION_PROGRESS_EVENT,
        TranscriptionProgress {
            meeting_id: meeting_id.to_string(),
            queued,
            completed,
            dropped,
            in_queue: queued.saturating_sub(completed),
        },
    );
}

fn emit_warning(app: &Option<AppHandle>, meeting_id: &str, kind: &str, message: &str) {
    let Some(app) = app else { return };
    let _ = app.emit(
        TRANSCRIPTION_WARNING_EVENT,
        TranscriptionWarning {
            meeting_id: meeting_id.to_string(),
            kind: kind.to_string(),
            message: message.to_string(),
        },
    );
}

/// Renders a transcript for a reader or for a language model.
///
/// Lines are prefixed with their channel and timestamp. The channel label is
/// what makes a two-sided conversation legible to a summarizer: without it the
/// model has to guess who proposed something and who agreed to it, and it
/// guesses wrong.
pub fn render_transcript(segments: &[TranscriptSegment]) -> String {
    let mut out = String::new();
    let mut last_channel: Option<SegmentChannel> = None;
    for segment in segments {
        let text = segment.text.trim();
        if text.is_empty() {
            continue;
        }
        if !out.is_empty() {
            out.push('\n');
        }
        // Repeat the speaker label only when it changes, the way a transcript
        // reads rather than the way a log does.
        if last_channel != Some(segment.channel) {
            out.push_str(&format!(
                "[{}] {}: {}",
                format_timestamp(segment.start_seconds),
                segment.channel.label(),
                text
            ));
            last_channel = Some(segment.channel);
        } else {
            out.push_str(&format!(
                "[{}] {}",
                format_timestamp(segment.start_seconds),
                text
            ));
        }
    }
    out
}

/// `mm:ss`, or `h:mm:ss` once a meeting passes an hour.
pub fn format_timestamp(seconds: f64) -> String {
    let total = seconds.max(0.0) as u64;
    let hours = total / 3600;
    let minutes = (total % 3600) / 60;
    let secs = total % 60;
    if hours > 0 {
        format!("{hours}:{minutes:02}:{secs:02}")
    } else {
        format!("{minutes:02}:{secs:02}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn segment(sequence: u64, channel: SegmentChannel, start: f64, text: &str) -> TranscriptSegment {
        TranscriptSegment {
            sequence,
            text: text.to_string(),
            start_seconds: start,
            end_seconds: start + 2.0,
            channel,
            no_speech_prob: 0.02,
            recorded_at: "2026-01-01T00:00:00Z".into(),
        }
    }

    #[test]
    fn timestamps_grow_a_leading_hour_only_when_there_is_one() {
        assert_eq!(format_timestamp(0.0), "00:00");
        assert_eq!(format_timestamp(9.6), "00:09");
        assert_eq!(format_timestamp(75.0), "01:15");
        assert_eq!(format_timestamp(3_661.0), "1:01:01");
        assert_eq!(format_timestamp(-5.0), "00:00");
    }

    #[test]
    fn a_rendered_transcript_names_the_speaker_when_it_changes() {
        let segments = vec![
            segment(0, SegmentChannel::Microphone, 0.0, "shall we start"),
            segment(1, SegmentChannel::Microphone, 3.0, "everyone here"),
            segment(2, SegmentChannel::System, 6.0, "yes go ahead"),
        ];
        let rendered = render_transcript(&segments);
        let lines: Vec<&str> = rendered.lines().collect();
        assert_eq!(lines[0], "[00:00] You: shall we start");
        assert_eq!(lines[1], "[00:03] everyone here", "label should not repeat");
        assert_eq!(lines[2], "[00:06] Others: yes go ahead");
    }

    #[test]
    fn rendering_skips_empty_lines_rather_than_emitting_blank_turns() {
        let segments = vec![
            segment(0, SegmentChannel::Mixed, 0.0, "one"),
            segment(1, SegmentChannel::Mixed, 2.0, "   "),
            segment(2, SegmentChannel::Mixed, 4.0, "two"),
        ];
        assert_eq!(render_transcript(&segments).lines().count(), 2);
    }

    #[test]
    fn rendering_nothing_produces_nothing() {
        assert_eq!(render_transcript(&[]), "");
    }

    #[test]
    fn dropping_the_queue_ends_the_worker() {
        // The stop path drops every queue clone and then joins the worker. If
        // the worker holds a sender of its own, its channel never closes and
        // that join blocks forever — which is a hang on every stop, not a
        // slow one.
        use crate::capture::stt::{SttEngine, SttLanguageConfig, WhisperDecodingConfig};
        use std::sync::atomic::AtomicBool;

        let (queue, worker) = spawn_worker(
            WorkerConfig {
                meeting_id: "meeting-shutdown".into(),
                model_path: String::new(),
                language: SttLanguageConfig {
                    whisper_language: None,
                    translate: false,
                },
                decoding: WhisperDecodingConfig::default(),
                glossary: Vec::new(),
            },
            SttEngine::new(),
            Arc::new(crate::meetings::MeetingStore::new(
                std::env::temp_dir().join("vox-worker-shutdown"),
            )),
            None,
            Arc::new(AtomicBool::new(false)),
        );

        // A second clone, the way the capture pump holds one.
        let pump_clone = queue.clone();
        drop(queue);
        drop(pump_clone);

        let finished = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let _ = worker.join();
            let _ = finished.0.send(());
        });
        assert!(
            finished
                .1
                .recv_timeout(std::time::Duration::from_secs(5))
                .is_ok(),
            "the worker did not exit when its queue was dropped"
        );
    }
}
