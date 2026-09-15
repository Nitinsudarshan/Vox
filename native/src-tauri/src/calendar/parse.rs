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
        description: item
            .get("description")
            .and_then(|d| d.as_str())
            .map(flatten_html)
            .filter(|d| !d.is_empty()),
        recurring_event_id: item
            .get("recurringEventId")
            .and_then(|id| id.as_str())
            .map(str::trim)
            .filter(|id| !id.is_empty())
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

/// Longest description Vox keeps.
///
/// An invitation's notes are occasionally an entire wiki page pasted into the
/// field. The cache is re-fetched on every sync and read on every agenda
/// render, so a bound belongs here rather than on the screen that shows it.
const MAX_DESCRIPTION_CHARS: usize = 4000;

/// Google's description HTML, as the text a person would have read.
///
/// Google stores the field as HTML and there is no plain-text alternative in
/// the response, so the choice is flattening it here or rendering foreign
/// markup in the app. It is flattened: `rules/untrusted-input.md` makes a
/// calendar invitation somebody else wrote exactly the kind of content that
/// gets shown as text and never interpreted, and an agenda is not a browser.
///
/// Block-level tags become line breaks rather than disappearing, because an
/// agenda pasted as a list reads as one run-on sentence otherwise.
pub fn flatten_html(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    let mut chars = raw.chars().peekable();

    while let Some(ch) = chars.next() {
        match ch {
            '<' => {
                let mut tag = String::new();
                for next in chars.by_ref() {
                    if next == '>' {
                        break;
                    }
                    tag.push(next);
                }
                out.push_str(tag_replacement(&tag));
            }
            '&' => {
                let mut entity = String::new();
                // Stops at the first character that cannot be part of an
                // entity name, and after ten of them. Scanning blindly for a
                // `;` was a hole: an entity name can only be letters, digits
                // and `#`, but the old loop happily ate a `<` on its way, so
                // `&<img src=x onerror=…>` reached the output with its tag
                // intact — the one input that made this flattener not flatten.
                // An unterminated `&` is just an ampersand somebody typed.
                while let Some(&next) = chars.peek() {
                    if !(next.is_ascii_alphanumeric() || next == '#') || entity.len() >= 10 {
                        break;
                    }
                    entity.push(next);
                    chars.next();
                }
                match chars.peek() {
                    // `&;` is not an entity, so it is not decoded as one.
                    Some(';') if !entity.is_empty() => {
                        chars.next();
                        out.push_str(&decode_entity(&entity));
                    }
                    _ => {
                        out.push('&');
                        out.push_str(&entity);
                    }
                }
            }
            other => out.push(other),
        }
    }

    tidy(&out)
}

/// What one tag leaves behind: a break, a bullet, or nothing.
///
/// A block break is emitted by the *closing* tag, not by both halves. Emitting
/// one for each is what turns a three-item list into three items separated by
/// blank lines, which is markup showing through rather than formatting.
fn tag_replacement(tag: &str) -> &'static str {
    let closing = tag.starts_with('/');
    let name = tag
        .trim_start_matches('/')
        .split(|c: char| c.is_whitespace() || c == '/')
        .next()
        .unwrap_or("")
        .to_ascii_lowercase();

    match (name.as_str(), closing) {
        // `<br>` has no closing half to wait for.
        ("br", _) => "\n",
        // The bullet has to precede the text, so this one is the open tag.
        ("li", false) => "\n• ",
        (
            "p" | "div" | "tr" | "h1" | "h2" | "h3" | "h4" | "h5" | "h6" | "ul" | "ol"
            | "blockquote" | "table",
            true,
        ) => "\n",
        ("td" | "th", true) => " ",
        _ => "",
    }
}

fn decode_entity(entity: &str) -> String {
    match entity.to_ascii_lowercase().as_str() {
        "amp" => "&".to_string(),
        "lt" => "<".to_string(),
        "gt" => ">".to_string(),
        "quot" => "\"".to_string(),
        "apos" | "#39" => "'".to_string(),
        "nbsp" => " ".to_string(),
        "hellip" => "…".to_string(),
        "mdash" => "—".to_string(),
        "ndash" => "–".to_string(),
        other => other
            .strip_prefix('#')
            .and_then(|code| {
                let value = match code.strip_prefix(['x', 'X']) {
                    Some(hex) => u32::from_str_radix(hex, 16).ok()?,
                    None => code.parse().ok()?,
                };
                char::from_u32(value).map(String::from)
            })
            // An entity Vox does not know is shown as it was written. Dropping
            // it silently turns "&pound;50" into "50".
            .unwrap_or_else(|| format!("&{other};")),
    }
}

/// Collapses the whitespace HTML flattening leaves behind, and caps the length.
fn tidy(raw: &str) -> String {
    let mut lines: Vec<String> = Vec::new();
    let mut blanks = 0usize;

    for line in raw.lines() {
        let trimmed = line.split_whitespace().collect::<Vec<_>>().join(" ");
        if trimmed.is_empty() {
            blanks += 1;
            // One blank line separates paragraphs; six is the markup showing
            // through.
            if blanks > 1 || lines.is_empty() {
                continue;
            }
            lines.push(String::new());
        } else {
            blanks = 0;
            lines.push(trimmed);
        }
    }

    while lines.last().is_some_and(|line| line.is_empty()) {
        lines.pop();
    }

    let text = lines.join("\n");
    if text.chars().count() <= MAX_DESCRIPTION_CHARS {
        return text;
    }
    let truncated: String = text.chars().take(MAX_DESCRIPTION_CHARS).collect();
    format!("{truncated}…")
}

