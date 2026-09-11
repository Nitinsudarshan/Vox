//! Getting Ollama onto the machine.
//!
//! [`super::ollama_manager`] can start a server it can find and pull models
//! into it. What it could not do is the first step: `OllamaStatus::NotInstalled`
//! was a dead end, and the comment above it said so — "Relay can manage the
//! *process*, but can't conjure the install itself". A user who wanted local
//! summaries had to work out on their own that Ollama existed.
//!
//! ## Why this rather than bundling an inference runtime
//!
//! Meetily's answer to the same problem is a llama.cpp sidecar shipped inside
//! the app. It works, and it costs a second inference runtime next to
//! whisper.cpp — they collide on GGML symbols, which is why it has to be a
//! separate process at all — plus a second model format, a second download
//! catalogue, and a child process to supervise. Vox already manages the Ollama
//! process and its model pulls. The gap was one install step, so this closes
//! one install step.
//!
//! ## What this does not do
//!
//! It never installs anything on its own. Every function here runs because
//! someone pressed a button, the download comes from one hard-coded host, and
//! on Windows the platform's own installer is launched *visibly* — the user
//! watches it and can cancel. Silently running a freshly downloaded executable
//! is not something an app should do on a user's behalf, and a silent install
//! buys them nothing they wanted.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// The only host Ollama is ever fetched from.
///
/// Hard-coded for the same reason the speech-model catalogue is: a function
/// that takes a URL and runs what comes back is a function that will one day
/// be handed a different URL.
const DOWNLOAD_HOST: &str = "https://ollama.com/download";

/// How a platform's download is turned into an installed Ollama.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ArtifactKind {
    /// An installer the user completes. Windows.
    Installer,
    /// An archive Vox unpacks into its own data directory.
    Archive,
}

/// What installing Ollama looks like on one platform.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InstallPlan {
    pub url: String,
    pub filename: String,
    pub kind: ArtifactKind,
    /// Roughly how large, so the UI can say before the download starts.
    pub approx_bytes: u64,
    /// One line describing what pressing the button will do.
    pub description: String,
}

/// The plan for a target triple's OS and architecture, or `None` where Ollama
/// publishes no build.
///
/// Takes the pair rather than reading `std::env::consts`, so every platform's
/// answer is testable from any machine — which matters for a Windows-first app
/// whose tests run on Linux in CI.
pub fn plan_for(os: &str, arch: &str) -> Option<InstallPlan> {
    let (filename, kind, approx_bytes, description) = match (os, arch) {
        ("windows", "x86_64") => (
            "OllamaSetup.exe",
            ArtifactKind::Installer,
            750_000_000,
            "Downloads Ollama's installer and opens it. You complete the install; it needs no \
             administrator rights.",
        ),
        // Ollama ships one macOS build covering both architectures.
        ("macos", _) => (
            "Ollama-darwin.zip",
            ArtifactKind::Archive,
            450_000_000,
            "Downloads Ollama and unpacks it where Vox keeps its own files.",
        ),
        ("linux", "x86_64") => (
            "ollama-linux-amd64.tgz",
            ArtifactKind::Archive,
            1_600_000_000,
            "Downloads Ollama and unpacks it where Vox keeps its own files — no root, and \
             nothing outside your home directory.",
        ),
        ("linux", "aarch64") => (
            "ollama-linux-arm64.tgz",
            ArtifactKind::Archive,
            1_500_000_000,
            "Downloads Ollama and unpacks it where Vox keeps its own files — no root, and \
             nothing outside your home directory.",
        ),
        _ => return None,
    };

    Some(InstallPlan {
        url: format!("{DOWNLOAD_HOST}/{filename}"),
        filename: filename.to_string(),
        kind,
        approx_bytes,
        description: description.to_string(),
    })
}

/// The plan for the machine this is running on.
pub fn plan_for_this_machine() -> Option<InstallPlan> {
    plan_for(std::env::consts::OS, std::env::consts::ARCH)
}

/// Where an unpacked Ollama lives, under Vox's own data directory.
///
/// Deliberately not `/usr/local/bin`: installing Ollama should not need root,
/// and an app that writes outside the user's home without being asked is an
/// app people uninstall.
pub fn managed_install_dir(data_dir: &Path) -> PathBuf {
    data_dir.join("ollama")
}

