//! Web capture: the extension bridge, stored captures, AI-conversation imports and external links.

use super::*;

// ---------------------------------------------------------------------------
// Web capture
// ---------------------------------------------------------------------------

/// Progress for one capture, broadcast so any surface can show it. The stages
/// are `SAVING`, `SAVED`, `ANALYSING`, `ANALYSED`, `FAILED`.
pub const CAPTURE_PROGRESS_EVENT: &str = "capture-progress";

#[derive(Debug, Clone, Serialize)]
pub struct CaptureProgress {
    pub stage: String,
    pub capture_id: Option<String>,
    pub title: Option<String>,
    pub application: Option<String>,
    pub message: Option<String>,
}

/// What the pairing UI needs to show, and what the extension needs to connect.
#[derive(Debug, Clone, Serialize)]
pub struct CaptureBridgeStatus {
    pub enabled: bool,
    /// Whether a listener is actually bound right now. Differs from `enabled`
    /// when the port could not be bound at all.
    pub running: bool,
    /// The port actually in use, which may differ from the configured one if
    /// that port was taken.
    pub port: u16,
    pub configured_port: u16,
    /// The pairing secret, shown so the user can paste it into the extension.
    /// `None` until capture has been enabled for the first time.
    pub pairing_token: Option<String>,
    pub protocol_version: u32,
    pub analyze_on_capture: bool,
    pub capture_hotkey: String,
    pub last_error: Option<String>,
}

fn emit_capture_progress(app: &AppHandle, progress: CaptureProgress) {
    let _ = app.emit(CAPTURE_PROGRESS_EVENT, progress);
}

/// Stores one capture payload and, if configured, kicks off analysis.
///
/// Storage and interpretation are separated here on purpose: the artifact is
/// durable the moment `ingest` returns, and the analysis pass runs afterwards
/// on a background task whose failure is logged and never propagated.
fn accept_capture(app: &AppHandle, bytes: &[u8]) -> (u16, String) {
    let state = app.state::<AppState>();
    emit_capture_progress(
        app,
        CaptureProgress {
            stage: "SAVING".to_string(),
            capture_id: None,
            title: None,
            application: None,
            message: None,
        },
    );

    match crate::capture::web::ingest(&state.vault, bytes) {
        Ok(artifact) => {
            let application = artifact.capture.as_ref().map(|c| c.application.clone());
            emit_capture_progress(
                app,
                CaptureProgress {
                    stage: "SAVED".to_string(),
                    capture_id: Some(artifact.id.clone()),
                    title: Some(artifact.original_filename.clone()),
                    application: application.clone(),
                    message: None,
                },
            );

            let analyze = state.settings.lock_or_recover().capture.analyze_on_capture;
            if analyze {
                spawn_capture_analysis(app.clone(), artifact.id.clone());
            }

            let body = serde_json::json!({
                "ok": true,
                "id": artifact.id,
                "title": artifact.original_filename,
                "capture_type": artifact.capture.as_ref().map(|c| c.capture_type.clone()),
                "application": application,
                "notes": artifact.capture.as_ref().map(|c| c.notes.clone()).unwrap_or_default(),
            });
            (200, body.to_string())
        }
        Err(e) => {
            // The message is the user-facing one from `WebCaptureError`; the
            // payload itself is never logged, because it is page content.
            tracing::warn!("[Capture] Rejected a capture: {}", e);
            emit_capture_progress(
                app,
                CaptureProgress {
                    stage: "FAILED".to_string(),
                    capture_id: None,
                    title: None,
                    application: None,
                    message: Some(e.to_string()),
                },
            );
            let status = match e {
                crate::capture::web::WebCaptureError::PayloadTooLarge(_) => 413,
                crate::capture::web::WebCaptureError::EmptyCapture => 422,
                crate::capture::web::WebCaptureError::Vault(_) => 500,
                _ => 400,
            };
            (
                status,
                crate::capture::web::bridge::error_body("CAPTURE_REJECTED", &e.to_string()),
            )
        }
    }
}

