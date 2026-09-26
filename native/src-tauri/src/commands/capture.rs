//! Capture: the recorder, the dictation pill window, hotkeys, and the pill's record-and-process path.

use super::*;

#[tauri::command]
pub async fn get_last_stt_diagnostics(
    state: State<'_, AppState>,
) -> Result<Option<crate::capture::SttDiagnosticSnapshot>, CommandError> {
    let guard = state.last_stt_diagnostics.lock_or_recover();
    Ok(guard.clone())
}

/// What the retained decode history says.
///
/// Deliberately a summary rather than the rows: the decision this exists to
/// support is "are levels low" or "does pinning starve decodes", and a caller
/// that has to compute percentiles itself will compute them differently from
/// the next caller.
#[tauri::command]
pub async fn get_stt_decode_summary(
    state: State<'_, AppState>,
) -> Result<crate::capture::decode_history::DecodeSummary, CommandError> {
    let records = crate::capture::decode_history::load(&state.config_dir);
    Ok(crate::capture::decode_history::summarize(&records))
}

#[tauri::command]
pub async fn get_capture_status(state: State<'_, AppState>) -> Result<CaptureStatus, CommandError> {
    let active = state.recorder.is_active();
    let mode = state.recorder.active_mode();
    let status = if active { "LISTENING" } else { "IDLE" };
    Ok(CaptureStatus {
        active,
        mode,
        status: status.to_string(),
        message: None,
    })
}

#[tauri::command]
pub async fn update_hotkeys(
    app: AppHandle,
    hotkeys: HotkeySettings,
    state: State<'_, AppState>,
) -> Result<(), CommandError> {
    hotkeys::apply_hotkeys(
        &app,
        &hotkeys.show_hide_hotkey,
        &hotkeys.dictation_hotkey,
        &hotkeys.capture_hotkey,
    )
    .map_err(|e| CommandError::new("HOTKEY_REGISTER_FAILED", &e))?;

    let mut settings = state.settings.lock_or_recover();
    settings.hotkeys = hotkeys;
    settings
        .save(&state.settings_path())
        .map_err(|e| CommandError::new("CONFIG_SAVE_FAILED", &e.to_string()))?;
    let updated = settings.clone();
    drop(settings);

    let _ = app.emit("settings-changed", &updated.for_webview());
    Ok(())
}

/// Where the floating pill anchors on screen. Re-anchors immediately using
/// a freshly computed monitor/work-area, at whatever size (resting or
/// expanded) it currently is.
#[tauri::command]
pub async fn set_pill_position(
    app: AppHandle,
    position: PillPosition,
    state: State<'_, AppState>,
) -> Result<(), CommandError> {
    let mut settings = state.settings.lock_or_recover();
    settings.ui.pill_position = position;
    settings
        .save(&state.settings_path())
        .map_err(|e| CommandError::new("CONFIG_SAVE_FAILED", &e.to_string()))?;
    drop(settings);

    crate::overlay::reposition_pill(&app, position);
    let _ = app.emit("pill-position-changed", position);
    Ok(())
}

/// The pill's own RESTING/EXPANDED presentation state is frontend-owned
/// (it's driven by hover and by the capture phase, not by the shared
/// capture session truth), but window geometry can only be changed from
/// Rust — this is the actuator the frontend calls whenever that state
/// flips, so the native window always tightly matches what's actually
/// visible (never a bigger invisible hit-region than the pill itself).
#[tauri::command]
pub async fn set_pill_expanded(
    app: AppHandle,
    expanded: bool,
    state: State<'_, AppState>,
) -> Result<(), CommandError> {
    let position = state.settings.lock_or_recover().ui.pill_position;
    crate::overlay::set_expanded(&app, expanded, position);
    Ok(())
}

#[tauri::command]
pub async fn set_pill_window_mode(
    app: AppHandle,
    mode: String,
    state: State<'_, AppState>,
) -> Result<(), CommandError> {
    let position = state.settings.lock_or_recover().ui.pill_position;
    crate::overlay::set_pill_window_geometry(&app, &mode, position);
    Ok(())
}

