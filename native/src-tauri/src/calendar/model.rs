//! What a calendar and an event *are*, on disk and over the IPC boundary.
//!
//! Serialized verbatim in both directions, the way `meetings::model` is, so a
//! field that nothing writes is a field that is visibly absent rather than a
//! column migrated into existence and left empty.

use serde::{Deserialize, Serialize};

/// One Google account whose calendars Vox reads.
///
/// Several of these, deliberately. A working person has a work calendar and a
/// personal calendar in two different Google accounts, and the interesting
/// question — "what am I actually doing today" — is the *union* of them. A
/// single-account integration answers a question nobody asked.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CalendarAccount {
    /// The signed-in address. Also the identity of the token entry, so it is
    /// what a disconnect deletes.
    pub email: String,
    #[serde(default)]
    pub display_name: Option<String>,
    /// Whether this account's events appear in the agenda. Off keeps the
    /// tokens and hides the events, which is what someone wants on the evening
    /// they do not care what work says.
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// Which of the account's calendars are read. Empty means the primary one.
    #[serde(default)]
    pub calendar_ids: Vec<String>,
    #[serde(default)]
    pub last_synced_at: Option<String>,
    /// The last sync failure, if the last sync failed. Cleared by a good one.
    #[serde(default)]
    pub last_error: Option<String>,
}

fn default_true() -> bool {
    true
}

/// Whether the user is going, as far as the calendar knows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Attendance {
    Accepted,
    Declined,
    Tentative,
    /// Invited and not answered.
    NeedsAction,
    /// No attendee list, or the user is not on it — usually an event they
    /// created for themselves.
    Unknown,
}

impl Attendance {
    /// Parses Google's `responseStatus`, defaulting rather than failing: an
    /// unrecognised status should hide an event from nobody.
    pub fn from_google(status: &str) -> Self {
        match status.trim().to_lowercase().as_str() {
            "accepted" => Self::Accepted,
            "declined" => Self::Declined,
            "tentative" => Self::Tentative,
            "needsaction" => Self::NeedsAction,
            _ => Self::Unknown,
        }
    }
}

/// Somebody on the invitation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EventAttendee {
    pub email: String,
    #[serde(default)]
    pub display_name: Option<String>,
    pub response: Attendance,
    /// Whether this attendee is the account holder.
    #[serde(default)]
    pub is_self: bool,
    #[serde(default)]
    pub organizer: bool,
}

/// One event, from one account.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CalendarEvent {
    /// Google's event id. Unique within a calendar, *not* across accounts —
    /// the same invitation in a work and a personal inbox carries the same id,
    /// which is what makes de-duplication possible at all.
    pub id: String,
    /// Which account this copy came from, so the UI can say where a meeting
    /// is from and so a disconnect can drop exactly its events.
    pub account_email: String,
    #[serde(default)]
    pub calendar_id: Option<String>,
    pub title: String,
    /// RFC 3339. An all-day event is stored as midnight-to-midnight local so
    /// that one comparison works for both kinds.
    pub start: String,
    pub end: String,
    #[serde(default)]
    pub all_day: bool,
    #[serde(default)]
    pub location: Option<String>,
    /// The video call to join, where the invitation carries one.
    #[serde(default)]
    pub conference_url: Option<String>,
    #[serde(default)]
    pub html_link: Option<String>,
    /// The invitation's own notes, as plain text.
    ///
    /// This is where the agenda, the dial-in details and the "read this
    /// first" link actually live, and reading them in Vox is the difference
    /// between a row that says a meeting exists and one that says what it is
    /// for. Google writes the field as HTML; [`super::parse`] flattens it on
    /// the way in, so nothing downstream has to render markup it did not
    /// write.
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub attendees: Vec<EventAttendee>,
    pub attendance: Attendance,
    /// A meeting Vox recorded that covers this event's time, once one exists.
    #[serde(default)]
    pub meeting_id: Option<String>,
}

impl CalendarEvent {
    /// Start as a timestamp, or `None` when the calendar sent something
    /// unparseable — which is a reason to skip an event, never to fail a sync.
    pub fn start_timestamp(&self) -> Option<chrono::DateTime<chrono::FixedOffset>> {
        chrono::DateTime::parse_from_rfc3339(&self.start).ok()
    }

    pub fn end_timestamp(&self) -> Option<chrono::DateTime<chrono::FixedOffset>> {
        chrono::DateTime::parse_from_rfc3339(&self.end).ok()
    }

    /// Everyone invited who is not the account holder.
    pub fn others(&self) -> Vec<&EventAttendee> {
        self.attendees.iter().filter(|a| !a.is_self).collect()
    }
}

/// One day's events across every connected account.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DayAgenda {
    /// `YYYY-MM-DD` in the viewer's timezone.
    pub date: String,
    pub events: Vec<CalendarEvent>,
}