/// The binary inside a managed install, if one is there.
///
/// Checked rather than assumed because the two archives unpack differently —
/// the Linux tarball has `bin/ollama`, the macOS zip an app bundle — and a
/// path that does not exist is worse than none: it makes
/// [`super::ollama_manager`] try to spawn something that is not there instead
/// of falling back to `PATH`.
pub fn managed_binary(data_dir: &Path) -> Option<PathBuf> {
    let root = managed_install_dir(data_dir);
    let candidates = [
        root.join("bin").join("ollama"),
        root.join("ollama"),
        root.join("Ollama.app")
            .join("Contents")
            .join("Resources")
            .join("ollama"),
        root.join("Ollama.app")
            .join("Contents")
            .join("MacOS")
            .join("ollama"),
    ];
    candidates.into_iter().find(|path| path.is_file())
}

/// How an install is going, for the UI.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "state")]
pub enum InstallProgress {
    Downloading {
        downloaded_bytes: u64,
        total_bytes: Option<u64>,
    },
    /// The bytes are here and being checked or unpacked.
    Preparing,
    /// The platform's installer is open and waiting for the user.
    AwaitingUser,
    /// Installed and answering.
    Ready,
    Failed {
        message: String,
    },
}

#[derive(Debug, thiserror::Error)]
pub enum InstallError {
    #[error("Vox does not know how to install Ollama on this platform. Install it from ollama.com.")]
    UnsupportedPlatform,

    #[error("could not download Ollama: {0}")]
    Download(String),

    #[error("the Ollama download server returned HTTP {0}")]
    HttpStatus(u16),

    #[error("could not write to {path}: {message}")]
    Io { path: String, message: String },

    #[error("the download does not look like Ollama's {expected}")]
    NotTheExpectedArtifact { expected: String },

    #[error("could not start the installer: {0}")]
    Launch(String),
}

/// The smallest a real Ollama download can be.
///
/// A guard against the case a size check exists for: a captive portal or an
/// error page served with a 200, saved under the right name and then executed.
const MIN_PLAUSIBLE_BYTES: u64 = 10_000_000;

/// Whether these first bytes are the artifact the plan expects.
///
/// The same standard the speech-model downloads hold: something that arrived
/// over the network is not run or unpacked until it looks like what was asked
/// for. It matters more here, because on Windows the thing being checked is an
/// executable that is about to be launched.
pub fn looks_like_artifact(kind: ArtifactKind, header: &[u8], total: u64) -> bool {
    if total < MIN_PLAUSIBLE_BYTES {
        return false;
    }
    match kind {
        // `MZ` — the DOS header every Windows executable still starts with.
        ArtifactKind::Installer => header.starts_with(b"MZ"),
        // A zip (`PK\x03\x04`) or a gzip (`\x1f\x8b`), which is what a `.tgz` is.
        ArtifactKind::Archive => header.starts_with(b"PK\x03\x04") || header.starts_with(&[0x1f, 0x8b]),
    }
}

/// Downloads the artifact, reporting progress, and returns where it landed.
///
/// Streams to a `.part` and renames only after the header check, so a partial
/// or wrong download is never left somewhere that looks installable.
pub async fn download<F>(
    plan: &InstallPlan,
    into: &Path,
    on_progress: F,
) -> Result<PathBuf, InstallError>
where
    F: Fn(InstallProgress),
{
    use std::io::Write;

    std::fs::create_dir_all(into).map_err(|e| InstallError::Io {
        path: into.display().to_string(),
        message: e.to_string(),
    })?;

    let target = into.join(&plan.filename);
    let part = target.with_extension("part");

    let response = reqwest::get(&plan.url)
        .await
        .map_err(|e| InstallError::Download(e.to_string()))?;
    if !response.status().is_success() {
        return Err(InstallError::HttpStatus(response.status().as_u16()));
    }
    let total_bytes = response.content_length();

    let mut file = std::fs::File::create(&part).map_err(|e| InstallError::Io {
        path: part.display().to_string(),
        message: e.to_string(),
    })?;

    let mut downloaded = 0u64;
    let mut header: Vec<u8> = Vec::with_capacity(4);
    let report_every = 4 * 1024 * 1024;
    let mut next_report = report_every;
    let mut response = response;

    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|e| InstallError::Download(e.to_string()))?
    {
        if header.len() < 4 {
            header.extend_from_slice(&chunk[..chunk.len().min(4 - header.len())]);
        }
        file.write_all(&chunk).map_err(|e| InstallError::Io {
            path: part.display().to_string(),
            message: e.to_string(),
        })?;
        downloaded += chunk.len() as u64;
        if downloaded >= next_report {
            next_report = downloaded + report_every;
            on_progress(InstallProgress::Downloading {
                downloaded_bytes: downloaded,
                total_bytes,
            });
        }
    }
    file.flush().map_err(|e| InstallError::Io {
        path: part.display().to_string(),
        message: e.to_string(),
    })?;
    drop(file);

    on_progress(InstallProgress::Preparing);

    if !looks_like_artifact(plan.kind, &header, downloaded) {
        let _ = std::fs::remove_file(&part);
        return Err(InstallError::NotTheExpectedArtifact {
            expected: plan.filename.clone(),
        });
    }

    std::fs::rename(&part, &target).map_err(|e| InstallError::Io {
        path: target.display().to_string(),
        message: e.to_string(),
    })?;
    Ok(target)
}

