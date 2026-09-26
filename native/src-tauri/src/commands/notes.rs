//! Notes: todos, voice notes, scribbles, and the knowledge surfaces built on them.

use super::*;

/// Creates a todo typed on the TODOs page.
///
/// An empty title is rejected rather than stored: a blank card is
/// indistinguishable from a bug, and the voice path depends on this
/// refusal so that a failed transcription cannot create an empty todo.
#[tauri::command]
pub async fn create_manual_todo(
    title: String,
    state: State<'_, AppState>,
) -> Result<KanbanCard, CommandError> {
    if title.trim().is_empty() {
        return Err(CommandError::new(
            "INVALID_INPUT",
            "A todo needs a title",
        ));
    }

    let card = KanbanCard::new_manual(&title);
    state
        .vault
        .save_kanban_card(&card)
        .map_err(|e| CommandError::new("VAULT_WRITE_FAILED", &e.to_string()))?;
    Ok(card)
}

#[tauri::command]
pub async fn set_todo_status(
    id: String,
    status: String,
    state: State<'_, AppState>,
) -> Result<KanbanCard, CommandError> {
    state
        .vault
        .set_kanban_status(&id, &status)
        .map_err(|e| CommandError::new("VAULT_UPDATE_FAILED", &e.to_string()))
}

#[tauri::command]
pub async fn delete_todo(id: String, state: State<'_, AppState>) -> Result<(), CommandError> {
    state
        .vault
        .delete_kanban_card(&id)
        .map_err(|e| CommandError::new("VAULT_DELETE_FAILED", &e.to_string()))
}

#[tauri::command]
pub async fn get_kanban_cards(state: State<'_, AppState>) -> Result<Vec<KanbanCard>, CommandError> {
    state
        .vault
        .list_kanban_cards()
        .map_err(|e| CommandError::new("VAULT_READ_FAILED", &e.to_string()))
}

/// All Voice Notes in the vault, newest first — the Transcript History the
/// Voice Note page renders and computes its stats from.
#[tauri::command]
pub async fn get_voice_notes(state: State<'_, AppState>) -> Result<Vec<VaultNote>, CommandError> {
    state
        .vault
        .list_notes_by_type(crate::vault::VOICE_NOTE_TYPE)
        .map_err(|e| CommandError::new("VAULT_READ_FAILED", &e.to_string()))
}

#[tauri::command]
pub async fn update_voice_note(
    id: String,
    content: String,
    state: State<'_, AppState>,
) -> Result<VaultNote, CommandError> {
    state
        .vault
        .update_note_content(&id, &content)
        .map_err(|e| CommandError::new("VAULT_UPDATE_FAILED", &e.to_string()))
}

/// A correction that was applied, and what it takes to undo it.
#[derive(Debug, Clone, Serialize)]
pub struct PhraseCorrectionResult {
    pub note: VaultNote,
    /// The correction as it was written to the note's history. Undo is
    /// `undo_voice_note_correction`, which reverses this record — which is why
    /// no versioning system is needed for it.
    pub record: crate::vault::CorrectionRecord,
    /// True when the phrase was also added to the learned vocabulary.
    pub learned: bool,
}

