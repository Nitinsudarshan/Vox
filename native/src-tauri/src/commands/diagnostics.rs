//! Diagnostics: STT variant comparison, evaluation and the corpus.

use super::*;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SttDiagnosticResult {
    pub model_used: String,
    pub auto_transcript: String,
    pub auto_duration_ms: u64,
    pub hindi_locked_transcript: String,
    pub hindi_locked_duration_ms: u64,
    pub english_locked_transcript: String,
    pub english_locked_duration_ms: u64,
}

/// Diagnostic development helper: runs an existing recorded WAV through Auto (None),
/// Hindi-locked ("hi"), and English-locked ("en") STT configurations to compare raw model emissions.
#[tauri::command]
pub async fn diagnose_stt_variants(
    wav_path: String,
    custom_model_path: Option<String>,
    state: State<'_, AppState>,
) -> Result<SttDiagnosticResult, CommandError> {
    let settings = state.settings.lock_or_recover().clone();
    let stt = state.stt.clone();
    let model_path = custom_model_path
        .filter(|p| !p.trim().is_empty())
        .or_else(|| settings.stt.whisper_model_path.clone());

    let samples = tokio::task::spawn_blocking(move || -> Result<Vec<f32>, String> {
        let mut reader = hound::WavReader::open(&wav_path)
            .map_err(|e| format!("Failed to open WAV: {}", e))?;
        let spec = reader.spec();
        let raw_samples: Vec<f32> = match spec.sample_format {
            hound::SampleFormat::Float => reader.samples::<f32>().filter_map(|s| s.ok()).collect(),
            hound::SampleFormat::Int => reader
                .samples::<i16>()
                .filter_map(|s| s.ok())
                .map(|s| s as f32 / i16::MAX as f32)
                .collect(),
        };
        Ok(raw_samples)
    })
    .await
    .map_err(|e| CommandError::new("IO_ERROR", &e.to_string()))?
    .map_err(|e| CommandError::new("WAV_ERROR", &e))?;

    let model_label = model_path
        .as_deref()
        .map(|p| p.split(['/', '\\']).next_back().unwrap_or(p))
        .unwrap_or("default")
        .to_string();

    let stt1 = stt.clone();
    let mp1 = model_path.clone();
    let s1 = samples.clone();
    let (auto_res, auto_duration_ms) = tokio::task::spawn_blocking(move || {
        let start = std::time::Instant::now();
        let cfg = crate::capture::SttLanguageConfig {
            whisper_language: None,
            translate: false,
        };
        let res = stt1.transcribe(mp1.as_deref(), &s1, &cfg).unwrap_or_default();
        (res, start.elapsed().as_millis() as u64)
    })
    .await
    .map_err(|e| CommandError::new("STT_FAILED", &e.to_string()))?;

    let stt2 = stt.clone();
    let mp2 = model_path.clone();
    let s2 = samples.clone();
    let (hi_res, hindi_locked_duration_ms) = tokio::task::spawn_blocking(move || {
        let start = std::time::Instant::now();
        let cfg = crate::capture::SttLanguageConfig {
            whisper_language: Some("hi".to_string()),
            translate: false,
        };
        let res = stt2.transcribe(mp2.as_deref(), &s2, &cfg).unwrap_or_default();
        (res, start.elapsed().as_millis() as u64)
    })
    .await
    .map_err(|e| CommandError::new("STT_FAILED", &e.to_string()))?;

    let stt3 = stt.clone();
    let mp3 = model_path.clone();
    let s3 = samples.clone();
    let (en_res, english_locked_duration_ms) = tokio::task::spawn_blocking(move || {
        let start = std::time::Instant::now();
        let cfg = crate::capture::SttLanguageConfig {
            whisper_language: Some("en".to_string()),
            translate: false,
        };
        let res = stt3.transcribe(mp3.as_deref(), &s3, &cfg).unwrap_or_default();
        (res, start.elapsed().as_millis() as u64)
    })
    .await
    .map_err(|e| CommandError::new("STT_FAILED", &e.to_string()))?;

    Ok(SttDiagnosticResult {
        model_used: model_label,
        auto_transcript: auto_res,
        auto_duration_ms,
        hindi_locked_transcript: hi_res,
        hindi_locked_duration_ms,
        english_locked_transcript: en_res,
        english_locked_duration_ms,
    })
}

