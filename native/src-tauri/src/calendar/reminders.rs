//! When to tell somebody a meeting is about to start, and what to tell them.
//!
//! Vox knows the schedule and knew it before this existed; what it did not do
//! was say anything. A calendar you have to remember to look at is a calendar
//! that does not remind you, and the meeting you miss is the one you were
//! heads-down through.
//!
//! ## The timing rule
//!
//! The user configures lead times — "ten minutes before", "one minute
//! before", "when it starts". Those are **buckets**, not alarms: at any
//! moment an event is in exactly one of them, the smallest configured lead it
//! has already reached. That is what makes the case below behave.
//!
//! > Vox is closed at 09:45 and opened at 09:57, three minutes before a 10:00
//! > call, with leads of 15, 5 and 0 configured.
//!
//! With one alarm per lead this is three notifications in a row, all about the
//! same meeting, two of them lying about how long is left. With buckets it is
//! one, and it says *three minutes*, because the text is written from the
//! clock rather than from the bucket that triggered it.
//!
//! A bucket fires once per event. Nothing here re-notifies, snoozes, or
//! escalates: a reminder that arrives twice is worse than one that arrives
//! once, and a person who dismissed it meant it.
//!
//! ## What is never announced
//!
//! - **All-day events.** "Diwali" is not something to join, and reminding
//!   somebody about it five minutes before midnight is noise.
//! - **Meetings that are already well under way.** Opening Vox at 11:40 must
//!   not announce the 10:00 call. [`LATE_GRACE_MINUTES`] is how late is still
//!   worth saying — long enough to catch a laptop opened a moment after a
//!   call began, short enough that it never announces history.
//! - **Anything the user has declined**, unless they ask for it. Declining is
//!   the clearest possible statement that they are not going.

use std::collections::HashSet;

use serde::{Deserialize, Serialize};

use super::model::{Attendance, CalendarEvent};

/// How long after a meeting has started a reminder is still worth sending.
///
/// Covers the laptop opened a minute into a call. Anything older is history,
/// and announcing history trains people to ignore the notification.
pub const LATE_GRACE_MINUTES: i64 = 5;

/// The lead times offered, in minutes. `0` means "when it starts".
///
/// A fixed set rather than free text: these are the intervals people actually
/// mean, and a free-form minutes box invites "0.5" and "90" — one of which is
/// not a number of minutes and the other of which is a different feature.
pub const LEAD_CHOICES: [i64; 5] = [15, 10, 5, 1, 0];

/// When and whether Vox announces a meeting.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ReminderSettings {
    /// Off means Vox says nothing at all. On by default: a calendar that never
    /// speaks up is the state this feature exists to fix.
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// Minutes before the start at which to announce. `0` is "when it starts".
    #[serde(default = "default_leads")]
    pub lead_minutes: Vec<i64>,
    /// Whether the OS also gets a toast, so a reminder reaches a user who is
    /// in another window.
    #[serde(default = "default_true")]
    pub system_notification: bool,
    /// Only announce meetings that carry a video link.
    ///
    /// Off by default. A meeting with no link is still a meeting, and someone
    /// whose calendar is mostly rooms and phone calls would otherwise get
    /// nothing.
    #[serde(default)]
    pub only_with_link: bool,
    /// Announce meetings the user declined. Off, because declining is the
    /// clearest possible statement that they are not going.
    #[serde(default)]
    pub include_declined: bool,
}

fn default_true() -> bool {
    true
}

fn default_leads() -> Vec<i64> {
    vec![5, 0]
}

impl Default for ReminderSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            lead_minutes: default_leads(),
            system_notification: true,
            only_with_link: false,
            include_declined: false,
        }
    }
}

impl ReminderSettings {
    /// The configured leads, cleaned up: known values only, largest first.
    ///
    /// Sorted largest-first so the bucket search can take the first match and
    /// stop; de-duplicated and filtered because these come off a settings file
    /// a user can edit by hand.
    pub fn buckets(&self) -> Vec<i64> {
        let mut leads: Vec<i64> = self
            .lead_minutes
            .iter()
            .copied()
            .filter(|lead| LEAD_CHOICES.contains(lead))
            .collect();
        leads.sort_unstable();
        leads.dedup();
        leads.reverse();
        leads
    }
}