/// Corrects one selected phrase inside a Voice Note.
///
/// A deterministic range edit, saved through the same `update_note_content`
/// the full editor uses — so this is a second way into the existing
/// persistence, not a second persistence. No model is called: the user has
/// already said what the text should be.
///
/// `start` and `end` are character offsets into the note's current content,
/// and `original` is what the caller believes is there. The edit refuses
/// rather than applying stale offsets to changed content.
///
/// `learn` is opt-in per correction. Most corrections are ordinary edits —
/// "Thursday" to "Tuesday" is not vocabulary — so nothing is added to the
/// learned list unless the user ticks the box.
// A Tauri command's parameters are the IPC payload's fields — they are flat by
// construction, and grouping them would change the frontend-facing contract.
#[allow(clippy::too_many_arguments)]
#[tauri::command]
pub async fn correct_voice_note_phrase(
    app: AppHandle,
    state: State<'_, AppState>,
    id: String,
    start: usize,
    end: usize,
    original: String,
    replacement: String,
    learn: Option<bool>,
) -> Result<PhraseCorrectionResult, CommandError> {
    if replacement.trim().is_empty() {
        return Err(CommandError::new(
            "CORRECTION_EMPTY",
            "A correction needs a replacement.",
        ));
    }

    let note = state
        .vault
        .get_note(&id)
        .map_err(|e| CommandError::new("VAULT_READ_FAILED", &e.to_string()))?;

    let corrected =
        crate::vault::correction::replace_range(&note.content, start, end, &original, &replacement)
            .map_err(|e| CommandError::new("CORRECTION_FAILED", &e.to_string()))?;

    let note = state
        .vault
        .update_note_content(&id, &corrected)
        .map_err(|e| CommandError::new("VAULT_UPDATE_FAILED", &e.to_string()))?;

    // Learning first, so the record can say truthfully whether it happened.
    let learned = if learn.unwrap_or(false) {
        let mut settings = state.settings.lock_or_recover();
        let added = settings.learn_correction(&original, &replacement);
        if added {
            let _ = settings.save(&state.settings_path());
            let updated = settings.clone();
            drop(settings);
            // The Settings window is a separate window holding its own copy,
            // and `save_settings` writes that copy whole. Without this it
            // would overwrite the rule the moment anything else there is
            // changed.
            let _ = app.emit("settings-changed", &updated.for_webview());
        }
        added
    } else {
        false
    };

    let record = crate::vault::CorrectionRecord::new(&id, &original, &replacement, start, learned);
    // A history that cannot be written costs the undo, not the correction the
    // user already sees applied — so it is logged rather than surfaced.
    if let Err(e) = state.vault.record_correction(&record) {
        tracing::warn!("Could not record correction history for {id}: {e}");
    }

    Ok(PhraseCorrectionResult {
        note,
        record,
        learned,
    })
}

/// Reverses the most recent phrase correction on a Voice Note.
///
/// Puts the original phrase back where the replacement now sits, rather than
/// restoring a snapshot of the whole note: a snapshot undo applied after the
/// full editor touched the note would throw that edit away. If the replacement
/// is no longer there, this refuses.
#[tauri::command]
pub async fn undo_voice_note_correction(
    state: State<'_, AppState>,
    id: String,
) -> Result<VaultNote, CommandError> {
    let record = state
        .vault
        .correction_history(&id)
        .map_err(|e| CommandError::new("VAULT_READ_FAILED", &e.to_string()))?
        .pop()
        .ok_or_else(|| {
            CommandError::new("NO_CORRECTION_TO_UNDO", "This note has no correction to undo.")
        })?;

    let note = state
        .vault
        .get_note(&id)
        .map_err(|e| CommandError::new("VAULT_READ_FAILED", &e.to_string()))?;

    let restored = record
        .reverse(&note.content)
        .map_err(|e| CommandError::new("UNDO_FAILED", &e.to_string()))?;

    let note = state
        .vault
        .update_note_content(&id, &restored)
        .map_err(|e| CommandError::new("VAULT_UPDATE_FAILED", &e.to_string()))?;

    // Only once the note is safely written: a correction that is still applied
    // must stay in the history, or the next undo reverses the wrong edit.
    let _ = state.vault.pop_correction(&id);

    // A learned rule is deliberately left in place. It is a standing statement
    // about the phrase, made on purpose through a separate checkbox, and
    // Settings › Dictionary is where it is turned off or removed.
    Ok(note)
}

/// Adds a phrase to the user's dictionary, from wherever they found it.
///
/// The same list Settings › Dictionary edits, reached from a Voice Note: a
/// spelling that is already right and that Relay should keep getting right,
/// used to prime the recognizer before it guesses. Distinct from a learned
/// correction, which repairs a guess already made.
#[tauri::command]
pub async fn add_dictionary_word(
    app: AppHandle,
    state: State<'_, AppState>,
    word: String,
) -> Result<Vec<String>, CommandError> {
    if word.trim().is_empty() {
        return Err(CommandError::new(
            "DICTIONARY_WORD_EMPTY",
            "A dictionary entry needs a word.",
        ));
    }

    let mut settings = state.settings.lock_or_recover();
    if !settings.add_dictionary_word(&word) {
        // Already known. Saying so is the honest answer, and rewriting the
        // file to change nothing is not.
        return Ok(settings.dictionary.clone());
    }
    settings
        .save(&state.settings_path())
        .map_err(|e| CommandError::new("CONFIG_SAVE_FAILED", &e.to_string()))?;

    let updated = settings.clone();
    drop(settings);
    // Same reason as above: the Settings window holds its own copy.
    let _ = app.emit("settings-changed", &updated.for_webview());
    Ok(updated.dictionary)
}

