//! The speech model catalogue, and the downloads that fill it.
//!
//! Before this module a model arrived by side effect: three filenames were
//! constants in `stt.rs`, dictation quietly fetched whichever one its quality
//! setting implied, and a meeting refused to start unless one specific file —
//! `ggml-small.bin` — happened to be on disk. A user who had only ever
//! dictated had `ggml-base.bin` and nothing else, so their first recording
//! failed with "no speech model is installed" while a perfectly usable model
//! sat in the same directory. The error pointed them at "Settings › Speech",
//! a section that did not exist.
//!
//! So the set of models is data here, in one list, and everything else reads
//! it:
//!
//! - [`catalogue`] is the whole managed set — the twelve whisper.cpp tiers,
//!   the same twelve Meetily offers, rather than the three that were reachable.
//! - [`download`] streams one to disk with progress, can be cancelled, and
//!   verifies what it wrote before moving it into place.
//! - [`installed`] answers what is actually on disk, including files the user
//!   put there themselves.
//!
//! ## Why a filename is the identity
//!
//! A model is addressed by catalogue id from the UI and by filename on disk,
//! and downloads are keyed against this list rather than against a URL the
//! frontend supplies. A command that accepts a URL is a command that will
//! eventually be asked to fetch something that is not a model into Vox's own
//! models directory. The set of things reachable from here is decided in this
//! file.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use crate::sync::MutexExt;

/// Where every managed model is fetched from.
///
/// One prefix rather than twelve URLs: the catalogue below names files, and a
/// typo in a URL that only appears for the tier nobody downloads is exactly
/// the kind of defect that ships.
const WHISPER_BASE_URL: &str = "https://huggingface.co/ggerganov/whisper.cpp/resolve/main";

/// What a model costs to run, in the terms a user choosing one cares about.
///
/// Not a benchmark. The ordering is the guarantee — `Fast` is never slower
/// than `Balanced` on the same machine — and the numbers users want come from
/// Diagnostics, measured on their hardware rather than asserted here.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelTier {
    /// Runs comfortably on any machine, and is the weakest.
    Fast,
    /// The working default: usable accuracy at a latency dictation can afford.
    Balanced,
    /// Noticeably better on accented and code-switched speech, and costs for it.
    Accurate,
    /// The ceiling. Minutes of decode on CPU for an hour of audio.
    Maximum,
}

/// One model Vox knows how to fetch and run.
#[derive(Debug, Clone, Copy)]
pub struct SpeechModel {
    /// Stable identifier used by the frontend and stored in settings.
    pub id: &'static str,
    /// Display name, without the "Whisper" prefix — the surface says that once.
    pub name: &'static str,
    /// The file on disk, and the last path segment of the download URL.
    pub filename: &'static str,
    /// Parameter count, in millions.
    pub parameters_millions: u32,
    /// Approximate download size, for the UI to show *before* a download
    /// starts. The real figure comes from `Content-Length` once one does.
    pub approx_bytes: u64,
    /// Whether the model handles languages other than English.
    ///
    /// The `.en` variants are meaningfully better on English and produce
    /// nonsense on anything else, which is worth saying plainly next to them.
    pub multilingual: bool,
    pub tier: ModelTier,
    /// One line explaining who should pick this.
    pub blurb: &'static str,
}

impl SpeechModel {
    /// Where this model lives once installed.
    pub fn path_in(&self, models_dir: &Path) -> PathBuf {
        models_dir.join(self.filename)
    }

    /// The URL this model is fetched from.
    pub fn url(&self) -> String {
        format!("{WHISPER_BASE_URL}/{}", self.filename)
    }
}

