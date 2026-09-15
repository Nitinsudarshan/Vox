//! The loop that actually says something.
//!
//! [`super::reminders`] decides what is due; this decides how often to ask and
//! where the answer goes. Both halves are needed and only one of them can be
//! tested without a running app, which is why they are separate files.
//!
//! ## Two channels, on purpose
//!
//! Every reminder is emitted as a Tauri event, which the app renders itself,
//! **and** — unless the user turns it off — as an OS toast. Neither alone is
//! enough. A toast is what reaches somebody working in another window, which
//! is where they are when they miss a meeting; the in-app surface is the one
//! that can offer Join and Start meeting as buttons, survive a notification
//! centre that swallowed the toast, and be styled like the rest of Vox
//! instead of like whatever Windows decided this year.
//!
//! ## Why a poll rather than a timer per meeting
//!
//! A timer array has to be torn down and rebuilt on every calendar sync,
//! every settings change and every clock change, and the failure mode of
//! getting that wrong is a reminder that silently never fires. A tick that
//! re-reads the cache has one moving part, costs a file read a minute, and is
//! correct after a laptop sleeps through four of its own timers.

use std::collections::HashSet;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_notification::NotificationExt;

use crate::commands::AppState;
use crate::sync::MutexExt;

use super::reminders::{self, MeetingReminder};
use super::store::CalendarStore;

/// How often the schedule is re-checked.
///
/// Thirty seconds: fine enough that a "one minute before" reminder is never
/// more than half a minute late, coarse enough to be a rounding error against
/// everything else the app does.
const TICK: Duration = Duration::from_secs(30);

/// The event the frontend listens on.
pub const REMINDER_EVENT: &str = "meeting-reminder";

/// Starts the reminder loop. Runs for the life of the app.
///
/// Spawned rather than awaited, and every failure inside it is swallowed and
/// logged: a calendar that cannot be read is a reason to say nothing this
/// tick, never a reason to take the app down or to stop checking.
pub fn spawn(app: AppHandle) {
    std::thread::spawn(move || {
        let sent: Arc<Mutex<HashSet<String>>> = Arc::new(Mutex::new(HashSet::new()));
        loop {
            std::thread::sleep(TICK);
            tick(&app, &sent);
        }
    });
}

/// One pass: read the schedule, work out what is due, say it.
fn tick(app: &AppHandle, sent: &Arc<Mutex<HashSet<String>>>) {
    let state = app.state::<AppState>();
    let settings = state.settings.lock_or_recover().meetings.reminders.clone();
    if !settings.enabled {
        return;
    }

    let store = CalendarStore::new(
        state.vault.vault_dir().join("calendar"),
        state.config_dir.clone(),
    );

    // Only accounts the user has switched on. An account turned off in
    // Settings is one whose meetings should not interrupt them.
    let enabled: Vec<String> = store
        .load_accounts()
        .into_iter()
        .filter(|account| account.enabled)
        .map(|account| account.email.to_lowercase())
        .collect();
    if enabled.is_empty() {
        return;
    }

    let events: Vec<super::CalendarEvent> = store
        .load_events()
        .into_iter()
        .filter(|event| enabled.contains(&event.account_email.to_lowercase()))
        .collect();

    let now = chrono::Utc::now();
    let due = {
        let mut guard = sent.lock_or_recover();
        *guard = reminders::prune_sent(&events, now, &guard);
        let due = reminders::due_reminders(&events, now, &settings, &guard);
        for reminder in &due {
            guard.insert(reminder.key.clone());
        }
        due
    };

    for reminder in due {
        announce(app, &reminder, settings.system_notification);
    }
}

/// Sends one reminder down both channels.
fn announce(app: &AppHandle, reminder: &MeetingReminder, system_notification: bool) {
    if let Err(err) = app.emit(REMINDER_EVENT, reminder) {
        tracing::warn!("could not deliver a meeting reminder to the window: {}", err);
    }

    if !system_notification {
        return;
    }
    // Best-effort. A user who denied notification permission at the OS level
    // still gets the in-app one, which is the reason there are two.
    let _ = app
        .notification()
        .builder()
        .title(reminder.headline())
        .body(reminder.detail())
        .show();
}