/// Unpacks a downloaded archive into Vox's own data directory.
///
/// Returns the binary it found. Nothing here needs root and nothing is written
/// outside the directory it is given — an app that puts files in
/// `/usr/local/bin` without being asked is an app people uninstall.
pub fn extract_archive(archive: &Path, data_dir: &Path) -> Result<PathBuf, InstallError> {
    let into = managed_install_dir(data_dir);
    std::fs::create_dir_all(&into).map_err(|e| InstallError::Io {
        path: into.display().to_string(),
        message: e.to_string(),
    })?;

    let file = std::fs::File::open(archive).map_err(|e| InstallError::Io {
        path: archive.display().to_string(),
        message: e.to_string(),
    })?;

    let is_zip = archive
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.eq_ignore_ascii_case("zip"))
        .unwrap_or(false);

    if is_zip {
        let mut zip = zip::ZipArchive::new(file).map_err(|e| InstallError::Io {
            path: archive.display().to_string(),
            message: e.to_string(),
        })?;
        zip.extract(&into).map_err(|e| InstallError::Io {
            path: into.display().to_string(),
            message: e.to_string(),
        })?;
    } else {
        let decoder = flate2::read::GzDecoder::new(file);
        tar::Archive::new(decoder)
            .unpack(&into)
            .map_err(|e| InstallError::Io {
                path: into.display().to_string(),
                message: e.to_string(),
            })?;
    }

    // The archive is several hundred megabytes and is of no further use.
    let _ = std::fs::remove_file(archive);

    let binary = managed_binary(data_dir).ok_or_else(|| InstallError::NotTheExpectedArtifact {
        expected: "an ollama binary".to_string(),
    })?;
    make_executable(&binary);
    Ok(binary)
}

/// Restores the executable bit, which neither archive format reliably carries
/// through every extractor.
#[cfg(unix)]
fn make_executable(path: &Path) {
    use std::os::unix::fs::PermissionsExt;
    if let Ok(metadata) = std::fs::metadata(path) {
        let mut perms = metadata.permissions();
        perms.set_mode(perms.mode() | 0o755);
        let _ = std::fs::set_permissions(path, perms);
    }
}

#[cfg(not(unix))]
fn make_executable(_path: &Path) {}

/// Opens the downloaded installer and leaves the rest to the user.
///
/// Visible, never silent. The user pressed a button in Vox and the platform's
/// own installer appearing is what they expect next; it is also their chance
/// to say no. A silent install of a freshly downloaded executable is a thing
/// an app should not do on someone's behalf, and it buys them nothing.
#[cfg(windows)]
pub fn launch_installer(path: &Path) -> Result<(), InstallError> {
    std::process::Command::new(path)
        .spawn()
        .map(|_| ())
        .map_err(|e| InstallError::Launch(e.to_string()))
}

