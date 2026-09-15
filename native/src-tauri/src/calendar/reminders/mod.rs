//! Telling somebody a meeting is about to happen, and letting them act on it.
//!
//! Three questions decide whether a reminder exists at all, and they are asked
//! of two independent sources. The calendar knows what was scheduled and who
//! was invited; `detection` knows which conferencing app is actually on screen.
//! Neither alone is enough — a calendar entry for a call the user quietly left
//! is not a meeting, and a stray idle Zoom window is not one either.
//!
//! Three properties this module holds:
//!
//! * **A queue, never a slot.** Every tracked reminder is one entry keyed by
//!   `(key, kind)`. The subsystem this replaces held a single overwritable
//!   payload, so a second reminder silently erased the first one before anybody
//!   had seen it (`docs/decisions.md` Decision 45, Broken #2).
//! * **Fired once, per transition.** `recompute` returns what *became* due on
//!   this call, not everything currently due, so the caller raises one
//!   notification per reminder rather than one per poll tick.
//! * **A key is not a session id.** A reminder names a calendar event or a
//!   window, both of which exist before any recording does. Handing such a
//!   string to a command that wants a real session id is what made the old
//!   popup's primary action fail for every reminder it ever showed (Decision
//!   45, Broken #1); the type keeps them apart.

pub mod detection;
pub mod notification;
pub mod scheduler;

use crate::calendar::CalendarEvent;
use crate::meetings::model::{Meeting, MeetingState};
use crate::sync::MutexExt;
use chrono::{DateTime, Duration, Utc};
use detection::WindowMatch;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, VecDeque};
use std::sync::Mutex;

pub use notification::{MeetingReminderPayload, NotificationService};

/// Why a reminder exists.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReminderKind {
    /// A scheduled meeting is about to start.
    Upcoming,
    /// A scheduled meeting has started and nothing is recording it.
    Unrecorded,
    /// A conferencing call is on screen that the calendar knows nothing about.
    Detected,
}

/// `Pending -> Fired -> { Snoozed | Dismissed | Actioned | Expired }`.
///
/// `Expired` is passive: the window passed with no interaction. It is a
/// terminal record, never an interruption of its own.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum ReminderStatus {
    Pending,
    Fired,
    Snoozed { until: DateTime<Utc> },
    Dismissed,
    Actioned,
    Expired,
}

/// One tracked reminder.
#[derive(Debug, Clone, Serialize)]
pub struct ReminderEvent {
    /// Stable identity of what is being reminded about — `cal:<event id>` for a
    /// calendar event, `win:<provider>:<title>` for a detected call.
    ///
    /// Deliberately not called `meeting_id`: nothing is recorded yet, so there
    /// is no session for this to be the id of.
    pub key: String,
    pub kind: ReminderKind,
    pub title: String,
    pub provider: String,
    pub participants: Vec<String>,
    /// When the meeting itself starts, where that is known.
    pub starts_at: Option<DateTime<Utc>>,
    /// The conferencing link, when the event carries one. What "Join" opens.
    pub join_url: Option<String>,
    pub fire_at: DateTime<Utc>,
    pub status: ReminderStatus,
}

impl ReminderEvent {
    pub fn key_for_event(event_id: &str) -> String {
        format!("cal:{event_id}")
    }

    pub fn key_for_window(window: &WindowMatch) -> String {
        format!("win:{}:{}", window.provider, window.title.to_lowercase())
    }
}

/// Every tracked reminder, plus how many consecutive ticks each detected window
/// has been seen for. Tauri-managed state.
#[derive(Default)]
pub struct ReminderQueue {
    entries: Mutex<Vec<ReminderEvent>>,
    /// Consecutive sightings per detection key, for the graduation rule below.
    sightings: Mutex<HashMap<String, u32>>,
}

/// How long before a scheduled start an `Upcoming` reminder appears.
///
/// Five minutes is enough to finish a sentence, open the call and press record;
/// much earlier and it is noise the user learns to swipe away.
const UPCOMING_LEAD_SECONDS: i64 = 300;

/// How long after a scheduled start an `Unrecorded` reminder becomes possible.
///
/// It opens late on purpose: a meeting that is five minutes old and still
/// unrecorded is a meeting somebody meant to record and forgot, whereas one
/// that is thirty seconds old is somebody still saying hello.
const UNRECORDED_FROM_SECONDS: i64 = 300;

/// How long the `Unrecorded` window stays open for a meeting with no end time.
///
/// It used to close ninety seconds after it opened, which meant the reminder
/// only ever existed for somebody who was already at their desk, already in the
/// call, and had left Vox running through all of it. Join ten minutes late,
/// start Vox mid-meeting, or wake a laptop, and the meeting ran unrecorded with
/// nothing ever said about it — the failure this app exists to prevent, missed
/// by a ninety-second window. It is open for as long as the meeting is, which
/// costs nothing: one entry is raised per meeting and a dismissal is final.
const UNRECORDED_FALLBACK_MINUTES: i64 = 60;

/// How far either side of a scheduled meeting a conferencing window on screen
/// is still taken to be that meeting rather than a different call.
const SCHEDULED_GRACE_MINUTES: i64 = 5;

/// How long a `Fired` or `Snoozed` reminder can go un-actioned before it is
/// `Expired`.
const EXPIRE_AFTER_MINUTES: i64 = 10;

/// Consecutive sightings a topicless conferencing window needs before it earns
/// a `Detected` reminder.
///
/// A window titled "Zoom Meeting" is as likely to be an idle app as a live
/// call. Requiring it to persist across ticks is what keeps an interruption
/// tied to evidence rather than to a single frame.
const GENERIC_SIGHTINGS_REQUIRED: u32 = 2;

