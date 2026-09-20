//! The Vox side of the ASR shootout: one engine configuration over one file.
//!
//! This binary is an **adapter**, not a comparison. It decodes one recording
//! under one explicitly named configuration and writes what it did alongside
//! what it produced. Comparing configurations is
//! `tests/transcription/runner/shootout.py`'s job, because half the engines
//! worth comparing against do not live in this process.
//!
//! ## Why the configuration is an argument and not a default
//!
//! The version of this file that came before took none of it. It built
//! [`WhisperDecodingConfig::baseline`] — greedy, `best_of = 1`, and
//! `trim_audio_context` off — while a live meeting builds
//! [`WhisperDecodingConfig::for_meetings`], which is beam search at 3 with the
//! encoder clamped to each segment's own audio. So the committed baseline
//! measured a configuration Vox does not ship, and its real-time factor was
//! paying a full thirty-second encoder window for every segment.
//!
//! A benchmark whose configuration is implicit cannot be compared with
//! anything, including itself a month later. Every knob that changes the
//! answer is now a flag, every flag is recorded in the output, and the
//! defaults are production.
//!
//! ## Modes
//!
//! ```text
//! --segmentation vox         audio -> Segmenter -> queue -> spawn_worker
//! --segmentation whole-file  audio -> one decode
//! --audio-stats-only         audio -> signal measurements, no model needed
//! ```
//!
//! The first two are the experiment that asks whether Vox's own segmentation
//! helps or hurts, which needs the same engine on both sides of it. The third
//! answers a question that has to come first: whether the recording is good
//! enough for the comparison to mean anything at all.

use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::time::Instant;

use serde::{Deserialize, Serialize};
use vox_lib::capture::stt::{
    join_utterance_text, SttEngine, SttLanguageConfig, SttPreset, WhisperDecodingConfig,
};
use vox_lib::capture::vocabulary::DomainVocabulary;
use vox_lib::meetings::segmenter::{Segmenter, SEGMENT_SAMPLE_RATE};
use vox_lib::meetings::store::MeetingStore;
use vox_lib::meetings::transcription::{spawn_worker, PromptContext, WorkerConfig};
use vox_lib::settings::SttSettings;

/// How audio reaches the decoder.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
enum SegmentationMode {
    /// Through the production segmenter and the production worker. What Vox
    /// does.
    Vox,
    /// One decode over the whole recording, with whisper's own internal
    /// windowing and nothing of Vox's in between. The control for asking
    /// whether Vox's segmentation is helping.
    WholeFile,
}

impl SegmentationMode {
    fn parse(value: &str) -> Option<Self> {
        match value.trim().to_lowercase().as_str() {
            "vox" | "segmented" => Some(Self::Vox),
            "whole-file" | "wholefile" | "whole" => Some(Self::WholeFile),
            _ => None,
        }
    }

    fn key(self) -> &'static str {
        match self {
            Self::Vox => "vox",
            Self::WholeFile => "whole-file",
        }
    }
}

/// Which decode profile the run uses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DecodePath {
    /// `for_meetings` — what a recording in progress decodes with, encoder
    /// clamp included.
    Live,
    /// `for_meeting_batch` — what a re-transcription decodes with. No clock to
    /// race, so nothing is traded for speed.
    Batch,
}

impl DecodePath {
    fn parse(value: &str) -> Option<Self> {
        match value.trim().to_lowercase().as_str() {
            "live" | "meeting" => Some(Self::Live),
            "batch" | "final" => Some(Self::Batch),
            _ => None,
        }
    }

    fn key(self) -> &'static str {
        match self {
            Self::Live => "live",
            Self::Batch => "batch",
        }
    }
}

