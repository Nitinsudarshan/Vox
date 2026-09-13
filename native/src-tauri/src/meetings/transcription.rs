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
#[derive(Debug, Default, Clone, Copy)]
struct SegmentTiming {
    audio_seconds: f64,
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

    for job in rx {
        if cancel.load(Ordering::SeqCst) {
            tracing::info!("meeting {}: transcription cancelled", config.meeting_id);
            break;
        }

        let sequence = job.sequence;
        let (outcome, mut timing) = decode_segment(&engine, &config, &job);
        counters.completed.fetch_add(1, Ordering::SeqCst);

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
                format!("kept ({} chars)", segment.text.chars().count())
            }
            DecodeOutcome::Discarded => {
                stats.discarded += 1;
                "discarded (empty or screened as hallucination)".to_string()
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
                format!("FAILED — {message}")
            }
        };

        let in_queue = {
            let (queued, completed, _) = counts_of(&counters);
            queued.saturating_sub(completed)
        };
        accumulate(&mut stats, &timing);
        if timing.decode_rtf() > stats.slowest_rtf {
            stats.slowest_rtf = timing.decode_rtf();
            stats.slowest_sequence = sequence;
        }
        print_segment_trace(&config.meeting_id, sequence, &timing, &outcome_label, in_queue);

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

fn accumulate(stats: &mut TranscriptionStats, timing: &SegmentTiming) {
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
    Kept(TranscriptSegment),
    /// Decoded, but screened out as a hallucination or as empty.
    Discarded,
    Failed(String),
}

fn decode_segment(
    engine: &SttEngine,
    config: &WorkerConfig,
    job: &DecodeJob,
) -> (DecodeOutcome, SegmentTiming) {
    let samples = &job.segment.samples;

    let mut timing = SegmentTiming {
        audio_seconds: job.segment.duration_seconds(),
        queue_wait_ms: job.submitted_at.elapsed().as_millis(),
        ..SegmentTiming::default()
    };

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
        Ok((utterances, diagnostics)) => {
            // The engine reports the decode alone; the waiting it did before
            // that is reported separately so the two never get confused.
            timing.decode_ms = diagnostics.transcription_latency_ms;
            timing.lock_wait_ms = diagnostics.lock_wait_ms;
            timing.model_load_ms = diagnostics.model_load_ms;
            timing.model_reloaded = diagnostics.model_reloaded;
            timing.audio_ctx = diagnostics.audio_ctx;
            utterances
        }
        Err(err) => return (DecodeOutcome::Failed(err.to_string()), timing),
    };

    let t_post = Instant::now();

    let text = join_utterance_text(&utterances);
    if text.trim().is_empty() {
        timing.post_ms = t_post.elapsed().as_millis();
        return (DecodeOutcome::Discarded, timing);
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
        timing.post_ms = t_post.elapsed().as_millis();
        return (DecodeOutcome::Discarded, timing);
    }

    let normalized = crate::capture::text_normalize::normalize_segment_text(&text, &config.glossary);
    timing.post_ms = t_post.elapsed().as_millis();

    (
        DecodeOutcome::Kept(TranscriptSegment {
            sequence: job.sequence,
            text: normalized.text,
            start_seconds: job.segment.start_seconds,
            end_seconds: job.segment.end_seconds,
            channel: job.segment.channel,
            no_speech_prob: mean_no_speech_prob,
            recorded_at: chrono::Utc::now().to_rfc3339(),
        }),
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
