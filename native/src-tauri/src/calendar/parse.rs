//! Google's event JSON, turned into [`CalendarEvent`].
//!
//! Kept apart from the HTTP in [`super::google`] so the interesting half can
//! be tested against real response shapes without a network, a token, or a
//! Google account. Every case below — an all-day event, a cancelled one, a
//! conference link in either of the two places Google puts it, an event with
//! no attendee list — came from the API's own documented output and is
//! exercised by a test in this file.

use super::model::{Attendance, CalendarEvent, EventAttendee};

/// Turns one page of `events.list` into events, skipping what cannot be shown.
///
/// Skipping rather than failing, per item: one malformed event in a page of
/// forty must not cost the other thirty-nine. A calendar Vox cannot fully
/// understand is still a calendar worth showing.
pub fn parse_events(body: &serde_json::Value, account_email: &str, calendar_id: &str) -> Vec<CalendarEvent> {
    body.get("items")
        .and_then(|items| items.as_array())
        .map(|items| {
            items
                .iter()
                .filter_map(|item| parse_event(item, account_email, calendar_id))
                .collect()
        })
        .unwrap_or_default()
}

/// The token for the next page, where the response says there is one.
pub fn next_page_token(body: &serde_json::Value) -> Option<String> {
    body.get("nextPageToken")
        .and_then(|token| token.as_str())
        .map(str::to_string)
        .filter(|token| !token.is_empty())
}

