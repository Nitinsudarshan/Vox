//! Standalone Meeting Pipeline Benchmark Runner for Vox.
//! Executes the full production meeting transcription pipeline:
//! Audio -> Segmenter -> TranscriptionQueue -> Worker (DomainVocabulary + Whisper + Quality Gate + Recovery) -> Segments + Telemetry.

use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::time::Instant;

use serde::{Deserialize, Serialize};
use vox_lib::capture::stt::{SttEngine, SttLanguageConfig, WhisperDecodingConfig};
use vox_lib::capture::vocabulary::DomainVocabulary;
use vox_lib::meetings::segmenter::Segmenter;
use vox_lib::meetings::store::MeetingStore;
use vox_lib::meetings::transcription::{spawn_worker, WorkerConfig};

#[derive(Debug, Serialize, Deserialize)]
pub struct BenchmarkSegmentOutput {
    pub sequence: u64,
    pub start_seconds: f64,
    pub end_seconds: f64,
    pub duration: f64,
    pub channel: String,
    pub text: String,
    pub telemetry: Option<vox_lib::meetings::model::SegmentTelemetry>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct BenchmarkRunResult {
    pub audio_file: String,
    pub model_path: String,
    pub language_config: String,
    pub total_audio_duration_seconds: f64,
    pub total_speech_seconds: f64,
    pub total_inference_ms: u128,
    pub decode_real_time_factor: f64,
    pub pipeline_real_time_factor: f64,
    pub segments_decoded_count: u64,
    pub segments_kept_count: u64,
    pub segments_discarded_count: u64,
    pub transcript: String,
    pub segments: Vec<BenchmarkSegmentOutput>,
}

fn load_wav_samples_16k_mono(path: &Path) -> Result<(Vec<f32>, f64), String> {
    let mut reader = hound::WavReader::open(path)
        .map_err(|e| format!("Failed to open WAV file {:?}: {}", path, e))?;
    let spec = reader.spec();

    let raw_samples: Vec<f32> = match spec.sample_format {
        hound::SampleFormat::Int => {
            let max_val = if spec.bits_per_sample > 0 {
                (1i64 << (spec.bits_per_sample - 1)) as f32
            } else {
                32768.0
            };
            reader
                .samples::<i32>()
                .map(|s| s.unwrap_or(0) as f32 / max_val)
                .collect()
        }
        hound::SampleFormat::Float => reader.samples::<f32>().map(|s| s.unwrap_or(0.0)).collect(),
    };

    let mono_samples = if spec.channels > 1 {
        let mut mono = Vec::with_capacity(raw_samples.len() / spec.channels as usize);
        for chunk in raw_samples.chunks_exact(spec.channels as usize) {
            let sum: f32 = chunk.iter().sum();
            mono.push(sum / spec.channels as f32);
        }
        mono
    } else {
        raw_samples
    };

    let resampled = if spec.sample_rate != 16000 {
        vox_lib::capture::resample_to_16k_mono(&mono_samples, spec.sample_rate)
    } else {
        mono_samples
    };

    let duration_sec = resampled.len() as f64 / 16000.0;
    Ok((resampled, duration_sec))
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().collect();

    let mut audio_path: Option<PathBuf> = None;
    let mut model_path: Option<PathBuf> = None;
    let mut language = "auto".to_string();
    let mut output_path: Option<PathBuf> = None;
    let mut meeting_title: Option<String> = None;
    let mut user_terms_raw: Option<String> = None;

    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--audio" => {
                i += 1;
                if i < args.len() {
                    audio_path = Some(PathBuf::from(&args[i]));
                }
            }
            "--model" => {
                i += 1;
                if i < args.len() {
                    model_path = Some(PathBuf::from(&args[i]));
                }
            }
            "--lang" | "--language" => {
                i += 1;
                if i < args.len() {
                    language = args[i].clone();
                }
            }
            "--output" => {
                i += 1;
                if i < args.len() {
                    output_path = Some(PathBuf::from(&args[i]));
                }
            }
            "--meeting-title" => {
                i += 1;
                if i < args.len() {
                    meeting_title = Some(args[i].clone());
                }
            }
            "--user-terms" => {
                i += 1;
                if i < args.len() {
                    user_terms_raw = Some(args[i].clone());
                }
            }
            _ => {}
        }
        i += 1;
    }

    let audio_file = audio_path.ok_or("Missing required argument --audio <path>")?;
    let model_file = model_path.ok_or("Missing required argument --model <path>")?;

    if !audio_file.exists() {
        return Err(format!("Audio file {:?} not found", audio_file).into());
    }
    if !model_file.exists() {
        return Err(format!("Model file {:?} not found", model_file).into());
    }

    eprintln!("[vox-benchmark] Loading audio: {:?}", audio_file);
    let (samples, audio_duration) = load_wav_samples_16k_mono(&audio_file)?;
    eprintln!(
        "[vox-benchmark] Audio loaded: {:.2}s ({}) samples",
        audio_duration,
        samples.len()
    );

    // 1. Setup DomainVocabulary
    let mut vocabulary = DomainVocabulary::new();
    if let Some(title) = meeting_title {
        vocabulary = vocabulary.with_meeting_terms(&[title]);
    }
    if let Some(raw) = user_terms_raw {
        let terms: Vec<String> = raw.split(',').map(|s| s.trim().to_string()).collect();
        vocabulary = vocabulary.with_user_terms(&terms);
    }

    // 2. Setup SttLanguageConfig
    let lang_config = match language.as_str() {
        "en" => SttLanguageConfig {
            whisper_language: Some("en".to_string()),
            translate: false,
        },
        "hi" => SttLanguageConfig {
            whisper_language: Some("hi".to_string()),
            translate: false,
        },
        _ => SttLanguageConfig {
            whisper_language: None,
            translate: false,
        },
    };

    // 3. WorkerConfig
    let meeting_id = format!("bench_{}", uuid::Uuid::new_v4());
    let temp_vault = std::env::temp_dir().join(format!("vox_bench_{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&temp_vault)?;
    let store = Arc::new(MeetingStore::new(&temp_vault));

    let decoding_cfg = WhisperDecodingConfig::baseline();
    let decoding_expensive = decoding_cfg.clone().for_expensive_script();

    let worker_config = WorkerConfig {
        meeting_id: meeting_id.clone(),
        model_path: model_file.to_string_lossy().to_string(),
        language: lang_config.clone(),
        decoding: decoding_cfg,
        decoding_expensive_script: decoding_expensive,
        glossary: Vec::new(),
        vocabulary,
    };

    // 4. Spawn Worker
    eprintln!("[vox-benchmark] Spawning transcription worker with model: {:?}", model_file);
    let cancel_flag = Arc::new(AtomicBool::new(false));
    let (queue, worker_handle) = spawn_worker(
        worker_config,
        SttEngine::new(),
        Arc::clone(&store),
        None,
        cancel_flag,
    );

    // 5. Run Segmenter streaming audio chunks
    eprintln!("[vox-benchmark] Streaming audio through Segmenter...");
    let mut segmenter = Segmenter::new();
    let chunk_size = 800; // 50ms at 16kHz
    let wall_start = Instant::now();

    for chunk in samples.chunks(chunk_size) {
        for seg in segmenter.push(chunk, chunk, &[]) {
            queue.submit(seg);
        }
    }
    for seg in segmenter.flush() {
        queue.submit(seg);
    }

    // Drop queue to close worker channel
    drop(queue);

    // 6. Wait for worker to finish
    eprintln!("[vox-benchmark] Draining transcription queue...");
    let stats = worker_handle
        .join()
        .map_err(|_| "Worker thread panicked")?;
    let _total_wall_ms = wall_start.elapsed().as_millis();

    eprintln!(
        "[vox-benchmark] Complete: decoded={} kept={} discarded={} decode_ms={} rtf={:.3}",
        stats.decoded,
        stats.kept,
        stats.discarded,
        stats.decode_ms,
        stats.decode_rtf()
    );

    // 7. Load segments and construct canonical transcript
    let saved_segments = store.load_transcript(&meeting_id).unwrap_or_default();
    let mut full_transcript_parts = Vec::new();
    let mut output_segments = Vec::new();

    for seg in saved_segments {
        let text_trimmed = seg.text.trim().to_string();
        if !text_trimmed.is_empty() {
            full_transcript_parts.push(text_trimmed.clone());
        }
        output_segments.push(BenchmarkSegmentOutput {
            sequence: seg.sequence,
            start_seconds: seg.start_seconds,
            end_seconds: seg.end_seconds,
            duration: seg.end_seconds - seg.start_seconds,
            channel: format!("{:?}", seg.channel),
            text: text_trimmed,
            telemetry: seg.telemetry,
        });
    }

    let full_transcript = full_transcript_parts.join(" ");

    let result = BenchmarkRunResult {
        audio_file: audio_file.to_string_lossy().to_string(),
        model_path: model_file.to_string_lossy().to_string(),
        language_config: language,
        total_audio_duration_seconds: audio_duration,
        total_speech_seconds: stats.speech_seconds,
        total_inference_ms: stats.decode_ms,
        decode_real_time_factor: stats.decode_rtf(),
        pipeline_real_time_factor: (stats.decode_ms as f64 / 1000.0) / audio_duration.max(0.001),
        segments_decoded_count: stats.decoded,
        segments_kept_count: stats.kept,
        segments_discarded_count: stats.discarded,
        transcript: full_transcript,
        segments: output_segments,
    };

    // Clean up temp vault
    let _ = std::fs::remove_dir_all(&temp_vault);

    let json_bytes = serde_json::to_string_pretty(&result)?;
    if let Some(out) = output_path {
        std::fs::write(&out, &json_bytes)?;
        eprintln!("[vox-benchmark] Output written to {:?}", out);
    } else {
        println!("{}", json_bytes);
    }

    Ok(())
}
