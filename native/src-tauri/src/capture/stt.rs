use crate::sync::MutexExt;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use thiserror::Error;

#[cfg(feature = "whisper-local")]
use whisper_rs::{FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters};

#[derive(Error, Debug)]
pub enum SttError {
    #[error("Set Whisper model path in Provider Settings.")]
    ModelNotConfigured,

    #[error("Failed to load Whisper model at {path}: {message}")]
    ModelLoadFailed { path: String, message: String },

    #[error("Whisper transcription failed: {0}")]
    TranscriptionFailed(String),
}

/// Production default multilingual Whisper model (ggml-small.bin, 244M params).
/// Provides reliable English/Hindi code-switching, robust language separation,
/// and low zero-cost latency on local CPU.
pub const DEFAULT_MODEL_FILENAME: &str = "ggml-small.bin";
pub const DEFAULT_MODEL_URL: &str =
    "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-small.bin";

/// High-speed multilingual Whisper model for fast universal dictation (ggml-base.bin, 39M params).
/// Provides ~3x lower latency (~0.8s vs ~2.4s) on CPU for conversational dictation.
pub const FAST_MODEL_FILENAME: &str = "ggml-base.bin";
pub const FAST_MODEL_URL: &str =
    "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-base.bin";

/// The accuracy ceiling: `ggml-large-v3-turbo.bin`, 809M params, ~1.6 GB.
///
/// Offered because `small` is genuinely marginal on the audio Relay is used
/// for. Its header above claims "reliable English/Hindi code-switching"; a
/// five-minute Hinglish standup arriving as fifty-six words is what that claim
/// failing looks like. Whisper's word error rate on Hindi falls sharply from
/// `small` to `large-v3`, and the turbo variant keeps most of that gain at
/// roughly a quarter of `large-v3`'s decode cost — which is what makes it
/// viable at all in a local-first app.
///
/// Never a default. It is a deliberate download and a real latency cost, so
/// the user chooses it and Diagnostics measures what it costs on their machine
/// rather than either of us guessing.
pub const ACCURATE_MODEL_FILENAME: &str = "ggml-large-v3-turbo.bin";
pub const ACCURATE_MODEL_URL: &str =
    "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-large-v3-turbo.bin";

/// Fetches the accuracy-ceiling model. Large, so never called implicitly.
pub async fn ensure_accurate_model(models_dir: &Path) -> Result<PathBuf, SttError> {
    ensure_model_file(models_dir, ACCURATE_MODEL_FILENAME, ACCURATE_MODEL_URL).await
}

/// Downloads one of Vox's three managed models, named by filename.
///
/// Keyed by filename against the managed set rather than taking a URL. A
/// command that accepts a URL from the frontend is a command that will
/// eventually be asked to fetch something that is not a model, and the
/// download writes into Relay's own models directory — so the set of things
/// it can be pointed at is decided here, in code, and not by its caller.
///
/// This is what makes [`ensure_accurate_model`] reachable. The function has
/// existed since the accuracy tier was added and never had a caller: the
/// models overview lists `large-v3-turbo` as "missing" precisely so the UI can
/// offer the download, and nothing exposed one.
/// Whether [`ensure_managed_model`] can fetch this filename.
///
/// Exists so the models overview and the download command cannot drift: a tier
/// offered by one and unknown to the other is a download button that always
/// fails.
pub fn managed_model_is_known(filename: &str) -> bool {
    matches!(
        filename,
        FAST_MODEL_FILENAME | DEFAULT_MODEL_FILENAME | ACCURATE_MODEL_FILENAME
    )
}

pub async fn ensure_managed_model(models_dir: &Path, filename: &str) -> Result<PathBuf, SttError> {
    match filename {
        FAST_MODEL_FILENAME => ensure_fast_model(models_dir).await,
        DEFAULT_MODEL_FILENAME => ensure_default_model(models_dir).await,
        ACCURATE_MODEL_FILENAME => ensure_accurate_model(models_dir).await,
        other => Err(SttError::ModelLoadFailed {
            path: other.to_string(),
            message: format!("'{other}' is not one of Vox's managed models"),
        }),
    }
}

/// Checks if a configured model path represents a legacy default model
/// (e.g. `ggml-tiny.en.bin`) so Relay can seamlessly promote to `ggml-small.bin`.
/// The Whisper model a meeting recording would use, or `None` when none is
/// available.
///
/// Factored out of the meetings engine so the diagnostics self-test asks the
/// *same* model a real recording would. A self-test that ran against a
/// different model than the app uses would be worse than none: it would report
/// green for a model the user never records with.
pub fn resolve_meeting_model_path(
    models_dir: &Path,
    stt_settings: &crate::settings::SttSettings,
) -> Option<PathBuf> {
    // 1. The model the user chose for meetings, if it is actually installed.
    //    A chosen-then-deleted model falls through rather than failing: the
    //    setting is a preference, not a promise about the filesystem.
    if let Some(id) = stt_settings
        .meeting_model_id
        .as_deref()
        .map(str::trim)
        .filter(|id| !id.is_empty())
    {
        if let Some(model) = crate::capture::models::by_id(id) {
            let path = model.path_in(models_dir);
            if crate::capture::models::is_installed(&path) {
                return Some(path);
            }
        } else {
            // An unmanaged file the user dropped in the models folder, chosen
            // by filename.
            let path = models_dir.join(id);
            if crate::capture::models::is_installed(&path) {
                return Some(path);
            }
        }
    }

    // 2. The path dictation is configured with, if it exists. Shared only as a
    //    fallback — a user who set this before meetings existed still gets a
    //    working recording.
    if let Some(path) = stt_settings
        .whisper_model_path
        .as_deref()
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .map(PathBuf::from)
    {
        if crate::capture::models::is_installed(&path) {
            return Some(path);
        }
    }

    // 3. Anything installed, best first.
    //
    //    This branch is why the "no speech model is installed" error used to
    //    be wrong. The old resolution looked only for `ggml-small.bin`, so a
    //    user who had downloaded Base for dictation — the fast-profile default,
    //    and the only model many installs ever fetch — was told they had no
    //    model at all while one sat in the same directory.
    best_installed_model(models_dir)
}

/// The most accurate installed model, or `None` when the directory is empty.
///
/// "Best" is catalogue order reversed within the managed set: Large v3 beats
/// Turbo beats Medium beats Small. An unmanaged file is used only when nothing
/// managed is present, since nothing here knows how good it is.
fn best_installed_model(models_dir: &Path) -> Option<PathBuf> {
    use crate::capture::models::{self, ModelTier};

    let listed = models::installed(models_dir);
    let rank = |tier: ModelTier| match tier {
        ModelTier::Maximum => 3,
        ModelTier::Accurate => 2,
        ModelTier::Balanced => 1,
        ModelTier::Fast => 0,
    };

    listed
        .iter()
        .filter(|m| m.installed && m.managed)
        .max_by_key(|m| rank(m.tier))
        .or_else(|| listed.iter().find(|m| m.installed))
        .map(|m| PathBuf::from(&m.path))
}

pub fn is_legacy_default_model(path: &Path) -> bool {
    if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
        name == "ggml-tiny.en.bin"
    } else {
        false
    }
}

/// Serializes downloads: `start_capture` fires one of these in the
/// background to get a head start, and the transcription step fires
/// another right after — without this, both could race to write the same
/// temp file at once instead of the second simply finding the first's
/// finished download already in place.
static DOWNLOAD_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// Fetches a model from `url` into `models_dir` if it does not already exist.
async fn ensure_model_file(models_dir: &Path, filename: &str, url: &str) -> Result<PathBuf, SttError> {
    let target = models_dir.join(filename);
    if target.exists() {
        return Ok(target);
    }

    let _guard = DOWNLOAD_LOCK.lock().await;
    if target.exists() {
        return Ok(target);
    }

    std::fs::create_dir_all(models_dir).map_err(|e| SttError::ModelLoadFailed {
        path: target.display().to_string(),
        message: e.to_string(),
    })?;

    tracing::info!(
        "Downloading Whisper model {} to {}",
        filename,
        target.display()
    );

    let response = reqwest::get(url)
        .await
        .map_err(|e| SttError::ModelLoadFailed {
            path: url.to_string(),
            message: format!("Failed to download model: {}", e),
        })?;

    if !response.status().is_success() {
        return Err(SttError::ModelLoadFailed {
            path: url.to_string(),
            message: format!("HTTP {} from model server", response.status()),
        });
    }

    let bytes = response.bytes().await.map_err(|e| SttError::ModelLoadFailed {
        path: url.to_string(),
        message: format!("Failed to read response body: {}", e),
    })?;

    let tmp_path = target.with_extension("bin.part");
    std::fs::write(&tmp_path, &bytes).map_err(|e| SttError::ModelLoadFailed {
        path: tmp_path.display().to_string(),
        message: e.to_string(),
    })?;
    std::fs::rename(&tmp_path, &target).map_err(|e| SttError::ModelLoadFailed {
        path: target.display().to_string(),
        message: e.to_string(),
    })?;

    tracing::info!("Whisper model ready at {}", target.display());
    Ok(target)
}

/// If no Whisper model is configured, fetches the production small default model into `models_dir` and returns its path.
pub async fn ensure_default_model(models_dir: &Path) -> Result<PathBuf, SttError> {
    ensure_model_file(models_dir, DEFAULT_MODEL_FILENAME, DEFAULT_MODEL_URL).await
}

/// Ensures the fast base model is available for Universal Dictation Fast profile.
pub async fn ensure_fast_model(models_dir: &Path) -> Result<PathBuf, SttError> {
    ensure_model_file(models_dir, FAST_MODEL_FILENAME, FAST_MODEL_URL).await
}

/// Which model file dictation would use, without fetching anything.
///
/// The read-only counterpart to [`resolve_dictation_model_path`], which
/// downloads as a side effect of being asked. A settings screen listing what is
/// installed must not start a download merely by rendering.
pub fn dictation_model_filename(stt_settings: &crate::settings::SttSettings) -> Option<String> {
    if let Some(ref p) = stt_settings.whisper_model_path {
        let trimmed = p.trim();
        if !trimmed.is_empty() {
            if let Some(filename) = Path::new(trimmed).file_name() {
                return Some(filename.to_string_lossy().to_string());
            }
        }
    }
    match stt_settings.dictation_quality {
        crate::settings::DictationSttQuality::Fast => Some(FAST_MODEL_FILENAME.to_string()),
        crate::settings::DictationSttQuality::Accurate => Some(DEFAULT_MODEL_FILENAME.to_string()),
    }
}

/// The dictation model the settings resolve to, if it is already on disk.
///
/// An explicitly configured model wins — by its path, or by its filename in
/// `models_dir`. Otherwise the quality preset picks: `Fast` is
/// `ggml-base.bin`, falling back to `ggml-small.bin` when only that is
/// installed; `Accurate` is `ggml-small.bin`.
///
/// Synchronous and side-effect free, so a caller can find out *before* the
/// user is waiting whether [`resolve_dictation_model_path`] is about to
/// download something.
pub fn installed_dictation_model_path(
    models_dir: &Path,
    stt_settings: &crate::settings::SttSettings,
) -> Option<String> {
    if let Some(ref path) = stt_settings.whisper_model_path {
        let trimmed = path.trim();
        if !trimmed.is_empty() {
            let p = Path::new(trimmed);
            if p.exists() {
                return Some(trimmed.to_string());
            }
            if let Some(filename) = p.file_name() {
                let candidate = models_dir.join(filename);
                if candidate.exists() {
                    return Some(candidate.to_string_lossy().to_string());
                }
            }
        }
    }

    let preferred: &[&str] = match stt_settings.dictation_quality {
        crate::settings::DictationSttQuality::Fast => &[FAST_MODEL_FILENAME, DEFAULT_MODEL_FILENAME],
        crate::settings::DictationSttQuality::Accurate => &[DEFAULT_MODEL_FILENAME],
    };
    preferred
        .iter()
        .map(|name| models_dir.join(name))
        .find(|candidate| candidate.exists())
        .map(|candidate| candidate.to_string_lossy().to_string())
}

