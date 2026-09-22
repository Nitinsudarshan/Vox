//! Multi-model dictation benchmarking harness and test lab.
//!
//! Provides a controlled, reproducible test environment to measure and compare:
//! - Transcription speed & RTF
//! - Transcription accuracy (WER, CER, substitutions, deletions, insertions)
//! - Model and configuration differences
//! - Raw STT output vs cleaned output
//! - Cleanup/rewrite latency across all available cleanup styles
//! - Cold vs warm model execution
//!
//! One dictation recording is captured once, saved as an immutable benchmark
//! audio artifact, and fanned out to every selected speech recognizer and
//! cleanup style.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter};

use crate::capture::evaluation::{calculate_accuracy, AccuracyMetrics};
use crate::capture::models;
use crate::capture::recognizer::{
    LanguageSupport, RecognitionError, RecognitionRequest, RecognizerCapabilities,
    RecognizerKind, SpeechRecognizer,
};
use crate::capture::recognizers::{ParakeetRecognizer, WhisperRecognizer};
use crate::capture::rewrite::{self, CleanupStyle, DiffSpan};
use crate::capture::stt::{SttEngine, WhisperDecodingConfig};
use crate::providers::{LLMClient, ProviderType};
use crate::settings::AppSettings;

/// Current schema version for dictation test run records.
pub const TEST_RUN_SCHEMA_VERSION: u32 = 1;

/// Event emitted when an individual model completes in a test run.
pub const DICTATION_TEST_PROGRESS_EVENT: &str = "dictation-test-progress";

/// Detailed information about an available recognizer test target.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AvailableModelTarget {
    /// Unique identifier for this target (e.g. "whisper:ggml-small.bin", "parakeet:default").
    pub target_id: String,
    /// Engine identifier ("whisper", "parakeet").
    pub engine_id: String,
    /// Engine display name ("Whisper", "NVIDIA Parakeet TDT").
    pub engine_display_name: String,
    /// Model name or variant ("Small", "Parakeet TDT 0.6B").
    pub model_name: String,
    /// Model filename or directory.
    pub model_filename: String,
    /// Full path to model file or directory.
    pub model_path: String,
    /// Whether the model is installed on disk.
    pub installed: bool,
    /// Whether the engine is compiled into this build.
    pub compiled_in: bool,
    /// Engine capabilities.
    pub capabilities: RecognizerCapabilities,
    /// Language handling capabilities.
    pub language_support: LanguageSupport,
    /// Parameter count in millions, if known.
    pub parameters_millions: Option<u32>,
    /// Model file size on disk in bytes, if installed.
    pub size_bytes: Option<u64>,
    /// Tier if available.
    pub tier: Option<String>,
    /// Brief description.
    pub blurb: String,
}

/// Information describing an available cleanup style.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CleanupStyleInfo {
    pub id: String,
    pub display_name: String,
    pub description: String,
    pub is_default: bool,
}

/// High-level summary of a past test run for list views.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DictationTestSummary {
    pub test_id: String,
    pub created_at_epoch_ms: u128,
    pub created_at_formatted: String,
    pub label: Option<String>,
    pub audio_duration_seconds: f32,
    pub models_tested_count: usize,
    pub successful_models_count: usize,
    pub fastest_model_name: Option<String>,
    pub fastest_total_ms: Option<u128>,
    pub cleanup_styles: Vec<String>,
    pub has_reference_transcript: bool,
}

/// The result of applying a single cleanup style to a model's raw transcript.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CleanupResult {
    pub style: String,
    pub style_display_name: String,
    pub cleaned_text: String,
    pub changed: bool,
    #[serde(default)]
    pub queue_wait_ms: u128,
    pub duration_ms: u128,
    pub raw_char_count: usize,
    pub raw_word_count: usize,
    pub cleaned_char_count: usize,
    pub cleaned_word_count: usize,
    pub diff_spans: Vec<DiffSpan>,
    pub error: Option<String>,
}

fn default_prod_cleanup_style() -> String {
    "faithful".to_string()
}

/// Detailed timing breakdown for a single model's execution.
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct DictationModelTimings {
    /// Time from recording stop until audio buffer was normalized & ready.
    pub recording_to_audio_ready_ms: u128,
    /// Queue wait time in the benchmark before this model started.
    pub queue_wait_ms: u128,
    /// Time spent loading the model into memory (non-zero if cold/reload).
    pub model_load_ms: u128,
    /// Whether the model was cold (loaded from disk or evicted previous model).
    pub is_cold_load: bool,
    /// Lock wait time attempting to acquire the engine model mutex.
    #[serde(default)]
    pub lock_wait_ms: u128,
    /// Duration of the outer recognizer.transcribe() call.
    #[serde(default)]
    pub recognizer_call_duration_ms: u128,
    /// STT inference execution time.
    pub stt_execution_ms: u128,
    /// Time between STT decode end and text being available.
    pub stt_to_text_available_ms: u128,
    /// The name of the cleanup style used for the production E2E path (default: "faithful").
    #[serde(default = "default_prod_cleanup_style")]
    pub production_cleanup_style: String,
    /// Execution duration of the selected production cleanup style.
    #[serde(default)]
    pub production_cleanup_duration_ms: u128,
    /// Estimated Production End-to-End latency:
    /// recording_to_audio_ready + model_load + stt_execution + stt_to_text + production_cleanup_duration.
    #[serde(default)]
    pub production_e2e_ms: u128,
    /// Estimated Production End-to-End latency for every evaluated cleanup style:
    /// recording_to_audio_ready + model_load + stt_execution + stt_to_text + style.duration.
    #[serde(default)]
    pub production_e2e_by_style: BTreeMap<String, u128>,
    /// Total cumulative time spent benchmarking all cleanup styles for this model.
    #[serde(default)]
    pub cleanup_benchmark_total_ms: u128,
    /// Wall-clock time for this model's benchmark run (model_test_end - model_test_start).
    #[serde(default)]
    pub benchmark_wall_clock_ms: u128,
    /// Legacy alias for total_cleanup_ms: equals cleanup_benchmark_total_ms.
    pub total_cleanup_ms: u128,
    /// Legacy alias for total_latency_ms: equals production_e2e_ms.
    pub total_latency_ms: u128,
    /// Real-time factor (stt_execution_s / audio_duration_s).
    pub rtf: f32,

    // Relative timeline offsets from recording_stop (in ms, for visual waterfall)
    #[serde(default)]
    pub timeline_queue_start_ms: u128,
    #[serde(default)]
    pub timeline_model_start_ms: u128,
    #[serde(default)]
    pub timeline_stt_start_ms: u128,
    #[serde(default)]
    pub timeline_stt_end_ms: u128,
    #[serde(default)]
    pub timeline_model_end_ms: u128,

    // Formatted ISO/timestamp markers matching existing DICTATION LATENCY TRACE
    pub recording_stop_ts: String,
    pub audio_ready_ts: String,
    #[serde(default)]
    pub model_start_ts: String,
    pub stt_start_ts: String,
    pub stt_end_ts: String,
    pub text_available_ts: String,
    pub test_complete_ts: String,

    // Future streaming STT fields for forward-compatibility
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub time_to_first_partial_ms: Option<u128>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub time_to_first_stable_partial_ms: Option<u128>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub streaming_finalization_ms: Option<u128>,
}

/// Execution configuration metadata for reproducibility.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ModelExecutionConfig {
    pub engine: String,
    pub model_name: String,
    pub model_filename: String,
    pub language: Option<String>,
    pub task: String,
    pub decoding_strategy: String,
    pub temperature: f32,
    pub initial_prompt: Option<String>,
    pub threads: Option<u32>,
    pub backend: String,
}

/// Comprehensive benchmark result for one speech model on a test recording.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DictationTestModelResult {
    pub target_id: String,
    pub engine_id: String,
    pub model_name: String,
    pub config: ModelExecutionConfig,
    pub success: bool,
    pub error: Option<String>,
    pub raw_transcript: String,
    pub cleanup_results: BTreeMap<String, CleanupResult>,
    pub timings: DictationModelTimings,
    pub accuracy: Option<AccuracyMetrics>,
    pub char_count: usize,
    pub word_count: usize,
    pub segment_count: usize,
}

/// Metadata describing the immutable captured audio artifact.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BenchmarkAudioMetadata {
    pub audio_path: String,
    pub filename: String,
    pub duration_seconds: f32,
    pub sample_rate: u32,
    pub channels: u16,
    pub format: String,
    pub sample_count: usize,
}

/// Environment metadata ensuring test reproducibility.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BenchmarkEnvironmentMetadata {
    pub os: String,
    pub vox_version: String,
    pub platform: String,
    pub cleanup_provider: String,
    pub cleanup_model: String,
}

/// An immutable, complete test run record containing multi-model results on one recording.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DictationTestRun {
    pub schema_version: u32,
    pub test_id: String,
    pub label: Option<String>,
    pub created_at_epoch_ms: u128,
    pub created_at_formatted: String,
    pub audio: BenchmarkAudioMetadata,
    pub environment: BenchmarkEnvironmentMetadata,
    pub reference_transcript: Option<String>,
    pub selected_models: Vec<String>,
    pub selected_cleanup_styles: Vec<String>,
    pub model_results: Vec<DictationTestModelResult>,
    pub total_run_duration_ms: u128,
}

/// Progress payload emitted as each model completes during execution.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DictationTestProgressPayload {
    pub test_id: String,
    pub current_model_index: usize,
    pub total_models: usize,
    pub completed_model: DictationTestModelResult,
}

/// Request payload to run dictation benchmark test on captured audio.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunDictationTestRequest {
    pub selected_target_ids: Vec<String>,
    pub selected_cleanup_styles: Vec<String>,
    #[serde(default)]
    pub production_cleanup_style: Option<String>,
    pub reference_transcript: Option<String>,
    pub test_label: Option<String>,
    pub language_override: Option<String>,
    #[serde(default)]
    pub concurrency: Option<usize>,
}

// ---------------------------------------------------------------------------
// Discovery Functions
// ---------------------------------------------------------------------------