/// Everything about this run that changes the answer.
///
/// §19 of the ASR reassessment, as a struct: a result without this is not a
/// result. Recorded in the output file rather than printed, because the thing
/// that reads it is a comparison across several of these.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunStamp {
    /// Which Vox built this. A baseline is only frozen if it can be named.
    pub vox_version: String,
    pub engine: String,
    /// Filename, never the path: a report gets shared, and a path names a
    /// machine and usually a person.
    pub model: String,
    /// Whatever the caller says the weights are quantized to. Recorded and not
    /// verified — the file knows and this binary does not, and a guess here
    /// would be worse than a blank.
    pub quantization: Option<String>,
    pub decode_path: String,
    pub preset: String,
    pub strategy: String,
    pub temperature_inc: f32,
    pub trim_audio_context: bool,
    pub n_threads: Option<i32>,
    /// `auto` when nothing was pinned.
    pub language_mode: String,
    pub context_mode: String,
    pub segmentation_mode: String,
}

/// What the recording itself is like, before any model touches it.
///
/// §21: if the audio is clipped, no model choice fixes it, and a shootout run
/// over it is measuring the microphone. Every field here comes from the same
/// `AudioStats` the live pipeline records per segment, so the benchmark and
/// the product cannot disagree about what "near clipping" means.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AudioQualityReport {
    pub duration_seconds: f64,
    pub rms: f32,
    pub peak_amplitude: f32,
    pub near_clipping_percent: f32,
    pub near_zero_percent: f32,
    pub has_non_finite: bool,
    pub voiced_seconds: f64,
    pub voiced_ratio: f64,
    pub noise_floor_rms: f32,
    /// `20·log10(rms / noise_floor)`. An estimate and labelled as one: it is
    /// the whole file's energy against its own quiet percentile, not speech
    /// against a separately measured noise recording.
    pub snr_db_estimate: Option<f32>,
    /// `ok`, or the reasons this recording should not settle an argument
    /// between two models.
    pub concerns: Vec<String>,
}

/// Share of samples at or above [`CLIPPING_CONCERN_PERCENT`] of full scale
/// before the recording is called clipped.
///
/// Deliberately low. `near_clipping_percent` counts samples at |s| >= 0.98,
/// and speech that has not been limited reaches that on a handful of vowel
/// peaks at most. A whole percent of the file sitting there is a gain stage
/// pinned to its ceiling, which flattens exactly the formant peaks an acoustic
/// model reads.
const CLIPPING_CONCERN_PERCENT: f32 = 1.0;

/// Below this, the recording is quiet enough that the segmenter's absolute
/// floor starts deciding what counts as speech.
const QUIET_CONCERN_RMS: f32 = 0.01;

/// Below this, there is not much room between the speech and the room.
const LOW_SNR_CONCERN_DB: f32 = 10.0;

impl AudioQualityReport {
    fn measure(samples: &[f32], duration_seconds: f64) -> Self {
        let stats = vox_lib::capture::AudioStats::compute(samples, SEGMENT_SAMPLE_RATE, 1);
        let profile = vox_lib::capture::speech_health::profile_speech(samples, SEGMENT_SAMPLE_RATE);

        let snr_db_estimate = if profile.noise_floor_rms > 0.0 && stats.rms > 0.0 {
            Some(20.0 * (stats.rms / profile.noise_floor_rms).log10())
        } else {
            None
        };

        let mut concerns = Vec::new();
        if stats.has_non_finite {
            concerns.push("audio contains NaN or infinite samples".to_string());
        }
        if stats.near_clipping_percent >= CLIPPING_CONCERN_PERCENT {
            concerns.push(format!(
                "{:.1}% of samples are at or beyond 98% of full scale — the recording is clipped, \
                 and distorted phonemes are not a model's fault",
                stats.near_clipping_percent
            ));
        }
        if stats.rms < QUIET_CONCERN_RMS {
            concerns.push(format!(
                "RMS {:.4} is very quiet; the segmenter's absolute floor, not the room, is \
                 deciding what counts as speech",
                stats.rms
            ));
        }
        if let Some(snr) = snr_db_estimate {
            if snr < LOW_SNR_CONCERN_DB {
                concerns.push(format!(
                    "estimated SNR {snr:.1} dB leaves little room between speech and background"
                ));
            }
        }

        Self {
            duration_seconds,
            rms: stats.rms,
            peak_amplitude: stats.peak_amplitude,
            near_clipping_percent: stats.near_clipping_percent,
            near_zero_percent: stats.near_zero_percent,
            has_non_finite: stats.has_non_finite,
            voiced_seconds: profile.voiced_seconds,
            voiced_ratio: profile.voiced_ratio(),
            noise_floor_rms: profile.noise_floor_rms,
            snr_db_estimate,
            concerns,
        }
    }
}

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
    /// Filename only. See [`RunStamp::model`].
    pub model_path: String,
    pub language_config: String,
    pub run: RunStamp,
    pub audio: AudioQualityReport,
    pub total_audio_duration_seconds: f64,
    pub total_speech_seconds: f64,
    pub total_inference_ms: u128,
    pub decode_real_time_factor: f64,
    pub pipeline_real_time_factor: f64,
    /// Wall clock for the whole run, decode and everything around it. The
    /// difference between this and `total_inference_ms` is what the harness
    /// itself costs, which a comparison has to be able to subtract.
    pub wall_ms: u128,
    pub segments_decoded_count: u64,
    pub segments_kept_count: u64,
    pub segments_discarded_count: u64,
    pub segments_failed_count: u64,
    /// Refused because the bounded queue was full. Non-zero means this run
    /// lost speech, and its word error rate is measuring that as much as the
    /// model.
    pub segments_dropped_count: u64,
    pub peak_queue_depth: u64,
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