/// Resolves the effective dictation model path, downloading the preset's model
/// when nothing suitable is installed.
///
/// In `Fast` mode the download is `ggml-base.bin` (~0.8s latency); in
/// `Accurate` mode it is `ggml-small.bin` (~2.4s latency). Callers on the
/// dictation hot path check [`installed_dictation_model_path`] first so they
/// can tell the user a download is happening.
pub async fn resolve_dictation_model_path(
    models_dir: &Path,
    stt_settings: &crate::settings::SttSettings,
) -> Option<String> {
    if let Some(installed) = installed_dictation_model_path(models_dir, stt_settings) {
        return Some(installed);
    }

    let downloaded = match stt_settings.dictation_quality {
        crate::settings::DictationSttQuality::Fast => ensure_fast_model(models_dir).await,
        crate::settings::DictationSttQuality::Accurate => ensure_default_model(models_dir).await,
    };
    match downloaded {
        Ok(p) => Some(p.to_string_lossy().to_string()),
        Err(e) => {
            tracing::warn!("Could not download the dictation Whisper model: {}", e);
            // A configured path that does not exist is still passed through,
            // so the decode reports the missing file by name rather than
            // falling back silently to a model the user did not pick.
            stt_settings.whisper_model_path.clone()
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SttModelInfo {
    pub name: String,
    pub filename: String,
    pub path: String,
    pub size_bytes: u64,
    pub exists: bool,
    pub is_managed: bool,
    pub profile: Option<String>,
    pub status: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SttModelsOverview {
    pub active_model_name: String,
    pub active_model_path: String,
    pub active_profile: String,
    pub models_dir: String,
    pub models: Vec<SttModelInfo>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SttModelTestResult {
    pub success: bool,
    pub path: String,
    pub size_bytes: u64,
    pub latency_ms: u64,
    pub error: Option<String>,
}

pub fn get_stt_models_overview(
    models_dir: &Path,
    stt_settings: &crate::settings::SttSettings,
) -> SttModelsOverview {
    let mut models = Vec::new();

    // 1. Fast Base Model
    let fast_path = models_dir.join(FAST_MODEL_FILENAME);
    let fast_exists = fast_path.is_file();
    let fast_size = if fast_exists {
        std::fs::metadata(&fast_path).map(|m| m.len()).unwrap_or(0)
    } else {
        0
    };
    models.push(SttModelInfo {
        name: "Whisper Base".to_string(),
        filename: FAST_MODEL_FILENAME.to_string(),
        path: fast_path.to_string_lossy().to_string(),
        size_bytes: fast_size,
        exists: fast_exists,
        is_managed: true,
        profile: Some("fast".to_string()),
        status: if fast_exists && fast_size > 1_000_000 {
            "ready".to_string()
        } else {
            "missing".to_string()
        },
    });

    // 3. The accuracy ceiling, listed whether or not it is present so the UI
    // can offer the download rather than hiding that the option exists.
    let accurate_path = models_dir.join(ACCURATE_MODEL_FILENAME);
    let accurate_exists = accurate_path.is_file();
    let accurate_size = if accurate_exists {
        std::fs::metadata(&accurate_path).map(|m| m.len()).unwrap_or(0)
    } else {
        0
    };

    // 2. Accurate Small Model (Production Default)
    let default_path = models_dir.join(DEFAULT_MODEL_FILENAME);
    let default_exists = default_path.is_file();
    let default_size = if default_exists {
        std::fs::metadata(&default_path).map(|m| m.len()).unwrap_or(0)
    } else {
        0
    };
    models.push(SttModelInfo {
        name: "Whisper Small (Default)".to_string(),
        filename: DEFAULT_MODEL_FILENAME.to_string(),
        path: default_path.to_string_lossy().to_string(),
        size_bytes: default_size,
        exists: default_exists,
        is_managed: true,
        profile: Some("accurate".to_string()),
        status: if default_exists && default_size > 1_000_000 {
            "ready".to_string()
        } else {
            "missing".to_string()
        },
    });

    models.push(SttModelInfo {
        name: "Whisper Large v3 Turbo".to_string(),
        filename: ACCURATE_MODEL_FILENAME.to_string(),
        path: accurate_path.to_string_lossy().to_string(),
        size_bytes: accurate_size,
        exists: accurate_exists,
        is_managed: true,
        profile: Some("maximum".to_string()),
        status: if accurate_exists && accurate_size > 1_000_000 {
            "ready".to_string()
        } else {
            "missing".to_string()
        },
    });

    // 4. Scan models_dir for any other .bin files
    if let Ok(entries) = std::fs::read_dir(models_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_file() {
                if let Some(ext) = path.extension() {
                    if ext == "bin" {
                        let fname = path.file_name().unwrap_or_default().to_string_lossy().to_string();
                        if fname != FAST_MODEL_FILENAME
                            && fname != DEFAULT_MODEL_FILENAME
                            && fname != ACCURATE_MODEL_FILENAME
                        {
                            let size = entry.metadata().map(|m| m.len()).unwrap_or(0);
                            models.push(SttModelInfo {
                                name: format!("Custom ({})", fname),
                                filename: fname,
                                path: path.to_string_lossy().to_string(),
                                size_bytes: size,
                                exists: true,
                                is_managed: true,
                                profile: Some("custom".to_string()),
                                status: if size > 1_000_000 { "ready".to_string() } else { "missing".to_string() },
                            });
                        }
                    }
                }
            }
        }
    }

    // 4. Configured custom model path if specified outside or inside models_dir
    let custom_path_str = stt_settings
        .whisper_model_path
        .as_ref()
        .map(|s| s.trim())
        .filter(|s| !s.is_empty());

    let mut custom_model_active = false;
    if let Some(cpath) = custom_path_str {
        let p = Path::new(cpath);
        let fname = p.file_name().unwrap_or_default().to_string_lossy().to_string();
        if fname != FAST_MODEL_FILENAME && fname != DEFAULT_MODEL_FILENAME {
            custom_model_active = true;
            let already_listed = models.iter().any(|m| m.path == cpath);
            if !already_listed {
                let exists = p.is_file();
                let size = if exists {
                    std::fs::metadata(p).map(|m| m.len()).unwrap_or(0)
                } else {
                    0
                };
                models.push(SttModelInfo {
                    name: format!("Custom ({})", fname),
                    filename: fname,
                    path: cpath.to_string(),
                    size_bytes: size,
                    exists,
                    is_managed: false,
                    profile: Some("custom".to_string()),
                    status: if exists && size > 1_000_000 {
                        "ready".to_string()
                    } else {
                        "missing".to_string()
                    },
                });
            }
        }
    }

    // Resolve Active Model and Profile
    let (active_profile, active_model_path, active_model_name) = if custom_model_active {
        let cpath = custom_path_str.unwrap_or_default();
        let fname = Path::new(cpath).file_name().unwrap_or_default().to_string_lossy().to_string();
        ("custom".to_string(), cpath.to_string(), format!("Custom ({})", fname))
    } else {
        match stt_settings.dictation_quality {
            crate::settings::DictationSttQuality::Fast => (
                "fast".to_string(),
                fast_path.to_string_lossy().to_string(),
                "Whisper Base".to_string(),
            ),
            crate::settings::DictationSttQuality::Accurate => (
                "accurate".to_string(),
                default_path.to_string_lossy().to_string(),
                "Whisper Small (Default)".to_string(),
            ),
        }
    };

    SttModelsOverview {
        active_model_name,
        active_model_path,
        active_profile,
        models_dir: models_dir.to_string_lossy().to_string(),
        models,
    }
}

pub fn test_stt_model_file(path_str: &str) -> SttModelTestResult {
    let start = std::time::Instant::now();
    let p = Path::new(path_str);
    if !p.exists() {
        return SttModelTestResult {
            success: false,
            path: path_str.to_string(),
            size_bytes: 0,
            latency_ms: start.elapsed().as_millis() as u64,
            error: Some("File does not exist".to_string()),
        };
    }
    let meta = match std::fs::metadata(p) {
        Ok(m) => m,
        Err(e) => {
            return SttModelTestResult {
                success: false,
                path: path_str.to_string(),
                size_bytes: 0,
                latency_ms: start.elapsed().as_millis() as u64,
                error: Some(format!("Could not read metadata: {}", e)),
            };
        }
    };

    let size_bytes = meta.len();
    if size_bytes < 1_000_000 {
        return SttModelTestResult {
            success: false,
            path: path_str.to_string(),
            size_bytes,
            latency_ms: start.elapsed().as_millis() as u64,
            error: Some(format!(
                "File too small ({} bytes). Expected a Whisper GGML model (>50MB).",
                size_bytes
            )),
        };
    }

    match std::fs::File::open(p) {
        Ok(mut f) => {
            use std::io::Read;
            let mut header = [0u8; 16];
            if let Err(e) = f.read_exact(&mut header) {
                return SttModelTestResult {
                    success: false,
                    path: path_str.to_string(),
                    size_bytes,
                    latency_ms: start.elapsed().as_millis() as u64,
                    error: Some(format!("Could not read header bytes: {}", e)),
                };
            }
            SttModelTestResult {
                success: true,
                path: path_str.to_string(),
                size_bytes,
                latency_ms: start.elapsed().as_millis() as u64,
                error: None,
            }
        }
        Err(e) => SttModelTestResult {
            success: false,
            path: path_str.to_string(),
            size_bytes,
            latency_ms: start.elapsed().as_millis() as u64,
            error: Some(format!("Failed to open file: {}", e)),
        },
    }
}

use crate::settings::LanguageSettings;

/// The `spoken_languages` entry that means "let Whisper decide".
///
/// Not an ISO code, so it is never a candidate to pin — it only ever expresses
/// an intent. Kept out of the language count for the same reason: a profile of
/// `["en", "auto"]` names one language the user speaks, not two.
const AUTO_LANGUAGE: &str = "auto";

/// How much audio one decode gets, which decides whether Whisper's own
/// language detection can be trusted for it.
///
/// This distinction is the whole reason the enum exists. Detection reads the
/// first window of a decode and commits; given a thirty-second meeting chunk it
/// has ample evidence, and given a two-second push-to-talk phrase it is close to
/// a guess. One resolution served to both surfaces means either meetings are
/// hard-locked to the wrong language or short utterances are misclassified — and
/// Relay had both, from the same call.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SttWindow {
    /// Tens of seconds per decode: meeting chunks, imported recordings.
    /// Detection is reliable here, so a multilingual profile can be honoured.
    LongForm,
    /// One phrase or one utterance, often under three seconds: push-to-talk
    /// dictation, a Scribble, a Talkback turn, the live-preview clock.
    /// Detection is not reliable here, so the primary language is pinned.
    ShortForm,
}

/// Resolved speech-to-text language configuration passed to the STT engine.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SttLanguageConfig {
    /// Language code passed to Whisper (`Some("en")`, `Some("hi")`, or `None` for auto-detect).
    pub whisper_language: Option<String>,
    /// Whether translation is enabled (`false` for dictation to preserve verbatim speech).
    pub translate: bool,
}

impl Default for SttLanguageConfig {
    fn default() -> Self {
        Self {
            whisper_language: Some("en".to_string()),
            translate: false,
        }
    }
}

impl SttLanguageConfig {
    /// Resolves the Whisper STT language configuration for one window class.
    ///
    /// The window is a required argument rather than a defaulted one on purpose:
    /// choosing it wrong is the defect this function exists to prevent, and a
    /// default would let a new call site inherit the wrong policy in silence.
    ///
    /// Rules, in order:
    /// 1. `translate` is always `false`, whatever the window — Relay never
    ///    turns Hindi speech into English text.
    /// 2. A primary of "auto" (or empty) means auto-detect for every window.
    ///    There is nothing to pin to, and the user asked explicitly.
    /// 3. [`SttWindow::ShortForm`] pins to the primary language. A phrase does
    ///    not carry enough audio to detect from, so the setting is better
    ///    evidence than the signal.
    /// 4. [`SttWindow::LongForm`] honours a multilingual profile with `None`,
    ///    so code-switched speech is not forced through one acoustic filter.
    ///    `AUTO_LANGUAGE` anywhere in the spoken profile also means `None`.
    /// 5. A single-language profile pins, in either window.
    /// 6. `output_script` ("latin" vs "native") is orthography and never
    ///    affects language selection. It is honoured downstream of the decode.
    pub fn from_settings(settings: &LanguageSettings, window: SttWindow) -> Self {
        let primary = settings.primary_dictation_language.trim().to_lowercase();
        let spoken: Vec<String> = settings
            .spoken_languages
            .iter()
            .map(|s| s.trim().to_lowercase())
            .filter(|s| !s.is_empty())
            .collect();

        // Deduplicate spoken languages while preserving ordering
        let mut unique_spoken = Vec::new();
        for s in spoken {
            if !unique_spoken.contains(&s) {
                unique_spoken.push(s);
            }
        }

        let asked_for_auto = unique_spoken.iter().any(|s| s == AUTO_LANGUAGE);
        // The languages that could actually be pinned, "auto" excluded.
        let real_spoken: Vec<&String> = unique_spoken
            .iter()
            .filter(|s| *s != AUTO_LANGUAGE)
            .collect();

        let whisper_language = if primary == AUTO_LANGUAGE || primary.is_empty() {
            None
        } else if window == SttWindow::ShortForm {
            // Pinned even for a bilingual profile: on one phrase, the user's
            // stated primary language beats a coin toss on the audio.
            Some(primary)
        } else if asked_for_auto || real_spoken.len() > 1 {
            // Bilingual / multilingual profile (e.g. English + Hindi, or
            // Hinglish) over long-form audio. Do not hard-lock Whisper to a
            // single language, and never pass a non-ISO token.
            None
        } else if real_spoken.len() == 1 && *real_spoken[0] == primary {
            // Unambiguous single-language profile.
            Some(primary)
        } else if real_spoken.is_empty() {
            Some(primary)
        } else {
            None
        };

        Self {
            whisper_language,
            translate: false,
        }
    }

    /// As [`from_settings`], with an explicit language override on top.
    ///
    /// The override exists because auto-detection is per *chunk*, not per
    /// meeting. Whisper re-decides the language every thirty seconds, so a
    /// bilingual profile over code-switched speech — the Hinglish case
    /// [`from_settings`] deliberately leaves unpinned — can be detected as
    /// Hindi on one chunk and English on the next. A chunk decoded under the
    /// wrong language does not fail; it comes back as fluent nonsense in the
    /// wrong language's phonology, which is far worse than a weak transcript
    /// because nothing about it looks broken.
    ///
    /// So the setting is not "pin or auto" in the abstract. It is: when the
    /// detector is demonstrably wrong about *this* recording, say what the
    /// recording is and stop asking.
    ///
    /// - `""` (empty) defers entirely to [`from_settings`]. The default, so
    ///   nobody who has not hit this gets a behaviour change.
    /// - [`AUTO_LANGUAGE`] forces detection on, even where the language
    ///   profile would have pinned.
    /// - Anything else pins that language for every chunk.
    ///
    /// [`from_settings`]: Self::from_settings
    pub fn from_settings_with_override(
        settings: &LanguageSettings,
        window: SttWindow,
        override_language: &str,
    ) -> Self {
        let chosen = override_language.trim().to_lowercase();
        if chosen.is_empty() {
            return Self::from_settings(settings, window);
        }
        Self {
            whisper_language: (chosen != AUTO_LANGUAGE).then_some(chosen),
            translate: false,
        }
    }
}

/// Sampling strategies supported by Whisper decoding.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum SttSamplingStrategy {
    Greedy { best_of: i32 },
    BeamSearch { beam_size: i32, patience: f32 },
}

impl SttSamplingStrategy {
    /// The whisper-rs equivalent.
    ///
    /// One conversion, shared by the batch engine and the streaming
    /// transcriber. Two copies is how a preset comes to apply on one path and
    /// not the other.
    #[cfg(feature = "whisper-local")]
    pub fn to_whisper(&self) -> SamplingStrategy {
        match self {
            Self::Greedy { best_of } => SamplingStrategy::Greedy { best_of: *best_of },
            Self::BeamSearch {
                beam_size,
                patience,
            } => SamplingStrategy::BeamSearch {
                beam_size: *beam_size,
                patience: *patience,
            },
        }
    }
}

/// How much decode cost a surface is willing to pay for recall.
///
/// The two knobs move together on purpose. A higher `no_speech_thold` discards
/// segments Whisper is unsure about; a wider beam gives it more hypotheses to
/// choose between, which is what makes it safe to *keep* those segments
/// instead. So accuracy-first means a wider beam **and** a lower threshold,
/// which reads backwards until you see them as one decision.
///
/// This matters most for the speech Relay was losing. In accented or
/// code-switched audio nearly every segment is low-confidence, so a threshold
/// tuned for clean English discards most of the transcript — which is what a
/// five-minute meeting arriving as fifty-six words looks like.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SttPreset {
    /// Greedy, Whisper's stock threshold. Lowest latency; borderline clauses
    /// are dropped. The right trade for push-to-talk, where the user is
    /// waiting.
    Fast,
    /// Beam search at 3, threshold loosened so the beam has candidates to
    /// work with. The middle ground.
    Balanced,
    /// Beam search at 5, threshold low enough that almost nothing is dropped
    /// silently, relying on the wider beam to pick among low-confidence
    /// segments. For audio that is recorded once and read later.
    Quality,
}

impl Default for SttPreset {
    /// [`SttPreset::Fast`] — Relay's shipped behaviour before presets existed,
    /// so introducing them changes nothing until a surface opts in.
    fn default() -> Self {
        Self::Fast
    }
}

impl SttPreset {
    /// Parses a persisted setting, defaulting rather than failing.
    pub fn from_setting(s: &str) -> Self {
        match s.trim().to_lowercase().as_str() {
            "balanced" => Self::Balanced,
            "quality" => Self::Quality,
            _ => Self::Fast,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Fast => "fast",
            Self::Balanced => "balanced",
            Self::Quality => "quality",
        }
    }