/// Every model Vox will download, coarsest first within each family.
///
/// The twelve whisper.cpp tiers. Listing all of them — including the ones most
/// users should not pick — is deliberate: a catalogue that hides the large
/// models is a catalogue that cannot explain why transcription of accented
/// speech is poor, and "download a bigger model" is the answer often enough to
/// be worth making reachable.
pub fn catalogue() -> &'static [SpeechModel] {
    &[
        SpeechModel {
            id: "whisper-tiny",
            name: "Tiny",
            filename: "ggml-tiny.bin",
            parameters_millions: 39,
            approx_bytes: 77_691_713,
            multilingual: true,
            tier: ModelTier::Fast,
            blurb: "Smallest and quickest. Fine for clear English dictation, weak on anything else.",
        },
        SpeechModel {
            id: "whisper-tiny-en",
            name: "Tiny (English)",
            filename: "ggml-tiny.en.bin",
            parameters_millions: 39,
            approx_bytes: 77_704_715,
            multilingual: false,
            tier: ModelTier::Fast,
            blurb: "Tiny, tuned for English only. Better than Tiny on English, unusable on other languages.",
        },
        SpeechModel {
            id: "whisper-base",
            name: "Base",
            filename: "ggml-base.bin",
            parameters_millions: 74,
            approx_bytes: 147_951_465,
            multilingual: true,
            tier: ModelTier::Fast,
            blurb: "The low-latency choice for push-to-talk dictation. Around a third of Small's decode time.",
        },
        SpeechModel {
            id: "whisper-base-en",
            name: "Base (English)",
            filename: "ggml-base.en.bin",
            parameters_millions: 74,
            approx_bytes: 147_964_211,
            multilingual: false,
            tier: ModelTier::Fast,
            blurb: "Base, tuned for English only.",
        },
        SpeechModel {
            id: "whisper-small",
            name: "Small",
            filename: "ggml-small.bin",
            parameters_millions: 244,
            approx_bytes: 487_601_967,
            multilingual: true,
            tier: ModelTier::Balanced,
            blurb: "The default. Handles English and Hindi, and is the smallest model worth pointing at a meeting.",
        },
        SpeechModel {
            id: "whisper-small-en",
            name: "Small (English)",
            filename: "ggml-small.en.bin",
            parameters_millions: 244,
            approx_bytes: 487_614_201,
            multilingual: false,
            tier: ModelTier::Balanced,
            blurb: "Small, tuned for English only.",
        },
        SpeechModel {
            id: "whisper-medium",
            name: "Medium",
            filename: "ggml-medium.bin",
            parameters_millions: 769,
            approx_bytes: 1_533_763_059,
            multilingual: true,
            tier: ModelTier::Accurate,
            blurb: "A real step up on accented speech. Large-v3 Turbo is usually the better trade at this size.",
        },
        SpeechModel {
            id: "whisper-medium-en",
            name: "Medium (English)",
            filename: "ggml-medium.en.bin",
            parameters_millions: 769,
            approx_bytes: 1_533_774_781,
            multilingual: false,
            tier: ModelTier::Accurate,
            blurb: "Medium, tuned for English only.",
        },
        SpeechModel {
            id: "whisper-large-v3-turbo",
            name: "Large v3 Turbo",
            filename: "ggml-large-v3-turbo.bin",
            parameters_millions: 809,
            approx_bytes: 1_624_555_275,
            multilingual: true,
            tier: ModelTier::Accurate,
            blurb: "Most of Large v3's accuracy at roughly a quarter of its decode cost. The best meeting model for most machines.",
        },
        SpeechModel {
            id: "whisper-large-v1",
            name: "Large v1",
            filename: "ggml-large-v1.bin",
            parameters_millions: 1550,
            approx_bytes: 3_094_623_691,
            multilingual: true,
            tier: ModelTier::Maximum,
            blurb: "Superseded by v2 and v3. Listed for reproducing an older transcript, not for new work.",
        },
        SpeechModel {
            id: "whisper-large-v2",
            name: "Large v2",
            filename: "ggml-large-v2.bin",
            parameters_millions: 1550,
            approx_bytes: 3_094_623_691,
            multilingual: true,
            tier: ModelTier::Maximum,
            blurb: "Superseded by v3 on most languages.",
        },
        SpeechModel {
            id: "whisper-large-v3",
            name: "Large v3",
            filename: "ggml-large-v3.bin",
            parameters_millions: 1550,
            approx_bytes: 3_095_033_483,
            multilingual: true,
            tier: ModelTier::Maximum,
            blurb: "The accuracy ceiling, and slow enough on CPU that an hour of audio takes hours to decode.",
        },
    ]
}

/// The model a fresh install should be pointed at.
pub const DEFAULT_MODEL_ID: &str = "whisper-small";

/// The model recommended for meetings when the user has not chosen.
///
/// Different from [`DEFAULT_MODEL_ID`] on purpose: a meeting is decoded in the
/// background while nobody waits on the next word, so it can afford accuracy
/// that push-to-talk dictation cannot.
pub const RECOMMENDED_MEETING_MODEL_ID: &str = "whisper-large-v3-turbo";

/// Looks a model up by catalogue id.
pub fn by_id(id: &str) -> Option<&'static SpeechModel> {
    catalogue().iter().find(|m| m.id == id)
}

