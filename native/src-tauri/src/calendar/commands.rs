//! The calendar IPC surface.
//!
//! Thin by policy (`rules/rust-backend.md`): validate, call one domain
//! function, map the error. The de-duplication, the matching and the day
//! grouping all live in [`super::agenda`], where they are tested without a
//! Google account.

use tauri::{AppHandle, Manager, State};

use crate::commands::{AppState, CommandError};
use crate::oauth::{start_desktop_oauth_flow, OAuthTokens, SCOPE_CALENDAR_READONLY, SCOPE_IDENTITY};

use super::agenda::{self, RecordingWindow};
use super::google::{self, CalendarApiError, CalendarSummary};
use super::model::{CalendarAccount, DayAgenda};
use super::parse;
use super::store::CalendarStore;
use crate::meetings::series::{self, MeetingSeries, SeriesSource};

/// Days of history and of future a sync asks Google for.
///
/// Backwards so a recording made last week can still find the meeting it
/// belongs to; forwards so "coming up" has something in it. Both are small:
/// this is a working window, not an archive, and it is re-fetched on every
/// sync rather than reconciled.
const SYNC_DAYS_BACK: i64 = 30;
const SYNC_DAYS_AHEAD: i64 = 30;

impl From<CalendarApiError> for CommandError {
    fn from(err: CalendarApiError) -> Self {
        match err {
            CalendarApiError::Reauthorize(detail) => CommandError::new(
                "CALENDAR_REAUTHORIZE",
                &format!("{detail} Connect this account again under Settings › Calendar."),
            ),
            other => CommandError::new("CALENDAR_UNAVAILABLE", &other.to_string()),
        }
    }
}

impl From<super::CalendarStoreError> for CommandError {
    fn from(err: super::CalendarStoreError) -> Self {
        CommandError::new("CALENDAR_STORAGE_FAILED", &err.to_string())
    }
}

fn calendar_store(state: &State<'_, AppState>) -> CalendarStore {
    CalendarStore::new(
        state.vault.vault_dir().join("calendar"),
        state.config_dir.clone(),
    )
}

/// Every connected account.
#[tauri::command]
pub fn list_calendar_accounts(state: State<'_, AppState>) -> Vec<CalendarAccount> {
    calendar_store(&state).load_accounts()
}

/// Signs in to one Google account and stores its calendar tokens.
///
/// Identity scopes are requested alongside the calendar scope because an
/// account with no address is one the user cannot tell apart from their other
/// account, and "which of my two Gmail accounts is this row from" is the first
/// question a multi-account agenda raises.
#[tauri::command]
pub async fn connect_calendar_account(
    app: AppHandle,
) -> Result<Vec<CalendarAccount>, CommandError> {
    let (client_id, client_secret) = google_credentials();

    let scopes = format!("{SCOPE_IDENTITY} {SCOPE_CALENDAR_READONLY}");
    let result = start_desktop_oauth_flow(client_id, client_secret, &scopes)
        .await
        .map_err(|err| CommandError::new("CALENDAR_SIGN_IN_FAILED", &err))?;

    let email = result
        .user_profile
        .as_ref()
        .map(|profile| profile.email.clone())
        .or_else(|| result.tokens.account_email.clone())
        .ok_or_else(|| {
            CommandError::new(
                "CALENDAR_SIGN_IN_FAILED",
                "Google did not say which account was signed in, so it cannot be told apart from \
                 the others. Try connecting again.",
            )
        })?;

    let state = app.state::<AppState>();
    let store = calendar_store(&state);
    store.save_tokens(&email, &result.tokens)?;

    let accounts = store.upsert_account(CalendarAccount {
        display_name: result
            .user_profile
            .as_ref()
            .and_then(|profile| profile.name.clone()),
        email,
        enabled: true,
        calendar_ids: Vec::new(),
        last_synced_at: None,
        last_error: None,
    })?;
    Ok(accounts)
}