/// Runs Relay's existing analysis contract over a stored capture.
///
/// Reuses `enrich_vault_file` rather than introducing a capture-specific
/// prompt: a capture is a Vault artifact, and it should be summarized and
/// tagged by exactly the same rules as an imported document.
fn spawn_capture_analysis(app: AppHandle, capture_id: String) {
    tauri::async_runtime::spawn(async move {
        let state = app.state::<AppState>();
        emit_capture_progress(
            &app,
            CaptureProgress {
                stage: "ANALYSING".to_string(),
                capture_id: Some(capture_id.clone()),
                title: None,
                application: None,
                message: None,
            },
        );

        let settings = state.settings.lock_or_recover().clone();
        let llm = LLMClient::new(settings.provider);
        match crate::pipeline::enrich_vault_file(&llm, &state.vault, &capture_id).await {
            Ok(_) => emit_capture_progress(
                &app,
                CaptureProgress {
                    stage: "ANALYSED".to_string(),
                    capture_id: Some(capture_id),
                    title: None,
                    application: None,
                    message: None,
                },
            ),
            Err(e) => {
                // A failed analysis is a missing summary, not a lost capture.
                tracing::warn!("[Capture] Analysis failed for {}: {}", capture_id, e);
                emit_capture_progress(
                    &app,
                    CaptureProgress {
                        stage: "ANALYSED".to_string(),
                        capture_id: Some(capture_id),
                        title: None,
                        application: None,
                        message: Some(
                            "Saved, but Relay could not analyse it. The capture is intact — try \
                             Analyse again from the capture."
                                .to_string(),
                        ),
                    },
                );
            }
        }
    });
}

/// Starts or stops the bridge so that it matches the current settings.
///
/// Called at startup and after every settings save, so "enabled" in Settings
/// and "a socket is open" can never drift apart.
pub fn apply_capture_bridge(app: &AppHandle, state: &AppState) {
    let (enabled, port, token) = {
        let settings = state.settings.lock_or_recover();
        (
            settings.capture.bridge_enabled,
            settings.capture.bridge_port,
            settings.capture.pairing_token.clone(),
        )
    };

    let mut bridge = state.capture_bridge.lock_or_recover();
    if let Some(existing) = bridge.take() {
        existing.stop();
    }

    if !enabled {
        return;
    }

    let Some(token) = token else {
        tracing::warn!("[Capture] Bridge enabled without a pairing token; not starting");
        return;
    };

    let handler_app = app.clone();
    match crate::capture::web::bridge::start(port, token, move |bytes| {
        accept_capture(&handler_app, bytes)
    }) {
        Ok(handle) => *bridge = Some(handle),
        Err(e) => tracing::error!("[Capture] Bridge failed to start: {}", e),
    }
}

fn bridge_status(state: &AppState, last_error: Option<String>) -> CaptureBridgeStatus {
    let settings = state.settings.lock_or_recover().clone();
    let bridge = state.capture_bridge.lock_or_recover();
    let running = bridge.as_ref().is_some_and(|b| b.is_running());
    CaptureBridgeStatus {
        enabled: settings.capture.bridge_enabled,
        running,
        port: bridge
            .as_ref()
            .map(|b| b.port)
            .unwrap_or(settings.capture.bridge_port),
        configured_port: settings.capture.bridge_port,
        pairing_token: settings.capture.pairing_token.clone(),
        protocol_version: crate::capture::web::PROTOCOL_VERSION,
        analyze_on_capture: settings.capture.analyze_on_capture,
        capture_hotkey: settings.hotkeys.capture_hotkey.clone(),
        last_error,
    }
}

#[tauri::command]
pub async fn get_capture_bridge_status(
    state: State<'_, AppState>,
) -> Result<CaptureBridgeStatus, CommandError> {
    Ok(bridge_status(&state, None))
}