/// A meeting Vox is about to announce.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MeetingReminder {
    /// `<account>|<event>|<bucket>`. Identity for "already said this".
    pub key: String,
    pub event_id: String,
    pub account_email: String,
    pub title: String,
    /// RFC 3339 start, so the surface showing this can run its own clock.
    pub start: String,
    pub conference_url: Option<String>,
    pub location: Option<String>,
    /// A recording Vox has already matched to this event, if any.
    pub meeting_id: Option<String>,
    /// Real minutes until it starts, from the clock — not the bucket.
    ///
    /// Negative means it has already begun. This is what the text says,
    /// which is why a reminder caught late reads "started 1 minute ago"
    /// rather than "in 5 minutes".
    pub minutes_until: i64,
    /// How many other people are invited.
    pub guest_count: usize,
}

impl MeetingReminder {
    /// The one-line summary a notification leads with.
    pub fn headline(&self) -> String {
        match self.minutes_until {
            minutes if minutes <= 0 && minutes > -1 => format!("{} is starting", self.title),
            minutes if minutes < 0 => format!(
                "{} started {} minute{} ago",
                self.title,
                -minutes,
                if minutes == -1 { "" } else { "s" }
            ),
            1 => format!("{} starts in 1 minute", self.title),
            minutes => format!("{} starts in {} minutes", self.title, minutes),
        }
    }

    /// The second line: where it is, and who else is in it.
    pub fn detail(&self) -> String {
        let mut parts: Vec<String> = Vec::new();
        if self.conference_url.is_some() {
            parts.push("Video call".to_string());
        } else if let Some(location) = self.location.as_deref() {
            parts.push(location.to_string());
        }
        if self.guest_count > 0 {
            parts.push(format!(
                "{} guest{}",
                self.guest_count,
                if self.guest_count == 1 { "" } else { "s" }
            ));
        }
        parts.join(" · ")
    }
}

/// Every reminder that has come due and has not been sent.
///
/// `already_sent` holds the keys of reminders that have gone out. It is the
/// caller's, deliberately: this function decides *what is due*, and nothing
/// about what is due depends on where the record of what was said is kept.
pub fn due_reminders(
    events: &[CalendarEvent],
    now: chrono::DateTime<chrono::Utc>,
    settings: &ReminderSettings,
    already_sent: &HashSet<String>,
) -> Vec<MeetingReminder> {
    if !settings.enabled {
        return Vec::new();
    }
    let buckets = settings.buckets();
    if buckets.is_empty() {
        return Vec::new();
    }

    let mut due = Vec::new();
    for event in events {
        // An all-day event is not something to join.
        if event.all_day {
            continue;
        }
        if !settings.include_declined && event.attendance == Attendance::Declined {
            continue;
        }
        if settings.only_with_link && event.conference_url.is_none() {
            continue;
        }
        let Some(start) = event.start_timestamp() else {
            continue;
        };

        let minutes_until = (start.with_timezone(&chrono::Utc) - now).num_minutes();
        if minutes_until < -LATE_GRACE_MINUTES {
            continue;
        }

        // The tightest bucket this event has reached. `buckets` is largest
        // first, so the last match is the smallest.
        let Some(bucket) = buckets
            .iter()
            .copied()
            .rfind(|lead| minutes_until <= *lead)
        else {
            continue;
        };

        let key = format!("{}|{}|{}", event.account_email, event.id, bucket);
        if already_sent.contains(&key) {
            continue;
        }

        due.push(MeetingReminder {
            key,
            event_id: event.id.clone(),
            account_email: event.account_email.clone(),
            title: event.title.clone(),
            start: event.start.clone(),
            conference_url: event.conference_url.clone(),
            location: event.location.clone(),
            meeting_id: event.meeting_id.clone(),
            minutes_until,
            guest_count: event.others().len(),
        });
    }

    // Soonest first: when two land in the same tick, the one about to start is
    // the one that matters.
    due.sort_by_key(|reminder| reminder.minutes_until);
    due
}