/// Confidence at or above which a detected window is specific enough to
/// interrupt somebody on the first sighting.
const CONFIDENT_DETECTION: f32 = 0.8;

/// Which reminders Vox is allowed to raise.
///
/// One switch per kind rather than one for the feature: they are different
/// evidence about different situations, and somebody who wants to be told
/// their meeting is unrecorded does not necessarily want to be told a Zoom
/// window is open.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ReminderSettings {
    /// A scheduled meeting is about to start.
    #[serde(default = "default_true")]
    pub remind_before_meeting: bool,
    /// A scheduled meeting has started and nothing is recording it.
    #[serde(default = "default_true")]
    pub remind_if_unrecorded: bool,
    /// A conferencing call is on screen that the calendar knows nothing about.
    ///
    /// On, like the other two. This is the only reminder that can catch an
    /// ad-hoc call somebody pulled you into — the exact meeting with no
    /// calendar entry to remind you about it — so defaulting it off left the
    /// one case nothing else covers covered by nothing. The window titles it
    /// reads never leave the machine, and a topicless one has to persist
    /// before it earns an interruption.
    #[serde(default = "default_true")]
    pub remind_on_detection: bool,
    /// Whether `remind_on_detection`'s stored value has been through
    /// [`ReminderSettings::migrate`].
    ///
    /// Not a preference — a marker, so the correction below happens once.
    #[serde(default)]
    pub detection_default_corrected: bool,
}

fn default_true() -> bool {
    true
}

impl Default for ReminderSettings {
    fn default() -> Self {
        Self {
            remind_before_meeting: true,
            remind_if_unrecorded: true,
            remind_on_detection: true,
            detection_default_corrected: true,
        }
    }
}

impl ReminderSettings {
    /// Repairs a `remind_on_detection` that was written out as false by a
    /// version whose default was wrong. Returns whether anything changed.
    ///
    /// Changing a default only reaches installs that never stored a value, and
    /// every install stored one: settings are serialized whole, so saving any
    /// unrelated preference wrote `"remind_on_detection": false` into the file.
    /// Those users were left with detection permanently off and a Settings
    /// switch that looked like it had been chosen deliberately.
    ///
    /// One correction, marked so it never repeats. It costs anybody who did
    /// turn detection off during that window a single re-enable, and their
    /// second switch-off sticks — which is the right way round, because the
    /// alternative leaves everybody else with a reminder that cannot fire.
    pub fn migrate(&mut self) -> bool {
        if self.detection_default_corrected {
            return false;
        }
        self.detection_default_corrected = true;
        if self.remind_on_detection {
            return true;
        }
        self.remind_on_detection = true;
        tracing::info!(
            "[reminders] detected-call reminders switched back on: the stored value came from a version whose default was wrong"
        );
        true
    }

    /// Every kind enabled, whatever the user's preferences say.
    ///
    /// For the developer smoke test only. That button exists to prove the
    /// reminder surface works at all, so a preference must not be able to
    /// silence it: a Send that respects `remind_on_detection` does nothing
    /// when detection is switched off, which looks exactly like the surface
    /// being broken — the failure it was added to rule out.
    pub fn all_enabled() -> Self {
        Self::default()
    }
}

/// Everything `recompute` needs to decide what should exist right now.
pub struct ReminderInputs<'a> {
    pub events: &'a [CalendarEvent],
    pub windows: &'a [WindowMatch],
    pub settings: &'a ReminderSettings,
    /// Whether Vox is recording anything at all. One session runs at a time,
    /// so this answers "is something being recorded right now".
    pub is_recording: bool,
    /// Recordings that already exist, so a meeting somebody recorded and
    /// stopped early is not then reported as unrecorded.
    ///
    /// "Nothing is recording" is not the same question as "this meeting went
    /// unrecorded", and answering the second with the first is what makes a
    /// reminder arrive about a meeting the user already dealt with.
    pub sessions: &'a [Meeting],
    pub now: DateTime<Utc>,
}

/// How far a recording's start may sit from a meeting's start and still be
/// taken as a recording *of* that meeting.
///
/// Generous on purpose: somebody who starts recording eight minutes late has
/// still recorded the meeting, and reminding them otherwise is the false
/// positive that gets reminders switched off.
const SESSION_COVERS_WITHIN_MINUTES: i64 = 30;

/// Whether an existing recording covers this meeting.
///
/// A recording counts whether it is still running or already finished — the
/// question is whether this meeting was captured, not whether it is being
/// captured at this instant.
fn a_recording_covers(
    sessions: &[Meeting],
    event_start: DateTime<Utc>,
    event_end: Option<DateTime<Utc>>,
) -> bool {
    sessions.iter().any(|session| {
        if !matches!(
            session.state,
            MeetingState::Recording
                | MeetingState::Paused
                | MeetingState::Transcribing
                | MeetingState::Completed
        ) {
            return false;
        }

        let Some(started) = DateTime::parse_from_rfc3339(&session.created_at)
            .ok()
            .map(|at| at.with_timezone(&Utc))
        else {
            return false;
        };

        let began_before_the_meeting_ended = event_end.is_none_or(|end| started < end);
        let began_near_the_meeting = (started - event_start).num_minutes().abs()
            <= SESSION_COVERS_WITHIN_MINUTES;

        began_before_the_meeting_ended && began_near_the_meeting
    })
}

fn participants_of(event: &CalendarEvent) -> Vec<String> {
    event
        .others()
        .iter()
        .map(|attendee| {
            attendee
                .display_name
                .as_deref()
                .map(str::trim)
                .filter(|name| !name.is_empty())
                .map(str::to_string)
                .unwrap_or_else(|| attendee.email.split('@').next().unwrap_or("").to_string())
        })
        .filter(|name| !name.is_empty())
        .collect()
}

