use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use vox_lib::capture::evaluation::calculate_accuracy;
use vox_lib::capture::recognizer::{RecognitionRequest, SpeechRecognizer};
use vox_lib::capture::recognizers::{ParakeetRecognizer, WhisperRecognizer};
use vox_lib::capture::stt::{SttEngine, WhisperDecodingConfig};
use vox_lib::settings::AppSettings;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelRunSample {
    pub run_number: usize,
    pub is_cold: bool,
    pub model_load_ms: u128,
    pub lock_wait_ms: u128,
    pub decode_ms: u128,
    pub total_latency_ms: u128,
    pub rtf: f64,
    pub avg_cpu_percent: f64,
    pub peak_cpu_percent: f64,
    pub transcript: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ShortWindowSample {
    pub window_label: String,
    pub duration_s: f64,
    pub sample_count: usize,
    pub decode_ms: u128,
    pub rtf: f64,
    pub transcript: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AccuracyData {
    pub wer: f64,
    pub cer: f64,
    pub substitutions: usize,
    pub deletions: usize,
    pub insertions: usize,
    pub reference_words: usize,
    pub hypothesis_words: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelFullBenchmark {
    pub model_id: String,
    pub engine: String,
    pub model_name: String,
    pub filename: String,
    pub file_size_bytes: u64,
    pub parameters_millions: Option<u32>,
    pub precision: String,
    pub backend: String,
    pub thread_count: usize,
    pub gpu_used: bool,
    pub cold_load_ms: u128,
    pub cold_stt_ms: u128,
    pub cold_total_ms: u128,
    pub warm_min_ms: u128,
    pub warm_median_ms: u128,
    pub warm_p90_ms: u128,
    pub warm_max_ms: u128,
    pub median_rtf: f64,
    pub accuracy: AccuracyData,
    pub avg_cpu_percent: f64,
    pub peak_cpu_percent: f64,
    pub runs: Vec<ModelRunSample>,
    pub short_windows: Vec<ShortWindowSample>,
    pub raw_transcript: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BenchmarkSuiteReport {
    pub created_at: String,
    pub audio_filename: String,
    pub audio_duration_seconds: f64,
    pub reference_transcript: String,
    pub logical_cpu_cores: usize,
    pub models: Vec<ModelFullBenchmark>,
}

fn get_process_cpu_time() -> u64 {
    #[cfg(target_os = "windows")]
    unsafe {
        use windows_sys::Win32::Foundation::FILETIME;
        use windows_sys::Win32::System::Threading::{GetCurrentProcess, GetProcessTimes};
        let mut creation = FILETIME { dwLowDateTime: 0, dwHighDateTime: 0 };
        let mut exit = FILETIME { dwLowDateTime: 0, dwHighDateTime: 0 };
        let mut kernel = FILETIME { dwLowDateTime: 0, dwHighDateTime: 0 };
        let mut user = FILETIME { dwLowDateTime: 0, dwHighDateTime: 0 };
        if GetProcessTimes(GetCurrentProcess(), &mut creation, &mut exit, &mut kernel, &mut user) != 0 {
            let k = ((kernel.dwHighDateTime as u64) << 32) | (kernel.dwLowDateTime as u64);
            let u = ((user.dwHighDateTime as u64) << 32) | (user.dwLowDateTime as u64);
            return k + u; // 100ns units
        }
    }
    0
}

struct CpuTracker {
    #[allow(dead_code)]
    num_cpus: usize,
    stop_signal: Arc<AtomicBool>,
    sampler_handle: Option<std::thread::JoinHandle<f64>>,
    start_cpu_time: u64,
    start_wall: Instant,
}

impl CpuTracker {
    pub fn start() -> Self {
        let num_cpus = std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(8);
        let stop_signal = Arc::new(AtomicBool::new(false));
        let stop_clone = Arc::clone(&stop_signal);

        let sampler_handle = std::thread::spawn(move || {
            let mut peak_pct = 0.0f64;
            let mut prev_cpu = get_process_cpu_time();
            let mut prev_wall = Instant::now();

            while !stop_clone.load(Ordering::Relaxed) {
                std::thread::sleep(Duration::from_millis(50));
                let now_cpu = get_process_cpu_time();
                let now_wall = Instant::now();
                let wall_elapsed_s = now_wall.duration_since(prev_wall).as_secs_f64();
                if wall_elapsed_s > 0.01 {
                    let cpu_diff_100ns = now_cpu.saturating_sub(prev_cpu);
                    let cpu_seconds = (cpu_diff_100ns as f64) * 1e-7;
                    let pct = (cpu_seconds / wall_elapsed_s) * 100.0;
                    if pct > peak_pct {
                        peak_pct = pct;
                    }
                }
                prev_cpu = now_cpu;
                prev_wall = now_wall;
            }
            peak_pct
        });

        Self {
            num_cpus,
            stop_signal,
            sampler_handle: Some(sampler_handle),
            start_cpu_time: get_process_cpu_time(),
            start_wall: Instant::now(),
        }
    }

    pub fn stop(mut self) -> (f64, f64) {
        self.stop_signal.store(true, Ordering::Relaxed);
        let peak_pct = if let Some(handle) = self.sampler_handle.take() {
            handle.join().unwrap_or(0.0)
        } else {
            0.0
        };

        let wall_elapsed_s = self.start_wall.elapsed().as_secs_f64();
        let end_cpu = get_process_cpu_time();
        let cpu_diff_100ns = end_cpu.saturating_sub(self.start_cpu_time);
        let cpu_seconds = (cpu_diff_100ns as f64) * 1e-7;
        let avg_pct = if wall_elapsed_s > 0.0 {
            (cpu_seconds / wall_elapsed_s) * 100.0
        } else {
            0.0
        };

        (avg_pct, peak_pct.max(avg_pct))
    }
}

fn percentile(sorted: &[u128], p: f64) -> u128 {
    if sorted.is_empty() {
        return 0;
    }
    let idx = ((sorted.len() as f64 * p).ceil() as usize).saturating_sub(1);
    sorted[idx.min(sorted.len() - 1)]
}

fn main() {
    let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    let config_dir = manifest_dir.join(".vox/config");
    let audio_path = config_dir.join("dictation_tests/audio/test_7aec643b_1790015082009.wav");
    let settings_path = config_dir.join("settings.json");

    println!("==========================================================================================");
    println!("VOX PHASE 4.5 — COMPREHENSIVE STT MODEL BENCHMARK (WHISPER + PARAKEET)");
    println!("==========================================================================================");

    let settings_raw = fs::read_to_string(&settings_path).expect("Failed to read settings.json");
    let settings: AppSettings = serde_json::from_str(&settings_raw).expect("Failed to parse settings.json");

    let num_logical_cores = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(8);
    println!("Hardware Execution Platform: {} logical CPU cores, Windows x86_64, GPU: None (CPU Only)", num_logical_cores);

    let mut reader = hound::WavReader::open(&audio_path).expect("Failed to open canonical benchmark audio");
    let samples_53s: Vec<f32> = reader
        .samples::<i16>()
        .filter_map(|s| s.ok())
        .map(|s| s as f32 / i16::MAX as f32)
        .collect();

    let total_audio_duration_s = samples_53s.len() as f64 / 16000.0;
    println!("Canonical Benchmark Corpus: {} ({:.2}s, {} samples)", audio_path.file_name().unwrap().to_string_lossy(), total_audio_duration_s, samples_53s.len());

    let reference_transcript = "Um, I think, like, the main thing we need to fix is the transcription speed because right now it takes too much time and, uh, sometimes the words doesn't come out correctly, especially when I'm speaking fast, and basically we should make it faster without, you know, changing what I actually meant.  Also, I was thinking that the cleanup should, kind of, remove all these unnecessary fillers and fix the grammar and punctuation, but it shouldn't rewrite everything or make it sound too formal, because, um, the point is still to keep the way I actually said it and just make it more readable, if that makes sense.".to_string();

    struct ModelTargetDef {
        id: &'static str,
        engine: &'static str,
        name: &'static str,
        filename: &'static str,
        path: PathBuf,
        parameters_millions: Option<u32>,
        precision: &'static str,
    }

    let models_dir = config_dir.join("models");
    let targets = vec![
        ModelTargetDef {
            id: "whisper:ggml-base.bin",
            engine: "whisper",
            name: "Whisper Base",
            filename: "ggml-base.bin",
            path: models_dir.join("ggml-base.bin"),
            parameters_millions: Some(74),
            precision: "f16/q8_0 mixed",
        },
        ModelTargetDef {
            id: "whisper:ggml-small.bin",
            engine: "whisper",
            name: "Whisper Small",
            filename: "ggml-small.bin",
            path: models_dir.join("ggml-small.bin"),
            parameters_millions: Some(244),
            precision: "f16/q8_0 mixed",
        },
        ModelTargetDef {
            id: "whisper:ggml-large-v3-turbo.bin",
            engine: "whisper",
            name: "Whisper Large v3 Turbo",
            filename: "ggml-large-v3-turbo.bin",
            path: models_dir.join("ggml-large-v3-turbo.bin"),
            parameters_millions: Some(809),
            precision: "f16/q8_0 mixed",
        },
        ModelTargetDef {
            id: "whisper:ggml-hindi2hinglish-apex-q5_0.bin",
            engine: "whisper",
            name: "Whisper Hindi/Hinglish Q5",
            filename: "ggml-hindi2hinglish-apex-q5_0.bin",
            path: models_dir.join("ggml-hindi2hinglish-apex-q5_0.bin"),
            parameters_millions: Some(809),
            precision: "Q5_0",
        },
        ModelTargetDef {
            id: "whisper:ggml-large-v3-turbo-q5_0.bin",
            engine: "whisper",
            name: "Whisper Large v3 Turbo Q5",
            filename: "ggml-large-v3-turbo-q5_0.bin",
            path: models_dir.join("ggml-large-v3-turbo-q5_0.bin"),
            parameters_millions: Some(809),
            precision: "Q5_0",
        },
        ModelTargetDef {
            id: "parakeet:tdt-0.6b-v3",
            engine: "parakeet",
            name: "NVIDIA Parakeet TDT 0.6B",
            filename: "parakeet-tdt-0.6b-v3",
            path: models_dir.join("parakeet"),
            parameters_millions: Some(600),
            precision: "int8",
        },
    ];

    // Standardized short-utterance benchmark slices from the same 53.8s recording
    let short_windows_def: Vec<(&str, f64)> = vec![
        ("0.5-1s", 1.0),
        ("1-2s", 2.0),
        ("2-3s", 3.0),
        ("3-5s", 5.0),
        ("5-10s", 10.0),
        ("10-20s", 20.0),
    ];

    let mut benchmark_results = Vec::new();

    for target in &targets {
        println!("\n------------------------------------------------------------------------------------------");
        println!("BENCHMARKING TARGET: {} [{}]", target.name, target.id);
        println!("Path: {}", target.path.display());
        println!("------------------------------------------------------------------------------------------");

        let file_size_bytes = if target.engine == "parakeet" {
            let mut total = 0u64;
            if let Ok(entries) = fs::read_dir(&target.path) {
                for entry in entries.flatten() {
                    if let Ok(meta) = entry.metadata() {
                        total += meta.len();
                    }
                }
            }
            total
        } else {
            fs::metadata(&target.path).map(|m| m.len()).unwrap_or(0)
        };

        // Determine decoding config for whisper
        let whisper_dec_cfg = WhisperDecodingConfig::for_dictation(&settings.stt);
        let whisper_threads = whisper_dec_cfg.n_threads.unwrap_or(4) as usize;
        let thread_count = if target.engine == "parakeet" {
            num_logical_cores
        } else {
            whisper_threads
        };
        let backend_str = if target.engine == "parakeet" {
            "ONNX Runtime (CPU)".to_string()
        } else {
            format!("whisper.cpp (CPU/AVX2, {} threads)", whisper_threads)
        };

        // Run 5 full-audio trials isolated:
        // Trial 1: Cold start (fresh engine, loads from disk)
        // Trials 2..5: Warm runs (reuse existing engine state)
        let mut run_samples = Vec::new();
        let mut warm_stt_times = Vec::new();
        let mut warm_cpu_avgs = Vec::new();
        let mut warm_cpu_peaks = Vec::new();

        // 1. COLD START RUN
        let (cold_load_ms, cold_stt_ms, cold_total_ms, primary_transcript) = {
            println!("  [Run 1: COLD START]");
            let engine = SttEngine::new();
            let cpu_tracker = CpuTracker::start();
            let t_cold_start = Instant::now();

            let (rec_res, load_ms, stt_ms) = if target.engine == "parakeet" {
                let recognizer = ParakeetRecognizer::new(engine, &target.path);
                let t_rec_start = Instant::now();
                let res = recognizer.transcribe(RecognitionRequest {
                    samples: &samples_53s,
                    language: Some("en".to_string()),
                    translate: false,
                });
                let dur = t_rec_start.elapsed().as_millis();
                let (text, l_ms, d_ms) = match res {
                    Ok(r) => (r.text, r.timing.model_load_ms, r.timing.decode_ms),
                    Err(e) => (format!("ERROR: {e}"), 0, dur),
                };
                (text, l_ms, d_ms)
            } else {
                let recognizer = WhisperRecognizer::new(engine, &target.path, whisper_dec_cfg.clone());
                let t_rec_start = Instant::now();
                let res = recognizer.transcribe(RecognitionRequest {
                    samples: &samples_53s,
                    language: Some("en".to_string()),
                    translate: false,
                });
                let dur = t_rec_start.elapsed().as_millis();
                let (text, l_ms, d_ms) = match res {
                    Ok(r) => (r.text, r.timing.model_load_ms, r.timing.decode_ms),
                    Err(e) => (format!("ERROR: {e}"), 0, dur),
                };
                (text, l_ms, d_ms)
            };

            let cold_wall_ms = t_cold_start.elapsed().as_millis();
            let (avg_cpu, peak_cpu) = cpu_tracker.stop();

            let cold_load_ms = if load_ms > 0 { load_ms } else { cold_wall_ms.saturating_sub(stt_ms) };
            let cold_stt_ms = stt_ms;
            let cold_total_ms = cold_wall_ms;
            let primary_transcript = rec_res.clone();

            let rtf = (cold_stt_ms as f64 / 1000.0) / total_audio_duration_s;
            println!("    Load: {} ms | STT: {} ms | Total: {} ms | RTF: {:.3}x | CPU: avg {:.1}%, peak {:.1}%",
                cold_load_ms, cold_stt_ms, cold_total_ms, rtf, avg_cpu, peak_cpu);

            run_samples.push(ModelRunSample {
                run_number: 1,
                is_cold: true,
                model_load_ms: cold_load_ms,
                lock_wait_ms: 0,
                decode_ms: cold_stt_ms,
                total_latency_ms: cold_total_ms,
                rtf,
                avg_cpu_percent: avg_cpu,
                peak_cpu_percent: peak_cpu,
                transcript: rec_res,
            });

            (cold_load_ms, cold_stt_ms, cold_total_ms, primary_transcript)
        };

        // 2. WARM RUNS (Runs 2..5 on persistent engine)
        {
            let persistent_engine = SttEngine::new();
            // Prime persistent engine so it is definitively warm
            if target.engine == "parakeet" {
                let p_rec = ParakeetRecognizer::new(persistent_engine.clone(), &target.path);
                let _ = p_rec.transcribe(RecognitionRequest {
                    samples: &samples_53s[0..16000],
                    language: Some("en".to_string()),
                    translate: false,
                });
            } else {
                let w_rec = WhisperRecognizer::new(persistent_engine.clone(), &target.path, whisper_dec_cfg.clone());
                let _ = w_rec.transcribe(RecognitionRequest {
                    samples: &samples_53s[0..16000],
                    language: Some("en".to_string()),
                    translate: false,
                });
            }

            for run_idx in 2..=5 {
                println!("  [Run {}: WARM]", run_idx);
                let cpu_tracker = CpuTracker::start();
                let t_warm_start = Instant::now();

                let (text, lock_ms, decode_ms) = if target.engine == "parakeet" {
                    let recognizer = ParakeetRecognizer::new(persistent_engine.clone(), &target.path);
                    let res = recognizer.transcribe(RecognitionRequest {
                        samples: &samples_53s,
                        language: Some("en".to_string()),
                        translate: false,
                    });
                    match res {
                        Ok(r) => (r.text, r.timing.lock_wait_ms, r.timing.decode_ms),
                        Err(e) => (format!("ERROR: {e}"), 0, t_warm_start.elapsed().as_millis()),
                    }
                } else {
                    let recognizer = WhisperRecognizer::new(persistent_engine.clone(), &target.path, whisper_dec_cfg.clone());
                    let res = recognizer.transcribe(RecognitionRequest {
                        samples: &samples_53s,
                        language: Some("en".to_string()),
                        translate: false,
                    });
                    match res {
                        Ok(r) => (r.text, r.timing.lock_wait_ms, r.timing.decode_ms),
                        Err(e) => (format!("ERROR: {e}"), 0, t_warm_start.elapsed().as_millis()),
                    }
                };

                let total_warm_ms = t_warm_start.elapsed().as_millis();
                let (avg_cpu, peak_cpu) = cpu_tracker.stop();
                let effective_stt_ms = if decode_ms > 0 { decode_ms } else { total_warm_ms };
                let rtf = (effective_stt_ms as f64 / 1000.0) / total_audio_duration_s;

                println!("    STT: {} ms | Lock: {} ms | Total: {} ms | RTF: {:.3}x | CPU: avg {:.1}%, peak {:.1}%",
                    effective_stt_ms, lock_ms, total_warm_ms, rtf, avg_cpu, peak_cpu);

                warm_stt_times.push(effective_stt_ms);
                warm_cpu_avgs.push(avg_cpu);
                warm_cpu_peaks.push(peak_cpu);

                run_samples.push(ModelRunSample {
                    run_number: run_idx,
                    is_cold: false,
                    model_load_ms: 0,
                    lock_wait_ms: lock_ms,
                    decode_ms: effective_stt_ms,
                    total_latency_ms: total_warm_ms,
                    rtf,
                    avg_cpu_percent: avg_cpu,
                    peak_cpu_percent: peak_cpu,
                    transcript: text,
                });
            }
        }

        warm_stt_times.sort();
        let warm_min = warm_stt_times[0];
        let warm_median = warm_stt_times[warm_stt_times.len() / 2];
        let warm_p90 = percentile(&warm_stt_times, 0.90);
        let warm_max = *warm_stt_times.last().unwrap();
        let median_rtf = (warm_median as f64 / 1000.0) / total_audio_duration_s;

        let overall_avg_cpu = warm_cpu_avgs.iter().sum::<f64>() / warm_cpu_avgs.len() as f64;
        let overall_peak_cpu = warm_cpu_peaks.iter().copied().fold(0.0f64, f64::max);

        // Compute Accuracy against Canonical Reference Transcript
        let acc_res = calculate_accuracy(&reference_transcript, &primary_transcript);
        let ref_words = reference_transcript.split_whitespace().count();
        let hyp_words = primary_transcript.split_whitespace().count();
        let accuracy_data = AccuracyData {
            wer: acc_res.wer as f64,
            cer: acc_res.cer as f64,
            substitutions: acc_res.substitutions,
            deletions: acc_res.deletions,
            insertions: acc_res.insertions,
            reference_words: ref_words,
            hypothesis_words: hyp_words,
        };

        println!("\n  Long-Form Warm Summary for {}:", target.name);
        println!("    Min: {} ms | Median: {} ms | p90: {} ms | Max: {} ms | Median RTF: {:.3}x",
            warm_min, warm_median, warm_p90, warm_max, median_rtf);
        println!("    Accuracy : WER = {:.2}% | CER = {:.2}% (Sub: {}, Del: {}, Ins: {})",
            acc_res.wer * 100.0, acc_res.cer * 100.0, acc_res.substitutions, acc_res.deletions, acc_res.insertions);
        println!("    CPU Impact: Avg = {:.1}% | Peak = {:.1}%", overall_avg_cpu, overall_peak_cpu);

        // 3. SHORT-UTTERANCE BENCHMARK
        println!("\n  [Short-Utterance Benchmark]");
        println!("  {:<10} | {:>8} | {:>10} | {:>8} | Transcript", "Window", "Duration", "Decode(ms)", "RTF");
        println!("  {}", "-".repeat(90));

        let short_engine = SttEngine::new();
        // Warm the short engine
        if target.engine == "parakeet" {
            let _ = ParakeetRecognizer::new(short_engine.clone(), &target.path).transcribe(RecognitionRequest {
                samples: &samples_53s[0..16000],
                language: Some("en".to_string()),
                translate: false,
            });
        } else {
            let _ = WhisperRecognizer::new(short_engine.clone(), &target.path, whisper_dec_cfg.clone()).transcribe(RecognitionRequest {
                samples: &samples_53s[0..16000],
                language: Some("en".to_string()),
                translate: false,
            });
        }

        let mut short_window_samples = Vec::new();

        for &(label, dur_s) in &short_windows_def {
            let slice_samples_count = ((dur_s * 16000.0).round() as usize).min(samples_53s.len());
            let slice = &samples_53s[0..slice_samples_count];

            let t_slice_start = Instant::now();
            let text = if target.engine == "parakeet" {
                let rec = ParakeetRecognizer::new(short_engine.clone(), &target.path);
                rec.transcribe(RecognitionRequest {
                    samples: slice,
                    language: Some("en".to_string()),
                    translate: false,
                })
                .map(|r| r.text)
                .unwrap_or_else(|e| format!("ERROR: {e}"))
            } else {
                let rec = WhisperRecognizer::new(short_engine.clone(), &target.path, whisper_dec_cfg.clone());
                rec.transcribe(RecognitionRequest {
                    samples: slice,
                    language: Some("en".to_string()),
                    translate: false,
                })
                .map(|r| r.text)
                .unwrap_or_else(|e| format!("ERROR: {e}"))
            };
            let slice_dur_ms = t_slice_start.elapsed().as_millis();
            let slice_rtf = (slice_dur_ms as f64 / 1000.0) / dur_s;

            println!("  {:<10} | {:>7.1}s | {:>8}ms | {:>7.3}x | {}",
                label, dur_s, slice_dur_ms, slice_rtf, text.trim());

            short_window_samples.push(ShortWindowSample {
                window_label: label.to_string(),
                duration_s: dur_s,
                sample_count: slice_samples_count,
                decode_ms: slice_dur_ms,
                rtf: slice_rtf,
                transcript: text.trim().to_string(),
            });
        }

        benchmark_results.push(ModelFullBenchmark {
            model_id: target.id.to_string(),
            engine: target.engine.to_string(),
            model_name: target.name.to_string(),
            filename: target.filename.to_string(),
            file_size_bytes,
            parameters_millions: target.parameters_millions,
            precision: target.precision.to_string(),
            backend: backend_str,
            thread_count,
            gpu_used: false,
            cold_load_ms,
            cold_stt_ms,
            cold_total_ms,
            warm_min_ms: warm_min,
            warm_median_ms: warm_median,
            warm_p90_ms: warm_p90,
            warm_max_ms: warm_max,
            median_rtf,
            accuracy: accuracy_data,
            avg_cpu_percent: overall_avg_cpu,
            peak_cpu_percent: overall_peak_cpu,
            runs: run_samples,
            short_windows: short_window_samples,
            raw_transcript: primary_transcript,
        });
    }

    let report = BenchmarkSuiteReport {
        created_at: chrono::Local::now().to_rfc3339(),
        audio_filename: "test_7aec643b_1790015082009.wav".to_string(),
        audio_duration_seconds: total_audio_duration_s,
        reference_transcript,
        logical_cpu_cores: num_logical_cores,
        models: benchmark_results,
    };

    let repo_root = manifest_dir.parent().unwrap().parent().unwrap();
    let json_out_path = repo_root.join("phase4_5_stt_model_benchmark.json");
    let json_str = serde_json::to_string_pretty(&report).expect("Failed to serialize benchmark report");
    fs::write(&json_out_path, json_str).expect("Failed to write phase4_5_stt_model_benchmark.json");
    println!("\nBenchmark results successfully written to: {}", json_out_path.display());
}