/// The filename of a model, for a report that will be read on another machine.
fn model_name(path: &Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().to_string())
        .unwrap_or_else(|| path.to_string_lossy().to_string())
}

#[derive(Debug)]
struct Args {
    audio: Option<PathBuf>,
    model: Option<PathBuf>,
    language: String,
    output: Option<PathBuf>,
    meeting_title: Option<String>,
    user_terms: Option<String>,
    preset: SttPreset,
    preset_label: String,
    decode_path: DecodePath,
    segmentation: SegmentationMode,
    context: PromptContext,
    trim_audio_context: bool,
    quantization: Option<String>,
    audio_stats_only: bool,
}

impl Default for Args {
    /// Production, in every field that has a production value. A run with no
    /// flags measures what Vox actually does.
    fn default() -> Self {
        Self {
            audio: None,
            model: None,
            language: "auto".to_string(),
            output: None,
            meeting_title: None,
            user_terms: None,
            preset: SttPreset::Balanced,
            preset_label: "balanced".to_string(),
            decode_path: DecodePath::Live,
            segmentation: SegmentationMode::Vox,
            context: PromptContext::Previous,
            trim_audio_context: true,
            quantization: None,
            audio_stats_only: false,
        }
    }
}

const USAGE: &str = "\
vox benchmark — one engine configuration over one recording

  --audio <path>                 the recording (required)
  --model <path>                 a ggml Whisper model (required unless --audio-stats-only)
  --lang auto|en|hi|<code>       language to pin, or auto to let whisper decide  [auto]
  --preset fast|balanced|quality decode preset                                   [balanced]
  --decode-path live|batch       for_meetings, or for_meeting_batch              [live]
  --segmentation vox|whole-file  through Vox's segmenter, or one decode          [vox]
  --context none|static|previous what may condition each decode                  [previous]
  --trim-audio-ctx on|off        clamp the encoder to each segment's own audio   [on]
  --quantization <label>         recorded, not verified (e.g. q5_0)
  --meeting-title <text>         seeds the meeting tier of the vocabulary
  --user-terms <a,b,c>           seeds the user tier of the vocabulary
  --audio-stats-only             measure the recording and exit; needs no model
  --output <path>                write JSON here instead of stdout
  --help
";