/// When an event starts, in UTC. `None` when the calendar sent something
/// unparseable, which is a reason to skip it and never to fail a tick.
fn starts_at(event: &CalendarEvent) -> Option<DateTime<Utc>> {
    event.start_timestamp().map(|at| at.with_timezone(&Utc))
}

fn ends_at(event: &CalendarEvent) -> Option<DateTime<Utc>> {
    event.end_timestamp().map(|at| at.with_timezone(&Utc))
}

fn provider_of(event: &CalendarEvent) -> String {
    event
        .conference_url
        .as_deref()
        .map(detection::provider_from_url)
        .unwrap_or(detection::PROVIDER_OTHER)
        .to_string()
}

/// Whether `now` falls inside this event, give or take the grace either side.
///
/// A meeting nobody is in yet, and one that finished, both have windows of
/// their own on somebody's screen — so only the one actually running can claim
/// a live conferencing window as itself.
fn is_in_progress(event: &CalendarEvent, now: DateTime<Utc>) -> bool {
    let Some(start) = starts_at(event) else {
        return false;
    };
    let end = ends_at(event).unwrap_or_else(|| start + Duration::minutes(UNRECORDED_FALLBACK_MINUTES));
    let grace = Duration::minutes(SCHEDULED_GRACE_MINUTES);
    now >= start - grace && now <= end + grace
}

/// Whether a live conferencing window plausibly belongs to this event.
///
/// Matching on provider rather than on title: window titles and calendar titles
/// agree far less often than they look like they should, and a false negative
/// here only costs a reminder, whereas matching on nothing at all resurrects
/// "reminded me about a meeting I had already left".
fn event_is_on_screen(event: &CalendarEvent, windows: &[WindowMatch]) -> bool {
    let provider = provider_of(event);
    if provider == detection::PROVIDER_OTHER {
        // An in-person meeting has no window to find, so its absence proves
        // nothing and must not suppress the reminder.
        return true;
    }
    windows
        .iter()
        .any(|window| window.provider.eq_ignore_ascii_case(&provider))
}

/// Adds an entry unless one already exists for this `(key, kind)`.
///
/// Never overwrites: an existing entry carries the user's snooze or dismissal,
/// and replacing it is how a dismissed reminder comes back.
fn ensure_entry(entries: &mut Vec<ReminderEvent>, candidate: ReminderEvent) {
    if entries
        .iter()
        .any(|e| e.key == candidate.key && e.kind == candidate.kind)
    {
        return;
    }
    entries.push(candidate);
}

/// Why this window would not raise a `Detected` reminder right now.
///
/// `None` means it would. The gates are deliberately the same ones `recompute`
/// applies, in the same order, because the question this answers is the one
/// that is otherwise unanswerable: a reminder that does not arrive and a
/// reminder that was never going to arrive look identical from outside, and
/// every silent gate below has at some point been mistaken for the feature
/// being broken.
pub fn detection_blocker(
    queue: &ReminderQueue,
    window: &WindowMatch,
    inputs: &ReminderInputs<'_>,
) -> Option<String> {
    if !inputs.settings.remind_on_detection {
        return Some("Detected-call reminders are switched off in Settings.".to_string());
    }
    if inputs.is_recording {
        return Some("Vox is already recording, so nothing is missing it.".to_string());
    }
    if inputs
        .events
        .iter()
        .filter(|event| is_in_progress(event, inputs.now))
        .any(|event| provider_of(event).eq_ignore_ascii_case(&window.provider))
    {
        return Some(
            "A scheduled meeting on this provider is running now — covered by the unrecorded reminder instead."
                .to_string(),
        );
    }
    if window.confidence < CONFIDENT_DETECTION {
        let seen = queue
            .sightings
            .lock_or_recover()
            .get(&ReminderEvent::key_for_window(window))
            .copied()
            .unwrap_or(0);
        if seen < GENERIC_SIGHTINGS_REQUIRED {
            return Some(format!(
                "Its title carries no meeting name, so it has to persist: seen {seen} of {GENERIC_SIGHTINGS_REQUIRED} times."
            ));
        }
    }
    None
}

