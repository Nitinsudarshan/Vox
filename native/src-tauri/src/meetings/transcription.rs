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
use std::time::Instant;

use tauri::{AppHandle, Emitter};

use crate::capture::speech_health::{self, DecodeEvidence};
use crate::capture::stt::{
    join_utterance_text, SttEngine, SttLanguageConfig, SttSamplingStrategy, WhisperDecodingConfig,
};

use super::model::{SegmentTelemetry, TranscriptSegment};
use super::segmenter::{SpeechSegment, SEGMENT_SAMPLE_RATE};
use super::store::MeetingStore;
use super::telemetry;

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

/// Queue depth at which the decoder stops paying for the careful profile.
///
/// Well below [`MAX_QUEUED_SEGMENTS`], because the point is to act while there
/// is still runway. At the queue's own ceiling the only remaining move is to
/// refuse speech, and by then the backlog has been growing for minutes.
const SHED_PROFILE_AT_DEPTH: u64 = 8;

/// Depth the backlog has to come back down to before the careful profile
/// returns. Hysteresis, for the same reason [`crate::capture::stt::ScriptTracker`]
/// has it: a single threshold on a queue that hovers around it would switch
/// profile every other segment, and a transcript whose decode settings
/// alternate line by line is harder to read than either setting alone.
const RESTORE_PROFILE_AT_DEPTH: u64 = 2;

/// Whether the decoder is far enough behind to trade beam width for speed.
///
/// The queue already has an answer for a decoder that cannot keep up: refuse
/// the segment, count it, and tell the user. That is the right *last* resort
/// and a poor first one — the speech is gone, and what bought its loss was a
/// beam search the machine could not afford. This is the move in between.
/// Under pressure the worker decodes on the cheaper profile, which is less
/// accurate on exactly the audio whose accuracy is already weakest, and that
/// is still strictly better than the alternative: a worse line is a line, and
/// a dropped segment is silence in the transcript where someone was talking.
///
/// Note what this is *not* keyed on: not the model, not the language, not a
/// setting. It is keyed on the one measurement that says the decoder is losing
/// — how much work is waiting — so a machine fast enough for the careful
/// profile never leaves it, and a machine that is not gets a transcript rather
/// than a gap.
#[derive(Debug, Default, Clone, Copy)]
pub struct BacklogTracker {
    shedding: bool,
}

impl BacklogTracker {
    pub fn new() -> Self {
        Self::default()
    }

    /// Whether the next segment should use the cheaper profile.
    pub fn is_shedding(self) -> bool {
        self.shedding
    }

    /// Records the queue depth left behind by a finished segment, returning
    /// `true` when that changed which profile the next one gets.
    pub fn observe(&mut self, in_queue: u64) -> bool {
        let next = if self.shedding {
            in_queue > RESTORE_PROFILE_AT_DEPTH
        } else {
            in_queue >= SHED_PROFILE_AT_DEPTH
        };
        if next == self.shedding {
            return false;
        }
        self.shedding = next;
        true
    }
}

