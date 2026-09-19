//! Measuring what the meeting speech pipeline actually produces.
//!
//! ## Why this is not `capture::evaluation`
//!
//! [`crate::capture::evaluation`] measures the *dictation* path: one clip, the
//! whole-buffer [`crate::capture::VadConfig`], one decode. That is the right
//! shape for dictation and the wrong shape for a meeting, and the difference is
//! not a detail — it is every effect worth measuring. A meeting is cut into
//! spans by [`Segmenter`], those spans queue against a bounded channel, and a
//! single decoder works through them in order. Segmentation errors, queue
//! backlog, dropped speech, repetition across a forced split and drift over an
//! hour all live in that machinery, and a clip benchmark cannot see any of it.
//!
//! So this module drives the **production** pieces:
//!
//! | Piece | Shared with the live path |
//! |---|---|
//! | [`Segmenter`] | the same struct, the same constants |
//! | [`speech_health::profile_speech`] / [`speech_health::screen_decode`] | identical calls |
//! | [`text_normalize::normalize_segment_text`] | identical call |
//! | queue depth | [`transcription::MAX_QUEUED_SEGMENTS`], asserted equal in tests |
//! | queue discipline | bounded `sync_channel` + `try_send`, one serial consumer |
//!
//! What is *not* shared is the forty-line worker loop itself, because the real
//! one writes to a [`crate::meetings::store::MeetingStore`] and emits Tauri
//! events. The queue geometry is taken from the same constant so the two cannot
//! drift silently, and a test enforces it.
//!
//! ## Nothing here changes a recording
//!
//! No function in this module is reachable from [`crate::meetings::engine`].
//! It reads audio, runs the same analysis, and writes a report.
//!
//! ## Long form, not clips
//!
//! Every category carries a [`BenchmarkCategory::min_duration_seconds`] floor
//! and a case below it is reported as `UnderLength` rather than scored as if it
//! were representative. Ten-word clips produce unstable word error rates and
//! cannot express the failures this pipeline actually has — see
//! `docs/speech-decision-log.md` D-015.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::mpsc as std_mpsc;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use crate::capture::evaluation::{calculate_accuracy, normalize_for_eval};
use crate::capture::speech_health::{self, DecodeEvidence};
use crate::capture::text_normalize;

use super::segmenter::{Segmenter, SpeechSegment, SEGMENT_SAMPLE_RATE};
use super::transcription::MAX_QUEUED_SEGMENTS;

pub mod engines;

/// Audio handed to the segmenter per iteration.
///
/// The live mixer wakes every 20 ms, so a benchmark that fed the segmenter one
/// large slab would exercise a frame cadence no recording ever produces.
const BLOCK_MS: usize = 20;
const BLOCK_SAMPLES: usize = (SEGMENT_SAMPLE_RATE as usize * BLOCK_MS) / 1000;

/// Longest adjacent phrase the repetition measure looks for. Matches the
/// horizon [`speech_health`]'s loop detector uses, so "repeated" means the same
/// thing to the measurement and to the screen that rejects on it.
const MAX_REPEAT_PHRASE_WORDS: usize = 16;

/// Manifest schema version. Bumped when a field changes meaning, so an old file
/// fails loudly rather than being read under new rules.
pub const MANIFEST_VERSION: u32 = 1;

// --- corpus -------------------------------------------------------------

/// The speech conditions a meeting recorder has to survive.
///
/// Fixed rather than free text: a category whose name is typed per case cannot
/// be aggregated, and "which conditions is this model bad at" is the question
/// the whole harness exists to answer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BenchmarkCategory {
    CleanEnglish,
    IndianEnglish,
    /// English/Hindi code switching inside one utterance.
    Hinglish,
    TechnicalVocabulary,
    ProperNouns,
    NumbersAndDates,
    TwoSpeaker,
    OverlappingSpeech,
    Noisy,
    /// Recorded off a call, so the far end arrives through system audio.
    SystemAudioCall,
    /// A whole meeting, 30 minutes or more.
    LongMeeting,
    /// One speaker, 500-1000 words, no turn taking.
    LongFormSpeech,
}

impl BenchmarkCategory {
    pub const ALL: [BenchmarkCategory; 12] = [
        Self::CleanEnglish,
        Self::IndianEnglish,
        Self::Hinglish,
        Self::TechnicalVocabulary,
        Self::ProperNouns,
        Self::NumbersAndDates,
        Self::TwoSpeaker,
        Self::OverlappingSpeech,
        Self::Noisy,
        Self::SystemAudioCall,
        Self::LongMeeting,
        Self::LongFormSpeech,
    ];

    pub fn key(self) -> &'static str {
        match self {
            Self::CleanEnglish => "clean_english",
            Self::IndianEnglish => "indian_english",
            Self::Hinglish => "hinglish",
            Self::TechnicalVocabulary => "technical_vocabulary",
            Self::ProperNouns => "proper_nouns",
            Self::NumbersAndDates => "numbers_and_dates",
            Self::TwoSpeaker => "two_speaker",
            Self::OverlappingSpeech => "overlapping_speech",
            Self::Noisy => "noisy",
            Self::SystemAudioCall => "system_audio_call",
            Self::LongMeeting => "long_meeting",
            Self::LongFormSpeech => "long_form_speech",
        }
    }

    /// Shortest recording that can represent this category honestly.
    ///
    /// These are not arbitrary. Under about three minutes a single misheard
    /// clause moves word error rate by several points, so two models separated
    /// by noise look separated by quality. The two long categories are longer
    /// still because they exist to expose what only time exposes: a backlog
    /// that grows, a noise floor that drifts, a decoder that starts repeating.
    pub fn min_duration_seconds(self) -> f64 {
        match self {
            Self::LongMeeting => 30.0 * 60.0,
            Self::LongFormSpeech => 5.0 * 60.0,
            _ => 3.0 * 60.0,
        }
    }
}

/// Terms a case asserts should survive transcription.
///
/// Recall of a named list, not an accuracy score over all names — Vox has no
/// named-entity recogniser to find the ones nobody listed, and pretending
/// otherwise would produce a number that looks like coverage and is not.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TermExpectations {
    #[serde(default)]
    pub proper_nouns: Vec<String>,
    /// Written as they should appear — "fourteen" and "14" are different
    /// expectations and the benchmark does not guess which one was meant.
    #[serde(default)]
    pub numbers: Vec<String>,
    #[serde(default)]
    pub technical_terms: Vec<String>,
}

/// One recording in the corpus.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BenchmarkCase {
    pub id: String,
    /// Audio file, relative to the manifest's directory.
    pub audio: String,
    pub category: BenchmarkCategory,
    #[serde(default)]
    pub language: Option<String>,
    /// Ground truth inline, for a short case.
    #[serde(default)]
    pub reference: Option<String>,
    /// Ground truth in a file beside the manifest. Long-form references belong
    /// here — a thousand words inside a JSON string is unreadable and
    /// unreviewable.
    #[serde(default)]
    pub reference_file: Option<String>,
    #[serde(default)]
    pub expect: TermExpectations,
    #[serde(default)]
    pub notes: String,
}

