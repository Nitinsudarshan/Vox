//! Account: profile, sign-in, installation identity and updates.

use super::*;

#[tauri::command]
pub async fn get_relay_profile(
    state: State<'_, AppState>,
) -> Result<crate::identity::RelayProfile, CommandError> {
    Ok(crate::identity::load_relay_profile(&state.config_dir))
}

#[tauri::command]
pub async fn update_profile_display_name(
    app: tauri::AppHandle,
    display_name: String,
    state: State<'_, AppState>,
) -> Result<crate::identity::RelayProfile, CommandError> {
    let profile = crate::identity::update_profile_display_name(&state.config_dir, &display_name)
        .map_err(|e| CommandError::new("UPDATE_NAME_FAILED", &e))?;

    let _ = app.emit("profile-changed", &profile);
    Ok(profile)
}

#[tauri::command]
pub async fn complete_profile_onboarding(
    app: tauri::AppHandle,
    display_name: String,
    account_mode: Option<crate::identity::AccountMode>,
    state: State<'_, AppState>,
) -> Result<crate::identity::RelayProfile, CommandError> {
    let profile = crate::identity::complete_profile_onboarding(
        &state.config_dir,
        &display_name,
        account_mode,
    )
    .map_err(|e| CommandError::new("ONBOARDING_FAILED", &e))?;

    // Also mark first_run_completed in settings
    if let Ok(mut settings) = state.settings.lock() {
        settings.diagnostics.first_run_completed = true;
        let _ = settings.save(&state.config_dir.join("settings.json"));
    }

    let _ = app.emit("profile-changed", &profile);
    Ok(profile)
}

#[tauri::command]
pub async fn get_developer_settings(
    state: State<'_, AppState>,
) -> Result<crate::developer::DeveloperSettings, CommandError> {
    Ok(crate::developer::load_developer_settings(&state.config_dir))
}

#[tauri::command]
pub async fn set_developer_force_onboarding(
    enabled: bool,
    state: State<'_, AppState>,
) -> Result<crate::developer::DeveloperSettings, CommandError> {
    crate::developer::set_force_onboarding(&state.config_dir, enabled)
        .map_err(|e| CommandError::new("DEV_SETTINGS_FAILED", &e))
}

#[tauri::command]
pub async fn get_account_state(
    state: State<'_, AppState>,
) -> Result<crate::identity::RelayAccount, CommandError> {
    Ok(crate::identity::load_relay_account(&state.config_dir))
}

#[tauri::command]
pub async fn start_google_sign_in(
    app: tauri::AppHandle,
    custom_client_id: Option<String>,
    custom_client_secret: Option<String>,
    state: State<'_, AppState>,
) -> Result<crate::identity::RelayAccount, CommandError> {
    let settings = state.settings.lock_or_recover().clone();
    let supabase_url = settings.cloud.supabase_url;
    let supabase_anon = settings.cloud.supabase_anon_key;

    let account = crate::identity::sign_in_with_google(
        &state.config_dir,
        custom_client_id,
        custom_client_secret,
        supabase_url,
        supabase_anon,
    )
    .await
    .map_err(|e| CommandError::new("AUTH_FAILED", &e))?;

    let _ = app.emit("account-changed", &account);

    // Report diagnostics event if enabled
    let settings = state.settings.lock_or_recover().clone();
    let inst = crate::identity::get_or_create_installation_info(
        &state.config_dir,
        env!("CARGO_PKG_VERSION"),
    );
    crate::diagnostics::DiagnosticsService::report_event(
        settings.diagnostics.allow_anonymous_diagnostics,
        &inst.installation_id,
        account.user_id.as_deref(),
        env!("CARGO_PKG_VERSION"),
        "account_sign_in",
        std::collections::HashMap::new(),
    );

    Ok(account)
}

#[tauri::command]
pub async fn sign_out_account(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
) -> Result<crate::identity::RelayAccount, CommandError> {
    let account = crate::identity::sign_out_account(&state.config_dir)
        .map_err(|e| CommandError::new("SIGNOUT_FAILED", &e))?;

    let _ = app.emit("account-changed", &account);

    let settings = state.settings.lock_or_recover().clone();
    let inst = crate::identity::get_or_create_installation_info(
        &state.config_dir,
        env!("CARGO_PKG_VERSION"),
    );
    crate::diagnostics::DiagnosticsService::report_event(
        settings.diagnostics.allow_anonymous_diagnostics,
        &inst.installation_id,
        None,
        env!("CARGO_PKG_VERSION"),
        "account_sign_out",
        std::collections::HashMap::new(),
    );

    Ok(account)
}

