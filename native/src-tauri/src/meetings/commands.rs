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

use tauri::{AppHandle, Manager, State};

use crate::commands::{AppState, CommandError};
use crate::sync::MutexExt;

use super::engine::{MeetingEngineError, MeetingRecordingStatus};
use super::import::{self, BatchConfig, ImportError};
use super::model::{Meeting, MeetingListItem, MeetingSummary, TranscriptSegment};
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

impl From<SummaryError> for CommandError {
    fn from(err: SummaryError) -> Self {
        let code = match err {
            SummaryError::EmptyTranscript => "MEETING_EMPTY_TRANSCRIPT",
            SummaryError::AlreadyRunning => "MEETING_SUMMARY_RUNNING",
            SummaryError::Cancelled => "MEETING_SUMMARY_CANCELLED",
            SummaryError::Provider(_) => "MEETING_SUMMARY_PROVIDER_FAILED",
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
) -> Result<Meeting, CommandError> {
    let handle = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let state = handle.state::<AppState>();
        let settings = state.settings.lock_or_recover().clone();
        state
            .meeting_engine
            .start(
                Some(handle.clone()),
                title,
                &settings,
                &state.config_dir,
                state.stt.clone(),
                capture_system_audio.unwrap_or(settings.meetings.capture_system_audio),
            )
            .map_err(CommandError::from)
    })
    .await
    .map_err(|err| CommandError::new("MEETING_TASK_FAILED", &err.to_string()))?
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
    Ok(MeetingDetail {
        meeting: store.load_meeting(&meeting_id)?,
        segments: store.load_transcript(&meeting_id)?,
        summary: store.load_summary(&meeting_id)?,
        notes: store.load_notes(&meeting_id)?,
    })
}

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
    state.meeting_store.load_summary(&meeting_id).map_err(CommandError::from)
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
    let body = summary
        .as_ref()
        .and_then(|s| s.markdown.clone())
        .filter(|markdown| !markdown.trim().is_empty())
        .unwrap_or_else(|| {
            super::transcription::render_transcript(&store.load_transcript(&meeting_id).unwrap_or_default())
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
        let config = batch_config(&state)?;
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

#[tauri::command]
pub async fn retranscribe_meeting(
    app: AppHandle,
    meeting_id: String,
) -> Result<Meeting, CommandError> {
    let handle = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let state = handle.state::<AppState>();
        let config = batch_config(&state)?;
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

#[tauri::command]
pub fn cancel_meeting_import(state: State<'_, AppState>, key: String) -> bool {
    state.meeting_imports.cancel(&key)
}

/// Batch decoding configuration from the user's current settings.
fn batch_config(state: &State<'_, AppState>) -> Result<BatchConfig, CommandError> {
    use crate::capture::stt::{
        resolve_meeting_model_path, SttLanguageConfig, SttPreset, SttWindow, WhisperDecodingConfig,
    };

    let settings = state.settings.lock_or_recover().clone();
    let models_dir = state.config_dir.join("models");
    let model_path =
        resolve_meeting_model_path(&models_dir, settings.stt.whisper_model_path.as_deref())
            .filter(|path| path.exists())
            .ok_or_else(|| {
                CommandError::new(
                    "MEETING_NO_SPEECH_MODEL",
                    "No speech model is installed. Install one under Settings › Speech.",
                )
            })?;

    Ok(BatchConfig {
        model_path: model_path.to_string_lossy().to_string(),
        language: SttLanguageConfig::from_settings(&settings.language, SttWindow::LongForm),
        decoding: WhisperDecodingConfig::from_settings_defaulting(
            &settings.stt,
            SttPreset::Balanced,
        ),
        glossary: settings.dictionary.clone(),
    })
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
