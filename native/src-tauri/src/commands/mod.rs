//! Tauri commands, grouped by the surface they serve.
//!
//! Every command is re-exported here, so `commands::start_capture` and the
//! `generate_handler!` list in `lib.rs` do not depend on which file a command
//! lives in. What the submodules share — `AppState`, `CommandError`, the
//! capture-status events, `persist_settings` — is defined in this file.

use crate::sync::MutexExt;
use crate::capture::{AudioRecorder, SttEngine};
use crate::hotkeys;
use crate::pipeline::{PipelineEngine, ProcessedPipelineResult};
use crate::providers::{LLMClient, OllamaStatus, ProviderType};
use crate::settings::{AppSettings, HotkeySettings, PillPosition};
use crate::vault::{
    GraphFilter, KanbanCard, KnowledgeGraphData, KnowledgeSearchResult,
    Scribble, ScribbleRelationship, TrashItem, VaultFile, VaultManager, VaultNote,
};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use tauri::{AppHandle, Emitter, Manager, State};
use tauri_plugin_dialog::DialogExt;

/// Broadcast to every window whenever the shared microphone session starts
/// or stops, so any surface (main window, floating pill, indicator) can
/// reflect the true backend state instead of guessing from its own clicks.
pub const CAPTURE_STATE_EVENT: &str = "capture-state-changed";

#[derive(Debug, Clone, Serialize)]
pub struct CaptureStatus {
    pub active: bool,
    pub mode: Option<String>,
    pub status: String,
    pub message: Option<String>,
}

pub fn emit_capture_state(app: &AppHandle, recorder: &AudioRecorder) {
    let mode = recorder.active_mode();
    let active = recorder.is_active();
    let status = if active { "LISTENING" } else { "IDLE" };
    let payload = CaptureStatus {
        active,
        mode,
        status: status.to_string(),
        message: None,
    };
    let _ = app.emit(CAPTURE_STATE_EVENT, payload);
}

pub fn emit_capture_status_event(
    app: &AppHandle,
    active: bool,
    mode: Option<String>,
    status: &str,
    message: Option<String>,
) {
    let payload = CaptureStatus {
        active,
        mode,
        status: status.to_string(),
        message,
    };
    let _ = app.emit(CAPTURE_STATE_EVENT, payload);
}

#[derive(Debug, Serialize, Deserialize)]
pub struct CommandError {
    pub code: String,
    pub message: String,
}

impl CommandError {
    pub fn new(code: &str, message: &str) -> Self {
        Self {
            code: code.to_string(),
            message: message.to_string(),
        }
    }
}

pub const STT_DIAGNOSTICS_EVENT: &str = "stt-diagnostics-updated";

pub struct AppState {
    pub recorder: AudioRecorder,
    pub vault: VaultManager,
    /// The process-relative vault path Relay used before Vault Directory
    /// Location was configurable — the "Use Default Vox Vault" choice in
    /// first-time setup, and the fallback whenever nothing is configured.
    pub default_vault_dir: PathBuf,
    pub config_dir: PathBuf,
    pub settings: Mutex<AppSettings>,
    pub stt: SttEngine,
    pub last_stt_diagnostics: Mutex<Option<crate::capture::SttDiagnosticSnapshot>>,
    /// The loopback listener the Relay browser extension posts captures to.
    /// `None` whenever capture is switched off, which is the default.
    pub capture_bridge: Mutex<Option<crate::capture::web::bridge::BridgeHandle>>,
    pub memory_store: Arc<crate::memory::MemoryStore>,
    pub relationship_store: Arc<crate::relationships::RelationshipStore>,
    pub entity_store: Arc<crate::entities::EntityStore>,
    /// Meetings on disk. Shared with the engine and the summary service, which
    /// both write through it rather than keeping their own view.
    pub meeting_store: Arc<crate::meetings::MeetingStore>,
    /// The one recording that can be in flight.
    pub meeting_engine: Arc<crate::meetings::engine::MeetingEngine>,
    pub summary_service: Arc<crate::meetings::summary::service::SummaryService>,
    /// Cancellation handles for in-flight imports and re-transcriptions.
    pub meeting_imports: Arc<crate::meetings::commands::ImportRegistry>,
}

