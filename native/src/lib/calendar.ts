/**
 * The calendar command wrappers.
 *
 * One place where `invoke` names live, so a renamed Rust command breaks at
 * compile time in one file rather than at runtime in six components.
 */

import { invoke } from '@tauri-apps/api/core';
import type {
  CalendarAccount,
  CalendarEvent,
  CalendarSummary,
  DayAgenda,
} from '@/types/calendar';

export const listCalendarAccounts = (): Promise<CalendarAccount[]> =>
  invoke('list_calendar_accounts');

/** Opens Google's consent screen. Resolves once the account is stored. */
export const connectCalendarAccount = (): Promise<CalendarAccount[]> =>
  invoke('connect_calendar_account');

export const disconnectCalendarAccount = (email: string): Promise<CalendarAccount[]> =>
  invoke('disconnect_calendar_account', { email });

export const setCalendarAccountEnabled = (
  email: string,
  enabled: boolean,
): Promise<CalendarAccount[]> =>
  invoke('set_calendar_account_enabled', { email, enabled });

export const listAccountCalendars = (email: string): Promise<CalendarSummary[]> =>
  invoke('list_account_calendars', { email });

export const setAccountCalendars = (
  email: string,
  calendarIds: string[],
): Promise<CalendarAccount[]> => invoke('set_account_calendars', { email, calendarIds });

/** Fetches every enabled account. One account failing never fails the sync. */
export const syncCalendars = (): Promise<CalendarAccount[]> => invoke('sync_calendars');

/** The cached agenda, with recordings already matched to events. */
export const getCalendarAgenda = (): Promise<DayAgenda[]> => invoke('get_calendar_agenda');

/**
 * Opens a join link, or an event in Google Calendar, in the default browser.
 *
 * The backend refuses anything that is not `http(s)` and anything that is not
 * on one of the user's own cached events, so this cannot be used to open an
 * arbitrary URL — which is why it is a command rather than an `<a href>`.
 */
export const openCalendarLink = (url: string): Promise<void> =>
  invoke('open_calendar_link', { url });

// --- formatting ---------------------------------------------------------

/** `HH:MM` for a timed event; nothing for an all-day one. */
export function eventTime(event: CalendarEvent): string {
  if (event.all_day) return 'All day';
  const start = new Date(event.start);
  if (Number.isNaN(start.getTime())) return '';
  return start.toLocaleTimeString(undefined, { hour: 'numeric', minute: '2-digit' });
}

/** How long an event runs, for a row that already shows when it starts. */
export function eventDuration(event: CalendarEvent): string {
  if (event.all_day) return '';
  const start = new Date(event.start).getTime();
  const end = new Date(event.end).getTime();
  if (!Number.isFinite(start) || !Number.isFinite(end) || end <= start) return '';
  const minutes = Math.round((end - start) / 60_000);
  if (minutes < 60) return `${minutes} min`;
  const hours = Math.floor(minutes / 60);
  const rest = minutes % 60;
  return rest === 0 ? `${hours} h` : `${hours} h ${rest} min`;
}

/**
 * Who else is invited, as a sentence.
 *
 * The account holder is left out: a list that starts with your own name tells
 * you nothing you did not know.
 */
export function attendeeSummary(event: CalendarEvent, limit = 3): string {
  const others = event.attendees.filter((attendee) => !attendee.is_self);
  if (others.length === 0) return '';
  const names = others
    .slice(0, limit)
    .map((attendee) => attendee.display_name?.trim() || attendee.email.split('@')[0]);
  const remaining = others.length - names.length;
  return remaining > 0 ? `${names.join(', ')} +${remaining}` : names.join(', ');
}

/**
 * Whether an event has already finished.
 *
 * Used to split "coming up" from the rest of the day, so the agenda opens on
 * what is about to happen rather than on what already did.
 */
export function isPast(event: CalendarEvent, now: Date = new Date()): boolean {
  const end = new Date(event.end).getTime();
  return Number.isFinite(end) && end < now.getTime();
}

/** Whether a day string is today, in the viewer's timezone. */
export function isToday(date: string, now: Date = new Date()): boolean {
  const local = `${now.getFullYear()}-${String(now.getMonth() + 1).padStart(2, '0')}-${String(
    now.getDate(),
  ).padStart(2, '0')}`;
  return date === local;
}

/** The heading a day sits under: "Today", "Tomorrow", or its date. */
export function agendaDayLabel(date: string, now: Date = new Date()): string {
  const parsed = new Date(`${date}T00:00:00`);
  if (Number.isNaN(parsed.getTime())) return date;

  const midnight = (value: Date) =>
    new Date(value.getFullYear(), value.getMonth(), value.getDate()).getTime();
  const days = Math.round((midnight(parsed) - midnight(now)) / 86_400_000);

  if (days === 0) return 'Today';
  if (days === 1) return 'Tomorrow';
  if (days === -1) return 'Yesterday';
  return parsed.toLocaleDateString(undefined, {
    weekday: 'long',
    month: 'short',
    day: 'numeric',
    ...(parsed.getFullYear() === now.getFullYear() ? {} : { year: 'numeric' }),
  });
}

