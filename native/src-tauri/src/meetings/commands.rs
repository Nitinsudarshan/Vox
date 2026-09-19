//! The Meetings IPC surface.
//!
//! Thin by policy (`rules/rust-backend.md`): each command validates its input,
//! calls one domain function, and maps the error. Everything that decides
//! anything lives in the modules beside this one.
//!
//! Commands that block — starting and stopping a recording, importing a file —
//! run on a blocking thread rather than on the async runtime, because they
//! wait on audio threads and on Whisper.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use tauri::{AppHandle, Emitter, Manager, State};

use crate::commands::{AppState, CommandError};
use crate::sync::MutexExt;

use super::benchmark;
use super::engine::{MeetingEngineError, MeetingRecordingStatus};
use super::import::{self, BatchConfig, ImportError};
use super::model::{Meeting, MeetingListItem, MeetingSummary, TranscriptSegment};
use super::series::{self, MeetingSeries, MeetingSeriesSummary, SeriesOccurrence, SeriesSource};
use super::store::{MeetingSearchHit, MeetingStoreError};
use super::summary::service::{SummaryError, SummaryOptions};
use super::summary::templates::{Template, TemplateLibrary, DEFAULT_TEMPLATE_ID};

/// Everything the meeting detail surface needs, in one round trip.
#[derive(Debug, Clone, serde::Serialize)]
pub struct MeetingDetail {
    pub meeting: Meeting,
    pub segments: Vec<TranscriptSegment>,
    pub summary: Option<MeetingSummary>,
    pub notes: String,
    /// Whoever detection has found so far. Empty until it runs, which is not
    /// an error — a transcript labelled "You" and "Others" is still a
    /// transcript.
    #[serde(default)]
    pub speakers: Vec<crate::meetings::model::Speaker>,
    /// How the recording and the transcription went. `None` for a meeting
    /// recorded before diagnostics existed, or one that never finished — both
    /// of which are ordinary, so the surface renders nothing rather than
    /// claiming zeroes.
    #[serde(default)]
    pub diagnostics: Option<crate::meetings::telemetry::MeetingDiagnostics>,
    /// The transcript as everything downstream reads it: ordered, sentences
    /// the decoder's window cut back together, speakers attached, gaps
    /// stated. `segments` above is the raw evidence underneath it.
    pub canonical: crate::meetings::canonical::CanonicalTranscript,
}

/// In-flight import and re-transcription runs, so they can be cancelled.
#[derive(Default)]
pub struct ImportRegistry {
    running: Mutex<HashMap<String, Arc<AtomicBool>>>,
}

impl ImportRegistry {
    fn register(&self, key: &str) -> Arc<AtomicBool> {
        let flag = Arc::new(AtomicBool::new(false));
        self.running.lock_or_recover().insert(key.to_string(), flag.clone());
        flag
    }

    fn finish(&self, key: &str) {
        self.running.lock_or_recover().remove(key);
    }

    /// Signals the run for `key` to stop. Returns whether there was one.
    pub fn cancel(&self, key: &str) -> bool {
        match self.running.lock_or_recover().get(key) {
            Some(flag) => {
                flag.store(true, Ordering::SeqCst);
                true
            }
            None => false,
        }
    }

    /// Cancels every in-flight run. Used on shutdown.
    pub fn cancel_all(&self) {
        for flag in self.running.lock_or_recover().values() {
            flag.store(true, Ordering::SeqCst);
        }
    }
}

// --- error mapping ------------------------------------------------------
//
// Internal error types never cross this boundary verbatim
// (`rules/api-conventions.md`). Each maps to a code the frontend can branch on
// and a message a user can act on.

impl From<MeetingEngineError> for CommandError {
    fn from(err: MeetingEngineError) -> Self {
        let code = match err {
            MeetingEngineError::AlreadyRecording => "MEETING_ALREADY_RECORDING",
            MeetingEngineError::NotRecording => "MEETING_NOT_RECORDING",
            MeetingEngineError::NoSpeechModel => "MEETING_NO_SPEECH_MODEL",
            MeetingEngineError::Capture(_) => "MEETING_CAPTURE_FAILED",
            MeetingEngineError::Store(_) => "MEETING_STORAGE_FAILED",
        };
        CommandError::new(code, &err.to_string())
    }
}

impl From<MeetingStoreError> for CommandError {
    fn from(err: MeetingStoreError) -> Self {
        let code = match err {
            MeetingStoreError::NotFound(_) => "MEETING_NOT_FOUND",
            MeetingStoreError::InvalidId(_) => "MEETING_INVALID_ID",
            _ => "MEETING_STORAGE_FAILED",
        };
        CommandError::new(code, &err.to_string())
    }
}

/// Benchmark failures map by cause, because the three the user can act on are
/// different actions: fix the manifest, supply the audio, install the model.
impl From<benchmark::BenchmarkError> for CommandError {
    fn from(err: benchmark::BenchmarkError) -> Self {
        let code = match err {
            benchmark::BenchmarkError::Io(_) => "BENCHMARK_IO_FAILED",
            benchmark::BenchmarkError::Json(_) | benchmark::BenchmarkError::Version { .. } => {
                "BENCHMARK_BAD_MANIFEST"
            }
            benchmark::BenchmarkError::Invalid(_) => "BENCHMARK_INVALID",
            benchmark::BenchmarkError::UnknownCase(_) => "BENCHMARK_UNKNOWN_CASE",
            benchmark::BenchmarkError::Audio(_) => "BENCHMARK_AUDIO_FAILED",
        };
        CommandError::new(code, &err.to_string())
    }
}

impl From<SummaryError> for CommandError {
    fn from(err: SummaryError) -> Self {
        let code = match err {
            SummaryError::EmptyTranscript => "MEETING_EMPTY_TRANSCRIPT",
            SummaryError::AlreadyRunning => "MEETING_SUMMARY_RUNNING",
            SummaryError::Cancelled => "MEETING_SUMMARY_CANCELLED",
            SummaryError::Provider(_) => "MEETING_SUMMARY_PROVIDER_FAILED",
            // Its own code: the frontend offers a way into Settings for this
            // one, because the fix is a setting rather than a retry.
            SummaryError::ProviderUnavailable(_) => "MEETING_SUMMARY_PROVIDER_UNAVAILABLE",
            SummaryError::Store(_) => "MEETING_STORAGE_FAILED",
        };
        CommandError::new(code, &err.to_string())
    }
}

