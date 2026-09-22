pub mod actions;
pub mod calendar;
pub mod capture;
pub mod commands;
pub mod context;
pub mod conversation;
pub mod developer;
pub mod diagnostics;
pub mod entities;
pub mod hotkeys;
pub mod identity;
pub mod mcp;
pub mod meetings;
pub mod memory;
pub mod oauth;
pub mod overlay;
pub mod pipeline;
pub mod providers;
pub mod relationships;
pub mod retrieval;
pub mod settings;
pub mod startup;
pub mod sync;
pub mod theme_icon;
pub mod triggers;
pub mod tts;
pub mod updates;
pub mod vault;

use capture::{AudioRecorder, SttEngine};
use commands::AppState;
use settings::AppSettings;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use tauri::Manager;
use vault::VaultManager;

#[cfg(target_os = "windows")]
fn set_app_user_model_id() {
    use std::ffi::OsStr;
    use std::os::windows::ffi::OsStrExt;

    let app_id: Vec<u16> = OsStr::new("com.vox.app")
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();

    unsafe {
        #[link(name = "shell32")]
        extern "system" {
            fn SetCurrentProcessExplicitAppUserModelID(AppID: *const u16) -> i32;
        }
        let _ = SetCurrentProcessExplicitAppUserModelID(app_id.as_ptr());
    }
}