/// Turns the capture bridge on or off, generating a pairing token the first
/// time it is switched on.
#[tauri::command]
pub async fn set_capture_bridge_enabled(
    app: AppHandle,
    enabled: bool,
    state: State<'_, AppState>,
) -> Result<CaptureBridgeStatus, CommandError> {
    {
        let mut settings = state.settings.lock_or_recover();
        settings.capture.bridge_enabled = enabled;
        if enabled && settings.capture.pairing_token.is_none() {
            settings.capture.pairing_token =
                Some(crate::capture::web::bridge::generate_token());
        }
        settings
            .save(&state.settings_path())
            .map_err(|e| CommandError::new("CONFIG_SAVE_FAILED", &e.to_string()))?;
    }

    apply_capture_bridge(&app, &state);
    let status = bridge_status(&state, None);
    if enabled && !status.running {
        return Err(CommandError::new(
            "CAPTURE_BRIDGE_START_FAILED",
            "Relay could not open a local port for capture. Another program may be using it — \
             try a different port in Capture settings.",
        ));
    }
    Ok(status)
}

/// Issues a new pairing token, which immediately invalidates every browser
/// that was paired with the old one.
#[tauri::command]
pub async fn regenerate_capture_pairing_token(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<CaptureBridgeStatus, CommandError> {
    {
        let mut settings = state.settings.lock_or_recover();
        settings.capture.pairing_token = Some(crate::capture::web::bridge::generate_token());
        settings
            .save(&state.settings_path())
            .map_err(|e| CommandError::new("CONFIG_SAVE_FAILED", &e.to_string()))?;
    }
    apply_capture_bridge(&app, &state);
    Ok(bridge_status(&state, None))
}

/// Sets the preferred loopback port and rebinds.
#[tauri::command]
pub async fn set_capture_bridge_port(
    app: AppHandle,
    port: u16,
    state: State<'_, AppState>,
) -> Result<CaptureBridgeStatus, CommandError> {
    if port < 1024 {
        return Err(CommandError::new(
            "INVALID_PORT",
            "Choose a port above 1023 — lower ports are reserved by the operating system.",
        ));
    }
    {
        let mut settings = state.settings.lock_or_recover();
        settings.capture.bridge_port = port;
        settings
            .save(&state.settings_path())
            .map_err(|e| CommandError::new("CONFIG_SAVE_FAILED", &e.to_string()))?;
    }
    apply_capture_bridge(&app, &state);
    Ok(bridge_status(&state, None))
}

/// Whether Relay analyses each capture as soon as it lands.
#[tauri::command]
pub async fn set_capture_analyze_on_capture(
    app: AppHandle,
    enabled: bool,
    state: State<'_, AppState>,
) -> Result<CaptureBridgeStatus, CommandError> {
    {
        let mut settings = state.settings.lock_or_recover();
        settings.capture.analyze_on_capture = enabled;
        settings
            .save(&state.settings_path())
            .map_err(|e| CommandError::new("CONFIG_SAVE_FAILED", &e.to_string()))?;
        let updated = settings.clone();
        drop(settings);
        let _ = app.emit("settings-changed", &updated.for_webview());
    }
    Ok(bridge_status(&state, None))
}

#[tauri::command]
pub async fn get_captures(state: State<'_, AppState>) -> Result<Vec<VaultFile>, CommandError> {
    state
        .vault
        .list_captures()
        .map_err(|e| CommandError::new("LIST_CAPTURES_FAILED", &e.to_string()))
}

#[tauri::command]
pub async fn get_capture(
    id: String,
    state: State<'_, AppState>,
) -> Result<VaultFile, CommandError> {
    let artifact = state
        .vault
        .get_vault_file(&id)
        .map_err(|e| CommandError::new("CAPTURE_NOT_FOUND", &e.to_string()))?;
    if !artifact.is_capture() {
        return Err(CommandError::new(
            "CAPTURE_NOT_FOUND",
            "That artifact is not a capture.",
        ));
    }
    Ok(artifact)
}

/// Returns the untouched structured payload behind a capture — what the page
/// actually said, before normalization made it readable.
#[tauri::command]
pub async fn get_capture_payload(
    id: String,
    state: State<'_, AppState>,
) -> Result<crate::capture::web::WebCapturePayload, CommandError> {
    state
        .vault
        .get_capture_payload(&id)
        .map_err(|e| CommandError::new("CAPTURE_PAYLOAD_UNAVAILABLE", &e.to_string()))
}

/// Rebuilds a capture's markdown from its stored payload.
#[tauri::command]
pub async fn renormalize_capture(
    id: String,
    state: State<'_, AppState>,
) -> Result<VaultFile, CommandError> {
    state
        .vault
        .renormalize_capture(&id)
        .map_err(|e| CommandError::new("RENORMALIZE_FAILED", &e.to_string()))
}

#[tauri::command]
pub async fn delete_capture(id: String, state: State<'_, AppState>) -> Result<(), CommandError> {
    state
        .vault
        .delete_vault_file(&id)
        .map_err(|e| CommandError::new("DELETE_FAILED", &e.to_string()))
}

/// Ingests a capture payload from inside the app rather than over the bridge.
///
/// Same validation, sanitization and storage as a bridged capture — this is
/// the seam a future non-browser capture source plugs into, and the path an
/// end-to-end test drives without opening a socket.
#[tauri::command]
pub async fn import_web_capture(
    app: AppHandle,
    payload_json: String,
    state: State<'_, AppState>,
) -> Result<VaultFile, CommandError> {
    if payload_json.len() > crate::capture::web::MAX_PAYLOAD_BYTES {
        return Err(CommandError::new(
            "PAYLOAD_TOO_LARGE",
            "That capture is larger than Vox's capture size limit.",
        ));
    }

    let artifact = crate::capture::web::ingest(&state.vault, payload_json.as_bytes())
        .map_err(|e| CommandError::new("CAPTURE_REJECTED", &e.to_string()))?;

    if state.settings.lock_or_recover().capture.analyze_on_capture {
        spawn_capture_analysis(app, artifact.id.clone());
    }
    Ok(artifact)
}

/// Retrieves the derived structured context for a capture, if already analyzed.
#[tauri::command]
pub async fn get_capture_context(
    id: String,
    state: State<'_, AppState>,
) -> Result<Option<crate::capture::web::SourceContext>, CommandError> {
    state
        .vault
        .get_capture_context(&id)
        .map_err(|e| CommandError::new("CONTEXT_UNAVAILABLE", &e.to_string()))
}

/// Analyzes a captured source to extract structured work context.
#[tauri::command]
pub async fn analyze_capture_context(
    id: String,
    state: State<'_, AppState>,
) -> Result<crate::capture::web::SourceContext, CommandError> {
    let file = state
        .vault
        .get_vault_file(&id)
        .map_err(|e| CommandError::new("CAPTURE_NOT_FOUND", &e.to_string()))?;

    let payload = state
        .vault
        .get_capture_payload(&id)
        .map_err(|e| CommandError::new("CAPTURE_PAYLOAD_UNAVAILABLE", &e.to_string()))?;

    let settings = state.settings.lock_or_recover().clone();
    let llm = LLMClient::new(settings.provider);

    // Which analysis runs is decided from the classification capture already
    // derived from the URL and stored on the artifact — not re-derived here by
    // testing the URL for a substring, which matched
    // `https://evil.example/?ref=github.com` and treated every GitHub issue and
    // pull request as a repository.
    let context = crate::capture::web::context::extract_source_context(Some(&llm), &file, &payload).await;

    state
        .vault
        .save_capture_context(&id, &context)
        .map_err(|e| CommandError::new("SAVE_CONTEXT_FAILED", &e.to_string()))?;

    // The action items this analysis found were, until now, written into
    // `context.json` and never seen again. Persist them as todos so the
    // TODOs surface shows work the user has already had extracted for
    // them rather than making them re-read the capture to find it.
    if let Some(conversation) = context.conversation() {
        let candidates: Vec<_> = conversation
            .action_items
            .iter()
            .map(|item| crate::vault::ExtractedTodo {
                title: item.description.clone(),
                kind: crate::vault::TodoSourceKind::WebCapture,
                source_ref: crate::vault::TodoSourceRef {
                    id: id.clone(),
                    // The extractor records which turns an item came from;
                    // the first is where to land the reader.
                    turn_ordinal: item.source_turn_ordinals.first().copied(),
                    label: Some(conversation.title.clone()),
                },
                // A web capture is not a scribble and carries no PARA band,
                // so these arrive uncategorised rather than filed somewhere
                // chosen on the user's behalf.
                para: None,
                captured_at: Some(context.generated_at.clone()),
            })
            .collect();

        // A failure to record todos must not lose the analysis that was
        // already written above — the context is the primary result here.
        match state.vault.record_extracted_todos(&candidates) {
            Ok(created) if !created.is_empty() => {
                tracing::info!(capture = %id, created = created.len(), "captured action items became todos");
            }
            Ok(_) => {}
            Err(e) => tracing::error!(capture = %id, "failed to record extracted todos: {}", e),
        }
    }

    Ok(context)
}

/// Turns a dialog result into a path string, or `None` when the user
/// cancelled.
pub fn picked_path(
    picked: Option<tauri_plugin_dialog::FilePath>,
) -> Result<Option<String>, CommandError> {
    match picked {
        Some(file_path) => file_path
            .into_path()
            .map(|p| Some(p.to_string_lossy().to_string()))
            .map_err(|e| CommandError::new("DIALOG_PATH_INVALID", &e.to_string())),
        None => Ok(None),
    }
}

/// Opens the native OS file picker for an AI conversation export archive (.zip or .json).
#[tauri::command]
pub async fn pick_ai_conversation_export_file(app: AppHandle) -> Result<Option<String>, CommandError> {
    let picked = tauri::async_runtime::spawn_blocking(move || {
        app.dialog()
            .file()
            .set_title("Select AI Conversation Export")
            .add_filter("AI Conversation Export (.zip, .json)", &["zip", "json"])
            .blocking_pick_file()
    })
    .await
    .map_err(|e| CommandError::new("DIALOG_TASK_FAILED", &e.to_string()))?;

    picked_path(picked)
}

/// Writes a dropped export to a private temp file.
///
/// The file name comes from the webview; it is reduced to one plain name so
/// a `..` or a separator cannot place the bytes anywhere but the staging
/// folder.
fn stage_dropped_export(filename: &str, bytes: &[u8]) -> Result<std::path::PathBuf, CommandError> {
    let temp_dir = std::env::temp_dir().join("vox_import_staging");
    std::fs::create_dir_all(&temp_dir)
        .map_err(|e| CommandError::new("TEMP_FILE_WRITE_FAILED", &e.to_string()))?;
    let name = crate::capture::web::importer::safe_asset_file_name(filename)
        .unwrap_or_else(|| "export".to_string());
    let temp_file = temp_dir.join(format!("{}_{}", uuid::Uuid::new_v4(), name));
    std::fs::write(&temp_file, bytes)
        .map_err(|e| CommandError::new("TEMP_FILE_WRITE_FAILED", &e.to_string()))?;
    Ok(temp_file)
}

/// Inspects an exported AI conversation archive (.zip or .json) from ChatGPT or Claude.
#[tauri::command]
pub async fn inspect_ai_conversation_export(
    path: String,
    state: State<'_, AppState>,
) -> Result<crate::capture::web::importer::ExportInspection, CommandError> {
    let p = std::path::PathBuf::from(&path);
    if !p.exists() {
        return Err(CommandError::new("FILE_NOT_FOUND", "Selected export file does not exist"));
    }
    crate::capture::web::importer::inspect_export_file(&p, &state.vault).await
}

/// Inspects exported AI conversation archive bytes staged directly from drag-and-drop.
#[tauri::command]
pub async fn inspect_ai_conversation_export_bytes(
    filename: String,
    bytes: Vec<u8>,
    state: State<'_, AppState>,
) -> Result<crate::capture::web::importer::ExportInspection, CommandError> {
    let temp_file = stage_dropped_export(&filename, &bytes)?;

    let res = crate::capture::web::importer::inspect_export_file(&temp_file, &state.vault).await;
    let _ = std::fs::remove_file(temp_file);
    res
}

/// Imports a chosen conversation from an AI export archive into the vault.
#[tauri::command]
pub async fn import_ai_conversation_export(
    path: String,
    conversation_id: String,
    duplicate_mode: Option<String>,
    state: State<'_, AppState>,
) -> Result<crate::vault::VaultFile, CommandError> {
    let p = std::path::PathBuf::from(&path);
    if !p.exists() {
        return Err(CommandError::new("FILE_NOT_FOUND", "Selected export file does not exist"));
    }
    let settings = state.settings.lock_or_recover().clone();
    crate::capture::web::importer::import_export_conversation(
        &p,
        &conversation_id,
        duplicate_mode.as_deref(),
        &state.vault,
        &settings,
    )
    .await
}

/// Imports a chosen conversation from staged export bytes into Relay's vault.
#[tauri::command]
pub async fn import_ai_conversation_export_bytes(
    filename: String,
    bytes: Vec<u8>,
    conversation_id: String,
    duplicate_mode: Option<String>,
    state: State<'_, AppState>,
) -> Result<crate::vault::VaultFile, CommandError> {
    let temp_file = stage_dropped_export(&filename, &bytes)?;

    let settings = state.settings.lock_or_recover().clone();
    let res = crate::capture::web::importer::import_export_conversation(
        &temp_file,
        &conversation_id,
        duplicate_mode.as_deref(),
        &state.vault,
        &settings,
    )
    .await;

    let _ = std::fs::remove_file(temp_file);
    res
}

/// Validates that a string is a safe HTTP or HTTPS URL before delegating to the OS.
pub fn validate_external_url(raw: &str) -> Result<String, &'static str> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err("URL cannot be empty.");
    }
    if !trimmed.starts_with("http://") && !trimmed.starts_with("https://") {
        return Err("Only HTTP and HTTPS URLs can be opened.");
    }
    if trimmed.chars().any(|c| c.is_control()) {
        return Err("URL contains invalid control characters.");
    }
    Ok(trimmed.to_string())
}