impl From<ImportError> for CommandError {
    fn from(err: ImportError) -> Self {
        let code = match err {
            ImportError::UnsupportedFormat(_) => "MEETING_UNSUPPORTED_FORMAT",
            ImportError::NoAudio | ImportError::Decode(_) => "MEETING_DECODE_FAILED",
            ImportError::NoSpeechModel => "MEETING_NO_SPEECH_MODEL",
            ImportError::NoRecording => "MEETING_NO_RECORDING",
            ImportError::Cancelled => "MEETING_IMPORT_CANCELLED",
            ImportError::Io(_) => "MEETING_FILE_UNREADABLE",
            ImportError::Store(_) => "MEETING_STORAGE_FAILED",
        };
        CommandError::new(code, &err.to_string())
    }
}

// --- recording ----------------------------------------------------------

#[tauri::command]
pub async fn start_meeting(
    app: AppHandle,
    title: Option<String>,
    capture_system_audio: Option<bool>,
    devices: Option<crate::meetings::capture::MeetingDevices>,
) -> Result<Meeting, CommandError> {
    let handle = app.clone();
    let meeting = tauri::async_runtime::spawn_blocking(move || {
        let state = handle.state::<AppState>();
        let settings = state.settings.lock_or_recover().clone();
        state
            .meeting_engine
            .start(crate::meetings::engine::StartRequest {
                app: Some(handle.clone()),
                title,
                settings: &settings,
                config_dir: &state.config_dir,
                stt: state.stt.clone(),
                capture_system_audio: capture_system_audio
                    .unwrap_or(settings.meetings.capture_system_audio),
                // A device named for this recording wins; otherwise the
                // standing choice from Meetings settings, which is what makes
                // the picker stick between recordings.
                devices: devices.unwrap_or_else(|| settings.meetings.devices.clone()),
            })
            .map_err(CommandError::from)
    })
    .await
    .map_err(|err| CommandError::new("MEETING_TASK_FAILED", &err.to_string()))??;

    // Bring the pill up only once recording is actually under way.
    crate::overlay::ensure_meeting_overlay(&app, true);

    // A reminder card asking "shall I record this?" is answered by a recording
    // starting, wherever it was started from. Clearing it here rather than in
    // the reminder path alone is what stops the list and the card from holding
    // different views of whether a meeting has been dealt with.
    if let Some(notifications) =
        app.try_state::<std::sync::Arc<crate::calendar::reminders::NotificationService>>()
    {
        notifications.dismiss(&app);
    }

    Ok(meeting)
}

/// The saved microphone and output choice for recordings.
#[tauri::command]
pub fn get_meeting_devices(
    state: State<'_, AppState>,
) -> Result<crate::meetings::capture::MeetingDevices, CommandError> {
    Ok(state.settings.lock_or_recover().meetings.devices.clone())
}

/// Remembers which devices recordings should open.
///
/// Its own command rather than a whole-`AppSettings` save from the frontend:
/// that round trip lets a stale copy of the document overwrite whatever else
/// changed meanwhile, and this is a two-field change made from a picker that
/// has no business carrying the rest of the settings.
#[tauri::command]
pub fn set_meeting_devices(
    app: AppHandle,
    state: State<'_, AppState>,
    devices: crate::meetings::capture::MeetingDevices,
) -> Result<(), CommandError> {
    let settings = {
        let mut guard = state.settings.lock_or_recover();
        guard.meetings.devices = devices;
        guard.clone()
    };
    crate::commands::persist_settings(&app, &state, settings)
}

#[tauri::command]
pub fn pause_meeting(app: AppHandle, state: State<'_, AppState>) -> Result<(), CommandError> {
    state.meeting_engine.pause(Some(app)).map_err(CommandError::from)
}

#[tauri::command]
pub fn resume_meeting(app: AppHandle, state: State<'_, AppState>) -> Result<(), CommandError> {
    state.meeting_engine.resume(Some(app)).map_err(CommandError::from)
}

/// Ends the recording. Returns only once the transcript is complete, so the
/// caller can navigate straight to a finished meeting.
///
/// Starts the report too, when the user has asked for that. It is started
/// here rather than from the frontend so that closing the window on the way
/// out of a meeting does not silently skip it.
#[tauri::command]
pub async fn stop_meeting(app: AppHandle) -> Result<Meeting, CommandError> {
    let handle = app.clone();
    let meeting = tauri::async_runtime::spawn_blocking({
        let handle = handle.clone();
        move || {
            let state = handle.state::<AppState>();
            state
                .meeting_engine
                .stop(Some(handle.clone()))
                .map_err(CommandError::from)
        }
    })
    .await
    .map_err(|err| CommandError::new("MEETING_TASK_FAILED", &err.to_string()))??;

    // The pill comes down once the recording is genuinely finished, rather than
    // vanishing while the transcript is still draining.
    crate::overlay::hide_meeting_overlay(&app);

    let state = handle.state::<AppState>();
    let (provider, meetings) = {
        let settings = state.settings.lock_or_recover();
        (settings.provider.clone(), settings.meetings.clone())
    };
    if meetings.auto_summarize {
        let options = SummaryOptions {
            template_id: meetings.default_template_id.clone(),
            language: Some(meetings.summary_language.clone())
                .map(|code| code.trim().to_string())
                .filter(|code| !code.is_empty()),
            user_instructions: None,
            force: false,
        };
        // A meeting with nothing in it is refused by the service; that is not
        // a reason to fail the stop the user just asked for.
        if let Err(err) = state.summary_service.start(
            Some(handle.clone()),
            &meeting.id,
            options,
            provider,
            TemplateLibrary::new(&state.config_dir),
        ) {
            tracing::info!("meeting {}: no automatic report ({})", meeting.id, err);
        }
    }

    Ok(meeting)
}

#[tauri::command]
pub fn get_meeting_recording_status(state: State<'_, AppState>) -> MeetingRecordingStatus {
    state.meeting_engine.status()
}

// --- reading ------------------------------------------------------------

#[tauri::command]
pub fn list_meetings(state: State<'_, AppState>) -> Result<Vec<MeetingListItem>, CommandError> {
    state.meeting_store.list_for_display().map_err(CommandError::from)
}

