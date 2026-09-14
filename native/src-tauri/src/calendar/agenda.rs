//! One day, out of several calendars.
//!
//! The whole point of the calendar integration is here. Reading one account is
//! a feature nobody needed: a working person keeps work in one Google account
//! and life in another, the two never merge, and "what am I doing today" is
//! the union. Five work meetings plus one personal one is six meetings, in one
//! list, in time order — and it has to *be* six, which means the invitation
//! that lands in both inboxes appears once.

use std::collections::BTreeMap;

use chrono::{DateTime, Datelike, FixedOffset, Local, TimeZone};

use super::model::{Attendance, CalendarEvent, DayAgenda};

/// Merges events from every account into days.
///
/// De-duplicated, then sorted, then grouped — in that order, because the copy
/// that survives de-duplication decides which account the row is labelled
/// with, and sorting before grouping is what makes each day's order the
/// timeline rather than the order the accounts happened to sync in.
pub fn build_agenda(events: Vec<CalendarEvent>) -> Vec<DayAgenda> {
    let mut by_day: BTreeMap<String, Vec<CalendarEvent>> = BTreeMap::new();
    for event in deduplicate(events) {
        let Some(start) = event.start_timestamp() else {
            continue;
        };
        by_day.entry(local_day(start)).or_default().push(event);
    }

    by_day
        .into_iter()
        .map(|(date, mut events)| {
            events.sort_by(|a, b| {
                // All-day events first: they are the day's context, not a slot
                // in it, and interleaving them by start time puts "Diwali"
                // between two morning calls.
                b.all_day
                    .cmp(&a.all_day)
                    .then_with(|| a.start.cmp(&b.start))
                    .then_with(|| a.title.cmp(&b.title))
            });
            DayAgenda { date, events }
        })
        .collect()
}

/// `YYYY-MM-DD` in the viewer's own timezone.
///
/// Converted rather than taken from the string: an event at 00:30 IST is
/// stored with a `+05:30` offset and belongs to that day, not to the UTC day
/// before it.
fn local_day(when: DateTime<FixedOffset>) -> String {
    let local = when.with_timezone(&Local);
    format!("{:04}-{:02}-{:02}", local.year(), local.month(), local.day())
}

/// Drops the second copy of one invitation that reached two accounts.
///
/// Google gives every copy of an invitation the same event id, so the id is
/// the signal. Where that fails — an event forwarded rather than invited, and
/// so genuinely a different event id — identical title and start is taken as
/// the same meeting, because two meetings with the same name at the same
/// second is a calendar mistake and showing it twice does not help anyone fix
/// it.
///
/// The copy that survives is the one where the user has actually responded:
/// a row that says "you accepted" is more useful than the same row saying
/// "not answered" from the account the invitation was only cc'd to.
pub fn deduplicate(events: Vec<CalendarEvent>) -> Vec<CalendarEvent> {
    let mut kept: Vec<CalendarEvent> = Vec::new();

    for event in events {
        let existing = kept.iter_mut().find(|other| is_same_meeting(other, &event));
        match existing {
            Some(other) => {
                if response_rank(event.attendance) > response_rank(other.attendance) {
                    // Keep the better-informed copy, but never lose a
                    // conference link the other copy happened to carry.
                    let conference = other.conference_url.take().or(event.conference_url.clone());
                    *other = event;
                    other.conference_url = other.conference_url.clone().or(conference);
                }
            }
            None => kept.push(event),
        }
    }
    kept
}

fn is_same_meeting(a: &CalendarEvent, b: &CalendarEvent) -> bool {
    if a.id == b.id {
        return true;
    }
    a.title.eq_ignore_ascii_case(&b.title) && a.start == b.start
}

/// How much a response tells us. Higher wins a de-duplication tie.
fn response_rank(attendance: Attendance) -> u8 {
    match attendance {
        Attendance::Accepted => 4,
        Attendance::Tentative => 3,
        Attendance::Declined => 2,
        Attendance::NeedsAction => 1,
        Attendance::Unknown => 0,
    }
}

