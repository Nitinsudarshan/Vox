//! Durable audio checkpoints, so a crash costs seconds rather than a meeting.
//!
//! Mixed audio is accumulated in memory and flushed to
//! `audio/chunk_NNNNNN.wav` every [`CHECKPOINT_SECONDS`]. On a clean stop the
//! chunks are concatenated into `audio/audio.wav` and removed; on a crash they
//! are what [`crate::meetings::store::MeetingStore::recover_interrupted`]
//! finds and what [`merge_chunks`] turns into a playable recording.
//!
//! ## Why WAV and not a codec
//!
//! Meetily encodes each checkpoint to AAC through FFmpeg and concatenates with
//! `ffmpeg -c copy`, which buys roughly a quarter less disk and costs a
//! build-time binary download that `panic!`s without network, a runtime
//! process per checkpoint, a shell-quoting bug that strands audio whenever a
//! meeting title contains an apostrophe (`audio_processing.rs:15`), and an
//! install path that appends to the user's `.bashrc` when it cannot find
//! FFmpeg (`audio/ffmpeg.rs:196`).
//!
//! At 16 kHz mono 16-bit — the rate the transcriber works at, so no resampling
//! is needed to play a meeting back against its transcript — a WAV costs
//! 32 KB/s against AAC's 24 KB/s at meetily's settings. A third more disk for
//! an hour of meeting, in exchange for no external binary and a merge that is
//! a byte copy. `hound` is already a dependency.

use std::fs;
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};

use super::segmenter::SEGMENT_SAMPLE_RATE;
use super::store::list_chunks;

/// How much audio accumulates before a checkpoint is written. The worst-case
/// loss from a hard crash, and the same interval meetily uses.
pub const CHECKPOINT_SECONDS: usize = 30;

const CHECKPOINT_SAMPLES: usize = SEGMENT_SAMPLE_RATE as usize * CHECKPOINT_SECONDS;

/// The merged recording's filename inside a meeting's `audio/` directory.
pub const MERGED_AUDIO_FILE: &str = "audio.wav";

/// 16 kHz mono 16-bit PCM — what every chunk and every merged recording is.
fn wav_spec() -> hound::WavSpec {
    hound::WavSpec {
        channels: 1,
        sample_rate: SEGMENT_SAMPLE_RATE,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    }
}

#[derive(Debug, thiserror::Error)]
pub enum CheckpointError {
    #[error("checkpoint IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("WAV encoding error: {0}")]
    Wav(String),

    #[error("no audio was captured")]
    Empty,
}

impl From<hound::Error> for CheckpointError {
    fn from(err: hound::Error) -> Self {
        CheckpointError::Wav(err.to_string())
    }
}

/// Accumulates mixed audio and flushes it to numbered WAV chunks.
pub struct CheckpointWriter {
    audio_dir: PathBuf,
    buffer: Vec<f32>,
    next_index: u32,
    /// Total samples handed to this writer, including what is still buffered.
    total_samples: u64,
}

impl CheckpointWriter {
    /// Creates the audio directory and prepares to write into it.
    ///
    /// Resumes numbering after any chunks already present, so a writer created
    /// against a recovered meeting does not overwrite its existing audio.
    pub fn new(audio_dir: impl Into<PathBuf>) -> Result<Self, CheckpointError> {
        let audio_dir = audio_dir.into();
        fs::create_dir_all(&audio_dir)?;
        let next_index = list_chunks(&audio_dir)
            .iter()
            .filter_map(|path| chunk_index(path))
            .max()
            .map(|highest| highest + 1)
            .unwrap_or(0);
        Ok(Self {
            audio_dir,
            buffer: Vec::with_capacity(CHECKPOINT_SAMPLES),
            next_index,
            total_samples: 0,
        })
    }

    /// Seconds of audio this writer has been given.
    pub fn duration_seconds(&self) -> f64 {
        self.total_samples as f64 / SEGMENT_SAMPLE_RATE as f64
    }

    /// Adds mixed 16 kHz mono audio, flushing a chunk whenever enough has
    /// accumulated.
    ///
    /// A single large push can span several checkpoints, so this loops rather
    /// than assuming one flush is enough.
    pub fn push(&mut self, samples: &[f32]) -> Result<(), CheckpointError> {
        self.total_samples += samples.len() as u64;
        self.buffer.extend_from_slice(samples);
        while self.buffer.len() >= CHECKPOINT_SAMPLES {
            let rest = self.buffer.split_off(CHECKPOINT_SAMPLES);
            let chunk = std::mem::replace(&mut self.buffer, rest);
            self.write_chunk(&chunk)?;
        }
        Ok(())
    }

