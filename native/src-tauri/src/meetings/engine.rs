//! The recording session: start, pause, resume, stop, recover.
//!
//! One meeting at a time, held in [`MeetingEngine`]. Starting one opens two
//! audio streams, a segmenter, a checkpoint writer and a decoder, and wires
//! them into a chain that runs until the user stops it:
//!
//! ```text
//! DualCapture ──MixedAudio──▶ pump thread ──┬─▶ Segmenter ──▶ TranscriptionQueue ──▶ worker
//!                                           └─▶ CheckpointWriter ──▶ audio/chunk_*.wav
//! ```
//!
//! ## Stopping is the interesting part
//!
//! A stop has to end four things in the right order or it loses audio, loses
//! transcript, or hangs:
//!
//! 1. Stop capture and join the mixer thread. That closes the audio channel.
//! 2. The pump thread sees the channel close, flushes the segmenter — which
//!    is what submits the sentence someone was still speaking — writes the
//!    final checkpoint, and merges the audio.
//! 3. Drop the queue so the decoder's channel closes, then join the decoder.
//!    It finishes its backlog first; this is the step that can take minutes
//!    after a long meeting on a slow model, and it is bounded by
//!    [`DRAIN_TIMEOUT`] rather than by hope.
//! 4. Write the finished meeting record.
//!
//! Meetily splits this across the IPC boundary — its Rust side drains and
//! merges, then deliberately does not persist, leaving the renderer to write
//! the meeting to SQLite seconds later (`recording_commands.rs:866`). A
//! webview reload in that window loses the meeting. Here every step is on
//! this side of the boundary and the frontend only reads.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use tauri::{AppHandle, Emitter};

use crate::capture::stt::{
    resolve_meeting_model_path, SttEngine, SttLanguageConfig, SttPreset, SttWindow,
    WhisperDecodingConfig,
};
use crate::settings::AppSettings;
use crate::sync::MutexExt;

use super::capture::{DualCapture, MeetingCaptureError, MeetingDevices, OpenedDevices};
use super::checkpoint::{self, CheckpointWriter};
use super::model::{Meeting, MeetingSource, MeetingState};
use super::segmenter::Segmenter;
use super::store::{MeetingStore, MeetingStoreError};
use super::transcription::{self, TranscriptionQueue, WorkerConfig};

/// Recording lifecycle, for every surface that shows recording state.
pub const MEETING_STATE_EVENT: &str = "meeting-state-changed";

/// Everything [`MeetingEngine::start`] needs to open a recording.
///
/// A struct rather than seven positional parameters. Two of them are
/// `Option<String>` and two are booleans, which is the shape where a
/// transposed pair compiles cleanly and records the wrong thing.
pub struct StartRequest<'a> {
    /// Present outside tests; `None` means nothing is emitted.
    pub app: Option<AppHandle>,
    /// `None` takes the generated "Meeting <timestamp>" title.
    pub title: Option<String>,
    pub settings: &'a AppSettings,
    pub config_dir: &'a std::path::Path,
    pub stt: SttEngine,
    pub capture_system_audio: bool,
    pub devices: MeetingDevices,
}

/// How long a stop waits for the decoder to finish its backlog.
///
/// Generous, because the alternative to waiting is discarding transcript the
/// user watched being queued. Bounded, because a wedged decoder must not make
/// the app unquittable.
const DRAIN_TIMEOUT: Duration = Duration::from_secs(600);

/// How often the stop path re-checks whether the decoder has drained.
const DRAIN_POLL: Duration = Duration::from_millis(200);

#[derive(Debug, thiserror::Error)]
pub enum MeetingEngineError {
    #[error("a meeting is already being recorded")]
    AlreadyRecording,

    #[error("no meeting is being recorded")]
    NotRecording,