/// Reconciles the queue against what should exist right now.
///
/// Returns `(everything tracked, what became due on this call)`. The second is
/// what the caller notifies on — once per transition into `Fired`, not once per
/// tick while it stays there.
pub fn recompute(queue: &ReminderQueue, inputs: &ReminderInputs<'_>) -> (Vec<ReminderEvent>, Vec<ReminderEvent>) {
    let ReminderInputs {
        events,
        windows,
        settings,
        is_recording,
        sessions,
        now,
    } = *inputs;

    let mut entries = queue.entries.lock_or_recover();

    for event in events {
        let Some(starts_at) = starts_at(event) else {
            continue;
        };
        let ends_at = ends_at(event);
        // A meeting that has already finished has nothing left to remind about.
        if ends_at.is_some_and(|end| end < now) {
            continue;
        }

        let seconds_until_start = (starts_at - now).num_seconds();

        if settings.remind_before_meeting
            && seconds_until_start > 0
            && seconds_until_start <= UPCOMING_LEAD_SECONDS
        {
            ensure_entry(
                &mut entries,
                ReminderEvent {
                    key: ReminderEvent::key_for_event(&event.id),
                    kind: ReminderKind::Upcoming,
                    title: event.title.clone(),
                    provider: provider_of(event),
                    participants: participants_of(event),
                    starts_at: Some(starts_at),
                    join_url: event.conference_url.clone(),
                    fire_at: now,
                    status: ReminderStatus::Pending,
                },
            );
        }

        let seconds_since_start = (now - starts_at).num_seconds();
        let unrecorded_until = ends_at
            .unwrap_or_else(|| starts_at + Duration::minutes(UNRECORDED_FALLBACK_MINUTES));
        if settings.remind_if_unrecorded
            && !is_recording
            && seconds_since_start >= UNRECORDED_FROM_SECONDS
            && now <= unrecorded_until
            && !a_recording_covers(sessions, starts_at, ends_at)
            && event_is_on_screen(event, windows)
        {
            ensure_entry(
                &mut entries,
                ReminderEvent {
                    key: ReminderEvent::key_for_event(&event.id),
                    kind: ReminderKind::Unrecorded,
                    title: event.title.clone(),
                    provider: provider_of(event),
                    participants: participants_of(event),
                    starts_at: Some(starts_at),
                    join_url: event.conference_url.clone(),
                    fire_at: now,
                    status: ReminderStatus::Pending,
                },
            );
        }
    }

    // Detection: only for calls the calendar does not already account for, so a
    // scheduled meeting never produces two reminders about itself.
    //
    // "Accounted for" means a meeting that is happening *now*, not one anywhere
    // in the surrounding two hours. The wider test was the reason detection
    // almost never fired during a working day: a single Meet invitation within
    // an hour either way — one already finished, one not yet started, one
    // declined — blocked every Google Meet window on screen, and the reminder
    // that was supposed to cover the gap only lives inside the scheduled
    // meeting's own few minutes. An ad-hoc call at 11:20 is not explained by an
    // 11:59, and the calendar should not vouch for it.
    let scheduled_providers: Vec<String> = events
        .iter()
        .filter(|event| is_in_progress(event, now))
        .map(provider_of)
        .collect();

    let mut sightings = queue.sightings.lock_or_recover();
    let visible_keys: Vec<String> = windows.iter().map(ReminderEvent::key_for_window).collect();
    sightings.retain(|key, _| visible_keys.contains(key));

    for window in windows {
        let key = ReminderEvent::key_for_window(window);
        let seen = sightings.entry(key.clone()).or_insert(0);
        *seen = seen.saturating_add(1);

        if !settings.remind_on_detection || is_recording {
            continue;
        }
        if scheduled_providers
            .iter()
            .any(|provider| provider.eq_ignore_ascii_case(&window.provider))
        {
            continue;
        }
        // A specific title is evidence enough on its own; a topicless one has
        // to persist before it earns an interruption.
        let earned = window.confidence >= CONFIDENT_DETECTION || *seen >= GENERIC_SIGHTINGS_REQUIRED;
        if !earned {
            continue;
        }

        ensure_entry(
            &mut entries,
            ReminderEvent {
                key,
                kind: ReminderKind::Detected,
                title: window.title.clone(),
                provider: window.provider.clone(),
                participants: Vec::new(),
                starts_at: None,
                join_url: None,
                fire_at: now,
                status: ReminderStatus::Pending,
            },
        );
    }
    drop(sightings);

    let mut newly_fired = Vec::new();
    for entry in entries.iter_mut() {
        let was_fired = matches!(entry.status, ReminderStatus::Fired);

        let due = match &entry.status {
            ReminderStatus::Pending => entry.fire_at <= now,
            ReminderStatus::Snoozed { until } => *until <= now,
            _ => false,
        };

        if due {
            entry.status = ReminderStatus::Fired;
        } else {
            let stale = match &entry.status {
                ReminderStatus::Fired => (now - entry.fire_at).num_minutes() > EXPIRE_AFTER_MINUTES,
                ReminderStatus::Snoozed { until } => {
                    (now - *until).num_minutes() > EXPIRE_AFTER_MINUTES
                }
                _ => false,
            };
            if stale {
                tracing::info!(
                    "[reminders] '{}' ({:?}) expired unactioned after {} minutes",
                    entry.title,
                    entry.kind,
                    EXPIRE_AFTER_MINUTES
                );
                entry.status = ReminderStatus::Expired;
            }
        }

        if !was_fired && matches!(entry.status, ReminderStatus::Fired) {
            newly_fired.push(entry.clone());
        }
    }

    // Drop reminders whose subject is gone: the calendar event ended or was
    // deleted, or the conferencing window closed.
    let live_event_keys: Vec<String> = events
        .iter()
        .filter(|event| ends_at(event).is_none_or(|end| end >= now))
        .map(|event| ReminderEvent::key_for_event(&event.id))
        .collect();
    entries.retain(|entry| match entry.kind {
        ReminderKind::Detected => visible_keys.contains(&entry.key),
        _ => live_event_keys.contains(&entry.key),
    });

    (entries.clone(), newly_fired)
}

/// The next reminder to put on screen, taken from what is waiting to be shown.
///
/// One card is on screen at a time, so reminders that become due while another
/// is up wait in a backlog rather than stacking or — as in the implementation
/// this replaces — being dropped on the floor. Two meetings starting in the
/// same minute is the ordinary case this exists for.
///
/// An entry the user has answered in the meantime is skipped rather than shown
/// late: dismissing the card for one meeting and then being shown a reminder
/// for a meeting already recording is worse than saying nothing.
pub fn take_next_to_show(
    queue: &ReminderQueue,
    backlog: &mut VecDeque<ReminderEvent>,
) -> Option<ReminderEvent> {
    while let Some(candidate) = backlog.pop_front() {
        let still_due = find(queue, &candidate.key, candidate.kind)
            .is_some_and(|entry| matches!(entry.status, ReminderStatus::Fired));
        if still_due {
            return Some(candidate);
        }
    }
    None
}