/// Opens a validated HTTP or HTTPS URL in the user's default OS browser.
#[tauri::command]
pub async fn open_external_url(url: String) -> Result<(), CommandError> {
    let validated = validate_external_url(&url)
        .map_err(|e| CommandError::new("INVALID_URL", e))?;

    #[cfg(target_os = "windows")]
    {
        std::process::Command::new("rundll32")
            .args(["url.dll,FileProtocolHandler", &validated])
            .spawn()
            .map_err(|e| CommandError::new("OPEN_URL_FAILED", &e.to_string()))?;
    }

    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open")
            .arg(&validated)
            .spawn()
            .map_err(|e| CommandError::new("OPEN_URL_FAILED", &e.to_string()))?;
    }

    #[cfg(target_os = "linux")]
    {
        std::process::Command::new("xdg-open")
            .arg(&validated)
            .spawn()
            .map_err(|e| CommandError::new("OPEN_URL_FAILED", &e.to_string()))?;
    }

    Ok(())
}

// ── Foundation Roadmap 11-20 Commands ────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validate_external_url_accepts_valid_http_and_https_urls() {
        assert_eq!(
            validate_external_url("https://github.com/stablyai/orca").unwrap(),
            "https://github.com/stablyai/orca"
        );
        assert_eq!(
            validate_external_url("  http://localhost:3000/path?query=1#hash  ").unwrap(),
            "http://localhost:3000/path?query=1#hash"
        );
    }

    #[test]
    fn validate_external_url_refuses_unsafe_schemes_and_control_characters() {
        assert!(validate_external_url("").is_err());
        assert!(validate_external_url("   ").is_err());
        assert!(validate_external_url("file:///C:/secrets.txt").is_err());
        assert!(validate_external_url("javascript:alert(1)").is_err());
        assert!(validate_external_url("data:text/html,hello").is_err());
        assert!(validate_external_url("https://example.com\n\revil").is_err());
    }
}