#[tauri::command]
pub async fn delete_voice_note(
    id: String,
    state: State<'_, AppState>,
) -> Result<TrashItem, CommandError> {
    state
        .vault
        .move_to_trash("voice_note", &id)
        .map_err(|e| CommandError::new("VAULT_DELETE_FAILED", &e.to_string()))
}

#[tauri::command]
pub async fn delete_voice_notes(
    ids: Vec<String>,
    state: State<'_, AppState>,
) -> Result<usize, CommandError> {
    let mut count = 0;
    for id in ids {
        if state.vault.move_to_trash("voice_note", &id).is_ok() {
            count += 1;
        }
    }
    Ok(count)
}


#[tauri::command]
pub async fn merge_voice_notes(
    app: AppHandle,
    primary_id: String,
    secondary_id: String,
    state: State<'_, AppState>,
) -> Result<VaultNote, CommandError> {
    let merged = state
        .vault
        .merge_notes(&primary_id, &secondary_id)
        .map_err(|e| CommandError::new("VAULT_MERGE_FAILED", &e.to_string()))?;

    // Synchronize and re-enrich any existing Scribbles derived from the merged Voice Notes
    if let Ok(affected_scribble_ids) = state.vault.sync_scribbles_for_voice_note_merge(&primary_id, &secondary_id) {
        for scribble_id in affected_scribble_ids {
            if let Ok(scribble) = state.vault.get_scribble(&scribble_id) {
                let _ = app.emit(SCRIBBLE_SAVED_EVENT, &scribble);
                spawn_scribble_enrichment(app.clone(), &state, scribble_id);
            }
        }
    }

    Ok(merged)
}

#[tauri::command]
pub async fn merge_multiple_voice_notes(
    app: AppHandle,
    ids: Vec<String>,
    state: State<'_, AppState>,
) -> Result<VaultNote, CommandError> {
    let merged = state
        .vault
        .merge_multiple_notes(&ids)
        .map_err(|e| CommandError::new("VAULT_MERGE_FAILED", &e.to_string()))?;

    // Synchronize and re-enrich any existing Scribbles derived from the merged Voice Notes
    for id in &ids {
        if let Ok(affected_scribble_ids) = state.vault.sync_scribbles_for_voice_note_merge(&merged.id, id) {
            for scribble_id in affected_scribble_ids {
                if let Ok(scribble) = state.vault.get_scribble(&scribble_id) {
                    let _ = app.emit(SCRIBBLE_SAVED_EVENT, &scribble);
                    spawn_scribble_enrichment(app.clone(), &state, scribble_id);
                }
            }
        }
    }

    Ok(merged)
}

#[derive(Serialize, Deserialize)]
pub struct UnmergeVoiceNotesResponse {
    pub primary: VaultNote,
    pub secondary: VaultNote,
}

#[tauri::command]
pub async fn unmerge_voice_note(
    id: String,
    state: State<'_, AppState>,
) -> Result<UnmergeVoiceNotesResponse, CommandError> {
    let result = state
        .vault
        .unmerge_notes(&id)
        .map_err(|e| CommandError::new("VAULT_UNMERGE_FAILED", &e.to_string()))?;

    Ok(UnmergeVoiceNotesResponse {
        primary: result.primary,
        secondary: result.secondary,
    })
}

pub const SCRIBBLE_SAVED_EVENT: &str = "scribble-saved";
pub const SCRIBBLE_ENRICHED_EVENT: &str = "scribble-enriched";

pub fn spawn_scribble_enrichment(
    app: AppHandle,
    state: &AppState,
    scribble_id: String,
) {
    let settings = state.settings.lock_or_recover().clone();
    let llm = LLMClient::new(settings.provider);
    let vault_dir = state.vault.vault_dir();

    tauri::async_runtime::spawn(async move {
        let vault = VaultManager::new(vault_dir);
        match crate::pipeline::enrich_scribble(&llm, &vault, &scribble_id).await {
            Ok(enriched) => {
                let _ = app.emit(SCRIBBLE_ENRICHED_EVENT, &enriched);
            }
            Err(e) => {
                tracing::warn!("Async scribble enrichment failed for {}: {}", scribble_id, e);
            }
        }
    });
}

