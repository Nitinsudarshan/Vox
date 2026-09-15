//! Which conferencing app is on screen right now.
//!
//! The calendar knows what was *scheduled*; only the desktop knows what is
//! actually happening. An ad-hoc Meet somebody pulled you into has no calendar
//! entry at all, and a scheduled call the user quietly left has one that says
//! it is still running. Enumerating visible top-level windows is what tells the
//! two apart.
//!
//! Pure signal, no side effects: this reports what it sees and nothing else
//! decides anything here. `super::recompute` weighs it against the calendar.
//!
//! Windows only. Every other platform returns an empty list, which degrades to
//! calendar-only reminders rather than to an error — the same shape the rest of
//! the meetings pipeline uses for platform-specific capture.

#[cfg(target_os = "windows")]
use std::sync::Mutex;

pub const PROVIDER_GOOGLE_MEET: &str = "google_meet";
pub const PROVIDER_ZOOM: &str = "zoom";
pub const PROVIDER_TEAMS: &str = "teams";
pub const PROVIDER_WEBEX: &str = "webex";
pub const PROVIDER_OTHER: &str = "other";

/// One open, visible top-level window whose title matched a conferencing app.
///
/// `confidence` reflects how specific the matched title is. A bare "Zoom
/// Meeting" or "Meet - Google Meet" carries no topic and is as likely to be an
/// idle app window or a stray browser tab as a live call; a title carrying a
/// real meeting name is much stronger evidence.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct WindowMatch {
    pub provider: String,
    pub title: String,
    pub raw_title: String,
    pub source: String,
    pub confidence: f32,
}

/// Dashes that turn up between an app's name and a meeting's in a window title.
///
/// Google Meet ships an en dash, not the ASCII hyphen everything here used to
/// match on, so "Meet – abc-defg-hij" looked like no provider at all and a live
/// Meet call registered as nothing on screen. The whole `detected` reminder
/// rests on these titles, so it matches every dash a title might use rather
/// than the one that happened to be tested.
const DASHES: [&str; 3] = ["-", "\u{2013}", "\u{2014}"];

/// A lowercase copy with every dash folded to an ASCII hyphen, so one pattern
/// matches all of them.
fn fold_dashes(text: &str) -> String {
    let mut folded = text.to_lowercase();
    for dash in &DASHES[1..] {
        folded = folded.replace(dash, "-");
    }
    folded
}

/// `title` without a trailing ` <dash> <tail>`, whichever dash it uses.
fn strip_dashed_suffix(title: &str, tail: &str) -> Option<String> {
    DASHES.iter().find_map(|dash| {
        title
            .strip_suffix(&format!(" {dash} {tail}"))
            .map(|rest| rest.trim().to_string())
    })
}

/// `title` without a leading `<head> <dash> `, whichever dash it uses.
fn strip_dashed_prefix(title: &str, head: &str) -> Option<String> {
    DASHES.iter().find_map(|dash| {
        title
            .strip_prefix(&format!("{head} {dash} "))
            .map(|rest| rest.trim().to_string())
    })
}

/// A window title's provider, if any — the same matching the enumerator uses,
/// exposed so a title from anywhere else can be classified the same way.
pub fn identify_meeting_provider(text: &str) -> Option<&'static str> {
    let lower = fold_dashes(text);
    if lower.contains("zoom meeting") || (lower.contains("zoom") && lower.contains("meeting id")) {
        Some(PROVIDER_ZOOM)
    } else if lower.contains("meet - ")
        || lower.contains(" - google meet")
        || lower.contains("meet.google.com")
    {
        Some(PROVIDER_GOOGLE_MEET)
    } else if lower.contains("microsoft teams meeting")
        || (lower.contains(" | microsoft teams") && !lower.starts_with("chat"))
    {
        Some(PROVIDER_TEAMS)
    } else if lower.contains("cisco webex meeting")
        || lower.contains("webex meeting")
        || lower.contains(" - cisco webex")
    {
        Some(PROVIDER_WEBEX)
    } else {
        None
    }
}

/// How a provider's name should read in a notification.
pub fn provider_display_name(provider: &str) -> &'static str {
    match provider.to_lowercase().as_str() {
        PROVIDER_GOOGLE_MEET | "google meet" => "Google Meet",
        PROVIDER_ZOOM => "Zoom",
        PROVIDER_TEAMS => "Teams" ,
        PROVIDER_WEBEX => "Webex",
        _ => "In Person",
    }
}

/// The provider a calendar event's conferencing link points at.
pub fn provider_from_url(url: &str) -> &'static str {
    let lower = url.to_lowercase();
    if lower.contains("meet.google.com") {
        PROVIDER_GOOGLE_MEET
    } else if lower.contains("zoom.us") {
        PROVIDER_ZOOM
    } else if lower.contains("teams.microsoft.com") || lower.contains("teams.live.com") {
        PROVIDER_TEAMS
    } else if lower.contains("webex.com") {
        PROVIDER_WEBEX
    } else {
        PROVIDER_OTHER
    }
}