fn parse_args(argv: &[String]) -> Result<Args, String> {
    let mut args = Args::default();
    let mut i = 1;
    while i < argv.len() {
        let flag = argv[i].as_str();
        // Every flag below except the switches takes exactly one value, so the
        // missing-value case is worth one helper rather than ten copies.
        let mut value = |name: &str| -> Result<String, String> {
            i += 1;
            argv.get(i)
                .cloned()
                .ok_or_else(|| format!("{name} needs a value"))
        };
        match flag {
            "--audio" => args.audio = Some(PathBuf::from(value("--audio")?)),
            "--model" => args.model = Some(PathBuf::from(value("--model")?)),
            "--lang" | "--language" => args.language = value("--lang")?,
            "--output" => args.output = Some(PathBuf::from(value("--output")?)),
            "--meeting-title" => args.meeting_title = Some(value("--meeting-title")?),
            "--user-terms" => args.user_terms = Some(value("--user-terms")?),
            "--quantization" => args.quantization = Some(value("--quantization")?),
            "--preset" => {
                let raw = value("--preset")?;
                // `from_setting` defaults rather than failing, which is right
                // for a settings file written by a previous version and wrong
                // for a benchmark flag: a typo would silently measure `fast`.
                let parsed = match raw.trim().to_lowercase().as_str() {
                    "fast" => SttPreset::Fast,
                    "balanced" => SttPreset::Balanced,
                    "quality" => SttPreset::Quality,
                    other => return Err(format!("--preset does not accept '{other}'")),
                };
                args.preset = parsed;
                args.preset_label = parsed.as_str().to_string();
            }
            "--decode-path" => {
                let raw = value("--decode-path")?;
                args.decode_path = DecodePath::parse(&raw)
                    .ok_or_else(|| format!("--decode-path does not accept '{raw}'"))?;
            }
            "--segmentation" => {
                let raw = value("--segmentation")?;
                args.segmentation = SegmentationMode::parse(&raw)
                    .ok_or_else(|| format!("--segmentation does not accept '{raw}'"))?;
            }
            "--context" => {
                let raw = value("--context")?;
                args.context = PromptContext::parse(&raw)
                    .ok_or_else(|| format!("--context does not accept '{raw}'"))?;
            }
            "--trim-audio-ctx" => {
                let raw = value("--trim-audio-ctx")?;
                args.trim_audio_context = match raw.trim().to_lowercase().as_str() {
                    "on" | "true" | "1" => true,
                    "off" | "false" | "0" => false,
                    other => return Err(format!("--trim-audio-ctx does not accept '{other}'")),
                };
            }
            "--audio-stats-only" => args.audio_stats_only = true,
            "--help" | "-h" => {
                print!("{USAGE}");
                std::process::exit(0);
            }
            // Refused rather than ignored. The loop this replaced skipped what
            // it did not recognise, so a misspelled flag ran production
            // settings under a name that claimed otherwise.
            other => return Err(format!("unrecognised argument '{other}'\n\n{USAGE}")),
        }
        i += 1;
    }
    Ok(args)
}

fn language_config(language: &str) -> SttLanguageConfig {
    match language.trim().to_lowercase().as_str() {
        "auto" | "" => SttLanguageConfig {
            whisper_language: None,
            translate: false,
        },
        pinned => SttLanguageConfig {
            whisper_language: Some(pinned.to_string()),
            translate: false,
        },
    }
}