/// One segment waiting to be decoded.
#[derive(Debug)]
pub struct DecodeJob {
    pub sequence: u64,
    pub segment: SpeechSegment,
    /// When the segment entered the queue. The gap between this and the start
    /// of its decode is the backlog the user eventually waits out, and it is
    /// the only part of the latency that is invisible from inside the decoder.
    pub submitted_at: Instant,
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
    /// Deepest the backlog ever got. A peak near [`MAX_QUEUED_SEGMENTS`] means
    /// the decoder was close to dropping speech, which the totals alone hide
    /// once the queue drains again.
    peak_in_queue: AtomicU64,
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
        match self.tx.try_send(DecodeJob {
            sequence,
            segment,
            submitted_at: Instant::now(),
        }) {
            Ok(()) => {
                let queued = self.counters.queued.fetch_add(1, Ordering::SeqCst) + 1;
                let in_queue =
                    queued.saturating_sub(self.counters.completed.load(Ordering::SeqCst));
                self.counters
                    .peak_in_queue
                    .fetch_max(in_queue, Ordering::SeqCst);
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

/// One segment's latency, split into the phases that fail for different
/// reasons.
///
/// Kept apart rather than summed because each phase points somewhere else: a
/// long `queue_wait` is backlog, a long `lock_wait` is another surface holding
/// the model, a non-zero `model_load` is the engine's single model slot being
/// evicted, and a long `decode` is the model itself being too slow for the
/// machine. A single total cannot distinguish "the model is slow" from
/// "something else was using it", which is exactly the question worth asking.
#[derive(Debug, Default, Clone)]
struct SegmentTiming {
    audio_seconds: f64,
    /// Seconds of the span that cleared the voiced threshold, and the span's
    /// own length. Kept because they are the evidence `speech_health` judged
    /// the decode against — without them a rejection is unexplainable after
    /// the fact, and a kept segment cannot be checked either.
    voiced_seconds: f64,
    total_seconds: f64,
    /// Whisper's own no-speech probability. `None` before a decode has
    /// produced one, and never a stand-in.
    no_speech_prob: Option<f32>,
    queue_wait_ms: u128,
    lock_wait_ms: u128,
    model_load_ms: u128,
    model_reloaded: bool,
    decode_ms: u128,
    /// Hallucination screen plus text normalisation.
    post_ms: u128,
    persist_ms: u128,
    /// The encoder clamp this segment decoded against, `None` for whisper's
    /// full thirty-second window.
    audio_ctx: Option<i32>,
    /// Whether this segment decoded with the cheaper profile because of the
    /// script it was written in.
    expensive_script: bool,
    /// Whether it decoded with the cheaper profile because the queue was
    /// backed up. Separate from [`Self::expensive_script`] rather than folded
    /// into one "cheap profile" flag: the two are answers to different
    /// questions, and a run where the second is set is a machine report rather
    /// than a language one.
    shedding_backlog: bool,
    /// The language whisper decoded under, as whisper reports it — the pinned
    /// one where the meeting pinned a language, and the one it detected for
    /// itself otherwise.
    ///
    /// Not the same question as `WorkerConfig::language`, which is what was
    /// *asked for*. A long-form meeting deliberately asks for nothing so
    /// whisper can follow a bilingual room, and the answer it gives is then
    /// the only record of why a line came out the way it did: a span decoded
    /// under the wrong language returns fluent text in that language rather
    /// than an error, so the transcript itself never looks broken.
    detected_language: Option<String>,
}

impl SegmentTiming {
    /// Decode time against the audio decoded — the model's own speed, with no
    /// queueing in it.
    fn decode_rtf(&self) -> f64 {
        if self.audio_seconds > 0.0 {
            (self.decode_ms as f64 / 1000.0) / self.audio_seconds
        } else {
            0.0
        }
    }
}

/// What a meeting's transcription cost, accumulated across every segment.
///
/// Returned by the worker thread so the stop path can report it next to the
/// numbers only it knows: how long the meeting ran, and how long the user
/// waited after pressing stop.
#[derive(Debug, Default, Clone)]
pub struct TranscriptionStats {
    pub model_path: String,
    pub strategy: String,
    pub language: String,
    pub threads: String,

    pub decoded: u64,
    pub kept: u64,
    pub discarded: u64,
    pub failed: u64,

    pub speech_seconds: f64,
    pub decode_ms: u128,
    pub queue_wait_ms: u128,
    pub lock_wait_ms: u128,
    pub model_load_ms: u128,
    pub model_reloads: u64,
    pub post_ms: u128,
    pub persist_ms: u128,

    pub peak_in_queue: u64,
    pub dropped: u64,

    /// Worst single-segment decode RTF, and which segment it was. Outliers
    /// here are where whisper's temperature fallback re-decoded a segment
    /// several times over, which the mean hides.
    pub slowest_rtf: f64,
    pub slowest_sequence: u64,

    /// Every profile change, as (first segment on the new profile, whether
    /// that profile is the cheaper one). Empty when the meeting stayed on the
    /// careful profile throughout, which is the English case.
    ///
    /// A list rather than a flag because a meeting that turns to another
    /// language and back is the case this tracking exists for, and a single
    /// switch point could not describe it.
    pub switches: Vec<(u64, bool)>,
    pub segments_on_cheap_profile: u64,

    /// Every time the queue depth moved the profile, as (first segment on the
    /// new profile, whether that profile is the cheaper one).
    ///
    /// Empty on a machine that kept up, which is the outcome worth aiming for.
    /// A non-empty list is the honest form of "your machine cannot decode this
    /// meeting at the quality you asked for" — and the segments it covers are
    /// the ones that would otherwise have been refused outright.
    pub backlog_switches: Vec<(u64, bool)>,
    pub segments_shed_by_backlog: u64,
}

impl TranscriptionStats {
    /// Decode time against the *speech* decoded. The model's raw speed.
    pub fn decode_rtf(&self) -> f64 {
        if self.speech_seconds > 0.0 {
            (self.decode_ms as f64 / 1000.0) / self.speech_seconds
        } else {
            0.0
        }
    }

    /// Decode time against wall-clock recording time — the number that decides
    /// whether the backlog grows.
    ///
    /// Lower than [`Self::decode_rtf`] by however much of the meeting was
    /// silence the segmenter never queued. This is the one the drain
    /// projection uses: audio arrives at wall-clock rate, not at speech rate.
    pub fn pipeline_rtf(&self, recording_seconds: f64) -> f64 {
        if recording_seconds > 0.0 {
            (self.decode_ms as f64 / 1000.0) / recording_seconds
        } else {
            0.0
        }
    }
}

/// Everything the worker needs that does not change during a meeting.
pub struct WorkerConfig {
    pub meeting_id: String,
    pub model_path: String,
    pub language: SttLanguageConfig,
    pub decoding: WhisperDecodingConfig,
    /// The same configuration wound back to what a machine under pressure can
    /// afford — see [`WhisperDecodingConfig::for_expensive_script`].
    ///
    /// Held alongside rather than replacing it, because which one a segment
    /// wants is not known until the meeting is under way: the careful profile
    /// costs an English meeting almost nothing and is worth keeping, and the
    /// cheap one is worth its accuracy only where the cost it avoids is
    /// actually being paid.
    ///
    /// Two things reach for it, and they are different questions. A run of
    /// segments in a script whisper writes expensively moves
    /// [`crate::capture::stt::ScriptTracker`], which is about what is being
    /// spoken. A growing queue moves [`BacklogTracker`], which is about
    /// whether this machine is keeping up with it. Either one is enough; the
    /// diagnostics record which.
    pub decoding_cheap: WhisperDecodingConfig,
    /// Words the user has told Vox about, applied to every decoded segment.
    pub glossary: Vec<String>,
    /// Domain vocabulary engine with prioritized global, meeting, and user terms.
    pub vocabulary: crate::capture::vocabulary::DomainVocabulary,
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
) -> (TranscriptionQueue, std::thread::JoinHandle<TranscriptionStats>) {
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
            run_worker(config, engine, store, app, cancel, rx, counters)
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
) -> TranscriptionStats {
    let mut stats = TranscriptionStats {
        model_path: config.model_path.clone(),
        strategy: format!("{:?}", config.decoding.strategy),
        language: config
            .language
            .whisper_language
            .clone()
            .unwrap_or_else(|| "auto-detect".to_string()),
        threads: config
            .decoding
            .n_threads
            .map(|n| n.to_string())
            .unwrap_or_else(|| "default".to_string()),
        ..TranscriptionStats::default()
    };

    // Starts careful and stays there for an all-Latin meeting; a run of
    // segments in another script moves it, and a run back moves it back.
    let mut script = crate::capture::stt::ScriptTracker::new();

    // And starts careful and stays there on a machine that keeps up. This one
    // reads the queue rather than the transcript.
    let mut backlog = BacklogTracker::new();

    let mut prev_kept_text: Option<String> = None;

    for job in rx {
        if cancel.load(Ordering::SeqCst) {
            tracing::info!("meeting {}: transcription cancelled", config.meeting_id);
            break;
        }

        let sequence = job.sequence;
        let (outcome, mut timing) = decode_segment(
            &engine,
            &config,
            &job,
            script.is_expensive(),
            backlog.is_shedding(),
            prev_kept_text.as_deref(),
        );

        // What the decoder has been writing decides how the *next* segment is
        // decoded. Read from the transcript rather than from a settings page:
        // a bilingual profile says what someone might speak, not what this
        // meeting is, so someone set up for English and Hindi who holds an
        // English meeting keeps the beam they can afford.
        //
        // Only a kept segment counts. One screened as a hallucination says
        // nothing about which language is being spoken.
        if let DecodeOutcome::Kept(segment) = &outcome {
            prev_kept_text = Some(segment.text.clone());
            if script.observe(&segment.text) {
                let now_expensive = script.is_expensive();
                tracing::info!(
                    "meeting {}: switching to the {} decode profile from segment {}",
                    config.meeting_id,
                    if now_expensive { "faster" } else { "careful" },
                    sequence + 1
                );
                stats.switches.push((sequence + 1, now_expensive));
            }
        } else {
            // Discarded or failed: do not propagate context into subsequent segment
            prev_kept_text = None;
        }
        counters.completed.fetch_add(1, Ordering::SeqCst);

        let mut status = telemetry::SegmentStatus::Discarded;
        let mut rejection: Option<String> = None;
        let mut error: Option<String> = None;
        let mut text_chars = 0usize;

        let outcome_label = match outcome {
            DecodeOutcome::Kept(segment) => {
                let t_persist = Instant::now();
                let persist_result =
                    store.append_segments(&config.meeting_id, std::slice::from_ref(&segment));
                timing.persist_ms = t_persist.elapsed().as_millis();
                if let Err(err) = persist_result {
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
                stats.kept += 1;
                status = telemetry::SegmentStatus::Kept;
                text_chars = segment.text.chars().count();
                format!("kept ({text_chars} chars)")
            }
            DecodeOutcome::Discarded(reason) => {
                stats.discarded += 1;
                rejection = reason.map(str::to_string);
                match reason {
                    Some(reason) => format!("discarded ({reason})"),
                    None => "discarded (empty decode)".to_string(),
                }
            }
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
                stats.failed += 1;
                status = telemetry::SegmentStatus::Failed;
                error = Some(message.clone());
                format!("FAILED — {message}")
            }
        };

        let in_queue = {
            let (queued, completed, _) = counts_of(&counters);
            queued.saturating_sub(completed)
        };
        if backlog.observe(in_queue) {
            let now_shedding = backlog.is_shedding();
            tracing::info!(
                "meeting {}: decode queue is {} segments deep — {} from segment {}",
                config.meeting_id,
                in_queue,
                if now_shedding {
                    "dropping to the faster decode profile to stop the backlog growing"
                } else {
                    "caught up, returning to the careful decode profile"
                },
                sequence + 1
            );
            stats.backlog_switches.push((sequence + 1, now_shedding));
        }
        accumulate(&mut stats, &timing);
        if timing.decode_rtf() > stats.slowest_rtf {
            stats.slowest_rtf = timing.decode_rtf();
            stats.slowest_sequence = sequence;
        }
        print_segment_trace(&config.meeting_id, sequence, &timing, &outcome_label, in_queue);

        // The trace above is for a person watching a run. This is for every
        // question that can only be answered afterwards — which is most of
        // them. A diagnostics write that fails must not cost the transcript,
        // so it is logged and the meeting carries on.
        let record = build_diagnostics(
            &config,
            &job,
            &timing,
            status,
            rejection,
            error,
            text_chars,
            in_queue,
        );
        if let Err(err) = store.append_segment_diagnostics(&config.meeting_id, &record) {
            tracing::warn!(
                "meeting {}: could not record diagnostics for segment {}: {}",
                config.meeting_id,
                sequence,
                err
            );
        }

        emit_progress(&app, &config.meeting_id, &counters);
    }
    emit_progress(&app, &config.meeting_id, &counters);
    let (_, completed, dropped) = counts_of(&counters);
    stats.decoded = completed;
    stats.dropped = dropped;
    stats.peak_in_queue = counters.peak_in_queue.load(Ordering::SeqCst);
    tracing::info!(
        "meeting {}: transcription worker finished ({:?})",
        config.meeting_id,
        counts_of(&counters)
    );
    stats
}

/// Turns one decode's measurements into the record that outlives the run.
///
/// Every field comes from something already measured; nothing here computes a
/// new number and nothing invents one. In particular there is no confidence:
/// what Whisper reports is `no_speech_prob` and that is what is stored.
#[allow(clippy::too_many_arguments)] // One segment's facts, with no natural grouping.
fn build_diagnostics(
    config: &WorkerConfig,
    job: &DecodeJob,
    timing: &SegmentTiming,
    status: telemetry::SegmentStatus,
    rejection: Option<String>,
    error: Option<String>,
    text_chars: usize,
    queue_depth_after: u64,
) -> telemetry::SegmentDiagnostics {
    telemetry::SegmentDiagnostics {
        version: telemetry::DIAGNOSTICS_VERSION,
        sequence: job.sequence,
        start_seconds: job.segment.start_seconds,
        end_seconds: job.segment.end_seconds,
        channel: job.segment.channel,
        forced_split: job.segment.forced_split(),
        end_reason: job.segment.end_reason,
        hangover_ms: job.segment.hangover_ms,
        voiced_seconds: timing.voiced_seconds,
        total_seconds: timing.total_seconds,
        no_speech_prob: timing.no_speech_prob,
        queue_wait_ms: timing.queue_wait_ms,
        lock_wait_ms: timing.lock_wait_ms,
        model_load_ms: timing.model_load_ms,
        model_reloaded: timing.model_reloaded,
        decode_ms: timing.decode_ms,
        post_ms: timing.post_ms,
        persist_ms: timing.persist_ms,
        // The filename, never the path: a diagnostics file gets shared and a
        // path names a machine and usually a person.
        model: model_name(&config.model_path),
        language: config.language.whisper_language.clone(),
        detected_language: timing.detected_language.clone(),
        expensive_script_profile: timing.expensive_script,
        backlog_shedding: timing.shedding_backlog,
        audio_ctx: timing.audio_ctx,
        status,
        rejection,
        error,
        text_chars,
        queue_depth_after,
    }
}

/// The model's filename, for a record that will be read on another machine.
pub fn model_name(model_path: &str) -> String {
    std::path::Path::new(model_path)
        .file_name()
        .map(|name| name.to_string_lossy().to_string())
        .unwrap_or_else(|| model_path.to_string())
}

fn accumulate(stats: &mut TranscriptionStats, timing: &SegmentTiming) {
    if timing.expensive_script {
        stats.segments_on_cheap_profile += 1;
    }
    if timing.shedding_backlog {
        stats.segments_shed_by_backlog += 1;
    }
    stats.speech_seconds += timing.audio_seconds;
    stats.decode_ms += timing.decode_ms;
    stats.queue_wait_ms += timing.queue_wait_ms;
    stats.lock_wait_ms += timing.lock_wait_ms;
    stats.model_load_ms += timing.model_load_ms;
    stats.post_ms += timing.post_ms;
    stats.persist_ms += timing.persist_ms;
    if timing.model_reloaded {
        stats.model_reloads += 1;
    }
}

/// Prints one segment's latency breakdown to the terminal.
///
/// Deliberately `println!` rather than `tracing`: this is the meetings
/// counterpart of the dictation latency trace, and it is read by a person
/// watching a run, not scraped out of a log file afterwards.
fn print_segment_trace(
    meeting_id: &str,
    sequence: u64,
    timing: &SegmentTiming,
    outcome: &str,
    in_queue: u64,
) {
    let total_ms = timing.queue_wait_ms
        + timing.lock_wait_ms
        + timing.model_load_ms
        + timing.decode_ms
        + timing.post_ms
        + timing.persist_ms;

    println!();
    println!("==================================================");
    println!("MEETING SEGMENT TRACE  [seq {sequence}]");
    println!("--------------------------------------------------");
    println!("meeting              : {meeting_id}");
    println!("audio_duration       : {:.2} s", timing.audio_seconds);
    println!();
    println!("queue_wait           : {} ms", timing.queue_wait_ms);
    println!("lock_wait            : {} ms", timing.lock_wait_ms);
    println!(
        "model_load           : {} ms{}",
        timing.model_load_ms,
        if timing.model_reloaded {
            "   <-- evicted another model"
        } else {
            ""
        }
    );
    println!(
        "encoder_window       : {}",
        match timing.audio_ctx {
            // Shown as the span it covers, since that is what has to be at
            // least as long as the audio above for nothing to be lost.
            Some(ctx) => format!(
                "{:.1} s (clamped to {ctx} of {} positions)",
                ctx as f32 / (crate::capture::stt::FULL_AUDIO_CTX as f32 / 30.0),
                crate::capture::stt::FULL_AUDIO_CTX
            ),
            None => "30.0 s (full window)".to_string(),
        }
    );
    println!(
        "decode profile       : {}",
        match (timing.expensive_script, timing.shedding_backlog) {
            (true, true) => "fast (non-Latin script, and behind: greedy, no fallback)",
            (true, false) => "fast (non-Latin script: greedy, no fallback)",
            (false, true) => "fast (decode queue backed up: greedy, no fallback)",
            (false, false) => "careful (beam search)",
        }
    );
    println!("decode               : {} ms", timing.decode_ms);
    println!("screen + normalize   : {} ms", timing.post_ms);
    println!("persist              : {} ms", timing.persist_ms);
    println!("TOTAL                : {total_ms} ms");
    println!();
    println!("decode_RTF           : {:.3}", timing.decode_rtf());
    println!("queue_depth_after    : {in_queue}");
    println!("outcome              : {outcome}");
    println!("==================================================");
}

/// How long the backlog will take to clear after a meeting of `length`.
///
/// Audio arrives at wall-clock rate and is consumed at `1 / pipeline_rtf` of
/// it, so over a meeting of length `L` the backlog reaches `L * (rtf - 1)` and
/// takes exactly that long to drain. Returned in whatever unit `length` is
/// given in.
///
/// At or below 1.0 the decoder consumes at least as fast as audio arrives,
/// nothing accumulates, and the user waits only for the segment still in
/// flight — which is why the whole point of tuning here is to cross 1.0, not
/// to chase a large multiple of real time.
fn projected_drain(pipeline_rtf: f64, length: f64) -> f64 {
    if pipeline_rtf > 1.0 {
        (pipeline_rtf - 1.0) * length
    } else {
        0.0
    }
}

/// Prints the whole meeting's transcription cost once the decoder has drained.
///
/// `recording_seconds` and `drain_seconds` come from the stop path because
/// only it knows them: the worker sees segments, not the meeting.
pub fn print_meeting_summary(
    meeting_id: &str,
    stats: &TranscriptionStats,
    recording_seconds: f64,
    drain_seconds: f64,
) {
    let pipeline_rtf = stats.pipeline_rtf(recording_seconds);
    let projected_60min = projected_drain(pipeline_rtf, 60.0);
    let speech_share = if recording_seconds > 0.0 {
        (stats.speech_seconds / recording_seconds) * 100.0
    } else {
        0.0
    };

    println!();
    println!("==================================================");
    println!("MEETING TRANSCRIPTION SUMMARY");
    println!("--------------------------------------------------");
    println!("meeting              : {meeting_id}");
    println!("model                : {}", stats.model_path);
    println!("strategy             : {}", stats.strategy);
    println!(
        "decode profile       : {}",
        if stats.switches.is_empty() {
            "careful throughout (no non-Latin script seen)".to_string()
        } else {
            let path = stats
                .switches
                .iter()
                .map(|(seq, expensive)| {
                    format!(" -> {} @seq {seq}", if *expensive { "fast" } else { "careful" })
                })
                .collect::<String>();
            format!(
                "careful{path}   ({} of {} segments on fast)",
                stats.segments_on_cheap_profile, stats.decoded
            )
        }
    );
    println!(
        "backlog shedding     : {}",
        if stats.backlog_switches.is_empty() {
            "never (the decoder kept up with the queue)".to_string()
        } else {
            let path = stats
                .backlog_switches
                .iter()
                .map(|(seq, shedding)| {
                    format!(
                        " -> {} @seq {seq}",
                        if *shedding { "fast" } else { "careful" }
                    )
                })
                .collect::<String>();
            format!(
                "careful{path}   ({} of {} segments decoded cheaply to stay ahead)",
                stats.segments_shed_by_backlog, stats.decoded
            )
        }
    );
    println!("language             : {}", stats.language);
    println!("threads              : {}", stats.threads);
    println!();
    println!(
        "segments             : {} decoded (kept {}, discarded {}, failed {})",
        stats.decoded, stats.kept, stats.discarded, stats.failed
    );
    println!("dropped (queue full) : {}", stats.dropped);
    println!("peak queue depth     : {}", stats.peak_in_queue);
    println!();
    println!("recording duration   : {recording_seconds:.1} s");
    println!(
        "speech decoded       : {:.1} s  ({speech_share:.0}% of the meeting)",
        stats.speech_seconds
    );
    println!("decode total         : {:.1} s", stats.decode_ms as f64 / 1000.0);
    println!("queue wait total     : {:.1} s", stats.queue_wait_ms as f64 / 1000.0);
    println!(
        "lock wait total      : {:.1} s{}",
        stats.lock_wait_ms as f64 / 1000.0,
        if stats.lock_wait_ms > 1000 {
            "   <-- contended with dictation"
        } else {
            ""
        }
    );
    println!(
        "model load total     : {:.1} s  (reloads: {}){}",
        stats.model_load_ms as f64 / 1000.0,
        stats.model_reloads,
        if stats.model_reloads > 0 {
            "   <-- model thrash"
        } else {
            ""
        }
    );
    println!("screen + normalize   : {:.1} s", stats.post_ms as f64 / 1000.0);
    println!("persist total        : {:.1} s", stats.persist_ms as f64 / 1000.0);
    println!();
    println!("decode_RTF (speech)  : {:.3}", stats.decode_rtf());
    println!(
        "pipeline_RTF (clock) : {pipeline_rtf:.3}   <-- below 1.0 keeps up, above 1.0 falls behind"
    );
    println!(
        "slowest segment RTF  : {:.3}  [seq {}]",
        stats.slowest_rtf, stats.slowest_sequence
    );
    println!();
    println!("drain after stop     : {drain_seconds:.1} s   <-- what the user actually waited");
    if projected_60min > 0.0 {
        println!("projected 60-min wait: {projected_60min:.1} min at this pipeline_RTF");
    } else {
        println!("projected 60-min wait: ~0 min (decoder keeps up with the meeting)");
    }
    println!("==================================================");
}

enum DecodeOutcome {
    /// Boxed because a kept segment now carries its own decode telemetry,
    /// which makes it far larger than the other two variants.
    Kept(Box<TranscriptSegment>),
    /// Decoded, but screened out as a hallucination or as empty. Carries the
    /// `speech_health` reason key where there was one — an empty decode has
    /// none, and "empty" and "looped" need different fixes.
    Discarded(Option<&'static str>),
    Failed(String),
}

/// The alternate decode a suspicious segment is retried with.
///
/// Everything that can condition a decode into repeating itself comes out: the
/// prompt that may have suggested the loop, the beam that may have been
/// exploring it, and the temperature fallback that re-decodes a segment
/// whisper is unsure of up to six times over.
///
/// A function rather than four lines inline so the worker can ask the question
/// that decides whether to pay for it — whether this differs from the decode
/// that just ran. On the profile kept for scripts whisper writes expensively it
/// frequently does not: that profile is already greedy with no fallback, so on
/// a segment that carried no prompt the "alternate" decode is the same decode.
fn recovery_config(base: &WhisperDecodingConfig) -> WhisperDecodingConfig {
    let mut cfg = base.clone();
    cfg.initial_prompt = None;
    cfg.strategy = SttSamplingStrategy::Greedy { best_of: 1 };
    cfg.temperature = 0.0;
    cfg.temperature_inc = 0.0;
    cfg
}

/// Why a suspicious segment is *not* being re-decoded, or `None` to go ahead.
///
/// The recovery pass buys one segment a second chance at the price of a second
/// full decode. Both reasons to decline it are about that price.
///
/// - **It would change nothing.** The alternate configuration is compared with
///   the one that produced the suspicion. Greedy at temperature zero with no
///   fallback is deterministic, so an identical configuration over identical
///   audio returns identical text: a second copy of the same suspicion, for a
///   second decode's cost. This is the case where no prompt was built — with
///   whisper's `prompt_past` off, an empty prompt is the only conditioning
///   there is to remove.
/// - **The meeting cannot afford it.** When the queue is backed up enough that
///   the worker has already given up beam width to keep pace
///   ([`BacklogTracker`]), a second decode of one segment is paid for by
///   whichever later segment the queue then refuses. Recovery is an investment
///   in one line's quality; a dropped segment is another line missing
///   entirely, and the trade does not come close.
fn skip_recovery(
    recovery: &WhisperDecodingConfig,
    used: &WhisperDecodingConfig,
    shedding_backlog: bool,
) -> Option<&'static str> {
    if recovery == used {
        return Some("the alternate decode is the decode that produced it");
    }
    if shedding_backlog {
        return Some("the decode queue is backed up and a second pass costs later speech");
    }
    None
}

fn decode_segment(
    engine: &SttEngine,
    config: &WorkerConfig,
    job: &DecodeJob,
    expensive_script: bool,
    shedding_backlog: bool,
    prev_context: Option<&str>,
) -> (DecodeOutcome, SegmentTiming) {
    let samples = &job.segment.samples;

    // Either reason is enough on its own, and they are recorded separately —
    // the decode does not care which of them asked for the cheaper profile,
    // and the person reading the diagnostics afterwards very much does.
    let cheap_profile = expensive_script || shedding_backlog;
    let base_decoding = if cheap_profile {
        &config.decoding_cheap
    } else {
        &config.decoding
    };

    let mut decoding = base_decoding.clone();
    // The prompt the user configured in settings is an *input* here, not
    // something to replace: it used to be assembled by
    // `WhisperDecodingConfig::from_settings_defaulting` and then overwritten
    // one line later, so a meeting silently decoded without it while dictation
    // decoded with it.
    if let Some(prompt) = config.vocabulary.build_prompt(
        base_decoding.initial_prompt.as_deref(),
        prev_context,
        job.segment.forced_split(),
        crate::capture::vocabulary::PROMPT_BUDGET_CHARS,
    ) {
        decoding.initial_prompt = Some(prompt);
    }

    let mut timing = SegmentTiming {
        audio_seconds: job.segment.duration_seconds(),
        queue_wait_ms: job.submitted_at.elapsed().as_millis(),
        expensive_script,
        shedding_backlog,
        ..SegmentTiming::default()
    };

    let profile = speech_health::profile_speech(samples, SEGMENT_SAMPLE_RATE);
    timing.voiced_seconds = profile.voiced_seconds;
    timing.total_seconds = profile.total_seconds;

    // The segmenter already decided this was speech; this second measurement
    // is about *how much* of the span is voice, which is what the
    // hallucination screen needs to judge the decode against.
    let result = engine.transcribe_utterances_with_config(
        Some(&config.model_path),
        samples,
        &config.language,
        &decoding,
    );

    let utterances = match result {
        Ok((utterances, diagnostics)) => {
            // The engine reports the decode alone; the waiting it did before
            // that is reported separately so the two never get confused.
            timing.decode_ms = diagnostics.transcription_latency_ms;
            timing.lock_wait_ms = diagnostics.lock_wait_ms;
            timing.model_load_ms = diagnostics.model_load_ms;
            timing.model_reloaded = diagnostics.model_reloaded;
            timing.audio_ctx = diagnostics.audio_ctx;
            timing.detected_language = diagnostics.detected_language.clone();
            // Building whisper's decode state is outside the engine's own
            // decode timer, so a run that pays it per segment would otherwise
            // report a real-time factor that leaves the cost out. It is zero
            // on every decode that reused the resident state, which is every
            // decode but the first after a model load.
            timing.model_load_ms += diagnostics.state_create_ms;
            utterances
        }
        Err(err) => return (DecodeOutcome::Failed(err.to_string()), timing),
    };

    let t_post = Instant::now();

    let text = join_utterance_text(&utterances);
    if text.trim().is_empty() {
        timing.post_ms = t_post.elapsed().as_millis();
        return (DecodeOutcome::Discarded(None), timing);
    }

    let mean_no_speech_prob = if utterances.is_empty() {
        1.0
    } else {
        utterances.iter().map(|u| u.no_speech_prob).sum::<f32>() / utterances.len() as f32
    };

    timing.no_speech_prob = Some(mean_no_speech_prob);
    let evidence = DecodeEvidence {
        voiced_seconds: profile.voiced_seconds,
        total_seconds: profile.total_seconds,
        mean_no_speech_prob,
    };

    let (quality_verdict, compression_ratio) = speech_health::evaluate_segment_quality(&text, evidence);

    // Phase 1J: Deterministic recovery if segment is Suspicious
    let (final_text, final_no_speech_prob, final_compression, quality_status, retry_count) = match quality_verdict {
        speech_health::SegmentQualityStatus::Good => {
            (text, mean_no_speech_prob, compression_ratio, "good".to_string(), 0)
        }
        speech_health::SegmentQualityStatus::Suspicious(ref reason) => {
            // Alternate decode configuration: greedy decode, zero temperature increment, clear initial prompt
            let recovery_decoding = recovery_config(base_decoding);

            if let Some(why) = skip_recovery(&recovery_decoding, &decoding, shedding_backlog) {
                tracing::debug!(
                    "meeting {}: segment {} suspicious ({}) — not attempting recovery: {}",
                    config.meeting_id,
                    job.sequence,
                    reason,
                    why
                );
                (
                    text,
                    mean_no_speech_prob,
                    compression_ratio,
                    "suspicious".to_string(),
                    0,
                )
            } else if let Ok((rec_utterances, rec_diag)) = {
                tracing::info!(
                    "meeting {}: segment {} suspicious ({}) — attempting deterministic recovery pass",
                    config.meeting_id,
                    job.sequence,
                    reason
                );
                engine.transcribe_utterances_with_config(
                    Some(&config.model_path),
                    samples,
                    &config.language,
                    &recovery_decoding,
                )
            } {
                timing.decode_ms += rec_diag.transcription_latency_ms;
                let rec_text = join_utterance_text(&rec_utterances);
                let rec_no_speech = if rec_utterances.is_empty() {
                    1.0
                } else {
                    rec_utterances.iter().map(|u| u.no_speech_prob).sum::<f32>() / rec_utterances.len() as f32
                };
                let rec_evidence = DecodeEvidence {
                    voiced_seconds: profile.voiced_seconds,
                    total_seconds: profile.total_seconds,
                    mean_no_speech_prob: rec_no_speech,
                };
                let (rec_quality, rec_compression) = speech_health::evaluate_segment_quality(&rec_text, rec_evidence);

                if rec_quality.is_good() || (!rec_quality.is_rejected() && rec_no_speech < mean_no_speech_prob) {
                    tracing::info!(
                        "meeting {}: segment {} successfully recovered via alternate decode",
                        config.meeting_id,
                        job.sequence
                    );
                    (rec_text, rec_no_speech, rec_compression, "recovered".to_string(), 1)
                } else if !quality_verdict.is_rejected() {
                    // Retain initial attempt but record suspicious status
                    (text, mean_no_speech_prob, compression_ratio, "suspicious".to_string(), 1)
                } else {
                    timing.post_ms = t_post.elapsed().as_millis();
                    // The recovery decode was no better and the first was
                    // rejected: the reason the screen gave is the one that
                    // belongs in the diagnostics.
                    return (
                        DecodeOutcome::Discarded(quality_verdict.rejection_key()),
                        timing,
                    );
                }
            } else {
                (text, mean_no_speech_prob, compression_ratio, "suspicious".to_string(), 1)
            }
        }
        speech_health::SegmentQualityStatus::Rejected(ref reason) => {
            tracing::debug!(
                "meeting {}: discarded segment {} — {}",
                config.meeting_id,
                job.sequence,
                reason.describe()
            );
            timing.post_ms = t_post.elapsed().as_millis();
            return (DecodeOutcome::Discarded(Some(reason.key())), timing);
        }
    };

    if final_text.trim().is_empty() {
        timing.post_ms = t_post.elapsed().as_millis();
        // Nothing came back. There is no screening reason for that — "empty"
        // and "looped" are different failures with different fixes.
        return (DecodeOutcome::Discarded(None), timing);
    }

    // Normalization first, then the glossary — and the glossary's changes are
    // kept. It used to run inside the normalizer, rewriting words in place
    // before the segment was ever written, with nothing recording what had
    // been changed or from what. A correction that cannot name its own
    // evidence is indistinguishable from a transcription.
    let normalized = crate::capture::text_normalize::normalize_segment_text(&final_text, &[]);
    let glossary = crate::capture::glossary::Glossary::from_settings(&config.glossary);
    let (normalized_text, corrections) = glossary.apply(&normalized.text);
    let normalized = crate::capture::text_normalize::SegmentOutcome {
        text: normalized_text,
        applied_rules: normalized.applied_rules,
    };
    let (text, original_text, romanized_text) = if crate::capture::romanize::contains_devanagari(&normalized.text) {
        let romanized = crate::capture::romanize::to_latin(&normalized.text);
        (normalized.text.clone(), Some(normalized.text), Some(romanized))
    } else {
        (normalized.text, None, None)
    };
    timing.post_ms = t_post.elapsed().as_millis();

    let telemetry = SegmentTelemetry {
        sequence: job.sequence,
        start_seconds: job.segment.start_seconds,
        end_seconds: job.segment.end_seconds,
        duration_seconds: job.segment.duration_seconds(),
        speech_seconds: profile.voiced_seconds,
        channel: job.segment.channel,
        // What whisper decided, and what it was asked. They differ exactly
        // when the meeting pinned nothing, which is the long-form default —
        // and that is the case where knowing is worth the most.
        detected_language: timing.detected_language.clone(),
        decode_language: config.language.whisper_language.clone(),
        model: config.model_path.clone(),
        decode_profile: if cheap_profile { "fast".to_string() } else { "careful".to_string() },
        decode_ms: timing.decode_ms,
        rtf: timing.decode_rtf() as f32,
        queue_wait_ms: timing.queue_wait_ms,
        no_speech_probability: final_no_speech_prob,
        compression_ratio: final_compression,
        quality_status,
        retry_count,
        forced_split: job.segment.forced_split(),
        rms: job.segment.audio_stats.rms,
        peak_amplitude: job.segment.audio_stats.peak_amplitude,
        near_clipping_percent: job.segment.audio_stats.near_clipping_percent,
    };

    (
        DecodeOutcome::Kept(Box::new(TranscriptSegment {
            sequence: job.sequence,
            text,
            start_seconds: job.segment.start_seconds,
            end_seconds: job.segment.end_seconds,
            channel: job.segment.channel,
            no_speech_prob: final_no_speech_prob,
            recorded_at: chrono::Utc::now().to_rfc3339(),
            cut_at_ceiling: job.segment.forced_split(),
            original_text,
            romanized_text,
            translated_text: None,
            corrections,
            telemetry: Some(telemetry),
        })),
        timing,
    )
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
    render_transcript_internal(segments, false, &Default::default(), &[])
}

/// Renders a transcript preferring translated or romanized text over raw non-Latin text.
///
/// Useful when feeding the transcript into a summarizer generating an English report.
pub fn render_transcript_prefer_translated(segments: &[TranscriptSegment]) -> String {
    render_transcript_internal(segments, true, &Default::default(), &[])
}

/// As [`render_transcript_prefer_translated`], naming speakers where detection
/// has found them.
///
/// This is what lets a report say "Payal committed to sending the deck" rather
/// than "Others committed to sending the deck". Two remote participants are
/// both `Others` on the channel alone, so a summary built from channel labels
/// cannot attribute anything on a call with more than two people — it can only
/// report that somebody said something.
pub fn render_transcript_with_speakers(
    segments: &[TranscriptSegment],
    attribution: &crate::meetings::speakers::SpeakerAttribution,
    speakers: &[crate::meetings::model::Speaker],
) -> String {
    render_transcript_internal(segments, true, attribution, speakers)
}

fn render_transcript_internal(
    segments: &[TranscriptSegment],
    prefer_translated: bool,
    attribution: &crate::meetings::speakers::SpeakerAttribution,
    speakers: &[crate::meetings::model::Speaker],
) -> String {
    let mut out = String::new();
    let mut last_label: Option<String> = None;
    for segment in segments {
        let text_source = if prefer_translated {
            segment
                .translated_text
                .as_deref()
                .or(segment.romanized_text.as_deref())
                .unwrap_or(&segment.text)
        } else {
            &segment.text
        };
        let text = text_source.trim();
        if text.is_empty() {
            continue;
        }
        if !out.is_empty() {
            out.push('\n');
        }
        // Repeat the speaker label only when it changes, the way a transcript
        // reads rather than the way a log does.
        let label = crate::meetings::speakers::label_for(segment, attribution, speakers);
        if last_label.as_deref() != Some(label.as_str()) {
            out.push_str(&format!(
                "[{}] {}: {}",
                format_timestamp(segment.start_seconds),
                label,
                text
            ));
            last_label = Some(label);
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
    use super::super::model::SegmentChannel;
    use crate::capture::stt::SttPreset;
    use crate::settings::SttSettings;

    /// A machine that keeps up never leaves the careful profile, which is the
    /// case this must not disturb.
    #[test]
    fn a_decoder_that_keeps_up_never_shifts_profile() {
        let mut backlog = BacklogTracker::new();
        for depth in [0, 1, 0, 2, 1, 0, 1] {
            assert!(!backlog.observe(depth));
            assert!(!backlog.is_shedding());
        }
    }

    /// And one that falls behind gives up the beam before it gives up speech.
    #[test]
    fn a_growing_queue_moves_to_the_cheaper_profile() {
        let mut backlog = BacklogTracker::new();
        for depth in 0..SHED_PROFILE_AT_DEPTH {
            assert!(!backlog.observe(depth));
        }
        assert!(
            backlog.observe(SHED_PROFILE_AT_DEPTH),
            "the switch is reported on the segment that crossed the line"
        );
        assert!(backlog.is_shedding());
    }

    /// Coming back is not the mirror of going: the queue has to drain well
    /// past the threshold that triggered the switch, or a queue hovering at it
    /// would change decode settings every other line.
    #[test]
    fn the_careful_profile_returns_only_once_the_queue_has_really_drained() {
        let mut backlog = BacklogTracker::new();
        backlog.observe(SHED_PROFILE_AT_DEPTH);
        assert!(backlog.is_shedding());

        // Still above the restore mark, and still below the shed mark: the
        // band where a single threshold would oscillate.
        for depth in (RESTORE_PROFILE_AT_DEPTH + 1)..SHED_PROFILE_AT_DEPTH {
            assert!(!backlog.observe(depth), "depth {depth} must not restore");
            assert!(backlog.is_shedding());
        }

        assert!(backlog.observe(RESTORE_PROFILE_AT_DEPTH));
        assert!(!backlog.is_shedding());
    }

    /// The whole argument for this existing: the queue's own answer to a slow
    /// decoder is to refuse speech, and shedding acts long before that.
    #[test]
    fn shedding_starts_with_the_queue_still_mostly_empty() {
        // Read into locals so the assertion is over values rather than over
        // constants, and so a failure prints the numbers that broke it.
        let shed = SHED_PROFILE_AT_DEPTH;
        let restore = RESTORE_PROFILE_AT_DEPTH;
        let ceiling = MAX_QUEUED_SEGMENTS as u64;

        assert!(
            shed < ceiling / 4,
            "shedding at {shed} of a {ceiling}-deep queue would act after the damage"
        );
        assert!(restore < shed, "restore {restore} must sit below shed {shed}");
    }

    /// The recovery pass exists to break a decode out of whatever conditioned
    /// it into repeating itself. On the careful profile there is something to
    /// break: a beam, a temperature schedule, and a prompt.
    #[test]
    fn recovery_differs_from_the_careful_profile_it_retries() {
        let settings = SttSettings::default();
        let mut careful = WhisperDecodingConfig::for_meetings(&settings, SttPreset::Balanced);
        careful.initial_prompt = Some("NavGurukul, SOSC".to_string());

        assert_ne!(recovery_config(&careful), careful);
        assert!(skip_recovery(&recovery_config(&careful), &careful, false).is_none());
    }

    /// And a segment that would genuinely benefit from one still does not get
    /// it while the queue is backed up: the second decode is paid for by
    /// whichever later segment the queue then refuses.
    #[test]
    fn recovery_is_declined_while_the_decoder_is_behind() {
        let settings = SttSettings::default();
        let mut careful = WhisperDecodingConfig::for_meetings(&settings, SttPreset::Balanced);
        careful.initial_prompt = Some("NavGurukul, SOSC".to_string());

        assert!(skip_recovery(&recovery_config(&careful), &careful, true).is_some());
    }

    /// And on the profile kept for scripts whisper writes expensively there is
    /// not: it is already greedy with no fallback, so on a segment that
    /// carried no prompt the "alternate" decode is the decode that just ran.
    /// Paying for it twice is what the worker checks for.
    #[test]
    fn recovery_is_the_same_decode_on_the_cheap_profile_without_a_prompt() {
        let settings = SttSettings::default();
        let cheap = WhisperDecodingConfig::for_meetings(&settings, SttPreset::Balanced)
            .for_expensive_script();

        assert!(cheap.initial_prompt.is_none());
        assert_eq!(
            recovery_config(&cheap),
            cheap,
            "a retry of this configuration would return the same text for a second decode's cost"
        );
        assert!(skip_recovery(&recovery_config(&cheap), &cheap, false).is_some());
    }

    /// The same profile *with* a prompt is a real alternate: dropping the
    /// prompt is exactly the conditioning the retry is there to remove.
    #[test]
    fn recovery_still_differs_on_the_cheap_profile_when_a_prompt_was_used() {
        let settings = SttSettings::default();
        let mut cheap = WhisperDecodingConfig::for_meetings(&settings, SttPreset::Balanced)
            .for_expensive_script();
        cheap.initial_prompt = Some("we were talking about the migration".to_string());

        assert_ne!(recovery_config(&cheap), cheap);
        assert!(skip_recovery(&recovery_config(&cheap), &cheap, false).is_none());
    }

    /// The whole reason the summary reports `pipeline_RTF` rather than only
    /// the decode speed: a decoder can be slower than real time against the
    /// speech it decodes and still keep up with the meeting, because the
    /// segmenter never queues the silence.
    #[test]
    fn a_decoder_slower_than_speech_still_keeps_up_when_the_meeting_is_mostly_silence() {
        let stats = TranscriptionStats {
            // 120s of speech took 180s to decode: 1.5x slower than the speech.
            speech_seconds: 120.0,
            decode_ms: 180_000,
            ..TranscriptionStats::default()
        };

        assert!(
            stats.decode_rtf() > 1.0,
            "the model really is slower than the speech it decoded"
        );
        // But the meeting ran for 600s, most of which nobody was talking.
        assert!(
            stats.pipeline_rtf(600.0) < 1.0,
            "against wall-clock the decoder still keeps up"
        );
        assert_eq!(projected_drain(stats.pipeline_rtf(600.0), 60.0), 0.0);
    }

    /// The observed case this instrumentation was built to explain: a five
    /// minute meeting that took another five minutes to finish transcribing.
    #[test]
    fn a_pipeline_rtf_of_two_makes_the_wait_as_long_as_the_meeting() {
        let stats = TranscriptionStats {
            speech_seconds: 300.0,
            decode_ms: 600_000,
            ..TranscriptionStats::default()
        };

        assert!((stats.pipeline_rtf(300.0) - 2.0).abs() < f64::EPSILON);
        // Which is what turns an hour-long meeting into an hour of waiting.
        assert!((projected_drain(2.0, 60.0) - 60.0).abs() < f64::EPSILON);
    }

    /// Crossing 1.0 is the goal, and just crossing it is nearly enough: the
    /// user's stated bar of "an hour of meeting, a minute of waiting" needs
    /// only a hair under 1.02, not some large multiple of real time.
    #[test]
    fn an_hour_of_meeting_drains_in_about_a_minute_just_above_real_time() {
        let drain = projected_drain(1.017, 60.0);
        assert!(
            (drain - 1.0).abs() < 0.05,
            "expected about a minute, got {drain}"
        );
    }

    /// Below real time the projection is zero rather than negative: there is
    /// no such thing as finishing before the meeting does.
    #[test]
    fn a_decoder_faster_than_real_time_projects_no_wait_at_all() {
        assert_eq!(projected_drain(0.4, 60.0), 0.0);
        assert_eq!(projected_drain(1.0, 60.0), 0.0);
    }

    /// Divide-by-zero guards: a meeting that recorded nothing must report a
    /// number, not a NaN that then prints as `NaN` in the summary.
    #[test]
    fn an_empty_meeting_reports_zero_rather_than_not_a_number() {
        let empty = TranscriptionStats::default();
        assert_eq!(empty.decode_rtf(), 0.0);
        assert_eq!(empty.pipeline_rtf(0.0), 0.0);
    }

    fn segment(sequence: u64, channel: SegmentChannel, start: f64, text: &str) -> TranscriptSegment {
        TranscriptSegment {
            sequence,
            text: text.to_string(),
            start_seconds: start,
            end_seconds: start + 2.0,
            channel,
            no_speech_prob: 0.02,
            recorded_at: "2026-01-01T00:00:00Z".into(),
            cut_at_ceiling: false,
            original_text: None,
            romanized_text: None,
            translated_text: None,
            corrections: Vec::new(),
            telemetry: None,
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
            decoding_cheap: WhisperDecodingConfig::default().for_expensive_script(),
                glossary: Vec::new(),
                vocabulary: crate::capture::vocabulary::DomainVocabulary::new(),
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