#[cfg(not(windows))]
pub fn launch_installer(_path: &Path) -> Result<(), InstallError> {
    Err(InstallError::UnsupportedPlatform)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_supported_platform_has_a_plan_and_an_official_url() {
        for (os, arch) in [
            ("windows", "x86_64"),
            ("macos", "x86_64"),
            ("macos", "aarch64"),
            ("linux", "x86_64"),
            ("linux", "aarch64"),
        ] {
            let plan = plan_for(os, arch)
                .unwrap_or_else(|| panic!("no plan for {os}/{arch}"));
            assert!(
                plan.url.starts_with("https://ollama.com/download/"),
                "{os}/{arch} would fetch from {}",
                plan.url
            );
            assert!(plan.url.ends_with(&plan.filename));
            assert!(!plan.description.is_empty());
        }
    }

    #[test]
    fn windows_gets_an_installer_and_everything_else_an_archive() {
        assert_eq!(plan_for("windows", "x86_64").unwrap().kind, ArtifactKind::Installer);
        assert_eq!(plan_for("linux", "x86_64").unwrap().kind, ArtifactKind::Archive);
        assert_eq!(plan_for("macos", "aarch64").unwrap().kind, ArtifactKind::Archive);
    }

    #[test]
    fn the_two_linux_architectures_fetch_different_builds() {
        let amd = plan_for("linux", "x86_64").unwrap();
        let arm = plan_for("linux", "aarch64").unwrap();
        assert_ne!(amd.url, arm.url);
        assert!(amd.filename.contains("amd64"));
        assert!(arm.filename.contains("arm64"));
    }

    #[test]
    fn an_unknown_platform_has_no_plan_rather_than_a_guess() {
        assert!(plan_for("freebsd", "x86_64").is_none());
        assert!(plan_for("windows", "aarch64").is_none());
    }

    #[test]
    fn an_unsupported_platform_still_tells_the_user_where_to_go() {
        assert!(InstallError::UnsupportedPlatform.to_string().contains("ollama.com"));
    }

    #[test]
    fn a_windows_installer_must_start_with_an_executable_header() {
        let big = 800_000_000;
        assert!(looks_like_artifact(ArtifactKind::Installer, b"MZ\x90\x00", big));
        // An error page saved under the right name and then executed is the
        // failure this check exists for.
        assert!(!looks_like_artifact(ArtifactKind::Installer, b"<!DO", big));
        assert!(!looks_like_artifact(ArtifactKind::Installer, b"PK\x03\x04", big));
    }

    #[test]
    fn an_archive_must_be_a_zip_or_a_gzip() {
        let big = 800_000_000;
        assert!(looks_like_artifact(ArtifactKind::Archive, b"PK\x03\x04", big));
        assert!(looks_like_artifact(ArtifactKind::Archive, &[0x1f, 0x8b, 0x08, 0x00], big));
        assert!(!looks_like_artifact(ArtifactKind::Archive, b"MZ\x90\x00", big));
        assert!(!looks_like_artifact(ArtifactKind::Archive, b"<!DO", big));
    }

    #[test]
    fn a_truncated_download_is_refused_whatever_its_header_says() {
        assert!(!looks_like_artifact(ArtifactKind::Installer, b"MZ\x90\x00", 4));
        assert!(!looks_like_artifact(ArtifactKind::Archive, b"PK\x03\x04", 1_000));
    }

    #[test]
    fn a_managed_install_lives_under_vox_and_not_in_usr_local() {
        let dir = managed_install_dir(Path::new("/home/u/.local/share/vox"));
        assert!(dir.ends_with("ollama"));
        assert!(dir.starts_with("/home/u"));
    }

    #[test]
    fn no_managed_binary_is_reported_when_nothing_was_unpacked() {
        // A path that does not exist is worse than none: it makes the manager
        // spawn something absent instead of falling back to PATH.
        assert!(managed_binary(Path::new("/definitely/not/here")).is_none());
    }

    #[test]
    fn a_managed_binary_is_found_where_the_linux_tarball_puts_it() {
        let root = std::env::temp_dir().join(format!("vox_test_ollama_{}", uuid::Uuid::new_v4()));
        let bin = root.join("ollama").join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        std::fs::write(bin.join("ollama"), b"#!/bin/sh\n").unwrap();

        let found = managed_binary(&root).expect("bin/ollama should be found");
        assert!(found.ends_with("bin/ollama"));

        let _ = std::fs::remove_dir_all(&root);
    }
}
