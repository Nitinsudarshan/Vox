//! App: settings, windows, the changelog, and first-run state.

use super::*;

#[tauri::command]
pub async fn copy_to_clipboard(text: String) -> Result<(), CommandError> {
    crate::hotkeys::injection::copy_to_clipboard(&text)
        .map_err(|e| CommandError::new("CLIPBOARD_COPY_FAILED", &e.to_string()))
}

#[tauri::command]
pub async fn get_settings(state: State<'_, AppState>) -> Result<AppSettings, CommandError> {
    Ok(state.settings.lock_or_recover().for_webview())
}

#[tauri::command]
pub async fn save_settings(
    app: AppHandle,
    settings: AppSettings,
    state: State<'_, AppState>,
) -> Result<(), CommandError> {
    // Capture is configured through its own commands, never through this one.
    // Carrying the stored section over means a settings object that predates
    // it — or one from a frontend that never read it — cannot switch capture
    // off and throw away the pairing token as a side effect.
    let stored_capture = state.settings.lock_or_recover().capture.clone();
    let mut settings = settings.preserving_capture(&stored_capture);

    // The webview only ever holds placeholders for stored API keys (see
    // `providers::secrets`); one sent back unchanged means "keep that key".
    let stored_provider = state.settings.lock_or_recover().provider.clone();
    settings.provider.resolve_placeholders(&stored_provider);

    // If incoming settings has no vault directory set, preserve the stored one
    let stored_vault = state.settings.lock_or_recover().vault.clone();
    if settings.vault.directory.is_none() && stored_vault.directory.is_some() {
        settings.vault.directory = stored_vault.directory.clone();
    }
    let vault_moved = settings.vault.directory != stored_vault.directory;
    if vault_moved {
        state.ensure_vault_can_move()?;
    }

    settings
        .save(&state.settings_path())
        .map_err(|e| CommandError::new("CONFIG_SAVE_FAILED", &e.to_string()))?;

    // Repointed only after the save succeeded, so a failed write cannot leave
    // the running app on a vault the settings file does not name.
    if vault_moved {
        if let Some(ref dir) = settings.vault.directory {
            state.repoint_vault(&app, std::path::Path::new(dir));
        }
    }
    state
        .recorder
        .set_keep_warm_duration(settings.audio_input.parse_keep_warm_duration());
    crate::capture::device::set_preference(&settings.audio_input);
    *state.settings.lock_or_recover() = settings.clone();

    // Re-register hotkeys dynamically with the OS immediately
    let _ = hotkeys::apply_hotkeys(
        &app,
        &settings.hotkeys.show_hide_hotkey,
        &settings.hotkeys.dictation_hotkey,
        &settings.hotkeys.capture_hotkey,
    );

    // The bridge's lifetime follows the setting: turning capture off closes
    // the socket immediately rather than at the next launch.
    apply_capture_bridge(&app, &state);

    // Same reason, for the OS launch entry. `start_minimized` has no
    // counterpart here on purpose — it is a decision taken at launch, and the
    // switch says so.
    crate::startup::reconcile_launch_at_login(&app, settings.startup.launch_at_login);

    if let Some(window) = app.get_webview_window(crate::overlay::MEETING_OVERLAY_LABEL) {
        crate::overlay::reposition_meeting_overlay(&app, &window);
    }

    let _ = app.emit("settings-changed", &settings.for_webview());
    Ok(())
}

#[tauri::command]
pub async fn open_settings_window(
    app: AppHandle,
    section: Option<String>,
) -> Result<(), CommandError> {
    if let Some(window) = app.get_webview_window(crate::hotkeys::MAIN_WINDOW_LABEL) {
        let _ = window.show();
        let _ = window.set_focus();
        let payload = serde_json::json!({
            "tab": "settings",
            "section": section,
        });
        let _ = app.emit("navigate-tab", payload);
    }
    Ok(())
}

/// Minimizes the main window to the OS taskbar without terminating.
#[tauri::command]
pub async fn minimize_main_window(app: AppHandle) -> Result<(), CommandError> {
    if let Some(window) = app.get_webview_window(crate::hotkeys::MAIN_WINDOW_LABEL) {
        let _ = window.minimize();
    }
    Ok(())
}

