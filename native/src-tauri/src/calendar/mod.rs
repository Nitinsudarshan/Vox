//! Google Calendar, read-only, across as many accounts as the user has.
//!
//! ## Shape
//!
//! ```text
//! account A ─┐
//!            ├─ google::list_events ─ parse ─┐
//! account B ─┘                               ├─ agenda::deduplicate ─ build_agenda ─ DayAgenda[]
//!                                            │
//! meetings/*/meeting.json ── recording windows ┘  (agenda::match_recordings)
//! ```
//!
//! ## Why more than one account
//!
//! Because one is not how anybody's calendar works. Work lives in a work
//! Google account and life lives in a personal one, the two do not merge, and
//! the question worth answering — *what am I actually doing today* — is the
//! union of them. Five work meetings plus one personal appointment is six
//! rows, once each, in time order.
//!
//! That "once each" is not free: an invitation sent to both addresses arrives
//! twice, and Google gives both copies the same event id, which is what
//! [`agenda::deduplicate`] keys on.
//!
//! ## Why read-only
//!
//! `SCOPE_CALENDAR_READONLY` and nothing more. Vox reads a calendar to know
//! what a recording *was*; it has no reason to be able to change one, and the
//! consent screen saying so is the difference between a reasonable ask and one
//! people decline.
//!
//! ## What the calendar is for, inside Vox
//!
//! Two things, and neither is "show a calendar":
//!
//! - A recording gets a **name and a guest list** it could not otherwise have.
//!   `meeting-2026-09-14-1030` becomes "Alumna Growth Team Daily Sync" with
//!   the people who were invited to it.
//! - The agenda shows **which meetings have notes**, so the day reads as a
//!   record rather than a schedule.
//!
//! A calendar attendee is not evidence that somebody spoke, and nothing here
//! feeds speaker attribution: `meetings::speakers` works from the audio.

pub mod agenda;
pub mod commands;
pub mod google;
pub mod model;
pub mod parse;
pub mod reminders;
pub mod store;

pub use model::{Attendance, CalendarAccount, CalendarEvent, DayAgenda, EventAttendee};
pub use store::{CalendarStore, CalendarStoreError};