#[tauri::command]
pub async fn get_scribbles(state: State<'_, AppState>) -> Result<Vec<Scribble>, CommandError> {
    state
        .vault
        .list_scribbles()
        .map_err(|e| CommandError::new("VAULT_READ_FAILED", &e.to_string()))
}

#[tauri::command]
pub async fn get_scribble(id: String, state: State<'_, AppState>) -> Result<Scribble, CommandError> {
    state
        .vault
        .get_scribble(&id)
        .map_err(|e| CommandError::new("VAULT_READ_FAILED", &e.to_string()))
}

// A Tauri command's parameters are the IPC payload's fields — they are flat by
// construction, and grouping them would change the frontend-facing contract.
#[allow(clippy::too_many_arguments)]
#[tauri::command]
pub async fn create_scribble(
    app: AppHandle,
    content: String,
    title: Option<String>,
    source_type: Option<String>,
    source_metadata: Option<serde_json::Value>,
    tags: Option<Vec<String>>,
    topics: Option<Vec<String>>,
    state: State<'_, AppState>,
) -> Result<Scribble, CommandError> {
    let mut scribble = Scribble::new_text(&content, title.as_deref());
    if let Some(st) = source_type {
        scribble.source_type = st;
    }
    if let Some(sm) = source_metadata {
        scribble.source_metadata = sm;
    }
    if let Some(tg) = tags {
        scribble.tags = tg;
    }
    if let Some(tp) = topics {
        scribble.topics = tp;
    }

    state
        .vault
        .save_scribble(&scribble)
        .map_err(|e| CommandError::new("VAULT_SAVE_FAILED", &e.to_string()))?;

    let _ = app.emit(SCRIBBLE_SAVED_EVENT, &scribble);
    spawn_scribble_enrichment(app, &state, scribble.id.clone());

    Ok(scribble)
}

#[tauri::command]
pub async fn promote_voice_note_to_scribble(
    app: AppHandle,
    voice_note_id: String,
    custom_title: Option<String>,
    state: State<'_, AppState>,
) -> Result<Scribble, CommandError> {
    let voice_note = state
        .vault
        .get_note(&voice_note_id)
        .map_err(|e| CommandError::new("NOTE_NOT_FOUND", &e.to_string()))?;

    let scribble = Scribble::from_voice_note(&voice_note.id, &voice_note.content, custom_title.as_deref());
    state
        .vault
        .save_scribble(&scribble)
        .map_err(|e| CommandError::new("VAULT_SAVE_FAILED", &e.to_string()))?;

    let _ = app.emit(SCRIBBLE_SAVED_EVENT, &scribble);
    spawn_scribble_enrichment(app, &state, scribble.id.clone());

    Ok(scribble)
}

#[tauri::command]
pub async fn create_file_scribble(
    app: AppHandle,
    filename: String,
    content: String,
    mime_type: Option<String>,
    size_bytes: Option<u64>,
    state: State<'_, AppState>,
) -> Result<Scribble, CommandError> {
    let scribble = Scribble::from_file(&filename, &content, mime_type.as_deref(), size_bytes);
    state
        .vault
        .save_scribble(&scribble)
        .map_err(|e| CommandError::new("VAULT_SAVE_FAILED", &e.to_string()))?;

    let _ = app.emit(SCRIBBLE_SAVED_EVENT, &scribble);
    spawn_scribble_enrichment(app, &state, scribble.id.clone());

    Ok(scribble)
}

#[tauri::command]
pub async fn update_scribble(
    app: AppHandle,
    scribble: Scribble,
    state: State<'_, AppState>,
) -> Result<Scribble, CommandError> {
    let updated = state
        .vault
        .update_scribble(&scribble)
        .map_err(|e| CommandError::new("VAULT_UPDATE_FAILED", &e.to_string()))?;

    let _ = app.emit(SCRIBBLE_SAVED_EVENT, &updated);
    Ok(updated)
}

/// Files a scribble under a PARA band, or clears it when `para` is absent.
///
/// An unrecognised band is rejected rather than silently dropped: a caller
/// asking for a band that does not exist has a bug, and quietly filing the
/// thought as uncategorised would hide it.
#[tauri::command]
pub async fn set_scribble_para(
    app: AppHandle,
    id: String,
    para: Option<String>,
    state: State<'_, AppState>,
) -> Result<Scribble, CommandError> {
    let band = match para.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        None => None,
        Some(raw) => Some(crate::vault::ParaBand::from_str_opt(raw).ok_or_else(|| {
            CommandError::new("INVALID_INPUT", &format!("Unknown PARA band: {}", raw))
        })?),
    };

    let updated = state
        .vault
        .set_scribble_para(&id, band)
        .map_err(|e| CommandError::new("VAULT_UPDATE_FAILED", &e.to_string()))?;

    let _ = app.emit(SCRIBBLE_SAVED_EVENT, &updated);
    Ok(updated)
}

