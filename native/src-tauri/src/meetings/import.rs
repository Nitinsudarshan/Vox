//! Importing an existing recording, and re-transcribing one Vox already has.
//!
//! Both are the same job with a different source: decode audio, run it
//! through the same segmenter the live pipeline uses, decode each segment
//! with the same engine, and write the same transcript. Sharing the segmenter
//! is the point — an imported meeting and a recorded one have to produce
//! transcripts that look alike, or the summary templates behave differently
//! on each.
//!
//! ## Decoded in a stream, not into memory
//!
//! Audio is decoded packet by packet and handed straight to the segmenter.
//! Meetily decodes the whole file into a `Vec<f32>` at 48 kHz and guards only
//! against files over 20 GB — a three-hour recording is about 2 GB of f32
//! before VAD copies the segments out again. Streaming makes peak memory a
//! function of segment length rather than file length.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use symphonia::core::audio::SampleBuffer;
use symphonia::core::codecs::{DecoderOptions, CODEC_TYPE_NULL};
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;
use tauri::{AppHandle, Emitter};

use crate::capture::speech_health::{self, DecodeEvidence};
use crate::capture::stt::{join_utterance_text, SttEngine, SttLanguageConfig, WhisperDecodingConfig};

use super::checkpoint::{self, CheckpointWriter};
use super::model::{Meeting, MeetingSource, MeetingState, TranscriptSegment};
use super::segmenter::{Segmenter, SEGMENT_SAMPLE_RATE};
use super::store::{MeetingStore, MeetingStoreError};

/// Import and re-transcription progress.
pub const IMPORT_PROGRESS_EVENT: &str = "meeting-import-progress";

/// Audio containers Vox can read.
pub const SUPPORTED_EXTENSIONS: &[&str] = &[
    "wav", "mp3", "m4a", "mp4", "aac", "flac", "ogg", "oga", "opus", "caf",
];

/// Samples decoded before the segmenter is fed. Roughly a second, which keeps
/// the progress events smooth without making the loop chatty.
const PUMP_SAMPLES: usize = SEGMENT_SAMPLE_RATE as usize;

#[derive(Debug, thiserror::Error)]
pub enum ImportError {
    #[error("{0} is not an audio format Vox can read")]
    UnsupportedFormat(String),