/// A corpus, as it is written down.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BenchmarkManifest {
    pub version: u32,
    pub cases: Vec<BenchmarkCase>,
}

#[derive(Debug, thiserror::Error)]
pub enum BenchmarkError {
    #[error("the benchmark manifest could not be read: {0}")]
    Io(#[from] std::io::Error),

    #[error("the benchmark manifest is not valid JSON: {0}")]
    Json(#[from] serde_json::Error),

    #[error("manifest version {found} is not supported (this build reads version {expected})")]
    Version { found: u32, expected: u32 },

    #[error("{0}")]
    Invalid(String),

    #[error("no case in the manifest has id '{0}'")]
    UnknownCase(String),

    #[error("that audio could not be decoded: {0}")]
    Audio(String),
}

impl BenchmarkManifest {
    /// Reads and validates a manifest.
    pub fn load(path: &Path) -> Result<Self, BenchmarkError> {
        let manifest: Self = serde_json::from_slice(&std::fs::read(path)?)?;
        manifest.validate()?;
        Ok(manifest)
    }

    /// Rejects a manifest that cannot be run, rather than failing per case
    /// halfway through an hour of decoding.
    ///
    /// The path checks are the interesting ones. `audio` and `reference_file`
    /// are joined onto the manifest's own directory, and a manifest is a file
    /// the user edits — so the same traversal guard the meeting store applies
    /// to ids applies here. A corpus entry may not reach outside its corpus.
    pub fn validate(&self) -> Result<(), BenchmarkError> {
        if self.version != MANIFEST_VERSION {
            return Err(BenchmarkError::Version {
                found: self.version,
                expected: MANIFEST_VERSION,
            });
        }
        if self.cases.is_empty() {
            return Err(BenchmarkError::Invalid(
                "the manifest lists no cases".to_string(),
            ));
        }
        let mut seen = std::collections::HashSet::new();
        for case in &self.cases {
            if case.id.trim().is_empty() {
                return Err(BenchmarkError::Invalid("a case has an empty id".to_string()));
            }
            if !seen.insert(case.id.as_str()) {
                return Err(BenchmarkError::Invalid(format!(
                    "two cases share the id '{}'",
                    case.id
                )));
            }
            check_relative(&case.audio, &case.id, "audio")?;
            if let Some(reference_file) = &case.reference_file {
                check_relative(reference_file, &case.id, "reference_file")?;
            }
            if case.reference.is_some() && case.reference_file.is_some() {
                return Err(BenchmarkError::Invalid(format!(
                    "case '{}' gives both reference and reference_file; it must give at most one",
                    case.id
                )));
            }
        }
        Ok(())
    }

    /// The categories with no case at all — printed with a report so a
    /// flattering average is visibly incomplete.
    pub fn missing_categories(&self) -> Vec<BenchmarkCategory> {
        BenchmarkCategory::ALL
            .into_iter()
            .filter(|category| !self.cases.iter().any(|case| case.category == *category))
            .collect()
    }

    /// A starter manifest: one entry per category, with the audio the user has
    /// to supply named but not invented.
    pub fn template() -> Self {
        Self {
            version: MANIFEST_VERSION,
            cases: BenchmarkCategory::ALL
                .into_iter()
                .map(|category| BenchmarkCase {
                    id: category.key().to_string(),
                    audio: format!("audio/{}.wav", category.key()),
                    category,
                    language: None,
                    reference: None,
                    reference_file: Some(format!("reference/{}.txt", category.key())),
                    expect: TermExpectations::default(),
                    notes: format!(
                        "Supply a recording of at least {:.0} minutes.",
                        category.min_duration_seconds() / 60.0
                    ),
                })
                .collect(),
        }
    }
}

/// Rejects an absolute path, a parent escape, or a Windows drive prefix.
fn check_relative(value: &str, case_id: &str, field: &str) -> Result<(), BenchmarkError> {
    let path = Path::new(value);
    let escapes = path.is_absolute()
        || path.components().any(|component| {
            matches!(
                component,
                std::path::Component::ParentDir | std::path::Component::Prefix(_)
            )
        });
    if escapes {
        return Err(BenchmarkError::Invalid(format!(
            "case '{case_id}' has a {field} path that leaves the corpus directory: {value}"
        )));
    }
    Ok(())
}

// --- the engine seam ----------------------------------------------------

/// What produced a run's transcript.
///
/// Recorded on every result so two runs are comparable only when they say they
/// are. `model` is a filename or model id, never a path — a report is shared
/// and a path names a machine and often a person.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EngineDescriptor {
    pub engine: String,
    pub model: String,
    /// The language the engine was pinned to, or `None` for auto-detection.
    pub language: Option<String>,
    /// Decode profile, in whatever terms the engine uses.
    pub profile: String,
}

/// One decode's output.
///
/// `mean_no_speech_prob` is `None` for an engine that does not report one.
/// There is no default and no derived stand-in: a number invented here would
/// be indistinguishable from a measurement downstream, which is the mistake
/// `docs/speech-decision-log.md` D-006 exists to prevent.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct DecodeOutput {
    pub text: String,
    pub mean_no_speech_prob: Option<f32>,
}

/// The one thing the benchmark needs an engine to do.
///
/// Deliberately narrower than a transcription provider: the benchmark supplies
/// the audio the segmenter chose, and wants text back. Stage 5's
/// `SpeechRecognizer` is the general interface; this is the slice of it a
/// measurement needs, so a new engine can be benchmarked before it is
/// integrated.
pub trait BenchmarkDecoder: Send {
    fn descriptor(&self) -> EngineDescriptor;

    fn decode(&mut self, samples: &[f32]) -> Result<DecodeOutput, String>;
}

// --- results ------------------------------------------------------------

/// Why a case was not scored.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CaseStatus {
    Scored,
    /// Ran, but shorter than its category's floor, so its accuracy numbers are
    /// reported and must not be aggregated.
    UnderLength,
    /// Ran, but no ground truth was supplied, so only the latency and
    /// throughput half of the result is meaningful.
    NoReference,
}

/// Word-level accuracy, as rates rather than raw counts.
///
/// Counts are kept alongside because a rate over a short reference is
/// misleading on its own, and the denominator is the thing that says so.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AccuracyReport {
    pub reference_words: usize,
    pub hypothesis_words: usize,
    pub wer: f32,
    pub cer: f32,
    pub substitutions: usize,
    pub deletions: usize,
    pub insertions: usize,
    pub substitution_rate: f32,
    pub deletion_rate: f32,
    pub insertion_rate: f32,
}

/// Recall of the terms a case said should survive.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TermRecall {
    pub expected: usize,
    pub found: usize,
    pub missing: Vec<String>,
}

