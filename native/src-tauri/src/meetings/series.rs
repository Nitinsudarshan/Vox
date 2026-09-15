//! Recurring meetings, as one thing rather than as twelve rows.
//!
//! A weekly standup recorded for three months is twelve meetings with twelve
//! near-identical names, ordered only by date. The question a person actually
//! has — *what has happened across this meeting* — cannot be asked, because
//! there is no object that is "the weekly standup".
//!
//! ## Where the identity comes from
//!
//! From Google, not from a guess. `singleEvents=true` (what
//! [`crate::calendar::google::list_events`] already asks for) expands a
//! recurrence rule into the occurrences a person attends, and every occurrence
//! carries `recurringEventId` — the parent's id, the same on all of them, and
//! unchanged by a rename or a moved occurrence.
//!
//! Inferring a series from matching titles and times was considered and
//! rejected: two teams both running a "Weekly Sync" at 10:00 would merge into
//! one series, silently, with no way for the user to tell that it happened.
//!
//! ## Where membership lives
//!
//! On the meeting ([`super::model::Meeting::series_id`]), written once during
//! a sync and never cleared by one. Deriving it live from the event cache
//! would look identical for a month and then break: the cache is a rolling
//! window (30 days each way) and an occurrence outside it no longer exists to
//! join against.
//!
//! One meeting belongs to at most one series. A recording is one conversation,
//! and letting it sit in two series makes both of them wrong.

use serde::{Deserialize, Serialize};

use super::model::Meeting;

/// Prefix for a series Google told us about.
const GOOGLE_PREFIX: &str = "google:";

/// Prefix for a series the user made by hand.
const MANUAL_PREFIX: &str = "manual:";

/// Where a series came from.
///
/// Worth keeping apart because the two behave differently: a Google series
/// gains occurrences on its own as meetings are recorded against its events,
/// and a manual one only ever gains what the user puts in it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SeriesSource {
    Google,
    Manual,
}

/// A recurring meeting.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MeetingSeries {
    /// `google:<recurringEventId>` or `manual:<slug>-<timestamp>`.
    pub id: String,
    /// What it is called. The user's rename wins over the calendar's title.
    pub title: String,
    pub source: SeriesSource,
    pub created_at: String,
    /// Whether the user has renamed it, so a later sync does not undo that.
    #[serde(default)]
    pub renamed_by_user: bool,
}

/// One series with what the list needs to show about it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MeetingSeriesSummary {
    #[serde(flatten)]
    pub series: MeetingSeries,
    /// How many recordings are in it.
    pub occurrence_count: usize,
    /// When the most recent one was recorded, if any.
    pub latest_at: Option<String>,
}

/// One recording's place in its series.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SeriesOccurrence {
    pub meeting_id: String,
    pub title: String,
    pub created_at: String,
    pub duration_seconds: f64,
    pub has_summary: bool,
}

/// The series id for a Google recurring event.
///
/// Namespaced by the recurring event alone, never by the account. An
/// invitation that reached two accounts is de-duplicated by
/// [`crate::calendar::agenda::deduplicate`], and which copy survives depends
/// on who answered — so putting the account in the key would split one series
/// in half the first time the surviving copy changed account.
pub fn google_series_id(recurring_event_id: &str) -> String {
    format!("{GOOGLE_PREFIX}{}", recurring_event_id.trim())
}

/// A stable id for a series the user is creating by hand.
///
/// The title is slugged into it so the file is readable, and a timestamp keeps
/// two series called the same thing apart — a user who names two series
/// "Sync" means two series.
pub fn manual_series_id(title: &str, now_millis: i64) -> String {
    let slug: String = title
        .trim()
        .to_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect::<String>()
        .split('-')
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("-")
        .chars()
        .take(40)
        .collect();
    let slug = if slug.is_empty() {
        "series".to_string()
    } else {
        slug
    };
    format!("{MANUAL_PREFIX}{slug}-{now_millis}")
}

impl MeetingSeries {
    pub fn is_google(&self) -> bool {
        self.source == SeriesSource::Google
    }
}

/// One meeting that should be stamped with a series, and which one.
#[derive(Debug, Clone, PartialEq)]
pub struct SeriesAssignment {
    pub meeting_id: String,
    pub series_id: String,
    /// The event's title, which names the series when it is first seen.
    pub title: String,
}