#[tauri::command]
pub fn get_meeting(
    state: State<'_, AppState>,
    meeting_id: String,
) -> Result<MeetingDetail, CommandError> {
    let store = &state.meeting_store;
    let summary = state
        .summary_service
        .reconcile_orphaned(&meeting_id)
        .map_err(CommandError::from)?;
    let mut segments = store.load_transcript(&meeting_id)?;
    // Filled in on the way out rather than written back: a read command that
    // writes is a surprise, and the projection is cheap enough to redo. The
    // explicit `romanize_meeting_transcript` is what persists it.
    crate::meetings::variants::ensure_romanized(&mut segments);
    let speakers = store.load_speakers(&meeting_id)?;
    let attribution = store.load_attribution(&meeting_id)?;
    let meeting = store.load_meeting(&meeting_id)?;
    let canonical = crate::meetings::canonical::assemble(
        &meeting_id,
        &segments,
        &attribution,
        &speakers,
        meeting.transcript.clone(),
        &crate::meetings::canonical::AssemblyOptions::default(),
    );
    Ok(MeetingDetail {
        meeting,
        segments,
        summary,
        notes: store.load_notes(&meeting_id)?,
        speakers: speakers.clone(),
        diagnostics: store.load_diagnostics(&meeting_id)?,
        canonical,
    })
}

/// Fills in every transcript variant that needs no model.
///
/// Separate from [`translate_meeting_transcript`] because it is a different
/// kind of operation and the difference is the point: this is a deterministic
/// projection of the same words into the Latin alphabet, it runs offline in
/// microseconds, and it cannot fail on a machine with no provider configured.
/// Translation is none of those things. Fusing them is what produced a
/// Romanized view containing English.
#[tauri::command]
pub fn romanize_meeting_transcript(
    state: State<'_, AppState>,
    meeting_id: String,
) -> Result<Vec<TranscriptSegment>, CommandError> {
    let mut segments = state.meeting_store.load_transcript(&meeting_id)?;
    if crate::meetings::variants::ensure_romanized(&mut segments) > 0 {
        state.meeting_store.save_transcript(&meeting_id, &segments)?;
    }
    Ok(segments)
}

/// Translates a transcript, one batch of lines at a time.
///
/// Three things this does that its predecessor did not, each one a failure
/// that reached a user:
///
/// - **It batches.** One request carried the whole meeting against a 4096-token
///   output cap, so anything past a few minutes came back truncated.
/// - **It checks what it stores.** [`accept_translation`] refuses a reply that
///   is still in the source script, or that is the romanization handed back —
///   which is what a local model produced when asked for a translation and a
///   romanization in the same object.
/// - **It reports failure.** The parse was `if let Ok(parsed)`, so a model that
///   answered with prose left the transcript saved back unchanged and the UI
///   saying the translation had succeeded.
///
/// [`accept_translation`]: crate::meetings::variants::accept_translation
#[tauri::command]
pub async fn translate_meeting_transcript(
    app: AppHandle,
    meeting_id: String,
    target_language: Option<String>,
) -> Result<Vec<TranscriptSegment>, CommandError> {
    use crate::meetings::variants;

    let handle = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let state = handle.state::<AppState>();
        let mut segments = state.meeting_store.load_transcript(&meeting_id)?;
        if segments.is_empty() {
            return Err(CommandError::new(
                "MEETING_EMPTY_TRANSCRIPT",
                "This meeting has no transcript to translate.",
            ));
        }

        // The romanized view never depends on a model answering, so it is
        // filled in before one is asked. A translation that fails outright
        // still leaves the user better off than they started.
        variants::ensure_romanized(&mut segments);

        let lang_name = target_language
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .unwrap_or("English")
            .to_string();

        let provider = state.settings.lock_or_recover().provider.clone();
        let rt = tokio::runtime::Handle::current();
        if let Err(reason) = rt.block_on(crate::providers::check_ready(&provider)) {
            // Checked before any work is queued, so a machine with no provider
            // gets the sentence naming the fix rather than a transport error
            // per batch.
            state.meeting_store.save_transcript(&meeting_id, &segments)?;
            return Err(CommandError::new(
                "MEETING_TRANSLATION_UNAVAILABLE",
                &reason.to_string(),
            ));
        }
        let client = crate::providers::LLMClient::new(provider);

        // Asking for English fills the gaps a whisper translate pass left,
        // because that pass produces better English than a small local model
        // and re-translating over it is a downgrade. Asking for any other
        // language translates everything: the stored track is English, which
        // is not the language that was requested.
        let english_requested =
            lang_name.eq_ignore_ascii_case("english") || lang_name.eq_ignore_ascii_case("en");
        let missing = variants::lines_missing_english(&segments);
        let outcome = rt.block_on(variants::translate_missing(
            &client,
            &mut segments,
            &lang_name,
            english_requested.then_some(missing.as_slice()),
            |done, total| emit_translation_progress(&handle, &meeting_id, done, total),
        ));

        state.meeting_store.save_transcript(&meeting_id, &segments)?;

        if outcome.is_total_failure() {
            return Err(CommandError::new(
                "MEETING_TRANSLATION_FAILED",
                &format!(
                    "Nothing could be translated into {lang_name}: {}. The Original and \
                     Romanized views are unaffected.",
                    outcome.failure_detail()
                ),
            ));
        }
        if !outcome.failures.is_empty() {
            tracing::warn!(
                "meeting {}: {} of {} translation batches failed ({})",
                meeting_id,
                outcome.failures.len(),
                outcome.batches,
                outcome.failures.join(", ")
            );
        }
        Ok(segments)
    })
    .await
    .map_err(|err| CommandError::new("MEETING_TASK_FAILED", &err.to_string()))?
}

/// Progress for a translation run, so a long transcript is not a dead spinner.
fn emit_translation_progress(app: &AppHandle, meeting_id: &str, done: usize, total: usize) {
    let fraction = if total == 0 {
        1.0
    } else {
        (done as f32 / total as f32).clamp(0.0, 1.0)
    };
    let _ = app.emit(
        TRANSLATION_PROGRESS_EVENT,
        serde_json::json!({
            "meeting_id": meeting_id,
            "completed": done,
            "total": total,
            "fraction": fraction,
        }),
    );
}

/// How far a translation run has got.
pub const TRANSLATION_PROGRESS_EVENT: &str = "meeting-translation-progress";