impl TermRecall {
    fn measure(expected: &[String], normalized_hypothesis: &str) -> Option<Self> {
        if expected.is_empty() {
            return None;
        }
        let haystack = normalized_hypothesis.to_lowercase();
        let mut missing = Vec::new();
        let mut found = 0usize;
        for term in expected {
            let needle = normalize_for_eval(term).to_lowercase();
            if !needle.is_empty() && haystack.contains(&needle) {
                found += 1;
            } else {
                missing.push(term.clone());
            }
        }
        Some(Self {
            expected: expected.len(),
            found,
            missing,
        })
    }

    pub fn rate(&self) -> f32 {
        if self.expected == 0 {
            return 0.0;
        }
        self.found as f32 / self.expected as f32
    }
}

/// What the pipeline did with the audio, independent of what it said.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct PipelineReport {
    pub audio_seconds: f64,
    /// Audio the segmenter judged to be speech and emitted as a span.
    pub segmented_seconds: f64,
    /// Audio behind transcript text that survived screening.
    pub transcribed_seconds: f64,
    pub segments_emitted: u64,
    pub segments_kept: u64,
    /// Decoded, then rejected by `speech_health` or empty.
    pub segments_discarded: u64,
    pub segments_failed: u64,
    /// Refused because the bounded queue was full.
    pub segments_dropped: u64,
    /// Cut at the segmenter's ceiling rather than at a silence, so the next
    /// segment continues the same sentence.
    pub segments_forced_split: u64,
    pub peak_queue_depth: u64,
}

impl PipelineReport {
    /// Audio time covered by transcript text, as a fraction of the recording.
    ///
    /// Well under 1.0 for any real recording — a meeting is mostly silence —
    /// so this is read against another run over the same audio, not against
    /// an absolute target.
    pub fn transcript_coverage(&self) -> f64 {
        ratio(self.transcribed_seconds, self.audio_seconds)
    }

    /// Of the audio the segmenter thought was speech, how much produced text.
    /// This one *should* approach 1.0, and the gap is speech that was lost.
    pub fn segmented_coverage(&self) -> f64 {
        ratio(self.transcribed_seconds, self.segmented_seconds)
    }
}

/// Where the time went.
///
/// Two real-time factors, because they answer different questions.
/// `decode_rtf` is the model's own speed against the speech it was given.
/// `pipeline_rtf` is decode time against the recording's wall-clock length,
/// and it is the one that decides whether a backlog grows: audio arrives at
/// wall-clock rate, not at speech rate.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct LatencyReport {
    pub decode_ms_total: u128,
    pub queue_wait_ms_total: u128,
    pub post_ms_total: u128,
    pub decode_rtf: f64,
    pub pipeline_rtf: f64,
    /// Wall clock from the first sample to the first transcript text.
    pub first_transcript_ms: Option<u128>,
    /// Per segment, from the moment the segmenter closed it to the moment its
    /// text existed: queue wait plus decode plus post-processing.
    pub finalization_p50_ms: u128,
    pub finalization_p95_ms: u128,
    pub finalization_max_ms: u128,
    /// Wall clock from the last sample to the last transcript text — what a
    /// user waits out after pressing stop.
    pub drain_ms: u128,
}

/// One engine's result on one case. This is `benchmark_run.json`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BenchmarkRun {
    pub case_id: String,
    pub category: BenchmarkCategory,
    pub status: CaseStatus,
    pub engine: EngineDescriptor,
    pub pacing: Pacing,

    pub pipeline: PipelineReport,
    pub latency: LatencyReport,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub accuracy: Option<AccuracyReport>,
    /// Share of hypothesis words inside an immediately repeated phrase.
    pub repetition_rate: f32,
    /// Share of decoded segments the hallucination screen rejected, by reason.
    pub hallucination_rate: f32,
    pub hallucination_reasons: BTreeMap<String, u64>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub proper_noun_recall: Option<TermRecall>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub number_recall: Option<TermRecall>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub technical_term_recall: Option<TermRecall>,

    pub transcript: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub errors: Vec<String>,
}

/// How audio is fed to the segmenter.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Pacing {
    /// As fast as the decoder manages. Accuracy and real-time factor are
    /// measured; queue depth and drops are not, because nothing is racing.
    Batch,
    /// Paced to wall clock, the way a recording arrives. Slower than the audio
    /// it measures by construction, and the only mode in which backlog, drops
    /// and time-to-first-transcript mean anything.
    Realtime,
}

/// Every run in one sitting, plus what the corpus did not cover.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BenchmarkReport {
    pub generated_at: String,
    pub runs: Vec<BenchmarkRun>,
    pub missing_categories: Vec<BenchmarkCategory>,
}

// --- running ------------------------------------------------------------

/// One segment's journey, recorded by the consumer.
#[derive(Debug, Clone)]
struct SegmentRecord {
    audio_seconds: f64,
    queue_wait_ms: u128,
    decode_ms: u128,
    post_ms: u128,
    /// Wall clock from the start of the run to the moment text existed.
    completed_at_ms: u128,
    outcome: SegmentOutcome,
    forced_split: bool,
    text: String,
}

#[derive(Debug, Clone, PartialEq)]
enum SegmentOutcome {
    Kept,
    Empty,
    Screened(&'static str),
    Failed(String),
}

struct QueuedSegment {
    segment: SpeechSegment,
    submitted_at: Instant,
}

/// One case in flight.
///
/// Audio is pushed in as it becomes available rather than handed over whole,
/// because the long categories are the point of the corpus and a two-hour
/// recording is roughly half a gigabyte of `f32`. The live pipeline never holds
/// a whole meeting in memory and neither does the thing measuring it.
pub struct CaseRunner {
    segmenter: Segmenter,
    tx: Option<std_mpsc::SyncSender<QueuedSegment>>,
    consumer: Option<std::thread::JoinHandle<Vec<SegmentRecord>>>,
    engine: EngineDescriptor,
    pacing: Pacing,
    started: Instant,
    /// Samples not yet a whole block, so a caller pushing awkward packet sizes
    /// still gets the 20 ms frame cadence a recording produces.
    pending: Vec<f32>,
    blocks_pushed: usize,
    counts: PipelineCounts,
    in_queue: i64,
}

impl CaseRunner {
    /// Starts the decode thread and the clock.
    pub fn start(decoder: Box<dyn BenchmarkDecoder>, pacing: Pacing, glossary: &[String]) -> Self {
        let engine = decoder.descriptor();
        let started = Instant::now();
        let (tx, rx) = std_mpsc::sync_channel::<QueuedSegment>(MAX_QUEUED_SEGMENTS);
        let language = engine.language.clone();
        let glossary = glossary.to_vec();
        let consumer = std::thread::Builder::new()
            .name("vox-benchmark-decode".into())
            .spawn(move || consume(rx, decoder, started, language, glossary))
            .expect("spawning the benchmark decode thread");

        Self {
            segmenter: Segmenter::new(),
            tx: Some(tx),
            consumer: Some(consumer),
            engine,
            pacing,
            started,
            pending: Vec::with_capacity(BLOCK_SAMPLES),
            blocks_pushed: 0,
            counts: PipelineCounts::default(),
            in_queue: 0,
        }
    }