#[tauri::command]
pub async fn start_capture(
    app: AppHandle,
    mode: String,
    state: State<'_, AppState>,
) -> Result<String, CommandError> {
    if mode.is_empty() {
        return Err(CommandError::new(
            "INVALID_INPUT",
            "Capture mode cannot be empty",
        ));
    }

    // The universal dictation hotkey (Ctrl+Space, held) and the in-app
    // Click-to-dictate button share one `AudioRecorder` — the microphone can
    // only feed one session at a time. Surface this as a distinct,
    // actionable error instead of the generic "already active" message so
    // the UI can explain *why* rather than looking broken.
    if state.recorder.active_mode().as_deref() == Some("dictation") {
        return Err(CommandError::new(
            "DICTATION_HOTKEY_ACTIVE",
            "Universal dictation is currently recording (hotkey held down). Release it first.",
        ));
    }

    let audio_dir = state.config_dir.join("audio");
    let result = state
        .recorder
        .start(&mode, &audio_dir, Some(app.clone()))
        .map_err(|e| CommandError::new("CAPTURE_FAILED", &e.to_string()));
    emit_capture_state(&app, &state.recorder);

    if result.is_ok() {
        // Kick these off now, in parallel with the user talking, rather
        // than waiting until they stop and need them immediately — by the
        // time transcription runs, a local Ollama that needed starting (or
        // a default Whisper model that needed downloading) has had real
        // time to come up.
        let provider = state.settings.lock_or_recover().provider.clone();
        if matches!(provider.active_provider, ProviderType::Ollama) {
            tauri::async_runtime::spawn(async move {
                crate::providers::ensure_ollama_ready(&provider.ollama_host, &provider.ollama_model).await;
            });
        }

        let has_model = state
            .settings
            .lock_or_recover()
            .stt
            .whisper_model_path
            .as_ref()
            .is_some_and(|p| !p.trim().is_empty());
        if !has_model {
            let models_dir = state.config_dir.join("models");
            tauri::async_runtime::spawn(async move {
                let _ = crate::capture::stt::ensure_default_model(&models_dir).await;
            });
        }
    }

    result
}

/// Broadcast whenever a capture session (from any surface — the in-app
/// pill, the floating overlay pill, or the universal dictation hotkey)
/// finishes processing, so every window can refresh its own view of the
/// vault/Kanban board without polling.
pub const CAPTURE_PROCESSED_EVENT: &str = "capture-processed";

/// Broadcast whenever a Voice Note is persisted to the vault, so the Voice
/// Note page can prepend it to Transcript History and refresh its stats
/// without polling or requiring a restart.
pub const VOICE_NOTE_SAVED_EVENT: &str = "voice-note-saved";

/// Persists `transcript` as a Voice Note and notifies every window. This is
/// the single funnel both the global dictation hotkey
/// (`hotkeys::stop_dictation_session`) and click-to-talk
/// (`process_captured_audio`) route every successful, non-empty transcript
/// through — regardless of whether OS text injection also happens for it —
/// so one recording can never produce more than one Voice Note. Callers
/// must already have guarded against an empty/whitespace-only transcript.
/// Failure to write is logged, not surfaced — a Voice Note write failure
/// must never interrupt dictation/injection, which have already succeeded
/// by the time this is called.
/// Returns the new note's id, so a caller that derives something from this
/// recording — a todo, say — can point at the note rather than at nothing.
/// `None` means the write failed, which is logged and never fatal.
pub fn save_voice_note(
    app: &AppHandle,
    vault: &VaultManager,
    transcript: &str,
    raw_transcript: Option<&str>,
    cleanup_style: Option<&str>,
) -> Option<String> {
    let note = VaultNote::new_voice_note_with_raw(
        transcript,
        raw_transcript.map(str::to_string),
        cleanup_style.map(str::to_string),
    );
    match vault.save_note(&note) {
        Ok(_) => {
            let _ = app.emit(VOICE_NOTE_SAVED_EVENT, &note);
            Some(note.id.clone())
        }
        Err(e) => {
            tracing::error!("Failed to save voice note: {}", e);
            None
        }
    }
}

/// The capture mode the TODOs page records in.
pub const TODO_CAPTURE_MODE: &str = "todo";