#[tauri::command]
pub fn search_meetings(
    state: State<'_, AppState>,
    query: String,
    limit: Option<usize>,
) -> Result<Vec<MeetingSearchHit>, CommandError> {
    state
        .meeting_store
        .search(&query, limit.unwrap_or(50).clamp(1, 500))
        .map_err(CommandError::from)
}

#[tauri::command]
pub fn rename_meeting(
    state: State<'_, AppState>,
    meeting_id: String,
    title: String,
) -> Result<Meeting, CommandError> {
    let title = title.trim().to_string();
    if title.is_empty() {
        return Err(CommandError::new(
            "MEETING_INVALID_TITLE",
            "A meeting needs a title.",
        ));
    }
    if title.chars().count() > 200 {
        return Err(CommandError::new(
            "MEETING_INVALID_TITLE",
            "That title is too long.",
        ));
    }
    state
        .meeting_store
        .update_meeting(&meeting_id, |record| record.title = title)
        .map_err(CommandError::from)
}

#[tauri::command]
pub fn save_meeting_notes(
    state: State<'_, AppState>,
    meeting_id: String,
    notes: String,
) -> Result<(), CommandError> {
    state
        .meeting_store
        .save_notes(&meeting_id, &notes)
        .map_err(CommandError::from)
}

/// Deletes a meeting, its transcript, its summary and its audio.
#[tauri::command]
pub fn delete_meeting(state: State<'_, AppState>, meeting_id: String) -> Result<(), CommandError> {
    if state
        .meeting_engine
        .status()
        .meeting_id
        .as_deref()
        == Some(meeting_id.as_str())
    {
        return Err(CommandError::new(
            "MEETING_ALREADY_RECORDING",
            "That meeting is still being recorded. Stop it first.",
        ));
    }
    state.summary_service.cancel(&meeting_id);
    state.meeting_imports.cancel(&meeting_id);
    state
        .meeting_store
        .delete_meeting(&meeting_id)
        .map_err(CommandError::from)
}

// --- recurring meeting series -------------------------------------------

/// Longest name a series may carry. Same bound as a meeting's title.
const MAX_SERIES_TITLE: usize = 200;

/// Every recurring meeting, with how many recordings are in each.
///
/// A series with no recordings is still listed: one created by hand before its
/// first meeting is recorded is not an error, and hiding it would make it look
/// as though creating it had failed.
#[tauri::command]
pub fn list_meeting_series(
    state: State<'_, AppState>,
) -> Result<Vec<MeetingSeriesSummary>, CommandError> {
    let meetings = state.meeting_store.list_meetings()?;
    let mut summaries: Vec<MeetingSeriesSummary> = state
        .meeting_store
        .load_series()
        .into_iter()
        .map(|entry| {
            let members = series::occurrences(&meetings, &entry.id);
            MeetingSeriesSummary {
                occurrence_count: members.len(),
                latest_at: members.last().map(|meeting| meeting.created_at.clone()),
                series: entry,
            }
        })
        .collect();

    // Most recently active first: a series nobody has met about in six months
    // is not what the picker should open on.
    summaries.sort_by(|a, b| {
        b.latest_at
            .cmp(&a.latest_at)
            .then_with(|| a.series.title.to_lowercase().cmp(&b.series.title.to_lowercase()))
    });
    Ok(summaries)
}

/// The recordings in one series, oldest first.
#[tauri::command]
pub fn get_meeting_series(
    state: State<'_, AppState>,
    series_id: String,
) -> Result<Vec<SeriesOccurrence>, CommandError> {
    let meetings = state.meeting_store.list_meetings()?;
    Ok(series::occurrences(&meetings, series_id.trim())
        .into_iter()
        .map(|meeting| SeriesOccurrence {
            meeting_id: meeting.id.clone(),
            title: meeting.title.clone(),
            created_at: meeting.created_at.clone(),
            duration_seconds: meeting.duration_seconds,
            has_summary: state
                .meeting_store
                .load_summary(&meeting.id)
                .ok()
                .flatten()
                .and_then(|summary| summary.markdown)
                .is_some_and(|markdown| !markdown.trim().is_empty()),
        })
        .collect())
}

/// Creates a series by hand, for meetings no calendar event covers.
///
/// Imported audio and ad-hoc recordings have no event and never will, so the
/// only honest way to group them is the user saying so.
#[tauri::command]
pub fn create_meeting_series(
    state: State<'_, AppState>,
    title: String,
) -> Result<MeetingSeries, CommandError> {
    let title = validate_series_title(&title)?;
    let entry = MeetingSeries {
        id: series::manual_series_id(&title, chrono::Utc::now().timestamp_millis()),
        title,
        source: SeriesSource::Manual,
        created_at: chrono::Utc::now().to_rfc3339(),
        renamed_by_user: true,
    };
    state.meeting_store.upsert_series(entry.clone())?;
    Ok(entry)
}

/// Renames a series. The new name survives every later sync.
#[tauri::command]
pub fn rename_meeting_series(
    state: State<'_, AppState>,
    series_id: String,
    title: String,
) -> Result<Vec<MeetingSeries>, CommandError> {
    let title = validate_series_title(&title)?;
    Ok(state.meeting_store.rename_series(series_id.trim(), &title)?)
}

/// Forgets a series. Its recordings stay; they stop being in one.
#[tauri::command]
pub fn delete_meeting_series(
    state: State<'_, AppState>,
    series_id: String,
) -> Result<(), CommandError> {
    state.meeting_store.delete_series(series_id.trim())?;
    Ok(())
}

/// Puts a recording in a series, or takes it out of the one it is in.
///
/// `None` removes it. A membership set here is the user's and a later sync
/// leaves it alone, which is what makes this the escape hatch for a series
/// Google split by having the recurrence deleted and recreated.
#[tauri::command]
pub fn set_meeting_series(
    state: State<'_, AppState>,
    meeting_id: String,
    series_id: Option<String>,
) -> Result<Meeting, CommandError> {
    let series_id = series_id
        .map(|id| id.trim().to_string())
        .filter(|id| !id.is_empty());

    if let Some(id) = series_id.as_deref() {
        let known = state
            .meeting_store
            .load_series()
            .into_iter()
            .any(|entry| entry.id == id);
        if !known {
            return Err(CommandError::new(
                "MEETING_SERIES_NOT_FOUND",
                "That series no longer exists.",
            ));
        }
    }

    Ok(state
        .meeting_store
        .set_meeting_series(meeting_id.trim(), series_id)?)
}

