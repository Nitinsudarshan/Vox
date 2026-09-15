//! The loop that actually says something.
//!
//! [`super::reminders`] decides what is due; this decides how often to ask and
//! where the answer goes. Both halves are needed and only one of them can be
//! tested without a running app, which is why they are separate files.
//!
//! ## Where a reminder goes
//!
//! To its own window ([`super::reminder_window`]) and nowhere else. Not into
//! the app — that only reaches somebody already looking at Vox, which is
//! precisely not the person about to miss a meeting — and not to a Windows
//! toast, which goes to the notification centre, cannot carry a working Join
//! button, and is silenced by Focus Assist without telling anybody.
//!
//! ## Why a poll rather than a timer per meeting
//!
//! A timer array has to be torn down and rebuilt on every calendar sync,
//! every settings change and every clock change, and the failure mode of
//! getting that wrong is a reminder that silently never fires. A tick that
//! re-reads the cache has one moving part, costs a file read a minute, and is
//! correct after a laptop sleeps through four of its own timers.
//!
//! ## Why the loop also syncs
//!
//! The calendar cache used to be refreshed only when the Meetings page was
//! opened, so a user who launched Vox and worked somewhere else all day was
//! reminded about nothing — the schedule Vox was reading was whatever it had
//! last time somebody looked at it. Reminders cannot depend on a page being
//! visited, so the loop keeps the cache fresh itself.

use std::collections::HashSet;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use std::time::Instant;

use tauri::{AppHandle, Emitter, Manager};

use crate::commands::AppState;
use crate::sync::MutexExt;

use super::reminder_window;
use super::reminders::{self, MeetingReminder};
use super::store::CalendarStore;

/// How often the schedule is re-checked.
///
/// Thirty seconds: fine enough that a "one minute before" reminder is never
/// more than half a minute late, coarse enough to be a rounding error against
/// everything else the app does.
const TICK: Duration = Duration::from_secs(30);

/// How long after launch the first check runs.
///
/// Short, not zero: the window this draws into is built during the same
/// startup, and a reminder fired before it exists has nowhere to go. Long
/// enough to settle, short enough that launching Vox three minutes before a
/// call still announces it.
const SETTLE: Duration = Duration::from_secs(5);

/// How often the loop refreshes the calendar itself.
///
/// The agenda's own sync runs when the Meetings page is opened, which is not
/// a thing a reminder can depend on. Five minutes is well inside the tightest
/// lead time Vox offers, so a meeting added on the phone still gets announced.
const SYNC_EVERY: Duration = Duration::from_secs(5 * 60);

/// The event the reminder window listens on.
pub const REMINDER_EVENT: &str = "meeting-reminder";

/// Starts the reminder loop. Runs for the life of the app.
///
/// Spawned rather than awaited, and every failure inside it is swallowed and
/// logged: a calendar that cannot be read is a reason to say nothing this
/// tick, never a reason to take the app down or to stop checking.
pub fn spawn(app: AppHandle) {
    std::thread::spawn(move || {
        let sent: Arc<Mutex<HashSet<String>>> = Arc::new(Mutex::new(HashSet::new()));
        let mut last_sync: Option<Instant> = None;

        std::thread::sleep(SETTLE);
        loop {
            // Gated on the attempt rather than on the result: a sync that
            // keeps failing must not turn into a request every thirty
            // seconds for as long as the app is open.
            if last_sync.is_none_or(|at| at.elapsed() >= SYNC_EVERY) {
                last_sync = Some(Instant::now());
                refresh_calendar(&app);
            }
            tick(&app, &sent);
            std::thread::sleep(TICK);
        }
    });
}

/// Re-reads the connected calendars, in the background.
///
/// Fire-and-forget: this tick reports on the cache as it stands, and whatever
/// the sync brings in is announced by the next one. Waiting for a network
/// round trip before saying anything would make a reminder late for the sake
/// of a meeting that is probably already in the cache.
fn refresh_calendar(app: &AppHandle) {
    let handle = app.clone();
    tauri::async_runtime::spawn(async move {
        match super::commands::sync_calendars(handle).await {
            Ok(accounts) => tracing::debug!(
                "reminder loop: synced {} calendar account(s)",
                accounts.len()
            ),
            Err(err) => tracing::warn!("reminder loop: calendar sync failed: {}", err.message),
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

    let mut events: Vec<super::CalendarEvent> = store
        .load_events()
        .into_iter()
        .filter(|event| enabled.contains(&event.account_email.to_lowercase()))
        .collect();

    // Which events Vox already has a recording of. The cache stores events as
    // Google sent them; the link to a recording is worked out on read, and
    // without doing it here the "nothing is being recorded" nudge would fire
    // for a meeting that was recorded perfectly well an hour ago.
    super::agenda::match_recordings(&mut events, &super::commands::recording_windows(&state));
    let recording_active = state.meeting_engine.status().active;

    // Logged because everything below it fails quietly by design. Without
    // this line "no reminder appeared" and "no meeting was due" look
    // identical from the outside, which is how a broken poller goes
    // unnoticed for a week.
    tracing::debug!(
        "reminder loop: {} account(s), {} cached event(s), leads {:?}, recording: {}",
        enabled.len(),
        events.len(),
        settings.buckets(),
        recording_active
    );

    let now = chrono::Utc::now();
    let due = {
        let mut guard = sent.lock_or_recover();
        *guard = reminders::prune_sent(&events, now, &guard);
        let due = reminders::due_reminders(&events, now, &settings, &guard, recording_active);
        for reminder in &due {
            guard.insert(reminder.key.clone());
        }
        due
    };

    for reminder in due {
        announce(app, &reminder);
    }
}

/// Raises the reminder window and hands it one reminder.
///
/// Shown before the event is emitted. The window is built hidden at startup
/// and kept, so this is a show and a message rather than a construction; a
/// listener that is not up yet would otherwise miss the only event it was
/// ever going to get.
pub fn announce(app: &AppHandle, reminder: &MeetingReminder) {
    tracing::info!(
        "meeting reminder ({:?}): {} ({} minute(s) out)",
        reminder.kind,
        reminder.title,
        reminder.minutes_until
    );
    reminder_window::show(app);
    if let Err(err) = app.emit(REMINDER_EVENT, reminder) {
        tracing::warn!("could not deliver a meeting reminder to its window: {}", err);
    }
}