impl AppState {
    fn settings_path(&self) -> PathBuf {
        self.config_dir.join("settings.json")
    }

    /// Refuses a vault move while a meeting is recording. The recording's
    /// chunks and checkpoints are being written under the current vault, and
    /// moving it underneath them would split one meeting across two vaults.
    fn ensure_vault_can_move(&self) -> Result<(), CommandError> {
        if self.meeting_engine.is_recording() {
            return Err(CommandError::new(
                "VAULT_BUSY",
                "Stop the meeting recording before moving the vault.",
            ));
        }
        Ok(())
    }

    /// Points every vault-backed store at `new_dir`: notes, meetings,
    /// memories, relationships and entities, plus the asset scope the meeting
    /// player reads recordings through. Nothing at the old location is moved,
    /// migrated or deleted.
    ///
    /// Only `vault` used to be repointed, so after a move the other stores kept
    /// reading — and writing — the vault Vox had launched with.
    fn repoint_vault(&self, app: &AppHandle, new_dir: &std::path::Path) {
        self.vault.set_vault_dir(new_dir.to_path_buf());
        self.meeting_store.set_vault_dir(new_dir);
        self.memory_store.reopen(new_dir);
        self.relationship_store.reopen(new_dir);
        self.entity_store.reopen(new_dir);

        let meetings_dir = self.meeting_store.meetings_dir();
        if let Err(error) = app.asset_protocol_scope().allow_directory(&meetings_dir, true) {
            tracing::warn!(
                "meeting audio playback unavailable in the moved vault: could not allow {}: {}",
                meetings_dir.display(),
                error
            );
        }
    }
}


pub fn record_stt_diagnostics(
    app: &AppHandle,
    state: &AppState,
    snapshot: crate::capture::SttDiagnosticSnapshot,
) {
    // The full snapshot stays in memory for the panel showing the last run.
    // A reduced, transcript-free record is retained on disk, because the
    // questions worth asking of this data — are input levels low enough to
    // want a gain stage, does pinning the language starve decodes — are about
    // the distribution across runs and cannot be answered from the last one.
    crate::capture::decode_history::append(
        &state.config_dir,
        &crate::capture::decode_history::DecodeRecord::from_snapshot(&snapshot),
    );

    let mut guard = state.last_stt_diagnostics.lock_or_recover();
    *guard = Some(snapshot.clone());
    let _ = app.emit(STT_DIAGNOSTICS_EVENT, &snapshot);
}

/// Writes one settings change through, without the whole `save_settings` ritual.
///
/// `save_settings` takes a complete document from the frontend and, on the way
/// past, re-registers hotkeys, restarts the capture bridge and reconciles the
/// OS launch entry. A command that changes one STT field wants none of that —
/// and, more to the point, a round trip through the frontend's copy of the
/// document would let a stale copy overwrite whatever else changed meanwhile.
pub(crate) fn persist_settings(
    app: &AppHandle,
    state: &State<'_, AppState>,
    settings: AppSettings,
) -> Result<(), CommandError> {
    settings
        .save(&state.settings_path())
        .map_err(|e| CommandError::new("CONFIG_SAVE_FAILED", &e.to_string()))?;
    *state.settings.lock_or_recover() = settings.clone();
    let _ = app.emit("settings-changed", &settings.for_webview());
    Ok(())
}

mod capture;
mod notes;
mod vault;
mod models;
mod app;
mod diagnostics;
mod account;
mod web_capture;
mod knowledge;
mod test_lab;

pub use capture::*;
pub use notes::*;
pub use vault::*;
pub use models::*;
pub use app::*;
pub use diagnostics::*;
pub use account::*;
pub use web_capture::*;
pub use knowledge::*;
pub use test_lab::*;