/// One event, or `None` when there is nothing worth showing.
pub fn parse_event(
    item: &serde_json::Value,
    account_email: &str,
    calendar_id: &str,
) -> Option<CalendarEvent> {
    // A cancelled event still comes back on an incremental sync so clients can
    // remove it. Vox re-reads a window rather than syncing incrementally, so
    // here it is simply something that is not happening.
    if item.get("status").and_then(|s| s.as_str()) == Some("cancelled") {
        return None;
    }

    let id = item.get("id").and_then(|id| id.as_str())?.to_string();
    let (start, all_day) = parse_time(item.get("start")?)?;
    let (end, _) = parse_time(item.get("end")?)?;

    let title = item
        .get("summary")
        .and_then(|s| s.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        // Google omits `summary` entirely for an event saved with no title,
        // and "(no title)" is what its own UI shows for one.
        .unwrap_or("(no title)")
        .to_string();

    let attendees = parse_attendees(item, account_email);
    let attendance = attendees
        .iter()
        .find(|a| a.is_self)
        .map(|a| a.response)
        // No attendee list means an event the user made for themselves: they
        // are going, and showing it as "not answered" would be wrong.
        .unwrap_or(Attendance::Unknown);

    Some(CalendarEvent {
        id,
        account_email: account_email.to_string(),
        calendar_id: Some(calendar_id.to_string()),
        title,
        start,
        end,
        all_day,
        location: item
            .get("location")
            .and_then(|l| l.as_str())
            .map(str::trim)
            .filter(|l| !l.is_empty())
            .map(str::to_string),
        conference_url: conference_url(item),
        html_link: item
            .get("htmlLink")
            .and_then(|l| l.as_str())
            .map(str::to_string),
        attendees,
        attendance,
        meeting_id: None,
    })
}

/// `(rfc3339, all_day)` from a Google start/end object.
///
/// Google sends `dateTime` for a timed event and `date` for an all-day one.
/// An all-day event is widened to midnight-to-midnight so that one comparison
/// orders both kinds, rather than every consumer branching on `all_day`.
fn parse_time(node: &serde_json::Value) -> Option<(String, bool)> {
    if let Some(date_time) = node.get("dateTime").and_then(|v| v.as_str()) {
        // Normalised through chrono so everything downstream compares the same
        // shape, whatever offset spelling the calendar used.
        let parsed = chrono::DateTime::parse_from_rfc3339(date_time).ok()?;
        return Some((parsed.to_rfc3339(), false));
    }
    let date = node.get("date").and_then(|v| v.as_str())?;
    let naive = chrono::NaiveDate::parse_from_str(date, "%Y-%m-%d").ok()?;
    let midnight = naive.and_hms_opt(0, 0, 0)?;
    // Local midnight: an all-day event is a statement about the viewer's day,
    // not about UTC.
    let local = chrono::Local
        .from_local_datetime(&midnight)
        .single()
        .or_else(|| chrono::Local.from_local_datetime(&midnight).earliest())?;
    Some((local.to_rfc3339(), true))
}

use chrono::TimeZone;

fn parse_attendees(item: &serde_json::Value, account_email: &str) -> Vec<EventAttendee> {
    let organizer_email = item
        .get("organizer")
        .and_then(|o| o.get("email"))
        .and_then(|e| e.as_str())
        .unwrap_or("")
        .to_lowercase();

    item.get("attendees")
        .and_then(|a| a.as_array())
        .map(|attendees| {
            attendees
                .iter()
                .filter(|attendee| {
                    // Meeting rooms and equipment are on the attendee list and
                    // are not people.
                    attendee.get("resource").and_then(|r| r.as_bool()) != Some(true)
                })
                .filter_map(|attendee| {
                    let email = attendee.get("email").and_then(|e| e.as_str())?.to_string();
                    let lowered = email.to_lowercase();
                    Some(EventAttendee {
                        // Google marks the account holder with `self`, but only
                        // on the copy in their own calendar; matching the
                        // address too covers the copy read from another.
                        is_self: attendee.get("self").and_then(|s| s.as_bool()) == Some(true)
                            || lowered == account_email.to_lowercase(),
                        organizer: attendee.get("organizer").and_then(|o| o.as_bool())
                            == Some(true)
                            || lowered == organizer_email,
                        response: attendee
                            .get("responseStatus")
                            .and_then(|s| s.as_str())
                            .map(Attendance::from_google)
                            .unwrap_or(Attendance::Unknown),
                        display_name: attendee
                            .get("displayName")
                            .and_then(|n| n.as_str())
                            .map(str::trim)
                            .filter(|n| !n.is_empty())
                            .map(str::to_string),
                        email,
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

/// The link to press to join.
///
/// `hangoutLink` is the old field and is still what a Meet invitation
/// populates; `conferenceData.entryPoints` is the current one and is where a
/// Zoom or Teams add-on writes. Both are checked because a calendar in
/// practice contains both.
fn conference_url(item: &serde_json::Value) -> Option<String> {
    if let Some(link) = item
        .get("hangoutLink")
        .and_then(|l| l.as_str())
        .filter(|l| !l.is_empty())
    {
        return Some(link.to_string());
    }
    item.get("conferenceData")
        .and_then(|data| data.get("entryPoints"))
        .and_then(|points| points.as_array())
        .and_then(|points| {
            points
                .iter()
                .find(|point| {
                    point.get("entryPointType").and_then(|t| t.as_str()) == Some("video")
                })
                .or_else(|| points.first())
        })
        .and_then(|point| point.get("uri"))
        .and_then(|uri| uri.as_str())
        .map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn timed_event() -> serde_json::Value {
        serde_json::json!({
            "id": "evt-1",
            "status": "confirmed",
            "summary": "CEO's office <> Weekly sync call",
            "start": { "dateTime": "2026-09-11T15:00:00+05:30" },
            "end": { "dateTime": "2026-09-11T16:00:00+05:30" },
            "htmlLink": "https://calendar.google.com/event?eid=abc",
            "hangoutLink": "https://meet.google.com/abc-defg-hij",
            "organizer": { "email": "saraswati@navgurukul.org" },
            "attendees": [
                { "email": "nitin@navgurukul.org", "self": true, "responseStatus": "accepted" },
                { "email": "saraswati@navgurukul.org", "displayName": "Saraswati", "responseStatus": "accepted" },
                { "email": "room-a@navgurukul.org", "resource": true, "responseStatus": "accepted" }
            ]
        })
    }

    #[test]
    fn a_timed_event_parses_with_everything_the_ui_shows() {
        let event = parse_event(&timed_event(), "nitin@navgurukul.org", "primary").expect("an event");

        assert_eq!(event.id, "evt-1");
        assert_eq!(event.title, "CEO's office <> Weekly sync call");
        assert!(!event.all_day);
        assert_eq!(event.attendance, Attendance::Accepted);
        assert_eq!(
            event.conference_url.as_deref(),
            Some("https://meet.google.com/abc-defg-hij")
        );
        assert_eq!(event.account_email, "nitin@navgurukul.org");
    }

    #[test]
    fn a_meeting_room_is_not_a_person() {
        let event = parse_event(&timed_event(), "nitin@navgurukul.org", "primary").expect("an event");
        assert_eq!(event.attendees.len(), 2, "the resource should be gone");
        assert!(event.attendees.iter().all(|a| a.email != "room-a@navgurukul.org"));
    }

    #[test]
    fn the_account_holder_is_marked_and_is_not_one_of_the_others() {
        let event = parse_event(&timed_event(), "nitin@navgurukul.org", "primary").expect("an event");
        let others = event.others();
        assert_eq!(others.len(), 1);
        assert_eq!(others[0].email, "saraswati@navgurukul.org");
        assert!(others[0].organizer, "the organizer is recognised by address");
    }

    #[test]
    fn the_account_holder_is_recognised_by_address_when_google_does_not_mark_them() {
        // The copy of an invitation read from another account carries no
        // `self` flag, so matching the address is what keeps "am I going?"
        // answerable.
        let mut item = timed_event();
        item["attendees"][0]["self"] = serde_json::json!(false);
        let event = parse_event(&item, "NITIN@navgurukul.org", "primary").expect("an event");
        assert_eq!(event.attendance, Attendance::Accepted);
    }

    #[test]
    fn a_cancelled_event_is_not_something_that_is_happening() {
        let mut item = timed_event();
        item["status"] = serde_json::json!("cancelled");
        assert!(parse_event(&item, "nitin@navgurukul.org", "primary").is_none());
    }

    #[test]
    fn an_all_day_event_becomes_midnight_to_midnight_so_one_comparison_orders_both_kinds() {
        let item = serde_json::json!({
            "id": "evt-holiday",
            "summary": "Diwali",
            "start": { "date": "2026-11-08" },
            "end": { "date": "2026-11-09" }
        });
        let event = parse_event(&item, "me@example.com", "primary").expect("an event");
        assert!(event.all_day);
        let start = event.start_timestamp().expect("a start");
        assert_eq!(start.format("%Y-%m-%d %H:%M").to_string(), "2026-11-08 00:00");
    }

    #[test]
    fn an_event_saved_with_no_title_says_so_rather_than_showing_as_blank() {
        let item = serde_json::json!({
            "id": "evt-blank",
            "start": { "dateTime": "2026-09-11T15:00:00+05:30" },
            "end": { "dateTime": "2026-09-11T15:30:00+05:30" }
        });
        let event = parse_event(&item, "me@example.com", "primary").expect("an event");
        assert_eq!(event.title, "(no title)");
    }

    #[test]
    fn an_event_with_no_attendees_is_one_the_user_made_for_themselves() {
        let item = serde_json::json!({
            "id": "evt-solo",
            "summary": "Focus block",
            "start": { "dateTime": "2026-09-11T09:00:00Z" },
            "end": { "dateTime": "2026-09-11T10:00:00Z" }
        });
        let event = parse_event(&item, "me@example.com", "primary").expect("an event");
        assert_eq!(event.attendance, Attendance::Unknown);
        assert!(event.attendees.is_empty());
    }

    #[test]
    fn a_zoom_link_is_found_where_an_add_on_puts_it() {
        let item = serde_json::json!({
            "id": "evt-zoom",
            "summary": "Client call",
            "start": { "dateTime": "2026-09-11T15:00:00Z" },
            "end": { "dateTime": "2026-09-11T16:00:00Z" },
            "conferenceData": {
                "entryPoints": [
                    { "entryPointType": "phone", "uri": "tel:+1-555-0100" },
                    { "entryPointType": "video", "uri": "https://zoom.us/j/123456" }
                ]
            }
        });
        let event = parse_event(&item, "me@example.com", "primary").expect("an event");
        assert_eq!(event.conference_url.as_deref(), Some("https://zoom.us/j/123456"));
    }

    #[test]
    fn an_event_with_no_usable_time_is_skipped_rather_than_shown_at_the_epoch() {
        let item = serde_json::json!({
            "id": "evt-broken",
            "summary": "Broken",
            "start": { "dateTime": "not a timestamp" },
            "end": { "dateTime": "also not" }
        });
        assert!(parse_event(&item, "me@example.com", "primary").is_none());
    }

    #[test]
    fn one_bad_event_does_not_cost_the_rest_of_the_page() {
        let body = serde_json::json!({
            "items": [
                timed_event(),
                { "id": "evt-broken", "start": {}, "end": {} },
                { "id": "evt-2", "summary": "Standup",
                  "start": { "dateTime": "2026-09-11T04:00:00Z" },
                  "end": { "dateTime": "2026-09-11T04:15:00Z" } }
            ]
        });
        let events = parse_events(&body, "nitin@navgurukul.org", "primary");
        assert_eq!(events.len(), 2);
        assert_eq!(events[1].title, "Standup");
    }

    #[test]
    fn a_response_with_no_items_is_an_empty_calendar_not_a_failure() {
        assert!(parse_events(&serde_json::json!({}), "me@example.com", "primary").is_empty());
        assert!(parse_events(&serde_json::json!({"items": []}), "me@example.com", "primary").is_empty());
    }

    #[test]
    fn paging_continues_only_while_google_says_there_is_more() {
        assert_eq!(
            next_page_token(&serde_json::json!({ "nextPageToken": "abc" })).as_deref(),
            Some("abc")
        );
        assert!(next_page_token(&serde_json::json!({})).is_none());
        assert!(next_page_token(&serde_json::json!({ "nextPageToken": "" })).is_none());
    }

    #[test]
    fn every_response_status_google_documents_is_understood() {
        assert_eq!(Attendance::from_google("accepted"), Attendance::Accepted);
        assert_eq!(Attendance::from_google("declined"), Attendance::Declined);
        assert_eq!(Attendance::from_google("tentative"), Attendance::Tentative);
        assert_eq!(Attendance::from_google("needsAction"), Attendance::NeedsAction);
        // An unrecognised status must hide an event from nobody.
        assert_eq!(Attendance::from_google("something new"), Attendance::Unknown);
    }
}
