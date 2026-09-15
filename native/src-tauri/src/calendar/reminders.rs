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
//!
//! ## Where it appears
//!
//! In a window of its own ([`super::reminder_window`]) — not inside the app,
//! which only reaches somebody already looking at Vox, and not as a Windows
//! toast, which the notification centre swallows.

use std::collections::HashSet;

use serde::{Deserialize, Serialize};

use super::model::{Attendance, CalendarEvent};

/// How long after a meeting has started a reminder is still worth sending.
///
/// Covers the laptop opened a minute into a call. Anything older is history,
/// and announcing history trains people to ignore the notification.
pub const LATE_GRACE_MINUTES: i64 = 5;

/// How long into a meeting the "nothing is being recorded" nudge waits.
///
/// Late enough that somebody who joined and pressed record in the first
/// minute is never nagged, early enough that the nudge still saves most of
/// the conversation.
pub const NOT_RECORDING_AFTER_MINUTES: i64 = 5;

/// What a reminder is telling you.
///
/// Three, and they are genuinely different messages rather than one message
/// at three times: the first two are "this is about to happen", and the third
/// is "this is happening and Vox is not capturing it", which is the failure a
/// meeting recorder exists to prevent and the only one whose action is not
/// optional.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReminderKind {
    /// Before it starts — t−15, −10, −5, −1.
    Upcoming,
    /// At the start, or a moment after it — t.
    Starting,
    /// Under way, with nothing being recorded — t+5.
    NotRecording,
}

impl ReminderKind {
    /// The slug, for a test reminder's own key. Same value as [`Self::slug`],
    /// exposed because the test command builds a key by hand.
    pub fn slug_for_test(self) -> &'static str {
        self.slug()
    }

    /// The part of a reminder's key that keeps the kinds apart.
    fn slug(self) -> &'static str {
        match self {
            Self::Upcoming => "upcoming",
            Self::Starting => "starting",
            Self::NotRecording => "not-recording",
        }
    }
}

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
    /// Whether to say something when a meeting is under way and Vox is
    /// recording nothing.
    ///
    /// On by default, and the reminder that matters most: a meeting that
    /// happened and was not captured is the failure the whole surface exists
    /// to prevent, and it is silent by nature — nothing goes wrong on screen.
    #[serde(default = "default_true")]
    pub nudge_when_not_recording: bool,
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
            only_with_link: false,
            include_declined: false,
            nudge_when_not_recording: true,
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
    /// Which of the three messages this is.
    pub kind: ReminderKind,
}

impl MeetingReminder {
    /// The one-line summary a reminder leads with.
    ///
    /// Written from the clock rather than from the bucket that fired it, so a
    /// reminder caught late says how late rather than repeating the lead time
    /// it was configured with.
    pub fn headline(&self) -> String {
        match self.kind {
            ReminderKind::NotRecording => format!("{} is not being recorded", self.title),
            _ => match self.minutes_until {
                minutes if minutes <= 0 && minutes > -1 => format!("{} is starting", self.title),
                minutes if minutes < 0 => format!(
                    "{} started {} minute{} ago",
                    self.title,
                    -minutes,
                    if minutes == -1 { "" } else { "s" }
                ),
                1 => format!("{} starts in 1 minute", self.title),
                minutes => format!("{} starts in {} minutes", self.title, minutes),
            },
        }
    }