    /// Adds 16 kHz mono audio. Any length; blocking is internal.
    pub fn push(&mut self, samples: &[f32]) {
        self.counts.audio_seconds += samples.len() as f64 / SEGMENT_SAMPLE_RATE as f64;
        let mut offset = 0usize;
        while offset < samples.len() {
            let want = BLOCK_SAMPLES - self.pending.len();
            let take = want.min(samples.len() - offset);
            self.pending.extend_from_slice(&samples[offset..offset + take]);
            offset += take;
            if self.pending.len() == BLOCK_SAMPLES {
                self.flush_block();
            }
        }
    }

    fn flush_block(&mut self) {
        if self.pacing == Pacing::Realtime {
            let due = Duration::from_millis(((self.blocks_pushed + 1) * BLOCK_MS) as u64);
            let elapsed = self.started.elapsed();
            if due > elapsed {
                std::thread::sleep(due - elapsed);
            }
        }
        self.blocks_pushed += 1;
        let block = std::mem::replace(&mut self.pending, Vec::with_capacity(BLOCK_SAMPLES));
        for segment in self.segmenter.push(&block, &[], &[]) {
            self.submit(segment);
        }
    }

    fn submit(&mut self, segment: SpeechSegment) {
        self.counts.emitted += 1;
        self.counts.segmented_seconds += segment.duration_seconds();
        if segment.forced_split {
            self.counts.forced += 1;
        }
        let Some(tx) = self.tx.as_ref() else {
            self.counts.dropped += 1;
            return;
        };
        match tx.try_send(QueuedSegment {
            segment,
            submitted_at: Instant::now(),
        }) {
            Ok(()) => {
                self.in_queue += 1;
                self.counts.peak_queue = self.counts.peak_queue.max(self.in_queue as u64);
            }
            // Both are lost speech and both are counted. Nothing here is
            // allowed to vanish quietly — that is the failure the whole
            // harness exists to make visible.
            Err(std_mpsc::TrySendError::Full(_)) => self.counts.dropped += 1,
            Err(std_mpsc::TrySendError::Disconnected(_)) => self.counts.dropped += 1,
        }
    }