/// Hides the main window while keeping Vox running in the background.
#[tauri::command]
pub async fn hide_main_window(app: AppHandle) -> Result<(), CommandError> {
    if let Some(window) = app.get_webview_window(crate::hotkeys::MAIN_WINDOW_LABEL) {
        let _ = window.hide();
    }
    Ok(())
}

/// Fully terminates the Vox desktop application.
#[tauri::command]
pub async fn close_app(app: AppHandle) -> Result<(), CommandError> {
    app.exit(0);
    Ok(())
}

/// Unminimizes, shows, and focuses the main Vox window.
#[tauri::command]
pub async fn show_main_window(app: AppHandle) -> Result<(), CommandError> {
    if let Some(window) = app.get_webview_window(crate::hotkeys::MAIN_WINDOW_LABEL) {
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
    }
    Ok(())
}

/// Toggles maximize state of the main window.
#[tauri::command]
pub async fn toggle_maximize_main_window(app: AppHandle) -> Result<bool, CommandError> {
    if let Some(window) = app.get_webview_window(crate::hotkeys::MAIN_WINDOW_LABEL) {
        if window.is_maximized().unwrap_or(false) {
            let _ = window.unmaximize();
            return Ok(false);
        } else {
            let _ = window.maximize();
            return Ok(true);
        }
    }
    Ok(false)
}

/// Checks if the main window is currently maximized.
#[tauri::command]
pub async fn is_main_window_maximized(app: AppHandle) -> Result<bool, CommandError> {
    if let Some(window) = app.get_webview_window(crate::hotkeys::MAIN_WINDOW_LABEL) {
        return Ok(window.is_maximized().unwrap_or(false));
    }
    Ok(false)
}

/// Updates the taskbar and tray icon based on theme or automatic Windows taskbar detection.
#[tauri::command]
pub async fn update_taskbar_theme_icon(
    app: AppHandle,
    is_dark_taskbar: Option<bool>,
) -> Result<bool, CommandError> {
    let dark = is_dark_taskbar.unwrap_or_else(crate::theme_icon::is_system_taskbar_dark);
    crate::theme_icon::apply_taskbar_icon(&app, dark);
    Ok(dark)
}


#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChangelogItem {
    pub category: String,
    pub domain: String,
    pub text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChangelogEntry {
    pub version: String,
    pub date: String,
    pub release_type: String,
    pub title: String,
    pub tags: Vec<String>,
    pub domains: Vec<String>,
    pub items: Vec<ChangelogItem>,
}

#[tauri::command]
pub async fn get_app_version() -> Result<String, CommandError> {
    let version = include_str!("../../../../VERSION").trim().to_string();
    Ok(version)
}

#[tauri::command]
pub async fn get_changelog() -> Result<Vec<ChangelogEntry>, CommandError> {
    let raw = include_str!("../../../../CHANGELOG.md");
    Ok(parse_changelog_markdown(raw))
}

