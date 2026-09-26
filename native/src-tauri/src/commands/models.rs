//! Models: speech models, Parakeet, Ollama, and the local LLM.

use super::*;

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum SttModelStatus {
    Ready { path: String },
    Failed { message: String },
}

/// Removes "go find and download a GGML model yourself" as a prerequisite:
/// downloads the default one now if nothing is configured, and reports
/// where things stand so Settings can show real status instead of the
/// user finding out only when a capture silently fails.
#[tauri::command]
pub async fn ensure_stt_model_ready(state: State<'_, AppState>) -> Result<SttModelStatus, CommandError> {
    let models_dir = state.config_dir.join("models");
    let configured = state
        .settings
        .lock_or_recover()
        .stt
        .whisper_model_path
        .clone()
        .filter(|p| !p.trim().is_empty());

    if let Some(ref path_str) = configured {
        let path = std::path::Path::new(path_str);
        if path.exists() {
            if !crate::capture::stt::is_legacy_default_model(path) {
                // User has an explicit custom model that exists
                return Ok(SttModelStatus::Ready {
                    path: path_str.clone(),
                });
            }
            // If it's a legacy default model (e.g. ggml-base.bin), proceed to ensure production ggml-small.bin
        } else {
            return Ok(SttModelStatus::Failed {
                message: format!(
                    "Configured model path not found: '{}'. Expected production model: '{}'.",
                    path_str,
                    crate::capture::stt::DEFAULT_MODEL_FILENAME
                ),
            });
        }
    }

    match crate::capture::stt::ensure_default_model(&models_dir).await {
        Ok(path) => {
            let path_str = path.to_string_lossy().to_string();
            let mut settings = state.settings.lock_or_recover();
            settings.stt.whisper_model_path = Some(path_str.clone());
            let _ = settings.save(&state.settings_path());
            Ok(SttModelStatus::Ready { path: path_str })
        }
        Err(e) => Ok(SttModelStatus::Failed {
            message: e.to_string(),
        }),
    }
}

/// Removes the "install and manually start Ollama" step for local mode:
/// starts it and pulls the configured model if needed. A no-op for the
/// Cloud API path, which is unaffected.
#[tauri::command]
pub async fn ensure_local_llm_ready(state: State<'_, AppState>) -> Result<OllamaStatus, CommandError> {
    let settings = state.settings.lock_or_recover().clone();
    if !matches!(settings.provider.active_provider, ProviderType::Ollama) {
        return Ok(OllamaStatus::Running);
    }
    Ok(crate::providers::ensure_ollama_ready(&settings.provider.ollama_host, &settings.provider.ollama_model).await)
}

#[tauri::command]
pub async fn get_available_llm_models(
    host: Option<String>,
    state: State<'_, AppState>,
) -> Result<Vec<crate::providers::OllamaModelDetails>, CommandError> {
    let host = host.unwrap_or_else(|| {
        state.settings.lock_or_recover().provider.ollama_host.clone()
    });
    crate::providers::list_installed_models(&host)
        .await
        .map_err(|e| CommandError::new("OLLAMA_QUERY_FAILED", &e))
}

#[tauri::command]
pub async fn test_llm_prompt(
    host: Option<String>,
    model: String,
    prompt: Option<String>,
    state: State<'_, AppState>,
) -> Result<crate::providers::OllamaPromptTestResult, CommandError> {
    let host = host.unwrap_or_else(|| {
        state.settings.lock_or_recover().provider.ollama_host.clone()
    });
    let prompt = prompt.unwrap_or_else(|| "Hello! Reply with 'Vox AI ready' in under 5 words.".to_string());
    Ok(crate::providers::test_ollama_prompt(&host, &model, &prompt).await)
}

#[tauri::command]
pub async fn get_available_stt_models(
    state: State<'_, AppState>,
) -> Result<crate::capture::stt::SttModelsOverview, CommandError> {
    let models_dir = state.config_dir.join("models");
    let stt_settings = state.settings.lock_or_recover().stt.clone();
    Ok(crate::capture::stt::get_stt_models_overview(&models_dir, &stt_settings))
}