    /// Closes the audio, drains the decoder, and scores what came back.
    pub fn finish(mut self, case: &BenchmarkCase, reference: Option<&str>) -> BenchmarkRun {
        if !self.pending.is_empty() {
            self.pending.resize(BLOCK_SAMPLES, 0.0);
            self.flush_block();
        }
        for segment in self.segmenter.flush() {
            self.submit(segment);
        }

        let last_sample_at = self.started.elapsed();
        drop(self.tx.take());
        let records = self
            .consumer
            .take()
            .map(|handle| handle.join().unwrap_or_default())
            .unwrap_or_default();
        let drain_ms = self
            .started
            .elapsed()
            .saturating_sub(last_sample_at)
            .as_millis();

        assemble(
            case,
            reference,
            self.engine.clone(),
            self.pacing,
            records,
            self.counts.clone(),
            drain_ms,
        )
    }
}

/// Runs one case whose audio is already in memory.
///
/// `samples` must be 16 kHz mono — the rate everything downstream of the mixer
/// works at. `reference` is ground truth, or `None` for a case that only
/// measures throughput.
pub fn run_case(
    case: &BenchmarkCase,
    samples: &[f32],
    reference: Option<&str>,
    decoder: Box<dyn BenchmarkDecoder>,
    pacing: Pacing,
    glossary: &[String],
) -> BenchmarkRun {
    let mut runner = CaseRunner::start(decoder, pacing, glossary);
    runner.push(samples);
    runner.finish(case, reference)
}

/// The serial decoder, in the shape the live worker uses.
fn consume(
    rx: std_mpsc::Receiver<QueuedSegment>,
    mut decoder: Box<dyn BenchmarkDecoder>,
    started: Instant,
    language: Option<String>,
    glossary: Vec<String>,
) -> Vec<SegmentRecord> {
    let mut records = Vec::new();
    for job in rx {
        let queue_wait_ms = job.submitted_at.elapsed().as_millis();
        let audio_seconds = job.segment.duration_seconds();
        let forced_split = job.segment.forced_split;

        // The same pre-decode voiced-time measurement the live worker takes.
        let profile = speech_health::profile_speech(&job.segment.samples, SEGMENT_SAMPLE_RATE);

        let decode_start = Instant::now();
        let decoded = decoder.decode(&job.segment.samples);
        let decode_ms = decode_start.elapsed().as_millis();

        let post_start = Instant::now();
        let (outcome, text) = match decoded {
            Err(message) => (SegmentOutcome::Failed(message), String::new()),
            Ok(output) if output.text.trim().is_empty() => (SegmentOutcome::Empty, String::new()),
            Ok(output) => {
                let evidence = DecodeEvidence {
                    voiced_seconds: profile.voiced_seconds,
                    total_seconds: profile.total_seconds,
                    // An engine that reports nothing must not be punished by a
                    // screen tuned for one that does, so the neutral value is
                    // "definitely speech" rather than an invented estimate.
                    mean_no_speech_prob: output.mean_no_speech_prob.unwrap_or(0.0),
                };
                match speech_health::screen_decode(
                    "meeting-benchmark",
                    language.as_deref(),
                    &output.text,
                    evidence,
                ) {
                    Some(reason) => (SegmentOutcome::Screened(reason.key()), String::new()),
                    None => {
                        let normalized =
                            text_normalize::normalize_segment_text(&output.text, &glossary);
                        (SegmentOutcome::Kept, normalized.text)
                    }
                }
            }
        };
        let post_ms = post_start.elapsed().as_millis();

        records.push(SegmentRecord {
            audio_seconds,
            queue_wait_ms,
            decode_ms,
            post_ms,
            completed_at_ms: started.elapsed().as_millis(),
            outcome,
            forced_split,
            text,
        });
    }
    records
}

#[derive(Debug, Clone, Default)]
struct PipelineCounts {
    audio_seconds: f64,
    segmented_seconds: f64,
    emitted: u64,
    dropped: u64,
    forced: u64,
    peak_queue: u64,
}

fn assemble(
    case: &BenchmarkCase,
    reference: Option<&str>,
    engine: EngineDescriptor,
    pacing: Pacing,
    records: Vec<SegmentRecord>,
    counts: PipelineCounts,
    drain_ms: u128,
) -> BenchmarkRun {
    let mut pipeline = PipelineReport {
        audio_seconds: counts.audio_seconds,
        segmented_seconds: counts.segmented_seconds,
        segments_emitted: counts.emitted,
        segments_dropped: counts.dropped,
        segments_forced_split: counts.forced,
        peak_queue_depth: counts.peak_queue,
        ..PipelineReport::default()
    };
    let mut latency = LatencyReport {
        drain_ms,
        ..LatencyReport::default()
    };
    let mut hallucination_reasons: BTreeMap<String, u64> = BTreeMap::new();
    let mut finalizations: Vec<u128> = Vec::with_capacity(records.len());
    let mut errors = Vec::new();
    let mut parts: Vec<String> = Vec::new();
    let mut decoded_seconds = 0.0f64;

    for record in &records {
        latency.decode_ms_total += record.decode_ms;
        latency.queue_wait_ms_total += record.queue_wait_ms;
        latency.post_ms_total += record.post_ms;
        finalizations.push(record.queue_wait_ms + record.decode_ms + record.post_ms);
        decoded_seconds += record.audio_seconds;

        match &record.outcome {
            SegmentOutcome::Kept => {
                pipeline.segments_kept += 1;
                pipeline.transcribed_seconds += record.audio_seconds;
                if latency.first_transcript_ms.is_none() {
                    latency.first_transcript_ms = Some(record.completed_at_ms);
                }
                parts.push(record.text.clone());
            }
            SegmentOutcome::Empty => pipeline.segments_discarded += 1,
            SegmentOutcome::Screened(reason) => {
                pipeline.segments_discarded += 1;
                *hallucination_reasons.entry((*reason).to_string()).or_insert(0) += 1;
            }
            SegmentOutcome::Failed(message) => {
                pipeline.segments_failed += 1;
                if errors.len() < 10 {
                    errors.push(message.clone());
                }
            }
        }
        let _ = record.forced_split;
    }

    latency.decode_rtf = ratio(latency.decode_ms_total as f64 / 1000.0, decoded_seconds);
    latency.pipeline_rtf = ratio(
        latency.decode_ms_total as f64 / 1000.0,
        counts.audio_seconds,
    );
    finalizations.sort_unstable();
    latency.finalization_p50_ms = percentile(&finalizations, 0.50);
    latency.finalization_p95_ms = percentile(&finalizations, 0.95);
    latency.finalization_max_ms = finalizations.last().copied().unwrap_or(0);

    let transcript = parts.join(" ");
    let normalized_hypothesis = normalize_for_eval(&transcript);

    let screened: u64 = hallucination_reasons.values().sum();
    let decoded_count = records.len() as u64;
    let hallucination_rate = if decoded_count == 0 {
        0.0
    } else {
        screened as f32 / decoded_count as f32
    };

    let accuracy = reference.map(|reference| accuracy_report(reference, &transcript));
    let status = if reference.is_none() {
        CaseStatus::NoReference
    } else if counts.audio_seconds < case.category.min_duration_seconds() {
        CaseStatus::UnderLength
    } else {
        CaseStatus::Scored
    };

    BenchmarkRun {
        case_id: case.id.clone(),
        category: case.category,
        status,
        engine,
        pacing,
        pipeline,
        latency,
        accuracy,
        repetition_rate: repetition_rate(&normalized_hypothesis),
        hallucination_rate,
        hallucination_reasons,
        proper_noun_recall: TermRecall::measure(&case.expect.proper_nouns, &normalized_hypothesis),
        number_recall: TermRecall::measure(&case.expect.numbers, &normalized_hypothesis),
        technical_term_recall: TermRecall::measure(
            &case.expect.technical_terms,
            &normalized_hypothesis,
        ),
        transcript,
        errors,
    }
}

fn accuracy_report(reference: &str, hypothesis: &str) -> AccuracyReport {
    let metrics = calculate_accuracy(reference, hypothesis);
    let words = metrics.word_count.max(1) as f32;
    AccuracyReport {
        reference_words: metrics.word_count,
        hypothesis_words: normalize_for_eval(hypothesis).split_whitespace().count(),
        wer: metrics.wer,
        cer: metrics.cer,
        substitutions: metrics.substitutions,
        deletions: metrics.deletions,
        insertions: metrics.insertions,
        substitution_rate: metrics.substitutions as f32 / words,
        deletion_rate: metrics.deletions as f32 / words,
        insertion_rate: metrics.insertions as f32 / words,
    }
}

/// Share of words inside an immediately repeated phrase.
///
/// Whisper's failure over silence is a loop, and a loop long enough to be
/// obvious is rejected by [`speech_health`] before it reaches a transcript.
/// What this measures is the rest: the doubled clause at a segment boundary,
/// the stutter a forced split produces. Those survive screening and are
/// exactly what gets worse as a meeting gets longer.
///
/// Longest phrase wins, so "we should ship it we should ship it" counts eight
/// repeated words rather than being read as one repeated "we".
///
/// **This counts repetition, not error.** A speaker who says "no, no, no"
/// raises it, and so does a standup where four people each say "yesterday I".
/// So the number is only meaningful compared against another run over the
/// *same* audio: between two engines, or between two profiles, the difference
/// is decoder repetition. The absolute value is not a defect count and must
/// not be reported as one.
pub fn repetition_rate(normalized: &str) -> f32 {
    let words: Vec<&str> = normalized.split_whitespace().collect();
    if words.len() < 2 {
        return 0.0;
    }
    let mut repeated = vec![false; words.len()];
    for length in (1..=MAX_REPEAT_PHRASE_WORDS.min(words.len() / 2)).rev() {
        let mut start = 0usize;
        while start + 2 * length <= words.len() {
            if words[start..start + length] == words[start + length..start + 2 * length] {
                for flag in repeated
                    .iter_mut()
                    .take((start + 2 * length).min(words.len()))
                    .skip(start)
                {
                    *flag = true;
                }
                start += length;
            } else {
                start += 1;
            }
        }
    }
    repeated.iter().filter(|flag| **flag).count() as f32 / words.len() as f32
}

fn percentile(sorted: &[u128], fraction: f64) -> u128 {
    if sorted.is_empty() {
        return 0;
    }
    let index = ((sorted.len() as f64 - 1.0) * fraction).round() as usize;
    sorted[index.min(sorted.len() - 1)]
}

fn ratio(numerator: f64, denominator: f64) -> f64 {
    if denominator > 0.0 {
        numerator / denominator
    } else {
        0.0
    }
}

// --- reporting ----------------------------------------------------------

impl BenchmarkReport {
    pub fn new(runs: Vec<BenchmarkRun>, missing_categories: Vec<BenchmarkCategory>) -> Self {
        Self {
            generated_at: chrono::Utc::now().to_rfc3339(),
            runs,
            missing_categories,
        }
    }

    /// Writes `benchmark_run.json` per run plus `report.json` and `report.md`.
    ///
    /// One file per run because a run is the unit that gets compared: diffing
    /// two engines is diffing two directories, and a single combined file makes
    /// that a text-processing exercise.
    pub fn write_to(&self, dir: &Path) -> Result<(), BenchmarkError> {
        std::fs::create_dir_all(dir)?;
        for run in &self.runs {
            let name = format!(
                "{}__{}__{}.json",
                sanitize(&run.case_id),
                sanitize(&run.engine.engine),
                sanitize(&run.engine.model)
            );
            std::fs::write(dir.join(name), serde_json::to_vec_pretty(run)?)?;
        }
        std::fs::write(dir.join("report.json"), serde_json::to_vec_pretty(self)?)?;
        std::fs::write(dir.join("report.md"), self.render_markdown())?;
        Ok(())
    }