fn resolve_base_dir() -> PathBuf {
    // 1. In debug/development, prioritize CARGO_MANIFEST_DIR/.vox
    #[cfg(debug_assertions)]
    {
        let manifest_vox = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(".vox");
        if manifest_vox.exists() {
            return manifest_vox;
        }
        let manifest_relay = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(".relay");
        if manifest_relay.exists() {
            return manifest_relay;
        }
    }

    // 2. Search CWD and its ancestors
    if let Ok(mut dir) = std::env::current_dir() {
        loop {
            let candidates = [
                dir.join("native").join("src-tauri").join(".vox"),
                dir.join("src-tauri").join(".vox"),
                dir.join(".vox"),
                dir.join("native").join("src-tauri").join(".relay"),
                dir.join("src-tauri").join(".relay"),
                dir.join(".relay"),
            ];

            for candidate in &candidates {
                if candidate.exists() {
                    return candidate.clone();
                }
            }

            if !dir.pop() {
                break;
            }
        }
    }

    // 3. Fallback to process cwd or manifest dir
    #[cfg(debug_assertions)]
    {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(".vox")
    }
    #[cfg(not(debug_assertions))]
    {
        let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        if cwd.join(".relay").exists() && !cwd.join(".vox").exists() {
            cwd.join(".relay")
        } else {
            cwd.join(".vox")
        }
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    #[cfg(target_os = "windows")]
    set_app_user_model_id();

    // The ONNX packaging spike, when this binary was built with it. First
    // thing, before any window or plugin, because what it is measuring is
    // whether the ONNX Runtime this executable was *shipped with* can be
    // loaded at all — and that answer must survive anything else failing.
    #[cfg(feature = "onnx-spike")]
    developer::onnx_spike::run_and_record();
    // In development, pick up the repo root's `.env` when cargo ran from
    // `native/src-tauri/` — otherwise `CARGO_MANIFEST_DIR`-relative assets
    // find their config while runtime environment variables go missing.
    #[cfg(debug_assertions)]
    {
        if let Ok(mut dir) = std::env::current_dir() {
            loop {
                let candidate = dir.join(".env");
                if candidate.exists() {
                    let _ = dotenvy::from_filename_override(candidate);
                    break;
                }
                if !dir.pop() {
                    break;
                }
            }
        }
    }

    let base_dir = resolve_base_dir();

    let default_vault_dir = base_dir.join("vault");
    let config_dir = base_dir.join("config");

    let settings_path = config_dir.join("settings.json");
    let mut settings = AppSettings::load(&settings_path).unwrap_or_default();
    // Written back immediately so the correction survives a crash before the
    // next ordinary save, and so the marker is never applied twice.
    if settings.meetings.reminders.migrate() {
        if let Err(err) = settings.save(&settings_path) {
            tracing::warn!("[reminders] could not persist the settings correction: {}", err);
        }
    }
    let settings = settings;
    let hotkeys_config = settings.hotkeys.clone();
    let pill_position = settings.ui.pill_position;
    let startup_config = settings.startup.clone();

    // An explicitly configured Vault Directory Location always wins; a
    // fresh install (or one where the user never confirmed a location)
    // keeps using the same process-relative default this already used
    // before Voice Notes existed, so existing notes/Kanban cards never
    // silently move.
    let vault_dir = settings
        .vault
        .directory
        .as_ref()
        .map(PathBuf::from)
        .unwrap_or_else(|| default_vault_dir.clone());

    // Route whisper.cpp/GGML's own logging through whisper-rs's hooks. Without
    // this, every decode dumps its full token-by-token trace to the terminal,
    // which at the live clock's cadence buries everything else.
    #[cfg(feature = "whisper-local")]
    whisper_rs::install_logging_hooks();

    let stt = SttEngine::new();
    let recorder = AudioRecorder::new();
    recorder.set_keep_warm_duration(settings.audio_input.parse_keep_warm_duration());
    crate::capture::device::set_preference(&settings.audio_input);

    let memory_store = Arc::new(memory::MemoryStore::new(&vault_dir));
    let relationship_store = Arc::new(relationships::RelationshipStore::new(&vault_dir));
    let entity_store = Arc::new(entities::EntityStore::new(&vault_dir));

    let meeting_store = Arc::new(meetings::MeetingStore::new(&vault_dir));
    let meeting_engine = Arc::new(meetings::engine::MeetingEngine::new(Arc::clone(&meeting_store)));
    let summary_service = Arc::new(meetings::summary::service::SummaryService::new(Arc::clone(
        &meeting_store,
    )));

    let state = AppState {
        recorder,
        vault: VaultManager::new(vault_dir),
        default_vault_dir,
        config_dir,
        settings: Mutex::new(settings),
        stt,
        last_stt_diagnostics: Mutex::new(None),
        last_dictation: Mutex::new(None),
        capture_bridge: Mutex::new(None),
        memory_store,
        relationship_store,
        entity_store,
        meeting_store,
        meeting_engine: Arc::clone(&meeting_engine),
        summary_service,
        meeting_imports: Arc::new(meetings::commands::ImportRegistry::default()),
    };

    tauri::Builder::default()
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_opener::init())
        // Writes the OS launch entry for Settings › Startup › "Launch at
        // login". Registered here; driven from `startup::reconcile_launch_at_login`,
        // never from the frontend — the plugin's JS commands are deliberately
        // not granted in `capabilities/`.
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            None,
        ))
        .manage(std::sync::Arc::new(calendar::reminders::ReminderQueue::default()))
        .manage(std::sync::Arc::new(calendar::reminders::NotificationService::new()))
        .manage(state)
        .setup(move |app| {
            let handle = app.handle();

            // Prefer an Ollama that Vox installed over whatever is on PATH.
            // Set once here rather than looked up per call: the capture path,
            // the summary path and a settings command all ask "which binary",
            // and three lookups is three places for the answer to differ.
            {
                let state = app.state::<AppState>();
                providers::set_managed_binary(providers::ollama_install::managed_binary(
                    &state.config_dir,
                ));
            }

            // Let the webview read a meeting's recording, and nothing else.
            //
            // The player in Meeting Detail needs to seek around a file that can
            // be hundreds of megabytes, so it has to be served rather than
            // handed over IPC. The scope is granted here at runtime, against
            // the vault's `meetings/` directory, because that path is a user
            // setting and cannot be a static entry in `tauri.conf.json`.
            //
            // Deliberately narrow. Meetily grants its window `fs:read-all` and
            // `fs:write-all` alongside a nominally scoped `$APPDATA/*`, which
            // makes the scope decorative; anything running in that webview can
            // read the user's disk. One directory is the whole of what this
            // feature needs.
            //
            // TODO(vault-relocation): `set_vault_dir` repoints `state.vault`
            // but not `meeting_store`, so moving the vault already leaves
            // meetings reading the old location (`commands.rs`, set_vault_path).
            // This scope inherits that staleness. Fixing the store's repoint is
            // what fixes both.
            {
                let state = app.state::<AppState>();
                let meetings_dir = state.meeting_store.meetings_dir();
                if let Err(error) = app
                    .asset_protocol_scope()
                    .allow_directory(&meetings_dir, true)
                {
                    tracing::warn!(
                        "meeting audio playback unavailable: could not allow {}: {}",
                        meetings_dir.display(),
                        error
                    );
                }
            }

            // First, and before anything that can fail: the main window is
            // configured hidden so "start minimized" does not flash the
            // control panel on screen, which means *something* has to show it.
            // A tray builder that errors below must not be what decides
            // whether Vox has a window.
            startup::apply_at_launch(handle, &startup_config);

            let quit_i = tauri::menu::MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
            let show_i = tauri::menu::MenuItem::with_id(app, "show", "Show Vox", true, None::<&str>)?;
            let record_i = tauri::menu::MenuItem::with_id(app, "record", "Start Recording", true, None::<&str>)?;
            
            let menu = tauri::menu::Menu::with_items(app, &[&show_i, &record_i, &quit_i])?;
            
            if let Some(window) = app.get_webview_window("main") {
                if let Some(icon) = app.default_window_icon() {
                    let _ = window.set_icon(icon.clone());
                }
            }

            let mut tray_builder = tauri::tray::TrayIconBuilder::with_id(crate::theme_icon::TRAY_ID).menu(&menu);
            if let Some(icon) = app.default_window_icon() {
                tray_builder = tray_builder.icon(icon.clone());
            }
            let _tray = tray_builder
                .on_menu_event(|app, event| match event.id.as_ref() {
                    "quit" => {
                        app.exit(0);
                    }
                    "show" => {
                        if let Some(window) = app.get_webview_window("main") {
                            let _ = window.unminimize();
                            let _ = window.show();
                            let _ = window.set_focus();
                        }
                    }
                    "record" => {
                        if let Some(window) = app.get_webview_window("main") {
                            let _ = window.unminimize();
                            let _ = window.show();
                            let _ = window.set_focus();
                        }
                    }
                    _ => {}
                })
                .build(app)?;

            // Apply theme-matching taskbar and window icon (light icon on dark taskbar, dark icon on light taskbar)
            crate::theme_icon::apply_taskbar_icon(handle, crate::theme_icon::is_system_taskbar_dark());

            hotkeys::register_hotkeys(
                handle,
                &hotkeys_config.show_hide_hotkey,
                &hotkeys_config.dictation_hotkey,
                &hotkeys_config.capture_hotkey,
            );

            // Opens the loopback capture listener only when the user has
            // switched capture on; a fresh install opens no socket.
            commands::apply_capture_bridge(handle, &handle.state::<commands::AppState>());
            // The dictation pill is now the one, permanent PTT surface — no
            // more docked/floating product-mode choice to hide it behind
            // (see docs/decisions.md Decision 36) — so it's always shown.
            overlay::ensure_pill_window(handle, true, pill_position);

            // Finalize any meeting a crash left mid-recording. Its audio
            // checkpoints and flushed transcript are on disk; without this it
            // would sit in the list forever claiming to be recording.
            let recovery_engine = Arc::clone(&meeting_engine);
            std::thread::spawn(move || {
                let recovered = recovery_engine.recover_interrupted();
                if !recovered.is_empty() {
                    tracing::info!("recovered {} interrupted meeting(s)", recovered.len());
                }
            });

            // Says something when a meeting is about to start. Polls rather
            // than holding a timer per event: a timer array has to be rebuilt
            // on every sync, settings change and clock change, and the failure
            // mode of getting that wrong is a reminder that silently never
            // fires.
            // Both overlays are built hidden now rather than on demand:
            // creating a webview is slow enough to be seen, and a reminder
            // that arrives a beat late is a reminder about a meeting that has
            // already started.
            overlay::ensure_reminder_window(handle);
            overlay::ensure_meeting_overlay(handle, false);
            calendar::reminders::scheduler::spawn(handle.clone());

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::start_capture,
            commands::stop_capture,
            commands::get_capture_status,
            commands::update_hotkeys,
            commands::set_pill_position,
            commands::set_pill_expanded,
            commands::set_pill_window_mode,
            commands::ensure_local_llm_ready,
            commands::get_available_llm_models,
            commands::get_ollama_install_plan,
            commands::install_ollama,
            commands::test_llm_prompt,
            commands::ensure_stt_model_ready,
            commands::get_stt_decode_summary,
            commands::rewrite_dictation,
            commands::get_cleanup_target,
            commands::apply_dictation_cleanup,
            commands::get_available_stt_models,
            commands::download_stt_model,
            commands::test_stt_model,
            commands::list_speech_models,
            commands::download_speech_model,
            commands::cancel_speech_model_download,
            commands::delete_speech_model,
            commands::set_meeting_speech_model,
            commands::set_dictation_speech_model,
            commands::get_parakeet_status,
            commands::download_parakeet_model,
            commands::delete_parakeet_model,
            commands::set_dictation_engine,
            commands::copy_to_clipboard,
            commands::get_kanban_cards,
            commands::create_manual_todo,
            commands::set_todo_status,
            commands::delete_todo,
            commands::get_triggers,
            commands::save_triggers,
            commands::get_audio_devices,
            commands::get_audio_output_devices,
            commands::get_settings,
            commands::save_settings,
            commands::open_settings_window,
            commands::minimize_main_window,
            commands::toggle_maximize_main_window,
            commands::is_main_window_maximized,
            commands::update_taskbar_theme_icon,
            commands::hide_main_window,
            commands::close_app,
            commands::show_main_window,
            commands::get_voice_notes,
            commands::update_voice_note,
            commands::correct_voice_note_phrase,
            commands::undo_voice_note_correction,
            commands::add_dictionary_word,
            commands::delete_voice_note,
            commands::delete_voice_notes,
            commands::merge_voice_notes,
            commands::merge_multiple_voice_notes,
            commands::unmerge_voice_note,
            commands::get_vault_location,
            commands::choose_vault_folder,
            commands::set_vault_location,
            commands::get_app_version,
            commands::get_changelog,
            commands::diagnose_stt_variants,
            commands::run_stt_evaluation,
            commands::get_last_stt_diagnostics,
            commands::get_stt_corpus,
            commands::get_scribbles,
            commands::get_scribble,
            commands::create_scribble,
            commands::promote_voice_note_to_scribble,
            commands::create_file_scribble,
            commands::update_scribble,
            commands::set_scribble_para,
            commands::delete_scribble,
            commands::merge_scribbles,

            commands::get_trash_items,
            commands::restore_trash_item,
            commands::delete_trash_item_permanently,
            commands::empty_trash,
            commands::add_scribble_relationship,
            commands::remove_scribble_relationship,
            commands::search_knowledge,
            commands::get_knowledge_graph,
            commands::trigger_enrich_scribble,
            commands::summarize_scribble,
            commands::get_relay_profile,
            commands::update_profile_display_name,
            commands::complete_profile_onboarding,
            commands::get_developer_settings,
            commands::set_developer_force_onboarding,
            commands::get_account_state,
            commands::start_google_sign_in,
            commands::sign_out_account,
            commands::delete_relay_account,
            commands::get_installation_info,
            commands::check_for_app_updates,
            commands::set_diagnostics_consent,
            commands::complete_first_run,
            commands::import_vault_file,
            commands::import_vault_file_bytes,
            commands::analyze_vault_file,
            commands::get_vault_files,
            commands::get_vault_file,
            commands::summarize_vault_file,
            commands::enrich_vault_file,
            commands::update_vault_file_tags,
            commands::create_scribble_from_vault_file,
            commands::reprocess_vault_file,
            commands::delete_vault_file,
            commands::open_vault_file_location,
            commands::get_capture_bridge_status,
            commands::set_capture_bridge_enabled,
            commands::set_capture_bridge_port,
            commands::set_capture_analyze_on_capture,
            commands::regenerate_capture_pairing_token,
            commands::get_captures,
            commands::get_capture,
            commands::get_capture_payload,
            commands::renormalize_capture,
            commands::delete_capture,
            commands::import_web_capture,
            commands::get_capture_context,
            commands::analyze_capture_context,
            commands::inspect_ai_conversation_export,
            commands::inspect_ai_conversation_export_bytes,
            commands::import_ai_conversation_export,
            commands::import_ai_conversation_export_bytes,
            commands::pick_ai_conversation_export_file,
            commands::open_external_url,
            commands::unified_retrieve,
            commands::assemble_context_pack,
            commands::get_knowledge_telemetry,
            commands::form_memory_candidate,
            commands::list_entities,
            commands::list_memories,
            commands::list_decisions,
            commands::record_decision,
            commands::confirm_decision,
            commands::supersede_decision,
            commands::create_memory,
            commands::supersede_memory,
            commands::extract_and_resolve_entities,
            commands::dispatch_universal_action,
            commands::list_relationships,
            commands::add_relationship,
            commands::get_available_test_models,
            commands::get_available_cleanup_styles,
            commands::start_dictation_test_recording,
            commands::stop_dictation_test_recording,
            commands::run_dictation_test_on_audio,
            commands::get_dictation_test_history,
            commands::get_dictation_test_run,
            commands::delete_dictation_test_run,
            commands::export_dictation_test_report,
            commands::inject_dictation_test_result,
            meetings::commands::start_meeting,
            meetings::commands::pause_meeting,
            meetings::commands::resume_meeting,
            meetings::commands::stop_meeting,
            meetings::commands::get_meeting_recording_status,
            meetings::commands::get_meeting_devices,
            meetings::commands::set_meeting_devices,
            meetings::commands::list_meetings,
            meetings::commands::get_meeting,
            meetings::commands::search_meetings,
            meetings::commands::rename_meeting,
            meetings::commands::save_meeting_notes,
            meetings::commands::delete_meeting,
            meetings::commands::open_meeting_folder,
            meetings::commands::list_meeting_templates,
            meetings::commands::generate_meeting_summary,
            meetings::commands::cancel_meeting_summary,
            meetings::commands::get_meeting_summary,
            meetings::commands::save_meeting_summary,
            meetings::commands::promote_meeting_to_scribble,
            meetings::commands::meeting_audio_extensions,
            meetings::commands::pick_meeting_audio_file,
            meetings::commands::import_meeting_audio,
            meetings::commands::retranscribe_meeting,
            meetings::commands::translate_meeting_transcript,
            meetings::commands::romanize_meeting_transcript,
            meetings::commands::generate_meeting_english_track,
            calendar::commands::list_calendar_accounts,
            calendar::commands::connect_calendar_account,
            calendar::commands::disconnect_calendar_account,
            calendar::commands::set_calendar_account_enabled,
            calendar::commands::list_account_calendars,
            calendar::commands::set_account_calendars,
            calendar::commands::sync_calendars,
            calendar::commands::get_calendar_agenda,
            calendar::commands::open_calendar_link,
            calendar::commands::get_pending_meeting_reminder,
            calendar::commands::meeting_reminder_ready,
            calendar::commands::meeting_reminder_hover_changed,
            calendar::commands::dismiss_meeting_reminder,
            calendar::commands::snooze_meeting_reminder,
            calendar::commands::join_meeting_from_reminder,
            calendar::commands::start_meeting_from_reminder,
            calendar::commands::trigger_mock_meeting_reminder,
            calendar::commands::debug_detect_conferencing_windows,
            calendar::commands::set_meeting_overlay_expanded,
            meetings::commands::list_meeting_series,
            meetings::commands::get_meeting_series,
            meetings::commands::create_meeting_series,
            meetings::commands::rename_meeting_series,
            meetings::commands::delete_meeting_series,
            meetings::commands::set_meeting_series,
            meetings::commands::detect_meeting_speakers,
            meetings::commands::rename_meeting_speaker,
            meetings::commands::cancel_meeting_import,
            meetings::commands::get_meeting_segment_diagnostics,
            meetings::commands::speech_benchmark_template,
            meetings::commands::inspect_speech_benchmark,
            meetings::commands::run_speech_benchmark,
            meetings::commands::cancel_speech_benchmark,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