    pub fn sampling(self) -> SttSamplingStrategy {
        match self {
            Self::Fast => SttSamplingStrategy::Greedy { best_of: 1 },
            Self::Balanced => SttSamplingStrategy::BeamSearch {
                beam_size: 3,
                patience: -1.0,
            },
            Self::Quality => SttSamplingStrategy::BeamSearch {
                beam_size: 5,
                patience: -1.0,
            },
        }
    }

    pub fn no_speech_thold(self) -> f32 {
        match self {
            Self::Fast => 0.6,
            Self::Balanced => 0.4,
            Self::Quality => 0.3,
        }
    }
}

/// Centralized Whisper decoding configuration for experiments and production.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct WhisperDecodingConfig {
    pub strategy: SttSamplingStrategy,
    pub temperature: f32,
    pub temperature_inc: f32,
    pub initial_prompt: Option<String>,
    pub suppress_blank: bool,
    pub no_speech_thold: f32,
    pub entropy_thold: f32,
    pub logprob_thold: f32,
    pub print_special: bool,
    pub print_timestamps: bool,
    /// Worker threads for this decode. `None` uses every available core.
    /// Meetings pin the durable clock below the core count so the live
    /// clock is never starved of CPU by a 30-second chunk decode.
    pub n_threads: Option<i32>,
    /// Encoder frames to allocate. `None` is whisper's full context (1500,
    /// covering thirty seconds).
    ///
    /// Clamping it buys latency on a short window and costs accuracy on a long
    /// one, so it belongs to the caller rather than to the transcriber. It was
    /// a constant inside [`StreamingTranscriber`], which is how Talkback came
    /// to decode a fifteen-second question with half an encoder context.
    #[serde(default)]
    pub audio_ctx: Option<i32>,
    /// Whether a decode with no explicit [`Self::audio_ctx`] should size the
    /// encoder clamp to the audio it was handed, via
    /// [`audio_ctx_for_seconds`]. An explicit `audio_ctx` always wins.
    pub trim_audio_context: bool,
    /// Force exactly one segment per decode.
    ///
    /// Correct for a sub-two-second window, where it stops whisper discarding
    /// the whole thing on a lone timestamp token. Wrong for a full utterance,
    /// which it truncates to its first segment.
    #[serde(default)]
    pub single_segment: bool,
    /// Discard decoder state between decodes.
    ///
    /// Set for independent live windows, and honoured by
    /// [`StreamingTranscriber`], which owns a private context and state for
    /// one stream.
    ///
    /// **[`SttEngine`] ignores it and always discards.** Its single slot is
    /// shared by dictation, meetings and imports, so whisper's own
    /// `prompt_past` there is not "this recording's previous chunk" — it is
    /// whichever surface decoded last. The clause this field used to carry,
    /// that "a chunked recording wants the opposite", described an intent the
    /// engine never implemented: it built a fresh state per decode, so nothing
    /// was ever carried. Cross-segment context on that path is explicit
    /// instead — an `initial_prompt` assembled from text the quality gate
    /// kept, which can be withdrawn the moment a segment is rejected.
    #[serde(default)]
    pub no_context: bool,
    /// Maximum allowable zlib compression ratio before hallucination screening rejects.
    #[serde(default)]
    pub compression_ratio_thold: Option<f32>,
}

impl Default for WhisperDecodingConfig {
    fn default() -> Self {
        Self::baseline()
    }
}

impl WhisperDecodingConfig {
    /// Baseline configuration representing production behavior:
    /// - Greedy decoding with best_of = 1
    /// - Temperature = 0.0, temperature_inc = 0.2
    /// - No initial prompt
    /// - suppress_blank = true, print_special = false
    pub fn baseline() -> Self {
        Self {
            strategy: SttSamplingStrategy::Greedy { best_of: 1 },
            temperature: 0.0,
            temperature_inc: 0.2,
            initial_prompt: None,
            suppress_blank: true,
            no_speech_thold: 0.6,
            entropy_thold: 2.4,
            logprob_thold: -1.0,
            print_special: false,
            print_timestamps: false,
            n_threads: None,
            audio_ctx: None,
            trim_audio_context: false,
            single_segment: false,
            no_context: false,
            compression_ratio_thold: Some(2.4),
        }
    }

    /// Configuration for Experiment B: Greedy with best_of = 3.
    pub fn experiment_best_of(best_of: i32) -> Self {
        let mut cfg = Self::baseline();
        cfg.strategy = SttSamplingStrategy::Greedy { best_of };
        cfg
    }

    /// Configuration for Experiment C: Temperature fallback with custom initial temperature and increment.
    pub fn experiment_temperature(initial_temp: f32, temp_inc: f32) -> Self {
        let mut cfg = Self::baseline();
        cfg.temperature = initial_temp;
        cfg.temperature_inc = temp_inc;
        cfg
    }

    /// Configuration for Experiment D: Technical vocabulary initial prompt.
    pub fn experiment_prompt(prompt: &str) -> Self {
        let mut cfg = Self::baseline();
        cfg.initial_prompt = Some(prompt.to_string());
        cfg
    }

    /// Configuration for Experiment F: Tuned hallucination/no-speech thresholds.
    pub fn experiment_thresholds(no_speech_thold: f32, entropy_thold: f32, logprob_thold: f32) -> Self {
        let mut cfg = Self::baseline();
        cfg.no_speech_thold = no_speech_thold;
        cfg.entropy_thold = entropy_thold;
        cfg.logprob_thold = logprob_thold;
        cfg
    }

    /// The decode parameters a low-latency streaming window needs.
    ///
    /// These are the values [`StreamingTranscriber`] used to hardcode inside
    /// its own `transcribe()`, which is why Talkback and the live meeting clock
    /// ran without the hallucination thresholds, without the user's
    /// vocabulary, and on half an encoder context — none of it visible from
    /// the call site, and none of it configurable. Naming the configuration is
    /// what makes those choices reviewable.
    ///
    /// `single_segment` is what stops whisper discarding a whole window when
    /// the decode ends on a lone timestamp token, which is the common outcome
    /// for a sub-two-second window. It is wrong for a full utterance, and
    /// callers that decode one should say so.
    pub fn for_live_window() -> Self {
        let mut cfg = Self::baseline();
        cfg.audio_ctx = Some(LIVE_AUDIO_CTX);
        cfg.single_segment = true;
        cfg.no_context = true;
        cfg.temperature_inc = 0.0;
        cfg
    }

    /// The decode parameters for one complete, self-contained utterance.
    ///
    /// Talkback's case, and the one [`for_live_window`] gets wrong. A Talkback
    /// turn is capped at thirty seconds (`TurnDetectorConfig::max_turn_ms`) and
    /// decoded once, when the turn ends — so it can afford whisper's full
    /// encoder context, and it needs it: at `LIVE_AUDIO_CTX` a question longer
    /// than about fifteen seconds lost its tail, and `single_segment` merged
    /// whatever survived into one segment. That is the reported "captures only
    /// part of what I say".
    ///
    /// `no_context` stays set. Each utterance is a new question, so there is
    /// nothing from the previous turn that should condition this decode.
    ///
    /// [`for_live_window`]: Self::for_live_window
    pub fn for_utterance() -> Self {
        let mut cfg = Self::baseline();
        // Full context: 1500 frames covers thirty seconds, which is exactly the
        // longest turn the detector will hand over.
        cfg.audio_ctx = None;
        cfg.single_segment = false;
        cfg.no_context = true;
        cfg
    }

    /// Applies a quality preset, leaving every other field alone.
    ///
    /// Sampling strategy and the no-speech threshold move together, which is
    /// why one setting sets both: a wider beam is what makes it safe to keep
    /// low-confidence segments, and keeping them is what makes the wider beam
    /// worth its cost. Tuning either alone gets the trade backwards.
    pub fn with_preset(mut self, preset: SttPreset) -> Self {
        self.strategy = preset.sampling();
        self.no_speech_thold = preset.no_speech_thold();
        self
    }

    /// Resolves the effective production decoding configuration from persisted user settings.
    /// If domain vocabulary prompt is disabled (default), returns the clean baseline configuration (initial_prompt = None).
    pub fn from_settings(stt_settings: &crate::settings::SttSettings) -> Self {
        Self::from_settings_defaulting(stt_settings, SttPreset::Fast)
    }

    /// As [`from_settings`], but the caller names the preset to use when the
    /// user has not chosen one.
    ///
    /// This is where per-surface policy lives. An explicit user setting still
    /// wins — `default_preset` only fills the gap.
    ///
    /// [`from_settings`]: Self::from_settings
    pub fn from_settings_defaulting(
        stt_settings: &crate::settings::SttSettings,
        default_preset: SttPreset,
    ) -> Self {
        let preset = stt_settings.configured_preset().unwrap_or(default_preset);
        let mut cfg = Self::baseline().with_preset(preset);
        if stt_settings.enable_initial_prompt {
            if let Some(ref prompt) = stt_settings.custom_initial_prompt {
                let trimmed = prompt.trim();
                if !trimmed.is_empty() {
                    cfg.initial_prompt = Some(trimmed.to_string());
                }
            }
        }
        cfg
    }

    /// As [`from_settings_defaulting`], plus the encoder clamp meetings want.
    ///
    /// Separate from the dictation resolver because the trade differs. A
    /// meeting is a stream of segments the segmenter cut at silence, so most
    /// of them are well under whisper's thirty-second window and each one pays
    /// for the difference. Dictation is one short utterance the user is
    /// waiting on, decoded once — and its own latency is dominated by the
    /// model load, not by encoder width.
    ///
    /// [`from_settings_defaulting`]: Self::from_settings_defaulting
    pub fn for_meetings(
        stt_settings: &crate::settings::SttSettings,
        default_preset: SttPreset,
    ) -> Self {
        let mut cfg = Self::from_settings_defaulting(stt_settings, default_preset);
        cfg.trim_audio_context = stt_settings.meeting_trim_audio_context;
        cfg
    }

    /// The same configuration, wound back to what a script whisper writes
    /// expensively can afford.
    ///
    /// Whisper's tokenizer is fitted to Latin text. Devanagari and the other
    /// non-Latin scripts cost several tokens where English costs one, and the
    /// decoder runs once per token — so the same sentence is several times
    /// more decoder work in Hindi than in English. Measured on one machine
    /// with `ggml-small.bin`: 18.3 ms of decode per character of English
    /// against 103.8 ms per character of Hindi, and decode time tracking
    /// transcript length at r = +0.80 in Hindi where English showed no
    /// relationship at all.
    ///
    /// A beam search multiplies precisely that cost, so it is the first thing
    /// to go. [`SttPreset::Fast`] is reused rather than hand-assembled because
    /// dropping the beam and loosening the no-speech threshold belong
    /// together: the wider beam is what made keeping low-confidence segments
    /// safe, so a config that drops one and not the other gets the trade
    /// backwards. `temperature_inc` goes to zero on top, which stops a
    /// segment whisper is unsure of being decoded up to six times over — a
    /// fallback whose signature showed up on Hindi audio and never on English.
    ///
    /// This is a real trade, not a free win: greedy decoding and no fallback
    /// are less accurate, on exactly the audio whose accuracy is already
    /// weakest. It buys keeping up with the meeting, and it is applied only
    /// where the cost it targets is actually being paid.
    pub fn for_expensive_script(&self) -> Self {
        let mut cfg = self.clone().with_preset(SttPreset::Fast);
        cfg.temperature_inc = 0.0;
        cfg
    }

    /// The profile for a batch run: an import, or a re-transcription.
    ///
    /// Everything [`for_meetings`] trades away is bought back here, because
    /// the thing it was bought for does not exist. A live meeting decodes
    /// against a clock — audio keeps arriving whether or not the decoder kept
    /// up, and a decoder slower than real time eventually drops speech. A
    /// batch run reads a file that is already on disk. Nothing arrives, so
    /// nothing can be lost by being slow, and every millisecond spent is spent
    /// on a transcript the user asked to be *better* than the one they have.
    ///
    /// Two differences from the live profile, both in that direction:
    ///
    /// - [`SttPreset::Quality`] rather than [`SttPreset::Balanced`] as the
    ///   default — its own doc says "for audio that is recorded once and read
    ///   later", which is exactly a file on disk. An explicit preset in
    ///   settings still wins.
    /// - The encoder clamp is off. [`trim_audio_context`] buys latency on a
    ///   short segment and costs accuracy on a long one; with no latency worth
    ///   buying, it is a loss with no matching gain.
    ///
    /// Note what is *not* here: a wound-back profile for non-Latin script.
    /// [`for_expensive_script`] exists to keep a live decoder ahead of
    /// arriving audio, and a batch run has nothing to keep ahead of — so
    /// [`crate::meetings::import::BatchConfig`] carries one profile and not
    /// two, and a re-transcription of Hindi audio structurally cannot be
    /// decoded more cheaply than the same audio in English.
    ///
    /// [`for_meetings`]: Self::for_meetings
    /// [`trim_audio_context`]: Self::trim_audio_context
    /// [`for_expensive_script`]: Self::for_expensive_script
    pub fn for_meeting_batch(stt_settings: &crate::settings::SttSettings) -> Self {
        let mut cfg = Self::from_settings_defaulting(stt_settings, SttPreset::Quality);
        cfg.trim_audio_context = false;
        cfg
    }

    /// Resolves the effective decoding configuration specifically for Universal Dictation.
    ///
    /// Defaults to [`SttPreset::Fast`], deliberately: push-to-talk is
    /// latency-bound because the user is waiting for the text to appear, and a
    /// beam search would be felt on every phrase. A meeting makes the opposite
    /// trade. Both read the same setting, and an explicit choice overrides
    /// either default.
    ///
    /// Uses the user-configured thread override if provided, or clamps thread
    /// allocation between 1 and 12 to saturate available physical/logical cores
    /// without exceeding logical core boundaries.
    pub fn for_dictation(stt_settings: &crate::settings::SttSettings) -> Self {
        let mut cfg = Self::from_settings_defaulting(stt_settings, SttPreset::Fast);
        if let Some(threads) = stt_settings.dictation_threads {
            cfg.n_threads = Some(threads.clamp(1, 64));
        } else {
            let threads = std::thread::available_parallelism()
                .map(|n| n.get() as i32)
                .unwrap_or(4)
                .clamp(1, 12);
            cfg.n_threads = Some(threads);
        }
        cfg
    }
}

/// One decoded utterance — a single Whisper segment, with the timing Whisper
/// itself assigned to it.
///
/// Whisper already segments a decode into utterance-sized spans and reports a
/// start and end for each. Before v2.5 the meetings worker concatenated those
/// spans into one string and discarded the timings, which left the whole
/// 30-second chunk as a single undifferentiated block of text — and therefore
/// unattributable to any speaker finer than "someone spoke during these thirty
/// seconds". Keeping the spans is what lets channel provenance resolve at
/// roughly sentence granularity.
#[derive(Debug, Clone, PartialEq)]
pub struct SttUtterance {
    /// Offset from the start of the submitted audio, in seconds.
    pub start_s: f64,
    pub end_s: f64,
    pub text: String,
    /// Whisper's own confidence that this span is not speech. High values mark
    /// the hallucinated filler Whisper emits over silence and music.
    pub no_speech_prob: f32,
}