    /// A short human-readable summary.
    ///
    /// Deliberately not an average across categories: a mean word error rate
    /// over clean English and overlapping speech describes no recording that
    /// exists, and the per-category row is the one that changes a decision.
    pub fn render_markdown(&self) -> String {
        let mut out = String::new();
        out.push_str("# Speech benchmark\n\n");
        out.push_str(&format!("Generated {}\n\n", self.generated_at));

        if !self.missing_categories.is_empty() {
            out.push_str("> **Incomplete corpus.** No case for: ");
            out.push_str(
                &self
                    .missing_categories
                    .iter()
                    .map(|category| category.key())
                    .collect::<Vec<_>>()
                    .join(", "),
            );
            out.push_str(".\n\n");
        }

        out.push_str("| case | category | engine | status | WER | CER | rep | halluc | coverage | decode RTF | pipeline RTF | p95 final | dropped |\n");
        out.push_str("|---|---|---|---|---|---|---|---|---|---|---|---|---|\n");
        for run in &self.runs {
            let (wer, cer) = run
                .accuracy
                .as_ref()
                .map(|a| (format!("{:.3}", a.wer), format!("{:.3}", a.cer)))
                .unwrap_or_else(|| ("—".to_string(), "—".to_string()));
            out.push_str(&format!(
                "| {} | {} | {} {} | {:?} | {} | {} | {:.3} | {:.3} | {:.2} | {:.2} | {:.2} | {} ms | {} |\n",
                run.case_id,
                run.category.key(),
                run.engine.engine,
                run.engine.model,
                run.status,
                wer,
                cer,
                run.repetition_rate,
                run.hallucination_rate,
                run.pipeline.transcript_coverage(),
                run.latency.decode_rtf,
                run.latency.pipeline_rtf,
                run.latency.finalization_p95_ms,
                run.pipeline.segments_dropped,
            ));
        }

        out.push_str(
            "\nA `pipeline RTF` above 1.0 means the decoder cannot keep up with a live \
             recording: over a meeting of length L the backlog reaches `L × (rtf − 1)` and \
             takes that long to clear after the user presses stop.\n",
        );
        out.push_str(
            "\n`UnderLength` rows ran but are shorter than their category's floor. Their \
             accuracy is reported and must not be aggregated — see D-015.\n",
        );
        out
    }
}

fn sanitize(value: &str) -> String {
    value
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { '-' })
        .collect()
}

/// Resolves a case's audio and ground truth relative to the manifest directory.
pub fn resolve_case(
    corpus_root: &Path,
    case: &BenchmarkCase,
) -> Result<(PathBuf, Option<String>), BenchmarkError> {
    check_relative(&case.audio, &case.id, "audio")?;
    let audio = corpus_root.join(&case.audio);
    let reference = match (&case.reference, &case.reference_file) {
        (Some(inline), _) => Some(inline.clone()),
        (None, Some(file)) => {
            check_relative(file, &case.id, "reference_file")?;
            Some(std::fs::read_to_string(corpus_root.join(file))?)
        }
        (None, None) => None,
    };
    Ok((audio, reference))
}

/// Runs one case from disk, streaming the recording rather than loading it.
///
/// Decoding goes through [`crate::meetings::import::decode_streaming`] — the
/// same path an imported meeting takes — so the corpus accepts every container
/// Vox accepts, and a benchmark cannot be reading the file differently from
/// the product.
pub fn run_case_from_disk(
    corpus_root: &Path,
    case: &BenchmarkCase,
    decoder: Box<dyn BenchmarkDecoder>,
    pacing: Pacing,
    glossary: &[String],
    cancel: &std::sync::atomic::AtomicBool,
) -> Result<BenchmarkRun, BenchmarkError> {
    use crate::meetings::import::{self, ImportError};

    let (audio_path, reference) = resolve_case(corpus_root, case)?;
    let mut runner = CaseRunner::start(decoder, pacing, glossary);
    {
        let mut sink = |chunk: &[f32], _total: Option<f64>| -> Result<(), ImportError> {
            if cancel.load(std::sync::atomic::Ordering::SeqCst) {
                return Err(ImportError::Cancelled);
            }
            runner.push(chunk);
            Ok(())
        };
        import::decode_streaming(&audio_path, &mut sink)
            .map_err(|err| BenchmarkError::Audio(err.to_string()))?;
    }
    Ok(runner.finish(case, reference.as_deref()))
}

/// Runs every case in a manifest against one engine.
///
/// `build_decoder` is called per case rather than once, so an engine that
/// holds a model can be rebuilt between cases and a failure on one case does
/// not poison the rest. A case that cannot run becomes an error row instead of
/// aborting the corpus — an hour into a long run, losing the finished cases to
/// a missing file is its own kind of defect.
pub fn run_manifest(
    corpus_root: &Path,
    manifest: &BenchmarkManifest,
    mut build_decoder: impl FnMut() -> Result<Box<dyn BenchmarkDecoder>, BenchmarkError>,
    pacing: Pacing,
    glossary: &[String],
    cancel: &std::sync::atomic::AtomicBool,
    mut progress: impl FnMut(&str, usize, usize),
) -> Result<BenchmarkReport, BenchmarkError> {
    manifest.validate()?;
    let total = manifest.cases.len();
    let mut runs = Vec::with_capacity(total);

    for (index, case) in manifest.cases.iter().enumerate() {
        if cancel.load(std::sync::atomic::Ordering::SeqCst) {
            break;
        }
        progress(&case.id, index, total);
        let decoder = build_decoder()?;
        match run_case_from_disk(corpus_root, case, decoder, pacing, glossary, cancel) {
            Ok(run) => runs.push(run),
            Err(err) => runs.push(failed_run(case, pacing, err)),
        }
    }

    Ok(BenchmarkReport::new(runs, manifest.missing_categories()))
}