fn build_vocabulary(args: &Args) -> DomainVocabulary {
    // `PromptContext::None` must reach the decoder with nothing, and the
    // vocabulary is the other half of that: a worker whose context mode is
    // `none` but whose vocabulary is full would still be measuring a prompt if
    // anything downstream ever consulted it.
    if args.context == PromptContext::None {
        return DomainVocabulary::default();
    }
    let mut vocabulary = DomainVocabulary::new();
    if let Some(title) = &args.meeting_title {
        vocabulary = vocabulary.with_meeting_terms(std::slice::from_ref(title));
    }
    if let Some(raw) = &args.user_terms {
        let terms: Vec<String> = raw
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();
        vocabulary = vocabulary.with_user_terms(&terms);
    }
    vocabulary
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let argv: Vec<String> = std::env::args().collect();
    let args = parse_args(&argv)?;

    let audio_file = args.audio.clone().ok_or("Missing required argument --audio <path>")?;
    if !audio_file.exists() {
        return Err(format!("Audio file {:?} not found", audio_file).into());
    }

    eprintln!("[vox-benchmark] Loading audio: {:?}", audio_file);
    let (samples, audio_duration) = load_wav_samples_16k_mono(&audio_file)?;
    eprintln!(
        "[vox-benchmark] Audio loaded: {:.2}s ({} samples)",
        audio_duration,
        samples.len()
    );

    let audio_quality = AudioQualityReport::measure(&samples, audio_duration);
    for concern in &audio_quality.concerns {
        eprintln!("[vox-benchmark] AUDIO CONCERN: {concern}");
    }

    if args.audio_stats_only {
        let json = serde_json::to_string_pretty(&audio_quality)?;
        return write_output(&args.output, &json);
    }

    let model_file = args.model.clone().ok_or("Missing required argument --model <path>")?;
    if !model_file.exists() {
        return Err(format!("Model file {:?} not found", model_file).into());
    }

    let lang_config = language_config(&args.language);

    // The decode configuration is resolved the way production resolves it,
    // from settings, so that "balanced + trim on" here and "balanced + trim
    // on" in a meeting are the same numbers and not two similar ones.
    let settings = SttSettings {
        preset: args.preset_label.clone(),
        meeting_trim_audio_context: args.trim_audio_context,
        ..SttSettings::default()
    };
    let decoding = match args.decode_path {
        DecodePath::Live => WhisperDecodingConfig::for_meetings(&settings, args.preset),
        DecodePath::Batch => WhisperDecodingConfig::for_meeting_batch(&settings),
    };

    let stamp = RunStamp {
        vox_version: env!("CARGO_PKG_VERSION").to_string(),
        engine: "whisper.cpp".to_string(),
        model: model_name(&model_file),
        quantization: args.quantization.clone(),
        decode_path: args.decode_path.key().to_string(),
        preset: args.preset_label.clone(),
        strategy: format!("{:?}", decoding.strategy),
        temperature_inc: decoding.temperature_inc,
        trim_audio_context: decoding.trim_audio_context,
        n_threads: decoding.n_threads,
        language_mode: lang_config
            .whisper_language
            .clone()
            .unwrap_or_else(|| "auto".to_string()),
        context_mode: args.context.key().to_string(),
        segmentation_mode: args.segmentation.key().to_string(),
    };
    eprintln!(
        "[vox-benchmark] Configuration: {} {} | {} | preset={} | {} | context={} | trim_ctx={}",
        stamp.engine,
        stamp.model,
        stamp.decode_path,
        stamp.preset,
        stamp.segmentation_mode,
        stamp.context_mode,
        stamp.trim_audio_context
    );

    let wall_start = Instant::now();
    let outcome = match args.segmentation {
        SegmentationMode::Vox => run_through_vox_pipeline(&args, &samples, &model_file, &lang_config, decoding)?,
        SegmentationMode::WholeFile => run_whole_file(&samples, &model_file, &lang_config, decoding)?,
    };
    let wall_ms = wall_start.elapsed().as_millis();

    let result = BenchmarkRunResult {
        audio_file: audio_file
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default(),
        model_path: stamp.model.clone(),
        language_config: args.language.clone(),
        run: stamp,
        audio: audio_quality,
        total_audio_duration_seconds: audio_duration,
        total_speech_seconds: outcome.speech_seconds,
        total_inference_ms: outcome.decode_ms,
        decode_real_time_factor: ratio(outcome.decode_ms as f64 / 1000.0, outcome.speech_seconds),
        pipeline_real_time_factor: ratio(outcome.decode_ms as f64 / 1000.0, audio_duration),
        wall_ms,
        segments_decoded_count: outcome.decoded,
        segments_kept_count: outcome.kept,
        segments_discarded_count: outcome.discarded,
        segments_failed_count: outcome.failed,
        segments_dropped_count: outcome.dropped,
        peak_queue_depth: outcome.peak_queue_depth,
        transcript: outcome.transcript,
        segments: outcome.segments,
    };

    let json_bytes = serde_json::to_string_pretty(&result)?;
    write_output(&args.output, &json_bytes)
}