    #[error(
        "no speech model is installed, so a meeting would record audio with no transcript. \
         Install one under Settings › Speech."
    )]
    NoSpeechModel,

    #[error(transparent)]
    Capture(#[from] MeetingCaptureError),

    #[error(transparent)]
    Store(#[from] MeetingStoreError),
}

/// What the UI needs to render the recording surface.
#[derive(Debug, Clone, serde::Serialize)]
pub struct MeetingRecordingStatus {
    pub active: bool,
    pub meeting_id: Option<String>,
    pub title: Option<String>,
    pub state: Option<MeetingState>,
    pub elapsed_seconds: f64,
    pub microphone_active: bool,
    pub system_audio_active: bool,
    /// Whether anything above the silence floor has been heard on each
    /// channel. `microphone_heard == false` after a minute is the signal that
    /// the wrong device is open — the one users otherwise discover at the end.
    pub microphone_heard: bool,
    pub system_audio_heard: bool,
    pub segments_queued: u64,
    pub segments_completed: u64,
    pub segments_dropped: u64,
    /// Set when system audio could not be opened, so the surface can say the
    /// far end of the call is not being recorded.
    pub warning: Option<String>,
    /// The devices this recording actually opened.
    ///
    /// The heard flags above are how a wrong device is noticed; these are what
    /// turn "the microphone bar never moved" into something the user can act
    /// on without leaving the app to guess which device that was.
    pub devices: OpenedDevices,
}

impl MeetingRecordingStatus {
    fn idle() -> Self {
        Self {
            active: false,
            meeting_id: None,
            title: None,
            state: None,
            elapsed_seconds: 0.0,
            microphone_active: false,
            system_audio_active: false,
            microphone_heard: false,
            system_audio_heard: false,
            segments_queued: 0,
            segments_completed: 0,
            segments_dropped: 0,
            warning: None,
            devices: OpenedDevices::default(),
        }
    }
}

/// What the pump thread reports back when the recording ends.
struct PumpResult {
    /// The merged recording, if any audio was captured.
    audio_path: Option<String>,
    duration_seconds: f64,
}

struct ActiveMeeting {
    id: String,
    title: String,
    capture: DualCapture,
    queue: TranscriptionQueue,
    worker: std::thread::JoinHandle<transcription::TranscriptionStats>,
    pump: std::thread::JoinHandle<PumpResult>,
    cancel: Arc<AtomicBool>,
    warning: Option<String>,
    /// Only for the status line. Every transcript timestamp comes from the
    /// segmenter's own frame clock, which does not advance while paused.
    started: Instant,
}

/// Owns the one recording that can be in flight.
pub struct MeetingEngine {
    store: Arc<MeetingStore>,
    active: Mutex<Option<ActiveMeeting>>,
}

impl MeetingEngine {
    pub fn new(store: Arc<MeetingStore>) -> Self {
        Self {
            store,
            active: Mutex::new(None),
        }
    }

    pub fn store(&self) -> &Arc<MeetingStore> {
        &self.store
    }

    pub fn is_recording(&self) -> bool {
        self.active.lock_or_recover().is_some()
    }

    /// The current recording's state, or an idle status.
    pub fn status(&self) -> MeetingRecordingStatus {
        let guard = self.active.lock_or_recover();
        let Some(active) = guard.as_ref() else {
            return MeetingRecordingStatus::idle();
        };
        let (queued, completed, dropped) = active.queue.counts();
        MeetingRecordingStatus {
            active: true,
            meeting_id: Some(active.id.clone()),
            title: Some(active.title.clone()),
            state: Some(if active.capture.is_paused() {
                MeetingState::Paused
            } else {
                MeetingState::Recording
            }),
            elapsed_seconds: active.started.elapsed().as_secs_f64(),
            microphone_active: active.capture.is_microphone_active(),
            system_audio_active: active.capture.is_system_audio_active(),
            microphone_heard: active.capture.microphone_heard(),
            system_audio_heard: active.capture.system_audio_heard(),
            segments_queued: queued,
            segments_completed: completed,
            segments_dropped: dropped,
            warning: active.warning.clone(),
            devices: active.capture.binding().opened,
        }
    }

    /// Begins recording.
    ///
    /// Refuses when no speech model is installed. A meeting that records
    /// audio and produces no transcript is a failure the user only discovers
    /// at the end, by which point the meeting is over.
    pub fn start(&self, request: StartRequest<'_>) -> Result<Meeting, MeetingEngineError> {
        let StartRequest {
            app,
            title,
            settings,
            config_dir,
            stt,
            capture_system_audio,
            devices,
        } = request;
        let mut guard = self.active.lock_or_recover();
        if guard.is_some() {
            return Err(MeetingEngineError::AlreadyRecording);
        }

        let models_dir = config_dir.join("models");
        let model_path =
            resolve_meeting_model_path(&models_dir, &settings.stt).ok_or(MeetingEngineError::NoSpeechModel)?;

        let id = format!("meeting-{}", uuid::Uuid::new_v4().simple());
        let title = title
            .map(|t| t.trim().to_string())
            .filter(|t| !t.is_empty())
            .unwrap_or_else(default_meeting_title);

        let mut meeting = Meeting::new(id.clone(), title.clone(), MeetingSource::Recorded);
        meeting.transcript_model = Some(model_path.to_string_lossy().to_string());
        meeting.language = Some(settings.language.primary_dictation_language.clone());
        self.store.create(&meeting)?;

        let (capture, audio_rx) =
            match DualCapture::start(app.clone(), capture_system_audio, devices) {
            Ok(started) => started,
            Err(err) => {
                // The meeting directory exists but nothing was recorded into
                // it; leaving it behind would put an empty meeting in the list.
                let _ = self.store.delete_meeting(&id);
                return Err(err.into());
            }
        };
        let binding = capture.binding();
        let warning = if capture_system_audio && !binding.system_audio {
            Some(
                "System audio could not be captured, so only this machine's microphone is being \
                 recorded. The other participants will not appear in the transcript."
                    .to_string(),
            )
        } else {
            None
        };
        if let Some(message) = &warning {
            tracing::warn!("meeting {}: {}", id, message);
        }

        let cancel = Arc::new(AtomicBool::new(false));
        let (queue, worker) = transcription::spawn_worker(
            WorkerConfig {
                meeting_id: id.clone(),
                model_path: model_path.to_string_lossy().to_string(),
                // Long-form: a meeting segment carries enough audio for
                // Whisper's own language detection to be trustworthy, so a
                // multilingual profile is honoured rather than pinned.
                language: SttLanguageConfig::from_settings(&settings.language, SttWindow::LongForm),
                // Balanced rather than Fast: a meeting is decoded once and
                // read later, so it can afford the wider beam that dictation
                // cannot. An explicit user preset still wins.
                decoding: WhisperDecodingConfig::for_meetings(&settings.stt, SttPreset::Balanced),
                decoding_expensive_script: WhisperDecodingConfig::for_meetings(
                    &settings.stt,
                    SttPreset::Balanced,
                )
                .for_expensive_script(),
                glossary: settings.dictionary.clone(),
            },
            stt,
            Arc::clone(&self.store),
            app.clone(),
            Arc::clone(&cancel),
        );

        let audio_dir = self.store.audio_dir(&id)?;
        let pump_queue = queue.clone();
        let pump = std::thread::Builder::new()
            .name("vox-meeting-pump".into())
            .spawn(move || run_pump(audio_rx, audio_dir, pump_queue))
            .map_err(|err| MeetingCaptureError::StartFailed(err.to_string()))?;

        let meeting = self.store.update_meeting(&id, |record| {
            record.mic_device = None;
            record.system_audio_captured = binding.system_audio;
        })?;

        *guard = Some(ActiveMeeting {
            id: id.clone(),
            title,
            capture,
            queue,
            worker,
            pump,
            cancel,
            warning,
            started: Instant::now(),
        });
        drop(guard);

        tracing::info!(
            "meeting {} recording (microphone: {}, system audio: {})",
            id,
            binding.microphone,
            binding.system_audio
        );
        self.emit_state(&app);
        Ok(meeting)
    }

    /// Stops discarding audio until [`MeetingEngine::resume`].
    pub fn pause(&self, app: Option<AppHandle>) -> Result<(), MeetingEngineError> {
        {
            let guard = self.active.lock_or_recover();
            let active = guard.as_ref().ok_or(MeetingEngineError::NotRecording)?;
            active.capture.pause();
            let _ = self.store.update_meeting(&active.id, |record| {
                record.state = MeetingState::Paused;
            });
        }
        self.emit_state(&app);
        Ok(())
    }

    pub fn resume(&self, app: Option<AppHandle>) -> Result<(), MeetingEngineError> {
        {
            let guard = self.active.lock_or_recover();
            let active = guard.as_ref().ok_or(MeetingEngineError::NotRecording)?;
            active.capture.resume();
            let _ = self.store.update_meeting(&active.id, |record| {
                record.state = MeetingState::Recording;
            });
        }
        self.emit_state(&app);
        Ok(())
    }

    /// Ends the recording and waits for the transcript to finish.
    ///
    /// Blocking, and deliberately so: the returned [`Meeting`] is complete,
    /// which is what lets the frontend navigate straight to it.
    pub fn stop(&self, app: Option<AppHandle>) -> Result<Meeting, MeetingEngineError> {
        let mut active = self
            .active
            .lock_or_recover()
            .take()
            .ok_or(MeetingEngineError::NotRecording)?;

        let id = active.id.clone();
        let _ = self.store.update_meeting(&id, |record| {
            record.state = MeetingState::Transcribing;
        });
        self.emit_state(&app);

        // 1. Stop capture. Joins the mixer thread, which closes the audio
        //    channel the pump is reading.
        active.capture.stop();
        let binding = active.capture.binding();
        let heard_system = active.capture.system_audio_heard();

        // 2. The pump flushes the segmenter, submitting whatever sentence was
        //    still open, then merges the audio.
        let pump = match active.pump.join() {
            Ok(result) => result,
            Err(_) => {
                tracing::error!("meeting {}: the audio pump thread panicked", id);
                PumpResult {
                    audio_path: None,
                    duration_seconds: 0.0,
                }
            }
        };

        // 3. Let the decoder finish its backlog, then close its channel and
        //    join it.
        //
        // Timed from here rather than from the start of `stop`: capture and
        // pump shutdown above are bounded and quick, while this is the
        // open-ended part the user sits through after pressing stop.
        let t_drain_start = Instant::now();
        let drained = wait_for_drain(&active.queue, DRAIN_TIMEOUT);
        if !drained {
            let (queued, completed, _) = active.queue.counts();
            tracing::warn!(
                "meeting {}: giving up on {} undecoded segment(s) after {}s",
                id,
                queued.saturating_sub(completed),
                DRAIN_TIMEOUT.as_secs()
            );
            active.cancel.store(true, Ordering::SeqCst);
        }
        let (queued, completed, dropped) = active.queue.counts();
        drop(active.queue);
        let stats = match active.worker.join() {
            Ok(stats) => Some(stats),
            Err(_) => {
                tracing::error!("meeting {}: the transcription worker panicked", id);
                None
            }
        };
        let drain_seconds = t_drain_start.elapsed().as_secs_f64();

        if let Some(stats) = stats {
            transcription::print_meeting_summary(
                &id,
                &stats,
                pump.duration_seconds as f64,
                drain_seconds,
            );
        }

        // 4. Write the finished record.
        let segments = self.store.load_transcript(&id)?;
        let lost = dropped + queued.saturating_sub(completed);
        let meeting = self.store.update_meeting(&id, |record| {
            record.state = MeetingState::Completed;
            record.duration_seconds = pump.duration_seconds;
            record.audio_path = pump.audio_path.clone();
            record.segment_count = segments.len();
            record.dropped_segments = lost as usize;
            record.system_audio_captured = binding.system_audio && heard_system;
        })?;

        tracing::info!(
            "meeting {} finished: {:.1}s, {} segment(s), {} lost",
            id,
            meeting.duration_seconds,
            meeting.segment_count,
            lost
        );
        self.emit_state(&app);
        Ok(meeting)
    }

    /// Finalizes meetings a crash left open. Call once at startup.
    pub fn recover_interrupted(&self) -> Vec<String> {
        let recovered = match self.store.recover_interrupted() {
            Ok(ids) => ids,
            Err(err) => {
                tracing::warn!("could not scan for interrupted meetings: {}", err);
                return Vec::new();
            }
        };
        // Recovery in the store marks the record; merging the orphaned
        // checkpoints into a playable recording happens here, because it is
        // the audio layer's job and it can fail independently.
        for id in &recovered {
            let Ok(audio_dir) = self.store.audio_dir(id) else {
                continue;
            };
            if audio_dir.join(checkpoint::MERGED_AUDIO_FILE).exists() {
                continue;
            }
            match checkpoint::merge_chunks(&audio_dir, true) {
                Ok(path) => {
                    let _ = self.store.update_meeting(id, |record| {
                        record.audio_path = Some(path.to_string_lossy().to_string());
                    });
                }
                Err(err) => {
                    tracing::warn!("meeting {}: could not recover audio: {}", id, err);
                }
            }
        }
        recovered
    }

    fn emit_state(&self, app: &Option<AppHandle>) {
        if let Some(handle) = app {
            let _ = handle.emit(MEETING_STATE_EVENT, self.status());
        }
    }
}

/// Reads mixed audio until capture ends, feeding the segmenter and the
/// checkpoint writer.
///
/// Runs on its own thread because both consumers do work that must not sit on
/// the mixer: segmentation allocates, and checkpointing writes to disk.
fn run_pump(
    audio_rx: std::sync::mpsc::Receiver<super::capture::MixedAudio>,
    audio_dir: std::path::PathBuf,
    queue: TranscriptionQueue,
) -> PumpResult {
    let mut segmenter = Segmenter::new();
    let mut writer = match CheckpointWriter::new(&audio_dir) {
        Ok(writer) => Some(writer),
        Err(err) => {
            // No checkpoints means no recording and no crash recovery, but
            // the transcript can still be produced — which is most of the
            // value — so this degrades rather than aborts.
            tracing::error!("meeting audio will not be saved: {}", err);
            None
        }
    };

    for block in audio_rx {
        if block.discontinuity {
            // Resuming after a pause: whatever was mid-sentence when the user
            // paused does not continue into what they said after.
            for segment in segmenter.flush() {
                queue.submit(segment);
            }
        }
        for segment in segmenter.push(&block.mixed, &block.mic, &block.sys) {
            queue.submit(segment);
        }
        if let Some(writer) = writer.as_mut() {
            if let Err(err) = writer.push(&block.mixed) {
                tracing::error!("meeting checkpoint failed: {}", err);
            }
        }
    }

    // Capture has ended. The sentence someone was still speaking is here.
    for segment in segmenter.flush() {
        queue.submit(segment);
    }

    let duration_seconds = writer
        .as_ref()
        .map(|writer| writer.duration_seconds())
        .unwrap_or_else(|| segmenter.position_seconds());
    let audio_path = writer.and_then(|writer| match writer.finalize() {
        Ok(path) => Some(path.to_string_lossy().to_string()),
        Err(err) => {
            tracing::error!("could not merge the meeting recording: {}", err);
            None
        }
    });

    PumpResult {
        audio_path,
        duration_seconds,
    }
}

/// Waits until every submitted segment has been decoded, or the timeout.
fn wait_for_drain(queue: &TranscriptionQueue, timeout: Duration) -> bool {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if queue.is_drained() {
            return true;
        }
        std::thread::sleep(DRAIN_POLL);
    }
    queue.is_drained()
}