/// Disconnects an account: its tokens, and its events with them.
#[tauri::command]
pub fn disconnect_calendar_account(
    state: State<'_, AppState>,
    email: String,
) -> Result<Vec<CalendarAccount>, CommandError> {
    Ok(calendar_store(&state).remove_account(email.trim())?)
}

/// Turns one account's events on or off without signing it out.
#[tauri::command]
pub fn set_calendar_account_enabled(
    state: State<'_, AppState>,
    email: String,
    enabled: bool,
) -> Result<Vec<CalendarAccount>, CommandError> {
    let store = calendar_store(&state);
    let mut accounts = store.load_accounts();
    let Some(account) = accounts
        .iter_mut()
        .find(|account| account.email.eq_ignore_ascii_case(email.trim()))
    else {
        return Err(CommandError::new(
            "CALENDAR_UNKNOWN_ACCOUNT",
            "That account is not connected.",
        ));
    };
    account.enabled = enabled;
    store.save_accounts(&accounts)?;
    Ok(accounts)
}

/// Which calendars an account can read, so the user can pick among them.
#[tauri::command]
pub async fn list_account_calendars(
    app: AppHandle,
    email: String,
) -> Result<Vec<CalendarSummary>, CommandError> {
    let (client_id, client_secret) = google_credentials();
    // Scoped so the state borrow ends before the awaits below: holding a
    // `State` across an await is what makes a command stop being `Send`.
    let tokens = {
        let state = app.state::<AppState>();
        calendar_store(&state).load_tokens(&email)
    }
    .ok_or_else(|| {
        CommandError::new("CALENDAR_UNKNOWN_ACCOUNT", "That account is not connected.")
    })?;

    let (access_token, refreshed) =
        google::fresh_access_token(&tokens, client_id, client_secret).await?;
    persist_refresh(&app, &email, refreshed);

    Ok(google::list_calendars(&access_token).await?)
}

/// Chooses which of an account's calendars are read.
#[tauri::command]
pub fn set_account_calendars(
    state: State<'_, AppState>,
    email: String,
    calendar_ids: Vec<String>,
) -> Result<Vec<CalendarAccount>, CommandError> {
    let store = calendar_store(&state);
    let mut accounts = store.load_accounts();
    let Some(account) = accounts
        .iter_mut()
        .find(|account| account.email.eq_ignore_ascii_case(email.trim()))
    else {
        return Err(CommandError::new(
            "CALENDAR_UNKNOWN_ACCOUNT",
            "That account is not connected.",
        ));
    };
    account.calendar_ids = calendar_ids;
    store.save_accounts(&accounts)?;
    Ok(accounts)
}

/// Fetches every enabled account's events into the cache.
///
/// One account failing never fails the sync. The others are still fetched, the
/// failure is recorded against the account that had it, and the agenda renders
/// from what did work — which is the difference between "your work calendar
/// needs reconnecting" and "the calendar is broken".
#[tauri::command]
pub async fn sync_calendars(app: AppHandle) -> Result<Vec<CalendarAccount>, CommandError> {
    let (client_id, client_secret) = google_credentials();
    let accounts = {
        let state = app.state::<AppState>();
        calendar_store(&state).load_accounts()
    };
    let (time_min, time_max) = agenda::sync_window(SYNC_DAYS_BACK, SYNC_DAYS_AHEAD);

    let mut updated = accounts.clone();
    for account in updated.iter_mut() {
        if !account.enabled {
            continue;
        }
        match sync_one(
            &app,
            account,
            &time_min,
            &time_max,
            client_id.clone(),
            client_secret.clone(),
        )
        .await
        {
            Ok(count) => {
                account.last_synced_at = Some(chrono::Utc::now().to_rfc3339());
                account.last_error = None;
                tracing::info!("calendar {}: synced {} event(s)", account.email, count);
            }
            Err(err) => {
                // Recorded against the account rather than returned, so one
                // expired grant does not hide the other account's meetings.
                account.last_error = Some(err.to_string());
                tracing::warn!("calendar {}: sync failed: {}", account.email, err);
            }
        }
    }

    let state = app.state::<AppState>();
    calendar_store(&state).save_accounts(&updated)?;
    stamp_recurring_series(&state);
    Ok(updated)
}