/// Which recordings belong to which Google series, from matched events.
///
/// Takes events that [`crate::calendar::agenda::match_recordings`] has already
/// linked to recordings: an event with both a `meeting_id` and a
/// `recurring_event_id` is a recording of one occurrence of a series, which is
/// the only evidence this needs.
///
/// Each meeting appears once. An event that matched no recording, or that is
/// not part of a recurrence, contributes nothing — a recording is left out of
/// a series rather than guessed into one.
pub fn assignments_from_events<'a, I>(events: I) -> Vec<SeriesAssignment>
where
    I: IntoIterator<Item = &'a crate::calendar::CalendarEvent>,
{
    let mut seen: Vec<String> = Vec::new();
    let mut assignments = Vec::new();

    for event in events {
        let (Some(meeting_id), Some(recurring)) =
            (event.meeting_id.as_deref(), event.recurring_event_id.as_deref())
        else {
            continue;
        };
        if seen.iter().any(|id| id == meeting_id) {
            continue;
        }
        seen.push(meeting_id.to_string());
        assignments.push(SeriesAssignment {
            meeting_id: meeting_id.to_string(),
            series_id: google_series_id(recurring),
            title: event.title.clone(),
        });
    }
    assignments
}

/// The recordings in one series, oldest first.
///
/// Oldest first because a series is read forwards: "what did we decide in week
/// one, and what has changed since" is the question, and the list that answers
/// it starts at the beginning. The meetings list itself is newest-first, which
/// is right for a list of what happened lately and wrong for a series.
pub fn occurrences<'a>(meetings: &'a [Meeting], series_id: &str) -> Vec<&'a Meeting> {
    let mut members: Vec<&Meeting> = meetings
        .iter()
        .filter(|meeting| meeting.series_id.as_deref() == Some(series_id))
        .collect();
    members.sort_by(|a, b| a.created_at.cmp(&b.created_at).then_with(|| a.id.cmp(&b.id)));
    members
}