fn validate_series_title(title: &str) -> Result<String, CommandError> {
    let title = title.trim().to_string();
    if title.is_empty() {
        return Err(CommandError::new(
            "MEETING_SERIES_INVALID_TITLE",
            "A series needs a name.",
        ));
    }
    if title.chars().count() > MAX_SERIES_TITLE {
        return Err(CommandError::new(
            "MEETING_SERIES_INVALID_TITLE",
            "That name is too long.",
        ));
    }
    Ok(title)
}

/// Opens a meeting's folder in the OS file manager.
#[tauri::command]
pub fn open_meeting_folder(
    app: AppHandle,
    state: State<'_, AppState>,
    meeting_id: String,
) -> Result<(), CommandError> {
    use tauri_plugin_opener::OpenerExt;
    let dir = state.meeting_store.meeting_dir(&meeting_id)?;
    if !dir.exists() {
        return Err(CommandError::new(
            "MEETING_NOT_FOUND",
            "That meeting's folder no longer exists.",
        ));
    }
    app.opener()
        .open_path(dir.to_string_lossy().to_string(), None::<&str>)
        .map_err(|err| CommandError::new("MEETING_OPEN_FAILED", &err.to_string()))
}

// --- summaries ----------------------------------------------------------

#[tauri::command]
pub fn list_meeting_templates(state: State<'_, AppState>) -> Vec<Template> {
    TemplateLibrary::new(&state.config_dir).list()
}

#[tauri::command]
pub fn generate_meeting_summary(
    app: AppHandle,
    state: State<'_, AppState>,
    meeting_id: String,
    template_id: Option<String>,
    language: Option<String>,
    instructions: Option<String>,
    force: Option<bool>,
) -> Result<(), CommandError> {
    let (provider, meetings) = {
        let settings = state.settings.lock_or_recover();
        (settings.provider.clone(), settings.meetings.clone())
    };
    // An explicit choice wins; otherwise the user's configured defaults do,
    // so the Generate button does the same thing as the last Generate button.
    let options = SummaryOptions {
        template_id: template_id
            .map(|id| id.trim().to_string())
            .filter(|id| !id.is_empty())
            .unwrap_or_else(|| {
                let configured = meetings.default_template_id.trim();
                if configured.is_empty() {
                    DEFAULT_TEMPLATE_ID.to_string()
                } else {
                    configured.to_string()
                }
            }),
        language: language
            .or_else(|| Some(meetings.summary_language.clone()))
            .map(|code| code.trim().to_string())
            .filter(|code| !code.is_empty()),
        user_instructions: instructions,
        force: force.unwrap_or(false),
    };
    state
        .summary_service
        .start(
            Some(app),
            &meeting_id,
            options,
            provider,
            TemplateLibrary::new(&state.config_dir),
        )
        .map_err(CommandError::from)
}

#[tauri::command]
pub fn cancel_meeting_summary(state: State<'_, AppState>, meeting_id: String) -> bool {
    state.summary_service.cancel(&meeting_id)
}

#[tauri::command]
pub fn get_meeting_summary(
    state: State<'_, AppState>,
    meeting_id: String,
) -> Result<Option<MeetingSummary>, CommandError> {
    state
        .summary_service
        .reconcile_orphaned(&meeting_id)
        .map_err(CommandError::from)
}

/// Saves the user's own edits to a report.
///
/// Clears the English cache: once a human has edited the report, regenerating
/// from a stale cached original would silently discard their edit.
#[tauri::command]
pub fn save_meeting_summary(
    state: State<'_, AppState>,
    meeting_id: String,
    markdown: String,
) -> Result<MeetingSummary, CommandError> {
    let store = &state.meeting_store;
    let mut summary = store
        .load_summary(&meeting_id)?
        .unwrap_or_else(|| MeetingSummary::pending(&meeting_id, DEFAULT_TEMPLATE_ID));
    summary.status = super::model::SummaryStatus::Completed;
    summary.markdown = Some(markdown);
    summary.english_markdown = None;
    summary.fingerprint = None;
    summary.error = None;
    summary.completed_at = Some(chrono::Utc::now().to_rfc3339());
    store.save_summary(&summary)?;
    Ok(summary)
}

/// Promotes a meeting into a Scribble, so it joins the knowledge graph.
///
/// The Scribble carries the report where one exists and the transcript
/// otherwise — a meeting with no summary is still a record worth connecting.
#[tauri::command]
pub fn promote_meeting_to_scribble(
    state: State<'_, AppState>,
    meeting_id: String,
) -> Result<crate::vault::Scribble, CommandError> {
    let store = &state.meeting_store;
    let meeting = store.load_meeting(&meeting_id)?;
    let summary = store.load_summary(&meeting_id)?;
    // The canonical transcript, not the raw segments: the knowledge layer is
    // downstream of the transcript everything else reads, and handing it a
    // different rendering would mean a graph built on text no report was
    // written from.
    let body = summary
        .as_ref()
        .and_then(|s| s.markdown.clone())
        .filter(|markdown| !markdown.trim().is_empty())
        .unwrap_or_else(|| {
            let segments = store.load_transcript(&meeting_id).unwrap_or_default();
            let canonical = crate::meetings::canonical::assemble(
                &meeting_id,
                &segments,
                &store.load_attribution(&meeting_id).unwrap_or_default(),
                &store.load_speakers(&meeting_id).unwrap_or_default(),
                meeting.transcript.clone(),
                &crate::meetings::canonical::AssemblyOptions::default(),
            );
            crate::meetings::canonical::render_transcript(&canonical)
        });
    if body.trim().is_empty() {
        return Err(CommandError::new(
            "MEETING_EMPTY_TRANSCRIPT",
            "That meeting has nothing to promote yet.",
        ));
    }

    let scribble = crate::vault::Scribble::from_meeting(&meeting.id, &meeting.title, &body);
    state
        .vault
        .save_scribble(&scribble)
        .map_err(|err| CommandError::new("MEETING_STORAGE_FAILED", &err.to_string()))?;
    Ok(scribble)
}

// --- import and re-transcription ----------------------------------------

#[tauri::command]
pub fn meeting_audio_extensions() -> Vec<String> {
    import::SUPPORTED_EXTENSIONS
        .iter()
        .map(|ext| (*ext).to_string())
        .collect()
}