/// A row for a case that never produced audio, so the report says what was
/// attempted rather than silently listing fewer cases than the corpus has.
fn failed_run(case: &BenchmarkCase, pacing: Pacing, err: BenchmarkError) -> BenchmarkRun {
    BenchmarkRun {
        case_id: case.id.clone(),
        category: case.category,
        status: CaseStatus::NoReference,
        engine: EngineDescriptor {
            engine: "none".into(),
            model: "none".into(),
            language: None,
            profile: "none".into(),
        },
        pacing,
        pipeline: PipelineReport::default(),
        latency: LatencyReport::default(),
        accuracy: None,
        repetition_rate: 0.0,
        hallucination_rate: 0.0,
        hallucination_reasons: BTreeMap::new(),
        proper_noun_recall: None,
        number_recall: None,
        technical_term_recall: None,
        transcript: String::new(),
        errors: vec![err.to_string()],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A decoder that returns scripted text, so every metric below is checked
    /// against a known answer rather than against whatever a model happened to
    /// say on the day.
    struct ScriptedDecoder {
        lines: Vec<Result<String, String>>,
        next: usize,
        no_speech_prob: Option<f32>,
    }

    impl ScriptedDecoder {
        fn new(lines: Vec<Result<String, String>>) -> Box<Self> {
            Box::new(Self {
                lines,
                next: 0,
                no_speech_prob: Some(0.0),
            })
        }
    }

    impl BenchmarkDecoder for ScriptedDecoder {
        fn descriptor(&self) -> EngineDescriptor {
            EngineDescriptor {
                engine: "scripted".into(),
                model: "test".into(),
                language: Some("en".into()),
                profile: "none".into(),
            }
        }

        fn decode(&mut self, _samples: &[f32]) -> Result<DecodeOutput, String> {
            let line = self.lines.get(self.next).cloned();
            self.next += 1;
            match line {
                Some(Ok(text)) => Ok(DecodeOutput {
                    text,
                    mean_no_speech_prob: self.no_speech_prob,
                }),
                Some(Err(message)) => Err(message),
                None => Ok(DecodeOutput::default()),
            }
        }
    }

    /// Speech-shaped audio: loud enough to clear the segmenter's absolute floor
    /// and modulated so it does not read as a constant tone.
    fn speech(seconds: f64) -> Vec<f32> {
        let count = (SEGMENT_SAMPLE_RATE as f64 * seconds) as usize;
        (0..count)
            .map(|i| {
                let t = i as f32 / SEGMENT_SAMPLE_RATE as f32;
                0.25 * (t * 220.0 * std::f32::consts::TAU).sin()
                    * (1.0 + 0.5 * (t * 3.0 * std::f32::consts::TAU).sin())
            })
            .collect()
    }

    fn silence(seconds: f64) -> Vec<f32> {
        vec![0.0; (SEGMENT_SAMPLE_RATE as f64 * seconds) as usize]
    }

    fn case(id: &str, category: BenchmarkCategory) -> BenchmarkCase {
        BenchmarkCase {
            id: id.into(),
            audio: "audio/a.wav".into(),
            category,
            language: Some("en".into()),
            reference: None,
            reference_file: None,
            expect: TermExpectations::default(),
            notes: String::new(),
        }
    }

    #[test]
    fn the_benchmark_queue_is_exactly_the_production_queue() {
        // The whole claim of this module is that it measures the real pipeline.
        // A different queue depth would make backlog and drop numbers describe
        // a pipeline that does not ship.
        assert_eq!(MAX_QUEUED_SEGMENTS, 64);
    }

    #[test]
    fn a_run_segments_decodes_and_scores_the_same_audio() {
        let mut samples = speech(1.5);
        samples.extend(silence(1.0));
        samples.extend(speech(1.5));

        let run = run_case(
            &case("c", BenchmarkCategory::CleanEnglish),
            &samples,
            Some("hello world hello world"),
            ScriptedDecoder::new(vec![Ok("hello world".into()), Ok("hello world".into())]),
            Pacing::Batch,
            &[],
        );

        assert_eq!(run.pipeline.segments_emitted, 2, "one span either side of the gap");
        assert_eq!(run.pipeline.segments_kept, 2);
        assert_eq!(run.pipeline.segments_dropped, 0);
        // The transcript is what the production normalizer produced — sentence
        // casing and terminal punctuation — because the benchmark runs the same
        // call the live worker runs. Scoring folds both away, so the casing Vox
        // added and the punctuation the reference lacks cost nothing.
        assert_eq!(run.transcript, "Hello world. Hello world.");
        assert_eq!(normalize_for_eval(&run.transcript), "hello world hello world");
        let accuracy = run.accuracy.expect("a reference was supplied");
        assert_eq!(accuracy.wer, 0.0);
        assert_eq!(accuracy.reference_words, 4);
    }

    #[test]
    fn a_case_under_its_categorys_floor_is_reported_rather_than_scored() {
        // Four seconds of audio cannot represent clean English, and a word
        // error rate over it is noise with three decimal places.
        let run = run_case(
            &case("short", BenchmarkCategory::CleanEnglish),
            &speech(2.0),
            Some("hello"),
            ScriptedDecoder::new(vec![Ok("hello".into())]),
            Pacing::Batch,
            &[],
        );
        assert_eq!(run.status, CaseStatus::UnderLength);
        assert!(run.accuracy.is_some(), "the numbers are still reported");
    }

    #[test]
    fn a_case_with_no_ground_truth_still_measures_throughput() {
        let run = run_case(
            &case("c", BenchmarkCategory::CleanEnglish),
            &speech(2.0),
            None,
            ScriptedDecoder::new(vec![Ok("anything".into())]),
            Pacing::Batch,
            &[],
        );
        assert_eq!(run.status, CaseStatus::NoReference);
        assert!(run.accuracy.is_none());
        assert!(run.pipeline.segments_kept > 0);
        assert!(run.latency.first_transcript_ms.is_some());
    }

    #[test]
    fn a_failed_decode_is_counted_and_its_message_kept() {
        let run = run_case(
            &case("c", BenchmarkCategory::CleanEnglish),
            &speech(2.0),
            None,
            ScriptedDecoder::new(vec![Err("model exploded".into())]),
            Pacing::Batch,
            &[],
        );
        assert_eq!(run.pipeline.segments_failed, 1);
        assert_eq!(run.pipeline.segments_kept, 0);
        assert_eq!(run.errors, vec!["model exploded".to_string()]);
    }

    #[test]
    fn a_looping_decode_is_screened_and_counted_as_a_hallucination() {
        // The exact failure `speech_health` exists for: the same clause over
        // and over. The benchmark must count it, not transcribe it.
        let looped = "thank you for watching ".repeat(12);
        let run = run_case(
            &case("c", BenchmarkCategory::CleanEnglish),
            &speech(2.0),
            None,
            ScriptedDecoder::new(vec![Ok(looped)]),
            Pacing::Batch,
            &[],
        );
        assert_eq!(run.pipeline.segments_kept, 0);
        assert_eq!(run.pipeline.segments_discarded, 1);
        assert!(run.hallucination_rate > 0.0);
        assert!(!run.hallucination_reasons.is_empty());
    }

    #[test]
    fn coverage_separates_the_audio_from_the_speech_in_it() {
        let mut samples = speech(1.5);
        samples.extend(silence(6.0));

        let run = run_case(
            &case("c", BenchmarkCategory::CleanEnglish),
            &samples,
            None,
            ScriptedDecoder::new(vec![Ok("hello".into())]),
            Pacing::Batch,
            &[],
        );
        // Most of the recording is silence, so transcript coverage is low and
        // that is correct rather than a defect.
        assert!(run.pipeline.transcript_coverage() < 0.5);
        // Almost all of what the segmenter *called* speech produced text.
        assert!(run.pipeline.segmented_coverage() > 0.9);
    }

    #[test]
    fn repetition_is_measured_by_the_longest_repeated_phrase() {
        assert_eq!(repetition_rate(""), 0.0);
        assert_eq!(repetition_rate("one two three four"), 0.0);
        // Four of four words repeat.
        assert_eq!(repetition_rate("ship it ship it"), 1.0);
        // "we should ship it" twice — eight repeated words out of eleven. The
        // four-word phrase has to win over the single repeated "we", or a
        // doubled clause would score the same as an ordinary reused word.
        let rate = repetition_rate("we should ship it we should ship it and then stop");
        assert!((rate - 8.0 / 11.0).abs() < 1e-6, "got {rate}");
    }

    #[test]
    fn repetition_counts_what_was_said_twice_even_when_nothing_went_wrong() {
        // Guarding the claim in the doc comment rather than leaving it prose:
        // a speaker who repeats themselves raises this, so the number is only
        // meaningful against another run over the same audio.
        assert_eq!(repetition_rate("no no"), 1.0);
    }

    #[test]
    fn term_recall_reports_what_was_missed_not_just_a_score() {
        let recall = TermRecall::measure(
            &["Payal".into(), "Bengaluru".into()],
            &normalize_for_eval("payal said the bangalore office"),
        )
        .expect("terms were expected");
        assert_eq!(recall.found, 1);
        assert_eq!(recall.missing, vec!["Bengaluru".to_string()]);
        assert!((recall.rate() - 0.5).abs() < 1e-6);
        assert!(TermRecall::measure(&[], "anything").is_none());
    }

    #[test]
    fn error_rates_are_reported_against_the_reference_length() {
        let report = accuracy_report("one two three four", "one two four");
        assert_eq!(report.reference_words, 4);
        assert_eq!(report.deletions, 1);
        assert!((report.deletion_rate - 0.25).abs() < 1e-6);
    }

    #[test]
    fn a_manifest_that_reaches_outside_the_corpus_is_refused() {
        // The manifest is a file the user edits and its paths are joined onto a
        // root, which is the same shape as the template-id traversal the
        // meeting store guards against.
        let mut manifest = BenchmarkManifest::template();
        manifest.cases[0].audio = "../../../etc/passwd".into();
        let err = manifest.validate().expect_err("traversal must be refused");
        assert!(matches!(err, BenchmarkError::Invalid(_)), "{err}");

        let mut absolute = BenchmarkManifest::template();
        absolute.cases[0].audio = if cfg!(windows) {
            "C:\\windows\\system32\\config".into()
        } else {
            "/etc/passwd".into()
        };
        assert!(absolute.validate().is_err(), "an absolute path must be refused");
    }

    #[test]
    fn a_manifest_from_another_schema_version_fails_loudly() {
        let manifest = BenchmarkManifest {
            version: MANIFEST_VERSION + 1,
            cases: BenchmarkManifest::template().cases,
        };
        assert!(matches!(
            manifest.validate(),
            Err(BenchmarkError::Version { .. })
        ));
    }

    #[test]
    fn duplicate_case_ids_are_refused_before_an_hour_of_decoding() {
        let mut manifest = BenchmarkManifest::template();
        let first = manifest.cases[0].clone();
        manifest.cases.push(first);
        assert!(manifest.validate().is_err());
    }

    #[test]
    fn a_case_may_not_give_both_a_reference_and_a_reference_file() {
        let mut manifest = BenchmarkManifest::template();
        manifest.cases[0].reference = Some("inline".into());
        assert!(manifest.validate().is_err());
    }

    #[test]
    fn the_template_covers_every_category_and_validates() {
        let manifest = BenchmarkManifest::template();
        manifest.validate().expect("the shipped template must be runnable");
        assert_eq!(manifest.cases.len(), BenchmarkCategory::ALL.len());
        assert!(manifest.missing_categories().is_empty());
    }

    #[test]
    fn a_partial_corpus_says_which_categories_it_is_missing() {
        let manifest = BenchmarkManifest {
            version: MANIFEST_VERSION,
            cases: vec![case("c", BenchmarkCategory::CleanEnglish)],
        };
        let missing = manifest.missing_categories();
        assert_eq!(missing.len(), BenchmarkCategory::ALL.len() - 1);
        assert!(!missing.contains(&BenchmarkCategory::CleanEnglish));
        assert!(missing.contains(&BenchmarkCategory::OverlappingSpeech));
    }

    #[test]
    fn a_report_names_an_incomplete_corpus_in_its_summary() {
        let run = run_case(
            &case("c", BenchmarkCategory::CleanEnglish),
            &speech(2.0),
            None,
            ScriptedDecoder::new(vec![Ok("hello".into())]),
            Pacing::Batch,
            &[],
        );
        let report = BenchmarkReport::new(vec![run], vec![BenchmarkCategory::Hinglish]);
        let markdown = report.render_markdown();
        assert!(markdown.contains("Incomplete corpus"));
        assert!(markdown.contains("hinglish"));
        assert!(markdown.contains("pipeline RTF"));
    }

    #[test]
    fn a_report_round_trips_through_its_own_json() {
        let run = run_case(
            &case("c", BenchmarkCategory::CleanEnglish),
            &speech(2.0),
            Some("hello"),
            ScriptedDecoder::new(vec![Ok("hello".into())]),
            Pacing::Batch,
            &[],
        );
        let report = BenchmarkReport::new(vec![run], Vec::new());
        let json = serde_json::to_string(&report).expect("serialize");
        let back: BenchmarkReport = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(report, back);
    }

    #[test]
    fn a_decoder_reporting_no_confidence_is_not_screened_for_the_lack_of_it() {
        // An engine with no `no_speech_prob` must not be systematically
        // rejected by a screen tuned for one that has it. D-006: absent is
        // absent, not zero-confidence.
        let mut decoder = ScriptedDecoder::new(vec![Ok("a real sentence of speech".into())]);
        decoder.no_speech_prob = None;
        let run = run_case(
            &case("c", BenchmarkCategory::CleanEnglish),
            &speech(2.0),
            None,
            decoder,
            Pacing::Batch,
            &[],
        );
        assert_eq!(run.pipeline.segments_kept, 1);
    }

    #[test]
    fn writing_a_report_produces_one_file_per_run_plus_a_summary() {
        let dir = std::env::temp_dir().join(format!(
            "vox-benchmark-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir);

        let run = run_case(
            &case("clean", BenchmarkCategory::CleanEnglish),
            &speech(2.0),
            Some("hello"),
            ScriptedDecoder::new(vec![Ok("hello".into())]),
            Pacing::Batch,
            &[],
        );
        BenchmarkReport::new(vec![run], Vec::new())
            .write_to(&dir)
            .expect("write the report");

        assert!(dir.join("report.json").exists());
        assert!(dir.join("report.md").exists());
        assert!(dir.join("clean__scripted__test.json").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