/// Records which recordings belong to which recurring meeting.
///
/// Runs at the end of a sync rather than on every agenda read, and writes the
/// membership onto the meeting: the event cache is a rolling window, so a
/// series derived fresh on each read would look right for a month and then
/// quietly lose its older occurrences.
///
/// Never clears a membership, for the same reason. An occurrence that has
/// aged out of the window, or whose event was deleted from the calendar, is
/// still a recording of that series.
///
/// Best-effort throughout: a series that cannot be written is not a reason to
/// fail a sync that fetched everything correctly.
fn stamp_recurring_series(state: &State<'_, AppState>) {
    let store = calendar_store(state);
    let mut events = store.load_events();
    agenda::match_recordings(&mut events, &recording_windows(state));

    let assignments = series::assignments_from_events(&events);
    if assignments.is_empty() {
        return;
    }

    let meetings = state.meeting_store.list_meetings().unwrap_or_default();
    let now = chrono::Utc::now().to_rfc3339();

    for assignment in assignments {
        let Some(meeting) = meetings.iter().find(|m| m.id == assignment.meeting_id) else {
            continue;
        };

        if let Err(err) = state.meeting_store.upsert_series(MeetingSeries {
            id: assignment.series_id.clone(),
            title: assignment.title.clone(),
            source: SeriesSource::Google,
            created_at: now.clone(),
            renamed_by_user: false,
        }) {
            tracing::warn!("could not record meeting series: {}", err);
            continue;
        }

        // A membership the user set by hand wins: they may have moved this
        // recording into a series Google split, and a sync must not move it
        // back out.
        if meeting.series_id.is_some() {
            continue;
        }
        if let Err(err) = state
            .meeting_store
            .set_meeting_series(&meeting.id, Some(assignment.series_id))
        {
            tracing::warn!("could not put {} in its series: {}", meeting.id, err);
        }
    }
}

async fn sync_one(
    app: &AppHandle,
    account: &CalendarAccount,
    time_min: &str,
    time_max: &str,
    client_id: Option<String>,
    client_secret: Option<String>,
) -> Result<usize, CalendarApiError> {
    let tokens = {
        let state = app.state::<AppState>();
        calendar_store(&state).load_tokens(&account.email)
    };
    let Some(tokens) = tokens else {
        return Err(CalendarApiError::Reauthorize(
            "no stored credentials for this account".to_string(),
        ));
    };

    let (access_token, refreshed) =
        google::fresh_access_token(&tokens, client_id, client_secret).await?;
    persist_refresh(app, &account.email, refreshed);

    // No explicit choice means the primary calendar, which is the one
    // everybody has and the one almost everybody means.
    let calendar_ids: Vec<String> = if account.calendar_ids.is_empty() {
        vec!["primary".to_string()]
    } else {
        account.calendar_ids.clone()
    };

    let mut events = Vec::new();
    for calendar_id in calendar_ids {
        events.extend(
            google::list_events(
                &access_token,
                &account.email,
                &calendar_id,
                time_min,
                time_max,
            )
            .await?,
        );
    }

    let count = events.len();
    let state = app.state::<AppState>();
    let _ = calendar_store(&state).replace_account_events(&account.email, events);
    Ok(count)
}

/// The agenda, from the cache, with recordings already matched to events.
///
/// Reads what the last sync stored rather than fetching: a calendar that shows
/// nothing until a network round trip finishes is one people stop opening, and
/// the sync runs on its own.
#[tauri::command]
pub fn get_calendar_agenda(state: State<'_, AppState>) -> Result<Vec<DayAgenda>, CommandError> {
    let store = calendar_store(&state);
    let enabled: Vec<String> = store
        .load_accounts()
        .into_iter()
        .filter(|account| account.enabled)
        .map(|account| account.email.to_lowercase())
        .collect();

    let mut events: Vec<super::CalendarEvent> = store
        .load_events()
        .into_iter()
        .filter(|event| enabled.contains(&event.account_email.to_lowercase()))
        .collect();

    agenda::match_recordings(&mut events, &recording_windows(&state));
    Ok(agenda::build_agenda(events))
}