    /// The second line: what state it is in, where, and who else is in it.
    pub fn detail(&self) -> String {
        let mut parts: Vec<String> = Vec::new();
        if self.kind == ReminderKind::NotRecording {
            let since = -self.minutes_until;
            parts.push(format!(
                "Started {since} minute{} ago",
                if since == 1 { "" } else { "s" }
            ));
        }
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
///
/// `recording_active` is whether Vox is capturing anything at all right now.
/// Anything, rather than "this meeting specifically": if a recording is
/// running during a meeting's own hour it is almost certainly that meeting,
/// and nagging somebody who is already recording is how a person learns to
/// ignore the one reminder that matters.
///
/// At most one reminder per event per tick — the most urgent that applies.
pub fn due_reminders(
    events: &[CalendarEvent],
    now: chrono::DateTime<chrono::Utc>,
    settings: &ReminderSettings,
    already_sent: &HashSet<String>,
    recording_active: bool,
) -> Vec<MeetingReminder> {
    if !settings.enabled {
        return Vec::new();
    }
    let buckets = settings.buckets();

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
        let (Some(start), Some(end)) = (event.start_timestamp(), event.end_timestamp()) else {
            continue;
        };

        let minutes_until = (start.with_timezone(&chrono::Utc) - now).num_minutes();
        let ended = end.with_timezone(&chrono::Utc) <= now;

        let Some((kind, suffix)) = kind_for(
            minutes_until,
            ended,
            &buckets,
            settings,
            recording_active,
            event.meeting_id.is_some(),
        ) else {
            continue;
        };

        let key = format!(
            "{}|{}|{}:{}",
            event.account_email,
            event.id,
            kind.slug(),
            suffix
        );
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
            kind,
        });
    }

    // Soonest first: when two land in the same tick, the one about to start —
    // or already running unrecorded — is the one that matters.
    due.sort_by_key(|reminder| reminder.minutes_until);
    due
}

