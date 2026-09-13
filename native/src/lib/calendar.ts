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