/// Every recording Vox has, as a time window.
///
/// A meeting with no duration is skipped rather than given a zero-length
/// window: it would match nothing, and including it only invites a
/// zero-overlap match somewhere downstream.
pub(super) fn recording_windows(state: &State<'_, AppState>) -> Vec<RecordingWindow> {
    state
        .meeting_store
        .list_meetings()
        .unwrap_or_default()
        .into_iter()
        .filter(|meeting| meeting.duration_seconds > 0.0)
        .filter_map(|meeting| {
            let start = chrono::DateTime::parse_from_rfc3339(&meeting.created_at).ok()?;
            let end = start + chrono::Duration::seconds(meeting.duration_seconds as i64);
            Some(RecordingWindow {
                meeting_id: meeting.id,
                start,
                end,
            })
        })
        .collect()
}

/// Opens a link that came off the calendar, in whatever the OS uses for links.
///
/// Two guards, and they are not the same guard. The scheme check
/// ([`parse::is_web_url`]) is what stops the opener — which hands what it is
/// given to the shell — from being asked to run a local file. The cache lookup
/// is what makes the command unable to open a URL that is not on one of the
/// user's own events: the frontend passes a string, and a command that opens
/// any string it is handed is a command the rest of the app can be tricked
/// into calling.
#[tauri::command]
pub fn open_calendar_link(
    app: AppHandle,
    state: State<'_, AppState>,
    url: String,
) -> Result<(), CommandError> {
    use tauri_plugin_opener::OpenerExt;

    let url = url.trim().to_string();
    if !parse::is_web_url(&url) {
        return Err(CommandError::new(
            "CALENDAR_LINK_REFUSED",
            "That link is not a web address, so Vox will not open it.",
        ));
    }

    let known = calendar_store(&state).load_events().into_iter().any(|event| {
        event.conference_url.as_deref() == Some(url.as_str())
            || event.html_link.as_deref() == Some(url.as_str())
    });
    if !known {
        return Err(CommandError::new(
            "CALENDAR_LINK_REFUSED",
            "That link is not on any event in your calendar. Sync and try again.",
        ));
    }

    app.opener()
        .open_url(url, None::<&str>)
        .map_err(|err| CommandError::new("CALENDAR_LINK_FAILED", &err.to_string()))
}

/// Closes the reminder window, once the last reminder on it is dismissed.
#[tauri::command]
pub fn dismiss_meeting_reminders(app: AppHandle) {
    super::reminder_window::hide(&app);
}

/// Resizes the reminder window to the height its content measured.
///
/// Called by the window itself: one card and three stacked are different
/// heights, and Rust cannot know how tall a meeting title is after it wraps.
#[tauri::command]
pub fn resize_meeting_reminders(app: AppHandle, height: f64) {
    super::reminder_window::resize(&app, height);
}

/// The OAuth client Vox signs in with.
///
/// `None` for both, deliberately: [`crate::oauth::GoogleOAuthConfig`] resolves
/// them from the build's or the machine's environment, and duplicating that
/// resolution here is how one of the two copies comes to disagree with the
/// other. Passing an override is what the parameters are for, and nothing
/// currently has one to pass.
fn google_credentials() -> (Option<String>, Option<String>) {
    (None, None)
}

/// Stores a token that was refreshed during a request.
///
/// A refresh whose result is dropped means refreshing again on the next sync,
/// and on every sync after that — a round trip per call, forever, for want of
/// one write.
fn persist_refresh(app: &AppHandle, email: &str, refreshed: Option<OAuthTokens>) {
    let Some(tokens) = refreshed else { return };
    let state = app.state::<AppState>();
    if let Err(err) = calendar_store(&state).save_tokens(email, &tokens) {
        tracing::warn!("calendar {}: could not store the refreshed token: {}", email, err);
    }
}