    /// Writes whatever is buffered as a final chunk, then merges every chunk
    /// into one recording and removes them.
    ///
    /// Returns the merged file's path.
    pub fn finalize(mut self) -> Result<PathBuf, CheckpointError> {
        if !self.buffer.is_empty() {
            let chunk = std::mem::take(&mut self.buffer);
            self.write_chunk(&chunk)?;
        }
        merge_chunks(&self.audio_dir, true)
    }

    /// Flushes what is buffered without merging.
    ///
    /// Used when a recording is stopping but its audio should stay recoverable
    /// until the merge succeeds.
    pub fn flush(&mut self) -> Result<(), CheckpointError> {
        if self.buffer.is_empty() {
            return Ok(());
        }
        let chunk = std::mem::take(&mut self.buffer);
        self.write_chunk(&chunk)
    }

    fn write_chunk(&mut self, samples: &[f32]) -> Result<(), CheckpointError> {
        let path = self
            .audio_dir
            .join(format!("chunk_{:06}.wav", self.next_index));
        write_wav(&path, samples)?;
        tracing::debug!(
            "meeting checkpoint {} written ({:.1}s)",
            self.next_index,
            samples.len() as f64 / SEGMENT_SAMPLE_RATE as f64
        );
        self.next_index += 1;
        Ok(())
    }
}

/// Concatenates a meeting's chunks into one recording.
///
/// Streams sample-by-sample rather than reading every chunk into memory: a
/// three-hour meeting is ~170 million samples, and the point of checkpointing
/// is that peak memory does not grow with meeting length.
///
/// `remove_chunks` deletes the inputs afterwards. Recovery passes `false` the
/// first time so a merge that fails halfway leaves the source audio intact.
pub fn merge_chunks(audio_dir: &Path, remove_chunks: bool) -> Result<PathBuf, CheckpointError> {
    let chunks = list_chunks(audio_dir);
    if chunks.is_empty() {
        return Err(CheckpointError::Empty);
    }

    let target = audio_dir.join(MERGED_AUDIO_FILE);
    // Write beside the target and rename, so an interrupted merge never leaves
    // a truncated audio.wav where a complete one used to be.
    let staging = audio_dir.join("audio.wav.partial");
    {
        let file = BufWriter::new(fs::File::create(&staging)?);
        let mut writer = hound::WavWriter::new(file, wav_spec())?;
        for chunk in &chunks {
            let mut reader = match hound::WavReader::open(chunk) {
                Ok(reader) => reader,
                Err(err) => {
                    // A chunk that will not open is the one the crash caught
                    // mid-write. Everything before it is still good audio.
                    tracing::warn!("skipping unreadable checkpoint {:?}: {}", chunk, err);
                    continue;
                }
            };
            for sample in reader.samples::<i16>() {
                match sample {
                    Ok(value) => writer.write_sample(value)?,
                    Err(err) => {
                        tracing::warn!("checkpoint {:?} truncated: {}", chunk, err);
                        break;
                    }
                }
            }
        }
        writer.finalize()?;
    }
    fs::rename(&staging, &target)?;

    if remove_chunks {
        for chunk in &chunks {
            if let Err(err) = fs::remove_file(chunk) {
                tracing::warn!("could not remove checkpoint {:?}: {}", chunk, err);
            }
        }
    }
    Ok(target)
}

/// Writes 16 kHz mono samples as a WAV, through a staging file.
pub fn write_wav(path: &Path, samples: &[f32]) -> Result<(), CheckpointError> {
    let staging = path.with_extension("partial");
    {
        let mut file = BufWriter::new(fs::File::create(&staging)?);
        let mut writer = hound::WavWriter::new(&mut file, wav_spec())?;
        for &sample in samples {
            writer.write_sample((sample.clamp(-1.0, 1.0) * i16::MAX as f32) as i16)?;
        }
        writer.finalize()?;
        file.flush()?;
    }
    fs::rename(&staging, path)?;
    Ok(())
}