/// Downloads a managed Whisper model the user asked for.
///
/// Separate from [`ensure_stt_model_ready`], which fetches the default so a
/// first capture works at all. This one is a deliberate choice: the accuracy
/// ceiling is ~1.6 GB and costs real decode time, so it is never fetched
/// implicitly and never becomes the active model as a side effect of being
/// downloaded. Selecting it stays the user's separate act.
///
/// Reports a failed download as `Failed` rather than as a command error, the
/// same way `ensure_stt_model_ready` does, so Settings can render the reason
/// instead of a toast with a stack in it.
#[tauri::command]
pub async fn download_stt_model(
    state: State<'_, AppState>,
    filename: String,
) -> Result<SttModelStatus, CommandError> {
    let models_dir = state.config_dir.join("models");
    match crate::capture::stt::ensure_managed_model(&models_dir, &filename).await {
        Ok(path) => Ok(SttModelStatus::Ready {
            path: path.to_string_lossy().to_string(),
        }),
        Err(e) => Ok(SttModelStatus::Failed {
            message: e.to_string(),
        }),
    }
}

#[tauri::command]
pub async fn test_stt_model(
    model_path: String,
) -> Result<crate::capture::stt::SttModelTestResult, CommandError> {
    Ok(crate::capture::stt::test_stt_model_file(&model_path))
}

/// The event an Ollama install reports progress on.
pub const OLLAMA_INSTALL_EVENT: &str = "ollama-install";

/// What installing Ollama on this machine would involve, or `None` where Vox
/// has no way to.
#[tauri::command]
pub async fn get_ollama_install_plan(
) -> Result<Option<crate::providers::ollama_install::InstallPlan>, CommandError> {
    Ok(crate::providers::ollama_install::plan_for_this_machine())
}