/// Opens the OS file picker for a recording to import.
///
/// In Rust rather than through the dialog plugin's JS API: the window's
/// capability set deliberately grants only core and notification permissions,
/// so the frontend cannot open a file dialog itself.
#[tauri::command]
pub async fn pick_meeting_audio_file(app: AppHandle) -> Result<Option<String>, CommandError> {
    use tauri_plugin_dialog::DialogExt;
    let picked = tauri::async_runtime::spawn_blocking(move || {
        app.dialog()
            .file()
            .set_title("Select a recording")
            .add_filter("Audio", import::SUPPORTED_EXTENSIONS)
            .blocking_pick_file()
    })
    .await
    .map_err(|err| CommandError::new("DIALOG_TASK_FAILED", &err.to_string()))?;

    crate::commands::picked_path(picked)
}

#[tauri::command]
pub async fn import_meeting_audio(
    app: AppHandle,
    path: String,
    title: Option<String>,
) -> Result<Meeting, CommandError> {
    let handle = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let state = handle.state::<AppState>();
        let config = batch_config(&state, &BatchOverrides::default())?;
        let key = format!("import:{path}");
        let cancel = state.meeting_imports.register(&key);
        let result = import::import_audio(
            Some(handle.clone()),
            &state.meeting_store,
            &state.stt,
            &PathBuf::from(&path),
            title,
            config,
            cancel,
        );
        state.meeting_imports.finish(&key);
        result.map_err(CommandError::from)
    })
    .await
    .map_err(|err| CommandError::new("MEETING_TASK_FAILED", &err.to_string()))?
}

/// Transcribes a meeting's saved recording again.
///
/// `overrides` is what makes this worth pressing twice. The first attempt
/// already used the user's settings; if it came back wrong, running it again
/// with the same settings produces the same wrong transcript. The dialog
/// behind this passes a language to pin, a bigger model, or a slower preset.
#[tauri::command]
pub async fn retranscribe_meeting(
    app: AppHandle,
    meeting_id: String,
    overrides: Option<BatchOverrides>,
) -> Result<Meeting, CommandError> {
    let overrides = overrides.unwrap_or_default();
    if let Some(preset) = overrides.preset.as_deref() {
        if !known_preset(preset) {
            return Err(CommandError::new(
                "MEETING_UNKNOWN_PRESET",
                "That is not a decode quality Vox knows. Choose fast, balanced or quality.",
            ));
        }
    }
    let handle = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let state = handle.state::<AppState>();
        let config = batch_config(&state, &overrides)?;
        let cancel = state.meeting_imports.register(&meeting_id);
        let result = import::retranscribe(
            Some(handle.clone()),
            &state.meeting_store,
            &state.stt,
            &meeting_id,
            config,
            cancel,
        );
        state.meeting_imports.finish(&meeting_id);
        result.map_err(CommandError::from)
    })
    .await
    .map_err(|err| CommandError::new("MEETING_TASK_FAILED", &err.to_string()))?
}

/// Produces the English view of a meeting by decoding its audio again.
///
/// Separate from [`retranscribe_meeting`] because it leaves the transcript
/// alone: someone who is happy with their Hindi transcript and only wants to
/// be able to read it should not have to risk replacing it.
#[tauri::command]
pub async fn generate_meeting_english_track(
    app: AppHandle,
    meeting_id: String,
    overrides: Option<BatchOverrides>,
) -> Result<usize, CommandError> {
    let overrides = overrides.unwrap_or_default();
    let handle = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let state = handle.state::<AppState>();
        let config = batch_config(&state, &overrides)?;
        let key = format!("english:{meeting_id}");
        let cancel = state.meeting_imports.register(&key);
        let result = import::generate_english_track(
            Some(handle.clone()),
            &state.meeting_store,
            &state.stt,
            &meeting_id,
            config,
            cancel,
        );
        state.meeting_imports.finish(&key);
        result.map_err(CommandError::from)
    })
    .await
    .map_err(|err| CommandError::new("MEETING_TASK_FAILED", &err.to_string()))?
}

/// Works out who spoke, and offers a sample of each voice to put a name to.
///
/// The grouping is a proposal, not an assertion — see
/// [`crate::meetings::voiceprint`] for what the technique underneath can and
/// cannot do. Every speaker comes back with a span of the recording where that
/// voice is talking alone, which is what makes a wrong proposal cost one
/// rename rather than an argument with the software.
#[tauri::command]
pub async fn detect_meeting_speakers(
    app: AppHandle,
    meeting_id: String,
) -> Result<crate::meetings::speakers::SpeakerReport, CommandError> {
    use crate::meetings::speakers;

    let handle = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let state = handle.state::<AppState>();
        let store = &state.meeting_store;
        let segments = store.load_transcript(&meeting_id)?;
        if segments.is_empty() {
            return Err(CommandError::new(
                "MEETING_EMPTY_TRANSCRIPT",
                "This meeting has no transcript to attribute.",
            ));
        }

        let key = format!("speakers:{meeting_id}");
        let cancel = state.meeting_imports.register(&key);
        let prints = import::fingerprint_segments(store, &meeting_id, &segments, &cancel);
        state.meeting_imports.finish(&key);
        let prints = prints.map_err(CommandError::from)?;

        // Names the user typed survive the re-run; Vox's own placeholders do
        // not, because a re-run is entitled to renumber its own guesses.
        let previous = store.load_speakers(&meeting_id)?;
        let report = speakers::assign_speakers(
            &segments,
            &prints,
            &speakers::DetectionSettings::default(),
            &previous,
        );

        // The transcript is not rewritten. Detection is an interpretation of
        // evidence and used to be written back onto the file holding what the
        // decoder said, so re-running it modified the record of the decode.
        store.save_attribution(&meeting_id, &report.attribution)?;
        store.save_speakers(&meeting_id, &report.speakers)?;
        Ok(report)
    })
    .await
    .map_err(|err| CommandError::new("MEETING_TASK_FAILED", &err.to_string()))?
}

