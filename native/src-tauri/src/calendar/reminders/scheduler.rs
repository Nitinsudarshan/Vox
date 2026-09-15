//! The clock behind the reminders.
//!
//! One background task, started once at launch. Every tick it asks the two
//! sources what they know, hands both to `recompute`, and raises whatever
//! became due. It owns no state of its own beyond a cache of the calendar.
//!
//! The two sources are polled at different rates on purpose. Window detection
//! is a local syscall and is cheap enough to run every tick; the calendar is a
//! network call against somebody's Google account and is refreshed far less
//! often. Reminder *timing* stays precise either way, because it is computed
//! from event start times rather than from when the fetch happened.

use super::{enqueue_to_show, recompute, take_next_to_show, ReminderEvent, ReminderInputs, ReminderQueue};
use crate::calendar::store::CalendarStore;
use crate::calendar::CalendarEvent;
use crate::commands::AppState;
use crate::sync::MutexExt;
use std::collections::VecDeque;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tauri::{AppHandle, Manager};

/// How often the queue is reconciled. Matches the granularity a person would
/// notice: a reminder five minutes out is not made better by being fifteen
/// seconds more punctual, and a tighter loop only spends battery.
const TICK: Duration = Duration::from_secs(15);

/// How often the calendar is re-read.
///
/// Vox's sync fetches a month either way, so a two-minute refresh cannot make
/// a reminder late; it only delays noticing an invitation accepted moments ago.
const CALENDAR_REFRESH: Duration = Duration::from_secs(120);

/// Starts the reminder loop. Called once, from `setup`.
pub fn spawn(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        let mut last_calendar_read: Option<Instant> = None;
        // Reminders that are due but have not been on screen yet. One card
        // shows at a time, so anything that comes due behind the current one
        // waits here rather than being dropped.
        let mut backlog: VecDeque<ReminderEvent> = VecDeque::new();
        let mut ticker = tokio::time::interval(TICK);

        loop {
            ticker.tick().await;

            let state = app.state::<AppState>();
            let settings = state.settings.lock_or_recover().meetings.clone();

            // Nothing to compute when every kind is off — and no calendar call
            // to make either, which is the point: switching reminders off in
            // settings stops Vox reading the calendar for them.
            if !settings.reminders.remind_before_meeting
                && !settings.reminders.remind_if_unrecorded
                && !settings.reminders.remind_on_detection
            {
                continue;
            }

            let wants_calendar = settings.reminders.remind_before_meeting
                || settings.reminders.remind_if_unrecorded;
            let due_for_refresh = last_calendar_read
                .is_none_or(|read_at| read_at.elapsed() >= CALENDAR_REFRESH);

            if wants_calendar && due_for_refresh {
                // Vox reads several accounts, and the agenda's own sync is the
                // thing that fills the cache — so the loop asks for a sync and
                // then reads the cache, rather than fetching for itself. The
                // sync is fire-and-forget: this tick reports on the cache as it
                // stands and whatever lands is announced by the next one.
                refresh_calendar(&app);
                last_calendar_read = Some(Instant::now());
            }

            // Re-read every tick: the cache is cheap to read and a sync that
            // landed a moment ago should not wait two minutes to be noticed.
            let events: Vec<CalendarEvent> = cached_events(&state);

            let windows = if settings.reminders.remind_on_detection
                || settings.reminders.remind_if_unrecorded
            {
                // A syscall walking every top-level window, off the async
                // runtime's threads.
                tauri::async_runtime::spawn_blocking(super::detection::detect_active_conferencing_windows)
                    .await
                    .unwrap_or_default()
            } else {
                Vec::new()
            };

            let is_recording = state.meeting_engine.status().active;
            // Recordings that already exist, so a meeting captured and stopped
            // early is not then reported as unrecorded.
            let sessions = state.meeting_store.list_meetings().unwrap_or_default();
            let queue = app.state::<Arc<ReminderQueue>>();
            let (_, newly_fired) = recompute(
                &queue,
                &ReminderInputs {
                    events: &events,
                    windows: &windows,
                    settings: &settings.reminders,
                    is_recording,
                    sessions: &sessions,
                    now: chrono::Utc::now(),
                },
            );

            enqueue_to_show(&mut backlog, newly_fired);

            let notifications = app.state::<Arc<super::NotificationService>>();
            // One card at a time, and only once the previous one has been
            // answered or timed out — so a second meeting starting in the same
            // minute waits its turn instead of replacing the first.
            if notifications.showing().is_some() {
                continue;
            }
            if let Some(entry) = take_next_to_show(&queue, &mut backlog) {
                tracing::info!("[reminders] raising '{}' ({:?})", entry.title, entry.kind);
                notifications.show(&app, &entry, is_recording, &settings.reminders);
            }
        }
    });
}

/// The cached events of every enabled account.
///
/// Vox reads several Google accounts into one cache, so "the calendar" here is
/// that cache filtered to the accounts the user has switched on — an account
/// turned off in Settings is one whose meetings should not interrupt them.
fn cached_events(state: &tauri::State<'_, AppState>) -> Vec<CalendarEvent> {
    let store = CalendarStore::new(
        state.vault.vault_dir().join("calendar"),
        state.config_dir.clone(),
    );
    let enabled: Vec<String> = store
        .load_accounts()
        .into_iter()
        .filter(|account| account.enabled)
        .map(|account| account.email.to_lowercase())
        .collect();
    if enabled.is_empty() {
        return Vec::new();
    }
    store
        .load_events()
        .into_iter()
        .filter(|event| enabled.contains(&event.account_email.to_lowercase()))
        .collect()
}

/// Asks for a calendar sync, without waiting for it.
///
/// The agenda syncs when the Meetings page is opened, which is not something a
/// reminder can depend on: a user who launches Vox and works somewhere else all
/// day would otherwise be reminded from whatever schedule was last looked at.
fn refresh_calendar(app: &AppHandle) {
    let handle = app.clone();
    tauri::async_runtime::spawn(async move {
        match crate::calendar::commands::sync_calendars(handle).await {
            Ok(accounts) => {
                tracing::debug!("[reminders] synced {} calendar account(s)", accounts.len())
            }
            Err(err) => tracing::warn!("[reminders] calendar sync failed: {}", err.message),
        }
    });
}