/// Dynamically discovers all available STT recognizers and installed model variants.
pub fn discover_available_models(models_dir: &Path) -> Vec<AvailableModelTarget> {
    let mut targets = Vec::new();

    // 1. Whisper models
    let whisper_compiled = RecognizerKind::Whisper.compiled_in();
    let installed_whisper = models::installed(models_dir);

    for m in installed_whisper {
        let target_id = format!("whisper:{}", m.filename);
        let tier_name = match m.tier {
            models::ModelTier::Fast => "Fast",
            models::ModelTier::Balanced => "Balanced",
            models::ModelTier::Accurate => "Accurate",
            models::ModelTier::Maximum => "Maximum",
        };

        targets.push(AvailableModelTarget {
            target_id,
            engine_id: RecognizerKind::Whisper.id().to_string(),
            engine_display_name: RecognizerKind::Whisper.display_name().to_string(),
            model_name: format!("Whisper {}", m.name),
            model_filename: m.filename.clone(),
            model_path: m.path.clone(),
            installed: m.installed,
            compiled_in: whisper_compiled,
            capabilities: RecognizerKind::Whisper.capabilities(),
            language_support: LanguageSupport::Selectable { detects: true },
            parameters_millions: Some(m.parameters_millions),
            size_bytes: if m.size_bytes > 0 { Some(m.size_bytes) } else { None },
            tier: Some(tier_name.to_string()),
            blurb: m.blurb,
        });
    }

    // 2. Parakeet TDT
    let parakeet_compiled = RecognizerKind::Parakeet.compiled_in();
    let parakeet_dir = models_dir.join("parakeet");
    let parakeet_installed = parakeet_compiled && crate::capture::parakeet::ModelFiles::is_installed_in(&parakeet_dir);

    targets.push(AvailableModelTarget {
        target_id: "parakeet:tdt-0.6b-v3".to_string(),
        engine_id: RecognizerKind::Parakeet.id().to_string(),
        engine_display_name: RecognizerKind::Parakeet.display_name().to_string(),
        model_name: "NVIDIA Parakeet TDT 0.6B".to_string(),
        model_filename: "parakeet-tdt-0.6b-v3".to_string(),
        model_path: parakeet_dir.to_string_lossy().to_string(),
        installed: parakeet_installed,
        compiled_in: parakeet_compiled,
        capabilities: RecognizerKind::Parakeet.capabilities(),
        language_support: LanguageSupport::Fixed {
            language: "en".to_string(),
        },
        parameters_millions: Some(600),
        size_bytes: None,
        tier: Some("Ultra-Fast".to_string()),
        blurb: "NVIDIA FastConformer-TDT 0.6B English ASR running via ONNX Runtime.".to_string(),
    });

    targets
}

/// Dynamically returns all available cleanup styles supported by Vox.
pub fn get_available_cleanup_styles() -> Vec<CleanupStyleInfo> {
    vec![
        CleanupStyleInfo {
            id: CleanupStyle::Raw.as_str().to_string(),
            display_name: "Raw (Verbatim)".to_string(),
            description: "Verbatim transcript. Skips any LLM pass completely.".to_string(),
            is_default: true,
        },
        CleanupStyleInfo {
            id: CleanupStyle::Faithful.as_str().to_string(),
            display_name: "Faithful".to_string(),
            description: "Removes filler words and false starts only; preserves speaker's own vocabulary.".to_string(),
            is_default: true,
        },
        CleanupStyleInfo {
            id: CleanupStyle::Clean.as_str().to_string(),
            display_name: "Clean".to_string(),
            description: "Fixes grammar and punctuation, and splits run-on sentences.".to_string(),
            is_default: false,
        },
        CleanupStyleInfo {
            id: CleanupStyle::Professional.as_str().to_string(),
            display_name: "Polished".to_string(),
            description: "Raises the register to professional correspondence while keeping facts exact.".to_string(),
            is_default: false,
        },
        CleanupStyleInfo {
            id: CleanupStyle::Concise.as_str().to_string(),
            display_name: "Concise".to_string(),
            description: "Cuts redundancy and repetitions; sharpens brevity.".to_string(),
            is_default: false,
        },
    ]
}

// ---------------------------------------------------------------------------
// Execution Coordinator
// ---------------------------------------------------------------------------

/// Formats Instant relative to now into a display timestamp HH:MM:SS.mmm.
fn format_instant_ts(instant: Instant, reference_instant: Instant, reference_sys: SystemTime) -> String {
    let sys_time = if instant <= reference_instant {
        let diff = reference_instant.duration_since(instant);
        reference_sys.checked_sub(diff).unwrap_or(reference_sys)
    } else {
        let diff = instant.duration_since(reference_instant);
        reference_sys.checked_add(diff).unwrap_or(reference_sys)
    };
    let dur = sys_time.duration_since(UNIX_EPOCH).unwrap_or_default();
    let secs = dur.as_secs();
    let millis = dur.subsec_millis();
    let hours = (secs / 3600) % 24;
    let mins = (secs / 60) % 60;
    let s = secs % 60;
    format!("{:02}:{:02}:{:02}.{:03}", hours, mins, s, millis)
}