fn parse_changelog_markdown(md: &str) -> Vec<ChangelogEntry> {
    let mut entries = Vec::new();
    let mut current_entry: Option<ChangelogEntry> = None;
    let mut current_item: Option<ChangelogItem> = None;

    for line in md.lines() {
        let trimmed = line.trim();

        if let Some(rest) = trimmed.strip_prefix("## [") {
            if let Some(mut item) = current_item.take() {
                if let Some(entry) = current_entry.as_mut() {
                    item.text = item.text.trim().to_string();
                    if !item.text.is_empty() {
                        entry.items.push(item);
                    }
                }
            }
            if let Some(entry) = current_entry.take() {
                entries.push(entry);
            }

            let (ver, date) = if let Some(close_idx) = rest.find(']') {
                let v = &rest[..close_idx];
                let d = if let Some(dash_idx) = rest.find(" - ") {
                    rest[dash_idx + 3..].trim()
                } else {
                    ""
                };
                (v, d)
            } else {
                ("", "")
            };

            let release_type = if ver.ends_with(".0.0") || ver == "0.1.0" {
                "major"
            } else if ver.ends_with(".0") {
                "minor"
            } else {
                "patch"
            };

            current_entry = Some(ChangelogEntry {
                version: ver.to_string(),
                date: date.to_string(),
                release_type: release_type.to_string(),
                title: String::new(),
                tags: Vec::new(),
                domains: Vec::new(),
                items: Vec::new(),
            });
            continue;
        }

        if let Some(heading) = trimmed.strip_prefix("### ") {
            if let Some(entry) = current_entry.as_mut() {
                if entry.title.is_empty() {
                    entry.title = heading.trim().to_string();
                }
            }
            continue;
        }

        if trimmed.starts_with("- ") || trimmed.starts_with("* ") {
            if let Some(mut item) = current_item.take() {
                if let Some(entry) = current_entry.as_mut() {
                    item.text = item.text.trim().to_string();
                    if !item.text.is_empty() {
                        entry.items.push(item);
                    }
                }
            }

            let bullet_content = trimmed[2..].trim();
            let (category, domain, text) = parse_bullet_line(bullet_content);

            if let Some(entry) = current_entry.as_mut() {
                if !category.is_empty() && !entry.tags.contains(&category) {
                    entry.tags.push(category.clone());
                }
                if !domain.is_empty() && !entry.domains.contains(&domain) {
                    entry.domains.push(domain.clone());
                }
            }

            current_item = Some(ChangelogItem {
                category,
                domain,
                text,
            });
            continue;
        }

        if let Some(item) = current_item.as_mut() {
            if !trimmed.is_empty() && !trimmed.starts_with('#') {
                item.text.push(' ');
                item.text.push_str(trimmed);
            }
        }
    }

    if let Some(mut item) = current_item.take() {
        if let Some(entry) = current_entry.as_mut() {
            item.text = item.text.trim().to_string();
            if !item.text.is_empty() {
                entry.items.push(item);
            }
        }
    }
    if let Some(entry) = current_entry.take() {
        entries.push(entry);
    }

    entries
}

fn parse_bullet_line(content: &str) -> (String, String, String) {
    if let Some(after_open) = content.strip_prefix("**") {
        if let Some(end_bold) = after_open.find("**") {
            let bold_text = &after_open[..end_bold];
            let after_bold = &after_open[end_bold + 2..];
            let text = after_bold.trim_start_matches(':').trim().to_string();

            if let Some(open_p) = bold_text.find('(') {
                if let Some(close_p) = bold_text.find(')') {
                    let cat = bold_text[..open_p].trim().to_string();
                    let dom_raw = bold_text[open_p + 1..close_p].trim().trim_matches('`');
                    let dom = if dom_raw.contains('/') || dom_raw.contains('\\') {
                        dom_raw.rsplit_once(['/', '\\']).map(|(_, f)| f).unwrap_or(dom_raw)
                    } else {
                        dom_raw
                    };
                    return (cat, dom.to_string(), text);
                }
            }

            return (bold_text.to_string(), "Core".to_string(), text);
        }
    }

    ("General".to_string(), "Vox".to_string(), content.to_string())
}

#[tauri::command]
pub async fn set_diagnostics_consent(
    enabled: bool,
    state: State<'_, AppState>,
) -> Result<AppSettings, CommandError> {
    let mut settings = state.settings.lock_or_recover();
    settings.diagnostics.allow_anonymous_diagnostics = enabled;
    settings
        .save(&state.config_dir.join("settings.json"))
        .map_err(|e| CommandError::new("SAVE_FAILED", &e.to_string()))?;
    Ok(settings.for_webview())
}

#[tauri::command]
pub async fn complete_first_run(
    state: State<'_, AppState>,
) -> Result<AppSettings, CommandError> {
    let mut settings = state.settings.lock_or_recover();
    settings.diagnostics.first_run_completed = true;
    settings
        .save(&state.config_dir.join("settings.json"))
        .map_err(|e| CommandError::new("SAVE_FAILED", &e.to_string()))?;
    Ok(settings.for_webview())
}