/// Keys worth keeping in the "already said this" set.
///
/// Anything for a meeting that is comfortably past can be forgotten: the set
/// otherwise grows for as long as Vox is open, and a key that can never match
/// again is a key that costs memory for nothing.
pub fn prune_sent(
    events: &[CalendarEvent],
    now: chrono::DateTime<chrono::Utc>,
    already_sent: &HashSet<String>,
) -> HashSet<String> {
    let live: HashSet<String> = events
        .iter()
        .filter(|event| {
            event.start_timestamp().is_some_and(|start| {
                (start.with_timezone(&chrono::Utc) - now).num_minutes() >= -LATE_GRACE_MINUTES
            })
        })
        .map(|event| format!("{}|{}", event.account_email, event.id))
        .collect();

    already_sent
        .iter()
        .filter(|key| {
            key.rsplit_once('|')
                .is_some_and(|(prefix, _)| live.contains(prefix))
        })
        .cloned()
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::calendar::model::EventAttendee;

    fn at(minutes_from_now: i64, now: chrono::DateTime<chrono::Utc>) -> String {
        (now + chrono::Duration::minutes(minutes_from_now)).to_rfc3339()
    }

    fn now() -> chrono::DateTime<chrono::Utc> {
        chrono::DateTime::parse_from_rfc3339("2026-09-15T09:57:00Z")
            .expect("a timestamp")
            .with_timezone(&chrono::Utc)
    }

    fn event(id: &str, starts_in: i64) -> CalendarEvent {
        let clock = now();
        CalendarEvent {
            id: id.to_string(),
            account_email: "me@work.com".into(),
            calendar_id: Some("primary".into()),
            title: "Weekly Sync".into(),
            start: at(starts_in, clock),
            end: at(starts_in + 30, clock),
            all_day: false,
            location: None,
            conference_url: Some("https://meet.google.com/abc-defg-hij".into()),
            html_link: None,
            description: None,
            recurring_event_id: None,
            attendees: Vec::new(),
            attendance: Attendance::Accepted,
            meeting_id: None,
        }
    }

    fn settings(leads: &[i64]) -> ReminderSettings {
        ReminderSettings {
            lead_minutes: leads.to_vec(),
            ..ReminderSettings::default()
        }
    }

    #[test]
    fn a_meeting_three_minutes_out_is_announced_once_not_once_per_lead() {
        // Vox opened at 09:57 for a 10:00 call, with 15, 5 and 0 configured.
        // One alarm per lead would be three notifications about one meeting,
        // two of them lying about how long is left.
        let due = due_reminders(&[event("evt-1", 3)], now(), &settings(&[15, 5, 0]), &HashSet::new());
        assert_eq!(due.len(), 1);
        assert_eq!(due[0].minutes_until, 3, "the clock, not the bucket");
        assert!(due[0].key.ends_with("|5"), "the tightest bucket reached");
    }

    #[test]
    fn the_text_comes_from_the_clock_so_it_is_never_wrong_about_the_time() {
        let due = due_reminders(&[event("evt-1", 3)], now(), &settings(&[15, 5, 0]), &HashSet::new());
        assert_eq!(due[0].headline(), "Weekly Sync starts in 3 minutes");

        let starting = due_reminders(&[event("evt-1", 0)], now(), &settings(&[0]), &HashSet::new());
        assert_eq!(starting[0].headline(), "Weekly Sync is starting");

        let late = due_reminders(&[event("evt-1", -2)], now(), &settings(&[0]), &HashSet::new());
        assert_eq!(late[0].headline(), "Weekly Sync started 2 minutes ago");
    }

    #[test]
    fn one_minute_reads_as_one_minute_rather_than_one_minutes() {
        let due = due_reminders(&[event("evt-1", 1)], now(), &settings(&[1]), &HashSet::new());
        assert_eq!(due[0].headline(), "Weekly Sync starts in 1 minute");
    }

    #[test]
    fn each_bucket_fires_once_and_the_next_one_still_fires() {
        let mut sent = HashSet::new();
        let clock = now();

        // Ten minutes out, with 15/5/0 configured: the 15 bucket.
        let first = due_reminders(&[event("evt-1", 10)], clock, &settings(&[15, 5, 0]), &sent);
        assert_eq!(first.len(), 1);
        sent.insert(first[0].key.clone());

        // Still ten minutes out on the next tick: nothing more to say.
        let again = due_reminders(&[event("evt-1", 10)], clock, &settings(&[15, 5, 0]), &sent);
        assert!(again.is_empty(), "a bucket fires once");

        // Four minutes out: a new bucket, which does fire.
        let closer = due_reminders(&[event("evt-1", 4)], clock, &settings(&[15, 5, 0]), &sent);
        assert_eq!(closer.len(), 1);
        assert!(closer[0].key.ends_with("|5"));
    }

    #[test]
    fn a_meeting_well_under_way_is_history_rather_than_a_reminder() {
        // Opening a laptop at 11:40 must not announce the 10:00 call.
        let late = due_reminders(
            &[event("evt-1", -(LATE_GRACE_MINUTES + 1))],
            now(),
            &settings(&[15, 5, 0]),
            &HashSet::new(),
        );
        assert!(late.is_empty());

        // A call that began a moment ago is still worth catching.
        let just_started = due_reminders(&[event("evt-1", -1)], now(), &settings(&[0]), &HashSet::new());
        assert_eq!(just_started.len(), 1);
    }

    #[test]
    fn a_meeting_further_out_than_every_lead_says_nothing_yet() {
        let due = due_reminders(&[event("evt-1", 60)], now(), &settings(&[15, 5]), &HashSet::new());
        assert!(due.is_empty());
    }

    #[test]
    fn an_all_day_event_is_not_something_to_join() {
        let mut holiday = event("evt-holiday", 3);
        holiday.all_day = true;
        assert!(due_reminders(&[holiday], now(), &settings(&[5]), &HashSet::new()).is_empty());
    }

    #[test]
    fn a_meeting_the_user_declined_is_not_announced_unless_they_ask() {
        let mut declined = event("evt-1", 3);
        declined.attendance = Attendance::Declined;
        assert!(
            due_reminders(&[declined.clone()], now(), &settings(&[5]), &HashSet::new()).is_empty()
        );

        let asking = ReminderSettings {
            include_declined: true,
            ..settings(&[5])
        };
        assert_eq!(due_reminders(&[declined], now(), &asking, &HashSet::new()).len(), 1);
    }

    #[test]
    fn a_meeting_with_no_video_link_is_still_a_meeting_by_default() {
        let mut in_person = event("evt-1", 3);
        in_person.conference_url = None;
        in_person.location = Some("Room 4".into());
        assert_eq!(
            due_reminders(&[in_person.clone()], now(), &settings(&[5]), &HashSet::new()).len(),
            1
        );

        let links_only = ReminderSettings {
            only_with_link: true,
            ..settings(&[5])
        };
        assert!(due_reminders(&[in_person], now(), &links_only, &HashSet::new()).is_empty());
    }

    #[test]
    fn turning_reminders_off_means_nothing_is_ever_due() {
        let off = ReminderSettings {
            enabled: false,
            ..settings(&[5, 0])
        };
        assert!(due_reminders(&[event("evt-1", 1)], now(), &off, &HashSet::new()).is_empty());
    }

    #[test]
    fn choosing_no_lead_times_is_the_same_as_choosing_silence() {
        assert!(due_reminders(&[event("evt-1", 1)], now(), &settings(&[]), &HashSet::new()).is_empty());
    }

    #[test]
    fn a_hand_edited_settings_file_cannot_produce_a_lead_nobody_offers() {
        let odd = ReminderSettings {
            lead_minutes: vec![7, 5, 5, 999],
            ..ReminderSettings::default()
        };
        assert_eq!(odd.buckets(), vec![5], "unknown leads dropped, duplicates collapsed");
    }

    #[test]
    fn the_soonest_meeting_leads_when_two_land_together() {
        let due = due_reminders(
            &[event("evt-late", 4), event("evt-soon", 1)],
            now(),
            &settings(&[5]),
            &HashSet::new(),
        );
        assert_eq!(due[0].event_id, "evt-soon");
    }

    #[test]
    fn the_detail_line_says_where_and_with_whom() {
        let mut with_guests = event("evt-1", 3);
        with_guests.attendees = vec![
            EventAttendee {
                email: "me@work.com".into(),
                display_name: None,
                response: Attendance::Accepted,
                is_self: true,
                organizer: false,
            },
            EventAttendee {
                email: "payal@navgurukul.org".into(),
                display_name: Some("Payal".into()),
                response: Attendance::Accepted,
                is_self: false,
                organizer: true,
            },
        ];
        let due = due_reminders(&[with_guests], now(), &settings(&[5]), &HashSet::new());
        assert_eq!(due[0].detail(), "Video call · 1 guest");
    }

    #[test]
    fn what_was_said_about_a_finished_meeting_is_forgotten() {
        // The set would otherwise grow for as long as Vox stays open.
        let clock = now();
        let live = event("evt-live", 3);
        let sent: HashSet<String> = ["me@work.com|evt-live|5", "me@work.com|evt-gone|5"]
            .iter()
            .map(|key| key.to_string())
            .collect();

        let kept = prune_sent(&[live], clock, &sent);
        assert!(kept.contains("me@work.com|evt-live|5"));
        assert!(!kept.contains("me@work.com|evt-gone|5"));
    }
}