/// Queues newly-due reminders behind whatever is already waiting.
///
/// Deduplicated on `(key, kind)`: a reminder that was snoozed and came back
/// must not sit in the backlog twice and interrupt twice.
pub fn enqueue_to_show(backlog: &mut VecDeque<ReminderEvent>, newly_fired: Vec<ReminderEvent>) {
    for entry in newly_fired {
        let already_queued = backlog
            .iter()
            .any(|queued| queued.key == entry.key && queued.kind == entry.kind);
        if !already_queued {
            backlog.push_back(entry);
        }
    }
}

/// The reminder the popup should be showing, if any — the earliest-firing entry
/// that is currently `Fired`.
///
/// Derived from the queue rather than stored beside it, so there is nowhere for
/// "the second reminder replaced the first" to reappear.
pub fn current(queue: &ReminderQueue) -> Option<ReminderEvent> {
    queue
        .entries
        .lock_or_recover()
        .iter()
        .filter(|entry| matches!(entry.status, ReminderStatus::Fired))
        .min_by_key(|entry| entry.fire_at)
        .cloned()
}

pub fn find(queue: &ReminderQueue, key: &str, kind: ReminderKind) -> Option<ReminderEvent> {
    queue
        .entries
        .lock_or_recover()
        .iter()
        .find(|entry| entry.key == key && entry.kind == kind)
        .cloned()
}

pub fn dismiss(queue: &ReminderQueue, key: &str, kind: ReminderKind) {
    let mut entries = queue.entries.lock_or_recover();
    if let Some(entry) = entries
        .iter_mut()
        .find(|entry| entry.key == key && entry.kind == kind)
    {
        entry.status = ReminderStatus::Dismissed;
    }
}

pub fn snooze(queue: &ReminderQueue, key: &str, kind: ReminderKind, minutes: i64) {
    let mut entries = queue.entries.lock_or_recover();
    if let Some(entry) = entries
        .iter_mut()
        .find(|entry| entry.key == key && entry.kind == kind)
    {
        entry.status = ReminderStatus::Snoozed {
            until: Utc::now() + Duration::minutes(minutes),
        };
    }
}

/// Resolves every open reminder for one subject as `Actioned`.
///
/// Called as a side effect of starting that meeting's recording, wherever that
/// was triggered from — the popup, the meetings list, or the tray. One function
/// clearing all of them is what keeps those entry points from disagreeing about
/// whether a meeting has been seen (Decision 45, Refactor #1).
pub fn mark_actioned(queue: &ReminderQueue, key: &str) {
    let mut entries = queue.entries.lock_or_recover();
    for entry in entries.iter_mut().filter(|entry| entry.key == key) {
        if matches!(
            entry.status,
            ReminderStatus::Pending | ReminderStatus::Fired | ReminderStatus::Snoozed { .. }
        ) {
            entry.status = ReminderStatus::Actioned;
        }
    }
}