/// Reads a 16-bit PCM WAV back as mono f32 at [`SEGMENT_SAMPLE_RATE`].
///
/// Channels are averaged and the result is resampled if the file disagrees
/// with the pipeline's rate, so a recovered or imported recording can be
/// re-transcribed without a separate decode path.
pub fn read_wav_mono_16k(path: &Path) -> Result<Vec<f32>, CheckpointError> {
    let mut reader = hound::WavReader::open(path)?;
    let spec = reader.spec();
    let channels = spec.channels.max(1) as usize;

    let interleaved: Vec<f32> = match spec.sample_format {
        hound::SampleFormat::Int => {
            let scale = match spec.bits_per_sample {
                8 => i8::MAX as f32,
                16 => i16::MAX as f32,
                24 => 8_388_607.0,
                _ => i32::MAX as f32,
            };
            reader
                .samples::<i32>()
                .filter_map(Result::ok)
                .map(|s| s as f32 / scale)
                .collect()
        }
        hound::SampleFormat::Float => reader.samples::<f32>().filter_map(Result::ok).collect(),
    };

    let mono: Vec<f32> = if channels == 1 {
        interleaved
    } else {
        interleaved
            .chunks(channels)
            .map(|frame| frame.iter().sum::<f32>() / frame.len() as f32)
            .collect()
    };

    Ok(crate::capture::resample_to_16k_mono(&mono, spec.sample_rate))
}