/// A recording, as the matcher sees one.
#[derive(Debug, Clone)]
pub struct RecordingWindow {
    pub meeting_id: String,
    pub start: DateTime<FixedOffset>,
    pub end: DateTime<FixedOffset>,
}

/// Seconds of slack allowed at either end when matching a recording to an
/// event.
///
/// Generous on purpose. Nobody presses record at exactly the top of the hour:
/// a call is joined a few minutes early, or the recording starts once the
/// small talk is over and the event has been running ten minutes. A window
/// this wide mismatches only where two meetings genuinely abut, and
/// [`match_recordings`] resolves that by overlap.
pub const MATCH_SLACK_SECONDS: i64 = 15 * 60;

/// Links each event to the recording that covers it.
///
/// Two things decide a match, in this order:
///
/// 1. **Real overlap beats near-miss.** A recording that runs *inside* an
///    event's own hour belongs to it, even if the event next door starts
///    closer to when recording began. Back-to-back meetings are the case this
///    gets wrong otherwise: with a fifteen-minute slack at both ends, a
///    recording of the 10:45 call overlaps the 10:30 call's padded window just
///    as much, and whichever was considered first would take it.
/// 2. **Then greatest overlap.** Among candidates of the same kind, the one
///    sharing the most seconds wins.
///
/// Assigned greedily over every pair rather than event by event, so the
/// strongest match in the whole day is made first and a weaker one can never
/// take a recording out from under it. One recording is claimed by at most one
/// event, because a recording is one conversation and attaching it to two
/// calendar rows makes both of them wrong.
pub fn match_recordings(events: &mut [CalendarEvent], recordings: &[RecordingWindow]) {
    let slack = chrono::Duration::seconds(MATCH_SLACK_SECONDS);

    // (tier, overlap, event index, recording index). Tier 1 overlaps the event
    // itself; tier 0 only reaches it through the slack.
    let mut candidates: Vec<(u8, i64, usize, usize)> = Vec::new();

    for (event_index, event) in events.iter().enumerate() {
        // An all-day event is not a conversation. Every recording made that
        // day overlaps it and none of them mean it.
        if event.all_day {
            continue;
        }
        let (Some(start), Some(end)) = (event.start_timestamp(), event.end_timestamp()) else {
            continue;
        };

        for (recording_index, recording) in recordings.iter().enumerate() {
            let exact = (recording.end.min(end) - recording.start.max(start)).num_seconds();
            if exact > 0 {
                candidates.push((1, exact, event_index, recording_index));
                continue;
            }
            let padded = (recording.end.min(end + slack) - recording.start.max(start - slack))
                .num_seconds();
            if padded > 0 {
                candidates.push((0, padded, event_index, recording_index));
            }
        }
    }

    // Strongest first: tier, then overlap. The index tie-breakers only make
    // the order deterministic when two candidates are otherwise identical.
    candidates.sort_by(|a, b| {
        b.0.cmp(&a.0)
            .then_with(|| b.1.cmp(&a.1))
            .then_with(|| a.2.cmp(&b.2))
            .then_with(|| a.3.cmp(&b.3))
    });

    let mut event_taken = vec![false; events.len()];
    let mut recording_taken = vec![false; recordings.len()];
    for (_, _, event_index, recording_index) in candidates {
        if event_taken[event_index] || recording_taken[recording_index] {
            continue;
        }
        event_taken[event_index] = true;
        recording_taken[recording_index] = true;
        events[event_index].meeting_id = Some(recordings[recording_index].meeting_id.clone());
    }
}