/// One spoken line, as a todo title.
///
/// Press-and-hold on the TODOs page is deliberately narrow: whatever was
/// said becomes one todo, not an extraction pass that might yield three or
/// none. Newlines are collapsed so the title stays one line, and a long
/// utterance is truncated for the title while the full text is kept as the
/// card's description — the recording is the record, and the title is only
/// how it is listed.
fn todo_title_from(transcript: &str) -> String {
    let single_line = transcript.split_whitespace().collect::<Vec<_>>().join(" ");
    if single_line.chars().count() <= 120 {
        return single_line;
    }
    let truncated: String = single_line.chars().take(119).collect();
    format!("{}…", truncated.trim_end())
}

/// Stops the active capture session and, only if the microphone actually
/// picked up audio (see [`crate::capture::CapturedAudio::had_audio`]),
/// transcribes and processes it. Returns `Ok(None)` — never a fabricated
/// `ProcessedPipelineResult` — when nothing was captured, so callers can
/// tell "recorded silence" apart from "recorded and processed speech"
/// instead of assuming every stop produced a result.
#[tauri::command]
pub async fn stop_capture(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<Option<ProcessedPipelineResult>, CommandError> {
    let captured = state
        .recorder
        .stop()
        .await
        .map_err(|e| CommandError::new("CAPTURE_STOP_FAILED", &e.to_string()));
    emit_capture_state(&app, &state.recorder);
    let captured = captured?;

    if !captured.had_audio {
        tracing::info!("[Dictation] Recording stopped with no audio input");
        emit_capture_status_event(&app, false, Some(captured.mode), "NO_SPEECH", None);
        return Ok(None);
    }

    emit_capture_status_event(&app, false, Some(captured.mode.clone()), "TRANSCRIBING", None);
    let result = process_captured_audio(&app, &state, captured).await;
    match &result {
        Ok(Some(processed)) => {
            let _ = app.emit(CAPTURE_PROCESSED_EVENT, processed);
        }
        Ok(None) => {
            // had_audio was true (real, sustained energy was captured) but
            // Whisper still produced no usable text — most commonly a short
            // hallucination (e.g. "Hello.") on a marginal recording that
            // whisper.cpp's own confidence/no-speech heuristics rejected
            // internally, leaving an empty transcript. Must not run
            // the note/kanban pipeline on nothing.
            tracing::info!("[Dictation] Transcription produced no usable text");
            emit_capture_status_event(&app, false, None, "NO_SPEECH", None);
        }
        Err(_) => {}
    }
    result
}

async fn process_captured_audio(
    app: &AppHandle,
    state: &State<'_, AppState>,
    captured: crate::capture::CapturedAudio,
) -> Result<Option<ProcessedPipelineResult>, CommandError> {
    let transcription = crate::capture::dictation::transcribe(app, state, &captured)
        .await
        .map_err(|message| CommandError::new("STT_FAILED", &message))?;

    let settings = state.settings.lock_or_recover().clone();

    // had_audio only proves the mic measured sustained energy — Whisper can
    // still land on nothing (most commonly a short hallucination that its
    // own internal confidence/no-speech heuristics then reject, leaving an
    // empty result) on a marginal recording. An empty transcript must never
    // reach the note/kanban pipeline.
    let Some(finished) = crate::capture::dictation::finish_text(&settings, &transcription.text) else {
        return Ok(None);
    };

    // A todo title and a scribble are structured by what comes next, so the
    // prose rewrite is for dictated voice notes only.
    let cleanup_style = if captured.mode == TODO_CAPTURE_MODE || captured.mode == "scribble" {
        crate::capture::rewrite::CleanupStyle::Raw
    } else {
        crate::capture::rewrite::CleanupStyle::from_setting(&settings.stt.cleanup_style)
    };
    let client = LLMClient::new(settings.provider.clone());
    let cleanup = crate::capture::dictation::clean_up(
        &client,
        finished,
        cleanup_style,
        crate::capture::dictation::CLEANUP_TIMEOUT,
    )
    .await;

    // Every successful, non-empty transcript becomes a Voice Note — this
    // must not depend on which mode-specific pipeline runs next, or on
    // whether it succeeds.
    let voice_note_id = save_voice_note(
        app,
        &state.vault,
        &cleanup.text,
        cleanup.before_if_changed(),
        cleanup.applied_style_name(),
    );
    let transcript = cleanup.text;

    match captured.mode.as_str() {
        // Press-and-hold on the TODOs page. Reuses this whole path —
        // recorder, STT, normalisation, dictionary — and differs only in
        // what it writes at the end. Everything upstream that can fail
        // (no audio, STT error, an empty transcript) has already returned
        // by here, so this arm cannot produce an empty todo.
        TODO_CAPTURE_MODE => {
            let mut card = KanbanCard::from_source(
                &todo_title_from(&transcript),
                crate::vault::TodoSourceKind::VoiceNote,
                crate::vault::TodoSourceRef {
                    // A voice note that failed to save leaves the todo
                    // pointing at nothing, which the surface reports as an
                    // unknown source rather than inventing an id.
                    id: voice_note_id.clone().unwrap_or_default(),
                    turn_ordinal: None,
                    label: Some("Spoken on the TODOs page".to_string()),
                },
                None,
                None,
            );
            card.description = transcript.clone();

            state
                .vault
                .save_kanban_card(&card)
                .map_err(|e| CommandError::new("VAULT_WRITE_FAILED", &e.to_string()))?;

            Ok(Some(ProcessedPipelineResult {
                mode: TODO_CAPTURE_MODE.to_string(),
                transcript: transcript.clone(),
                note_id: voice_note_id,
                kanban_cards_created: 1,
                output_markdown: card.title,
                sources: Vec::new(),
                spoken_audio_base64: None,
            }))
        }
        "voice_note" => Ok(Some(ProcessedPipelineResult {
            mode: "voice_note".to_string(),
            transcript: transcript.clone(),
            note_id: voice_note_id,
            kanban_cards_created: 0,
            output_markdown: transcript,
            sources: Vec::new(),
            spoken_audio_base64: None,
        })),
        "scribble" => {
            let llm = LLMClient::new(settings.provider.clone());
            PipelineEngine::process_scribble(&llm, &state.vault, &transcript)
                .await
                .map(Some)
                .map_err(|e| CommandError::new("PIPELINE_ERROR", &e.to_string()))
        }
        _ => Ok(Some(ProcessedPipelineResult {
            mode: captured.mode.clone(),
            transcript: transcript.clone(),
            note_id: None,
            kanban_cards_created: 0,
            output_markdown: transcript,
            sources: Vec::new(),
            spoken_audio_base64: None,
        })),
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AudioDeviceInfo {
    pub name: String,
    pub is_default: bool,
}


#[tauri::command]
pub async fn get_audio_devices() -> Result<Vec<AudioDeviceInfo>, CommandError> {
    use cpal::traits::{DeviceTrait, HostTrait};
    let host = cpal::default_host();
    let default_device_name = host.default_input_device().and_then(|d| d.name().ok());

    let mut devices = Vec::new();
    if let Ok(input_devices) = host.input_devices() {
        for device in input_devices {
            if let Ok(name) = device.name() {
                let is_default = default_device_name.as_deref() == Some(&name);
                devices.push(AudioDeviceInfo {
                    name,
                    is_default,
                });
            }
        }
    }
    Ok(devices)
}

/// The output devices a meeting can capture in loopback.
///
/// Separate from [`get_audio_devices`], which lists microphones and is what
/// every other surface asks for. A meeting is the one recording where the
/// output matters too: the far end of a call arrives through whichever device
/// the user is listening on, and a laptop with a headset connected has more
/// than one.
#[tauri::command]
pub async fn get_audio_output_devices() -> Result<Vec<AudioDeviceInfo>, CommandError> {
    use cpal::traits::{DeviceTrait, HostTrait};
    let host = cpal::default_host();
    let default_name = host.default_output_device().and_then(|d| d.name().ok());

    let mut devices = Vec::new();
    if let Ok(found) = host.output_devices() {
        for device in found {
            if let Ok(name) = device.name() {
                let is_default = default_name.as_deref() == Some(&name);
                devices.push(AudioDeviceInfo { name, is_default });
            }
        }
    }
    Ok(devices)
}