/// Puts a name to one of the voices detection found.
#[tauri::command]
pub fn rename_meeting_speaker(
    state: State<'_, AppState>,
    meeting_id: String,
    speaker_id: String,
    label: String,
) -> Result<Vec<crate::meetings::model::Speaker>, CommandError> {
    let label = label.trim().to_string();
    if label.is_empty() {
        return Err(CommandError::new(
            "MEETING_INVALID_SPEAKER_NAME",
            "A speaker needs a name.",
        ));
    }
    if label.chars().count() > 80 {
        return Err(CommandError::new(
            "MEETING_INVALID_SPEAKER_NAME",
            "That name is too long.",
        ));
    }

    let store = &state.meeting_store;
    let mut speakers = store.load_speakers(&meeting_id)?;
    let Some(speaker) = speakers.iter_mut().find(|s| s.id == speaker_id) else {
        return Err(CommandError::new(
            "MEETING_UNKNOWN_SPEAKER",
            "That speaker is not part of this meeting.",
        ));
    };
    speaker.label = label;
    // Marked as the user's word, which is what stops the next detection run
    // renumbering it back to "Speaker 3".
    speaker.named_by_user = true;
    store.save_speakers(&meeting_id, &speakers)?;
    Ok(speakers)
}

#[tauri::command]
pub fn cancel_meeting_import(state: State<'_, AppState>, key: String) -> bool {
    state.meeting_imports.cancel(&key)
}

/// What one batch run may override about the user's saved settings.
///
/// A re-transcription is what someone reaches for when the transcript they
/// have is wrong, so it is the one place where the settings that produced it
/// are the least trustworthy thing to reuse. Every field is `None` by default
/// and falls back to settings, so an import — which has no transcript to
/// disbelieve yet — passes `Default::default()` and behaves exactly as before.
#[derive(Debug, Clone, Default, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BatchOverrides {
    /// An ISO code to pin, or `"auto"` to force detection. Empty/absent means
    /// whatever settings resolve to.
    pub language: Option<String>,
    /// A speech-model catalogue id to decode with, overriding the configured
    /// meeting model for this run only.
    pub model_id: Option<String>,
    /// `"fast"`, `"balanced"` or `"quality"`. Absent means the batch default,
    /// which is `quality`.
    pub preset: Option<String>,
    /// Whether the run also decodes an English rendering of every segment.
    /// Roughly doubles the time, and is what makes a meeting held in another
    /// language readable without a model call.
    pub english_track: Option<bool>,
}

/// Batch decoding configuration from the user's current settings.
fn batch_config(
    state: &State<'_, AppState>,
    overrides: &BatchOverrides,
) -> Result<BatchConfig, CommandError> {
    use crate::capture::stt::{
        resolve_meeting_model_path, SttLanguageConfig, SttWindow, WhisperDecodingConfig,
    };

    let mut settings = state.settings.lock_or_recover().clone();
    if let Some(model_id) = overrides.model_id.as_deref().map(str::trim) {
        if !model_id.is_empty() {
            settings.stt.meeting_model_id = Some(model_id.to_string());
        }
    }
    if let Some(preset) = overrides.preset.as_deref().map(str::trim) {
        if !preset.is_empty() {
            settings.stt.preset = preset.to_string();
        }
    }

    let models_dir = state.config_dir.join("models");
    let model_path = resolve_meeting_model_path(&models_dir, &settings.stt).ok_or_else(|| {
        CommandError::new(
            "MEETING_NO_SPEECH_MODEL",
            "No speech model is installed. Install one under Settings › Speech.",
        )
    })?;

    // An explicit choice for this run wins over the standing setting, which
    // wins over the language profile's own resolution.
    let language_override = overrides
        .language
        .as_deref()
        .map(str::trim)
        .filter(|code| !code.is_empty())
        .unwrap_or(settings.meetings.transcription_language.as_str());

    Ok(BatchConfig {
        model_path: model_path.to_string_lossy().to_string(),
        language: SttLanguageConfig::from_settings_with_override(
            &settings.language,
            SttWindow::LongForm,
            language_override,
        ),
        // `for_meeting_batch`, not `for_meetings`: nothing is racing a file on
        // disk, so the encoder clamp and the fast preset both come off. The
        // absent `decoding_expensive_script` is the other half of that — see
        // [`BatchConfig`].
        decoding: WhisperDecodingConfig::for_meeting_batch(&settings.stt),
        glossary: settings.dictionary.clone(),
        english_track: overrides.english_track.unwrap_or(false),
    })
}