fn write_output(path: &Option<PathBuf>, json: &str) -> Result<(), Box<dyn std::error::Error>> {
    match path {
        Some(out) => {
            std::fs::write(out, json)?;
            eprintln!("[vox-benchmark] Output written to {:?}", out);
        }
        None => println!("{json}"),
    }
    Ok(())
}

fn ratio(numerator: f64, denominator: f64) -> f64 {
    if denominator > 0.0 {
        numerator / denominator
    } else {
        0.0
    }
}

/// What a run produced, whichever way the audio reached the decoder.
struct RunOutcome {
    speech_seconds: f64,
    decode_ms: u128,
    decoded: u64,
    kept: u64,
    discarded: u64,
    failed: u64,
    dropped: u64,
    peak_queue_depth: u64,
    transcript: String,
    segments: Vec<BenchmarkSegmentOutput>,
}

/// The production path: segmenter, bounded queue, serial worker, quality gate.
fn run_through_vox_pipeline(
    args: &Args,
    samples: &[f32],
    model_file: &Path,
    lang_config: &SttLanguageConfig,
    decoding: WhisperDecodingConfig,
) -> Result<RunOutcome, Box<dyn std::error::Error>> {
    let meeting_id = format!("bench_{}", uuid::Uuid::new_v4());
    let temp_vault = std::env::temp_dir().join(format!("vox_bench_{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&temp_vault)?;
    let store = Arc::new(MeetingStore::new(&temp_vault));

    let worker_config = WorkerConfig {
        meeting_id: meeting_id.clone(),
        model_path: model_file.to_string_lossy().to_string(),
        language: lang_config.clone(),
        decoding_cheap: decoding.for_expensive_script(),
        decoding,
        glossary: Vec::new(),
        vocabulary: build_vocabulary(args),
        prompt_context: args.context,
    };

    eprintln!("[vox-benchmark] Spawning transcription worker...");
    let cancel_flag = Arc::new(AtomicBool::new(false));
    let (queue, worker_handle) = spawn_worker(
        worker_config,
        SttEngine::new(),
        Arc::clone(&store),
        None,
        cancel_flag,
    );

    eprintln!("[vox-benchmark] Streaming audio through Segmenter...");
    let mut segmenter = Segmenter::new();
    // 50 ms, which is a whole number of the segmenter's 20 ms frames and close
    // enough to the live mixer's cadence that the frame boundaries land where
    // a recording puts them.
    let chunk_size = 800;
    for chunk in samples.chunks(chunk_size) {
        for seg in segmenter.push(chunk, chunk, &[]) {
            queue.submit(seg);
        }
    }
    for seg in segmenter.flush() {
        queue.submit(seg);
    }
    let (_, _, dropped) = queue.counts();
    drop(queue);

    eprintln!("[vox-benchmark] Draining transcription queue...");
    let stats = worker_handle.join().map_err(|_| "Worker thread panicked")?;

    eprintln!(
        "[vox-benchmark] Complete: decoded={} kept={} discarded={} failed={} dropped={} decode_ms={} rtf={:.3}",
        stats.decoded, stats.kept, stats.discarded, stats.failed, dropped, stats.decode_ms, stats.decode_rtf()
    );

    let saved_segments = store.load_transcript(&meeting_id).unwrap_or_default();
    let mut transcript_parts = Vec::new();
    let mut segments = Vec::new();
    for seg in saved_segments {
        let text = seg.text.trim().to_string();
        if !text.is_empty() {
            transcript_parts.push(text.clone());
        }
        segments.push(BenchmarkSegmentOutput {
            sequence: seg.sequence,
            start_seconds: seg.start_seconds,
            end_seconds: seg.end_seconds,
            duration: seg.end_seconds - seg.start_seconds,
            channel: format!("{:?}", seg.channel),
            text,
            telemetry: seg.telemetry,
        });
    }

    let _ = std::fs::remove_dir_all(&temp_vault);

    Ok(RunOutcome {
        speech_seconds: stats.speech_seconds,
        decode_ms: stats.decode_ms,
        decoded: stats.decoded,
        kept: stats.kept,
        discarded: stats.discarded,
        failed: stats.failed,
        dropped,
        peak_queue_depth: stats.peak_in_queue,
        transcript: transcript_parts.join(" "),
        segments,
    })
}

/// The control: one decode over the whole recording.
///
/// No segmenter, no queue, no quality gate — whisper's own thirty-second
/// windowing and nothing else. This is what says whether Vox's segmentation is
/// buying accuracy or costing it, and the answer is only meaningful because
/// both sides run the same engine, the same model and the same decode
/// configuration.
///
/// The pipeline counters come back as one "segment" because that is what
/// happened. Reporting whisper's internal windows as Vox segments would invent
/// a comparison between two things that are not the same unit.
fn run_whole_file(
    samples: &[f32],
    model_file: &Path,
    lang_config: &SttLanguageConfig,
    decoding: WhisperDecodingConfig,
) -> Result<RunOutcome, Box<dyn std::error::Error>> {
    eprintln!("[vox-benchmark] Decoding the whole file in one pass...");
    let engine = SttEngine::new();
    let profile = vox_lib::capture::speech_health::profile_speech(samples, SEGMENT_SAMPLE_RATE);

    let (utterances, diagnostics) = engine
        .transcribe_utterances_with_config(
            Some(&model_file.to_string_lossy()),
            samples,
            lang_config,
            &decoding,
        )
        .map_err(|err| format!("whole-file decode failed: {err}"))?;

    let transcript = join_utterance_text(&utterances);
    let duration = samples.len() as f64 / SEGMENT_SAMPLE_RATE as f64;
    let kept = if transcript.trim().is_empty() { 0 } else { 1 };

    eprintln!(
        "[vox-benchmark] Complete: whisper segments={} decode_ms={} chars={}",
        utterances.len(),
        diagnostics.transcription_latency_ms,
        transcript.chars().count()
    );

    let segments = if kept == 1 {
        vec![BenchmarkSegmentOutput {
            sequence: 0,
            start_seconds: 0.0,
            end_seconds: duration,
            duration,
            channel: "Microphone".to_string(),
            text: transcript.clone(),
            // No `SegmentTelemetry`: every field on it describes a segment the
            // segmenter produced and a gate that screened it, and none of that
            // happened here. An invented record would make the two modes look
            // more comparable than they are.
            telemetry: None,
        }]
    } else {
        Vec::new()
    };

    Ok(RunOutcome {
        speech_seconds: profile.voiced_seconds,
        decode_ms: diagnostics.transcription_latency_ms + diagnostics.state_create_ms,
        decoded: 1,
        kept,
        discarded: 1 - kept,
        failed: 0,
        dropped: 0,
        peak_queue_depth: 0,
        transcript,
        segments,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn argv(rest: &[&str]) -> Vec<String> {
        std::iter::once("benchmark".to_string())
            .chain(rest.iter().map(|s| s.to_string()))
            .collect()
    }

    /// No flags means production, which is the whole point of the defaults:
    /// the version of this binary that came before measured greedy decoding
    /// with the encoder clamp off, and a meeting does neither.
    #[test]
    fn the_default_configuration_is_the_one_vox_ships() {
        let args = parse_args(&argv(&["--audio", "a.wav", "--model", "m.bin"])).unwrap();
        assert_eq!(args.preset_label, "balanced");
        assert_eq!(args.decode_path, DecodePath::Live);
        assert_eq!(args.segmentation, SegmentationMode::Vox);
        assert_eq!(args.context, PromptContext::Previous);
        assert!(args.trim_audio_context);

        let settings = SttSettings {
            preset: args.preset_label.clone(),
            meeting_trim_audio_context: args.trim_audio_context,
            ..SttSettings::default()
        };
        let decoding = WhisperDecodingConfig::for_meetings(&settings, args.preset);
        assert!(
            decoding.trim_audio_context,
            "a default run must pay for the audio it has, not a flat thirty seconds"
        );
        assert_ne!(
            decoding,
            WhisperDecodingConfig::baseline(),
            "the default must not be the bare baseline this binary used to measure"
        );
    }

    /// A benchmark that ignores a flag it does not recognise runs production
    /// settings under a name claiming otherwise, and nothing about the output
    /// says so.
    #[test]
    fn an_unrecognised_flag_fails_rather_than_being_skipped() {
        let err = parse_args(&argv(&["--audio", "a.wav", "--segmantation", "vox"])).unwrap_err();
        assert!(err.contains("--segmantation"), "error was {err:?}");
    }

    /// Same reason, for a value: `--preset fastt` must not quietly become
    /// `fast`, which is what `SttPreset::from_setting` would have done.
    #[test]
    fn a_misspelled_preset_fails_rather_than_defaulting() {
        let err = parse_args(&argv(&["--preset", "fastt"])).unwrap_err();
        assert!(err.contains("fastt"), "error was {err:?}");
    }

    #[test]
    fn a_flag_with_no_value_says_which_one() {
        let err = parse_args(&argv(&["--audio"])).unwrap_err();
        assert!(err.contains("--audio"), "error was {err:?}");
    }

    #[test]
    fn every_mode_round_trips_through_its_own_key() {
        for mode in [SegmentationMode::Vox, SegmentationMode::WholeFile] {
            assert_eq!(SegmentationMode::parse(mode.key()), Some(mode));
        }
        for path in [DecodePath::Live, DecodePath::Batch] {
            assert_eq!(DecodePath::parse(path.key()), Some(path));
        }
        for context in [
            PromptContext::None,
            PromptContext::Static,
            PromptContext::Previous,
        ] {
            assert_eq!(PromptContext::parse(context.key()), Some(context));
        }
    }

    /// `--context none` has to reach the decoder with nothing. The vocabulary
    /// is the other half of that, and it is built somewhere else.
    #[test]
    fn asking_for_no_context_also_empties_the_vocabulary() {
        let mut args = Args {
            context: PromptContext::None,
            meeting_title: Some("Sprint review".to_string()),
            ..Args::default()
        };
        assert!(build_vocabulary(&args).active_terms().is_empty());

        args.context = PromptContext::Static;
        assert!(!build_vocabulary(&args).active_terms().is_empty());
    }

    /// Clean speech at a sane level raises no concern; a file pinned to its
    /// ceiling does. The gate exists because a clipped recording cannot settle
    /// an argument between two models.
    #[test]
    fn the_audio_gate_flags_a_clipped_recording_and_passes_a_clean_one() {
        let sample_rate = SEGMENT_SAMPLE_RATE as f32;
        let clean: Vec<f32> = (0..sample_rate as usize * 2)
            .map(|i| 0.35 * (i as f32 * 440.0 * std::f32::consts::TAU / sample_rate).sin())
            .collect();
        let clipped: Vec<f32> = clean.iter().map(|s| (s * 8.0).clamp(-1.0, 1.0)).collect();

        let clean_report = AudioQualityReport::measure(&clean, 2.0);
        let clipped_report = AudioQualityReport::measure(&clipped, 2.0);

        assert!(
            clean_report.near_clipping_percent < CLIPPING_CONCERN_PERCENT,
            "clean audio measured {}% near clipping",
            clean_report.near_clipping_percent
        );
        assert!(
            clipped_report
                .concerns
                .iter()
                .any(|c| c.contains("clipped")),
            "concerns were {:?}",
            clipped_report.concerns
        );
    }

    #[test]
    fn silence_is_reported_as_quiet_rather_than_as_a_good_recording() {
        let report = AudioQualityReport::measure(&vec![0.0; 16_000], 1.0);
        assert!(report.concerns.iter().any(|c| c.contains("quiet")));
    }
}