#[tauri::command]
pub async fn delete_scribble(
    id: String,
    state: State<'_, AppState>,
) -> Result<TrashItem, CommandError> {
    state
        .vault
        .move_to_trash("scribble", &id)
        .map_err(|e| CommandError::new("VAULT_DELETE_FAILED", &e.to_string()))
}

#[tauri::command]
pub async fn merge_scribbles(
    app: AppHandle,
    source_ids: Vec<String>,
    state: State<'_, AppState>,
) -> Result<Scribble, CommandError> {
    let merged = state
        .vault
        .merge_scribbles(&source_ids)
        .map_err(|e| CommandError::new("VAULT_MERGE_FAILED", &e.to_string()))?;

    let _ = app.emit(SCRIBBLE_SAVED_EVENT, &merged);
    spawn_scribble_enrichment(app, &state, merged.id.clone());

    Ok(merged)
}

#[tauri::command]
pub async fn add_scribble_relationship(
    app: AppHandle,
    source_id: String,
    relationship: ScribbleRelationship,
    state: State<'_, AppState>,
) -> Result<Scribble, CommandError> {
    let updated = state
        .vault
        .add_scribble_relationship(&source_id, relationship)
        .map_err(|e| CommandError::new("VAULT_UPDATE_FAILED", &e.to_string()))?;

    let _ = app.emit(SCRIBBLE_SAVED_EVENT, &updated);
    Ok(updated)
}

#[tauri::command]
pub async fn remove_scribble_relationship(
    app: AppHandle,
    source_id: String,
    relationship_id: String,
    state: State<'_, AppState>,
) -> Result<Scribble, CommandError> {
    let updated = state
        .vault
        .remove_scribble_relationship(&source_id, &relationship_id)
        .map_err(|e| CommandError::new("VAULT_UPDATE_FAILED", &e.to_string()))?;

    let _ = app.emit(SCRIBBLE_SAVED_EVENT, &updated);
    Ok(updated)
}

#[tauri::command]
pub async fn search_knowledge(
    query: String,
    state: State<'_, AppState>,
) -> Result<KnowledgeSearchResult, CommandError> {
    state
        .vault
        .search_knowledge(&query)
        .map_err(|e| CommandError::new("VAULT_SEARCH_FAILED", &e.to_string()))
}

#[tauri::command]
pub async fn get_knowledge_graph(
    filter: Option<GraphFilter>,
    state: State<'_, AppState>,
) -> Result<KnowledgeGraphData, CommandError> {
    state
        .vault
        .get_knowledge_graph(filter.as_ref())
        .map_err(|e| CommandError::new("GRAPH_BUILD_FAILED", &e.to_string()))
}

#[tauri::command]
pub async fn trigger_enrich_scribble(
    app: AppHandle,
    id: String,
    state: State<'_, AppState>,
) -> Result<Scribble, CommandError> {
    let settings = state.settings.lock_or_recover().clone();
    let llm = LLMClient::new(settings.provider);

    let enriched = crate::pipeline::enrich_scribble(&llm, &state.vault, &id)
        .await
        .map_err(|e| CommandError::new("ENRICH_FAILED", &e))?;

    let _ = app.emit(SCRIBBLE_ENRICHED_EVENT, &enriched);
    let _ = app.emit(SCRIBBLE_SAVED_EVENT, &enriched);
    Ok(enriched)
}

#[tauri::command]
pub async fn summarize_scribble(
    app: AppHandle,
    id: String,
    state: State<'_, AppState>,
) -> Result<Scribble, CommandError> {
    let settings = state.settings.lock_or_recover().clone();
    let llm = LLMClient::new(settings.provider);

    let updated = crate::pipeline::summarize_scribble(&llm, &state.vault, &id)
        .await
        .map_err(|e| CommandError::new("SUMMARIZE_FAILED", &e))?;

    let _ = app.emit(SCRIBBLE_SAVED_EVENT, &updated);
    Ok(updated)
}