/// Diagnostic metadata recorded for an STT transcription session.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct SttSessionDiagnostics {
    pub model_path: String,
    pub audio_duration_seconds: f32,
    pub whisper_language: Option<String>,
    pub decoding_strategy: String,
    pub temperature: f32,
    pub temperature_inc: f32,
    pub best_of: i32,
    pub used_initial_prompt: bool,
    pub transcription_latency_ms: u128,
    pub real_time_factor: f32,
    pub segment_count: usize,
    pub is_empty: bool,
    pub transcript_char_count: usize,
    /// How long this call waited for the engine's model lock before it could
    /// begin. Non-zero means another surface — a meeting worker, a dictation —
    /// was mid-decode and this one queued behind it.
    #[serde(default)]
    pub lock_wait_ms: u128,
    /// Time spent getting the model into memory. Zero when it was already
    /// resident, which is the normal case.
    #[serde(default)]
    pub model_load_ms: u128,
    /// Time spent building whisper's decode state — its KV caches and compute
    /// buffers. Zero on every decode that reused the resident one, which is
    /// every decode but the first after a model load.
    ///
    /// Reported rather than folded into [`Self::model_load_ms`] because the
    /// two answer different questions: a non-zero load means the model was not
    /// in memory, and a non-zero state build on a call that did *not* load
    /// means the slot is being rebuilt under something that should be reusing
    /// it. It is also outside [`Self::transcription_latency_ms`], which starts
    /// at `whisper_full` — so before this field existed the cost was paid and
    /// then left out of every real-time factor the pipeline reports.
    #[serde(default)]
    pub state_create_ms: u128,
    /// Whether a *different* model had to be evicted to run this call. True
    /// here is the model-thrash signal: meetings and dictation are fighting
    /// over the engine's single model slot.
    #[serde(default)]
    pub model_reloaded: bool,
    /// The encoder clamp this decode ran with, or `None` for whisper's full
    /// thirty-second window. Reported because it is the difference between a
    /// segment costing what its audio is worth and costing a flat thirty
    /// seconds — and because a decode that loses its tail will name it.
    #[serde(default)]
    pub audio_ctx: Option<i32>,
    /// The language whisper decoded under, as it reports it back — the pinned
    /// one where the caller pinned it, and the auto-detected one otherwise.
    /// `None` for an engine that does not report a language.
    #[serde(default)]
    pub detected_language: Option<String>,
}

/// What the engine's single model slot holds: a path, and the decode state
/// that was built for the model at it.
///
/// The state is *kept* rather than built per decode, which is the whole point
/// of this type existing. `whisper_init_state` allocates the KV caches and the
/// compute buffers — roughly 330 MB for `ggml-small`, as
/// [`StreamingTranscriber`] has said in its own doc comment since it was
/// written — so on a short segment that allocation costs more than the
/// inference it is for. A meeting pays it once per segment, several hundred
/// times an hour, and none of it appeared in `decode_ms`: the worker's timer
/// starts at `whisper_full`, so the cost was not only paid but invisible to
/// every number the pipeline reports.
///
/// The context itself is not held alongside: `WhisperState` owns an `Arc` to
/// it, so the state keeps the model alive on its own. Replacing the slot drops
/// both together, which is what makes a model switch a switch of both.
///
/// What it costs: that allocation is now resident between decodes rather than
/// churned, so Vox holds it for as long as it holds the model. It is the same
/// peak the process already reached during every decode, and the model's own
/// weights were already resident for the life of the process — there is no
/// whisper unload path. Said plainly here because it is the reason someone
/// might later want one.
#[cfg(feature = "whisper-local")]
struct LoadedWhisper {
    path: String,
    state: whisper_rs::WhisperState,
}

/// Local, zero-cost speech-to-text via whisper.cpp (through whisper-rs) or NVIDIA Parakeet TDT.
#[derive(Clone)]
pub struct SttEngine {
    #[cfg(feature = "whisper-local")]
    loaded: Arc<Mutex<Option<LoadedWhisper>>>,
    #[cfg(feature = "parakeet")]
    parakeet_loaded: Arc<Mutex<Option<crate::capture::parakeet::ParakeetEngine>>>,
}

impl Default for SttEngine {
    fn default() -> Self {
        Self::new()
    }
}

impl SttEngine {
    pub fn new() -> Self {
        Self {
            #[cfg(feature = "whisper-local")]
            loaded: Arc::new(Mutex::new(None)),
            #[cfg(feature = "parakeet")]
            parakeet_loaded: Arc::new(Mutex::new(None)),
        }
    }

    /// Unloads any cached Parakeet engine session.
    pub fn unload_parakeet(&self) {
        #[cfg(feature = "parakeet")]
        {
            *self.parakeet_loaded.lock_or_recover() = None;
        }
    }

    /// Transcribe using the NVIDIA Parakeet TDT engine with sub-100ms latency.
    pub fn transcribe_parakeet(
        &self,
        parakeet_dir: &Path,
        samples_16k_mono: &[f32],
    ) -> Result<(String, SttSessionDiagnostics), SttError> {
        #[cfg(not(feature = "parakeet"))]
        {
            let _ = (parakeet_dir, samples_16k_mono);
            Err(SttError::TranscriptionFailed(
                "Parakeet engine is not enabled in this build".to_string(),
            ))
        }

        #[cfg(feature = "parakeet")]
        {
            let t_start = std::time::Instant::now();
            let files = crate::capture::parakeet::ModelFiles::find_in(parakeet_dir).ok_or_else(|| {
                SttError::TranscriptionFailed(
                    "Parakeet model files are incomplete or missing from models directory".to_string(),
                )
            })?;

            let mut guard = self.parakeet_loaded.lock_or_recover();
            if guard.is_none() {
                let engine = crate::capture::parakeet::ParakeetEngine::load(&files).map_err(|e| {
                    SttError::TranscriptionFailed(format!("Failed to load Parakeet engine: {e}"))
                })?;
                *guard = Some(engine);
            }

            let engine = guard.as_mut().expect("parakeet engine loaded");
            let result = engine.transcribe(samples_16k_mono).map_err(|e| {
                SttError::TranscriptionFailed(format!("Parakeet transcription failed: {e}"))
            })?;

            let elapsed = t_start.elapsed().as_millis();
            let duration_s = samples_16k_mono.len() as f32 / 16_000.0;
            let rtf = if duration_s > 0.0 {
                (elapsed as f32 / 1000.0) / duration_s
            } else {
                0.0
            };

            let diag = SttSessionDiagnostics {
                model_path: "parakeet-tdt-0.6b-v3".to_string(),
                audio_duration_seconds: duration_s,
                whisper_language: Some("en".to_string()),
                decoding_strategy: "tdt_greedy".to_string(),
                temperature: 0.0,
                temperature_inc: 0.0,
                best_of: 1,
                used_initial_prompt: false,
                transcription_latency_ms: elapsed,
                real_time_factor: rtf,
                segment_count: result.tokens.len(),
                is_empty: result.text.trim().is_empty(),
                transcript_char_count: result.text.chars().count(),
                lock_wait_ms: 0,
                model_load_ms: 0,
                state_create_ms: 0,
                model_reloaded: false,
                audio_ctx: None,
                // Parakeet is English-only and says so through its
                // capabilities; echoing that here would dress a fixed
                // property up as a detection.
                detected_language: None,
            };

            Ok((result.text, diag))
        }
    }

    /// Transcribe using default baseline decoding configuration.
    pub fn transcribe(
        &self,
        model_path: Option<&str>,
        samples_16k_mono: &[f32],
        language_config: &SttLanguageConfig,
    ) -> Result<String, SttError> {
        let (text, _) = self.transcribe_with_config(
            model_path,
            samples_16k_mono,
            language_config,
            &WhisperDecodingConfig::default(),
        )?;
        Ok(text)
    }

    /// Transcribe with explicit Whisper decoding configuration and return diagnostic metrics.
    ///
    /// Text is joined from the same utterances
    /// [`SttEngine::transcribe_utterances_with_config`] returns, so the two can
    /// never disagree about what was said.
    pub fn transcribe_with_config(
        &self,
        model_path: Option<&str>,
        samples_16k_mono: &[f32],
        language_config: &SttLanguageConfig,
        decoding_config: &WhisperDecodingConfig,
    ) -> Result<(String, SttSessionDiagnostics), SttError> {
        let (utterances, diag) = self.transcribe_utterances_with_config(
            model_path,
            samples_16k_mono,
            language_config,
            decoding_config,
        )?;
        Ok((join_utterance_text(&utterances), diag))
    }

    /// Transcribe and return Whisper's own utterance spans, each with its timing.
    ///
    /// The one decode path. Callers that only want text go through
    /// [`SttEngine::transcribe_with_config`], which joins these.
    pub fn transcribe_utterances_with_config(
        &self,
        model_path: Option<&str>,
        samples_16k_mono: &[f32],
        language_config: &SttLanguageConfig,
        decoding_config: &WhisperDecodingConfig,
    ) -> Result<(Vec<SttUtterance>, SttSessionDiagnostics), SttError> {
        #[cfg(not(feature = "whisper-local"))]
        {
            let _ = (model_path, samples_16k_mono, language_config, decoding_config);
            Err(SttError::ModelNotConfigured)
        }

        #[cfg(feature = "whisper-local")]
        {
            let model_path = model_path.ok_or(SttError::ModelNotConfigured)?;
            if model_path.trim().is_empty() {
                return Err(SttError::ModelNotConfigured);
            }
            let audio_dur = samples_16k_mono.len() as f32 / 16000.0;
            if samples_16k_mono.is_empty() {
                let diag = SttSessionDiagnostics {
                    model_path: model_path.to_string(),
                    audio_duration_seconds: 0.0,
                    whisper_language: language_config.whisper_language.clone(),
                    decoding_strategy: format!("{:?}", decoding_config.strategy),
                    temperature: decoding_config.temperature,
                    temperature_inc: decoding_config.temperature_inc,
                    best_of: match decoding_config.strategy {
                        SttSamplingStrategy::Greedy { best_of } => best_of,
                        _ => 1,
                    },
                    used_initial_prompt: decoding_config.initial_prompt.is_some(),
                    transcription_latency_ms: 0,
                    real_time_factor: 0.0,
                    segment_count: 0,
                    is_empty: true,
                    transcript_char_count: 0,
                    lock_wait_ms: 0,
                    model_load_ms: 0,
                    state_create_ms: 0,
                    model_reloaded: false,
                    audio_ctx: None,
                    detected_language: None,
                };
                return Ok((Vec::new(), diag));
            }

            // TEMP: whisper internal latency diagnostics
            let t_whisper_start = std::time::Instant::now();

            // Timed separately from the decode because they fail for
            // different reasons and are fixed in different places: a long
            // `lock_wait_ms` means contention with another surface, a non-zero
            // `model_load_ms` means the single model slot was evicted. Rolling
            // either into the decode number hides both.
            let t_lock_start = std::time::Instant::now();
            let mut guard = self.loaded.lock_or_recover();
            let lock_wait_ms = t_lock_start.elapsed().as_millis();

            let needs_reload = match guard.as_ref() {
                Some(loaded) => loaded.path != model_path,
                None => true,
            };
            // A first load is not thrash; evicting a *different* model is.
            let evicted_other_model =
                matches!(guard.as_ref(), Some(loaded) if loaded.path != model_path);

            let mut model_load_ms = 0u128;
            let mut state_create_ms = 0u128;
            if needs_reload {
                tracing::info!("Loading Whisper model from {}", model_path);
                let t_load_start = std::time::Instant::now();
                let ctx =
                    WhisperContext::new_with_params(model_path, WhisperContextParameters::default())
                        .map_err(|e| SttError::ModelLoadFailed {
                            path: model_path.to_string(),
                            message: e.to_string(),
                        })?;
                model_load_ms = t_load_start.elapsed().as_millis();

                // Built once, with the model, and kept for every decode after
                // it — see [`LoadedWhisper`]. Non-zero here and zero on every
                // subsequent call is what a reader should see; a run where it
                // is non-zero per segment is the model slot thrashing.
                let t_state_start = std::time::Instant::now();
                let state = ctx
                    .create_state()
                    .map_err(|e| SttError::TranscriptionFailed(e.to_string()))?;
                state_create_ms = t_state_start.elapsed().as_millis();

                *guard = Some(LoadedWhisper {
                    path: model_path.to_string(),
                    state,
                });
            }

            let state = &mut guard
                .as_mut()
                .expect("model was just loaded or already present")
                .state;

            let strategy = decoding_config.strategy.to_whisper();

            let mut params = FullParams::new(strategy);
            params.set_language(language_config.whisper_language.as_deref());
            params.set_translate(language_config.translate);
            params.set_print_special(decoding_config.print_special);
            params.set_print_progress(false);
            params.set_print_realtime(false);
            params.set_print_timestamps(decoding_config.print_timestamps);
            params.set_suppress_blank(decoding_config.suppress_blank);
            params.set_temperature(decoding_config.temperature);
            params.set_temperature_inc(decoding_config.temperature_inc);
            params.set_no_speech_thold(decoding_config.no_speech_thold);
            params.set_entropy_thold(decoding_config.entropy_thold);
            params.set_logprob_thold(decoding_config.logprob_thold);
            if let Some(ref prompt) = decoding_config.initial_prompt {
                params.set_initial_prompt(prompt);
            }
            params.set_n_threads(decoding_config.n_threads.unwrap_or_else(num_cpus));
            params.set_single_segment(decoding_config.single_segment);
            // Always, whatever the caller asked for — and this is the one
            // place [`WhisperDecodingConfig::no_context`] is not honoured.
            // The slot's state is shared by every surface, so whisper's own
            // `prompt_past` would carry one caller's decode into the next
            // one's: a dictated phrase priming a meeting segment, or the
            // hallucination a segment was just rejected for priming its
            // successor. It also carried nothing before, because each decode
            // built a fresh state, so this is the behaviour that was already
            // in force rather than a new restriction. Cross-segment context
            // that is *wanted* travels as an explicit `initial_prompt`, built
            // by [`crate::capture::vocabulary`] out of text the quality gate
            // kept — which is context Vox can name, screen and withdraw.
            params.set_no_context(true);

            // Whisper decodes a thirty-second window whatever it is given, so a
            // shorter segment pays for silence it does not contain. An explicit
            // clamp wins; otherwise it is sized to this segment's own audio.
            let effective_audio_ctx = decoding_config.audio_ctx.or_else(|| {
                if decoding_config.trim_audio_context {
                    audio_ctx_for_seconds(audio_dur)
                } else {
                    None
                }
            });
            if let Some(audio_ctx) = effective_audio_ctx {
                params.set_audio_ctx(audio_ctx);
            }

            // TEMP: whisper internal latency diagnostics (state_full)
            let t_state_full_start = std::time::Instant::now();
            state
                .full(params, samples_16k_mono)
                .map_err(|e| SttError::TranscriptionFailed(e.to_string()))?;
            let t_state_full_end = std::time::Instant::now();

            let elapsed_ms = t_state_full_end.duration_since(t_state_full_start).as_millis();

            // What whisper decided it was listening to. Set whether or not the
            // caller pinned a language: on a pinned decode it echoes the pin,
            // and on an auto-detecting one it is the only record of a choice
            // that changes every word of the output. A chunk decoded under the
            // wrong language does not fail — it returns fluent text in the
            // wrong one — so without this a transcript cannot be asked why.
            let detected_language =
                whisper_rs::get_lang_str(state.full_lang_id_from_state()).map(str::to_string);

            let mut utterances: Vec<SttUtterance> = Vec::new();
            let mut segment_count = 0;
            for segment in state.as_iter() {
                segment_count += 1;
                let segment_text = segment
                    .to_str_lossy()
                    .map_err(|e| SttError::TranscriptionFailed(e.to_string()))?;
                let text = segment_text.trim();
                if text.is_empty() {
                    continue;
                }
                // whisper.cpp reports segment bounds in centiseconds.
                utterances.push(SttUtterance {
                    start_s: segment.start_timestamp() as f64 / 100.0,
                    end_s: segment.end_timestamp() as f64 / 100.0,
                    text: text.to_string(),
                    no_speech_prob: segment.no_speech_probability(),
                });
            }

            let trimmed_text = join_utterance_text(&utterances);
            let rtf = if audio_dur > 0.0 {
                (elapsed_ms as f32 / 1000.0) / audio_dur
            } else {
                0.0
            };

            let diag = SttSessionDiagnostics {
                model_path: model_path.to_string(),
                audio_duration_seconds: audio_dur,
                whisper_language: language_config.whisper_language.clone(),
                decoding_strategy: format!("{:?}", decoding_config.strategy),
                temperature: decoding_config.temperature,
                temperature_inc: decoding_config.temperature_inc,
                best_of: match decoding_config.strategy {
                    SttSamplingStrategy::Greedy { best_of } => best_of,
                    _ => 1,
                },
                used_initial_prompt: decoding_config.initial_prompt.is_some(),
                transcription_latency_ms: elapsed_ms,
                real_time_factor: rtf,
                segment_count,
                is_empty: trimmed_text.is_empty(),
                transcript_char_count: trimmed_text.chars().count(),
                lock_wait_ms,
                model_load_ms,
                state_create_ms,
                model_reloaded: evicted_other_model,
                audio_ctx: effective_audio_ctx,
                detected_language,
            };

            // TEMP: whisper internal latency diagnostics (timing summary)
            let t_whisper_end = std::time::Instant::now();
            let state_full_ms = t_state_full_end.duration_since(t_state_full_start).as_millis();
            let whisper_total_ms = t_whisper_end.duration_since(t_whisper_start).as_millis();
            let other_ms = whisper_total_ms.saturating_sub(state_create_ms + state_full_ms);

            // At `debug`, not on stdout. This is one record per decode, which
            // is fine for a dictation phrase and is 500 of them for an hour of
            // meeting — as eight lines of unconditional `println!` each, it
            // buried every other log line the moment meetings existed.
            // `rules/rust-backend.md` asks for `tracing` for exactly this
            // reason.
            tracing::debug!(
                state_create_ms,
                state_full_ms,
                other_ms,
                whisper_total_ms,
                "whisper internal latency"
            );

            tracing::debug!(
                "Whisper STT finished: audio={:.2}s, latency={}ms, RTF={:.2}, lang={:?}, segments={}, chars={}",
                diag.audio_duration_seconds,
                diag.transcription_latency_ms,
                diag.real_time_factor,
                diag.whisper_language,
                diag.segment_count,
                diag.transcript_char_count
            );

            Ok((utterances, diag))
        }
    }
}