#[tauri::command]
pub async fn delete_relay_account(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
) -> Result<crate::identity::RelayAccount, CommandError> {
    let account = crate::identity::delete_relay_account(&state.config_dir)
        .await
        .map_err(|e| CommandError::new("DELETE_ACCOUNT_FAILED", &e))?;

    let _ = app.emit("account-changed", &account);

    let settings = state.settings.lock_or_recover().clone();
    let inst = crate::identity::get_or_create_installation_info(
        &state.config_dir,
        env!("CARGO_PKG_VERSION"),
    );
    crate::diagnostics::DiagnosticsService::report_event(
        settings.diagnostics.allow_anonymous_diagnostics,
        &inst.installation_id,
        None,
        env!("CARGO_PKG_VERSION"),
        "account_deleted",
        std::collections::HashMap::new(),
    );

    Ok(account)
}

#[tauri::command]
pub async fn get_installation_info(
    state: State<'_, AppState>,
) -> Result<crate::identity::InstallationInfo, CommandError> {
    Ok(crate::identity::get_or_create_installation_info(
        &state.config_dir,
        env!("CARGO_PKG_VERSION"),
    ))
}

#[tauri::command]
pub async fn check_for_app_updates(
    _state: State<'_, AppState>,
) -> Result<crate::updates::UpdateInfo, CommandError> {
    let current_ver = env!("CARGO_PKG_VERSION");
    Ok(crate::updates::UpdateService::check_for_updates(current_ver).await)
}

/// A plain-text summary of this installation for a support request, as
/// Settings › Diagnostics copies it to the clipboard.
///
/// Configuration only: no API key, no file path, no transcript and no note
/// content, so it is safe to paste into an issue.
#[tauri::command]
pub async fn get_diagnostic_summary(state: State<'_, AppState>) -> Result<String, CommandError> {
    let settings = state.settings.lock_or_recover().clone();
    let installation =
        crate::identity::get_or_create_installation_info(&state.config_dir, env!("CARGO_PKG_VERSION"));
    Ok(diagnostic_summary(&settings, &installation.installation_id))
}

fn diagnostic_summary(settings: &AppSettings, installation_id: &str) -> String {
    let provider = &settings.provider;
    let on_off = |on: bool| if on { "on" } else { "off" };
    [
        format!("Vox {}", env!("CARGO_PKG_VERSION")),
        format!("Platform: {} / {}", std::env::consts::OS, std::env::consts::ARCH),
        format!("Installation: {installation_id}"),
        format!(
            "Language model: {} ({})",
            provider.active_provider.slug(),
            if provider.active_provider_is_local() { "on this machine" } else { "cloud" }
        ),
        format!(
            "Dictation: engine {}, quality {:?}, cleanup {}",
            settings.stt.dictation_engine.as_deref().unwrap_or("whisper"),
            settings.stt.dictation_quality,
            crate::capture::rewrite::CleanupStyle::from_setting(&settings.stt.cleanup_style).as_str()
        ),
        format!(
            "Meeting model: {}",
            settings.stt.meeting_model_id.as_deref().unwrap_or("best installed")
        ),
        format!("Web capture: {}", on_off(settings.capture.bridge_enabled)),
        format!(
            "Anonymous crash reports: {}",
            on_off(settings.diagnostics.allow_anonymous_diagnostics)
        ),
    ]
    .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_diagnostic_summary_carries_no_secret_or_path() {
        let mut settings = AppSettings::default();
        settings.provider.provider_keys.insert("groq".into(), "gsk-secret".into());
        settings.vault.directory = Some("C:/Users/asha/Vox".into());
        let summary = diagnostic_summary(&settings, "inst-123");

        assert!(summary.contains("Installation: inst-123"));
        assert!(summary.contains("cleanup raw"));
        assert!(!summary.contains("gsk-secret"));
        assert!(!summary.contains("asha"));
    }
}