/// Turns a raw browser or app window title into something worth showing.
///
/// Returns one of a small, fixed set of fallback strings when the title carries
/// no real topic — `is_generic_fallback_title` checks against exactly those,
/// which is what the confidence score below is derived from.
pub fn clean_meeting_window_title(raw_title: &str, provider: &str) -> String {
    let mut title = raw_title.trim().to_string();

    for browser in &[
        "Google Chrome",
        "Microsoft\u{200b} Edge",
        "Microsoft Edge",
        "Mozilla Firefox",
        "Brave",
        "Opera",
        "Vivaldi",
    ] {
        if let Some(rest) = strip_dashed_suffix(&title, browser) {
            title = rest;
        }
    }

    match provider {
        PROVIDER_GOOGLE_MEET => {
            if let Some(rest) = strip_dashed_prefix(&title, "Meet") {
                title = rest;
            }
            if let Some(rest) = strip_dashed_suffix(&title, "Google Meet") {
                title = rest;
            }
            if title.is_empty() || title == "Meet" || title == "Google Meet" {
                title = "Google Meet Session".to_string();
            }
        }
        PROVIDER_ZOOM => {
            if let Some(rest) = strip_dashed_prefix(&title, "Zoom") {
                title = rest;
            }
            if let Some(rest) = strip_dashed_suffix(&title, "Zoom") {
                title = rest;
            }
            if title.is_empty() || title == "Zoom" || title == "Zoom Meeting" {
                title = "Zoom Meeting".to_string();
            }
        }
        PROVIDER_TEAMS => {
            if let Some(rest) = title.strip_suffix(" | Microsoft Teams") {
                title = rest.trim().to_string();
            }
            if let Some(rest) = title.strip_prefix("Meeting in ") {
                title = rest.trim().to_string();
            }
            if title.is_empty() || title == "Microsoft Teams" || title == "Microsoft Teams Meeting"
            {
                title = "Teams Meeting".to_string();
            }
        }
        PROVIDER_WEBEX => {
            if let Some(rest) = strip_dashed_suffix(&title, "Cisco Webex Meetings") {
                title = rest;
            }
            if let Some(rest) = strip_dashed_suffix(&title, "Webex") {
                title = rest;
            }
            if title.is_empty() || title == "Webex" {
                title = "Webex Meeting".to_string();
            }
        }
        _ => {}
    }

    title
}

/// The exact fallback strings `clean_meeting_window_title` produces when a
/// window title carried no distinguishing topic.
pub fn is_generic_fallback_title(title: &str) -> bool {
    matches!(
        title,
        "Google Meet Session" | "Zoom Meeting" | "Teams Meeting" | "Webex Meeting"
    )
}

/// A generic title is weaker evidence of a live call than a distinctive one.
///
/// Used as the gate on detection reminders: interrupting someone over a stray
/// idle Zoom window is exactly the notification that gets the whole feature
/// switched off.
pub fn score_confidence(cleaned_title: &str) -> f32 {
    if is_generic_fallback_title(cleaned_title) {
        0.55
    } else {
        0.85
    }
}

/// Visible top-level windows that look like a live conferencing session.
#[cfg(target_os = "windows")]
pub fn detect_active_conferencing_windows() -> Vec<WindowMatch> {
    type Hwnd = *mut std::ffi::c_void;
    type Lparam = isize;
    type Bool = i32;

    #[link(name = "user32")]
    extern "system" {
        fn EnumWindows(
            lpEnumFunc: Option<unsafe extern "system" fn(Hwnd, Lparam) -> Bool>,
            lParam: Lparam,
        ) -> Bool;
        fn GetWindowTextW(hWnd: Hwnd, lpString: *mut u16, nMaxCount: i32) -> i32;
        fn IsWindowVisible(hWnd: Hwnd) -> Bool;
    }

    static FOUND: Mutex<Vec<WindowMatch>> = Mutex::new(Vec::new());

    if let Ok(mut found) = FOUND.lock() {
        found.clear();
    }

    unsafe extern "system" fn enum_proc(hwnd: Hwnd, _: Lparam) -> Bool {
        if IsWindowVisible(hwnd) == 0 {
            return 1;
        }

        let mut buffer = [0u16; 512];
        let len = GetWindowTextW(hwnd, buffer.as_mut_ptr(), 512);
        if len > 0 {
            let raw_title = String::from_utf16_lossy(&buffer[..len as usize]);
            if let Some(provider) = identify_meeting_provider(&raw_title) {
                let title = clean_meeting_window_title(&raw_title, provider);
                let confidence = score_confidence(&title);
                if let Ok(mut found) = FOUND.lock() {
                    found.push(WindowMatch {
                        provider: provider.to_string(),
                        title,
                        raw_title,
                        source: "window_detector".to_string(),
                        confidence,
                    });
                }
            }
        }
        1
    }

    unsafe {
        let _ = EnumWindows(Some(enum_proc), 0);
    }

    FOUND.lock().map(|found| found.clone()).unwrap_or_default()
}

