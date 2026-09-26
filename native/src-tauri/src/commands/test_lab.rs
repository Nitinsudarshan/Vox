//! The Dictation Test Lab: recording, multi-model runs and reports.

use super::*;

// ---------------------------------------------------------------------------
// Dictation Test Lab Benchmarking Commands
// ---------------------------------------------------------------------------

#[tauri::command]
pub async fn get_available_test_models(
    state: State<'_, AppState>,
) -> Result<Vec<crate::capture::benchmark::AvailableModelTarget>, CommandError> {
    let models_dir = state.config_dir.join("models");
    Ok(crate::capture::benchmark::discover_available_models(&models_dir))
}

#[tauri::command]
pub async fn get_available_cleanup_styles() -> Result<Vec<crate::capture::benchmark::CleanupStyleInfo>, CommandError> {
    Ok(crate::capture::benchmark::get_available_cleanup_styles())
}

#[tauri::command]
pub async fn start_dictation_test_recording(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<String, CommandError> {
    if state.recorder.is_active() {
        return Err(CommandError::new(
            "RECORDER_ACTIVE",
            "A recording session is already active.",
        ));
    }
    let audio_dir = state.config_dir.join("audio");
    state
        .recorder
        .start("dictation_test", &audio_dir, Some(app))
        .map_err(|e| CommandError::new("CAPTURE_FAILED", &e.to_string()))
}

/// Discards an in-progress Test Lab recording without benchmarking it, e.g.
/// when the user leaves the page mid-recording. Only a `dictation_test`
/// session is touched, so this can never cut off hotkey dictation or a meeting.
#[tauri::command]
pub async fn cancel_dictation_test_recording(state: State<'_, AppState>) -> Result<bool, CommandError> {
    if state.recorder.active_mode().as_deref() != Some("dictation_test") {
        return Ok(false);
    }
    let captured = state
        .recorder
        .stop()
        .await
        .map_err(|e| CommandError::new("CAPTURE_STOP_FAILED", &e.to_string()))?;
    let _ = std::fs::remove_file(&captured.audio_path);
    Ok(true)
}

#[tauri::command]
pub async fn stop_dictation_test_recording(
    app: AppHandle,
    request: crate::capture::benchmark::RunDictationTestRequest,
    state: State<'_, AppState>,
) -> Result<crate::capture::benchmark::DictationTestRun, CommandError> {
    let t_stop = std::time::Instant::now();
    let captured = state
        .recorder
        .stop()
        .await
        .map_err(|e| CommandError::new("CAPTURE_STOP_FAILED", &e.to_string()))?;
    let t_audio_ready = std::time::Instant::now();

    if !captured.had_audio || captured.samples.is_empty() {
        return Err(CommandError::new(
            "NO_SPEECH",
            "No speech detected in recording session.",
        ));
    }

    let settings = state.settings.lock_or_recover().clone();
    crate::capture::benchmark::execute_benchmark_run(
        Some(&app),
        &state.config_dir,
        &captured.samples,
        &captured.audio_path,
        t_stop,
        t_audio_ready,
        request,
        settings,
    )
    .await
    .map_err(|e| CommandError::new("BENCHMARK_FAILED", &e))
}

#[tauri::command]
pub async fn run_dictation_test_on_audio(
    app: AppHandle,
    audio_path: String,
    request: crate::capture::benchmark::RunDictationTestRequest,
    state: State<'_, AppState>,
) -> Result<crate::capture::benchmark::DictationTestRun, CommandError> {
    let path = std::path::PathBuf::from(&audio_path);
    if !path.is_file() {
        return Err(CommandError::new("FILE_NOT_FOUND", "Audio file not found."));
    }

    // Shared decoder: honours bit depth (8/16/24/32-bit int, float) and
    // averages channels before resampling, so stereo and 24-bit files decode
    // correctly instead of garbled or empty.
    let path_clone = path.clone();
    let samples = tokio::task::spawn_blocking(move || {
        crate::meetings::checkpoint::read_wav_mono_16k(&path_clone).map_err(|e| format!("Failed to read WAV: {}", e))
    })
    .await
    .map_err(|e| CommandError::new("IO_ERROR", &e.to_string()))?
    .map_err(|e| CommandError::new("WAV_ERROR", &e))?;

    if samples.is_empty() {
        return Err(CommandError::new("WAV_ERROR", "WAV file contains no decodable audio."));
    }

    let t_now = std::time::Instant::now();
    let settings = state.settings.lock_or_recover().clone();

    crate::capture::benchmark::execute_benchmark_run(
        Some(&app),
        &state.config_dir,
        &samples,
        &audio_path,
        t_now,
        t_now,
        request,
        settings,
    )
    .await
    .map_err(|e| CommandError::new("BENCHMARK_FAILED", &e))
}

#[tauri::command]
pub async fn get_dictation_test_history(
    state: State<'_, AppState>,
) -> Result<Vec<crate::capture::benchmark::DictationTestSummary>, CommandError> {
    crate::capture::benchmark::list_dictation_test_history(&state.config_dir)
        .map_err(|e| CommandError::new("STORAGE_ERROR", &e))
}

#[tauri::command]
pub async fn get_dictation_test_run(
    test_id: String,
    state: State<'_, AppState>,
) -> Result<Option<crate::capture::benchmark::DictationTestRun>, CommandError> {
    crate::capture::benchmark::load_dictation_test_run(&state.config_dir, &test_id)
        .map_err(|e| CommandError::new("STORAGE_ERROR", &e))
}

#[tauri::command]
pub async fn delete_dictation_test_run(
    test_id: String,
    state: State<'_, AppState>,
) -> Result<bool, CommandError> {
    crate::capture::benchmark::delete_dictation_test_run(&state.config_dir, &test_id)
        .map_err(|e| CommandError::new("STORAGE_ERROR", &e))
}

#[tauri::command]
pub async fn export_dictation_test_report(
    test_id: String,
    format: String,
    state: State<'_, AppState>,
) -> Result<String, CommandError> {
    let run = crate::capture::benchmark::load_dictation_test_run(&state.config_dir, &test_id)
        .map_err(|e| CommandError::new("STORAGE_ERROR", &e))?
        .ok_or_else(|| CommandError::new("NOT_FOUND", "Test run not found"))?;

    if format.to_lowercase() == "json" {
        serde_json::to_string_pretty(&run)
            .map_err(|e| CommandError::new("SERIALIZATION_ERROR", &e.to_string()))
    } else {
        Ok(crate::capture::benchmark::format_markdown_report(&run))
    }
}

#[tauri::command]
pub async fn inject_dictation_test_result(
    text: String,
) -> Result<(), CommandError> {
    crate::hotkeys::injection::copy_to_clipboard(&text)
        .map_err(|e| CommandError::new("CLIPBOARD_FAILED", &e.to_string()))?;
    crate::hotkeys::injection::paste_from_clipboard()
        .map_err(|e| CommandError::new("INJECTION_FAILED", &e.to_string()))
}