/// `Meeting 2026-09-11 14:31`.
///
/// Recognised by [`super::summary::service::is_generated_title`] as
/// replaceable, so a summary's own title supersedes it — and anything the
/// user types instead does not get overwritten.
pub fn default_meeting_title() -> String {
    format!("Meeting {}", chrono::Local::now().format("%Y-%m-%d %H:%M"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_generated_title_is_one_the_summary_may_replace() {
        let title = default_meeting_title();
        assert!(title.starts_with("Meeting "));
        assert!(
            super::super::summary::service::is_generated_title(&title),
            "{title} must be recognised as replaceable"
        );
    }

    #[test]
    fn an_idle_engine_reports_nothing_recording() {
        let store = Arc::new(MeetingStore::new(std::env::temp_dir().join("vox-engine-idle")));
        let engine = MeetingEngine::new(store);
        let status = engine.status();
        assert!(!status.active);
        assert!(status.meeting_id.is_none());
        assert_eq!(status.elapsed_seconds, 0.0);
        assert!(!engine.is_recording());
    }

    #[test]
    fn pausing_or_stopping_when_nothing_is_recording_is_an_error_not_a_panic() {
        let store = Arc::new(MeetingStore::new(std::env::temp_dir().join("vox-engine-none")));
        let engine = MeetingEngine::new(store);
        assert!(matches!(
            engine.pause(None),
            Err(MeetingEngineError::NotRecording)
        ));
        assert!(matches!(
            engine.resume(None),
            Err(MeetingEngineError::NotRecording)
        ));
        assert!(matches!(
            engine.stop(None),
            Err(MeetingEngineError::NotRecording)
        ));
    }

    #[test]
    fn starting_without_a_speech_model_is_refused_before_anything_is_recorded() {
        // Recording audio that can never become a transcript is a failure the
        // user would only find at the end of the meeting.
        let dir = std::env::temp_dir().join(format!(
            "vox-engine-nomodel-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let store = Arc::new(MeetingStore::new(dir.join("vault")));
        let engine = MeetingEngine::new(Arc::clone(&store));

        let result = engine.start(StartRequest {
            app: None,
            title: None,
            settings: &AppSettings::default(),
            config_dir: &dir,
            stt: SttEngine::new(),
            capture_system_audio: true,
            devices: MeetingDevices::default(),
        });

        assert!(matches!(result, Err(MeetingEngineError::NoSpeechModel)));
        assert!(!engine.is_recording());
        assert!(
            store.list_meetings().unwrap().is_empty(),
            "a refused start must not leave a meeting behind"
        );
    }

    #[test]
    fn recovery_on_an_empty_vault_finds_nothing_and_does_not_fail() {
        let dir = std::env::temp_dir().join(format!(
            "vox-engine-recover-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        let engine = MeetingEngine::new(Arc::new(MeetingStore::new(&dir)));
        assert!(engine.recover_interrupted().is_empty());
    }

    #[test]
    fn recovery_merges_the_checkpoints_a_crash_left_behind() {
        let dir = std::env::temp_dir().join(format!(
            "vox-engine-recover-audio-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        let store = Arc::new(MeetingStore::new(&dir));
        let meeting = Meeting::new("meeting-crash".into(), "Crashed".into(), MeetingSource::Recorded);
        store.create(&meeting).unwrap();

        // Two checkpoints reached disk before the process died.
        let audio_dir = store.audio_dir("meeting-crash").unwrap();
        let mut writer = CheckpointWriter::new(&audio_dir).unwrap();
        writer
            .push(&vec![0.1f32; super::super::segmenter::SEGMENT_SAMPLE_RATE as usize * 65])
            .unwrap();
        drop(writer);

        let engine = MeetingEngine::new(Arc::clone(&store));
        let recovered = engine.recover_interrupted();

        assert_eq!(recovered, vec!["meeting-crash".to_string()]);
        let record = store.load_meeting("meeting-crash").unwrap();
        assert_eq!(record.state, MeetingState::Completed);
        let audio = record.audio_path.expect("recovered audio path");
        assert!(std::path::Path::new(&audio).exists());
        assert!((record.duration_seconds - 60.0).abs() < 0.1);
    }

    #[test]
    fn waiting_on_an_already_drained_queue_returns_at_once() {
        let (queue, worker) = transcription::spawn_worker(
            WorkerConfig {
                meeting_id: "meeting-drain".into(),
                model_path: String::new(),
                language: SttLanguageConfig {
                    whisper_language: None,
                    translate: false,
                },
                decoding: WhisperDecodingConfig::default(),
            decoding_expensive_script: WhisperDecodingConfig::default().for_expensive_script(),
                glossary: Vec::new(),
            },
            SttEngine::new(),
            Arc::new(MeetingStore::new(std::env::temp_dir().join("vox-engine-drain"))),
            None,
            Arc::new(AtomicBool::new(false)),
        );
        assert!(wait_for_drain(&queue, Duration::from_millis(500)));
        drop(queue);
        let _ = worker.join();
    }
}