/// Where one recording sits in its series: `(position, total)`, 1-based.
///
/// `None` when the meeting is not in a series, or — defensively — when it
/// claims a series it is not a member of, which would otherwise render as
/// "0 of 4".
pub fn position(meetings: &[Meeting], meeting_id: &str, series_id: &str) -> Option<(usize, usize)> {
    let members = occurrences(meetings, series_id);
    let index = members.iter().position(|meeting| meeting.id == meeting_id)?;
    Some((index + 1, members.len()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::calendar::model::{Attendance, CalendarEvent};
    use crate::meetings::model::MeetingSource;

    fn event(id: &str, title: &str, meeting: Option<&str>, recurring: Option<&str>) -> CalendarEvent {
        CalendarEvent {
            id: id.to_string(),
            account_email: "me@work.com".into(),
            calendar_id: Some("primary".into()),
            title: title.to_string(),
            start: "2026-09-14T10:00:00Z".into(),
            end: "2026-09-14T10:30:00Z".into(),
            all_day: false,
            location: None,
            conference_url: None,
            html_link: None,
            description: None,
            recurring_event_id: recurring.map(str::to_string),
            attendees: Vec::new(),
            attendance: Attendance::Accepted,
            meeting_id: meeting.map(str::to_string),
        }
    }

    fn meeting(id: &str, created_at: &str, series: Option<&str>) -> Meeting {
        let mut record = Meeting::new(id.to_string(), "Standup".into(), MeetingSource::Recorded);
        record.created_at = created_at.to_string();
        record.series_id = series.map(str::to_string);
        record
    }

    #[test]
    fn a_recorded_occurrence_is_assigned_to_its_series() {
        let events = vec![event("evt_1", "Weekly Sync", Some("m-1"), Some("rec-abc"))];
        let assignments = assignments_from_events(&events);
        assert_eq!(assignments.len(), 1);
        assert_eq!(assignments[0].meeting_id, "m-1");
        assert_eq!(assignments[0].series_id, "google:rec-abc");
        assert_eq!(assignments[0].title, "Weekly Sync");
    }

    #[test]
    fn a_one_off_meeting_is_left_out_rather_than_guessed_into_a_series() {
        let events = vec![
            event("evt_1", "Coffee", Some("m-1"), None),
            event("evt_2", "Weekly Sync", None, Some("rec-abc")),
        ];
        assert!(assignments_from_events(&events).is_empty());
    }

    #[test]
    fn one_recording_is_assigned_once_even_when_two_accounts_carry_the_event() {
        // De-duplication normally removes the second copy, but the assignment
        // pass must not depend on having been handed a de-duplicated list.
        let mut second = event("evt_1", "Weekly Sync", Some("m-1"), Some("rec-abc"));
        second.account_email = "me@gmail.com".into();
        let events = vec![event("evt_1", "Weekly Sync", Some("m-1"), Some("rec-abc")), second];
        assert_eq!(assignments_from_events(&events).len(), 1);
    }

    #[test]
    fn the_series_id_does_not_depend_on_which_account_answered() {
        // Namespacing by account would split one series the first time the
        // surviving copy of a de-duplicated invitation changed account.
        let work = event("evt_1", "Weekly Sync", Some("m-1"), Some("rec-abc"));
        let mut personal = event("evt_1", "Weekly Sync", Some("m-2"), Some("rec-abc"));
        personal.account_email = "me@gmail.com".into();

        let assignments = assignments_from_events(&[work, personal]);
        assert_eq!(assignments[0].series_id, assignments[1].series_id);
    }

    #[test]
    fn a_series_reads_forwards_even_though_the_meetings_list_reads_backwards() {
        let meetings = vec![
            meeting("m-3", "2026-09-28T10:00:00Z", Some("google:rec-abc")),
            meeting("m-1", "2026-09-14T10:00:00Z", Some("google:rec-abc")),
            meeting("m-2", "2026-09-21T10:00:00Z", Some("google:rec-abc")),
        ];
        let ordered: Vec<&str> = occurrences(&meetings, "google:rec-abc")
            .iter()
            .map(|meeting| meeting.id.as_str())
            .collect();
        assert_eq!(ordered, vec!["m-1", "m-2", "m-3"]);
    }

    #[test]
    fn a_meeting_from_another_series_is_not_in_this_one() {
        let meetings = vec![
            meeting("m-1", "2026-09-14T10:00:00Z", Some("google:rec-abc")),
            meeting("m-2", "2026-09-14T11:00:00Z", Some("google:rec-xyz")),
            meeting("m-3", "2026-09-14T12:00:00Z", None),
        ];
        assert_eq!(occurrences(&meetings, "google:rec-abc").len(), 1);
    }

    #[test]
    fn a_recording_knows_where_it_sits_in_its_series() {
        let meetings = vec![
            meeting("m-1", "2026-09-14T10:00:00Z", Some("google:rec-abc")),
            meeting("m-2", "2026-09-21T10:00:00Z", Some("google:rec-abc")),
            meeting("m-3", "2026-09-28T10:00:00Z", Some("google:rec-abc")),
        ];
        assert_eq!(position(&meetings, "m-2", "google:rec-abc"), Some((2, 3)));
        assert_eq!(position(&meetings, "m-3", "google:rec-abc"), Some((3, 3)));
    }

    #[test]
    fn a_meeting_that_is_not_a_member_reports_no_position_rather_than_zero_of_four() {
        let meetings = vec![meeting("m-1", "2026-09-14T10:00:00Z", Some("google:rec-abc"))];
        assert_eq!(position(&meetings, "m-9", "google:rec-abc"), None);
    }

    #[test]
    fn a_manual_series_id_is_readable_and_two_of_the_same_name_stay_apart() {
        let first = manual_series_id("Weekly Sync", 1_757_000_000_000);
        assert_eq!(first, "manual:weekly-sync-1757000000000");
        assert_ne!(first, manual_series_id("Weekly Sync", 1_757_000_000_001));
    }

    #[test]
    fn a_title_of_nothing_but_punctuation_still_produces_a_usable_id() {
        let id = manual_series_id("!!! ???", 1_757_000_000_000);
        assert_eq!(id, "manual:series-1757000000000");
    }

    #[test]
    fn a_google_series_id_is_namespaced_so_it_cannot_collide_with_a_manual_one() {
        assert!(google_series_id("rec-abc").starts_with(GOOGLE_PREFIX));
        assert!(manual_series_id("x", 1).starts_with(MANUAL_PREFIX));
        assert_ne!(google_series_id("x-1"), manual_series_id("x", 1));
    }
}