/// Executes an isolated test pass for a single target model, recording all timing and accuracy metrics.
#[allow(clippy::too_many_arguments)]
async fn execute_single_model(
    target: &AvailableModelTarget,
    samples: &[f32],
    audio_duration_seconds: f32,
    t_recording_stop: Instant,
    t_audio_ready: Instant,
    model_test_start: Instant,
    language_to_use: Option<String>,
    cleanup_styles: &[CleanupStyle],
    selected_prod_style: &str,
    reference_transcript: Option<&str>,
    settings: &AppSettings,
    now_system: SystemTime,
) -> DictationTestModelResult {
    let audio_prep_ms = t_audio_ready.duration_since(t_recording_stop).as_millis();
    let queue_wait_ms = model_test_start.duration_since(t_audio_ready).as_millis();

    let target_clone = target.clone();
    let samples_vec = samples.to_vec();
    let lang_clone = language_to_use.clone();
    let stt_settings = settings.stt.clone();

    // 1. Run STT Inference in blocking task
    let stt_task_result = tokio::task::spawn_blocking(move || {
        let t_stt_start = Instant::now();

        let (recognition_res, config, call_dur) = match target_clone.engine_id.as_str() {
            "whisper" => {
                let dec_config = WhisperDecodingConfig::for_dictation(&stt_settings);
                let config_snapshot = ModelExecutionConfig {
                    engine: "Whisper".to_string(),
                    model_name: target_clone.model_name.clone(),
                    model_filename: target_clone.model_filename.clone(),
                    language: lang_clone.clone(),
                    task: "transcribe".to_string(),
                    decoding_strategy: format!("{:?}", dec_config.strategy),
                    temperature: dec_config.temperature,
                    initial_prompt: dec_config.initial_prompt.clone(),
                    threads: dec_config.n_threads.map(|n| n as u32),
                    backend: "whisper.cpp (CPU/AVX2)".to_string(),
                };

                let recognizer = WhisperRecognizer::new(
                    SttEngine::new(),
                    PathBuf::from(&target_clone.model_path),
                    dec_config,
                );

                let req = RecognitionRequest {
                    samples: &samples_vec,
                    language: lang_clone,
                    translate: false,
                };
                let t_call_start = Instant::now();
                let res = recognizer.transcribe(req);
                let call_duration = t_call_start.elapsed().as_millis();
                (res, config_snapshot, call_duration)
            }
            "parakeet" => {
                let config_snapshot = ModelExecutionConfig {
                    engine: "NVIDIA Parakeet TDT".to_string(),
                    model_name: target_clone.model_name.clone(),
                    model_filename: target_clone.model_filename.clone(),
                    language: Some("en".to_string()),
                    task: "transcribe".to_string(),
                    decoding_strategy: "greedy_tdt".to_string(),
                    temperature: 0.0,
                    initial_prompt: None,
                    threads: None,
                    backend: "ONNX Runtime".to_string(),
                };

                let recognizer = ParakeetRecognizer::new(
                    SttEngine::new(),
                    PathBuf::from(&target_clone.model_path),
                );

                let req = RecognitionRequest {
                    samples: &samples_vec,
                    language: Some("en".to_string()),
                    translate: false,
                };
                let t_call_start = Instant::now();
                let res = recognizer.transcribe(req);
                let call_duration = t_call_start.elapsed().as_millis();
                (res, config_snapshot, call_duration)
            }
            other => (
                Err(RecognitionError::Unavailable {
                    engine: other.to_string(),
                }),
                ModelExecutionConfig {
                    engine: other.to_string(),
                    model_name: target_clone.model_name.clone(),
                    model_filename: target_clone.model_filename.clone(),
                    language: None,
                    task: "transcribe".to_string(),
                    decoding_strategy: "unknown".to_string(),
                    temperature: 0.0,
                    initial_prompt: None,
                    threads: None,
                    backend: "unknown".to_string(),
                },
                0,
            ),
        };

        let t_stt_end = Instant::now();
        (t_stt_start, t_stt_end, recognition_res, config, call_dur)
    })
    .await;

    let (t_stt_start, t_stt_end, recognition_res, config, call_duration_ms) = match stt_task_result {
        Ok(tuple) => tuple,
        Err(e) => {
            tracing::error!("STT join error on {}: {}", target.model_name, e);
            return DictationTestModelResult {
                target_id: target.target_id.clone(),
                engine_id: target.engine_id.clone(),
                model_name: target.model_name.clone(),
                config: ModelExecutionConfig {
                    engine: target.engine_id.clone(),
                    model_name: target.model_name.clone(),
                    model_filename: target.model_filename.clone(),
                    language: None,
                    task: "transcribe".to_string(),
                    decoding_strategy: "failed".to_string(),
                    temperature: 0.0,
                    initial_prompt: None,
                    threads: None,
                    backend: "unknown".to_string(),
                },
                success: false,
                error: Some(e.to_string()),
                raw_transcript: String::new(),
                cleanup_results: BTreeMap::new(),
                timings: DictationModelTimings {
                    recording_to_audio_ready_ms: audio_prep_ms,
                    queue_wait_ms,
                    model_start_ts: format_instant_ts(model_test_start, t_recording_stop, now_system),
                    ..Default::default()
                },
                accuracy: None,
                char_count: 0,
                word_count: 0,
                segment_count: 0,
            };
        }
    };

    let t_text_available = Instant::now();
    let stt_to_text_available_ms = t_text_available.duration_since(t_stt_end).as_millis();

    let (raw_transcript, is_cold, model_load_ms, lock_wait_ms, stt_execution_ms, success, err_msg) =
        match recognition_res {
            Ok(rec) => {
                let exec_ms = rec.timing.decode_ms;
                (
                    rec.text,
                    rec.timing.model_reloaded || rec.timing.model_load_ms > 0,
                    rec.timing.model_load_ms,
                    rec.timing.lock_wait_ms,
                    if exec_ms > 0 {
                        exec_ms
                    } else {
                        t_stt_end.duration_since(t_stt_start).as_millis()
                    },
                    true,
                    None,
                )
            }
            Err(e) => (
                String::new(),
                false,
                0,
                0,
                t_stt_end.duration_since(t_stt_start).as_millis(),
                false,
                Some(e.to_string()),
            ),
        };

    let component_sum = lock_wait_ms + model_load_ms + stt_execution_ms;
    tracing::debug!(
        "Model [{}] timing audit: recognizer_call={}ms vs components(lock={} + load={} + decode={}) = {}ms (delta: {}ms)",
        target.model_name,
        call_duration_ms,
        lock_wait_ms,
        model_load_ms,
        stt_execution_ms,
        component_sum,
        call_duration_ms.abs_diff(component_sum)
    );

    // 2. Cleanup Fan-Out: Run production cleanup rewrite on this model's raw output
    let mut cleanup_results = BTreeMap::new();
    let t_cleanup_all_start = Instant::now();

    // Always provide raw verbatim transcript
    cleanup_results.insert(
        "raw".to_string(),
        CleanupResult {
            style: "raw".to_string(),
            style_display_name: "Raw".to_string(),
            cleaned_text: raw_transcript.clone(),
            changed: false,
            queue_wait_ms: 0,
            duration_ms: 0,
            raw_char_count: raw_transcript.chars().count(),
            raw_word_count: raw_transcript.split_whitespace().count(),
            cleaned_char_count: raw_transcript.chars().count(),
            cleaned_word_count: raw_transcript.split_whitespace().count(),
            diff_spans: vec![DiffSpan::Same(raw_transcript.clone())],
            error: None,
        },
    );

    let non_raw_styles: Vec<CleanupStyle> = cleanup_styles
        .iter()
        .copied()
        .filter(|&s| s != CleanupStyle::Raw)
        .collect();

    if success && !raw_transcript.trim().is_empty() && !non_raw_styles.is_empty() {
        let client = LLMClient::new(settings.provider.clone());

        for &style in &non_raw_styles {
            let t_style_queue_enter = t_cleanup_all_start;
            let t_style_start = Instant::now();
            let style_queue_wait_ms = t_style_start.duration_since(t_style_queue_enter).as_millis();

            let rewrite_fut = rewrite::propose(&client, &raw_transcript, style);
            let (cleaned_text, spans, changed, err_out) = match tokio::time::timeout(Duration::from_secs(8), rewrite_fut).await {
                Ok(proposal) => (
                    proposal.rewritten,
                    proposal.spans,
                    proposal.changed,
                    None,
                ),
                Err(_) => (
                    raw_transcript.clone(),
                    vec![DiffSpan::Same(raw_transcript.clone())],
                    false,
                    Some("Cleanup rewrite timed out after 8s".to_string()),
                ),
            };

            let style_duration_ms = t_style_start.elapsed().as_millis();
            cleanup_results.insert(
                style.as_str().to_string(),
                CleanupResult {
                    style: style.as_str().to_string(),
                    style_display_name: match style {
                        CleanupStyle::Raw => "Raw",
                        CleanupStyle::Faithful => "Faithful",
                        CleanupStyle::Clean => "Clean",
                        CleanupStyle::Professional => "Polished",
                        CleanupStyle::Concise => "Concise",
                    }
                    .to_string(),
                    cleaned_text: cleaned_text.clone(),
                    changed,
                    queue_wait_ms: style_queue_wait_ms,
                    duration_ms: style_duration_ms,
                    raw_char_count: raw_transcript.chars().count(),
                    raw_word_count: raw_transcript.split_whitespace().count(),
                    cleaned_char_count: cleaned_text.chars().count(),
                    cleaned_word_count: cleaned_text.split_whitespace().count(),
                    diff_spans: spans,
                    error: err_out,
                },
            );
        }
    }

    let model_test_end = Instant::now();
    let cleanup_benchmark_total_ms = model_test_end.duration_since(t_cleanup_all_start).as_millis();
    let benchmark_wall_clock_ms = model_test_end.duration_since(model_test_start).as_millis();

    // Compute production E2E for EVERY cleanup style
    let mut production_e2e_by_style = BTreeMap::new();
    for (style_name, res) in &cleanup_results {
        let style_e2e = audio_prep_ms
            + model_load_ms
            + stt_execution_ms
            + stt_to_text_available_ms
            + res.duration_ms;
        production_e2e_by_style.insert(style_name.clone(), style_e2e);
    }

    // Exact selected production cleanup duration lookup
    let prod_cleanup_duration_ms = cleanup_results
        .get(selected_prod_style)
        .map(|c| c.duration_ms)
        .unwrap_or_else(|| {
            cleanup_results
                .get("faithful")
                .or_else(|| cleanup_results.get("raw"))
                .map(|c| c.duration_ms)
                .unwrap_or(0)
        });

    let production_e2e_ms = audio_prep_ms
        + model_load_ms
        + stt_execution_ms
        + stt_to_text_available_ms
        + prod_cleanup_duration_ms;

    let rtf = if audio_duration_seconds > 0.0 {
        (stt_execution_ms as f32 / 1000.0) / audio_duration_seconds
    } else {
        0.0
    };

    // 3. Accuracy Evaluation (if reference transcript provided)
    let accuracy_metrics = if let Some(ref_text) = reference_transcript {
        if !ref_text.trim().is_empty() {
            Some(calculate_accuracy(ref_text, &raw_transcript))
        } else {
            None
        }
    } else {
        None
    };

    let timings = DictationModelTimings {
        recording_to_audio_ready_ms: audio_prep_ms,
        queue_wait_ms,
        model_load_ms,
        is_cold_load: is_cold,
        lock_wait_ms,
        recognizer_call_duration_ms: call_duration_ms,
        stt_execution_ms,
        stt_to_text_available_ms,
        production_cleanup_style: selected_prod_style.to_string(),
        production_cleanup_duration_ms: prod_cleanup_duration_ms,
        production_e2e_ms,
        production_e2e_by_style,
        cleanup_benchmark_total_ms,
        benchmark_wall_clock_ms,
        total_cleanup_ms: cleanup_benchmark_total_ms,
        total_latency_ms: production_e2e_ms,
        rtf,
        timeline_queue_start_ms: t_audio_ready.duration_since(t_recording_stop).as_millis(),
        timeline_model_start_ms: model_test_start.duration_since(t_recording_stop).as_millis(),
        timeline_stt_start_ms: t_stt_start.duration_since(t_recording_stop).as_millis(),
        timeline_stt_end_ms: t_stt_end.duration_since(t_recording_stop).as_millis(),
        timeline_model_end_ms: model_test_end.duration_since(t_recording_stop).as_millis(),
        recording_stop_ts: format_instant_ts(t_recording_stop, t_recording_stop, now_system),
        audio_ready_ts: format_instant_ts(t_audio_ready, t_recording_stop, now_system),
        model_start_ts: format_instant_ts(model_test_start, t_recording_stop, now_system),
        stt_start_ts: format_instant_ts(t_stt_start, t_recording_stop, now_system),
        stt_end_ts: format_instant_ts(t_stt_end, t_recording_stop, now_system),
        text_available_ts: format_instant_ts(t_text_available, t_recording_stop, now_system),
        test_complete_ts: format_instant_ts(model_test_end, t_recording_stop, now_system),
        time_to_first_partial_ms: None,
        time_to_first_stable_partial_ms: None,
        streaming_finalization_ms: None,
    };

    let raw_char_count = raw_transcript.chars().count();
    let raw_word_count = raw_transcript.split_whitespace().count();

    // Print exact, un-conflated DICTATION MODEL TRACE per model
    println!("\n==================================================");
    println!("DICTATION MODEL TRACE — {}", target.model_name);
    println!("--------------------------------------------------");
    println!("model                  : {}", target.engine_display_name);
    println!("model_variant          : {}", target.model_filename);
    println!("language               : {:?}", language_to_use);
    println!("recording_stop         : {}", timings.recording_stop_ts);
    println!("audio_ready            : {}", timings.audio_ready_ts);
    println!("model_start            : {}", timings.model_start_ts);
    println!("stt_start              : {}", timings.stt_start_ts);
    println!("stt_end                : {}", timings.stt_end_ts);
    println!("text_available         : {}", timings.text_available_ts);
    println!("model_test_complete    : {}", timings.test_complete_ts);
    println!("\nProduction-Path Latency Breakdown:");
    println!("recording → audio_ready       : {} ms", timings.recording_to_audio_ready_ms);
    println!("model load                    : {} ms (cold: {})", timings.model_load_ms, timings.is_cold_load);
    println!("recognizer lock wait          : {} ms", timings.lock_wait_ms);
    println!("STT inference execution       : {} ms", timings.stt_execution_ms);
    println!("recognizer transcribe() call  : {} ms", timings.recognizer_call_duration_ms);
    println!("STT → text available          : {} ms", timings.stt_to_text_available_ms);
    println!("{} cleanup             : {} ms", timings.production_cleanup_style, timings.production_cleanup_duration_ms);
    println!("--------------------------------------------------");
    println!("PRODUCTION E2E ESTIMATE       : {} ms", timings.production_e2e_ms);
    println!("Real-Time Factor (RTF)        : {:.3}x", timings.rtf);
    println!("\nBenchmark Orchestration Overhead:");
    println!("queue wait in benchmark       : {} ms", timings.queue_wait_ms);
    println!("all cleanups benchmark time   : {} ms", timings.cleanup_benchmark_total_ms);
    println!("model benchmark wall clock    : {} ms", timings.benchmark_wall_clock_ms);
    if let Some(ref acc) = accuracy_metrics {
        println!("WER (vs reference)            : {:.1}%", acc.wer * 100.0);
        println!("CER (vs reference)            : {:.1}%", acc.cer * 100.0);
    }
    println!("RAW TRANSCRIPT: {}", raw_transcript);
    println!("==================================================\n");

    DictationTestModelResult {
        target_id: target.target_id.clone(),
        engine_id: target.engine_id.clone(),
        model_name: target.model_name.clone(),
        config,
        success,
        error: err_msg,
        raw_transcript,
        cleanup_results,
        timings,
        accuracy: accuracy_metrics,
        char_count: raw_char_count,
        word_count: raw_word_count,
        segment_count: 1,
    }
}