/// The RFC 3339 bounds of a window `days_back` before and `days_ahead` after
/// today, in the viewer's timezone.
///
/// What the sync asks Google for. Returned as a pair rather than computed at
/// the call site because "what counts as today" is a timezone question and
/// having one answer to it is the point.
pub fn sync_window(days_back: i64, days_ahead: i64) -> (String, String) {
    let now = Local::now();
    let start = (now - chrono::Duration::days(days_back.max(0)))
        .date_naive()
        .and_hms_opt(0, 0, 0)
        .unwrap_or_else(|| now.naive_local());
    let end = (now + chrono::Duration::days(days_ahead.max(0)))
        .date_naive()
        .and_hms_opt(23, 59, 59)
        .unwrap_or_else(|| now.naive_local());

    let to_rfc = |naive: chrono::NaiveDateTime| {
        Local
            .from_local_datetime(&naive)
            .single()
            .map(|dt| dt.to_rfc3339())
            .unwrap_or_else(|| naive.and_utc().to_rfc3339())
    };
    (to_rfc(start), to_rfc(end))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::calendar::model::EventAttendee;

    fn event(id: &str, account: &str, title: &str, start: &str, end: &str) -> CalendarEvent {
        CalendarEvent {
            id: id.to_string(),
            account_email: account.to_string(),
            calendar_id: Some("primary".into()),
            title: title.to_string(),
            start: start.to_string(),
            end: end.to_string(),
            all_day: false,
            location: None,
            conference_url: None,
            html_link: None,
            attendees: Vec::new(),
            attendance: Attendance::Unknown,
            meeting_id: None,
        }
    }

    fn at(rfc: &str) -> DateTime<FixedOffset> {
        DateTime::parse_from_rfc3339(rfc).expect("a timestamp")
    }

    fn recording(id: &str, start: &str, end: &str) -> RecordingWindow {
        RecordingWindow {
            meeting_id: id.to_string(),
            start: at(start),
            end: at(end),
        }
    }

    #[test]
    fn work_and_personal_calendars_become_one_day() {
        // The whole reason this module exists: five from work plus one from
        // life is six meetings today, in one list.
        let mut events: Vec<CalendarEvent> = (0..5)
            .map(|i| {
                event(
                    &format!("work-{i}"),
                    "me@work.com",
                    &format!("Work {i}"),
                    &format!("2026-09-14T{:02}:00:00+05:30", 10 + i),
                    &format!("2026-09-14T{:02}:30:00+05:30", 10 + i),
                )
            })
            .collect();
        events.push(event(
            "life-1",
            "me@gmail.com",
            "Dentist",
            "2026-09-14T18:00:00+05:30",
            "2026-09-14T18:45:00+05:30",
        ));

        let agenda = build_agenda(events);
        assert_eq!(agenda.len(), 1, "one day");
        assert_eq!(agenda[0].events.len(), 6);
        assert_eq!(agenda[0].events[5].title, "Dentist", "sorted by time");
    }

    #[test]
    fn one_invitation_in_two_inboxes_is_one_meeting() {
        let a = event(
            "shared-id",
            "me@work.com",
            "All hands",
            "2026-09-14T10:00:00Z",
            "2026-09-14T11:00:00Z",
        );
        let b = event(
            "shared-id",
            "me@gmail.com",
            "All hands",
            "2026-09-14T10:00:00Z",
            "2026-09-14T11:00:00Z",
        );
        assert_eq!(deduplicate(vec![a, b]).len(), 1);
    }

    #[test]
    fn the_copy_that_knows_whether_you_are_going_is_the_one_kept() {
        let mut invited = event(
            "shared",
            "me@gmail.com",
            "All hands",
            "2026-09-14T10:00:00Z",
            "2026-09-14T11:00:00Z",
        );
        invited.attendance = Attendance::NeedsAction;

        let mut accepted = event(
            "shared",
            "me@work.com",
            "All hands",
            "2026-09-14T10:00:00Z",
            "2026-09-14T11:00:00Z",
        );
        accepted.attendance = Attendance::Accepted;
        accepted.attendees = vec![EventAttendee {
            email: "me@work.com".into(),
            display_name: None,
            response: Attendance::Accepted,
            is_self: true,
            organizer: false,
        }];

        let kept = deduplicate(vec![invited, accepted]);
        assert_eq!(kept.len(), 1);
        assert_eq!(kept[0].attendance, Attendance::Accepted);
        assert_eq!(kept[0].account_email, "me@work.com");
    }

    #[test]
    fn de_duplication_never_loses_the_only_link_to_join_with() {
        let mut with_link = event(
            "shared",
            "me@gmail.com",
            "All hands",
            "2026-09-14T10:00:00Z",
            "2026-09-14T11:00:00Z",
        );
        with_link.conference_url = Some("https://meet.google.com/abc".into());

        let mut accepted = event(
            "shared",
            "me@work.com",
            "All hands",
            "2026-09-14T10:00:00Z",
            "2026-09-14T11:00:00Z",
        );
        accepted.attendance = Attendance::Accepted;

        let kept = deduplicate(vec![with_link, accepted]);
        assert_eq!(kept.len(), 1);
        assert_eq!(
            kept[0].conference_url.as_deref(),
            Some("https://meet.google.com/abc"),
            "the surviving copy must still be joinable"
        );
    }

    #[test]
    fn a_forwarded_invitation_with_a_different_id_is_still_one_meeting() {
        let a = event(
            "id-a",
            "me@work.com",
            "Board review",
            "2026-09-14T10:00:00Z",
            "2026-09-14T11:00:00Z",
        );
        let b = event(
            "id-b",
            "me@gmail.com",
            "board review",
            "2026-09-14T10:00:00Z",
            "2026-09-14T11:00:00Z",
        );
        assert_eq!(deduplicate(vec![a, b]).len(), 1);
    }

    #[test]
    fn two_genuinely_different_meetings_are_not_collapsed() {
        let a = event(
            "id-a",
            "me@work.com",
            "Standup",
            "2026-09-14T10:00:00Z",
            "2026-09-14T10:15:00Z",
        );
        let b = event(
            "id-b",
            "me@work.com",
            "Standup",
            "2026-09-15T10:00:00Z",
            "2026-09-15T10:15:00Z",
        );
        assert_eq!(deduplicate(vec![a, b]).len(), 2, "same name, different day");
    }

    #[test]
    fn all_day_events_lead_the_day_rather_than_sitting_in_the_middle_of_it() {
        let mut holiday = event(
            "holiday",
            "me@gmail.com",
            "Diwali",
            "2026-09-14T00:00:00+05:30",
            "2026-09-15T00:00:00+05:30",
        );
        holiday.all_day = true;
        let call = event(
            "call",
            "me@work.com",
            "Morning call",
            "2026-09-14T09:00:00+05:30",
            "2026-09-14T09:30:00+05:30",
        );

        let agenda = build_agenda(vec![call, holiday]);
        assert_eq!(agenda[0].events[0].title, "Diwali");
    }

    #[test]
    fn a_recording_is_matched_to_the_event_it_overlaps_most() {
        let mut events = vec![
            event(
                "standup",
                "me@work.com",
                "Standup",
                "2026-09-14T10:30:00+05:30",
                "2026-09-14T10:45:00+05:30",
            ),
            event(
                "sync",
                "me@work.com",
                "Daily sync",
                "2026-09-14T10:45:00+05:30",
                "2026-09-14T11:00:00+05:30",
            ),
        ];
        // Recorded across the sync, not the standup.
        let recordings = vec![recording(
            "meeting-1",
            "2026-09-14T10:46:00+05:30",
            "2026-09-14T10:59:00+05:30",
        )];
        match_recordings(&mut events, &recordings);

        assert_eq!(events[1].meeting_id.as_deref(), Some("meeting-1"));
        assert_eq!(events[0].meeting_id, None, "abutting is not overlapping");
    }

    #[test]
    fn a_recording_started_late_still_finds_its_meeting() {
        // Nobody presses record at the top of the hour.
        let mut events = vec![event(
            "sync",
            "me@work.com",
            "Weekly sync",
            "2026-09-14T15:00:00+05:30",
            "2026-09-14T16:00:00+05:30",
        )];
        let recordings = vec![recording(
            "meeting-1",
            "2026-09-14T15:08:00+05:30",
            "2026-09-14T15:55:00+05:30",
        )];
        match_recordings(&mut events, &recordings);
        assert_eq!(events[0].meeting_id.as_deref(), Some("meeting-1"));
    }

    #[test]
    fn one_recording_is_never_claimed_by_two_events() {
        let mut events = vec![
            event(
                "a",
                "me@work.com",
                "First",
                "2026-09-14T10:00:00Z",
                "2026-09-14T10:30:00Z",
            ),
            event(
                "b",
                "me@work.com",
                "Second",
                "2026-09-14T10:30:00Z",
                "2026-09-14T11:00:00Z",
            ),
        ];
        let recordings = vec![recording(
            "meeting-1",
            "2026-09-14T10:05:00Z",
            "2026-09-14T10:25:00Z",
        )];
        match_recordings(&mut events, &recordings);

        let claimed = events.iter().filter(|e| e.meeting_id.is_some()).count();
        assert_eq!(claimed, 1, "a recording is one conversation");
        assert_eq!(events[0].meeting_id.as_deref(), Some("meeting-1"));
    }

    #[test]
    fn an_all_day_event_never_claims_a_recording() {
        // Every recording made that day overlaps it, and none of them mean it.
        let mut holiday = event(
            "holiday",
            "me@gmail.com",
            "Diwali",
            "2026-09-14T00:00:00Z",
            "2026-09-15T00:00:00Z",
        );
        holiday.all_day = true;
        let mut events = vec![holiday];
        match_recordings(
            &mut events,
            &[recording("meeting-1", "2026-09-14T10:00:00Z", "2026-09-14T10:30:00Z")],
        );
        assert_eq!(events[0].meeting_id, None);
    }

    #[test]
    fn a_recording_inside_one_event_is_not_taken_by_the_event_next_door() {
        // Both padded windows reach the recording equally; only the real hour
        // tells them apart. Ordered so the wrong answer would win on a tie.
        let mut events = vec![
            event(
                "earlier",
                "me@work.com",
                "Earlier call",
                "2026-09-14T10:00:00Z",
                "2026-09-14T11:00:00Z",
            ),
            event(
                "later",
                "me@work.com",
                "Later call",
                "2026-09-14T11:00:00Z",
                "2026-09-14T12:00:00Z",
            ),
        ];
        let recordings = vec![recording(
            "meeting-1",
            "2026-09-14T11:05:00Z",
            "2026-09-14T11:50:00Z",
        )];
        match_recordings(&mut events, &recordings);

        assert_eq!(events[1].meeting_id.as_deref(), Some("meeting-1"));
        assert_eq!(events[0].meeting_id, None);
    }

    #[test]
    fn the_strongest_match_in_the_day_is_made_first() {
        // Assigned greedily over every pair: a weak candidate considered
        // earlier must not take a recording that belongs squarely to a later
        // one.
        let mut events = vec![
            event(
                "brief",
                "me@work.com",
                "Brief touch",
                "2026-09-14T09:00:00Z",
                "2026-09-14T09:10:00Z",
            ),
            event(
                "long",
                "me@work.com",
                "Long review",
                "2026-09-14T09:05:00Z",
                "2026-09-14T10:05:00Z",
            ),
        ];
        let recordings = vec![recording(
            "meeting-1",
            "2026-09-14T09:06:00Z",
            "2026-09-14T10:00:00Z",
        )];
        match_recordings(&mut events, &recordings);

        assert_eq!(events[1].meeting_id.as_deref(), Some("meeting-1"));
        assert_eq!(events[0].meeting_id, None);
    }

    #[test]
    fn a_recording_with_no_matching_event_leaves_every_event_unlinked() {
        let mut events = vec![event(
            "a",
            "me@work.com",
            "Morning",
            "2026-09-14T10:00:00Z",
            "2026-09-14T11:00:00Z",
        )];
        match_recordings(
            &mut events,
            &[recording("meeting-1", "2026-09-14T18:00:00Z", "2026-09-14T19:00:00Z")],
        );
        assert_eq!(events[0].meeting_id, None);
    }

    #[test]
    fn matching_nothing_against_nothing_is_not_an_error() {
        let mut events: Vec<CalendarEvent> = Vec::new();
        match_recordings(&mut events, &[]);
        assert!(events.is_empty());
        assert!(build_agenda(Vec::new()).is_empty());
    }

    #[test]
    fn the_sync_window_spans_the_days_it_was_asked_for_and_starts_before_it_ends() {
        let (start, end) = sync_window(7, 14);
        let start = DateTime::parse_from_rfc3339(&start).expect("a start");
        let end = DateTime::parse_from_rfc3339(&end).expect("an end");
        assert!(start < end);
        assert!((end - start).num_days() >= 20);
    }
}