/// Joins utterance text the way a single Whisper decode would have produced it.
pub fn join_utterance_text(utterances: &[SttUtterance]) -> String {
    let mut out = String::new();
    for utterance in utterances {
        if !out.is_empty() {
            out.push(' ');
        }
        out.push_str(&utterance.text);
    }
    out.trim().to_string()
}

/// Encoder context used for live streaming windows.
///
/// Whisper pads its mel spectrogram to 30 s regardless of input length, so a
/// 2-second window costs a full 30-second encoder pass at the default context
/// of 1500 positions. Clamping the encoder to 768 positions (~15 s of audio)
/// roughly halves that cost — the same trick whisper.cpp's own `stream`
/// example uses — and every window a live stream submits is far shorter than
/// the clamped span.
/// Letters outside this range are treated as a script whisper writes
/// expensively. Basic Latin through Latin Extended-B, so the accented letters
/// of European languages count as Latin — those are cheap, and flagging French
/// or Turkish here would spend accuracy for nothing.
const LAST_LATIN_CODEPOINT: u32 = 0x024F;

/// Share of a segment's letters that must be non-Latin before its script is
/// judged expensive. Well clear of a stray character or a single borrowed
/// word, well under the mix in genuinely code-switched speech.
const EXPENSIVE_SCRIPT_PERCENT: usize = 20;

/// Too few letters to judge from. A three-word segment that happens to be one
/// Hindi word should not decide how the rest of a meeting is decoded.
const EXPENSIVE_SCRIPT_MIN_LETTERS: usize = 8;

/// Whether `text` is written in a script that costs whisper's decoder more
/// than Latin does.
///
/// Measured rather than assumed: on one machine the same model spent 18.3 ms
/// of decode per character of English and 103.8 ms per character of Hindi.
/// The tokenizer is the reason — non-Latin scripts take several tokens where
/// Latin takes one, and the decoder runs once per token.
///
/// Judged from the transcript rather than from the user's configured
/// languages, because a bilingual profile says what someone *might* speak,
/// not what this meeting actually is. Someone set up for English and Hindi
/// who holds an all-English meeting should keep the careful decoder, and they
/// do.
pub fn uses_expensive_script(text: &str) -> bool {
    let (latin, other) = text
        .chars()
        .filter(|c| c.is_alphabetic())
        .fold((0usize, 0usize), |(latin, other), c| {
            if (c as u32) <= LAST_LATIN_CODEPOINT {
                (latin + 1, other)
            } else {
                (latin, other + 1)
            }
        });
    let letters = latin + other;
    if letters < EXPENSIVE_SCRIPT_MIN_LETTERS {
        return false;
    }
    other * 100 / letters >= EXPENSIVE_SCRIPT_PERCENT
}

/// Consecutive segments in an expensive script before the decoder gives up the
/// beam search.
///
/// Two rather than one so a single code-switched sentence in an otherwise
/// English meeting does not decide the rest of it.
const SEGMENTS_BEFORE_GOING_FAST: u32 = 2;

/// Consecutive Latin segments before the decoder takes the beam search back.
///
/// Three rather than two on purpose. Returning to the careful profile is the
/// accuracy win but also the throughput risk, so it asks for more evidence
/// that the expensive stretch is actually over than it asked to leave.
const SEGMENTS_BEFORE_GOING_CAREFUL: u32 = 3;

/// Leaving the careful profile must never ask for more evidence than
/// returning to it: the asymmetry is the whole design, and reversing it by
/// editing one constant would be silent.
const _: () = assert!(SEGMENTS_BEFORE_GOING_CAREFUL > SEGMENTS_BEFORE_GOING_FAST);

/// Follows which script a meeting is being held in, so the decoder can change
/// profile when the language genuinely changes without flapping on one
/// sentence.
///
/// A run in both directions is what separates the two cases that look alike
/// segment by segment. Code-switched Hinglish crosses language inside single
/// sentences, and a decoder that changed settings with it would make the
/// transcript's quality depend on which language a segment happened to start
/// in; a meeting that runs in English, turns to Hindi for a few minutes and
/// goes back is a genuine change and should be followed. Requiring several
/// consecutive segments before moving handles the first by ignoring it and
/// the second by noticing it.
#[derive(Debug, Clone)]
pub struct ScriptTracker {
    expensive: bool,
    /// Consecutive segments disagreeing with the current profile.
    run: u32,
}

impl Default for ScriptTracker {
    fn default() -> Self {
        Self::new()
    }
}

impl ScriptTracker {
    /// Starts careful. An English meeting never leaves this state, and a
    /// meeting in another script pays the careful profile for its opening
    /// segments — the price of reading the meeting rather than a settings
    /// page.
    pub fn new() -> Self {
        Self {
            expensive: false,
            run: 0,
        }
    }

    /// Whether the next segment should use the cheaper profile.
    pub fn is_expensive(&self) -> bool {
        self.expensive
    }

    /// Records what the decoder just wrote, returning `true` if that changed
    /// which profile the next segment gets.
    pub fn observe(&mut self, text: &str) -> bool {
        let seen = uses_expensive_script(text);
        if seen == self.expensive {
            // Agreement resets the run: the segments have to be consecutive,
            // or an alternating meeting would creep across the threshold.
            self.run = 0;
            return false;
        }
        self.run += 1;
        let needed = if self.expensive {
            SEGMENTS_BEFORE_GOING_CAREFUL
        } else {
            SEGMENTS_BEFORE_GOING_FAST
        };
        if self.run < needed {
            return false;
        }
        self.expensive = seen;
        self.run = 0;
        true
    }
}

/// Encoder positions whisper produces for one full thirty-second window.
///
/// whisper.cpp mels a window into 3000 frames and the encoder halves that, so
/// 1500 positions cover 30 s — fifty positions per second of audio. Passing
/// this, or anything larger, is the same as not clamping at all.
pub const FULL_AUDIO_CTX: i32 = 1500;

/// Encoder positions per second of audio. See [`FULL_AUDIO_CTX`].
const AUDIO_CTX_PER_SECOND: f32 = FULL_AUDIO_CTX as f32 / 30.0;

/// Slack added on top of the audio's own span, in seconds.
///
/// The minimum safe clamp is exactly the audio's length: everything past it is
/// simply not encoded. The margin exists because "exactly" is where the last
/// incident came from — a *fixed* [`LIVE_AUDIO_CTX`] cut the tail off anything
/// longer than about fifteen seconds, and the report read as "captures only
/// part of what I say". Two seconds covers the convolution front-end's reach
/// past its input and leaves whisper some silence to end a segment against,
/// at a cost of a hundred positions.
const AUDIO_CTX_HEADROOM_SECONDS: f32 = 2.0;

/// The encoder clamp for a segment holding `audio_seconds` of speech, or
/// `None` when it is long enough to want the whole window.
///
/// Whisper works in thirty-second windows whatever it is given, so a segment
/// shorter than that pays for silence it does not have. Measured on a real
/// meeting, decode cost 28.6 s fixed plus 0.09 s per second of speech — 94% of
/// it independent of how much was said. Clamping the encoder to the audio's
/// own span is what turns that fixed cost back into a proportional one.
///
/// Sized per segment rather than fixed, which is the difference between this
/// and the clamp that truncated audio before: a value derived from the samples
/// in hand cannot be smaller than the samples in hand.
pub fn audio_ctx_for_seconds(audio_seconds: f32) -> Option<i32> {
    if !audio_seconds.is_finite() || audio_seconds <= 0.0 {
        return None;
    }
    let needed = ((audio_seconds + AUDIO_CTX_HEADROOM_SECONDS) * AUDIO_CTX_PER_SECOND).ceil();
    if needed >= FULL_AUDIO_CTX as f32 {
        // Long enough that clamping would save nothing and risk the tail.
        return None;
    }
    Some(needed as i32)
}

pub const LIVE_AUDIO_CTX: i32 = 768;

/// A dedicated Whisper context for one low-latency stream.
///
/// Two properties matter and neither is available through [`SttEngine`]:
///
/// 1. **Its own context.** `SttEngine` holds one model behind a mutex that is
///    locked for the whole of `whisper_full`, so a live clock sharing it would
///    serialize behind every 30-second chunk decode (and behind dictation).
/// 2. **A reused state.** `create_state` allocates whisper's KV cache and
///    compute buffers — roughly 330 MB for `ggml-small` — so allocating one
///    per window is far more expensive than the inference itself. This holds a
///    single state for the stream's lifetime.
///
/// Decoding is configured for short windows: one segment, no cross-window
/// prompt carry-over, greedy at temperature 0, and a clamped encoder context.
/// `single_segment` in particular stops whisper from discarding a whole window
/// when the decode ends on a lone timestamp token ("single timestamp ending -
/// skip entire chunk"), which is the common outcome for sub-2-second windows.
pub struct StreamingTranscriber {
    #[cfg(feature = "whisper-local")]
    state: whisper_rs::WhisperState,
    language: Option<String>,
    translate: bool,
    n_threads: i32,
    /// Every other decode parameter, so this transcriber decides none of them
    /// itself. It used to decide all of them, in a function no call site could
    /// see into.
    decoding: WhisperDecodingConfig,
}

impl StreamingTranscriber {
    /// Loads `model_path` into a private context and pre-allocates its state.
    ///
    /// `decoding` is a required argument. Before it was one, this transcriber
    /// hardcoded its own parameters — which meant Talkback and the live meeting
    /// clock silently ran without the hallucination thresholds and without the
    /// user's vocabulary, while the batch path had both. Callers that want the
    /// old low-latency behaviour ask for it by name:
    /// [`WhisperDecodingConfig::for_live_window`].
    #[cfg(feature = "whisper-local")]
    pub fn new(
        model_path: &str,
        language_config: &SttLanguageConfig,
        decoding: &WhisperDecodingConfig,
        n_threads: i32,
    ) -> Result<Self, SttError> {
        if model_path.trim().is_empty() {
            return Err(SttError::ModelNotConfigured);
        }

        let ctx = WhisperContext::new_with_params(model_path, WhisperContextParameters::default())
            .map_err(|e| SttError::ModelLoadFailed {
                path: model_path.to_string(),
                message: e.to_string(),
            })?;
        let state = ctx
            .create_state()
            .map_err(|e| SttError::TranscriptionFailed(e.to_string()))?;

        Ok(Self {
            state,
            language: language_config.whisper_language.clone(),
            translate: language_config.translate,
            n_threads: n_threads.max(1),
            decoding: decoding.clone(),
        })
    }

    #[cfg(not(feature = "whisper-local"))]
    pub fn new(
        _model_path: &str,
        _language_config: &SttLanguageConfig,
        _decoding: &WhisperDecodingConfig,
        _n_threads: i32,
    ) -> Result<Self, SttError> {
        Err(SttError::ModelNotConfigured)
    }

    /// The decode parameters in force, for diagnostics and for the observability
    /// record. A caller that cannot read them back cannot report what it ran.
    pub fn decoding_config(&self) -> &WhisperDecodingConfig {
        &self.decoding
    }