/// Runs the multi-model benchmarking test across all selected models using the identical audio samples.
#[allow(clippy::too_many_arguments)]
pub async fn execute_benchmark_run(
    app: Option<&AppHandle>,
    config_dir: &Path,
    samples: &[f32],
    audio_path: &str,
    t_recording_stop: Instant,
    t_audio_ready: Instant,
    request: RunDictationTestRequest,
    settings: AppSettings,
) -> Result<DictationTestRun, String> {
    let test_id = uuid::Uuid::new_v4().to_string();
    let now_system = SystemTime::now();
    let epoch_ms = now_system
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();

    let audio_duration_seconds = samples.len() as f32 / 16000.0;
    let models_dir = config_dir.join("models");

    // Retain captured audio permanently in benchmark artifacts directory
    let benchmark_audio_dir = config_dir.join("dictation_tests").join("audio");
    fs::create_dir_all(&benchmark_audio_dir).map_err(|e| e.to_string())?;
    let permanent_audio_filename = format!("test_{}_{}.wav", &test_id[..8], epoch_ms);
    let permanent_audio_path = benchmark_audio_dir.join(&permanent_audio_filename);

    if let Err(e) = fs::copy(audio_path, &permanent_audio_path) {
        tracing::warn!("Failed to copy audio to permanent benchmark store: {}", e);
    }

    let audio_metadata = BenchmarkAudioMetadata {
        audio_path: permanent_audio_path.to_string_lossy().to_string(),
        filename: permanent_audio_filename,
        duration_seconds: audio_duration_seconds,
        sample_rate: 16000,
        channels: 1,
        format: "wav_pcm16".to_string(),
        sample_count: samples.len(),
    };

    let env_metadata = BenchmarkEnvironmentMetadata {
        os: std::env::consts::OS.to_string(),
        vox_version: env!("CARGO_PKG_VERSION").to_string(),
        platform: std::env::consts::ARCH.to_string(),
        cleanup_provider: settings.provider.active_provider.slug().to_string(),
        cleanup_model: match settings.provider.active_provider {
            ProviderType::Ollama => settings.provider.ollama_model.clone(),
            _ => settings.provider.cloud_model.clone().unwrap_or_default(),
        },
    };

    let all_available = discover_available_models(&models_dir);
    let targets_to_run: Vec<AvailableModelTarget> = all_available
        .into_iter()
        .filter(|m| request.selected_target_ids.contains(&m.target_id) && m.installed && m.compiled_in)
        .collect();

    if targets_to_run.is_empty() {
        return Err("No installed, compiled-in models selected for benchmarking.".to_string());
    }

    // Parse requested cleanup styles
    let cleanup_styles: Vec<CleanupStyle> = if request.selected_cleanup_styles.is_empty() {
        vec![CleanupStyle::Raw, CleanupStyle::Faithful]
    } else {
        request
            .selected_cleanup_styles
            .iter()
            .map(|s| CleanupStyle::from_setting(s))
            .collect()
    };

    let mut model_results = Vec::new();
    let total_models = targets_to_run.len();
    let run_start_instant = Instant::now();

    let selected_prod_style = request
        .production_cleanup_style
        .as_deref()
        .unwrap_or("faithful")
        .to_lowercase();

    let language_to_use = request
        .language_override
        .clone()
        .or_else(|| settings.language.spoken_languages.first().cloned());

    let concurrency = request.concurrency.unwrap_or(1).max(1);

    if concurrency <= 1 {
        for (index, target) in targets_to_run.iter().enumerate() {
            let model_test_start = Instant::now();
            let model_result = execute_single_model(
                target,
                samples,
                audio_duration_seconds,
                t_recording_stop,
                t_audio_ready,
                model_test_start,
                language_to_use.clone(),
                &cleanup_styles,
                &selected_prod_style,
                request.reference_transcript.as_deref(),
                &settings,
                now_system,
            )
            .await;

            let progress_payload = DictationTestProgressPayload {
                test_id: test_id.clone(),
                current_model_index: index + 1,
                total_models,
                completed_model: model_result.clone(),
            };
            if let Some(app) = app {
                let _ = app.emit(DICTATION_TEST_PROGRESS_EVENT, &progress_payload);
            }

            model_results.push(model_result);
        }
    } else {
        let semaphore = std::sync::Arc::new(tokio::sync::Semaphore::new(concurrency));
        let mut set = tokio::task::JoinSet::new();

        for (index, target) in targets_to_run.iter().enumerate() {
            let sem = semaphore.clone();
            let target = target.clone();
            let samples_vec = samples.to_vec();
            let lang_opt = language_to_use.clone();
            let styles_vec = cleanup_styles.clone();
            let prod_style = selected_prod_style.clone();
            let ref_transcript = request.reference_transcript.clone();
            let app_settings = settings.clone();

            set.spawn(async move {
                let _permit = sem.acquire().await.expect("semaphore acquired");
                let model_test_start = Instant::now();
                let res = execute_single_model(
                    &target,
                    &samples_vec,
                    audio_duration_seconds,
                    t_recording_stop,
                    t_audio_ready,
                    model_test_start,
                    lang_opt,
                    &styles_vec,
                    &prod_style,
                    ref_transcript.as_deref(),
                    &app_settings,
                    now_system,
                )
                .await;
                (index, res)
            });
        }

        let mut completed_items = Vec::new();
        while let Some(res) = set.join_next().await {
            if let Ok((idx, model_res)) = res {
                completed_items.push((idx, model_res));
            }
        }
        completed_items.sort_by_key(|(idx, _)| *idx);
        for (idx, res) in completed_items {
            let progress_payload = DictationTestProgressPayload {
                test_id: test_id.clone(),
                current_model_index: idx + 1,
                total_models,
                completed_model: res.clone(),
            };
            if let Some(app) = app {
                let _ = app.emit(DICTATION_TEST_PROGRESS_EVENT, &progress_payload);
            }
            model_results.push(res);
        }
    }

    let total_run_duration_ms = run_start_instant.elapsed().as_millis();

    let test_run = DictationTestRun {
        schema_version: TEST_RUN_SCHEMA_VERSION,
        test_id: test_id.clone(),
        label: request.test_label,
        created_at_epoch_ms: epoch_ms,
        created_at_formatted: chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string(),
        audio: audio_metadata,
        environment: env_metadata,
        reference_transcript: request.reference_transcript,
        selected_models: request.selected_target_ids,
        selected_cleanup_styles: cleanup_styles.iter().map(|s| s.as_str().to_string()).collect(),
        model_results,
        total_run_duration_ms,
    };

    // Persist to test history
    if let Err(e) = save_dictation_test_run(config_dir, &test_run) {
        tracing::warn!("Failed to persist dictation test run: {}", e);
    }

    Ok(test_run)
}

// ---------------------------------------------------------------------------
// Storage & Persistence
// ---------------------------------------------------------------------------

fn test_runs_dir(config_dir: &Path) -> PathBuf {
    config_dir.join("dictation_tests").join("runs")
}

pub fn save_dictation_test_run(config_dir: &Path, run: &DictationTestRun) -> Result<(), String> {
    let dir = test_runs_dir(config_dir);
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let file_path = dir.join(format!("{}.json", run.test_id));
    let json = serde_json::to_string_pretty(run).map_err(|e| e.to_string())?;
    fs::write(file_path, json).map_err(|e| e.to_string())?;
    Ok(())
}

pub fn load_dictation_test_run(config_dir: &Path, test_id: &str) -> Result<Option<DictationTestRun>, String> {
    let file_path = test_runs_dir(config_dir).join(format!("{}.json", test_id));
    if !file_path.is_file() {
        return Ok(None);
    }
    let content = fs::read_to_string(file_path).map_err(|e| e.to_string())?;
    let run: DictationTestRun = serde_json::from_str(&content).map_err(|e| e.to_string())?;
    Ok(Some(run))
}

pub fn list_dictation_test_history(config_dir: &Path) -> Result<Vec<DictationTestSummary>, String> {
    let dir = test_runs_dir(config_dir);
    if !dir.is_dir() {
        return Ok(Vec::new());
    }

    let mut summaries = Vec::new();
    let entries = fs::read_dir(dir).map_err(|e| e.to_string())?;

    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }

        if let Ok(content) = fs::read_to_string(&path) {
            if let Ok(run) = serde_json::from_str::<DictationTestRun>(&content) {
                let successful_count = run.model_results.iter().filter(|r| r.success).count();
                let fastest = run
                    .model_results
                    .iter()
                    .filter(|r| r.success)
                    .min_by_key(|r| {
                        if r.timings.production_e2e_ms > 0 {
                            r.timings.production_e2e_ms
                        } else {
                            r.timings.total_latency_ms
                        }
                    });

                let fastest_ms = fastest.map(|f| {
                    if f.timings.production_e2e_ms > 0 {
                        f.timings.production_e2e_ms
                    } else {
                        f.timings.total_latency_ms
                    }
                });

                summaries.push(DictationTestSummary {
                    test_id: run.test_id,
                    created_at_epoch_ms: run.created_at_epoch_ms,
                    created_at_formatted: run.created_at_formatted,
                    label: run.label,
                    audio_duration_seconds: run.audio.duration_seconds,
                    models_tested_count: run.model_results.len(),
                    successful_models_count: successful_count,
                    fastest_model_name: fastest.map(|f| f.model_name.clone()),
                    fastest_total_ms: fastest_ms,
                    cleanup_styles: run.selected_cleanup_styles,
                    has_reference_transcript: run.reference_transcript.as_ref().is_some_and(|r| !r.trim().is_empty()),
                });
            }
        }
    }

    // Newest first
    summaries.sort_by_key(|a| std::cmp::Reverse(a.created_at_epoch_ms));
    Ok(summaries)
}

pub fn delete_dictation_test_run(config_dir: &Path, test_id: &str) -> Result<bool, String> {
    let file_path = test_runs_dir(config_dir).join(format!("{}.json", test_id));
    if file_path.is_file() {
        fs::remove_file(file_path).map_err(|e| e.to_string())?;
        Ok(true)
    } else {
        Ok(false)
    }
}

// ---------------------------------------------------------------------------
// Export Formatting
// ---------------------------------------------------------------------------