/// Whether `preset` names a decode preset Vox knows.
///
/// `SttPreset::from_setting` defaults an unknown value to `Fast`, which is the
/// safe thing for a settings file written by an older build and the wrong
/// thing for an argument the user just chose: silently decoding at the
/// *lowest* quality is indistinguishable from the bug this whole path exists
/// to fix.
fn known_preset(preset: &str) -> bool {
    matches!(
        preset.trim().to_lowercase().as_str(),
        "" | "fast" | "balanced" | "quality"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_import_can_be_cancelled_and_only_once_it_is_registered() {
        let registry = ImportRegistry::default();
        assert!(!registry.cancel("nothing"));

        let flag = registry.register("meeting-a");
        assert!(!flag.load(Ordering::SeqCst));
        assert!(registry.cancel("meeting-a"));
        assert!(flag.load(Ordering::SeqCst));

        registry.finish("meeting-a");
        assert!(!registry.cancel("meeting-a"));
    }

    #[test]
    fn cancelling_everything_reaches_every_run() {
        let registry = ImportRegistry::default();
        let a = registry.register("a");
        let b = registry.register("b");
        registry.cancel_all();
        assert!(a.load(Ordering::SeqCst));
        assert!(b.load(Ordering::SeqCst));
    }

    #[test]
    fn engine_errors_carry_a_code_the_frontend_can_branch_on() {
        let err: CommandError = MeetingEngineError::NoSpeechModel.into();
        assert_eq!(err.code, "MEETING_NO_SPEECH_MODEL");
        assert!(err.message.contains("Settings"), "the message must say what to do");

        let err: CommandError = MeetingEngineError::AlreadyRecording.into();
        assert_eq!(err.code, "MEETING_ALREADY_RECORDING");
    }

    #[test]
    fn a_missing_meeting_is_distinguishable_from_a_broken_vault() {
        let missing: CommandError = MeetingStoreError::NotFound("meeting-x".into()).into();
        assert_eq!(missing.code, "MEETING_NOT_FOUND");
        let broken: CommandError =
            MeetingStoreError::Serde("unexpected end of input".into()).into();
        assert_eq!(broken.code, "MEETING_STORAGE_FAILED");
    }

    #[test]
    fn an_unsupported_import_is_distinguishable_from_a_corrupt_one() {
        let unsupported: CommandError = ImportError::UnsupportedFormat("txt".into()).into();
        assert_eq!(unsupported.code, "MEETING_UNSUPPORTED_FORMAT");
        let corrupt: CommandError = ImportError::Decode("bad header".into()).into();
        assert_eq!(corrupt.code, "MEETING_DECODE_FAILED");
    }

    #[test]
    fn every_supported_extension_is_offered_to_the_file_picker() {
        let offered = meeting_audio_extensions();
        assert_eq!(offered.len(), import::SUPPORTED_EXTENSIONS.len());
        assert!(offered.contains(&"wav".to_string()));
        assert!(offered.contains(&"m4a".to_string()));
    }
}

// --- diagnostics --------------------------------------------------------

/// Every per-segment record for a meeting, in sequence order.
///
/// Separate from the rollup because it is large and is only wanted when the
/// rollup has already said something is wrong: the rollup says the p95 was
/// twelve seconds, and this says which segments they were.
#[tauri::command]
pub fn get_meeting_segment_diagnostics(
    state: State<'_, AppState>,
    meeting_id: String,
) -> Result<Vec<crate::meetings::telemetry::SegmentDiagnostics>, CommandError> {
    state
        .meeting_store
        .load_segment_diagnostics(&meeting_id)
        .map_err(CommandError::from)
}

// --- speech benchmark ---------------------------------------------------

/// Key the benchmark's cancellation registers under. One run at a time: a
/// benchmark that competed with itself for the single loaded model would
/// measure the contention rather than the engine.
const BENCHMARK_RUN_KEY: &str = "speech-benchmark";

/// A starter corpus manifest — one entry per condition, with the recordings
/// the user has to supply named rather than invented.
#[tauri::command]
pub fn speech_benchmark_template() -> benchmark::BenchmarkManifest {
    benchmark::BenchmarkManifest::template()
}

/// Reads a corpus manifest and reports what it covers, without running it.
///
/// Separate from running because a corpus is assembled over weeks: "which
/// conditions do I still have no recording for" is a question worth answering
/// in a second rather than after an hour of decoding.
#[tauri::command]
pub fn inspect_speech_benchmark(
    manifest_path: String,
) -> Result<benchmark::BenchmarkManifest, CommandError> {
    benchmark::BenchmarkManifest::load(std::path::Path::new(&manifest_path))
        .map_err(CommandError::from)
}

/// What one benchmark run needs that the manifest does not say.
#[derive(Debug, Clone, serde::Deserialize)]
pub struct BenchmarkRequest {
    /// Path to the corpus manifest. Its directory is the corpus root, and
    /// every path inside it is resolved relative to that and may not escape.
    pub manifest_path: String,
    pub engine: crate::capture::recognizers::RecognizerChoice,
    #[serde(default)]
    pub pacing: Option<String>,
    /// Where to write the report. Defaults to a timestamped directory beside
    /// the manifest.
    #[serde(default)]
    pub out_dir: Option<String>,
    /// Decode overrides, so two profiles can be compared over one corpus.
    #[serde(default)]
    pub overrides: Option<BatchOverrides>,
}

/// Runs a corpus against one engine and writes the report.
///
/// Long-running by design — the long categories are half the point — so it
/// reports progress and honours cancellation through the same registry
/// imports use.
#[tauri::command]
pub async fn run_speech_benchmark(
    app: AppHandle,
    request: BenchmarkRequest,
) -> Result<benchmark::BenchmarkReport, CommandError> {
    let handle = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let state = handle.state::<AppState>();
        let manifest_path = std::path::PathBuf::from(&request.manifest_path);
        let corpus_root = manifest_path
            .parent()
            .map(std::path::Path::to_path_buf)
            .ok_or_else(|| {
                CommandError::new(
                    "BENCHMARK_BAD_MANIFEST",
                    "The manifest path has no directory to resolve the corpus against.",
                )
            })?;
        let manifest = benchmark::BenchmarkManifest::load(&manifest_path)?;

        let pacing = match request.pacing.as_deref() {
            Some("realtime") => benchmark::Pacing::Realtime,
            _ => benchmark::Pacing::Batch,
        };
        let overrides = request.overrides.unwrap_or_default();
        let batch = batch_config(&state, &overrides)?;
        let glossary = state.settings.lock_or_recover().dictionary.clone();

        let cancel = state.meeting_imports.register(BENCHMARK_RUN_KEY);
        let stt = state.stt.clone();
        let choice = request.engine.clone();
        let decoding = batch.decoding.clone();
        let run = benchmark::RunProfile {
            pacing,
            language: batch.language.whisper_language.clone(),
            translate: batch.language.translate,
            profile: format!("{:?}", batch.decoding.strategy),
        };

        let emitter = handle.clone();
        let result = benchmark::run_manifest(
            &corpus_root,
            &manifest,
            || {
                choice
                    .build(stt.clone(), decoding.clone())
                    .map(std::sync::Arc::from)
                    .map_err(|err| benchmark::BenchmarkError::Invalid(err.to_string()))
            },
            run,
            &glossary,
            &cancel,
            |case_id, index, total| {
                let _ = emitter.emit(
                    BENCHMARK_PROGRESS_EVENT,
                    BenchmarkProgress {
                        case_id: case_id.to_string(),
                        index,
                        total,
                    },
                );
            },
        );
        state.meeting_imports.finish(BENCHMARK_RUN_KEY);
        let report = result?;

        let out_dir = request
            .out_dir
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| {
                corpus_root.join("results").join(
                    chrono::Utc::now()
                        .format("%Y%m%dT%H%M%SZ")
                        .to_string(),
                )
            });
        report.write_to(&out_dir)?;
        Ok(report)
    })
    .await
    .map_err(|err| CommandError::new("MEETING_TASK_FAILED", &err.to_string()))?
}

/// Stops a benchmark part-way. The cases already finished are still reported.
#[tauri::command]
pub fn cancel_speech_benchmark(state: State<'_, AppState>) -> bool {
    state.meeting_imports.cancel(BENCHMARK_RUN_KEY)
}

/// How far through a corpus a run is.
pub const BENCHMARK_PROGRESS_EVENT: &str = "speech-benchmark-progress";

#[derive(Debug, Clone, serde::Serialize)]
pub struct BenchmarkProgress {
    pub case_id: String,
    pub index: usize,
    pub total: usize,
}