/// Whether a link is one Vox is willing to hand to the browser.
///
/// The join link on an invitation is written by whoever sent it, and
/// `tauri-plugin-opener` hands what it is given to the shell. `http`/`https`
/// only is the whole check: it is what stops a `file:` path from opening a
/// local document, and anything more elaborate would be guessing at a URL
/// Google already validated.
pub fn is_web_url(url: &str) -> bool {
    let trimmed = url.trim();
    if trimmed.chars().any(char::is_whitespace) {
        return false;
    }
    let lowered = trimmed.to_ascii_lowercase();
    lowered
        .strip_prefix("https://")
        .or_else(|| lowered.strip_prefix("http://"))
        .is_some_and(|host| !host.is_empty())
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
    fn an_invitation_carries_the_notes_somebody_wrote_on_it() {
        // The agenda, the dial-in and the "read this first" link live here,
        // and Google only ever sends them as HTML.
        let mut item = timed_event();
        item["description"] = serde_json::json!(
            "<p>Agenda:</p><ul><li>Budget</li><li>Hiring</li></ul><p>Doc: \
             <a href=\"https://docs.example.com/x\">https://docs.example.com/x</a></p>"
        );
        let event = parse_event(&item, "nitin@navgurukul.org", "primary").expect("an event");
        assert_eq!(
            event.description.as_deref(),
            Some("Agenda:\n\n• Budget\n• Hiring\nDoc: https://docs.example.com/x")
        );
    }

    #[test]
    fn an_invitation_with_no_notes_says_nothing_rather_than_saying_nothing_loudly() {
        let mut item = timed_event();
        item["description"] = serde_json::json!("<div><br></div>");
        let event = parse_event(&item, "nitin@navgurukul.org", "primary").expect("an event");
        assert!(event.description.is_none(), "empty markup is not a description");
    }

    #[test]
    fn markup_is_flattened_rather_than_carried_into_the_app() {
        // `rules/untrusted-input.md`: an invitation somebody else wrote is
        // shown as text. Nothing downstream should receive a tag at all.
        let flattened = flatten_html("<script>alert(1)</script>Hello <b>there</b>");
        assert_eq!(flattened, "alert(1)Hello there");
        assert!(!flattened.contains('<'));
    }

    #[test]
    fn an_ampersand_cannot_smuggle_the_tag_that_follows_it() {
        // The entity scan used to run to the first `;` whatever it passed, so
        // a `<` inside that window was flushed verbatim along with everything
        // after it. Both of these came back with their markup intact.
        for smuggled in [
            "&<img src=x onerror=alert(1)>",
            "&x<script>alert(1)</script>;",
            "&#<iframe src=javascript:alert(1)></iframe>",
            "&0123456789<b>bold</b>",
        ] {
            let flattened = flatten_html(smuggled);
            assert!(
                !flattened.contains('<') && !flattened.contains('>'),
                "{smuggled:?} flattened to {flattened:?}"
            );
        }
    }

    #[test]
    fn an_ampersand_that_is_not_an_entity_survives_as_itself() {
        // The fix must not eat the ordinary ones.
        assert_eq!(flatten_html("Tom & Jerry"), "Tom & Jerry");
        assert_eq!(flatten_html("R&D"), "R&D");
        assert_eq!(flatten_html("&;"), "&;");
        assert_eq!(flatten_html("Q&A: &amp; and &lt;"), "Q&A: & and <");
    }

    #[test]
    fn the_entities_a_pasted_agenda_actually_contains_are_decoded() {
        assert_eq!(
            flatten_html("R&amp;D &lt;sync&gt; &quot;weekly&quot; &#39;26 &nbsp;&#x2014;"),
            "R&D <sync> \"weekly\" '26 —"
        );
        // An entity Vox does not know is left as written rather than dropped.
        assert_eq!(flatten_html("&pound;50"), "&pound;50");
        // A bare ampersand is an ampersand.
        assert_eq!(flatten_html("Tom & Jerry"), "Tom & Jerry");
    }

    #[test]
    fn a_wall_of_blank_markup_collapses_to_readable_paragraphs() {
        assert_eq!(
            flatten_html("One<br><br><br><br>Two<br>Three"),
            "One\n\nTwo\nThree"
        );
    }

    #[test]
    fn a_description_pasted_from_a_wiki_is_capped_rather_than_cached_whole() {
        let long = "x".repeat(MAX_DESCRIPTION_CHARS + 500);
        let flattened = flatten_html(&long);
        assert_eq!(flattened.chars().count(), MAX_DESCRIPTION_CHARS + 1);
        assert!(flattened.ends_with('…'));
    }

    #[test]
    fn only_a_web_link_is_worth_handing_to_the_browser() {
        assert!(is_web_url("https://meet.google.com/abc-defg-hij"));
        assert!(is_web_url("http://10.0.0.5:8080/join"));
        // The shell would happily open every one of these.
        assert!(!is_web_url("file:///C:/Windows/System32/calc.exe"));
        assert!(!is_web_url("javascript:alert(1)"));
        assert!(!is_web_url("C:\\Windows\\System32\\calc.exe"));
        assert!(!is_web_url("https://"));
        assert!(!is_web_url(""));
        assert!(!is_web_url("https://example.com /extra"));
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