/// Evaluates a recorded WAV file against a specific STT decoding configuration variant
/// (e.g. baseline, relay_prompt, best_of_3, beam_2, temperature_fallback) using the Phase 5 harness.
#[tauri::command]
pub async fn run_stt_evaluation(
    wav_path: String,
    variant: String,
    reference_text: Option<String>,
    custom_model_path: Option<String>,
    state: State<'_, AppState>,
) -> Result<crate::capture::EvaluationResult, CommandError> {
    let settings = state.settings.lock_or_recover().clone();
    let stt = state.stt.clone();
    let model_path = custom_model_path
        .filter(|p| !p.trim().is_empty())
        .or_else(|| settings.stt.whisper_model_path.clone());

    let wav_path_for_read = wav_path.clone();
    let (samples, sample_rate) = tokio::task::spawn_blocking(move || -> Result<(Vec<f32>, u32), String> {
        let mut reader = hound::WavReader::open(&wav_path_for_read)
            .map_err(|e| format!("Failed to open WAV: {}", e))?;
        let spec = reader.spec();
        let raw_samples: Vec<f32> = match spec.sample_format {
            hound::SampleFormat::Float => reader.samples::<f32>().filter_map(|s| s.ok()).collect(),
            hound::SampleFormat::Int => reader
                .samples::<i16>()
                .filter_map(|s| s.ok())
                .map(|s| s as f32 / i16::MAX as f32)
                .collect(),
        };
        Ok((raw_samples, spec.sample_rate))
    })
    .await
    .map_err(|e| CommandError::new("IO_ERROR", &e.to_string()))?
    .map_err(|e| CommandError::new("WAV_ERROR", &e))?;

    if variant.to_lowercase() == "parakeet" {
        let models_dir = state.config_dir.join("models");
        let parakeet_dir = models_dir.join("parakeet");
        if !crate::capture::parakeet::ModelFiles::is_installed_in(&parakeet_dir) {
            return Err(CommandError::new(
                "NOT_INSTALLED",
                "Parakeet TDT model files are not installed. Download them in Settings -> Models first.",
            ));
        }
        let original_dur = samples.len() as f32 / sample_rate.max(1) as f32;
        let mono_16k = crate::capture::resample_to_16k_mono(&samples, sample_rate);
        let audio_stats = crate::capture::AudioStats::compute(&mono_16k, 16000, 1);
        let vad = crate::capture::VadConfig::default();
        let (vad_samples, vad_result) = vad.process(&mono_16k, 16000);

        let clean_label = std::path::Path::new(&wav_path)
            .file_name()
            .map(|f| f.to_string_lossy().to_string())
            .unwrap_or_else(|| "eval_audio.wav".to_string());

        if !vad_result.speech_detected || vad_samples.is_empty() {
            let res = crate::capture::EvaluationResult {
                test_id: "manual_eval".to_string(),
                audio_file: clean_label,
                configuration: "NVIDIA Parakeet TDT 0.6B".to_string(),
                language_setting: "en-US (punctuation+casing)".to_string(),
                resolved_whisper_language: None,
                original_duration_seconds: original_dur,
                processed_duration_seconds: audio_stats.duration_seconds,
                inference_duration_ms: 0,
                real_time_factor: 0.0,
                transcript: String::new(),
                audio_rms: audio_stats.rms,
                audio_peak: audio_stats.peak_amplitude,
                near_zero_percent: audio_stats.near_zero_percent,
                speech_detected: vad_result.speech_detected,
                vad_trimmed_duration: vad_result.trimmed_duration,
                model_filename: "parakeet-tdt-0.6b-v3".to_string(),
                sampling_strategy: "Fast TDT (Greedy)".to_string(),
                best_of: 1,
                beam_size: None,
                temperature: 0.0,
                temperature_increment: 0.0,
                initial_prompt_used: false,
                no_speech_threshold: 0.0,
                entropy_threshold: 0.0,
                logprob_threshold: 0.0,
                accuracy: reference_text.as_deref().map(|r| crate::capture::calculate_accuracy(r, "")),
                fallback_triggered: false,
                error: None,
            };
            return Ok(res);
        }

        let parakeet_dir_clone = parakeet_dir.clone();
        let stt_clone = stt.clone();
        let (text, diag) = tokio::task::spawn_blocking(move || {
            stt_clone.transcribe_parakeet(&parakeet_dir_clone, &vad_samples)
        })
        .await
        .map_err(|e| CommandError::new("EVAL_ERROR", &e.to_string()))?
        .map_err(|e| CommandError::new("EVAL_ERROR", &e.to_string()))?;

        let res = crate::capture::EvaluationResult {
            test_id: "manual_eval".to_string(),
            audio_file: clean_label,
            configuration: "NVIDIA Parakeet TDT 0.6B".to_string(),
            language_setting: "en-US (punctuation+casing)".to_string(),
            resolved_whisper_language: None,
            original_duration_seconds: original_dur,
            processed_duration_seconds: audio_stats.duration_seconds,
            inference_duration_ms: diag.transcription_latency_ms,
            real_time_factor: diag.real_time_factor,
            transcript: text.clone(),
            audio_rms: audio_stats.rms,
            audio_peak: audio_stats.peak_amplitude,
            near_zero_percent: audio_stats.near_zero_percent,
            speech_detected: vad_result.speech_detected,
            vad_trimmed_duration: vad_result.trimmed_duration,
            model_filename: "parakeet-tdt-0.6b-v3".to_string(),
            sampling_strategy: "Fast TDT (Greedy)".to_string(),
            best_of: 1,
            beam_size: None,
            temperature: 0.0,
            temperature_increment: 0.0,
            initial_prompt_used: false,
            no_speech_threshold: 0.0,
            entropy_threshold: 0.0,
            logprob_threshold: 0.0,
            accuracy: reference_text.as_deref().map(|r| crate::capture::calculate_accuracy(r, &text)),
            fallback_triggered: false,
            error: None,
        };
        return Ok(res);
    }

    let eval_variant = match variant.to_lowercase().as_str() {
        "relay_prompt" | "prompt" => crate::capture::EvalConfigVariant::RelayPrompt,
        "best_of_3" | "best_of" => crate::capture::EvalConfigVariant::BestOf3,
        "beam_2" | "beam" => crate::capture::EvalConfigVariant::Beam2,
        "temperature_fallback" | "fallback" => crate::capture::EvalConfigVariant::TemperatureFallback,
        _ => crate::capture::EvalConfigVariant::Baseline,
    };

    let result = tokio::task::spawn_blocking(move || {
        crate::capture::evaluate_audio_buffer(
            "manual_eval",
            "eval_audio.wav",
            &samples,
            sample_rate,
            eval_variant,
            &settings.language,
            model_path.as_deref(),
            reference_text.as_deref(),
            &stt,
        )
    })
    .await
    .map_err(|e| CommandError::new("EVAL_ERROR", &e.to_string()))?;

    Ok(result)
}

/// Retrieves the full 35-item curated evaluation corpus manifest for UI test-bench selection.
#[tauri::command]
pub async fn get_stt_corpus() -> Result<Vec<crate::capture::CorpusItem>, CommandError> {
    Ok(crate::capture::get_curated_corpus())
}