/// Which reminder, if any, one event has earned right now.
///
/// The suffix is what keeps a kind from firing twice for the same reason: the
/// bucket that triggered an upcoming reminder, and a constant for the others,
/// which fire once each per meeting.
fn kind_for(
    minutes_until: i64,
    ended: bool,
    buckets: &[i64],
    settings: &ReminderSettings,
    recording_active: bool,
    already_recorded: bool,
) -> Option<(ReminderKind, String)> {
    // Under way and unrecorded. Checked first: once a meeting is running with
    // nothing being captured, that is the more urgent thing to say, and the
    // start reminder for it has either fired already or is moot.
    if settings.nudge_when_not_recording
        && !ended
        && !recording_active
        && !already_recorded
        && minutes_until <= -NOT_RECORDING_AFTER_MINUTES
    {
        return Some((ReminderKind::NotRecording, "once".to_string()));
    }

    if ended || minutes_until < -LATE_GRACE_MINUTES {
        return None;
    }

    // The tightest bucket this event has reached. `buckets` is largest first,
    // so the last match is the smallest.
    let bucket = buckets.iter().copied().rfind(|lead| minutes_until <= *lead)?;
    let kind = if bucket > 0 {
        ReminderKind::Upcoming
    } else {
        ReminderKind::Starting
    };
    Some((kind, bucket.to_string()))
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
        let due = due_reminders(&[event("evt-1", 3)], now(), &settings(&[15, 5, 0]), &HashSet::new(), false);
        assert_eq!(due.len(), 1);
        assert_eq!(due[0].minutes_until, 3, "the clock, not the bucket");
        assert!(due[0].key.ends_with("upcoming:5"), "the tightest bucket reached");
    }

    #[test]
    fn the_text_comes_from_the_clock_so_it_is_never_wrong_about_the_time() {
        let due = due_reminders(&[event("evt-1", 3)], now(), &settings(&[15, 5, 0]), &HashSet::new(), false);
        assert_eq!(due[0].headline(), "Weekly Sync starts in 3 minutes");

        let starting = due_reminders(&[event("evt-1", 0)], now(), &settings(&[0]), &HashSet::new(), false);
        assert_eq!(starting[0].headline(), "Weekly Sync is starting");

        let late = due_reminders(&[event("evt-1", -2)], now(), &settings(&[0]), &HashSet::new(), false);
        assert_eq!(late[0].headline(), "Weekly Sync started 2 minutes ago");
    }

    #[test]
    fn one_minute_reads_as_one_minute_rather_than_one_minutes() {
        let due = due_reminders(&[event("evt-1", 1)], now(), &settings(&[1]), &HashSet::new(), false);
        assert_eq!(due[0].headline(), "Weekly Sync starts in 1 minute");
    }

    #[test]
    fn each_bucket_fires_once_and_the_next_one_still_fires() {
        let mut sent = HashSet::new();
        let clock = now();

        // Ten minutes out, with 15/5/0 configured: the 15 bucket.
        let first = due_reminders(&[event("evt-1", 10)], clock, &settings(&[15, 5, 0]), &sent, false);
        assert_eq!(first.len(), 1);
        sent.insert(first[0].key.clone());

        // Still ten minutes out on the next tick: nothing more to say.
        let again = due_reminders(&[event("evt-1", 10)], clock, &settings(&[15, 5, 0]), &sent, false);
        assert!(again.is_empty(), "a bucket fires once");

        // Four minutes out: a new bucket, which does fire.
        let closer = due_reminders(&[event("evt-1", 4)], clock, &settings(&[15, 5, 0]), &sent, false);
        assert_eq!(closer.len(), 1);
        assert!(closer[0].key.ends_with("upcoming:5"));
    }

    #[test]
    fn a_meeting_well_under_way_is_never_announced_as_if_it_were_starting() {
        // Opening a laptop at 11:40 must not announce the 10:00 call as
        // though it were about to begin. What it may say — once the meeting
        // is running unrecorded — is the nudge, which is a different message.
        let quiet = ReminderSettings {
            nudge_when_not_recording: false,
            ..settings(&[15, 5, 0])
        };
        let late = due_reminders(
            &[event("evt-1", -(LATE_GRACE_MINUTES + 1))],
            now(),
            &quiet,
            &HashSet::new(),
            false,
        );
        assert!(late.is_empty());

        // A call that began a moment ago is still worth catching.
        let just_started = due_reminders(&[event("evt-1", -1)], now(), &settings(&[0]), &HashSet::new(), false);
        assert_eq!(just_started.len(), 1);
        assert_eq!(just_started[0].kind, ReminderKind::Starting);
    }

    #[test]
    fn a_meeting_further_out_than_every_lead_says_nothing_yet() {
        let due = due_reminders(&[event("evt-1", 60)], now(), &settings(&[15, 5]), &HashSet::new(), false);
        assert!(due.is_empty());
    }

    #[test]
    fn an_all_day_event_is_not_something_to_join() {
        let mut holiday = event("evt-holiday", 3);
        holiday.all_day = true;
        assert!(due_reminders(&[holiday], now(), &settings(&[5]), &HashSet::new(), false).is_empty());
    }

    #[test]
    fn a_meeting_the_user_declined_is_not_announced_unless_they_ask() {
        let mut declined = event("evt-1", 3);
        declined.attendance = Attendance::Declined;
        assert!(
            due_reminders(&[declined.clone()], now(), &settings(&[5]), &HashSet::new(), false).is_empty()
        );

        let asking = ReminderSettings {
            include_declined: true,
            ..settings(&[5])
        };
        assert_eq!(due_reminders(&[declined], now(), &asking, &HashSet::new(), false).len(), 1);
    }

    #[test]
    fn a_meeting_with_no_video_link_is_still_a_meeting_by_default() {
        let mut in_person = event("evt-1", 3);
        in_person.conference_url = None;
        in_person.location = Some("Room 4".into());
        assert_eq!(
            due_reminders(&[in_person.clone()], now(), &settings(&[5]), &HashSet::new(), false).len(),
            1
        );

        let links_only = ReminderSettings {
            only_with_link: true,
            ..settings(&[5])
        };
        assert!(due_reminders(&[in_person], now(), &links_only, &HashSet::new(), false).is_empty());
    }

    #[test]
    fn turning_reminders_off_means_nothing_is_ever_due() {
        let off = ReminderSettings {
            enabled: false,
            ..settings(&[5, 0])
        };
        assert!(due_reminders(&[event("evt-1", 1)], now(), &off, &HashSet::new(), false).is_empty());
    }

    #[test]
    fn choosing_no_lead_times_is_the_same_as_choosing_silence() {
        assert!(due_reminders(&[event("evt-1", 1)], now(), &settings(&[]), &HashSet::new(), false).is_empty());
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
            false,
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
        let due = due_reminders(&[with_guests], now(), &settings(&[5]), &HashSet::new(), false);
        assert_eq!(due[0].detail(), "Video call · 1 guest");
    }

    #[test]
    fn a_meeting_under_way_with_nothing_recording_is_the_reminder_that_matters() {
        // The failure a meeting recorder exists to prevent, and the only one
        // that is silent by nature: nothing goes wrong on screen.
        let due = due_reminders(
            &[event("evt-1", -NOT_RECORDING_AFTER_MINUTES)],
            now(),
            &settings(&[5, 0]),
            &HashSet::new(),
            false,
        );
        assert_eq!(due.len(), 1);
        assert_eq!(due[0].kind, ReminderKind::NotRecording);
        assert_eq!(due[0].headline(), "Weekly Sync is not being recorded");
        assert!(due[0].detail().starts_with("Started 5 minutes ago"));
    }

    #[test]
    fn somebody_already_recording_is_not_nagged_about_it() {
        // Nagging a person who is already recording is how they learn to
        // ignore the one reminder that matters.
        let due = due_reminders(
            &[event("evt-1", -NOT_RECORDING_AFTER_MINUTES)],
            now(),
            &settings(&[5, 0]),
            &HashSet::new(),
            true,
        );
        assert!(due.iter().all(|r| r.kind != ReminderKind::NotRecording));
    }

    #[test]
    fn a_meeting_vox_already_recorded_is_not_nagged_about_either() {
        // The link to a recording is worked out on read, so an event that
        // carries one has been captured — an hour ago, perhaps, by a
        // recording that has since stopped.
        let mut recorded = event("evt-1", -NOT_RECORDING_AFTER_MINUTES);
        recorded.meeting_id = Some("meeting-1".into());
        let due = due_reminders(&[recorded], now(), &settings(&[5, 0]), &HashSet::new(), false);
        assert!(due.iter().all(|r| r.kind != ReminderKind::NotRecording));
    }

    #[test]
    fn the_nudge_waits_rather_than_firing_the_moment_a_meeting_starts() {
        // Somebody who joined and pressed record in the first minute must
        // never see it.
        let due = due_reminders(
            &[event("evt-1", -1)],
            now(),
            &settings(&[0]),
            // The start reminder has already gone out, so only the nudge
            // could fire here — and it is too early for it.
            &["me@work.com|evt-1|starting:0".to_string()].into_iter().collect(),
            false,
        );
        assert!(due.is_empty());
    }

    #[test]
    fn a_meeting_that_has_ended_is_not_nagged_about() {
        // 30 minutes long, ended 5 minutes ago.
        let due = due_reminders(
            &[event("evt-1", -35)],
            now(),
            &settings(&[5, 0]),
            &HashSet::new(),
            false,
        );
        assert!(due.is_empty());
    }

    #[test]
    fn turning_the_nudge_off_leaves_the_other_two_alone() {
        let quiet = ReminderSettings {
            nudge_when_not_recording: false,
            ..settings(&[5, 0])
        };
        let due = due_reminders(
            &[event("evt-1", -NOT_RECORDING_AFTER_MINUTES)],
            now(),
            &quiet,
            &HashSet::new(),
            false,
        );
        assert!(due.iter().all(|r| r.kind != ReminderKind::NotRecording));

        let upcoming = due_reminders(&[event("evt-1", 3)], now(), &quiet, &HashSet::new(), false);
        assert_eq!(upcoming[0].kind, ReminderKind::Upcoming);
    }

    #[test]
    fn the_three_kinds_do_not_cancel_each_other_out() {
        // Each fires once, and having said one does not suppress the next.
        let mut sent = HashSet::new();
        let leads = settings(&[5, 0]);

        let upcoming = due_reminders(&[event("evt-1", 3)], now(), &leads, &sent, false);
        assert_eq!(upcoming[0].kind, ReminderKind::Upcoming);
        sent.insert(upcoming[0].key.clone());

        let starting = due_reminders(&[event("evt-1", 0)], now(), &leads, &sent, false);
        assert_eq!(starting[0].kind, ReminderKind::Starting);
        sent.insert(starting[0].key.clone());

        let nudge = due_reminders(
            &[event("evt-1", -NOT_RECORDING_AFTER_MINUTES)],
            now(),
            &leads,
            &sent,
            false,
        );
        assert_eq!(nudge[0].kind, ReminderKind::NotRecording);
    }

    #[test]
    fn a_bucket_below_the_start_is_a_starting_reminder_rather_than_an_upcoming_one() {
        let starting = due_reminders(&[event("evt-1", 0)], now(), &settings(&[0]), &HashSet::new(), false);
        assert_eq!(starting[0].kind, ReminderKind::Starting);
        let upcoming = due_reminders(&[event("evt-1", 4)], now(), &settings(&[5]), &HashSet::new(), false);
        assert_eq!(upcoming[0].kind, ReminderKind::Upcoming);
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