// --- joining ------------------------------------------------------------

/**
 * How long before its own day a join link becomes live, in minutes.
 *
 * Exists for one case: a meeting at 00:15 that everybody thinks of as
 * tonight's. Without the lead it becomes reachable fifteen minutes after it
 * started.
 */
export const JOIN_LEAD_MINUTES = 30;

/** Midnight at the start of the local day `when` falls on. */
function startOfLocalDay(when: Date): number {
  return new Date(when.getFullYear(), when.getMonth(), when.getDate()).getTime();
}

/** Midnight at the end of the local day `when` falls on. */
function endOfLocalDay(when: Date): number {
  return new Date(when.getFullYear(), when.getMonth(), when.getDate() + 1).getTime();
}

/**
 * Whether an event's join link should be pressable right now.
 *
 * Live for the whole of the event's own day, rather than for the meeting's own
 * hour. Two things pushed the window out that far: a call that breaks and
 * resumes on the same link an hour later, and a meeting that runs long — both
 * of which a tight window turns into a trip to the calendar. Two things keep
 * it from being every event Vox has synced: a link ten days out is one you
 * only ever press by accident, and a link from last Tuesday opens a room
 * nobody is in.
 */
export function canJoinEvent(event: CalendarEvent, now: Date = new Date()): boolean {
  if (!event.conference_url) return false;
  const start = new Date(event.start);
  if (Number.isNaN(start.getTime())) return false;
  const opensAt = startOfLocalDay(start) - JOIN_LEAD_MINUTES * 60_000;
  return now.getTime() >= opensAt && now.getTime() < endOfLocalDay(start);
}

/** Why a join link is not pressable yet, for the control's tooltip. */
export function joinUnavailableReason(event: CalendarEvent, now: Date = new Date()): string {
  if (!event.conference_url) return 'No video link on this invitation.';
  const start = new Date(event.start);
  if (Number.isNaN(start.getTime())) return 'This event has no usable start time.';
  if (now.getTime() >= endOfLocalDay(start)) return 'This meeting was on an earlier day.';
  return 'Opens on the day of the meeting.';
}

/** Whether the user said no to this invitation. */
export function isDeclined(event: CalendarEvent): boolean {
  return event.attendance === 'declined';
}

// --- telling the accounts apart -----------------------------------------

/**
 * The colours accounts are distinguished by, in the order they are handed out.
 *
 * Six, because nobody keeps seven Google accounts, and hue rather than shade
 * because two accounts have to be told apart at a glance in a list where every
 * other row is also grey. Tailwind palette classes rather than tokens: these
 * are identity colours with no semantic meaning, and the theme's own
 * foreground/accent tokens have exactly one of each.
 */
export const ACCOUNT_COLORS = [
  {
    dot: 'bg-emerald-500',
    text: 'text-emerald-700 dark:text-emerald-400',
    soft: 'bg-emerald-500/10',
    border: 'border-l-emerald-500',
  },
  {
    dot: 'bg-sky-500',
    text: 'text-sky-700 dark:text-sky-400',
    soft: 'bg-sky-500/10',
    border: 'border-l-sky-500',
  },
  {
    dot: 'bg-violet-500',
    text: 'text-violet-700 dark:text-violet-400',
    soft: 'bg-violet-500/10',
    border: 'border-l-violet-500',
  },
  {
    dot: 'bg-amber-500',
    text: 'text-amber-700 dark:text-amber-400',
    soft: 'bg-amber-500/10',
    border: 'border-l-amber-500',
  },
  {
    dot: 'bg-rose-500',
    text: 'text-rose-700 dark:text-rose-400',
    soft: 'bg-rose-500/10',
    border: 'border-l-rose-500',
  },
  {
    dot: 'bg-teal-500',
    text: 'text-teal-700 dark:text-teal-400',
    soft: 'bg-teal-500/10',
    border: 'border-l-teal-500',
  },
] as const;

export type AccountColor = (typeof ACCOUNT_COLORS)[number];

/**
 * The colour one account's events wear.
 *
 * Keyed on the account's position in the connected list, so the colours stay
 * put as long as the accounts do — a colour that moves when an unrelated
 * account syncs teaches the user nothing. An address that is not in the list
 * (an event cached from an account since disconnected) falls back to a hash,
 * which is stable for the same reason.
 */
export function accountColor(email: string, connected: string[]): AccountColor {
  const lowered = email.trim().toLowerCase();
  const position = connected.findIndex(
    (candidate) => candidate.trim().toLowerCase() === lowered,
  );
  if (position >= 0) return ACCOUNT_COLORS[position % ACCOUNT_COLORS.length];

  let hash = 0;
  for (let index = 0; index < lowered.length; index += 1) {
    hash = (hash * 31 + lowered.charCodeAt(index)) % 100_003;
  }
  return ACCOUNT_COLORS[hash % ACCOUNT_COLORS.length];
}

/** The short name an account is known by in the legend: the local part. */
export function accountLabel(account: CalendarAccount): string {
  return account.display_name?.trim() || account.email;
}