/// Downloads and installs Ollama, reporting progress on
/// [`OLLAMA_INSTALL_EVENT`].
///
/// Runs only because someone pressed a button. On Windows it opens the
/// platform's own installer and returns — the user completes it and can
/// cancel, which is the right shape for running something freshly downloaded.
/// Elsewhere the archive is unpacked into Vox's data directory, needing no
/// root and touching nothing outside it.
#[tauri::command]
pub async fn install_ollama(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<crate::providers::ollama_install::InstallProgress, CommandError> {
    use crate::providers::ollama_install::{self, ArtifactKind, InstallProgress};

    let plan = ollama_install::plan_for_this_machine().ok_or_else(|| {
        CommandError::new(
            "OLLAMA_UNSUPPORTED_PLATFORM",
            &ollama_install::InstallError::UnsupportedPlatform.to_string(),
        )
    })?;

    let data_dir = state.config_dir.clone();
    let downloads = data_dir.join("downloads");
    let emitter = app.clone();

    let archive = ollama_install::download(&plan, &downloads, move |progress| {
        let _ = emitter.emit(OLLAMA_INSTALL_EVENT, &progress);
    })
    .await
    .map_err(|e| CommandError::new("OLLAMA_INSTALL_FAILED", &e.to_string()))?;

    let outcome = match plan.kind {
        ArtifactKind::Installer => {
            ollama_install::launch_installer(&archive)
                .map_err(|e| CommandError::new("OLLAMA_INSTALL_FAILED", &e.to_string()))?;
            InstallProgress::AwaitingUser
        }
        ArtifactKind::Archive => {
            let binary = ollama_install::extract_archive(&archive, &data_dir)
                .map_err(|e| CommandError::new("OLLAMA_INSTALL_FAILED", &e.to_string()))?;
            crate::providers::set_managed_binary(Some(binary));
            InstallProgress::Ready
        }
    };

    let _ = app.emit(OLLAMA_INSTALL_EVENT, &outcome);
    Ok(outcome)
}

/// The event a speech-model download reports progress on.
pub const SPEECH_MODEL_DOWNLOAD_EVENT: &str = "speech-model-download";

/// The whole speech-model catalogue crossed with what is installed.
///
/// Supersedes [`get_available_stt_models`], which knew about three of the
/// twelve whisper.cpp tiers and could not say which of them a meeting would
/// actually use. Kept alongside it rather than replacing it outright because
/// Diagnostics reads the older shape.
#[tauri::command]
pub async fn list_speech_models(
    state: State<'_, AppState>,
) -> Result<crate::capture::models::SpeechModelCatalogue, CommandError> {
    use crate::capture::models;

    let models_dir = state.config_dir.join("models");
    let stt = state.settings.lock_or_recover().stt.clone();

    let active_meeting_model =
        crate::capture::stt::resolve_meeting_model_path(&models_dir, &stt).map(|path| {
            models::by_path(&path)
                .map(|m| m.id.to_string())
                .unwrap_or_else(|| {
                    path.file_name()
                        .map(|n| n.to_string_lossy().to_string())
                        .unwrap_or_default()
                })
        });

    let active_dictation_model = crate::capture::stt::dictation_model_filename(&stt)
        .map(|filename| {
            models::by_filename(&filename)
                .map(|m| m.id.to_string())
                .unwrap_or(filename)
        });

    Ok(models::SpeechModelCatalogue {
        models_dir: models_dir.to_string_lossy().to_string(),
        models: models::installed(&models_dir),
        active_meeting_model,
        active_dictation_model,
        recommended_meeting_model: models::RECOMMENDED_MEETING_MODEL_ID.to_string(),
    })
}

/// Fetches one catalogue model, emitting progress on
/// [`SPEECH_MODEL_DOWNLOAD_EVENT`] as it goes.
///
/// Returns once the download finishes. The events are what make a 1.6 GB fetch
/// bearable to watch; the returned `Result` is what tells the caller whether to
/// re-read the catalogue. Downloading does not select the model — a larger
/// model costs decode time on every utterance, so making it active stays a
/// separate press.
#[tauri::command]
pub async fn download_speech_model(
    app: AppHandle,
    state: State<'_, AppState>,
    id: String,
) -> Result<crate::capture::models::InstalledModel, CommandError> {
    use crate::capture::models;

    let models_dir = state.config_dir.join("models");
    let emitter = app.clone();
    models::download(&models_dir, &id, move |progress| {
        let _ = emitter.emit(SPEECH_MODEL_DOWNLOAD_EVENT, &progress);
    })
    .await
    .map_err(|e| CommandError::new("SPEECH_MODEL_DOWNLOAD_FAILED", &e.to_string()))?;

    models::installed(&models_dir)
        .into_iter()
        .find(|m| m.id == id)
        .ok_or_else(|| {
            CommandError::new(
                "SPEECH_MODEL_DOWNLOAD_FAILED",
                "the download finished but the model is not on disk",
            )
        })
}

/// Asks a running download to stop. False when none was running.
#[tauri::command]
pub fn cancel_speech_model_download(id: String) -> bool {
    crate::capture::models::cancel(&id)
}

/// Removes an installed model from disk.
///
/// Clears the meeting selection if it pointed here, so the next recording
/// resolves to something that exists rather than failing on a model the user
/// just deleted.
#[tauri::command]
pub async fn delete_speech_model(
    app: AppHandle,
    state: State<'_, AppState>,
    id: String,
) -> Result<bool, CommandError> {
    let models_dir = state.config_dir.join("models");
    let removed = crate::capture::models::delete(&models_dir, &id)
        .map_err(|e| CommandError::new("SPEECH_MODEL_DELETE_FAILED", &e.to_string()))?;

    if removed {
        let cleared = {
            let mut settings = state.settings.lock_or_recover();
            let mut changed = false;
            if settings.stt.meeting_model_id.as_deref() == Some(id.as_str()) {
                settings.stt.meeting_model_id = None;
                changed = true;
            }
            if let Some(ref path) = settings.stt.whisper_model_path {
                let deleted_filename = crate::capture::models::by_id(&id)
                    .map(|m| m.filename)
                    .unwrap_or(&id);
                if path.ends_with(deleted_filename) {
                    settings.stt.whisper_model_path = None;
                    changed = true;
                }
            }
            if changed {
                Some(settings.clone())
            } else {
                None
            }
        };
        if let Some(settings) = cleared {
            persist_settings(&app, &state, settings)?;
        }
    }

    Ok(removed)
}

/// Chooses the model meetings are transcribed with. `None` means "whatever is
/// installed", resolved at the moment a recording starts.
#[tauri::command]
pub async fn set_meeting_speech_model(
    app: AppHandle,
    state: State<'_, AppState>,
    id: Option<String>,
) -> Result<(), CommandError> {
    let id = id.map(|v| v.trim().to_string()).filter(|v| !v.is_empty());

    if let Some(ref id) = id {
        let models_dir = state.config_dir.join("models");
        let known = crate::capture::models::by_id(id)
            .map(|m| m.path_in(&models_dir))
            .unwrap_or_else(|| models_dir.join(id));
        if !crate::capture::models::is_installed(&known) {
            return Err(CommandError::new(
                "SPEECH_MODEL_NOT_INSTALLED",
                "that model is not installed yet",
            ));
        }
    }

    let settings = {
        let mut guard = state.settings.lock_or_recover();
        guard.stt.meeting_model_id = id;
        guard.clone()
    };
    persist_settings(&app, &state, settings)
}

/// Chooses the model dictation uses. `None` means default.
#[tauri::command]
pub async fn set_dictation_speech_model(
    app: AppHandle,
    state: State<'_, AppState>,
    id: Option<String>,
) -> Result<(), CommandError> {
    let id = id.map(|v| v.trim().to_string()).filter(|v| !v.is_empty());

    let path_str = if let Some(ref id) = id {
        let models_dir = state.config_dir.join("models");
        let known = crate::capture::models::by_id(id)
            .map(|m| m.path_in(&models_dir))
            .unwrap_or_else(|| models_dir.join(id));
        if !crate::capture::models::is_installed(&known) {
            return Err(CommandError::new(
                "SPEECH_MODEL_NOT_INSTALLED",
                "that model is not installed yet",
            ));
        }
        Some(known.to_string_lossy().to_string())
    } else {
        None
    };

    let settings = {
        let mut guard = state.settings.lock_or_recover();
        guard.stt.whisper_model_path = path_str;
        // When setting a Whisper model for dictation, ensure dictation engine is whisper
        if guard.stt.dictation_engine.as_deref() == Some("parakeet") {
            guard.stt.dictation_engine = Some("whisper".to_string());
        }
        guard.clone()
    };
    persist_settings(&app, &state, settings)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParakeetStatus {
    pub supported: bool,
    pub installed: bool,
    pub active_for_dictation: bool,
    pub missing_files: Vec<String>,
    pub models_dir: String,
    pub approx_total_bytes: u64,
}

#[tauri::command]
pub async fn get_parakeet_status(state: State<'_, AppState>) -> Result<ParakeetStatus, CommandError> {
    let models_dir = state.config_dir.join("models");
    let parakeet_dir = models_dir.join("parakeet");
    let files = crate::capture::parakeet::ModelFiles::in_dir(&parakeet_dir, true);
    let installed = crate::capture::parakeet::ModelFiles::is_installed_in(&parakeet_dir);
    let stt = state.settings.lock_or_recover().stt.clone();
    let active_for_dictation = stt.dictation_engine.as_deref() == Some("parakeet");

    Ok(ParakeetStatus {
        supported: cfg!(feature = "parakeet"),
        installed,
        active_for_dictation,
        missing_files: if installed { Vec::new() } else { files.missing() },
        models_dir: parakeet_dir.to_string_lossy().to_string(),
        approx_total_bytes: crate::capture::models::PARAKEET_TOTAL_BYTES,
    })
}

#[tauri::command]
pub async fn download_parakeet_model(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<String, CommandError> {
    use crate::capture::models;

    let models_dir = state.config_dir.join("models");
    let emitter = app.clone();
    let path = models::download_parakeet(&models_dir, move |progress| {
        let _ = emitter.emit(SPEECH_MODEL_DOWNLOAD_EVENT, &progress);
    })
    .await
    .map_err(|e| CommandError::new("PARAKEET_DOWNLOAD_FAILED", &e.to_string()))?;

    Ok(path.to_string_lossy().to_string())
}

#[tauri::command]
pub async fn delete_parakeet_model(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<bool, CommandError> {
    let models_dir = state.config_dir.join("models");
    let parakeet_dir = models_dir.join("parakeet");

    state.stt.unload_parakeet();

    let mut deleted = false;
    for (filename, _) in crate::capture::models::PARAKEET_FILES {
        let path = parakeet_dir.join(filename);
        if path.is_file() {
            let _ = std::fs::remove_file(&path);
            deleted = true;
        }
        let part = parakeet_dir.join(format!("{filename}.part"));
        if part.is_file() {
            let _ = std::fs::remove_file(&part);
        }
    }

    let settings = {
        let mut guard = state.settings.lock_or_recover();
        if guard.stt.dictation_engine.as_deref() == Some("parakeet") {
            guard.stt.dictation_engine = Some("whisper".to_string());
        }
        guard.clone()
    };
    persist_settings(&app, &state, settings)?;

    Ok(deleted)
}

#[tauri::command]
pub async fn set_dictation_engine(
    app: AppHandle,
    state: State<'_, AppState>,
    engine: String,
) -> Result<(), CommandError> {
    let engine = engine.trim().to_lowercase();
    if engine == "parakeet" {
        let models_dir = state.config_dir.join("models");
        let parakeet_dir = models_dir.join("parakeet");
        if !crate::capture::parakeet::ModelFiles::is_installed_in(&parakeet_dir) {
            return Err(CommandError::new(
                "PARAKEET_NOT_INSTALLED",
                "NVIDIA Parakeet TDT model is not installed yet",
            ));
        }
    }

    let settings = {
        let mut guard = state.settings.lock_or_recover();
        guard.stt.dictation_engine = Some(engine);
        guard.clone()
    };
    persist_settings(&app, &state, settings)
}
