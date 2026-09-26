//! Vault: imported files, the trash, and where the vault lives.

use super::*;

#[tauri::command]
pub async fn import_vault_file(
    source_path: String,
    state: State<'_, AppState>,
) -> Result<VaultFile, CommandError> {
    let path = std::path::Path::new(&source_path);
    state
        .vault
        .import_vault_file(path)
        .map_err(|e| CommandError::new("IMPORT_FAILED", &e.to_string()))
}

#[tauri::command]
pub async fn import_vault_file_bytes(
    filename: String,
    bytes: Vec<u8>,
    source_path: Option<String>,
    state: State<'_, AppState>,
) -> Result<VaultFile, CommandError> {
    state
        .vault
        .import_vault_file_bytes(&filename, &bytes, source_path.as_deref())
        .map_err(|e| CommandError::new("IMPORT_FAILED", &e.to_string()))
}

#[tauri::command]
pub async fn get_vault_files(
    state: State<'_, AppState>,
) -> Result<Vec<VaultFile>, CommandError> {
    state
        .vault
        .list_vault_files()
        .map_err(|e| CommandError::new("LIST_FILES_FAILED", &e.to_string()))
}

#[tauri::command]
pub async fn get_vault_file(
    id: String,
    state: State<'_, AppState>,
) -> Result<VaultFile, CommandError> {
    state
        .vault
        .get_vault_file(&id)
        .map_err(|e| CommandError::new("FILE_NOT_FOUND", &e.to_string()))
}

#[tauri::command]
pub async fn analyze_vault_file(
    id: String,
    state: State<'_, AppState>,
) -> Result<VaultFile, CommandError> {
    let _ = state.vault.reprocess_vault_file(&id);
    let settings = state.settings.lock_or_recover().clone();
    let llm = LLMClient::new(settings.provider);
    crate::pipeline::enrich_vault_file(&llm, &state.vault, &id)
        .await
        .map_err(|e| CommandError::new("ANALYZE_FAILED", &e))
}

#[tauri::command]
pub async fn summarize_vault_file(
    id: String,
    state: State<'_, AppState>,
) -> Result<VaultFile, CommandError> {
    let _ = state.vault.reprocess_vault_file(&id);
    let settings = state.settings.lock_or_recover().clone();
    let llm = LLMClient::new(settings.provider);
    crate::pipeline::summarize_vault_file(&llm, &state.vault, &id)
        .await
        .map_err(|e| CommandError::new("SUMMARIZE_FAILED", &e))
}

#[tauri::command]
pub async fn enrich_vault_file(
    id: String,
    state: State<'_, AppState>,
) -> Result<VaultFile, CommandError> {
    analyze_vault_file(id, state).await
}

#[tauri::command]
pub async fn update_vault_file_tags(
    id: String,
    tags: Vec<String>,
    topics: Vec<String>,
    entities: Vec<String>,
    state: State<'_, AppState>,
) -> Result<VaultFile, CommandError> {
    let mut file = state
        .vault
        .get_vault_file(&id)
        .map_err(|e| CommandError::new("FILE_NOT_FOUND", &e.to_string()))?;

    file.tags = tags;
    file.topics = topics;
    file.entities = entities;
    file.updated_at = chrono::Utc::now().to_rfc3339();

    state
        .vault
        .save_vault_file(&file)
        .map_err(|e| CommandError::new("SAVE_FAILED", &e.to_string()))?;

    Ok(file)
}

#[tauri::command]
pub async fn create_scribble_from_vault_file(
    app: AppHandle,
    id: String,
    state: State<'_, AppState>,
) -> Result<Scribble, CommandError> {
    let scribble = state
        .vault
        .create_scribble_from_file(&id)
        .map_err(|e| CommandError::new("CREATE_SCRIBBLE_FAILED", &e.to_string()))?;

    let _ = app.emit(SCRIBBLE_SAVED_EVENT, &scribble);
    spawn_scribble_enrichment(app, &state, scribble.id.clone());

    Ok(scribble)
}

#[tauri::command]
pub async fn reprocess_vault_file(
    id: String,
    state: State<'_, AppState>,
) -> Result<VaultFile, CommandError> {
    analyze_vault_file(id, state).await
}

#[tauri::command]
pub async fn delete_vault_file(
    id: String,
    state: State<'_, AppState>,
) -> Result<(), CommandError> {
    state
        .vault
        .delete_vault_file(&id)
        .map_err(|e| CommandError::new("DELETE_FAILED", &e.to_string()))
}