/// Testing only: injects an already-`Fired` reminder, bypassing the timing and
/// detection gates.
///
/// Backs Settings › Developer's mock reminder buttons, so the popup's real
/// actions can be exercised without waiting for a real meeting.
pub fn inject_mock(queue: &ReminderQueue, kind: ReminderKind) -> ReminderEvent {
    let (title, provider, join_url) = match kind {
        ReminderKind::Upcoming => (
            "Placement review",
            detection::PROVIDER_GOOGLE_MEET,
            Some("https://meet.google.com/abc-defg-hij".to_string()),
        ),
        ReminderKind::Unrecorded => ("Sprint planning", detection::PROVIDER_ZOOM, None),
        ReminderKind::Detected => ("Google Meet Session", detection::PROVIDER_GOOGLE_MEET, None),
    };

    let event = ReminderEvent {
        key: format!("mock:{kind:?}").to_lowercase(),
        kind,
        title: title.to_string(),
        provider: provider.to_string(),
        participants: vec!["Pranjali Sharma".to_string(), "Ayush Kumar".to_string()],
        starts_at: Some(Utc::now() + Duration::minutes(2)),
        join_url,
        fire_at: Utc::now(),
        status: ReminderStatus::Fired,
    };

    let mut entries = queue.entries.lock_or_recover();
    entries.retain(|entry| entry.key != event.key);
    entries.push(event.clone());
    event
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::calendar::model::{Attendance, EventAttendee};
    use crate::meetings::model::MeetingSource;

    #[test]
    fn every_reminder_kind_is_on_out_of_the_box() {
        // A reminder nobody switched off has to arrive. Detection shipped
        // defaulted off, which left the one call with no calendar entry — the
        // only kind nothing else can catch — covered by nothing at all.
        let settings = ReminderSettings::default();
        assert!(settings.remind_before_meeting);
        assert!(settings.remind_if_unrecorded);
        assert!(settings.remind_on_detection);
    }

    #[test]
    fn a_detection_flag_stored_by_the_wrong_default_is_corrected_once() {
        // Settings are serialized whole, so saving any unrelated preference
        // wrote the old default into the file. Changing the default in code
        // reaches nobody who has ever opened Settings; this is what does.
        let mut stored: ReminderSettings =
            serde_json::from_str(r#"{"remind_on_detection": false}"#).unwrap();
        assert!(!stored.remind_on_detection);

        assert!(stored.migrate());
        assert!(stored.remind_on_detection);

        // And having been told once, it stays told: a user who switches
        // detection off after the correction keeps it off.
        stored.remind_on_detection = false;
        assert!(!stored.migrate());
        assert!(!stored.remind_on_detection);
    }

    fn settings_all_on() -> ReminderSettings {
        ReminderSettings::default()
    }

    fn event_starting_in(id: &str, seconds: i64) -> CalendarEvent {
        let start = Utc::now() + Duration::seconds(seconds);
        CalendarEvent {
            id: id.to_string(),
            account_email: "me@work.com".to_string(),
            calendar_id: Some("primary".to_string()),
            title: format!("Meeting {id}"),
            start: start.to_rfc3339(),
            end: (start + Duration::minutes(30)).to_rfc3339(),
            all_day: false,
            location: None,
            conference_url: Some("https://meet.google.com/abc-defg-hij".to_string()),
            html_link: None,
            description: None,
            recurring_event_id: None,
            attendees: vec![EventAttendee {
                email: "pranjali@example.com".to_string(),
                display_name: Some("Pranjali Sharma".to_string()),
                response: Attendance::Accepted,
                is_self: false,
                organizer: true,
            }],
            attendance: Attendance::Accepted,
            meeting_id: None,
        }
    }

    fn zoom_window(title: &str) -> WindowMatch {
        WindowMatch {
            provider: detection::PROVIDER_ZOOM.to_string(),
            title: title.to_string(),
            raw_title: format!("{title} - Zoom"),
            source: detection::SOURCE_WINDOW_TITLE.to_string(),
            confidence: detection::score_confidence(title),
        }
    }

    fn inputs<'a>(
        events: &'a [CalendarEvent],
        windows: &'a [WindowMatch],
        settings: &'a ReminderSettings,
        is_recording: bool,
    ) -> ReminderInputs<'a> {
        ReminderInputs {
            events,
            windows,
            settings,
            is_recording,
            sessions: &[],
            now: Utc::now(),
        }
    }

    fn session_started_at(state: MeetingState, started: DateTime<Utc>) -> Meeting {
        let mut session = Meeting::new(
            "meet_test".to_string(),
            "Recorded".to_string(),
            MeetingSource::Recorded,
        );
        session.state = state;
        session.created_at = started.to_rfc3339();
        session
    }

    #[test]
    fn a_meeting_inside_the_lead_window_fires_once_and_stays_fired() {
        let settings = settings_all_on();
        let events = vec![event_starting_in("evt_1", 120)];
        let queue = ReminderQueue::default();

        let (all, fired) = recompute(&queue, &inputs(&events, &[], &settings, false));
        assert_eq!(fired.len(), 1, "the reminder becomes due on this call");
        assert!(matches!(all[0].status, ReminderStatus::Fired));

        // A second pass while still due must not raise a second notification.
        let (_, fired_again) = recompute(&queue, &inputs(&events, &[], &settings, false));
        assert!(fired_again.is_empty(), "already-fired reminders stay quiet");
    }

    #[test]
    fn a_meeting_beyond_the_lead_window_is_not_yet_a_reminder() {
        let settings = settings_all_on();
        let events = vec![event_starting_in("evt_1", UPCOMING_LEAD_SECONDS + 120)];
        let queue = ReminderQueue::default();

        let (all, fired) = recompute(&queue, &inputs(&events, &[], &settings, false));
        assert!(all.is_empty());
        assert!(fired.is_empty());
    }

    #[test]
    fn a_second_reminder_never_erases_the_first() {
        // The single-slot design this replaces lost one of these entirely.
        let settings = settings_all_on();
        let events = vec![event_starting_in("evt_1", 60), event_starting_in("evt_2", 90)];
        let queue = ReminderQueue::default();

        let (all, fired) = recompute(&queue, &inputs(&events, &[], &settings, false));
        assert_eq!(all.len(), 2);
        assert_eq!(fired.len(), 2);
    }

    #[test]
    fn an_unrecorded_reminder_needs_the_call_to_still_be_on_screen() {
        let settings = settings_all_on();
        let mut event = event_starting_in("evt_1", -330);
        event.conference_url = Some("https://zoom.us/j/123".to_string());
        let events = vec![event];

        // Nobody is on the call any more — the user left, so there is nothing
        // to record and nothing to say.
        let queue = ReminderQueue::default();
        let (all, _) = recompute(&queue, &inputs(&events, &[], &settings, false));
        assert!(all.iter().all(|e| e.kind != ReminderKind::Unrecorded));

        // With Zoom actually open, the reminder is real.
        let queue = ReminderQueue::default();
        let windows = vec![zoom_window("Sprint planning")];
        let (all, _) = recompute(&queue, &inputs(&events, &windows, &settings, false));
        assert!(all.iter().any(|e| e.kind == ReminderKind::Unrecorded));
    }

    #[test]
    fn nothing_is_raised_while_a_recording_is_already_running() {
        let settings = settings_all_on();
        let mut event = event_starting_in("evt_1", -330);
        event.conference_url = Some("https://zoom.us/j/123".to_string());
        let events = vec![event];
        let windows = vec![zoom_window("Sprint planning")];
        let queue = ReminderQueue::default();

        let (all, _) = recompute(&queue, &inputs(&events, &windows, &settings, true));
        assert!(all.iter().all(|e| e.kind != ReminderKind::Unrecorded));
        assert!(all.iter().all(|e| e.kind != ReminderKind::Detected));
    }

    #[test]
    fn a_topicless_window_has_to_persist_before_it_interrupts() {
        let settings = settings_all_on();
        let windows = vec![zoom_window("Zoom Meeting")];
        let queue = ReminderQueue::default();

        let (all, _) = recompute(&queue, &inputs(&[], &windows, &settings, false));
        assert!(all.is_empty(), "one sighting of a generic title proves nothing");

        let (all, fired) = recompute(&queue, &inputs(&[], &windows, &settings, false));
        assert_eq!(all.len(), 1);
        assert_eq!(fired.len(), 1);
    }

    #[test]
    fn a_named_call_is_evidence_enough_on_the_first_sighting() {
        let settings = settings_all_on();
        let windows = vec![zoom_window("Placement review")];
        let queue = ReminderQueue::default();

        let (_, fired) = recompute(&queue, &inputs(&[], &windows, &settings, false));
        assert_eq!(fired.len(), 1);
        assert_eq!(fired[0].kind, ReminderKind::Detected);
    }

    #[test]
    fn a_scheduled_call_does_not_also_report_itself_as_detected() {
        let settings = settings_all_on();
        let mut event = event_starting_in("evt_1", -60);
        event.conference_url = Some("https://zoom.us/j/123".to_string());
        let events = vec![event];
        let windows = vec![zoom_window("Sprint planning")];
        let queue = ReminderQueue::default();

        let (all, _) = recompute(&queue, &inputs(&events, &windows, &settings, false));
        assert!(all.iter().all(|e| e.kind != ReminderKind::Detected));
    }

    #[test]
    fn a_meeting_elsewhere_in_the_hour_does_not_vouch_for_the_call_on_screen() {
        // The reason detection almost never fired during a working day. Any
        // event within an hour either way used to block every window of its
        // provider — so one Zoom invitation at 12:00 silenced an ad-hoc Zoom
        // at 11:10, which nothing else covers.
        let settings = settings_all_on();
        let mut later = event_starting_in("evt_later", 50 * 60);
        later.conference_url = Some("https://zoom.us/j/123".to_string());
        let mut finished = event_starting_in("evt_earlier", -50 * 60);
        finished.conference_url = Some("https://zoom.us/j/456".to_string());
        let events = vec![later, finished];
        let windows = vec![zoom_window("Ad-hoc with the placement team")];
        let queue = ReminderQueue::default();

        let (all, _) = recompute(&queue, &inputs(&events, &windows, &settings, false));

        assert!(
            all.iter().any(|e| e.kind == ReminderKind::Detected),
            "no meeting was running, so the calendar explains nothing about this call"
        );
    }

    #[test]
    fn a_meeting_joined_late_is_still_reported_unrecorded() {
        // The window used to close ninety seconds after it opened, so the
        // reminder existed only for somebody already at their desk, already in
        // the call, with Vox already running. Everybody else recorded nothing
        // and heard nothing about it.
        let settings = settings_all_on();
        let events = vec![event_starting_in("evt_1", -20 * 60)];
        let windows = vec![WindowMatch {
            provider: detection::PROVIDER_GOOGLE_MEET.to_string(),
            title: "Meeting evt_1".to_string(),
            raw_title: "Meeting evt_1 - Google Meet".to_string(),
            source: detection::SOURCE_WINDOW_TITLE.to_string(),
            confidence: 0.85,
        }];
        let queue = ReminderQueue::default();

        let (all, _) = recompute(&queue, &inputs(&events, &windows, &settings, false));

        assert!(
            all.iter().any(|e| e.kind == ReminderKind::Unrecorded),
            "twenty minutes into an unrecorded meeting is exactly when to say so"
        );
    }

    #[test]
    fn a_meeting_that_has_finished_is_not_still_called_unrecorded() {
        // The other half of widening the window: it follows the meeting's own
        // end, so a call that ran to 11:30 stops being nagged about at 11:31.
        let settings = settings_all_on();
        let mut event = event_starting_in("evt_1", -90 * 60);
        event.end = (Utc::now() - Duration::minutes(60)).to_rfc3339();
        let events = vec![event];
        let windows = vec![zoom_window("Sprint planning")];
        let queue = ReminderQueue::default();

        let (all, _) = recompute(&queue, &inputs(&events, &windows, &settings, false));

        assert!(all.iter().all(|e| e.kind != ReminderKind::Unrecorded));
    }

    #[test]
    fn a_closed_window_drops_its_reminder() {
        let settings = settings_all_on();
        let windows = vec![zoom_window("Placement review")];
        let queue = ReminderQueue::default();
        recompute(&queue, &inputs(&[], &windows, &settings, false));

        let (all, _) = recompute(&queue, &inputs(&[], &[], &settings, false));
        assert!(all.is_empty(), "the call ended, so the reminder goes with it");
    }

    #[test]
    fn a_snoozed_reminder_stays_quiet_until_its_time() {
        let settings = settings_all_on();
        let events = vec![event_starting_in("evt_1", 120)];
        let queue = ReminderQueue::default();
        recompute(&queue, &inputs(&events, &[], &settings, false));

        snooze(&queue, "cal:evt_1", ReminderKind::Upcoming, 5);
        let (all, fired) = recompute(&queue, &inputs(&events, &[], &settings, false));
        assert!(fired.is_empty());
        assert!(matches!(all[0].status, ReminderStatus::Snoozed { .. }));

        // Rewinding the snooze to the past is what "5 minutes later" looks like
        // without sleeping in a test.
        snooze(&queue, "cal:evt_1", ReminderKind::Upcoming, -1);
        let (_, fired) = recompute(&queue, &inputs(&events, &[], &settings, false));
        assert_eq!(fired.len(), 1, "the snooze elapsed, so it comes back once");
    }

    #[test]
    fn a_dismissal_is_final_for_that_kind_and_leaves_the_others_alone() {
        let settings = settings_all_on();
        let events = vec![event_starting_in("evt_1", 120)];
        let queue = ReminderQueue::default();
        recompute(&queue, &inputs(&events, &[], &settings, false));

        dismiss(&queue, "cal:evt_1", ReminderKind::Upcoming);
        let (all, fired) = recompute(&queue, &inputs(&events, &[], &settings, false));
        assert!(fired.is_empty());
        assert!(matches!(all[0].status, ReminderStatus::Dismissed));
    }

    #[test]
    fn starting_a_recording_resolves_every_reminder_for_that_meeting() {
        let settings = settings_all_on();
        let events = vec![event_starting_in("evt_1", 120)];
        let queue = ReminderQueue::default();
        recompute(&queue, &inputs(&events, &[], &settings, false));

        mark_actioned(&queue, "cal:evt_1");
        let entries = queue.entries.lock_or_recover();
        assert!(entries
            .iter()
            .filter(|e| e.key == "cal:evt_1")
            .all(|e| matches!(e.status, ReminderStatus::Actioned)));
    }

    #[test]
    fn each_reminder_kind_can_be_switched_off_on_its_own() {
        let settings = ReminderSettings {
            remind_before_meeting: false,
            ..ReminderSettings::default()
        };
        let events = vec![event_starting_in("evt_1", 120)];
        let queue = ReminderQueue::default();

        let (all, _) = recompute(&queue, &inputs(&events, &[], &settings, false));
        assert!(all.is_empty());
    }

    #[test]
    fn the_popup_shows_the_earliest_fired_reminder() {
        let settings = settings_all_on();
        let events = vec![event_starting_in("evt_1", 60), event_starting_in("evt_2", 90)];
        let queue = ReminderQueue::default();
        recompute(&queue, &inputs(&events, &[], &settings, false));

        assert!(current(&queue).is_some());
        dismiss(&queue, "cal:evt_1", ReminderKind::Upcoming);
        dismiss(&queue, "cal:evt_2", ReminderKind::Upcoming);
        assert!(current(&queue).is_none());
    }

    #[test]
    fn a_meeting_that_was_already_recorded_is_not_called_unrecorded() {
        // The user recorded this meeting and stopped early. Nothing is
        // recording *now*, but the meeting did not go uncaptured — and saying
        // it did is the false positive that gets reminders switched off.
        let settings = settings_all_on();
        let mut event = event_starting_in("evt_1", -330);
        event.conference_url = Some("https://zoom.us/j/123".to_string());
        let events = vec![event];
        let windows = vec![zoom_window("Sprint planning")];
        let finished = vec![session_started_at(
            MeetingState::Completed,
            Utc::now() - Duration::seconds(330),
        )];

        let queue = ReminderQueue::default();
        let (all, _) = recompute(
            &queue,
            &ReminderInputs {
                events: &events,
                windows: &windows,
                settings: &settings,
                is_recording: false,
                sessions: &finished,
                now: Utc::now(),
            },
        );
        assert!(all.iter().all(|e| e.kind != ReminderKind::Unrecorded));
    }

    #[test]
    fn a_recording_of_some_other_meeting_does_not_cover_this_one() {
        let settings = settings_all_on();
        let mut event = event_starting_in("evt_1", -330);
        event.conference_url = Some("https://zoom.us/j/123".to_string());
        let events = vec![event];
        let windows = vec![zoom_window("Sprint planning")];
        // Recorded three hours ago — a different meeting entirely.
        let unrelated = vec![session_started_at(
            MeetingState::Completed,
            Utc::now() - Duration::hours(3),
        )];

        let queue = ReminderQueue::default();
        let (all, _) = recompute(
            &queue,
            &ReminderInputs {
                events: &events,
                windows: &windows,
                settings: &settings,
                is_recording: false,
                sessions: &unrelated,
                now: Utc::now(),
            },
        );
        assert!(all.iter().any(|e| e.kind == ReminderKind::Unrecorded));
    }

    #[test]
    fn two_reminders_due_at_once_are_both_shown_in_turn() {
        // One card is on screen at a time. The second must wait, not vanish —
        // this is the failure the single-slot design was known for.
        let settings = settings_all_on();
        let events = vec![event_starting_in("evt_1", 60), event_starting_in("evt_2", 90)];
        let queue = ReminderQueue::default();
        let (_, newly_fired) = recompute(&queue, &inputs(&events, &[], &settings, false));

        let mut backlog = VecDeque::new();
        enqueue_to_show(&mut backlog, newly_fired);

        let first = take_next_to_show(&queue, &mut backlog).expect("a card to show");
        let second = take_next_to_show(&queue, &mut backlog).expect("the second still waiting");
        assert_ne!(first.key, second.key);
        assert!(take_next_to_show(&queue, &mut backlog).is_none());
    }

    #[test]
    fn a_reminder_answered_while_another_was_on_screen_is_not_shown_late() {
        let settings = settings_all_on();
        let events = vec![event_starting_in("evt_1", 60), event_starting_in("evt_2", 90)];
        let queue = ReminderQueue::default();
        let (_, newly_fired) = recompute(&queue, &inputs(&events, &[], &settings, false));

        let mut backlog = VecDeque::new();
        enqueue_to_show(&mut backlog, newly_fired);
        take_next_to_show(&queue, &mut backlog).expect("the first card");

        // The user starts recording the second meeting from the Meetings list
        // while the first card is still up.
        mark_actioned(&queue, "cal:evt_2");
        assert!(take_next_to_show(&queue, &mut backlog).is_none());
    }

    #[test]
    fn the_same_reminder_never_waits_in_the_backlog_twice() {
        let settings = settings_all_on();
        let events = vec![event_starting_in("evt_1", 60)];
        let queue = ReminderQueue::default();
        let (_, newly_fired) = recompute(&queue, &inputs(&events, &[], &settings, false));

        let mut backlog = VecDeque::new();
        enqueue_to_show(&mut backlog, newly_fired.clone());
        enqueue_to_show(&mut backlog, newly_fired);
        assert_eq!(backlog.len(), 1);
    }

    #[test]
    fn a_finished_meeting_is_not_reminded_about() {
        let settings = settings_all_on();
        let mut event = event_starting_in("evt_1", -3600);
        event.end = (Utc::now() - Duration::minutes(30)).to_rfc3339();
        let queue = ReminderQueue::default();

        let (all, _) = recompute(&queue, &inputs(&[event], &[], &settings, false));
        assert!(all.is_empty());
    }
}