    #[error("that file could not be read: {0}")]
    Io(#[from] std::io::Error),

    #[error("that audio could not be decoded: {0}")]
    Decode(String),

    #[error("that file contains no audio")]
    NoAudio,

    #[error("no speech model is installed")]
    NoSpeechModel,

    #[error(transparent)]
    Store(#[from] MeetingStoreError),

    #[error("this meeting has no saved recording to transcribe again")]
    NoRecording,

    #[error("cancelled")]
    Cancelled,
}

impl From<super::checkpoint::CheckpointError> for ImportError {
    fn from(err: super::checkpoint::CheckpointError) -> Self {
        ImportError::Decode(err.to_string())
    }
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct ImportProgress {
    pub meeting_id: String,
    pub stage: String,
    pub processed_seconds: f64,
    /// `None` for a container that does not declare its length.
    pub total_seconds: Option<f64>,
    pub fraction: Option<f32>,
    pub segments: usize,
}

/// Everything a batch transcription run needs.
pub struct BatchConfig {
    pub model_path: String,
    pub language: SttLanguageConfig,
    pub decoding: WhisperDecodingConfig,
    pub glossary: Vec<String>,
}

/// Whether Vox will attempt this file.
pub fn is_supported(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| ext.to_lowercase())
        .is_some_and(|ext| SUPPORTED_EXTENSIONS.contains(&ext.as_str()))
}

/// Imports an audio file as a completed meeting.
///
/// Blocking and CPU-bound — call it from a blocking context, not from the
/// async runtime.
pub fn import_audio(
    app: Option<AppHandle>,
    store: &Arc<MeetingStore>,
    engine: &SttEngine,
    source: &Path,
    title: Option<String>,
    config: BatchConfig,
    cancel: Arc<AtomicBool>,
) -> Result<Meeting, ImportError> {
    if !source.exists() {
        return Err(ImportError::Io(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            source.display().to_string(),
        )));
    }
    if !is_supported(source) {
        return Err(ImportError::UnsupportedFormat(
            source
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or("that file")
                .to_string(),
        ));
    }
    if config.model_path.trim().is_empty() {
        return Err(ImportError::NoSpeechModel);
    }

    let id = format!("meeting-{}", uuid::Uuid::new_v4().simple());
    let title = title
        .map(|t| t.trim().to_string())
        .filter(|t| !t.is_empty())
        .unwrap_or_else(|| title_from_filename(source));

    let mut meeting = Meeting::new(id.clone(), title, MeetingSource::Imported);
    meeting.state = MeetingState::Transcribing;
    meeting.transcript_model = Some(config.model_path.clone());
    // An imported file is one mixed track: there is no second stream to
    // attribute anything to, so every segment will be `Mixed`. Saying the
    // system audio was not captured is what stops a summary being written as
    // though speaker labels meant something.
    meeting.system_audio_captured = false;
    store.create(&meeting)?;

    let result = run_batch(
        app.clone(),
        store,
        engine,
        &id,
        AudioSource::File(source.to_path_buf()),
        &config,
        &cancel,
        true,
    );

    finish(store, &id, result, app)
}

/// Transcribes a meeting's saved recording again, replacing its transcript.
///
/// Used to re-run a meeting through a better model. The audio is untouched;
/// the transcript is replaced wholesale rather than merged, because a second
/// model's segmentation will not line up with the first's.
pub fn retranscribe(
    app: Option<AppHandle>,
    store: &Arc<MeetingStore>,
    engine: &SttEngine,
    meeting_id: &str,
    config: BatchConfig,
    cancel: Arc<AtomicBool>,
) -> Result<Meeting, ImportError> {
    if config.model_path.trim().is_empty() {
        return Err(ImportError::NoSpeechModel);
    }
    let meeting = store.load_meeting(meeting_id)?;
    let audio_path = meeting
        .audio_path
        .as_deref()
        .map(PathBuf::from)
        .filter(|path| path.exists())
        .or_else(|| {
            let candidate = store
                .audio_dir(meeting_id)
                .ok()?
                .join(checkpoint::MERGED_AUDIO_FILE);
            candidate.exists().then_some(candidate)
        })
        .ok_or(ImportError::NoRecording)?;

    // Keep the old transcript until the new one is written: a re-transcription
    // that fails halfway must not leave the meeting with nothing.
    let previous = store.load_transcript(meeting_id)?;

    store.update_meeting(meeting_id, |record| {
        record.state = MeetingState::Transcribing;
        record.transcript_model = Some(config.model_path.clone());
    })?;

    let result = run_batch(
        app.clone(),
        store,
        engine,
        meeting_id,
        AudioSource::File(audio_path),
        &config,
        &cancel,
        false,
    );

    if result.is_err() {
        if let Err(err) = store.save_transcript(meeting_id, &previous) {
            tracing::error!(
                "meeting {}: could not restore the previous transcript: {}",
                meeting_id,
                err
            );
        }
    }
    finish(store, meeting_id, result, app)
}

/// Writes the final record for a batch run, whichever way it went.
fn finish(
    store: &Arc<MeetingStore>,
    meeting_id: &str,
    result: Result<BatchOutcome, ImportError>,
    app: Option<AppHandle>,
) -> Result<Meeting, ImportError> {
    match result {
        Ok(outcome) => {
            let meeting = store.update_meeting(meeting_id, |record| {
                record.state = MeetingState::Completed;
                record.duration_seconds = outcome.duration_seconds;
                record.segment_count = outcome.segments;
                record.error = None;
                if let Some(path) = &outcome.audio_path {
                    record.audio_path = Some(path.clone());
                }
            })?;
            emit(
                &app,
                meeting_id,
                "Done",
                outcome.duration_seconds,
                Some(outcome.duration_seconds),
                outcome.segments,
            );
            Ok(meeting)
        }
        Err(err) => {
            let cancelled = matches!(err, ImportError::Cancelled);
            let _ = store.update_meeting(meeting_id, |record| {
                record.state = if cancelled {
                    MeetingState::Completed
                } else {
                    MeetingState::Failed
                };
                record.error = Some(err.to_string());
            });
            emit(&app, meeting_id, "Failed", 0.0, None, 0);
            Err(err)
        }
    }
}

struct BatchOutcome {
    duration_seconds: f64,
    segments: usize,
    audio_path: Option<String>,
}

enum AudioSource {
    File(PathBuf),
}

/// Decodes a source and transcribes it into a meeting's transcript.
///
/// `save_audio` copies the decoded audio into the meeting's own directory as
/// `audio.wav`, which is what makes an imported meeting playable and
/// re-transcribable. Re-transcription passes `false` — it is already reading
/// that file.
#[allow(clippy::too_many_arguments)]
fn run_batch(
    app: Option<AppHandle>,
    store: &Arc<MeetingStore>,
    engine: &SttEngine,
    meeting_id: &str,
    source: AudioSource,
    config: &BatchConfig,
    cancel: &Arc<AtomicBool>,
    save_audio: bool,
) -> Result<BatchOutcome, ImportError> {
    let AudioSource::File(path) = source;

    let mut writer = if save_audio {
        Some(CheckpointWriter::new(store.audio_dir(meeting_id)?)?)
    } else {
        None
    };

    // A fresh transcript: a re-run replaces, it does not append.
    store.save_transcript(meeting_id, &[])?;

    let mut segmenter = Segmenter::new();
    let mut sequence: u64 = 0;
    let mut kept = 0usize;
    let mut total_seconds: Option<f64> = None;
    let mut last_emit = std::time::Instant::now();

    {
        let mut sink = |chunk: &[f32], declared_total: Option<f64>| -> Result<(), ImportError> {
            if cancel.load(Ordering::SeqCst) {
                return Err(ImportError::Cancelled);
            }
            if total_seconds.is_none() {
                total_seconds = declared_total;
            }
            if let Some(writer) = writer.as_mut() {
                writer.push(chunk).map_err(|err| ImportError::Decode(err.to_string()))?;
            }
            for segment in segmenter.push(chunk, &[], &[]) {
                if let Some(transcript) =
                    decode_one(engine, config, &segment, sequence)
                {
                    store.append_segments(meeting_id, std::slice::from_ref(&transcript))?;
                    kept += 1;
                }
                sequence += 1;
                if cancel.load(Ordering::SeqCst) {
                    return Err(ImportError::Cancelled);
                }
            }
            if last_emit.elapsed() >= std::time::Duration::from_millis(500) {
                last_emit = std::time::Instant::now();
                let processed = segmenter.position_seconds();
                emit(
                    &app,
                    meeting_id,
                    "Transcribing",
                    processed,
                    total_seconds,
                    kept,
                );
            }
            Ok(())
        };

        decode_streaming(&path, &mut sink)?;
    }

    // Whatever was still open when the file ended.
    for segment in segmenter.flush() {
        if cancel.load(Ordering::SeqCst) {
            return Err(ImportError::Cancelled);
        }
        if let Some(transcript) = decode_one(engine, config, &segment, sequence) {
            store.append_segments(meeting_id, std::slice::from_ref(&transcript))?;
            kept += 1;
        }
        sequence += 1;
    }

    let duration_seconds = segmenter.position_seconds();
    if duration_seconds <= 0.0 {
        return Err(ImportError::NoAudio);
    }

    let audio_path = match writer {
        Some(writer) => match writer.finalize() {
            Ok(path) => Some(path.to_string_lossy().to_string()),
            Err(err) => {
                tracing::error!("meeting {}: could not save imported audio: {}", meeting_id, err);
                None
            }
        },
        None => None,
    };

    Ok(BatchOutcome {
        duration_seconds,
        segments: kept,
        audio_path,
    })
}

/// Decodes one segment, screening the result the same way the live worker
/// does.
fn decode_one(
    engine: &SttEngine,
    config: &BatchConfig,
    segment: &super::segmenter::SpeechSegment,
    sequence: u64,
) -> Option<TranscriptSegment> {
    let profile = speech_health::profile_speech(&segment.samples, SEGMENT_SAMPLE_RATE);
    let (utterances, _) = engine
        .transcribe_utterances_with_config(
            Some(&config.model_path),
            &segment.samples,
            &config.language,
            &config.decoding,
        )
        .map_err(|err| {
            tracing::warn!("imported segment {} failed to decode: {}", sequence, err);
            err
        })
        .ok()?;

    let text = join_utterance_text(&utterances);
    if text.trim().is_empty() {
        return None;
    }
    let mean_no_speech_prob = if utterances.is_empty() {
        1.0
    } else {
        utterances.iter().map(|u| u.no_speech_prob).sum::<f32>() / utterances.len() as f32
    };
    let evidence = DecodeEvidence {
        voiced_seconds: profile.voiced_seconds,
        total_seconds: profile.total_seconds,
        mean_no_speech_prob,
    };
    if speech_health::screen_decode(
        "meeting-import",
        config.language.whisper_language.as_deref(),
        &text,
        evidence,
    )
    .is_some()
    {
        return None;
    }

    let normalized = crate::capture::text_normalize::normalize_segment_text(&text, &config.glossary);
    Some(TranscriptSegment {
        sequence,
        text: normalized.text,
        start_seconds: segment.start_seconds,
        end_seconds: segment.end_seconds,
        channel: segment.channel,
        no_speech_prob: mean_no_speech_prob,
        recorded_at: chrono::Utc::now().to_rfc3339(),
    })
}

/// What [`decode_streaming`] hands each block of audio to.
///
/// The second argument is the container's declared total duration, where it
/// has one, so a caller can show progress as a fraction. Returning `Err` stops
/// the decode — which is how cancellation reaches it.
pub type AudioSink<'a> = dyn FnMut(&[f32], Option<f64>) -> Result<(), ImportError> + 'a;

/// Decodes an audio file, handing the caller 16 kHz mono blocks as they
/// arrive.
///
pub fn decode_streaming(path: &Path, sink: &mut AudioSink<'_>) -> Result<(), ImportError> {
    let file = std::fs::File::open(path)?;
    let stream = MediaSourceStream::new(Box::new(file), Default::default());

    let mut hint = Hint::new();
    if let Some(extension) = path.extension().and_then(|e| e.to_str()) {
        hint.with_extension(extension);
    }

    let probed = symphonia::default::get_probe()
        .format(
            &hint,
            stream,
            &FormatOptions {
                enable_gapless: true,
                ..Default::default()
            },
            &MetadataOptions::default(),
        )
        .map_err(|err| ImportError::Decode(err.to_string()))?;
    let mut format = probed.format;

    let track = format
        .tracks()
        .iter()
        .find(|track| track.codec_params.codec != CODEC_TYPE_NULL)
        .ok_or(ImportError::NoAudio)?;
    let track_id = track.id;
    let params = track.codec_params.clone();

    let declared_total = match (params.n_frames, params.sample_rate) {
        (Some(frames), Some(rate)) if rate > 0 => Some(frames as f64 / rate as f64),
        _ => None,
    };

    let mut decoder = symphonia::default::get_codecs()
        .make(&params, &DecoderOptions::default())
        .map_err(|err| ImportError::Decode(err.to_string()))?;

    let mut pending: Vec<f32> = Vec::with_capacity(PUMP_SAMPLES * 2);
    let mut sample_buf: Option<SampleBuffer<f32>> = None;
    let mut source_rate = params.sample_rate.unwrap_or(SEGMENT_SAMPLE_RATE);
    let mut channels;
    let mut decoded_anything = false;

    loop {
        let packet = match format.next_packet() {
            Ok(packet) => packet,
            // Symphonia signals end-of-stream as an IO error; anything else
            // is a real read failure.
            Err(symphonia::core::errors::Error::IoError(err))
                if err.kind() == std::io::ErrorKind::UnexpectedEof =>
            {
                break
            }
            Err(symphonia::core::errors::Error::ResetRequired) => break,
            Err(err) => return Err(ImportError::Decode(err.to_string())),
        };
        if packet.track_id() != track_id {
            continue;
        }

        let decoded = match decoder.decode(&packet) {
            Ok(decoded) => decoded,
            // A corrupt packet in the middle of a long recording should cost
            // that packet, not the import.
            Err(symphonia::core::errors::Error::DecodeError(err)) => {
                tracing::warn!("skipping a corrupt audio packet: {}", err);
                continue;
            }
            Err(symphonia::core::errors::Error::IoError(_)) => break,
            Err(err) => return Err(ImportError::Decode(err.to_string())),
        };

        let spec = *decoded.spec();
        source_rate = spec.rate;
        channels = spec.channels.count().max(1);

        let buffer = sample_buf.get_or_insert_with(|| {
            SampleBuffer::<f32>::new(decoded.capacity() as u64, spec)
        });
        buffer.copy_interleaved_ref(decoded);

        let interleaved = buffer.samples();
        if channels == 1 {
            pending.extend_from_slice(interleaved);
        } else {
            pending.extend(
                interleaved
                    .chunks(channels)
                    .map(|frame| frame.iter().sum::<f32>() / frame.len() as f32),
            );
        }
        decoded_anything = true;

        if pending.len() >= PUMP_SAMPLES {
            let block = std::mem::take(&mut pending);
            let resampled = crate::capture::resample_to_16k_mono(&block, source_rate);
            if !resampled.is_empty() {
                sink(&resampled, declared_total)?;
            }
            pending = Vec::with_capacity(PUMP_SAMPLES * 2);
        }
    }

    if !pending.is_empty() {
        let resampled = crate::capture::resample_to_16k_mono(&pending, source_rate);
        if !resampled.is_empty() {
            sink(&resampled, declared_total)?;
        }
    }

    if !decoded_anything {
        return Err(ImportError::NoAudio);
    }
    Ok(())
}

/// A meeting title from a filename: `team-sync-2026.m4a` → `team sync 2026`.
///
/// Two names have no title in them and fall back instead. A stem carrying no
/// letter or digit (`---.mp3`), and a dotfile — for `.wav` the filesystem
/// reports the whole name as the stem, so the naive reading puts a meeting
/// called ".wav" in the list.
fn title_from_filename(path: &Path) -> String {
    const FALLBACK: &str = "Imported recording";

    let Some(stem) = path.file_stem().and_then(|stem| stem.to_str()) else {
        return FALLBACK.to_string();
    };
    if stem.starts_with('.') {
        return FALLBACK.to_string();
    }
    let derived = stem.replace(['_', '-'], " ");
    let trimmed = derived.trim();
    if trimmed.chars().any(char::is_alphanumeric) {
        trimmed.to_string()
    } else {
        FALLBACK.to_string()
    }
}

fn emit(
    app: &Option<AppHandle>,
    meeting_id: &str,
    stage: &str,
    processed_seconds: f64,
    total_seconds: Option<f64>,
    segments: usize,
) {
    let Some(app) = app else { return };
    let fraction = total_seconds
        .filter(|total| *total > 0.0)
        .map(|total| (processed_seconds / total).clamp(0.0, 1.0) as f32);
    let _ = app.emit(
        IMPORT_PROGRESS_EVENT,
        ImportProgress {
            meeting_id: meeting_id.to_string(),
            stage: stage.to_string(),
            processed_seconds,
            total_seconds,
            fraction,
            segments,
        },
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "vox-import-{}-{}-{:?}",
            name,
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// Writes `seconds` of a tone as a WAV at `rate`, with `channels`.
    fn write_wav(path: &Path, seconds: f64, rate: u32, channels: u16) {
        let spec = hound::WavSpec {
            channels,
            sample_rate: rate,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        let mut writer = hound::WavWriter::create(path, spec).unwrap();
        let frames = (seconds * rate as f64) as usize;
        for i in 0..frames {
            let t = i as f32 / rate as f32;
            let value = ((t * 220.0 * std::f32::consts::TAU).sin() * 0.3 * i16::MAX as f32) as i16;
            for _ in 0..channels {
                writer.write_sample(value).unwrap();
            }
        }
        writer.finalize().unwrap();
    }

    #[test]
    fn the_formats_vox_offers_are_the_ones_it_accepts() {
        assert!(is_supported(Path::new("a.wav")));
        assert!(is_supported(Path::new("a.MP3")));
        assert!(is_supported(Path::new("call.m4a")));
        assert!(is_supported(Path::new("call.flac")));
        assert!(!is_supported(Path::new("notes.txt")));
        assert!(!is_supported(Path::new("noextension")));
    }

    #[test]
    fn a_wav_decodes_to_the_pipeline_rate_in_streamed_blocks() {
        let dir = temp_dir("decode");
        let path = dir.join("source.wav");
        write_wav(&path, 3.0, 44_100, 1);

        let mut total = 0usize;
        let mut blocks = 0usize;
        let mut declared = None;
        decode_streaming(&path, &mut |chunk, total_seconds| {
            total += chunk.len();
            blocks += 1;
            declared = declared.or(total_seconds);
            Ok(())
        })
        .expect("decode");

        let seconds = total as f64 / SEGMENT_SAMPLE_RATE as f64;
        assert!((seconds - 3.0).abs() < 0.1, "decoded {seconds}s");
        assert!(blocks > 1, "audio should arrive in a stream, not one block");
        assert!(declared.is_some(), "a WAV declares its length");
    }

    #[test]
    fn a_stereo_source_is_averaged_to_mono() {
        let dir = temp_dir("stereo");
        let path = dir.join("stereo.wav");
        write_wav(&path, 1.0, 16_000, 2);

        let mut total = 0usize;
        decode_streaming(&path, &mut |chunk, _| {
            total += chunk.len();
            Ok(())
        })
        .expect("decode");

        let seconds = total as f64 / SEGMENT_SAMPLE_RATE as f64;
        assert!((seconds - 1.0).abs() < 0.05, "got {seconds}s, channels not merged");
    }

    #[test]
    fn a_cancelled_decode_stops_at_the_next_block() {
        let dir = temp_dir("cancel");
        let path = dir.join("long.wav");
        write_wav(&path, 10.0, 16_000, 1);

        let mut blocks = 0usize;
        let result = decode_streaming(&path, &mut |_, _| {
            blocks += 1;
            if blocks >= 2 {
                Err(ImportError::Cancelled)
            } else {
                Ok(())
            }
        });

        assert!(matches!(result, Err(ImportError::Cancelled)));
        assert_eq!(blocks, 2, "decoding should stop, not run to the end");
    }

    #[test]
    fn a_file_that_is_not_audio_reports_that_rather_than_panicking() {
        let dir = temp_dir("garbage");
        let path = dir.join("fake.wav");
        std::fs::write(&path, b"this is definitely not a wav file").unwrap();
        assert!(decode_streaming(&path, &mut |_, _| Ok(())).is_err());
    }

    #[test]
    fn importing_an_unsupported_extension_is_refused_before_a_meeting_exists() {
        let dir = temp_dir("unsupported");
        let store = Arc::new(MeetingStore::new(dir.join("vault")));
        let path = dir.join("notes.txt");
        std::fs::write(&path, b"hello").unwrap();

        let result = import_audio(
            None,
            &store,
            &SttEngine::new(),
            &path,
            None,
            BatchConfig {
                model_path: "/models/ggml-small.bin".into(),
                language: SttLanguageConfig {
                    whisper_language: None,
                    translate: false,
                },
                decoding: WhisperDecodingConfig::default(),
                glossary: Vec::new(),
            },
            Arc::new(AtomicBool::new(false)),
        );

        assert!(matches!(result, Err(ImportError::UnsupportedFormat(_))));
        assert!(store.list_meetings().unwrap().is_empty());
    }

    #[test]
    fn importing_a_missing_file_reports_it_rather_than_creating_a_meeting() {
        let dir = temp_dir("missing");
        let store = Arc::new(MeetingStore::new(dir.join("vault")));
        let result = import_audio(
            None,
            &store,
            &SttEngine::new(),
            &dir.join("nope.wav"),
            None,
            BatchConfig {
                model_path: "/models/ggml-small.bin".into(),
                language: SttLanguageConfig {
                    whisper_language: None,
                    translate: false,
                },
                decoding: WhisperDecodingConfig::default(),
                glossary: Vec::new(),
            },
            Arc::new(AtomicBool::new(false)),
        );
        assert!(matches!(result, Err(ImportError::Io(_))));
        assert!(store.list_meetings().unwrap().is_empty());
    }

    #[test]
    fn importing_without_a_speech_model_is_refused_up_front() {
        let dir = temp_dir("nomodel");
        let store = Arc::new(MeetingStore::new(dir.join("vault")));
        let path = dir.join("call.wav");
        write_wav(&path, 1.0, 16_000, 1);

        let result = import_audio(
            None,
            &store,
            &SttEngine::new(),
            &path,
            None,
            BatchConfig {
                model_path: "   ".into(),
                language: SttLanguageConfig {
                    whisper_language: None,
                    translate: false,
                },
                decoding: WhisperDecodingConfig::default(),
                glossary: Vec::new(),
            },
            Arc::new(AtomicBool::new(false)),
        );

        assert!(matches!(result, Err(ImportError::NoSpeechModel)));
        assert!(store.list_meetings().unwrap().is_empty());
    }

    #[test]
    fn re_transcribing_a_meeting_with_no_recording_says_so() {
        let dir = temp_dir("norecording");
        let store = Arc::new(MeetingStore::new(dir.join("vault")));
        let meeting = Meeting::new("meeting-x".into(), "No audio".into(), MeetingSource::Recorded);
        store.create(&meeting).unwrap();

        let result = retranscribe(
            None,
            &store,
            &SttEngine::new(),
            "meeting-x",
            BatchConfig {
                model_path: "/models/ggml-small.bin".into(),
                language: SttLanguageConfig {
                    whisper_language: None,
                    translate: false,
                },
                decoding: WhisperDecodingConfig::default(),
                glossary: Vec::new(),
            },
            Arc::new(AtomicBool::new(false)),
        );

        assert!(matches!(result, Err(ImportError::NoRecording)));
        // The meeting is untouched: still whatever it was before.
        assert_eq!(
            store.load_meeting("meeting-x").unwrap().state,
            MeetingState::Recording
        );
    }

    #[test]
    fn a_title_is_derived_from_the_filename_when_none_is_given() {
        assert_eq!(
            title_from_filename(Path::new("/tmp/team-sync_2026.m4a")),
            "team sync 2026"
        );
        // A name with nothing in it but punctuation is not a title. `.wav` is
        // this case: the leading dot makes the whole name the stem.
        for nameless in ["/tmp/.wav", "/tmp/ .wav", "/tmp/---.mp3", "/tmp/.hidden.m4a"] {
            assert_eq!(
                title_from_filename(Path::new(nameless)),
                "Imported recording",
                "{nameless} should fall back"
            );
        }
    }
}