pub fn format_markdown_report(run: &DictationTestRun) -> String {
    let mut md = String::new();
    md.push_str("# Dictation Test Report\n\n");
    md.push_str(&format!("- **Test ID:** `{}`\n", run.test_id));
    md.push_str(&format!("- **Date:** {}\n", run.created_at_formatted));
    if let Some(ref label) = run.label {
        md.push_str(&format!("- **Label:** {}\n", label));
    }
    md.push_str(&format!("- **Audio Duration:** {:.2} sec\n", run.audio.duration_seconds));
    md.push_str(&format!("- **Vox Version:** {}\n", run.environment.vox_version));
    md.push_str(&format!("- **OS:** {} ({})\n", run.environment.os, run.environment.platform));
    md.push_str(&format!("- **Cleanup Provider / Model:** {} / {}\n\n", run.environment.cleanup_provider, run.environment.cleanup_model));

    if let Some(ref ref_text) = run.reference_transcript {
        md.push_str("## Reference Transcript\n\n");
        md.push_str(&format!("> {}\n\n", ref_text));
        md.push_str("*Note: WER and CER accuracy are computed against this ground-truth reference using word-level Levenshtein alignment (case and punctuation normalized).*\n\n");
    }

    md.push_str("## Summary Comparison\n\n");
    md.push_str("| Model | Model Load (ms) | Lock Wait (ms) | Queue (ms) | STT (ms) | Faithful (ms) | Production E2E (ms) | RTF | WER | Status |\n");
    md.push_str("| :--- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | :---: |\n");

    for m in &run.model_results {
        let wer_str = m.accuracy.as_ref().map_or("—".to_string(), |a| format!("{:.1}%", a.wer * 100.0));
        let status = if m.success { "✓" } else { "✕" };
        let faithful_ms = m.cleanup_results.get("faithful").map(|c| c.duration_ms).unwrap_or(m.timings.production_cleanup_duration_ms);
        md.push_str(&format!(
            "| **{}** | {} | {} | {} | {} | {} | {} | {:.2}x | {} | {} |\n",
            m.model_name,
            m.timings.model_load_ms,
            m.timings.lock_wait_ms,
            m.timings.queue_wait_ms,
            m.timings.stt_execution_ms,
            faithful_ms,
            m.timings.production_e2e_ms,
            m.timings.rtf,
            wer_str,
            status
        ));
    }
    md.push_str("\n---\n\n");

    for m in &run.model_results {
        md.push_str(&format!("## {}\n\n", m.model_name));
        md.push_str("### Configuration\n\n");
        md.push_str(&format!("- **Engine:** {}\n", m.config.engine));
        md.push_str(&format!("- **Model:** {}\n", m.config.model_filename));
        md.push_str(&format!("- **Language:** {}\n", m.config.language.as_deref().unwrap_or("Auto")));
        md.push_str(&format!("- **Backend:** {}\n", m.config.backend));
        md.push_str(&format!("- **Decoding Strategy:** {}\n\n", m.config.decoding_strategy));

        md.push_str("### Production-Path Latency Breakdown\n\n");
        md.push_str("| Component | Duration |\n");
        md.push_str("| :--- | ---: |\n");
        md.push_str(&format!("| Audio Preparation | {} ms |\n", m.timings.recording_to_audio_ready_ms));
        md.push_str(&format!("| Model Load (cold: {}) | {} ms |\n", m.timings.is_cold_load, m.timings.model_load_ms));
        md.push_str(&format!("| STT Inference Execution | {} ms |\n", m.timings.stt_execution_ms));
        md.push_str(&format!("| STT → Text Available | {} ms |\n", m.timings.stt_to_text_available_ms));
        md.push_str(&format!("| Selected Cleanup ({}) | {} ms |\n", m.timings.production_cleanup_style, m.timings.production_cleanup_duration_ms));
        md.push_str(&format!("| **Production E2E Estimate** | **{} ms** |\n\n", m.timings.production_e2e_ms));

        md.push_str("### Benchmark Orchestration Overhead\n\n");
        md.push_str(&format!("- **Benchmark Queue Wait:** {} ms\n", m.timings.queue_wait_ms));
        md.push_str(&format!("- **All Cleanups Benchmark Time:** {} ms\n", m.timings.cleanup_benchmark_total_ms));
        md.push_str(&format!("- **Model Benchmark Wall Clock:** {} ms\n", m.timings.benchmark_wall_clock_ms));
        md.push_str(&format!("- **Real-Time Factor (RTF):** {:.3}x\n\n", m.timings.rtf));

        md.push_str("### Raw Speech Recognition Output\n\n");
        if m.raw_transcript.is_empty() {
            md.push_str("*(Empty / No speech detected)*\n\n");
        } else {
            md.push_str(&format!("```\n{}\n```\n\n", m.raw_transcript));
        }

        if !m.cleanup_results.is_empty() {
            md.push_str("### Cleanup Benchmark Results\n\n");
            for clean in m.cleanup_results.values() {
                md.push_str(&format!("#### {}\n\n", clean.style_display_name));
                md.push_str(&format!("- **Execution Duration:** {} ms\n", clean.duration_ms));
                md.push_str(&format!("- **Queue Wait:** {} ms\n", clean.queue_wait_ms));
                md.push_str(&format!("- **Changed Text:** {}\n\n", clean.changed));
                md.push_str(&format!("```\n{}\n```\n\n", clean.cleaned_text));
            }
        }

        if !m.timings.production_e2e_by_style.is_empty() {
            md.push_str("### Production Paths Comparison\n\n");
            md.push_str("| Cleanup Style | Cleanup Duration (ms) | Production E2E (ms) |\n");
            md.push_str("| :--- | ---: | ---: |\n");
            for (style, e2e) in &m.timings.production_e2e_by_style {
                let cleanup_dur = m.cleanup_results.get(style).map(|c| c.duration_ms).unwrap_or(0);
                md.push_str(&format!("| **{}** | {} ms | **{} ms** |\n", style, cleanup_dur, e2e));
            }
            md.push('\n');
        }

        if let Some(ref acc) = m.accuracy {
            md.push_str("### Accuracy Metrics (vs Ground Truth Reference)\n\n");
            md.push_str(&format!("- **Word Error Rate (WER):** {:.1}%\n", acc.wer * 100.0));
            md.push_str(&format!("- **Character Error Rate (CER):** {:.1}%\n", acc.cer * 100.0));
            md.push_str(&format!("- **Substitutions:** {}\n", acc.substitutions));
            md.push_str(&format!("- **Deletions:** {}\n", acc.deletions));
            md.push_str(&format!("- **Insertions:** {}\n\n", acc.insertions));
        }

        md.push_str("---\n\n");
    }

    md
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn discover_models_lists_all_installed_or_known() {
        let temp_dir = std::env::temp_dir().join("vox_test_models_discover");
        let _ = fs::create_dir_all(&temp_dir);
        let models = discover_available_models(&temp_dir);
        assert!(!models.is_empty(), "Should discover Whisper catalogue entries and Parakeet");
        assert!(models.iter().any(|m| m.engine_id == "whisper"));
        assert!(models.iter().any(|m| m.engine_id == "parakeet"));
    }

    #[test]
    fn get_cleanup_styles_returns_expected_variants() {
        let styles = get_available_cleanup_styles();
        let ids: Vec<String> = styles.into_iter().map(|s| s.id).collect();
        assert!(ids.contains(&"raw".to_string()));
        assert!(ids.contains(&"faithful".to_string()));
        assert!(ids.contains(&"clean".to_string()));
        assert!(ids.contains(&"polished".to_string()));
        assert!(ids.contains(&"concise".to_string()));
    }

    #[test]
    fn markdown_report_formats_valid_document() {
        let run = DictationTestRun {
            schema_version: 1,
            test_id: "test-123".to_string(),
            label: Some("Baseline Test".to_string()),
            created_at_epoch_ms: 1700000000000,
            created_at_formatted: "2026-09-21 16:00:00".to_string(),
            audio: BenchmarkAudioMetadata {
                audio_path: "/dummy/path.wav".to_string(),
                filename: "path.wav".to_string(),
                duration_seconds: 5.0,
                sample_rate: 16000,
                channels: 1,
                format: "wav".to_string(),
                sample_count: 80000,
            },
            environment: BenchmarkEnvironmentMetadata {
                os: "windows".to_string(),
                vox_version: "0.1.0".to_string(),
                platform: "x86_64".to_string(),
                cleanup_provider: "ollama".to_string(),
                cleanup_model: "llama3.2".to_string(),
            },
            reference_transcript: Some("Hello world".to_string()),
            selected_models: vec!["whisper:ggml-small.bin".to_string()],
            selected_cleanup_styles: vec!["raw".to_string(), "faithful".to_string()],
            model_results: vec![DictationTestModelResult {
                target_id: "whisper:ggml-small.bin".to_string(),
                engine_id: "whisper".to_string(),
                model_name: "Whisper Small".to_string(),
                config: ModelExecutionConfig {
                    engine: "Whisper".to_string(),
                    model_name: "Whisper Small".to_string(),
                    model_filename: "ggml-small.bin".to_string(),
                    language: Some("en".to_string()),
                    task: "transcribe".to_string(),
                    decoding_strategy: "greedy".to_string(),
                    temperature: 0.0,
                    initial_prompt: None,
                    threads: None,
                    backend: "whisper.cpp".to_string(),
                },
                success: true,
                error: None,
                raw_transcript: "Hello world".to_string(),
                cleanup_results: BTreeMap::new(),
                timings: DictationModelTimings {
                    recording_to_audio_ready_ms: 10,
                    queue_wait_ms: 0,
                    model_load_ms: 0,
                    is_cold_load: false,
                    stt_execution_ms: 1500,
                    stt_to_text_available_ms: 5,
                    production_cleanup_style: "faithful".to_string(),
                    production_cleanup_duration_ms: 500,
                    production_e2e_ms: 2015,
                    cleanup_benchmark_total_ms: 500,
                    benchmark_wall_clock_ms: 2005,
                    total_cleanup_ms: 500,
                    total_latency_ms: 2015,
                    rtf: 0.3,
                    timeline_queue_start_ms: 10,
                    timeline_model_start_ms: 10,
                    timeline_stt_start_ms: 10,
                    timeline_stt_end_ms: 1510,
                    timeline_model_end_ms: 2015,
                    recording_stop_ts: "16:00:00.000".to_string(),
                    audio_ready_ts: "16:00:00.010".to_string(),
                    model_start_ts: "16:00:00.010".to_string(),
                    stt_start_ts: "16:00:00.010".to_string(),
                    stt_end_ts: "16:00:01.510".to_string(),
                    text_available_ts: "16:00:01.515".to_string(),
                    test_complete_ts: "16:00:02.015".to_string(),
                    time_to_first_partial_ms: None,
                    time_to_first_stable_partial_ms: None,
                    streaming_finalization_ms: None,
                    lock_wait_ms: 0,
                    recognizer_call_duration_ms: 1500,
                    production_e2e_by_style: BTreeMap::new(),
                },
                accuracy: Some(AccuracyMetrics {
                    reference: "Hello world".to_string(),
                    hypothesis: "Hello world".to_string(),
                    word_count: 2,
                    substitutions: 0,
                    deletions: 0,
                    insertions: 0,
                    wer: 0.0,
                    cer: 0.0,
                    technical_term_accuracy: None,
                    technical_term_errors: Vec::new(),
                }),
                char_count: 11,
                word_count: 2,
                segment_count: 1,
            }],
            total_run_duration_ms: 2015,
        };

        let md = format_markdown_report(&run);
        assert!(md.contains("# Dictation Test Report"));
        assert!(md.contains("Whisper Small"));
        assert!(md.contains("WER"));
        assert!(md.contains("0.0%"));
    }

    #[tokio::test]
    #[ignore]
    async fn run_baseline_test_2() {
        let config_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join(".vox").join("config");
        let audio_path = config_dir.join("dictation_tests").join("audio").join("test_de7dc5da_1790011172436.wav");
        if !audio_path.exists() {
            panic!("Audio file not found at {:?}", audio_path);
        }

        let mut reader = hound::WavReader::open(&audio_path).unwrap();
        let raw_samples: Vec<f32> = reader
            .samples::<i16>()
            .filter_map(|s| s.ok())
            .map(|s| s as f32 / i16::MAX as f32)
            .collect();

        let settings_raw = std::fs::read_to_string(config_dir.join("settings.json")).unwrap();
        let settings: AppSettings = serde_json::from_str(&settings_raw).unwrap();

        let request = RunDictationTestRequest {
            test_label: Some("English Baseline Test 2".to_string()),
            reference_transcript: Some(
                "Today I want to review the performance of our voice transcription system, especially its accuracy, latency, and ability to preserve the meaning of what I actually said. The goal is to make dictation feel almost instantaneous while ensuring that cleanup improves readability without changing the original intent or introducing information that was never spoken.".to_string()
            ),
            selected_target_ids: vec![
                "whisper:ggml-base.bin".to_string(),
                "whisper:ggml-small.bin".to_string(),
                "whisper:ggml-large-v3-turbo.bin".to_string(),
                "whisper:ggml-hindi2hinglish-apex-q5_0.bin".to_string(),
                "whisper:ggml-large-v3-turbo-q5_0.bin".to_string(),
            ],
            selected_cleanup_styles: vec![
                "raw".to_string(),
                "faithful".to_string(),
                "clean".to_string(),
                "polished".to_string(),
                "concise".to_string(),
            ],
            production_cleanup_style: Some("faithful".to_string()),
            language_override: None,
            concurrency: Some(1),
        };

        let t_now = Instant::now();
        let run = execute_benchmark_run(
            None,
            &config_dir,
            &raw_samples,
            audio_path.to_str().unwrap(),
            t_now,
            t_now,
            request,
            settings,
        )
        .await
        .expect("Benchmark run failed");

        println!("\n=== BENCHMARK COMPLETED: {} ===", run.label.as_deref().unwrap_or("Untitled"));
        println!("Test ID: {}", run.test_id);
        println!("Total Run Duration: {} ms\n", run.total_run_duration_ms);

        println!("{:<25} | {:>10} | {:>8} | {:>8} | {:>10} | {:>15} | {:>6} | {:>5}",
            "Model", "Model Load", "Queue", "STT", "Faithful", "Production E2E", "RTF", "WER");
        println!("{}", "-".repeat(105));

        for m in &run.model_results {
            let wer_str = m.accuracy.as_ref().map(|a| format!("{:.1}%", a.wer * 100.0)).unwrap_or_else(|| "N/A".to_string());
            let faithful_dur = m.cleanup_results.get("faithful").map(|c| format!("{}ms", c.duration_ms)).unwrap_or_else(|| "N/A".to_string());
            println!("{:<25} | {:>8}ms | {:>6}ms | {:>6}ms | {:>10} | {:>13}ms | {:>5.2}x | {:>5}",
                m.model_name,
                m.timings.model_load_ms,
                m.timings.queue_wait_ms,
                m.timings.stt_execution_ms,
                faithful_dur,
                m.timings.production_e2e_ms,
                m.timings.rtf,
                wer_str
            );
        }
    }

    #[tokio::test]
    #[ignore]
    async fn run_phase2_diagnostics() {
        let config_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join(".vox").join("config");
        let audio_path = config_dir.join("dictation_tests").join("audio").join("test_7aec643b_1790015082009.wav");
        if !audio_path.exists() {
            panic!("Audio file not found at {:?}", audio_path);
        }

        let mut reader = hound::WavReader::open(&audio_path).unwrap();
        let raw_samples: Vec<f32> = reader
            .samples::<i16>()
            .filter_map(|s| s.ok())
            .map(|s| s as f32 / i16::MAX as f32)
            .collect();

        let settings_raw = std::fs::read_to_string(config_dir.join("settings.json")).unwrap();
        let settings: AppSettings = serde_json::from_str(&settings_raw).unwrap();

        let model_targets = vec![
            ("whisper:ggml-base.bin", "Whisper Base"),
            ("whisper:ggml-small.bin", "Whisper Small"),
            ("whisper:ggml-large-v3-turbo.bin", "Whisper Large v3 Turbo"),
            ("whisper:ggml-hindi2hinglish-apex-q5_0.bin", "Hindi/Hinglish Q5"),
            ("whisper:ggml-large-v3-turbo-q5_0.bin", "Large v3 Turbo Q5"),
        ];

        println!("\n=========================================================================================");
        println!("PHASE 2 DIAGNOSTICS: 1. SINGLE MODEL ISOLATED (STT ONLY, NO OTHER MODEL, NO CLEANUP)");
        println!("=========================================================================================");
        println!("{:<25} | {:>10} | {:>9} | {:>10} | {:>12} | {:>8} | {:>6}",
            "Model", "Load (ms)", "Lock (ms)", "Decode(ms)", "CallDur(ms)", "Total(ms)", "RTF");
        println!("{}", "-".repeat(95));

        let mut isolated_decodes: BTreeMap<String, u128> = BTreeMap::new();
        let mut isolated_queues: BTreeMap<String, u128> = BTreeMap::new();

        for (target_id, model_name) in &model_targets {
            let req = RunDictationTestRequest {
                test_label: Some(format!("Isolated STT - {}", model_name)),
                reference_transcript: None,
                selected_target_ids: vec![target_id.to_string()],
                selected_cleanup_styles: vec!["raw".to_string()],
                production_cleanup_style: Some("raw".to_string()),
                language_override: Some("en".to_string()),
                concurrency: Some(1),
            };

            let t_now = Instant::now();
            let run = execute_benchmark_run(
                None,
                &config_dir,
                &raw_samples,
                audio_path.to_str().unwrap(),
                t_now,
                t_now,
                req,
                settings.clone(),
            )
            .await
            .unwrap();

            let m = &run.model_results[0];
            isolated_decodes.insert(target_id.to_string(), m.timings.stt_execution_ms);
            isolated_queues.insert(target_id.to_string(), m.timings.queue_wait_ms);

            println!("{:<25} | {:>10} | {:>9} | {:>10} | {:>12} | {:>8} | {:>6.3}x",
                model_name,
                m.timings.model_load_ms,
                m.timings.lock_wait_ms,
                m.timings.stt_execution_ms,
                m.timings.recognizer_call_duration_ms,
                run.total_run_duration_ms,
                m.timings.rtf
            );
        }

        println!("\n=========================================================================================");
        println!("PHASE 2 DIAGNOSTICS: 2. MULTI-MODEL STT ONLY (SEQUENTIAL, NO CLEANUP)");
        println!("=========================================================================================");
        let multi_stt_req = RunDictationTestRequest {
            test_label: Some("English Baseline test 4 — STT Isolation".to_string()),
            reference_transcript: None,
            selected_target_ids: model_targets.iter().map(|(id, _)| id.to_string()).collect(),
            selected_cleanup_styles: vec!["raw".to_string()],
            production_cleanup_style: Some("raw".to_string()),
            language_override: Some("en".to_string()),
            concurrency: Some(1),
        };

        let t_now = Instant::now();
        let multi_run = execute_benchmark_run(
            None,
            &config_dir,
            &raw_samples,
            audio_path.to_str().unwrap(),
            t_now,
            t_now,
            multi_stt_req,
            settings.clone(),
        )
        .await
        .unwrap();

        println!("{:<25} | {:>10} | {:>9} | {:>10} | {:>10} | {:>12} | {:>6}",
            "Model", "Queue(ms)", "Lock(ms)", "Load(ms)", "Decode(ms)", "WallClock(ms)", "RTF");
        println!("{}", "-".repeat(95));
        for m in &multi_run.model_results {
            println!("{:<25} | {:>10} | {:>9} | {:>10} | {:>10} | {:>12} | {:>6.3}x",
                m.model_name,
                m.timings.queue_wait_ms,
                m.timings.lock_wait_ms,
                m.timings.model_load_ms,
                m.timings.stt_execution_ms,
                m.timings.benchmark_wall_clock_ms,
                m.timings.rtf
            );
        }
        println!("Total Multi-Model STT Wall Clock: {} ms", multi_run.total_run_duration_ms);

        println!("\n=========================================================================================");
        println!("PHASE 2 DIAGNOSTICS: 3. ISOLATED VS MULTI-MODEL COMPARISON");
        println!("=========================================================================================");
        println!("{:<25} | {:>16} | {:>18} | {:>15} | {:>18}",
            "Model", "Isolated Decode", "Multi-Model Decode", "Isolated Queue", "Multi-Model Queue");
        println!("{}", "-".repeat(105));
        for (target_id, model_name) in &model_targets {
            let iso_dec = isolated_decodes.get(*target_id).copied().unwrap_or(0);
            let iso_q = isolated_queues.get(*target_id).copied().unwrap_or(0);
            let multi_m = multi_run.model_results.iter().find(|m| m.target_id == *target_id).unwrap();
            println!("{:<25} | {:>13} ms | {:>15} ms | {:>12} ms | {:>15} ms",
                model_name,
                iso_dec,
                multi_m.timings.stt_execution_ms,
                iso_q,
                multi_m.timings.queue_wait_ms
            );
        }

        println!("\n=========================================================================================");
        println!("PHASE 2 DIAGNOSTICS: 4. CONCURRENCY EXPERIMENT (Concurrency = 2 vs Concurrency = 5)");
        println!("=========================================================================================");
        for c in [2, 5] {
            let conc_req = RunDictationTestRequest {
                test_label: Some(format!("Concurrency = {} (STT Only)", c)),
                reference_transcript: None,
                selected_target_ids: model_targets.iter().map(|(id, _)| id.to_string()).collect(),
                selected_cleanup_styles: vec!["raw".to_string()],
                production_cleanup_style: Some("raw".to_string()),
                language_override: Some("en".to_string()),
                concurrency: Some(c),
            };

            let t_now = Instant::now();
            let c_run = execute_benchmark_run(
                None,
                &config_dir,
                &raw_samples,
                audio_path.to_str().unwrap(),
                t_now,
                t_now,
                conc_req,
                settings.clone(),
            )
            .await
            .unwrap();

            println!("\n--- Concurrency = {} (Wall Clock: {} ms) ---", c, c_run.total_run_duration_ms);
            println!("{:<25} | {:>10} | {:>10} | {:>10} | {:>12} | {:>6}",
                "Model", "Queue(ms)", "Load(ms)", "Decode(ms)", "WallClock(ms)", "RTF");
            for m in &c_run.model_results {
                println!("{:<25} | {:>10} | {:>10} | {:>10} | {:>12} | {:>6.3}x",
                    m.model_name,
                    m.timings.queue_wait_ms,
                    m.timings.model_load_ms,
                    m.timings.stt_execution_ms,
                    m.timings.benchmark_wall_clock_ms,
                    m.timings.rtf
                );
            }
        }
    }

    #[tokio::test]
    #[ignore]
    async fn run_phase3_diagnostics() {
        let config_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join(".vox").join("config");
        let audio_path = config_dir.join("dictation_tests").join("audio").join("test_7aec643b_1790015082009.wav");
        if !audio_path.exists() {
            panic!("Audio file not found at {:?}", audio_path);
        }

        let mut reader = hound::WavReader::open(&audio_path).unwrap();
        let raw_samples: Vec<f32> = reader
            .samples::<i16>()
            .filter_map(|s| s.ok())
            .map(|s| s as f32 / i16::MAX as f32)
            .collect();

        let total_samples = raw_samples.len();
        let total_seconds = total_samples as f64 / 16000.0;

        let test_models = vec![
            (config_dir.join("models").join("ggml-base.bin"), "Whisper Base"),
            (config_dir.join("models").join("ggml-small.bin"), "Whisper Small"),
        ];

        let test_windows = vec![2.0, 4.0, 6.0, 8.0, 10.0, 15.0, 20.0, 30.0, total_seconds];

        println!("\n=========================================================================================");
        println!("PHASE 3 DIAGNOSTICS: STREAMING SIMULATION & INCREMENTAL DECODE");
        println!("Audio: {:.2} seconds ({} samples at 16 kHz)", total_seconds, total_samples);
        println!("=========================================================================================");

        for (target_id, model_name) in &test_models {
            println!("\n-----------------------------------------------------------------------------------------");
            println!("MODEL: {}", model_name);
            println!("-----------------------------------------------------------------------------------------");
            println!("{:<12} | {:>10} | {:>11} | {:>8} | Transcript",
                "Window (s)", "Samples", "Decode (ms)", "RTF");
            println!("{}", "-".repeat(110));

            let recognizer = WhisperRecognizer::new(SttEngine::new(), target_id, WhisperDecodingConfig::default());
            let mut previous_transcript = String::new();
            let mut cumulative_decode_ms = 0u128;
            let mut first_useful_partial: Option<(f64, String)> = None;
            let mut stable_prefix = String::new();
            let mut final_single_decode_ms = 0u128;

            for &window_sec in &test_windows {
                let sample_count = ((window_sec * 16000.0).round() as usize).min(total_samples);
                let window_samples = &raw_samples[0..sample_count];

                let t_decode_start = Instant::now();
                let recognition = recognizer
                    .transcribe(RecognitionRequest {
                        samples: window_samples,
                        language: Some("en".to_string()),
                        translate: false,
                    })
                    .expect("transcribe failed");
                let decode_ms = t_decode_start.elapsed().as_millis();
                cumulative_decode_ms += decode_ms;
                if (window_sec - total_seconds).abs() < 0.1 {
                    final_single_decode_ms = decode_ms;
                }

                let rtf = if window_sec > 0.0 {
                    (decode_ms as f32 / 1000.0) / window_sec as f32
                } else {
                    0.0
                };

                let text = recognition.text.trim().to_string();

                if first_useful_partial.is_none() && !text.is_empty() {
                    first_useful_partial = Some((window_sec, text.clone()));
                }

                // Compute common prefix with previous
                let current_words: Vec<&str> = text.split_whitespace().collect();
                let prev_words: Vec<&str> = previous_transcript.split_whitespace().collect();
                let mut common_words = Vec::new();
                for (w1, w2) in prev_words.iter().zip(current_words.iter()) {
                    if w1 == w2 {
                        common_words.push(*w1);
                    } else {
                        break;
                    }
                }
                if !common_words.is_empty() {
                    stable_prefix = common_words.join(" ");
                }

                println!("{:<12.1} | {:>10} | {:>11} | {:>7.3}x | {}",
                    window_sec, sample_count, decode_ms, rtf, text);

                previous_transcript = text;
            }

            println!("\nSUMMARY FOR {}:", model_name);
            println!("  First useful partial at: {:.1}s -> {:?}",
                first_useful_partial.as_ref().map(|p| p.0).unwrap_or(0.0),
                first_useful_partial.as_ref().map(|p| &p.1).unwrap_or(&"".to_string())
            );
            println!("  Final stable prefix: {:?}", stable_prefix);
            println!("  Cumulative repeated-decode time : {} ms", cumulative_decode_ms);
            println!("  Single final decode time        : {} ms", final_single_decode_ms);
            if final_single_decode_ms > 0 {
                println!("  Repeated decode overhead factor : {:.2}x",
                    cumulative_decode_ms as f64 / final_single_decode_ms as f64);
            }
        }
    }

    #[tokio::test]
    #[ignore]
    async fn run_phase4_diagnostics() {
        use std::sync::Arc;
        use crate::capture::eligibility::{evaluate_cleanup_eligibility, CleanupEligibility};
        use crate::meetings::segmenter::{SegmentCloseReason, StreamingSegmenter};

        let config_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join(".vox").join("config");
        let audio_path = config_dir
            .join("dictation_tests")
            .join("audio")
            .join("test_7aec643b_1790015082009.wav");
        if !audio_path.exists() {
            panic!("Audio file not found at {:?}", audio_path);
        }

        let mut reader = hound::WavReader::open(&audio_path).unwrap();
        let raw_samples: Vec<f32> = reader
            .samples::<i16>()
            .filter_map(|s| s.ok())
            .map(|s| s as f32 / i16::MAX as f32)
            .collect();

        let total_samples = raw_samples.len();
        let total_seconds = total_samples as f64 / 16000.0;

        let settings_raw = std::fs::read_to_string(config_dir.join("settings.json")).unwrap();
        let settings: AppSettings = serde_json::from_str(&settings_raw).unwrap();

        let small_model_path = config_dir.join("models").join("ggml-small.bin");
        let base_model_path = config_dir.join("models").join("ggml-base.bin");

        let engine = SttEngine::new();
        let client = Arc::new(LLMClient::new(settings.provider.clone()));
        let decoding_config = WhisperDecodingConfig::for_dictation(&settings.stt);
        let language_config = crate::capture::SttLanguageConfig::from_settings(
            &settings.language,
            crate::capture::stt::SttWindow::ShortForm,
        );

        println!("\n=========================================================================================");
        println!("PHASE 4 SHADOW STREAMING PIPELINE: EMPIRICAL DIAGNOSTICS & BENCHMARK HARNESS");
        println!("Audio Artifact: test_7aec643b_1790015082009.wav ({:.2}s, {} samples at 16 kHz)", total_seconds, total_samples);
        println!("Production STT Model: Whisper Small ({})", small_model_path.display());
        println!("Production Cleanup: Faithful (Ollama {})", settings.provider.ollama_model);
        println!("=========================================================================================\n");

        // -----------------------------------------------------------------------------------------
        // 1. REAL SEGMENTATION OVER 53.8-SECOND RECORDING
        // -----------------------------------------------------------------------------------------
        println!("-----------------------------------------------------------------------------------------");
        println!("1. ACTUAL REAL SEGMENTATION BREAKDOWN (WHISPER SMALL + FAITHFUL)");
        println!("-----------------------------------------------------------------------------------------");
        println!("{:<4} | {:>6} | {:>6} | {:>8} | {:<12} | {:>8} | {:>11} | {:<20} | Raw Transcript",
            "Seg#", "Start", "End", "Duration", "Close Reason", "STT (ms)", "Faithful ms", "Eligibility");
        println!("{}", "-".repeat(120));

        let mut segmenter = StreamingSegmenter::new("phase4-session");
        // Push in chunks of 512 samples (~32 ms, matching CPAL callback cadence)
        let chunk_size = 512;
        let mut completed_segments = Vec::new();
        for chunk in raw_samples.chunks(chunk_size) {
            completed_segments.extend(segmenter.push(chunk));
        }
        completed_segments.extend(segmenter.flush(false));

        #[allow(dead_code)]
        struct SegmentExecutionRecord {
            id: u64,
            start_s: f64,
            end_s: f64,
            duration_ms: u64,
            reason: SegmentCloseReason,
            raw_text: String,
            faithful_text: Option<String>,
            stt_ms: u128,
            faithful_ms: u128,
            eligibility: CleanupEligibility,
            audio: Vec<f32>,
        }

        let mut segment_records = Vec::new();

        for seg in &completed_segments {
            let t_stt = Instant::now();
            let audio_slice = seg.audio.clone();
            let sm_path = small_model_path.to_string_lossy().to_string();
            let lang_cfg = language_config.clone();
            let dec_cfg = decoding_config.clone();
            let eng = engine.clone();

            let (raw_text, _diag) = tokio::task::spawn_blocking(move || {
                eng.transcribe_with_config(Some(&sm_path), &audio_slice, &lang_cfg, &dec_cfg)
                    .unwrap_or_else(|_| (String::new(), crate::capture::stt::SttSessionDiagnostics::default()))
            })
            .await
            .unwrap();

            let stt_ms = t_stt.elapsed().as_millis();
            let raw_text = raw_text.trim().to_string();

            let eligibility = evaluate_cleanup_eligibility(&raw_text, seg.reason_closed, seg.hangover_ms);

            let mut faithful_ms = 0u128;
            let mut faithful_text = None;

            if eligibility.is_safe() {
                let t_faith = Instant::now();
                let prop = rewrite::propose(client.as_ref(), &raw_text, CleanupStyle::Faithful).await;
                faithful_ms = t_faith.elapsed().as_millis();
                faithful_text = Some(if prop.changed { prop.rewritten } else { raw_text.clone() });
            }

            println!("{:<4} | {:>5.2}s | {:>5.2}s | {:>6}ms | {:<12} | {:>6}ms | {:>9}ms | {:<20} | {}",
                seg.segment_id,
                seg.start_seconds,
                seg.end_seconds,
                seg.duration_ms,
                seg.reason_closed.as_str(),
                stt_ms,
                if faithful_ms > 0 { format!("{}ms", faithful_ms) } else { "-".to_string() },
                if eligibility.is_safe() { "SAFE" } else { "NOT_SAFE" },
                raw_text
            );

            segment_records.push(SegmentExecutionRecord {
                id: seg.segment_id,
                start_s: seg.start_seconds,
                end_s: seg.end_seconds,
                duration_ms: seg.duration_ms,
                reason: seg.reason_closed,
                raw_text,
                faithful_text,
                stt_ms,
                faithful_ms,
                eligibility,
                audio: seg.audio.clone(),
            });
        }

        // -----------------------------------------------------------------------------------------
        // 2. FAITHFUL THROUGHPUT & BACKLOG ANALYSIS
        // -----------------------------------------------------------------------------------------
        println!("\n-----------------------------------------------------------------------------------------");
        println!("2. FAITHFUL THROUGHPUT & BACKLOG DYNAMICS");
        println!("-----------------------------------------------------------------------------------------");
        let mut durations: Vec<f64> = segment_records.iter().map(|s| s.duration_ms as f64).collect();
        durations.sort_by(|a, b| a.partial_cmp(b).unwrap());

        let avg_dur = if !durations.is_empty() { durations.iter().sum::<f64>() / durations.len() as f64 } else { 0.0 };
        let median_dur = if !durations.is_empty() { durations[durations.len() / 2] } else { 0.0 };
        let p90_idx = ((durations.len() as f64 * 0.9).ceil() as usize).saturating_sub(1);
        let p90_dur = durations.get(p90_idx).copied().unwrap_or(0.0);
        let p95_idx = ((durations.len() as f64 * 0.95).ceil() as usize).saturating_sub(1);
        let p95_dur = durations.get(p95_idx).copied().unwrap_or(0.0);

        let safe_faithful_times: Vec<u128> = segment_records.iter().filter(|s| s.faithful_ms > 0).map(|s| s.faithful_ms).collect();
        let avg_faithful_ms = if !safe_faithful_times.is_empty() { safe_faithful_times.iter().sum::<u128>() as f64 / safe_faithful_times.len() as f64 } else { 0.0 };

        println!("Segment Duration Distribution:");
        println!("  Total Segments Produced : {}", segment_records.len());
        println!("  Average Duration        : {:.1} ms", avg_dur);
        println!("  Median Duration         : {:.1} ms", median_dur);
        println!("  p90 Duration            : {:.1} ms", p90_dur);
        println!("  p95 Duration            : {:.1} ms", p95_dur);
        println!("Faithful Execution Metrics:");
        println!("  Eligible Segments       : {} of {}", safe_faithful_times.len(), segment_records.len());
        println!("  Average Faithful Latency: {:.1} ms", avg_faithful_ms);

        // Calculate inter-segment arrival intervals
        let mut arrival_intervals = Vec::new();
        for i in 1..segment_records.len() {
            let interval = (segment_records[i].end_s - segment_records[i - 1].end_s) * 1000.0;
            arrival_intervals.push(interval);
        }
        let avg_arrival = if !arrival_intervals.is_empty() { arrival_intervals.iter().sum::<f64>() / arrival_intervals.len() as f64 } else { 0.0 };
        println!("Arrival Dynamics:");
        println!("  Average Arrival Interval: {:.1} ms", avg_arrival);
        if avg_arrival > avg_faithful_ms {
            println!("  Verdict                 : Faithful CAN keep pace with speech (Arrival {:.1} ms > Faithful {:.1} ms)", avg_arrival, avg_faithful_ms);
        } else {
            println!("  Verdict                 : BACKLOG GROWS (Arrival {:.1} ms < Faithful {:.1} ms, Net lag: +{:.1} ms/segment)",
                avg_arrival, avg_faithful_ms, avg_faithful_ms - avg_arrival);
        }

        // -----------------------------------------------------------------------------------------
        // 3. WHISPER BASE VS. WHISPER SMALL THROUGHPUT ON REAL SEGMENTS
        // -----------------------------------------------------------------------------------------
        println!("\n-----------------------------------------------------------------------------------------");
        println!("3. STT THROUGHPUT: WHISPER BASE VS. WHISPER SMALL (REAL SEGMENTS)");
        println!("-----------------------------------------------------------------------------------------");
        println!("{:<4} | {:>8} | {:>10} | {:>8} | {:>10} | {:>8} | {:>7}",
            "Seg#", "Audio ms", "Base (ms)", "Base RTF", "Small (ms)", "Sml RTF", "Ratio");
        println!("{}", "-".repeat(75));

        let mut base_times = Vec::new();
        let mut small_times = Vec::new();

        for seg in &segment_records {
            let audio_slice = seg.audio.clone();
            let base_p = base_model_path.to_string_lossy().to_string();
            let lang_cfg = language_config.clone();
            let dec_cfg = decoding_config.clone();
            let eng = engine.clone();

            let t_base = Instant::now();
            let _ = tokio::task::spawn_blocking(move || {
                eng.transcribe_with_config(Some(&base_p), &audio_slice, &lang_cfg, &dec_cfg)
            })
            .await
            .unwrap();
            let base_ms = t_base.elapsed().as_millis();

            let sm_ms = seg.stt_ms;
            base_times.push(base_ms);
            small_times.push(sm_ms);

            let base_rtf = (base_ms as f64 / 1000.0) / (seg.duration_ms as f64 / 1000.0);
            let sm_rtf = (sm_ms as f64 / 1000.0) / (seg.duration_ms as f64 / 1000.0);
            let ratio = sm_ms as f64 / base_ms.max(1) as f64;

            println!("{:<4} | {:>6}ms | {:>8}ms | {:>7.2}x | {:>8}ms | {:>6.2}x | {:>6.2}x",
                seg.id, seg.duration_ms, base_ms, base_rtf, sm_ms, sm_rtf, ratio);
        }

        base_times.sort();
        small_times.sort();

        let base_median = base_times[base_times.len() / 2];
        let base_p90 = base_times[((base_times.len() as f64 * 0.9).ceil() as usize).saturating_sub(1)];
        let sm_median = small_times[small_times.len() / 2];
        let sm_p90 = small_times[((small_times.len() as f64 * 0.9).ceil() as usize).saturating_sub(1)];

        println!("\nThroughput Summary:");
        println!("  Whisper Base  : Median = {} ms, p90 = {} ms", base_median, base_p90);
        println!("  Whisper Small : Median = {} ms, p90 = {} ms", sm_median, sm_p90);
        println!("  Overhead Factor: Small is ~{:.2}x slower than Base per segment", sm_median as f64 / base_median.max(1) as f64);

        // -----------------------------------------------------------------------------------------
        // 4. TAIL BENCHMARK ACROSS 21 SYNTHETIC RELEASE POINTS
        // -----------------------------------------------------------------------------------------
        println!("\n-----------------------------------------------------------------------------------------");
        println!("4. TAIL BENCHMARK ACROSS 21 SYNTHETIC RELEASE POINTS");
        println!("-----------------------------------------------------------------------------------------");
        println!("{:<12} | {:>10} | {:>10} | {:>12} | {:>15}",
            "Release At", "Tail Dur", "Tail STT", "Tail Faithful", "Release->Final");
        println!("{}", "-".repeat(70));

        let release_points: Vec<f64> = (1..=21).map(|i| i as f64 * 2.5).collect();
        let mut tail_durations = Vec::new();
        let mut release_to_finals = Vec::new();

        for &rel_sec in &release_points {
            let sample_count = ((rel_sec * 16000.0).round() as usize).min(total_samples);
            let prefix_samples = &raw_samples[0..sample_count];

            let mut seg = StreamingSegmenter::new(format!("tail-{}", rel_sec));
            for chunk in prefix_samples.chunks(512) {
                let _ = seg.push(chunk);
            }
            let flushed = seg.flush(true);
            let tail = flushed.last();

            let tail_dur_ms = tail.map(|t| t.duration_ms).unwrap_or(0);
            tail_durations.push(tail_dur_ms);

            let mut tail_stt_ms = 0u128;
            let mut tail_faith_ms = 0u128;
            let t_release = Instant::now();

            if let Some(t_seg) = tail {
                let eng = engine.clone();
                let a = t_seg.audio.clone();
                let sm_path = small_model_path.to_string_lossy().to_string();
                let lang_cfg = language_config.clone();
                let dec_cfg = decoding_config.clone();

                let t_start_stt = Instant::now();
                let (raw, _) = tokio::task::spawn_blocking(move || {
                    eng.transcribe_with_config(Some(&sm_path), &a, &lang_cfg, &dec_cfg).unwrap_or_default()
                })
                .await
                .unwrap();
                tail_stt_ms = t_start_stt.elapsed().as_millis();

                let elig = evaluate_cleanup_eligibility(&raw, t_seg.reason_closed, t_seg.hangover_ms);
                if elig.is_safe() {
                    let t_f = Instant::now();
                    let _ = rewrite::propose(client.as_ref(), &raw, CleanupStyle::Faithful).await;
                    tail_faith_ms = t_f.elapsed().as_millis();
                }
            }

            let release_to_final_ms = t_release.elapsed().as_millis();
            release_to_finals.push(release_to_final_ms);

            println!("{:<11.1}s | {:>8}ms | {:>8}ms | {:>10}ms | {:>13}ms",
                rel_sec, tail_dur_ms, tail_stt_ms, tail_faith_ms, release_to_final_ms);
        }

        tail_durations.sort();
        release_to_finals.sort();
        let median_tail = tail_durations[tail_durations.len() / 2];
        let p90_tail = tail_durations[((tail_durations.len() as f64 * 0.9).ceil() as usize).saturating_sub(1)];
        let median_rel = release_to_finals[release_to_finals.len() / 2];
        let p90_rel = release_to_finals[((release_to_finals.len() as f64 * 0.9).ceil() as usize).saturating_sub(1)];

        println!("\nTail Metrics Summary:");
        println!("  Tail Duration : Median = {} ms, p90 = {} ms", median_tail, p90_tail);
        println!("  Release->Final: Median = {} ms, p90 = {} ms", median_rel, p90_rel);

        // -----------------------------------------------------------------------------------------
        // 5. ACCURACY RECONCILIATION & TRANSCRIPT EQUIVALENCE
        // -----------------------------------------------------------------------------------------
        println!("\n-----------------------------------------------------------------------------------------");
        println!("5. ACCURACY RECONCILIATION: BASELINE (SINGLE-SHOT) VS. SHADOW (STREAMING)");
        println!("-----------------------------------------------------------------------------------------");

        // Baseline single-shot STT
        let t_base_start = Instant::now();
        let eng = engine.clone();
        let sm_path = small_model_path.to_string_lossy().to_string();
        let lang_cfg = language_config.clone();
        let dec_cfg = decoding_config.clone();
        let all_audio = raw_samples.clone();

        let (baseline_raw, _) = tokio::task::spawn_blocking(move || {
            eng.transcribe_with_config(Some(&sm_path), &all_audio, &lang_cfg, &dec_cfg).unwrap()
        })
        .await
        .unwrap();
        let baseline_stt_ms = t_base_start.elapsed().as_millis();

        let t_base_faith = Instant::now();
        let baseline_faithful_prop = rewrite::propose(client.as_ref(), &baseline_raw, CleanupStyle::Faithful).await;
        let baseline_faithful_ms = t_base_faith.elapsed().as_millis();
        let baseline_faithful = if baseline_faithful_prop.changed { baseline_faithful_prop.rewritten } else { baseline_raw.clone() };

        // Shadow assembly
        let shadow_raw_parts: Vec<&str> = segment_records.iter().map(|s| s.raw_text.as_str()).filter(|t| !t.is_empty()).collect();
        let shadow_raw = shadow_raw_parts.join(" ");

        let shadow_faithful_parts: Vec<&str> = segment_records
            .iter()
            .map(|s| s.faithful_text.as_deref().unwrap_or(s.raw_text.as_str()))
            .filter(|t| !t.trim().is_empty())
            .collect();
        let shadow_faithful = shadow_faithful_parts.join(" ");

        let raw_acc = calculate_accuracy(&baseline_raw, &shadow_raw);
        let faithful_acc = calculate_accuracy(&baseline_faithful, &shadow_faithful);

        println!("Baseline Single-Shot STT ({} ms):", baseline_stt_ms);
        println!("  {}\n", baseline_raw);
        println!("Shadow Segmented STT:");
        println!("  {}\n", shadow_raw);
        println!("Baseline Faithful ({} ms):", baseline_faithful_ms);
        println!("  {}\n", baseline_faithful);
        println!("Shadow Faithful:");
        println!("  {}\n", shadow_faithful);

        println!("Transcript Alignment Metrics:");
        println!("  Raw STT Alignment     : WER = {:.2}%, CER = {:.2}%, Subs = {}, Dels = {}, Ins = {}",
            raw_acc.wer * 100.0, raw_acc.cer * 100.0,
            raw_acc.substitutions, raw_acc.deletions, raw_acc.insertions);
        println!("  Faithful Alignment    : WER = {:.2}%, CER = {:.2}%, Subs = {}, Dels = {}, Ins = {}",
            faithful_acc.wer * 100.0, faithful_acc.cer * 100.0,
            faithful_acc.substitutions, faithful_acc.deletions, faithful_acc.insertions);

        // -----------------------------------------------------------------------------------------
        // 6. CPU UTILIZATION & CONCURRENCY IMPACT
        // -----------------------------------------------------------------------------------------
        println!("\n-----------------------------------------------------------------------------------------");
        println!("6. CPU UTILIZATION & CONCURRENCY CONTROLLER");
        println!("-----------------------------------------------------------------------------------------");
        println!("Comparing Isolated STT vs. Concurrent STT + Faithful LLM Execution:");

        let seg_sample1 = segment_records[0].audio.clone();
        let sm_path2 = small_model_path.to_string_lossy().to_string();
        let eng2 = engine.clone();
        let lang2 = language_config.clone();
        let dec2 = decoding_config.clone();

        let t_iso_start = Instant::now();
        let _ = tokio::task::spawn_blocking(move || {
            eng2.transcribe_with_config(Some(&sm_path2), &seg_sample1, &lang2, &dec2).unwrap()
        })
        .await
        .unwrap();
        let iso_stt_ms = t_iso_start.elapsed().as_millis();

        // Concurrent run: STT + Faithful simultaneously
        let seg_sample2 = segment_records[0].audio.clone();
        let sm_path3 = small_model_path.to_string_lossy().to_string();
        let eng3 = engine.clone();
        let lang3 = language_config.clone();
        let dec3 = decoding_config.clone();
        let client_ref = client.clone();
        let text_sample = segment_records[0].raw_text.clone();

        let t_conc_start = Instant::now();
        let stt_task = tokio::task::spawn_blocking(move || {
            eng3.transcribe_with_config(Some(&sm_path3), &seg_sample2, &lang3, &dec3).unwrap()
        });
        let faith_task = tokio::spawn(async move {
            rewrite::propose(client_ref.as_ref(), &text_sample, CleanupStyle::Faithful).await
        });

        let (stt_res, faith_res) = tokio::join!(stt_task, faith_task);
        let conc_total_ms = t_conc_start.elapsed().as_millis();
        assert!(stt_res.is_ok());
        assert!(faith_res.is_ok());

        println!("  Isolated Segment STT Decode : {} ms", iso_stt_ms);
        println!("  Concurrent STT + Faithful   : {} ms (Overlap Wall Clock)", conc_total_ms);
        println!("  Concurrency Degradation     : {:.2}x", conc_total_ms as f64 / iso_stt_ms.max(1) as f64);
        println!("=========================================================================================\n");
    }
}