/// Detection is a Windows capability; elsewhere reminders are calendar-only.
#[cfg(not(target_os = "windows"))]
pub fn detect_active_conferencing_windows() -> Vec<WindowMatch> {
    Vec::new()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_title_names_the_conferencing_app_behind_it() {
        assert_eq!(
            identify_meeting_provider("Meet - Architecture Review - Google Chrome"),
            Some(PROVIDER_GOOGLE_MEET)
        );
        assert_eq!(
            identify_meeting_provider("Zoom Meeting"),
            Some(PROVIDER_ZOOM)
        );
        assert_eq!(
            identify_meeting_provider("Product Review | Microsoft Teams"),
            Some(PROVIDER_TEAMS)
        );
        assert_eq!(identify_meeting_provider("Inbox - Gmail"), None);
    }

    #[test]
    fn an_en_dash_in_a_title_is_still_a_meeting() {
        // Google Meet writes an en dash, and matching only the ASCII hyphen
        // made a live Meet call look like nothing on screen — the `detected`
        // reminder failing silently, which is the one failure mode it has.
        assert_eq!(
            identify_meeting_provider("Meet \u{2013} abc-defg-hij \u{2013} Google Chrome"),
            Some(PROVIDER_GOOGLE_MEET)
        );
        assert_eq!(
            clean_meeting_window_title(
                "Meet \u{2013} Sprint Planning \u{2013} Google Chrome",
                PROVIDER_GOOGLE_MEET
            ),
            "Sprint Planning"
        );
        assert_eq!(
            clean_meeting_window_title(
                "Placement sync \u{2014} Zoom \u{2014} Mozilla Firefox",
                PROVIDER_ZOOM
            ),
            "Placement sync"
        );
    }

    #[test]
    fn every_reminder_kind_is_on_out_of_the_box() {
        // A reminder nobody switched off must arrive. Detection was shipped
        // defaulted off, which left the one call with no calendar entry —
        // the only kind nothing else can catch — covered by nothing.
        let settings = super::super::ReminderSettings::default();
        assert!(settings.remind_before_meeting);
        assert!(settings.remind_if_unrecorded);
        assert!(settings.remind_on_detection);

        // And an older settings file that predates the field reads the same
        // way, rather than silently keeping the old default.
        let restored: super::super::ReminderSettings = serde_json::from_str("{}").unwrap();
        assert_eq!(restored, settings);
    }

    #[test]
    fn a_teams_chat_window_is_not_a_meeting() {
        // Teams keeps chat and calls in windows whose titles differ by one
        // word. Treating a chat as a live call is a notification over
        // somebody's shoulder for a meeting that is not happening.
        assert_eq!(identify_meeting_provider("Chat | Microsoft Teams"), None);
    }

    #[test]
    fn browser_and_app_chrome_is_stripped_from_the_title() {
        assert_eq!(
            clean_meeting_window_title(
                "Meet - Sprint Architecture Planning - Google Chrome",
                PROVIDER_GOOGLE_MEET
            ),
            "Sprint Architecture Planning"
        );
        assert_eq!(
            clean_meeting_window_title("Product Review | Microsoft Teams", PROVIDER_TEAMS),
            "Product Review"
        );
        assert_eq!(
            clean_meeting_window_title("Placement sync - Zoom", PROVIDER_ZOOM),
            "Placement sync"
        );
    }

    #[test]
    fn a_topicless_window_falls_back_and_scores_lower() {
        assert_eq!(
            clean_meeting_window_title("Zoom Meeting", PROVIDER_ZOOM),
            "Zoom Meeting"
        );
        assert!(is_generic_fallback_title("Google Meet Session"));
        assert!(!is_generic_fallback_title("Sprint Architecture Planning"));
        assert!(score_confidence("Zoom Meeting") < score_confidence("Sprint Architecture Planning"));
    }

    #[test]
    fn a_conferencing_link_names_its_provider() {
        assert_eq!(
            provider_from_url("https://meet.google.com/abc-defg-hij"),
            PROVIDER_GOOGLE_MEET
        );
        assert_eq!(provider_from_url("https://zoom.us/j/123"), PROVIDER_ZOOM);
        assert_eq!(
            provider_from_url("https://example.com/whereby"),
            PROVIDER_OTHER
        );
    }

    #[test]
    fn detection_is_empty_rather_than_failing_off_windows() {
        // The call must be safe to make on any platform: the scheduler runs the
        // same loop everywhere and reads "nothing on screen", not an error.
        let _ = detect_active_conferencing_windows();
    }
}
