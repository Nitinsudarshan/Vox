//! The reminder's own window.
//!
//! A meeting reminder is not a panel inside Vox and it is not a Windows toast.
//! Both were tried and both are wrong for the one job this has:
//!
//! - **Inside the app** it only reaches somebody already looking at Vox, which
//!   is precisely not the person who is about to miss a meeting.
//! - **A Windows toast** goes to the notification centre, cannot carry a Join
//!   button that does the right thing, is silenced by Focus Assist without
//!   telling anybody, and looks like Windows rather than like Vox.
//!
//! So it is a window of its own — the same shape as the dictation pill in
//! [`crate::overlay`]: undecorated, transparent, always on top, off the
//! taskbar, and unfocused so that appearing never steals a keystroke from
//! whatever the user is typing into.
//!
//! ## Built once, shown many times
//!
//! The window is created hidden at startup and afterwards only shown and
//! hidden. Building a webview takes long enough to be visible, and a reminder
//! that arrives a beat late is a reminder about a meeting that has already
//! started.
//!
//! ## Height comes from the content
//!
//! One reminder is a small card; three stacked are not. The webview measures
//! itself and calls [`resize_reminder_window`], because Rust cannot know how
//! tall a card is after the text wraps.

use tauri::{AppHandle, LogicalPosition, LogicalSize, Manager, WebviewUrl, WebviewWindowBuilder};

pub const REMINDER_WINDOW_LABEL: &str = "meeting-reminder";

/// Fixed width. Wide enough for a meeting title and two buttons, narrow
/// enough to sit in a corner without covering what is underneath.
const WIDTH: f64 = 380.0;

/// Height before the webview has measured itself. One card's worth.
const INITIAL_HEIGHT: f64 = 132.0;

/// Gap from the edges of the work area, so it does not touch the screen edge.
const MARGIN: f64 = 16.0;

/// Creates the reminder window, hidden, if it does not exist yet.
///
/// Idempotent, so it is safe on every startup.
pub fn ensure_window(app: &AppHandle) {
    if app.get_webview_window(REMINDER_WINDOW_LABEL).is_some() {
        return;
    }

    let mut builder = WebviewWindowBuilder::new(
        app,
        REMINDER_WINDOW_LABEL,
        WebviewUrl::App("index.html#/meeting-reminder".into()),
    )
    .title("Vox — Meeting reminder")
    .inner_size(WIDTH, INITIAL_HEIGHT)
    .resizable(false)
    .decorations(false)
    .always_on_top(true)
    .skip_taskbar(true)
    .transparent(true)
    .shadow(false)
    .visible(false)
    // Never steal focus by appearing. A reminder that eats the keystroke
    // somebody was typing into another app is worse than a missed meeting.
    .focused(false);

    if let Some((x, y)) = anchor(app, INITIAL_HEIGHT) {
        builder = builder.position(x, y);
    }

    if let Err(err) = builder.build() {
        tracing::error!("could not create the meeting reminder window: {}", err);
    }
}

/// Brings the reminder window up, creating it first if startup missed it.
pub fn show(app: &AppHandle) {
    ensure_window(app);
    let Some(window) = app.get_webview_window(REMINDER_WINDOW_LABEL) else {
        return;
    };
    // Re-anchored on every show: the laptop may have been undocked, or the
    // cursor may be on a different monitor than it was at startup.
    if let Ok(size) = window.inner_size() {
        let scale = window.scale_factor().unwrap_or(1.0);
        if let Some((x, y)) = anchor(app, size.height as f64 / scale) {
            let _ = window.set_position(LogicalPosition::new(x, y));
        }
    }
    if let Err(err) = window.show() {
        tracing::warn!("could not show the meeting reminder window: {}", err);
    }
    // Windows can place a newly shown always-on-top window behind the
    // foreground one; asking again after it is visible is what makes it stick.
    let _ = window.set_always_on_top(true);
}

/// Hides it. Called when the last reminder on it is dismissed.
pub fn hide(app: &AppHandle) {
    if let Some(window) = app.get_webview_window(REMINDER_WINDOW_LABEL) {
        let _ = window.hide();
    }
}

/// Resizes to the height the webview measured, and re-anchors.
///
/// Bounded at both ends: a zero height would make the window unclickable, and
/// an unbounded one could cover the screen if the measurement ever went wrong.
pub fn resize(app: &AppHandle, height: f64) {
    let height = height.clamp(64.0, 600.0);
    let Some(window) = app.get_webview_window(REMINDER_WINDOW_LABEL) else {
        return;
    };
    let _ = window.set_size(LogicalSize::new(WIDTH, height));
    if let Some((x, y)) = anchor(app, height) {
        let _ = window.set_position(LogicalPosition::new(x, y));
    }
}

/// Bottom-right of the work area of the monitor the cursor is on.
///
/// The work area rather than the screen, so it sits above the taskbar wherever
/// the taskbar happens to be; the cursor's monitor because Tauri has no
/// cross-platform way to ask which monitor the user is actually working on,
/// and the cursor is the closest proxy — the same reasoning as
/// [`crate::overlay`]'s pill anchor.
fn anchor(app: &AppHandle, height: f64) -> Option<(f64, f64)> {
    let monitor = active_monitor(app)?;
    let scale = monitor.scale_factor();
    let work_area = monitor.work_area();

    let wa_x = work_area.position.x as f64 / scale;
    let wa_y = work_area.position.y as f64 / scale;
    let wa_w = work_area.size.width as f64 / scale;
    let wa_h = work_area.size.height as f64 / scale;

    Some((
        wa_x + wa_w - WIDTH - MARGIN,
        wa_y + wa_h - height - MARGIN,
    ))
}

fn active_monitor(app: &AppHandle) -> Option<tauri::Monitor> {
    if let Ok(cursor) = app.cursor_position() {
        if let Ok(Some(monitor)) = app.monitor_from_point(cursor.x, cursor.y) {
            return Some(monitor);
        }
    }
    app.get_webview_window(crate::hotkeys::MAIN_WINDOW_LABEL)?
        .primary_monitor()
        .ok()
        .flatten()
}
