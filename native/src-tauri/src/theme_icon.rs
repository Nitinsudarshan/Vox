use tauri::{AppHandle, Manager};

const ICON_DARK_BYTES: &[u8] = include_bytes!("../icons/vox_dark.png");
const ICON_LIGHT_BYTES: &[u8] = include_bytes!("../icons/vox_light.png");

pub const TRAY_ID: &str = "main-tray";

/// Returns true if the Windows taskbar / system is using a dark theme.
pub fn is_system_taskbar_dark() -> bool {
    #[cfg(target_os = "windows")]
    {
        // On Windows 10/11:
        // HKCU\Software\Microsoft\Windows\CurrentVersion\Themes\Personalize\SystemUsesLightTheme
        // 0 = Dark taskbar (needs light/white icon)
        // 1 = Light taskbar (needs dark/black icon)
        let output = std::process::Command::new("reg")
            .args([
                "query",
                "HKCU\\Software\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize",
                "/v",
                "SystemUsesLightTheme",
            ])
            .output();

        if let Ok(out) = output {
            let text = String::from_utf8_lossy(&out.stdout);
            if text.contains("0x0") {
                return true;
            } else if text.contains("0x1") {
                return false;
            }
        }
    }

    // Default fallback: dark taskbar
    true
}

/// Applies the appropriate icon (light circle on dark taskbar, dark circle on light taskbar)
/// to both the main window and the system tray.
pub fn apply_taskbar_icon(app: &AppHandle, is_dark_taskbar: bool) {
    let bytes = if is_dark_taskbar {
        ICON_DARK_BYTES
    } else {
        ICON_LIGHT_BYTES
    };

    if let Ok(icon) = tauri::image::Image::from_bytes(bytes) {
        if let Some(window) = app.get_webview_window(crate::hotkeys::MAIN_WINDOW_LABEL) {
            let _ = window.set_icon(icon.clone());
        }
        if let Some(tray) = app.tray_by_id(TRAY_ID) {
            let _ = tray.set_icon(Some(icon));
        }
    }
}