    /// Transcribes one window with the configured parameters.
    #[cfg(feature = "whisper-local")]
    pub fn transcribe(&mut self, samples_16k_mono: &[f32]) -> Result<String, SttError> {
        if samples_16k_mono.is_empty() {
            return Ok(String::new());
        }

        let mut params = FullParams::new(self.decoding.strategy.to_whisper());
        params.set_language(self.language.as_deref());
        params.set_translate(self.translate);
        params.set_n_threads(self.n_threads);
        if let Some(audio_ctx) = self.decoding.audio_ctx {
            params.set_audio_ctx(audio_ctx);
        }
        params.set_single_segment(self.decoding.single_segment);
        params.set_no_context(self.decoding.no_context);
        params.set_temperature(self.decoding.temperature);
        params.set_temperature_inc(self.decoding.temperature_inc);
        params.set_suppress_blank(self.decoding.suppress_blank);
        // The three thresholds the hardcoded path never set, so whisper's own
        // defaults applied and nothing downstream screened the result.
        params.set_no_speech_thold(self.decoding.no_speech_thold);
        params.set_entropy_thold(self.decoding.entropy_thold);
        params.set_logprob_thold(self.decoding.logprob_thold);
        if let Some(ref prompt) = self.decoding.initial_prompt {
            params.set_initial_prompt(prompt);
        }
        params.set_token_timestamps(false);
        params.set_print_special(self.decoding.print_special);
        params.set_print_progress(false);
        params.set_print_realtime(false);
        params.set_print_timestamps(self.decoding.print_timestamps);

        self.state
            .full(params, samples_16k_mono)
            .map_err(|e| SttError::TranscriptionFailed(e.to_string()))?;

        let mut text = String::new();
        for segment in self.state.as_iter() {
            let segment_text = segment
                .to_str_lossy()
                .map_err(|e| SttError::TranscriptionFailed(e.to_string()))?;
            text.push_str(&segment_text);
        }

        Ok(text.trim().to_string())
    }

    #[cfg(not(feature = "whisper-local"))]
    pub fn transcribe(&mut self, _samples_16k_mono: &[f32]) -> Result<String, SttError> {
        Err(SttError::ModelNotConfigured)
    }
}