/// Looks a model up by the name of its file on disk.
///
/// Exists because settings store a path, not an id: a model configured before
/// this catalogue existed is still recognisable as a managed one.
pub fn by_filename(filename: &str) -> Option<&'static SpeechModel> {
    catalogue().iter().find(|m| m.filename == filename)
}

/// Looks a model up from a full path, by its file name.
pub fn by_path(path: &Path) -> Option<&'static SpeechModel> {
    path.file_name()
        .and_then(|n| n.to_str())
        .and_then(by_filename)
}

/// The smallest size a file can be and still plausibly be a Whisper model.
///
/// Guards against a truncated download or an HTML error page saved under a
/// `.bin` name — both of which otherwise present as "installed" and fail much
/// later, inside whisper.cpp, with a message about tensors.
const MIN_PLAUSIBLE_MODEL_BYTES: u64 = 1_000_000;

/// What is known about one model on this machine.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct InstalledModel {
    pub id: String,
    pub name: String,
    pub filename: String,
    pub path: String,
    /// Bytes on disk, or the catalogue's estimate when not installed.
    pub size_bytes: u64,
    pub installed: bool,
    /// False for a `.bin` the user dropped in the models directory themselves.
    pub managed: bool,
    pub multilingual: bool,
    pub parameters_millions: u32,
    pub tier: ModelTier,
    pub blurb: String,
}

/// The catalogue crossed with what is on disk, plus what each surface will use.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SpeechModelCatalogue {
    pub models_dir: String,
    pub models: Vec<InstalledModel>,
    /// Id (or, for an unmanaged file, the filename) the next meeting will use.
    pub active_meeting_model: Option<String>,
    /// Id the next dictation will use.
    pub active_dictation_model: Option<String>,
    /// Catalogue id a fresh install should be offered first.
    pub recommended_meeting_model: String,
}

/// Whether a path is a plausible, present model file.
pub fn is_installed(path: &Path) -> bool {
    std::fs::metadata(path)
        .map(|m| m.is_file() && m.len() >= MIN_PLAUSIBLE_MODEL_BYTES)
        .unwrap_or(false)
}

/// Every model on disk, catalogue entries and hand-placed files alike.
///
/// Catalogue order is preserved so the list does not reshuffle as downloads
/// land; unmanaged files follow, sorted, so the tail is stable too.
pub fn installed(models_dir: &Path) -> Vec<InstalledModel> {
    let mut out: Vec<InstalledModel> = catalogue()
        .iter()
        .map(|model| {
            let path = model.path_in(models_dir);
            let size = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
            InstalledModel {
                id: model.id.to_string(),
                name: model.name.to_string(),
                filename: model.filename.to_string(),
                path: path.to_string_lossy().to_string(),
                size_bytes: if size > 0 { size } else { model.approx_bytes },
                installed: is_installed(&path),
                managed: true,
                multilingual: model.multilingual,
                parameters_millions: model.parameters_millions,
                tier: model.tier,
                blurb: model.blurb.to_string(),
            }
        })
        .collect();

    // Files the user put here themselves. Listed rather than ignored: a model
    // Vox cannot see is a model the user cannot select, and they went to the
    // trouble of placing it.
    let mut extras: Vec<InstalledModel> = Vec::new();
    if let Ok(entries) = std::fs::read_dir(models_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("bin") {
                continue;
            }
            let Some(filename) = path.file_name().and_then(|n| n.to_str()) else {
                continue;
            };
            if by_filename(filename).is_some() {
                continue;
            }
            if !is_installed(&path) {
                continue;
            }
            extras.push(InstalledModel {
                id: filename.to_string(),
                name: filename.to_string(),
                filename: filename.to_string(),
                path: path.to_string_lossy().to_string(),
                size_bytes: entry.metadata().map(|m| m.len()).unwrap_or(0),
                installed: true,
                managed: false,
                multilingual: true,
                parameters_millions: 0,
                tier: ModelTier::Balanced,
                blurb: "Added to the models folder by hand.".to_string(),
            });
        }
    }
    extras.sort_by(|a, b| a.filename.cmp(&b.filename));
    out.extend(extras);
    out
}

/// Deletes an installed model, returning whether there was one to delete.
///
/// Only reaches inside `models_dir`, and only files this module would have
/// written or listed — a delete command that took a path would eventually be
/// handed one that is not a model.
pub fn delete(models_dir: &Path, id: &str) -> Result<bool, std::io::Error> {
    let filename = by_id(id).map(|m| m.filename.to_string()).unwrap_or_else(|| id.to_string());
    // Reject anything that could escape the models directory.
    if filename.contains('/') || filename.contains('\\') || filename.contains("..") {
        return Ok(false);
    }
    let path = models_dir.join(&filename);
    if !path.is_file() {
        return Ok(false);
    }
    std::fs::remove_file(&path)?;
    Ok(true)
}