#[tauri::command]
pub async fn open_vault_file_location(
    id: String,
    state: State<'_, AppState>,
) -> Result<(), CommandError> {
    let file = state
        .vault
        .get_vault_file(&id)
        .map_err(|e| CommandError::new("FILE_NOT_FOUND", &e.to_string()))?;

    let full_path = state.vault.vault_dir().join(&file.vault_path);
    let target_dir = if full_path.exists() {
        full_path.parent().unwrap_or(&full_path).to_path_buf()
    } else {
        state.vault.vault_dir().join("files").join(&id).join("original")
    };

    if target_dir.exists() {
        #[cfg(target_os = "windows")]
        {
            let path_buf = std::fs::canonicalize(&target_dir).unwrap_or(target_dir);
            let path_str = path_buf.to_string_lossy().replace('/', "\\");
            let clean_path = path_str.trim_start_matches(r"\\?\");
            let _ = std::process::Command::new("explorer")
                .arg(clean_path)
                .spawn();
        }
    }
    Ok(())
}

#[tauri::command]
pub async fn get_trash_items(state: State<'_, AppState>) -> Result<Vec<TrashItem>, CommandError> {
    state
        .vault
        .get_trash_items()
        .map_err(|e| CommandError::new("TRASH_READ_FAILED", &e.to_string()))
}

#[tauri::command]
pub async fn restore_trash_item(
    trash_id: String,
    state: State<'_, AppState>,
) -> Result<(), CommandError> {
    state
        .vault
        .restore_trash_item(&trash_id)
        .map_err(|e| CommandError::new("TRASH_RESTORE_FAILED", &e.to_string()))
}

#[tauri::command]
pub async fn delete_trash_item_permanently(
    trash_id: String,
    state: State<'_, AppState>,
) -> Result<(), CommandError> {
    state
        .vault
        .delete_trash_item_permanently(&trash_id)
        .map_err(|e| CommandError::new("TRASH_DELETE_FAILED", &e.to_string()))
}

#[tauri::command]
pub async fn empty_trash(state: State<'_, AppState>) -> Result<usize, CommandError> {
    state
        .vault
        .empty_trash()
        .map_err(|e| CommandError::new("TRASH_EMPTY_FAILED", &e.to_string()))
}

#[derive(Debug, Clone, Serialize)]
pub struct VaultLocationInfo {
    /// Absolute path currently in use, whether from an explicit user choice
    /// or the process-relative default.
    pub path: String,
    /// The process-relative default path — what "Use Default Vox Vault"
    /// would set `path` to.
    pub default_path: String,
    /// Whether the user has explicitly chosen/confirmed a location (Voice
    /// Note first-time setup, or Settings) — distinct from "currently using
    /// the unconfirmed default".
    pub configured: bool,
    /// Whether `path` currently exists (or can be created) and is usable.
    pub accessible: bool,
}

/// Reports where Relay's vault currently lives, so the Voice Note page can
/// decide whether to show first-time setup, a "can't access your folder"
/// recovery state, or the normal history view.
#[tauri::command]
pub async fn get_vault_location(
    state: State<'_, AppState>,
) -> Result<VaultLocationInfo, CommandError> {
    let configured = state.settings.lock_or_recover().vault.directory.is_some();
    let path = state.vault.vault_dir();
    // Reuses `VaultManager::init` (already called by every read/write path)
    // as the accessibility probe, rather than duplicating filesystem-
    // permission-checking logic.
    let accessible = state.vault.init().is_ok();
    Ok(VaultLocationInfo {
        path: path.to_string_lossy().to_string(),
        default_path: state.default_vault_dir.to_string_lossy().to_string(),
        configured,
        accessible,
    })
}

/// Opens the native OS folder picker and returns the chosen path, or `None`
/// if the user cancelled. Runs on a blocking task since the dialog blocks
/// its calling thread until the user responds.
#[tauri::command]
pub async fn choose_vault_folder(app: AppHandle) -> Result<Option<String>, CommandError> {
    let selected =
        tauri::async_runtime::spawn_blocking(move || app.dialog().file().blocking_pick_folder())
            .await
            .map_err(|e| CommandError::new("DIALOG_TASK_FAILED", &e.to_string()))?;

    match selected {
        Some(file_path) => file_path
            .into_path()
            .map(|p| Some(p.to_string_lossy().to_string()))
            .map_err(|e| CommandError::new("DIALOG_PATH_INVALID", &e.to_string())),
        None => Ok(None),
    }
}

/// Validates `path`, repoints the live vault at it (no restart needed —
/// future Voice Notes, and any other vault reads/writes, immediately use
/// it), and persists it to the existing Vault Directory Location setting.
/// Never moves, migrates, or deletes whatever is at the old location.
#[tauri::command]
pub async fn set_vault_location(
    app: AppHandle,
    path: String,
    state: State<'_, AppState>,
) -> Result<VaultLocationInfo, CommandError> {
    state.ensure_vault_can_move()?;
    let new_dir = PathBuf::from(&path);
    let probe = VaultManager::new(new_dir.clone());
    probe
        .init()
        .map_err(|e| CommandError::new("VAULT_PATH_INVALID", &e.to_string()))?;

    // Persist before repointing the live vault — if the settings write
    // fails, the running app must keep using the old (still-working)
    // location rather than silently diverging from what's on disk.
    let mut settings = state.settings.lock_or_recover();
    settings.vault.directory = Some(path.clone());
    settings
        .save(&state.settings_path())
        .map_err(|e| CommandError::new("CONFIG_SAVE_FAILED", &e.to_string()))?;
    let updated_settings = settings.clone();
    drop(settings);

    state.repoint_vault(&app, &new_dir);

    let _ = app.emit("settings-changed", &updated_settings.for_webview());
    let _ = app.emit("vault-changed", &path);

    Ok(VaultLocationInfo {
        path,
        default_path: state.default_vault_dir.to_string_lossy().to_string(),
        configured: true,
        accessible: true,
    })
}
