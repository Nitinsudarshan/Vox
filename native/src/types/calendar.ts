/**
 * The calendar vocabulary, mirroring `native/src-tauri/src/calendar/model.rs`.
 *
 * Serde writes these shapes verbatim, so a field that changes here has to
 * change there too.
 */

/** Whether the user is going, as far as the calendar knows. */
export type Attendance =
  | 'accepted'
  | 'declined'
  | 'tentative'
  | 'needs_action'
  /** No attendee list, or the user is not on it — usually their own event. */
  | 'unknown';

/**
 * One Google account whose calendars Vox reads.
 *
 * Several of these, deliberately: work lives in one account and life in
 * another, and "what am I doing today" is the union of them.
 */
export interface CalendarAccount {
  email: string;
  display_name?: string | null;
  /** Off hides the events and keeps the tokens. */
  enabled: boolean;
  /** Empty means the primary calendar. */
  calendar_ids: string[];
  last_synced_at?: string | null;
  /** The last sync failure, cleared by a good sync. */
  last_error?: string | null;
}

export interface EventAttendee {
  email: string;
  display_name?: string | null;
  response: Attendance;
  /** Whether this attendee is the account holder. */
  is_self: boolean;
  organizer: boolean;
}

export interface CalendarEvent {
  id: string;
  /** Which account this copy came from. */
  account_email: string;
  calendar_id?: string | null;
  title: string;
  /** RFC 3339. An all-day event is midnight-to-midnight local. */
  start: string;
  end: string;
  all_day: boolean;
  location?: string | null;
  conference_url?: string | null;
  html_link?: string | null;
  /**
   * The invitation's own notes — agenda, dial-in, links — as plain text.
   *
   * Google stores this as HTML; the Rust side flattens it, so this is text to
   * render as text and never as markup.
   */
  description?: string | null;
  attendees: EventAttendee[];
  attendance: Attendance;
  /** A recording Vox made that covers this event's time, once one exists. */
  meeting_id?: string | null;
}

/** One day's events across every connected account. */
export interface DayAgenda {
  /** `YYYY-MM-DD` in the viewer's timezone. */
  date: string;
  events: CalendarEvent[];
}

/** A calendar an account can read. */
export interface CalendarSummary {
  id: string;
  name: string;
  primary: boolean;
  selected: boolean;
}