// ---------------------------------------------------------------------------
// Downloads
// ---------------------------------------------------------------------------

/// How a download is going, as the UI needs to render it.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case", tag = "state")]
pub enum DownloadProgress {
    /// Bytes are arriving. `total_bytes` is `None` when the server sent no
    /// `Content-Length` — rare on Hugging Face, but the UI has to cope.
    Downloading {
        id: String,
        downloaded_bytes: u64,
        total_bytes: Option<u64>,
    },
    /// The transfer finished and the file is being checked.
    Verifying { id: String },
    Ready { id: String, path: String },
    Failed { id: String, message: String },
    Cancelled { id: String },
}

impl DownloadProgress {
    /// The model this progress is about, for routing in the frontend.
    pub fn id(&self) -> &str {
        match self {
            DownloadProgress::Downloading { id, .. }
            | DownloadProgress::Verifying { id }
            | DownloadProgress::Ready { id, .. }
            | DownloadProgress::Failed { id, .. }
            | DownloadProgress::Cancelled { id } => id,
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum DownloadError {
    #[error("'{0}' is not a model Vox knows how to download")]
    UnknownModel(String),

    #[error("a download of '{0}' is already running")]
    AlreadyRunning(String),

    #[error("download cancelled")]
    Cancelled,

    #[error("could not reach the model server: {0}")]
    Network(String),

    #[error("model server returned HTTP {0}")]
    HttpStatus(u16),

    #[error("could not write to {path}: {message}")]
    Io { path: String, message: String },

    #[error("the downloaded file is not a Whisper model")]
    NotAModel,
}

/// In-flight downloads, so a second press cancels rather than races.
static IN_FLIGHT: OnceLock<Mutex<HashMap<String, Arc<AtomicBool>>>> = OnceLock::new();

fn in_flight() -> &'static Mutex<HashMap<String, Arc<AtomicBool>>> {
    IN_FLIGHT.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Asks a running download to stop. Returns false when none was running.
pub fn cancel(id: &str) -> bool {
    match in_flight().lock_or_recover().get(id) {
        Some(flag) => {
            flag.store(true, Ordering::SeqCst);
            true
        }
        None => false,
    }
}

/// Whether a download for this model is currently running.
pub fn is_downloading(id: &str) -> bool {
    in_flight().lock_or_recover().contains_key(id)
}

/// Clears the in-flight entry however the download ended.
struct InFlightGuard(String);

impl Drop for InFlightGuard {
    fn drop(&mut self) {
        in_flight().lock_or_recover().remove(&self.0);
    }
}

/// Whether these bytes start with a GGML/GGUF magic number.
///
/// The check Meetily does, and worth doing: Hugging Face serves an HTML error
/// page with a 200 often enough that "the file exists and is large" is not
/// sufficient evidence that a model was downloaded.
fn has_model_magic(header: &[u8]) -> bool {
    matches!(
        header.get(..4),
        Some(b"ggml") | Some(b"lmgg") | Some(b"GGUF") | Some(b"FUGG")
    )
}

/// Fetches one catalogue model into `models_dir`, reporting progress.
///
/// Streams to a `.part` file and renames only after the magic-number check, so
/// an interrupted or corrupt download can never be mistaken for an installed
/// model. `on_progress` is called on the async task, frequently — a Tauri
/// `emit` is cheap enough for that; anything expensive belongs behind a
/// throttle in the caller.
pub async fn download<F>(
    models_dir: &Path,
    id: &str,
    on_progress: F,
) -> Result<PathBuf, DownloadError>
where
    F: Fn(DownloadProgress) + Send + 'static,
{
    let model = by_id(id).ok_or_else(|| DownloadError::UnknownModel(id.to_string()))?;
    let target = model.path_in(models_dir);

    if is_installed(&target) {
        on_progress(DownloadProgress::Ready {
            id: id.to_string(),
            path: target.to_string_lossy().to_string(),
        });
        return Ok(target);
    }

    let cancel_flag = {
        let mut running = in_flight().lock_or_recover();
        if running.contains_key(id) {
            return Err(DownloadError::AlreadyRunning(id.to_string()));
        }
        let flag = Arc::new(AtomicBool::new(false));
        running.insert(id.to_string(), Arc::clone(&flag));
        flag
    };
    let _guard = InFlightGuard(id.to_string());

    let result = stream_to_disk(models_dir, model, &cancel_flag, &on_progress).await;

    match &result {
        Ok(path) => on_progress(DownloadProgress::Ready {
            id: id.to_string(),
            path: path.to_string_lossy().to_string(),
        }),
        Err(DownloadError::Cancelled) => {
            on_progress(DownloadProgress::Cancelled { id: id.to_string() })
        }
        Err(err) => on_progress(DownloadProgress::Failed {
            id: id.to_string(),
            message: err.to_string(),
        }),
    }

    result
}

/// The transfer itself, split out so [`download`] owns only the bookkeeping.
async fn stream_to_disk<F>(
    models_dir: &Path,
    model: &SpeechModel,
    cancel_flag: &AtomicBool,
    on_progress: &F,
) -> Result<PathBuf, DownloadError>
where
    F: Fn(DownloadProgress),
{
    use std::io::Write;

    std::fs::create_dir_all(models_dir).map_err(|e| DownloadError::Io {
        path: models_dir.display().to_string(),
        message: e.to_string(),
    })?;

    let target = model.path_in(models_dir);
    let part = target.with_extension("part");

    let response = reqwest::get(model.url())
        .await
        .map_err(|e| DownloadError::Network(e.to_string()))?;

    if !response.status().is_success() {
        return Err(DownloadError::HttpStatus(response.status().as_u16()));
    }

    let total_bytes = response.content_length();
    let mut file = std::fs::File::create(&part).map_err(|e| DownloadError::Io {
        path: part.display().to_string(),
        message: e.to_string(),
    })?;

    let mut downloaded: u64 = 0;
    let mut header: Vec<u8> = Vec::with_capacity(4);
    // Report on a byte interval rather than per chunk: a 3 GB model arrives in
    // tens of thousands of chunks, and an event for each one is a frontend
    // that spends the download re-rendering a progress bar.
    let report_every = 2 * 1024 * 1024;
    let mut next_report = report_every;

    let mut response = response;
    loop {
        if cancel_flag.load(Ordering::SeqCst) {
            drop(file);
            let _ = std::fs::remove_file(&part);
            return Err(DownloadError::Cancelled);
        }

        let chunk = match response.chunk().await {
            Ok(Some(chunk)) => chunk,
            Ok(None) => break,
            Err(e) => {
                drop(file);
                let _ = std::fs::remove_file(&part);
                return Err(DownloadError::Network(e.to_string()));
            }
        };

        if header.len() < 4 {
            header.extend_from_slice(&chunk[..chunk.len().min(4 - header.len())]);
        }

        file.write_all(&chunk).map_err(|e| DownloadError::Io {
            path: part.display().to_string(),
            message: e.to_string(),
        })?;
        downloaded += chunk.len() as u64;

        if downloaded >= next_report {
            next_report = downloaded + report_every;
            on_progress(DownloadProgress::Downloading {
                id: model.id.to_string(),
                downloaded_bytes: downloaded,
                total_bytes,
            });
        }
    }

    file.flush().map_err(|e| DownloadError::Io {
        path: part.display().to_string(),
        message: e.to_string(),
    })?;
    drop(file);

    on_progress(DownloadProgress::Verifying {
        id: model.id.to_string(),
    });

    if downloaded < MIN_PLAUSIBLE_MODEL_BYTES || !has_model_magic(&header) {
        let _ = std::fs::remove_file(&part);
        return Err(DownloadError::NotAModel);
    }

    std::fs::rename(&part, &target).map_err(|e| DownloadError::Io {
        path: target.display().to_string(),
        message: e.to_string(),
    })?;

    tracing::info!("speech model {} installed at {}", model.id, target.display());
    Ok(target)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A models directory that cleans itself up.
    ///
    /// Hand-rolled rather than pulling in `tempfile`: the crate has no
    /// dev-dependencies and this is the same `env::temp_dir()` + uuid idiom
    /// `oauth::tokens` already uses, one `Drop` short of being tidy.
    struct TestDir(PathBuf);

    impl TestDir {
        fn new() -> Self {
            let path = std::env::temp_dir()
                .join(format!("vox_test_models_{}", uuid::Uuid::new_v4()));
            std::fs::create_dir_all(&path).unwrap();
            Self(path)
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TestDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    /// A file big enough and magic enough to pass for an installed model.
    fn write_model(dir: &Path, filename: &str) {
        let mut bytes = b"ggml".to_vec();
        bytes.resize(MIN_PLAUSIBLE_MODEL_BYTES as usize + 1, b'x');
        std::fs::write(dir.join(filename), bytes).unwrap();
    }

    #[test]
    fn every_catalogue_entry_is_uniquely_identified() {
        let mut ids: Vec<&str> = catalogue().iter().map(|m| m.id).collect();
        let count = ids.len();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), count, "catalogue ids must be unique");

        let mut files: Vec<&str> = catalogue().iter().map(|m| m.filename).collect();
        files.sort_unstable();
        files.dedup();
        assert_eq!(files.len(), count, "catalogue filenames must be unique");
    }

    #[test]
    fn the_defaults_name_models_that_exist() {
        assert!(by_id(DEFAULT_MODEL_ID).is_some());
        assert!(by_id(RECOMMENDED_MEETING_MODEL_ID).is_some());
    }

    #[test]
    fn english_only_variants_are_marked_as_such() {
        for model in catalogue() {
            let is_en_file = model.filename.contains(".en.");
            assert_eq!(
                is_en_file, !model.multilingual,
                "{} disagrees with its filename about being English-only",
                model.id
            );
        }
    }

    #[test]
    fn a_model_url_is_built_from_its_filename() {
        let small = by_id("whisper-small").unwrap();
        assert_eq!(
            small.url(),
            "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-small.bin"
        );
    }

    #[test]
    fn a_short_or_html_file_is_not_a_model() {
        assert!(has_model_magic(b"ggml rest of header"));
        assert!(has_model_magic(b"GGUF rest of header"));
        assert!(!has_model_magic(b"<!DOCTYPE html>"));
        assert!(!has_model_magic(b"gg"));
    }

    #[test]
    fn a_truncated_file_does_not_count_as_installed() {
        let dir = TestDir::new();
        let path = dir.path().join("ggml-small.bin");
        std::fs::write(&path, b"ggml but far too short").unwrap();
        assert!(!is_installed(&path));
    }

    #[test]
    fn installed_lists_the_whole_catalogue_and_marks_what_is_present() {
        let dir = TestDir::new();
        write_model(dir.path(), "ggml-base.bin");

        let listed = installed(dir.path());
        assert_eq!(listed.len(), catalogue().len());

        let base = listed.iter().find(|m| m.id == "whisper-base").unwrap();
        assert!(base.installed);
        let small = listed.iter().find(|m| m.id == "whisper-small").unwrap();
        assert!(!small.installed);
        // A model that is not installed still reports a size, so the UI can
        // say what the download will cost before it starts.
        assert_eq!(small.size_bytes, by_id("whisper-small").unwrap().approx_bytes);
    }

    #[test]
    fn a_hand_placed_model_is_listed_as_unmanaged() {
        let dir = TestDir::new();
        write_model(dir.path(), "my-finetune.bin");

        let listed = installed(dir.path());
        let extra = listed.iter().find(|m| m.filename == "my-finetune.bin").unwrap();
        assert!(extra.installed);
        assert!(!extra.managed);
    }

    #[test]
    fn delete_removes_a_managed_model_and_refuses_a_traversal() {
        let dir = TestDir::new();
        let path = dir.path().join("ggml-base.bin");
        write_model(dir.path(), "ggml-base.bin");

        assert!(delete(dir.path(), "whisper-base").unwrap());
        assert!(!path.exists());
        // Deleting again is not an error, it is simply nothing to do.
        assert!(!delete(dir.path(), "whisper-base").unwrap());
        assert!(!delete(dir.path(), "../../etc/passwd").unwrap());
    }

    #[test]
    fn cancelling_a_download_that_is_not_running_reports_so() {
        assert!(!cancel("whisper-large-v3"));
        assert!(!is_downloading("whisper-large-v3"));
    }

    #[tokio::test]
    async fn downloading_a_model_that_is_already_present_succeeds_immediately() {
        let dir = TestDir::new();
        write_model(dir.path(), "ggml-base.bin");

        let path = download(dir.path(), "whisper-base", |_| {}).await.unwrap();
        assert!(path.ends_with("ggml-base.bin"));
    }

    #[tokio::test]
    async fn an_unknown_model_is_refused_before_any_network_call() {
        let dir = TestDir::new();
        let err = download(dir.path(), "not-a-model", |_| {}).await.unwrap_err();
        assert!(matches!(err, DownloadError::UnknownModel(_)));
    }
}