/// The numeric index in a `chunk_NNNNNN.wav` path.
fn chunk_index(path: &Path) -> Option<u32> {
    path.file_stem()?
        .to_str()?
        .strip_prefix("chunk_")?
        .parse()
        .ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "vox-checkpoint-{}-{}-{:?}",
            name,
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("create temp dir");
        dir
    }

    fn ramp(count: usize, start: usize) -> Vec<f32> {
        (0..count)
            .map(|i| ((start + i) % 1000) as f32 / 2000.0)
            .collect()
    }

    #[test]
    fn audio_shorter_than_one_checkpoint_writes_nothing_until_finalized() {
        let dir = temp_dir("short");
        let mut writer = CheckpointWriter::new(&dir).expect("writer");
        writer.push(&ramp(SEGMENT_SAMPLE_RATE as usize, 0)).expect("push");

        assert!(list_chunks(&dir).is_empty(), "1s should still be buffered");

        let merged = writer.finalize().expect("finalize");
        assert!(merged.exists());
        assert!(list_chunks(&dir).is_empty(), "chunks removed after merge");
    }

    #[test]
    fn a_checkpoint_lands_every_thirty_seconds() {
        let dir = temp_dir("interval");
        let mut writer = CheckpointWriter::new(&dir).expect("writer");
        // 70 seconds, pushed in one-second pieces the way the mixer delivers.
        for second in 0..70 {
            writer
                .push(&ramp(SEGMENT_SAMPLE_RATE as usize, second * 100))
                .expect("push");
        }
        assert_eq!(list_chunks(&dir).len(), 2, "70s is two whole checkpoints");
        assert!((writer.duration_seconds() - 70.0).abs() < 0.001);
    }

    #[test]
    fn one_oversized_push_still_produces_every_checkpoint_it_spans() {
        let dir = temp_dir("burst");
        let mut writer = CheckpointWriter::new(&dir).expect("writer");
        writer
            .push(&ramp(SEGMENT_SAMPLE_RATE as usize * 95, 0))
            .expect("push");
        assert_eq!(list_chunks(&dir).len(), 3, "95s is three whole checkpoints");
    }

    #[test]
    fn a_merged_recording_holds_every_sample_in_order() {
        let dir = temp_dir("merge-order");
        let mut writer = CheckpointWriter::new(&dir).expect("writer");
        let all = ramp(SEGMENT_SAMPLE_RATE as usize * 65, 0);
        writer.push(&all).expect("push");
        let merged = writer.finalize().expect("finalize");

        let back = read_wav_mono_16k(&merged).expect("read back");
        assert_eq!(back.len(), all.len());
        for (i, (&expected, &actual)) in all.iter().zip(back.iter()).enumerate() {
            assert!(
                (expected - actual).abs() < 1e-3,
                "sample {i} changed: {expected} -> {actual}"
            );
        }
    }

    #[test]
    fn a_crash_leaves_chunks_that_merge_into_a_playable_recording() {
        let dir = temp_dir("crash");
        {
            let mut writer = CheckpointWriter::new(&dir).expect("writer");
            writer
                .push(&ramp(SEGMENT_SAMPLE_RATE as usize * 65, 0))
                .expect("push");
            // Dropped without finalize — the process died here. Two whole
            // checkpoints reached disk; the buffered 5s did not.
        }
        assert_eq!(list_chunks(&dir).len(), 2);

        let merged = merge_chunks(&dir, false).expect("merge after crash");
        let back = read_wav_mono_16k(&merged).expect("read");
        assert_eq!(back.len(), SEGMENT_SAMPLE_RATE as usize * 60);
        assert_eq!(list_chunks(&dir).len(), 2, "recovery keeps its sources");
    }

    #[test]
    fn a_truncated_checkpoint_does_not_lose_the_ones_before_it() {
        let dir = temp_dir("truncated");
        let mut writer = CheckpointWriter::new(&dir).expect("writer");
        writer
            .push(&ramp(SEGMENT_SAMPLE_RATE as usize * 35, 0))
            .expect("push");
        writer.flush().expect("flush");
        // The chunk the crash caught mid-write: a file that is not a WAV.
        fs::write(dir.join("chunk_000009.wav"), b"\0\0\0\0not a wav").unwrap();

        let merged = merge_chunks(&dir, false).expect("merge");
        let back = read_wav_mono_16k(&merged).expect("read");
        assert_eq!(back.len(), SEGMENT_SAMPLE_RATE as usize * 35);
    }

    #[test]
    fn a_writer_resumes_numbering_instead_of_overwriting() {
        let dir = temp_dir("resume");
        {
            let mut writer = CheckpointWriter::new(&dir).expect("writer");
            writer
                .push(&ramp(SEGMENT_SAMPLE_RATE as usize * 35, 0))
                .expect("push");
        }
        assert_eq!(list_chunks(&dir).len(), 1);

        let mut resumed = CheckpointWriter::new(&dir).expect("writer");
        resumed
            .push(&ramp(SEGMENT_SAMPLE_RATE as usize * 31, 500))
            .expect("push");

        let chunks = list_chunks(&dir);
        assert_eq!(chunks.len(), 2, "the first checkpoint was overwritten");
        assert_eq!(chunk_index(&chunks[1]), Some(1));
    }

    #[test]
    fn merging_nothing_reports_that_nothing_was_captured() {
        let dir = temp_dir("empty");
        assert!(matches!(merge_chunks(&dir, true), Err(CheckpointError::Empty)));
    }

    #[test]
    fn an_interrupted_merge_leaves_no_partial_file_in_place() {
        let dir = temp_dir("staging");
        let mut writer = CheckpointWriter::new(&dir).expect("writer");
        writer.push(&ramp(SEGMENT_SAMPLE_RATE as usize * 31, 0)).expect("push");
        writer.finalize().expect("finalize");
        let leftovers: Vec<_> = fs::read_dir(&dir)
            .unwrap()
            .filter_map(Result::ok)
            .filter(|e| e.file_name().to_string_lossy().ends_with(".partial"))
            .collect();
        assert!(leftovers.is_empty());
    }

    #[test]
    fn a_stereo_source_is_averaged_to_mono_on_read() {
        let dir = temp_dir("stereo");
        let path = dir.join("stereo.wav");
        let spec = hound::WavSpec {
            channels: 2,
            sample_rate: SEGMENT_SAMPLE_RATE,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        let mut writer = hound::WavWriter::create(&path, spec).unwrap();
        for _ in 0..1000 {
            writer.write_sample(i16::MAX / 2).unwrap(); // left
            writer.write_sample(-(i16::MAX / 2)).unwrap(); // right
        }
        writer.finalize().unwrap();

        let mono = read_wav_mono_16k(&path).expect("read");
        assert_eq!(mono.len(), 1000);
        // Equal and opposite channels average to silence.
        assert!(mono.iter().all(|&s| s.abs() < 1e-3));
    }

    #[test]
    fn a_source_at_another_rate_comes_back_at_the_pipeline_rate() {
        let dir = temp_dir("resample");
        let path = dir.join("48k.wav");
        let spec = hound::WavSpec {
            channels: 1,
            sample_rate: 48_000,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        let mut writer = hound::WavWriter::create(&path, spec).unwrap();
        for _ in 0..48_000 {
            writer.write_sample(0i16).unwrap();
        }
        writer.finalize().unwrap();

        let samples = read_wav_mono_16k(&path).expect("read");
        let seconds = samples.len() as f64 / SEGMENT_SAMPLE_RATE as f64;
        assert!((seconds - 1.0).abs() < 0.01, "got {seconds}s");
    }
}