#[cfg(feature = "whisper-local")]
fn num_cpus() -> std::ffi::c_int {
    std::thread::available_parallelism()
        .map(|n| n.get() as std::ffi::c_int)
        .unwrap_or(4)
        .clamp(1, 8)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn models_dir_with(files: &[&str]) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("vox_test_models_{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).expect("temp models dir");
        for file in files {
            std::fs::write(dir.join(file), b"model").expect("fake model");
        }
        dir
    }

    #[test]
    fn installed_dictation_model_prefers_the_fast_model() {
        let dir = models_dir_with(&[FAST_MODEL_FILENAME, DEFAULT_MODEL_FILENAME]);
        let stt = crate::settings::SttSettings::default();
        let path = installed_dictation_model_path(&dir, &stt).expect("installed");
        assert!(path.ends_with(FAST_MODEL_FILENAME), "{path}");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn installed_dictation_model_falls_back_to_small_instead_of_downloading() {
        let dir = models_dir_with(&[DEFAULT_MODEL_FILENAME]);
        let stt = crate::settings::SttSettings::default();
        let path = installed_dictation_model_path(&dir, &stt).expect("installed");
        assert!(path.ends_with(DEFAULT_MODEL_FILENAME), "{path}");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn installed_dictation_model_is_none_when_a_download_is_needed() {
        let dir = models_dir_with(&[]);
        let stt = crate::settings::SttSettings::default();
        assert_eq!(installed_dictation_model_path(&dir, &stt), None);

        let accurate = crate::settings::SttSettings {
            dictation_quality: crate::settings::DictationSttQuality::Accurate,
            ..Default::default()
        };
        let dir_fast_only = models_dir_with(&[FAST_MODEL_FILENAME]);
        assert_eq!(installed_dictation_model_path(&dir_fast_only, &accurate), None);
        let _ = std::fs::remove_dir_all(dir);
        let _ = std::fs::remove_dir_all(dir_fast_only);
    }

    #[test]
    fn installed_dictation_model_honours_a_configured_filename() {
        let dir = models_dir_with(&["ggml-medium.bin", FAST_MODEL_FILENAME]);
        let stt = crate::settings::SttSettings {
            whisper_model_path: Some("C:/elsewhere/ggml-medium.bin".into()),
            ..Default::default()
        };
        let path = installed_dictation_model_path(&dir, &stt).expect("installed");
        assert!(path.ends_with("ggml-medium.bin"), "{path}");
        let _ = std::fs::remove_dir_all(dir);
    }

    const HI: &str = "मुझे लगता है कि यह ठीक है और हमें आगे बढ़ना चाहिए";
    const EN: &str = "I think that is fine and we should move ahead with it";

    /// Feeds a sequence of segments and returns which profile each one after
    /// them would get, so a scenario reads as what the meeting sounded like.
    fn profiles_for(segments: &[&str]) -> Vec<bool> {
        let mut tracker = ScriptTracker::new();
        segments
            .iter()
            .map(|text| {
                tracker.observe(text);
                tracker.is_expensive()
            })
            .collect()
    }

    /// An English meeting never leaves the careful profile, so it never pays
    /// accuracy for speed it does not need.
    #[test]
    fn an_english_meeting_stays_careful_throughout() {
        assert_eq!(profiles_for(&[EN; 8]), vec![false; 8]);
    }

    /// A meeting held in Hindi moves after two segments and stays there.
    #[test]
    fn a_hindi_meeting_moves_to_the_fast_profile_and_stays() {
        assert_eq!(
            profiles_for(&[HI, HI, HI, HI, HI]),
            vec![false, true, true, true, true],
            "two segments of evidence, then fast for the rest"
        );
    }

    /// The reported case: English, a Hindi stretch, then English again. The
    /// tail must get its beam search back rather than being stuck on the
    /// cheaper profile for the rest of the meeting.
    #[test]
    fn a_hindi_stretch_inside_an_english_meeting_is_left_behind_afterwards() {
        let profiles = profiles_for(&[EN, EN, HI, HI, HI, HI, EN, EN, EN, EN, EN]);
        assert!(!profiles[1], "still careful before the switch");
        assert!(profiles[3], "fast once the Hindi stretch is established");
        assert!(profiles[5], "and stays fast while it lasts");
        assert!(
            !profiles[9],
            "back to careful once English has settled again: {profiles:?}"
        );
    }

    /// And the other direction, which has the same shape.
    #[test]
    fn an_english_stretch_inside_a_hindi_meeting_returns_to_hindi() {
        let profiles = profiles_for(&[HI, HI, HI, EN, EN, EN, EN, HI, HI, HI, HI]);
        assert!(profiles[2], "fast during the opening Hindi");
        assert!(!profiles[6], "careful once English has settled");
        assert!(profiles[9], "fast again when Hindi comes back");
    }

    /// The case the run length exists for: sentence-level code-switching must
    /// not make the profile flap, because then transcript quality would depend
    /// on which language a segment happened to start in.
    #[test]
    fn alternating_code_switched_segments_do_not_flap() {
        // Strictly alternating never accumulates a run, so nothing moves.
        assert_eq!(
            profiles_for(&[EN, HI, EN, HI, EN, HI, EN, HI]),
            vec![false; 8],
            "a run has to be consecutive"
        );
    }

    /// A single Hindi segment in an English meeting is not enough on its own.
    #[test]
    fn one_isolated_segment_never_moves_the_profile() {
        assert_eq!(
            profiles_for(&[EN, EN, HI, EN, EN, EN]),
            vec![false; 6],
            "one segment is not a change of language"
        );
    }

    /// Leaving the careful profile asks for less evidence than returning to
    /// it, because falling behind costs more than a slightly cheaper decode.
    #[test]
    fn returning_to_careful_asks_for_more_evidence_than_leaving_it() {
        // Two Hindi segments are enough to leave.
        let mut tracker = ScriptTracker::new();
        tracker.observe(HI);
        assert!(!tracker.is_expensive());
        tracker.observe(HI);
        assert!(tracker.is_expensive());

        // Two English segments are not enough to come back.
        tracker.observe(EN);
        tracker.observe(EN);
        assert!(tracker.is_expensive(), "two is not yet three");
        tracker.observe(EN);
        assert!(!tracker.is_expensive());
    }

    /// `observe` reports the change so a caller can record where a meeting
    /// turned, and reports it exactly once per change.
    #[test]
    fn a_switch_is_reported_once() {
        let mut tracker = ScriptTracker::new();
        assert!(!tracker.observe(HI));
        assert!(tracker.observe(HI), "the segment that crosses reports it");
        assert!(!tracker.observe(HI), "staying put is not a switch");
        assert!(!tracker.observe(HI));
    }

    /// The case this exists for: Devanagari costs whisper's decoder several
    /// times what Latin does, and a meeting written in it needs the cheaper
    /// profile.
    #[test]
    fn devanagari_is_recognised_as_an_expensive_script() {
        assert!(uses_expensive_script("मुझे लगता है कि यह ठीक है"));
        // Code-switched Hinglish counts too: the Hindi half is what costs.
        assert!(uses_expensive_script(
            "So basically हमें यह करना चाहिए before the deadline"
        ));
    }

    /// The mistake worth avoiding: European languages are Latin script and
    /// cheap, and flagging them would spend accuracy for no speed.
    #[test]
    fn accented_european_text_is_not_expensive() {
        assert!(!uses_expensive_script("Je pense que c'est déjà réglé"));
        assert!(!uses_expensive_script("Das hätte größer sein müssen"));
        assert!(!uses_expensive_script("La reunión terminó sin más"));
        assert!(!uses_expensive_script("Bugün toplantı iyi geçti"));
    }

    /// Plain English never switches the profile, so an English meeting keeps
    /// the beam search it can afford.
    #[test]
    fn english_keeps_the_careful_profile() {
        assert!(!uses_expensive_script(
            "I think we should ship this before the end of the week"
        ));
    }

    /// One borrowed word does not redecide the meeting.
    #[test]
    fn a_stray_foreign_word_does_not_flip_a_latin_transcript() {
        assert!(!uses_expensive_script(
            "The word नमस्ते came up but the rest of this meeting is in English \
             and should stay on the careful decoder throughout"
        ));
    }

    /// Too little evidence is not evidence.
    #[test]
    fn a_very_short_segment_never_decides() {
        assert!(!uses_expensive_script("हाँ"));
        assert!(!uses_expensive_script(""));
        assert!(!uses_expensive_script("   ...   "));
    }

    /// The cheap profile drops the beam and the fallback together, and keeps
    /// the threshold that makes greedy decoding safe.
    #[test]
    fn the_expensive_script_profile_drops_the_beam_and_the_fallback() {
        let careful = WhisperDecodingConfig::baseline().with_preset(SttPreset::Balanced);
        assert!(matches!(
            careful.strategy,
            SttSamplingStrategy::BeamSearch { .. }
        ));
        assert!(careful.temperature_inc > 0.0);

        let cheap = careful.for_expensive_script();
        assert!(matches!(
            cheap.strategy,
            SttSamplingStrategy::Greedy { best_of: 1 }
        ));
        assert_eq!(cheap.temperature_inc, 0.0);
        // The no-speech threshold moves with the beam rather than being left
        // where a beam search put it — keeping low-confidence segments is only
        // safe while something is choosing among candidates.
        assert_eq!(cheap.no_speech_thold, SttPreset::Fast.no_speech_thold());
    }

    /// Everything the careful profile carried that is not about decode cost
    /// survives the switch.
    #[test]
    fn the_expensive_script_profile_keeps_everything_unrelated_to_cost() {
        let mut careful = WhisperDecodingConfig::baseline().with_preset(SttPreset::Balanced);
        careful.initial_prompt = Some("Vox, Ollama, Parakeet".to_string());
        careful.trim_audio_context = true;
        careful.n_threads = Some(6);

        let cheap = careful.for_expensive_script();
        assert_eq!(cheap.initial_prompt, careful.initial_prompt);
        assert!(cheap.trim_audio_context, "the encoder clamp still applies");
        assert_eq!(cheap.n_threads, careful.n_threads);
    }

    /// The property the last truncation bug violated, checked across the whole
    /// range a segmenter can produce: the clamp must always cover at least as
    /// much audio as it was given, or the tail is silently not encoded.
    #[test]
    fn the_encoder_clamp_always_covers_the_audio_it_was_sized_for() {
        let mut checked = 0;
        for tenths in 1..=400 {
            let seconds = tenths as f32 / 10.0;
            let Some(ctx) = audio_ctx_for_seconds(seconds) else {
                // Falling back to the full window is always safe.
                continue;
            };
            let covered = ctx as f32 / AUDIO_CTX_PER_SECOND;
            assert!(
                covered >= seconds,
                "a {seconds:.1}s segment clamped to {ctx} covers only {covered:.2}s"
            );
            checked += 1;
        }
        assert!(checked > 0, "the sweep has to actually exercise the clamp");
    }

    /// A fixed clamp is what cut tails off before. This is the case that
    /// distinguishes the two: at `LIVE_AUDIO_CTX` a twenty-second segment lost
    /// its end, and sizing per segment must not.
    #[test]
    fn a_segment_longer_than_the_old_fixed_clamp_is_not_truncated() {
        let ctx = audio_ctx_for_seconds(20.0).expect("20s still fits inside the window");
        assert!(
            ctx > LIVE_AUDIO_CTX,
            "sizing to the audio must exceed the fixed clamp that truncated it"
        );
        assert!(ctx as f32 / AUDIO_CTX_PER_SECOND >= 20.0);
    }

    /// Anything at or past the window gets the full encoder rather than a
    /// clamp that would round down into the audio.
    #[test]
    fn audio_at_or_beyond_the_window_keeps_the_full_encoder() {
        assert_eq!(audio_ctx_for_seconds(28.0), None);
        assert_eq!(audio_ctx_for_seconds(30.0), None);
        assert_eq!(audio_ctx_for_seconds(45.0), None);
    }

    /// Short segments are where the saving is, since they pay the same flat
    /// cost as long ones today.
    #[test]
    fn a_short_segment_asks_for_much_less_than_the_full_window() {
        let ctx = audio_ctx_for_seconds(8.0).expect("8s is well inside the window");
        assert!(
            ctx < FULL_AUDIO_CTX / 2,
            "an 8s segment should need under half the window, asked for {ctx}"
        );
    }

    /// Nonsense in, full window out — never a clamp of zero, which would
    /// encode nothing at all.
    #[test]
    fn degenerate_durations_fall_back_to_the_full_window() {
        assert_eq!(audio_ctx_for_seconds(0.0), None);
        assert_eq!(audio_ctx_for_seconds(-1.0), None);
        assert_eq!(audio_ctx_for_seconds(f32::NAN), None);
        assert_eq!(audio_ctx_for_seconds(f32::INFINITY), None);
    }

    /// Meetings opt in through the setting; dictation is left alone, and an
    /// explicit clamp still wins over the automatic one.
    #[test]
    fn only_meetings_trim_the_encoder_and_only_when_the_setting_allows() {
        let on = crate::settings::SttSettings::default();
        assert!(on.meeting_trim_audio_context, "on unless turned off");
        assert!(WhisperDecodingConfig::for_meetings(&on, SttPreset::Balanced).trim_audio_context);

        let off = crate::settings::SttSettings {
            meeting_trim_audio_context: false,
            ..crate::settings::SttSettings::default()
        };
        assert!(!WhisperDecodingConfig::for_meetings(&off, SttPreset::Balanced).trim_audio_context);

        // Dictation never trims, whatever the meeting setting says.
        assert!(!WhisperDecodingConfig::for_dictation(&on).trim_audio_context);
        assert!(!WhisperDecodingConfig::baseline().trim_audio_context);
    }
    use crate::settings::LanguageSettings;

    #[test]
    fn test_stt_language_config_english_only() {
        let settings = LanguageSettings {
            primary_dictation_language: "en".to_string(),
            spoken_languages: vec!["en".to_string()],
            notes_language: "en".to_string(),
            output_script: "latin".to_string(),
        };
        let config = SttLanguageConfig::from_settings(&settings, SttWindow::LongForm);
        assert_eq!(config.whisper_language, Some("en".to_string()));
        assert!(!config.translate);
    }

    #[test]
    fn test_stt_language_config_hindi_only() {
        let settings = LanguageSettings {
            primary_dictation_language: "hi".to_string(),
            spoken_languages: vec!["hi".to_string()],
            notes_language: "hi".to_string(),
            output_script: "native".to_string(),
        };
        let config = SttLanguageConfig::from_settings(&settings, SttWindow::LongForm);
        assert_eq!(config.whisper_language, Some("hi".to_string()));
        assert!(!config.translate);
    }

    #[test]
    fn test_stt_language_config_hinglish_mixed_profile() {
        // Selecting Hinglish in pill: primary="en", spoken=["en", "hi"]
        let settings = LanguageSettings {
            primary_dictation_language: "en".to_string(),
            spoken_languages: vec!["en".to_string(), "hi".to_string()],
            notes_language: "en".to_string(),
            output_script: "latin".to_string(),
        };
        let config = SttLanguageConfig::from_settings(&settings, SttWindow::LongForm);
        // Must NOT pass "hinglish" or hard-lock to "en"
        assert_eq!(config.whisper_language, None);
        assert!(!config.translate);
    }

    #[test]
    fn test_stt_language_config_hindi_primary_mixed_profile() {
        // Hindi primary + English secondary: primary="hi", spoken=["hi", "en"]
        let settings = LanguageSettings {
            primary_dictation_language: "hi".to_string(),
            spoken_languages: vec!["hi".to_string(), "en".to_string()],
            notes_language: "en".to_string(),
            output_script: "latin".to_string(),
        };
        let config = SttLanguageConfig::from_settings(&settings, SttWindow::LongForm);
        // Must NOT hard-lock to "hi"
        assert_eq!(config.whisper_language, None);
        assert!(!config.translate);
    }

    #[test]
    fn test_stt_language_config_auto() {
        let settings = LanguageSettings {
            primary_dictation_language: "auto".to_string(),
            spoken_languages: vec!["en".to_string()],
            notes_language: "en".to_string(),
            output_script: "latin".to_string(),
        };
        let config = SttLanguageConfig::from_settings(&settings, SttWindow::LongForm);
        assert_eq!(config.whisper_language, None);
        assert!(!config.translate);
    }

    #[test]
    fn test_stt_language_config_duplicates_normalized() {
        let settings = LanguageSettings {
            primary_dictation_language: "en".to_string(),
            spoken_languages: vec!["en".to_string(), "en".to_string(), "EN".to_string()],
            notes_language: "en".to_string(),
            output_script: "latin".to_string(),
        };
        let config = SttLanguageConfig::from_settings(&settings, SttWindow::LongForm);
        assert_eq!(config.whisper_language, Some("en".to_string()));
        assert!(!config.translate);
    }

    #[test]
    fn test_stt_language_config_whitespace_and_case() {
        let settings = LanguageSettings {
            primary_dictation_language: " HI ".to_string(),
            spoken_languages: vec!["  hi  ".to_string()],
            notes_language: "hi".to_string(),
            output_script: "native".to_string(),
        };
        let config = SttLanguageConfig::from_settings(&settings, SttWindow::LongForm);
        assert_eq!(config.whisper_language, Some("hi".to_string()));
        assert!(!config.translate);
    }

    #[test]
    fn test_stt_language_config_output_script_independence() {
        let mut settings = LanguageSettings {
            primary_dictation_language: "hi".to_string(),
            spoken_languages: vec!["hi".to_string()],
            notes_language: "hi".to_string(),
            output_script: "latin".to_string(),
        };
        let config_latin = SttLanguageConfig::from_settings(&settings, SttWindow::LongForm);

        settings.output_script = "native".to_string();
        let config_native = SttLanguageConfig::from_settings(&settings, SttWindow::LongForm);

        // Output script setting must NOT change STT language configuration
        assert_eq!(config_latin, config_native);
        assert_eq!(config_latin.whisper_language, Some("hi".to_string()));
        assert!(!config_latin.translate);
    }

    #[test]
    fn test_whisper_decoding_config_baseline_invariants() {
        let baseline = WhisperDecodingConfig::baseline();
        assert_eq!(baseline.strategy, SttSamplingStrategy::Greedy { best_of: 1 });
        assert_eq!(baseline.temperature, 0.0);
        assert_eq!(baseline.temperature_inc, 0.2);
        assert_eq!(baseline.initial_prompt, None);
        assert!(baseline.suppress_blank);
        assert!(!baseline.print_special);
        assert!(!baseline.print_timestamps);
        assert_eq!(baseline.no_speech_thold, 0.6);
        assert_eq!(baseline.entropy_thold, 2.4);
        assert_eq!(baseline.logprob_thold, -1.0);
    }

    #[test]
    fn test_whisper_decoding_experiment_matrix_constructors() {
        let exp_b = WhisperDecodingConfig::experiment_best_of(3);
        assert_eq!(exp_b.strategy, SttSamplingStrategy::Greedy { best_of: 3 });

        let exp_c = WhisperDecodingConfig::experiment_temperature(0.2, 0.2);
        assert_eq!(exp_c.temperature, 0.2);

        let exp_d = WhisperDecodingConfig::experiment_prompt("Relay, Tauri, Rust");
        assert_eq!(exp_d.initial_prompt, Some("Relay, Tauri, Rust".to_string()));

        let exp_f = WhisperDecodingConfig::experiment_thresholds(0.7, 2.2, -1.2);
        assert_eq!(exp_f.no_speech_thold, 0.7);
        assert_eq!(exp_f.entropy_thold, 2.2);
        assert_eq!(exp_f.logprob_thold, -1.2);
    }

    #[test]
    #[cfg(feature = "whisper-local")]
    fn test_whisper_live_decoding_experiments_if_model_present() {
        let current_dir = std::env::current_dir().unwrap();
        let model_paths = [
            current_dir.join(".relay/config/models/ggml-small.bin"),
            current_dir.join("native/src-tauri/.relay/config/models/ggml-small.bin"),
            current_dir.join(".relay/config/models/ggml-base.bin"),
            current_dir.join("native/src-tauri/.relay/config/models/ggml-base.bin"),
        ];

        let model_path = model_paths.into_iter().find(|p| p.exists());
        if let Some(path) = model_path {
            let model_str = path.to_string_lossy().to_string();
            let engine = SttEngine::new();

            // Find a real test WAV file from .relay/config/audio with duration > 2s
            let audio_dirs = [
                current_dir.join(".relay/config/audio"),
                current_dir.join("native/src-tauri/.relay/config/audio"),
            ];

            let mut test_samples: Option<Vec<f32>> = None;
            for audio_dir in &audio_dirs {
                if audio_dir.exists() {
                    if let Ok(entries) = std::fs::read_dir(audio_dir) {
                        for entry in entries.flatten() {
                            let p = entry.path();
                            if p.extension().and_then(|e| e.to_str()) == Some("wav") {
                                if let Ok(reader) = hound::WavReader::open(&p) {
                                    let spec = reader.spec();
                                    let samples: Vec<f32> = match spec.sample_format {
                                        hound::SampleFormat::Float => reader
                                            .into_samples::<f32>()
                                            .filter_map(|s| s.ok())
                                            .collect(),
                                        hound::SampleFormat::Int => {
                                            let max_val = i16::MAX as f32;
                                            reader
                                                .into_samples::<i32>()
                                                .filter_map(|s| s.ok())
                                                .map(|s| s as f32 / max_val)
                                                .collect()
                                        }
                                    };
                                    if samples.len() >= 32000 && samples.len() <= 160000 {
                                        // 2s to 10s audio
                                        test_samples = Some(samples);
                                        break;
                                    }
                                }
                            }
                        }
                    }
                }
                if test_samples.is_some() {
                    break;
                }
            }

            if let Some(samples) = test_samples {
                let lang_en = SttLanguageConfig {
                    whisper_language: Some("en".to_string()),
                    translate: false,
                };
                let lang_auto = SttLanguageConfig {
                    whisper_language: None,
                    translate: false,
                };

                println!("\n==========================================================================");
                println!("           PHASE 4: WHISPER DECODING EXPERIMENT MATRIX RESULTS            ");
                println!("==========================================================================");

                // Exp A: Baseline Greedy
                let cfg_a = WhisperDecodingConfig::baseline();
                let (res_a, diag_a) = engine
                    .transcribe_with_config(Some(&model_str), &samples, &lang_en, &cfg_a)
                    .unwrap();
                println!(
                    "EXP A (Baseline Greedy): latency={}ms, RTF={:.2}, segments={}, len={}\n  -> \"{}\"",
                    diag_a.transcription_latency_ms, diag_a.real_time_factor, diag_a.segment_count, diag_a.transcript_char_count, res_a
                );

                // Exp B: best_of = 3
                let cfg_b = WhisperDecodingConfig::experiment_best_of(3);
                let (res_b, diag_b) = engine
                    .transcribe_with_config(Some(&model_str), &samples, &lang_en, &cfg_b)
                    .unwrap();
                println!(
                    "EXP B (Greedy best_of=3): latency={}ms, RTF={:.2}, segments={}, len={}\n  -> \"{}\"",
                    diag_b.transcription_latency_ms, diag_b.real_time_factor, diag_b.segment_count, diag_b.transcript_char_count, res_b
                );

                // Exp C: Temperature fallback
                let cfg_c = WhisperDecodingConfig::experiment_temperature(0.2, 0.2);
                let (res_c, diag_c) = engine
                    .transcribe_with_config(Some(&model_str), &samples, &lang_en, &cfg_c)
                    .unwrap();
                println!(
                    "EXP C (Temp Fallback 0.2): latency={}ms, RTF={:.2}, segments={}, len={}\n  -> \"{}\"",
                    diag_c.transcription_latency_ms, diag_c.real_time_factor, diag_c.segment_count, diag_c.transcript_char_count, res_c
                );

                // Exp D: Initial Prompt
                let cfg_d = WhisperDecodingConfig::experiment_prompt("Relay, Whisper, Tauri, Rust, Supabase, GitHub, Vercel, n8n");
                let (res_d, diag_d) = engine
                    .transcribe_with_config(Some(&model_str), &samples, &lang_en, &cfg_d)
                    .unwrap();
                println!(
                    "EXP D (Tech Initial Prompt): latency={}ms, RTF={:.2}, segments={}, len={}\n  -> \"{}\"",
                    diag_d.transcription_latency_ms, diag_d.real_time_factor, diag_d.segment_count, diag_d.transcript_char_count, res_d
                );

                // Exp E: Auto vs Locked Language
                let (res_e, diag_e) = engine
                    .transcribe_with_config(Some(&model_str), &samples, &lang_auto, &cfg_a)
                    .unwrap();
                println!(
                    "EXP E (Auto Language Detect): latency={}ms, RTF={:.2}, segments={}, len={}\n  -> \"{}\"",
                    diag_e.transcription_latency_ms, diag_e.real_time_factor, diag_e.segment_count, diag_e.transcript_char_count, res_e
                );

                // Exp F: Hallucination Suppression Thresholds
                let cfg_f = WhisperDecodingConfig::experiment_thresholds(0.7, 2.2, -0.9);
                let (res_f, diag_f) = engine
                    .transcribe_with_config(Some(&model_str), &samples, &lang_en, &cfg_f)
                    .unwrap();
                println!(
                    "EXP F (Hallucination Thresholds): latency={}ms, RTF={:.2}, segments={}, len={}\n  -> \"{}\"",
                    diag_f.transcription_latency_ms, diag_f.real_time_factor, diag_f.segment_count, diag_f.transcript_char_count, res_f
                );
                println!("==========================================================================\n");

                assert!(!diag_a.is_empty);
            }
        }
    }

    #[test]
    fn test_whisper_decoding_config_for_dictation_thread_bounds() {
        let mut settings = crate::settings::SttSettings::default();

        // 1. Default (no override) clamps within [1, 12]
        let cfg_default = WhisperDecodingConfig::for_dictation(&settings);
        let threads = cfg_default.n_threads.unwrap();
        assert!((1..=12).contains(&threads));

        // 2. Safe user override
        settings.dictation_threads = Some(8);
        let cfg_custom = WhisperDecodingConfig::for_dictation(&settings);
        assert_eq!(cfg_custom.n_threads, Some(8));

        // 3. User override <= 0 clamps to 1
        settings.dictation_threads = Some(0);
        let cfg_zero = WhisperDecodingConfig::for_dictation(&settings);
        assert_eq!(cfg_zero.n_threads, Some(1));

        settings.dictation_threads = Some(-5);
        let cfg_neg = WhisperDecodingConfig::for_dictation(&settings);
        assert_eq!(cfg_neg.n_threads, Some(1));

        // 4. Extreme user override clamps to 64
        settings.dictation_threads = Some(128);
        let cfg_high = WhisperDecodingConfig::for_dictation(&settings);
        assert_eq!(cfg_high.n_threads, Some(64));
    }

    #[test]
    fn test_get_stt_models_overview_and_verification() {
        let temp_dir = std::env::temp_dir().join("relay_test_models_dir_test");
        let _ = std::fs::create_dir_all(&temp_dir);

        let mut settings = crate::settings::SttSettings::default();
        settings.dictation_quality = crate::settings::DictationSttQuality::Fast;

        let overview = get_stt_models_overview(&temp_dir, &settings);
        assert_eq!(overview.active_profile, "fast");
        assert_eq!(overview.active_model_name, "Whisper Base");
        // Three managed tiers: fast, default, and the accuracy ceiling. The
        // ceiling is listed whether or not it is on disk, so the UI can offer
        // the download instead of hiding that the option exists.
        assert_eq!(overview.models.len(), 3);
        assert_eq!(overview.models[0].filename, FAST_MODEL_FILENAME);
        assert_eq!(overview.models[1].filename, DEFAULT_MODEL_FILENAME);
        assert_eq!(overview.models[2].filename, ACCURATE_MODEL_FILENAME);
        assert!(!overview.models[2].exists, "not downloaded by default");
        assert_eq!(overview.models[2].status, "missing");

        // Every tier the overview offers must be one the download command can
        // actually fetch. The accuracy ceiling is listed as "missing" so the UI
        // can offer it; if that name did not resolve, the offer would be a
        // button that always fails.
        for model in &overview.models {
            assert!(
                crate::capture::stt::managed_model_is_known(&model.filename),
                "the overview offers {} but nothing can download it",
                model.filename
            );
        }

        // Test file verification on nonexistent file
        let res = test_stt_model_file("nonexistent_model.bin");
        assert!(!res.success);
        assert_eq!(res.error, Some("File does not exist".to_string()));

        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    // ---------------------------------------------------------------------
    // Window-class resolution.
    //
    // One resolution served both a thirty-second meeting chunk and a
    // two-second push-to-talk phrase, and could not be right for both: the
    // profile that stops a Hindi meeting being decoded as English is the same
    // profile that makes Whisper guess the language of a short phrase. These
    // tests pin the two answers apart.
    // ---------------------------------------------------------------------

    /// The profile a bilingual user actually ends up with in Settings ›
    /// Languages & Script: a primary, a second language, and the explicit
    /// auto-detect entry.
    fn bilingual_profile() -> LanguageSettings {
        LanguageSettings {
            primary_dictation_language: "en".to_string(),
            spoken_languages: vec!["en".to_string(), "hi".to_string(), "auto".to_string()],
            notes_language: "en".to_string(),
            output_script: "latin".to_string(),
        }
    }

    #[test]
    fn long_form_honours_a_bilingual_profile() {
        // A meeting chunk carries enough audio for detection to work, so
        // code-switched speech must not be forced through one language.
        let config = SttLanguageConfig::from_settings(&bilingual_profile(), SttWindow::LongForm);
        assert_eq!(config.whisper_language, None);
        assert!(!config.translate);
    }

    #[test]
    fn short_form_pins_the_primary_language() {
        // One phrase is not enough evidence to detect from. The user's stated
        // primary language is better information than the audio.
        let config = SttLanguageConfig::from_settings(&bilingual_profile(), SttWindow::ShortForm);
        assert_eq!(config.whisper_language, Some("en".to_string()));
        assert!(!config.translate);
    }

    #[test]
    fn the_two_windows_disagree_only_when_the_profile_is_multilingual() {
        let bilingual = bilingual_profile();
        assert_ne!(
            SttLanguageConfig::from_settings(&bilingual, SttWindow::LongForm),
            SttLanguageConfig::from_settings(&bilingual, SttWindow::ShortForm),
        );

        // A single-language profile has one right answer, so both windows
        // give it. This is what stops the split becoming a second setting.
        let monolingual = LanguageSettings {
            primary_dictation_language: "hi".to_string(),
            spoken_languages: vec!["hi".to_string()],
            notes_language: "hi".to_string(),
            output_script: "latin".to_string(),
        };
        assert_eq!(
            SttLanguageConfig::from_settings(&monolingual, SttWindow::LongForm),
            SttLanguageConfig::from_settings(&monolingual, SttWindow::ShortForm),
        );
        assert_eq!(
            SttLanguageConfig::from_settings(&monolingual, SttWindow::ShortForm).whisper_language,
            Some("hi".to_string())
        );
    }

    #[test]
    fn an_explicit_auto_primary_detects_in_both_windows() {
        // Choosing "auto" as the primary language is a deliberate request, and
        // there is nothing to pin to. Short-form obeys it rather than
        // inventing a language.
        let settings = LanguageSettings {
            primary_dictation_language: "auto".to_string(),
            spoken_languages: vec!["en".to_string(), "hi".to_string()],
            notes_language: "en".to_string(),
            output_script: "latin".to_string(),
        };
        assert_eq!(
            SttLanguageConfig::from_settings(&settings, SttWindow::LongForm).whisper_language,
            None
        );
        assert_eq!(
            SttLanguageConfig::from_settings(&settings, SttWindow::ShortForm).whisper_language,
            None
        );
    }

    #[test]
    fn auto_in_the_spoken_profile_is_not_a_language() {
        // `auto` is not an ISO code, so it is never pinned and never counted.
        // Long-form reads it as the request it is; a profile of one real
        // language plus `auto` still means "detect", not "two languages".
        let settings = LanguageSettings {
            primary_dictation_language: "en".to_string(),
            spoken_languages: vec!["en".to_string(), "auto".to_string()],
            notes_language: "en".to_string(),
            output_script: "latin".to_string(),
        };
        assert_eq!(
            SttLanguageConfig::from_settings(&settings, SttWindow::LongForm).whisper_language,
            None
        );
        // Whatever the profile says, a non-ISO token must never reach Whisper.
        for window in [SttWindow::LongForm, SttWindow::ShortForm] {
            let resolved = SttLanguageConfig::from_settings(&settings, window);
            assert_ne!(resolved.whisper_language, Some(AUTO_LANGUAGE.to_string()));
        }
    }

    // ---------------------------------------------------------------------
    // Decode presets, and the configuration the streaming transcriber used to
    // hold privately.
    // ---------------------------------------------------------------------

    #[test]
    fn the_default_preset_is_what_relay_already_did() {
        // Presets must be inert until a surface opts in. Fast is greedy at
        // whisper's stock threshold, which is exactly the behaviour that
        // shipped before this enum existed.
        let cfg = WhisperDecodingConfig::baseline();
        assert_eq!(cfg.strategy, SttSamplingStrategy::Greedy { best_of: 1 });
        assert_eq!(cfg.no_speech_thold, 0.6);
        assert_eq!(SttPreset::default(), SttPreset::Fast);
        // A `SttSettings` built by `Default` carries an empty preset string,
        // and `from_setting` is total, so the two agree.
        let defaulted = crate::settings::SttSettings::default();
        assert_eq!(SttPreset::from_setting(&defaulted.preset), SttPreset::Fast);
    }

    #[test]
    fn accuracy_first_widens_the_beam_and_lowers_the_threshold_together() {
        // The counter-intuitive half: keeping more low-confidence speech is
        // only safe because the beam is wider. A preset that raised the
        // threshold *and* the beam would be tuning against itself.
        let fast = SttPreset::Fast;
        let quality = SttPreset::Quality;
        assert!(quality.no_speech_thold() < fast.no_speech_thold());
        assert!(matches!(
            quality.sampling(),
            SttSamplingStrategy::BeamSearch { beam_size: 5, .. }
        ));
        assert!(matches!(
            fast.sampling(),
            SttSamplingStrategy::Greedy { .. }
        ));

        // And the ordering holds across all three, in both knobs at once.
        let balanced = SttPreset::Balanced;
        assert!(balanced.no_speech_thold() < fast.no_speech_thold());
        assert!(quality.no_speech_thold() < balanced.no_speech_thold());
    }

    #[test]
    fn an_unknown_preset_falls_back_rather_than_failing() {
        // A corrupted or hand-edited settings file must not break capture.
        for garbage in ["", "  ", "turbo", "QUALITY_"] {
            assert_eq!(SttPreset::from_setting(garbage), SttPreset::Fast);
        }
        // Casing and padding are tolerated for the real values.
        assert_eq!(SttPreset::from_setting(" Quality "), SttPreset::Quality);
        assert_eq!(SttPreset::from_setting("BALANCED"), SttPreset::Balanced);
    }

    #[test]
    fn a_preset_changes_only_its_own_two_fields() {
        // `with_preset` is applied over a configuration that already carries a
        // vocabulary prompt and a thread count. Losing either would silently
        // undo the glossary fix.
        let mut base = WhisperDecodingConfig::baseline();
        base.initial_prompt = Some("NavGurukul, Pragati".to_string());
        base.n_threads = Some(6);
        let tuned = base.clone().with_preset(SttPreset::Quality);

        assert_eq!(tuned.initial_prompt, base.initial_prompt);
        assert_eq!(tuned.n_threads, base.n_threads);
        assert_eq!(tuned.entropy_thold, base.entropy_thold);
        assert_eq!(tuned.logprob_thold, base.logprob_thold);
        assert_ne!(tuned.strategy, base.strategy);
    }

    #[test]
    fn the_live_window_configuration_carries_the_thresholds() {
        // The regression this exists to prevent: `for_live_window` replaced
        // hardcoded parameters that set *none* of the three thresholds, so
        // whisper's defaults applied and nothing screened the result.
        let live = WhisperDecodingConfig::for_live_window();
        let baseline = WhisperDecodingConfig::baseline();

        assert_eq!(live.no_speech_thold, baseline.no_speech_thold);
        assert_eq!(live.entropy_thold, baseline.entropy_thold);
        assert_eq!(live.logprob_thold, baseline.logprob_thold);

        // And it still describes a short independent window.
        assert_eq!(live.audio_ctx, Some(LIVE_AUDIO_CTX));
        assert!(live.single_segment);
        assert!(live.no_context);
    }

    #[test]
    fn an_utterance_gets_the_full_context_and_every_segment() {
        // The reported "captures only part of what I say". A Talkback turn
        // runs up to thirty seconds; the live-window shape truncated it at the
        // ~15 seconds `LIVE_AUDIO_CTX` covers and merged the rest into one
        // segment.
        let cfg = WhisperDecodingConfig::for_utterance();
        assert_eq!(cfg.audio_ctx, None, "a full turn needs the full context");
        assert!(!cfg.single_segment, "a 30-second question is not one segment");
        // Still independent: each turn is a new question.
        assert!(cfg.no_context);
    }


    #[test]
    fn each_surface_defaults_the_preset_and_the_user_overrides_both() {
        // Per-surface policy from one setting. Dictation is latency-bound and
        // a meeting is recall-bound, so they cannot share a default — but a
        // user who states a preference must not have to state it twice.
        let unset = crate::settings::SttSettings::default();
        assert_eq!(unset.configured_preset(), None);

        let dictation = WhisperDecodingConfig::for_dictation(&unset);
        let meeting = WhisperDecodingConfig::from_settings_defaulting(&unset, SttPreset::Quality);
        assert_eq!(dictation.strategy, SttPreset::Fast.sampling());
        assert_eq!(meeting.strategy, SttPreset::Quality.sampling());
        assert!(meeting.no_speech_thold < dictation.no_speech_thold);

        // An explicit choice wins on both surfaces.
        let chosen = crate::settings::SttSettings {
            preset: "balanced".to_string(),
            ..Default::default()
        };
        assert_eq!(chosen.configured_preset(), Some(SttPreset::Balanced));
        assert_eq!(
            WhisperDecodingConfig::for_dictation(&chosen).strategy,
            SttPreset::Balanced.sampling()
        );
        assert_eq!(
            WhisperDecodingConfig::from_settings_defaulting(&chosen, SttPreset::Quality).strategy,
            SttPreset::Balanced.sampling()
        );
    }

    #[test]
    fn the_batch_configuration_does_not_clamp_or_truncate() {
        // A thirty-second chunk needs whisper's full encoder context and more
        // than one segment. These are the two values the live window sets and
        // the batch path must not inherit.
        let cfg = WhisperDecodingConfig::baseline();
        assert_eq!(cfg.audio_ctx, None, "batch decodes use the full context");
        assert!(!cfg.single_segment, "a chunk holds many segments");
    }

    #[test]
    fn no_window_ever_enables_translation() {
        // Relay never turns Hindi speech into English text. This is the one
        // property the window class must not be able to change.
        for window in [SttWindow::LongForm, SttWindow::ShortForm] {
            assert!(!SttLanguageConfig::from_settings(&bilingual_profile(), window).translate);
        }
    }
    /// A models directory that cleans itself up.
    struct ModelDir(PathBuf);

    impl ModelDir {
        fn new() -> Self {
            let path =
                std::env::temp_dir().join(format!("vox_test_stt_{}", uuid::Uuid::new_v4()));
            std::fs::create_dir_all(&path).unwrap();
            Self(path)
        }

        fn with(models: &[&str]) -> Self {
            let dir = Self::new();
            for filename in models {
                let mut bytes = b"ggml".to_vec();
                bytes.resize(2_000_000, b'x');
                std::fs::write(dir.0.join(filename), bytes).unwrap();
            }
            dir
        }
    }

    impl Drop for ModelDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn a_meeting_uses_an_installed_model_even_when_it_is_not_the_default() {
        // The defect this exists for: resolution looked only for
        // `ggml-small.bin`, so a machine that had fetched Base for dictation —
        // the fast profile's model, and the only one many installs ever get —
        // was told no speech model was installed.
        let dir = ModelDir::with(&["ggml-base.bin"]);
        let settings = crate::settings::SttSettings::default();

        let resolved = resolve_meeting_model_path(&dir.0, &settings).unwrap();
        assert!(resolved.ends_with("ggml-base.bin"));
    }

    #[test]
    fn a_meeting_prefers_the_most_accurate_installed_model() {
        let dir = ModelDir::with(&["ggml-base.bin", "ggml-small.bin", "ggml-large-v3-turbo.bin"]);
        let settings = crate::settings::SttSettings::default();

        let resolved = resolve_meeting_model_path(&dir.0, &settings).unwrap();
        assert!(resolved.ends_with("ggml-large-v3-turbo.bin"));
    }

    #[test]
    fn an_explicit_meeting_model_wins_over_the_most_accurate_one() {
        let dir = ModelDir::with(&["ggml-base.bin", "ggml-large-v3-turbo.bin"]);
        let settings = crate::settings::SttSettings {
            meeting_model_id: Some("whisper-base".into()),
            ..Default::default()
        };

        let resolved = resolve_meeting_model_path(&dir.0, &settings).unwrap();
        assert!(resolved.ends_with("ggml-base.bin"));
    }

    #[test]
    fn a_chosen_model_that_was_deleted_falls_through_rather_than_failing() {
        let dir = ModelDir::with(&["ggml-base.bin"]);
        let settings = crate::settings::SttSettings {
            meeting_model_id: Some("whisper-large-v3".into()),
            ..Default::default()
        };

        let resolved = resolve_meeting_model_path(&dir.0, &settings).unwrap();
        assert!(resolved.ends_with("ggml-base.bin"));
    }

    #[test]
    fn a_dictation_path_is_used_only_when_the_file_is_there() {
        let dir = ModelDir::with(&["ggml-base.bin"]);
        let ghost = dir.0.join("ggml-medium.bin");
        let settings = crate::settings::SttSettings {
            whisper_model_path: Some(ghost.to_string_lossy().to_string()),
            ..Default::default()
        };

        let resolved = resolve_meeting_model_path(&dir.0, &settings).unwrap();
        assert!(resolved.ends_with("ggml-base.bin"));
    }

    #[test]
    fn an_empty_models_directory_still_has_no_meeting_model() {
        let dir = ModelDir::new();
        let settings = crate::settings::SttSettings::default();
        assert!(resolve_meeting_model_path(&dir.0, &settings).is_none());
    }

    #[test]
    fn dictation_reports_the_file_it_would_use_without_fetching_it() {
        use crate::settings::DictationSttQuality;

        let fast = crate::settings::SttSettings {
            dictation_quality: DictationSttQuality::Fast,
            ..Default::default()
        };
        assert_eq!(
            dictation_model_filename(&fast).as_deref(),
            Some(FAST_MODEL_FILENAME)
        );

        let accurate = crate::settings::SttSettings {
            dictation_quality: DictationSttQuality::Accurate,
            ..Default::default()
        };
        assert_eq!(
            dictation_model_filename(&accurate).as_deref(),
            Some(DEFAULT_MODEL_FILENAME)
        );

        let pinned = crate::settings::SttSettings {
            dictation_quality: DictationSttQuality::Accurate,
            whisper_model_path: Some("/somewhere/ggml-medium.bin".into()),
            ..Default::default()
        };
        assert_eq!(
            dictation_model_filename(&pinned).as_deref(),
            Some("ggml-medium.bin")
        );
    }
}
