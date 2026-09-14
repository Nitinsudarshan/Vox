import { describe, test, expect } from 'vitest';

import {
  agendaDayLabel,
  attendeeSummary,
  eventDuration,
  eventTime,
  isPast,
  isToday,
} from './calendar';
import type { CalendarEvent } from '@/types/calendar';

const event = (overrides: Partial<CalendarEvent> = {}): CalendarEvent => ({
  id: 'evt-1',
  account_email: 'me@work.com',
  calendar_id: 'primary',
  title: 'Weekly sync',
  start: '2026-09-14T10:00:00Z',
  end: '2026-09-14T11:00:00Z',
  all_day: false,
  attendees: [],
  attendance: 'accepted',
  ...overrides,
});

describe('eventTime', () => {
  test('an all-day event says so rather than showing midnight', () => {
    expect(eventTime(event({ all_day: true }))).toBe('All day');
  });

  test('an unparseable start renders as nothing rather than as Invalid Date', () => {
    expect(eventTime(event({ start: 'nonsense' }))).toBe('');
  });
});

describe('eventDuration', () => {
  test('reads in minutes under an hour and in hours above', () => {
    expect(eventDuration(event({ end: '2026-09-14T10:30:00Z' }))).toBe('30 min');
    expect(eventDuration(event({ end: '2026-09-14T12:00:00Z' }))).toBe('2 h');
    expect(eventDuration(event({ end: '2026-09-14T11:30:00Z' }))).toBe('1 h 30 min');
  });

  test('an all-day event has no duration worth showing', () => {
    expect(eventDuration(event({ all_day: true }))).toBe('');
  });

  test('an end before its start is not rendered as negative time', () => {
    expect(eventDuration(event({ end: '2026-09-14T09:00:00Z' }))).toBe('');
  });
});

describe('attendeeSummary', () => {
  const guest = (email: string, displayName?: string, isSelf = false) => ({
    email,
    display_name: displayName ?? null,
    response: 'accepted' as const,
    is_self: isSelf,
    organizer: false,
  });

  test('leaves the account holder out', () => {
    // A guest list that starts with your own name tells you nothing.
    const withSelf = event({
      attendees: [guest('me@work.com', 'Nitin', true), guest('payal@work.com', 'Payal')],
    });
    expect(attendeeSummary(withSelf)).toBe('Payal');
  });

  test('falls back to the address when Google sent no display name', () => {
    expect(attendeeSummary(event({ attendees: [guest('saraswati@navgurukul.org')] }))).toBe(
      'saraswati',
    );
  });

  test('counts the rest rather than listing everyone', () => {
    const many = event({
      attendees: [
        guest('a@x.com', 'Aisha'),
        guest('b@x.com', 'Bilal'),
        guest('c@x.com', 'Chandni'),
        guest('d@x.com', 'Dev'),
        guest('e@x.com', 'Esha'),
      ],
    });
    expect(attendeeSummary(many)).toBe('Aisha, Bilal, Chandni +2');
  });

  test('an event with nobody else on it says nothing', () => {
    expect(attendeeSummary(event())).toBe('');
    expect(attendeeSummary(event({ attendees: [guest('me@work.com', 'Nitin', true)] }))).toBe('');
  });
});

describe('isPast', () => {
  test('a finished event is past and a running one is not', () => {
    const now = new Date('2026-09-14T10:30:00Z');
    expect(isPast(event(), now)).toBe(false);
    expect(isPast(event({ end: '2026-09-14T10:15:00Z' }), now)).toBe(true);
  });

  test('an event with an unreadable end is never hidden as past', () => {
    // "Coming up" losing a real meeting is worse than it carrying a stale one.
    expect(isPast(event({ end: 'nonsense' }), new Date('2030-01-01T00:00:00Z'))).toBe(false);
  });
});

describe('isToday', () => {
  test('compares calendar days in the viewer timezone', () => {
    const now = new Date(2026, 8, 14, 23, 30);
    expect(isToday('2026-09-14', now)).toBe(true);
    expect(isToday('2026-09-15', now)).toBe(false);
  });
});

describe('agendaDayLabel', () => {
  const now = new Date(2026, 8, 14, 12, 0);

  test('names the days around today rather than dating them', () => {
    expect(agendaDayLabel('2026-09-14', now)).toBe('Today');
    expect(agendaDayLabel('2026-09-15', now)).toBe('Tomorrow');
    expect(agendaDayLabel('2026-09-13', now)).toBe('Yesterday');
  });

  test('dates anything further out', () => {
    expect(agendaDayLabel('2026-09-20', now)).toMatch(/Sep/);
  });

  test('an unparseable date is passed through rather than rendered as Invalid Date', () => {
    expect(agendaDayLabel('not-a-date', now)).toBe('not-a-date');
  });
});
