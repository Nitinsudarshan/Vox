import { describe, test, expect } from 'vitest';

import {
  ACCOUNT_COLORS,
  accountColor,
  agendaDayLabel,
  attendeeSummary,
  canJoinEvent,
  eventDuration,
  eventTime,
  isDeclined,
  isPast,
  isToday,
  joinUnavailableReason,
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

describe('canJoinEvent', () => {
  /** An event on `day`, starting at `hour` local. */
  const at = (day: Date, hour: number, minute = 0): CalendarEvent => {
    const start = new Date(day.getFullYear(), day.getMonth(), day.getDate(), hour, minute);
    const end = new Date(start.getTime() + 30 * 60_000);
    return event({
      start: start.toISOString(),
      end: end.toISOString(),
      conference_url: 'https://meet.google.com/abc-defg-hij',
    });
  };

  const today = new Date(2026, 8, 14, 16, 0);

  test('a meeting earlier today is still joinable, because calls resume', () => {
    // The case that decided the window: "let us join back on the same link".
    expect(canJoinEvent(at(today, 10), today)).toBe(true);
  });

  test('every meeting on today is live, not just the one about to start', () => {
    expect(canJoinEvent(at(today, 8), today)).toBe(true);
    expect(canJoinEvent(at(today, 23, 30), today)).toBe(true);
  });

  test('a meeting later today is joinable', () => {
    expect(canJoinEvent(at(today, 18), today)).toBe(true);
  });

  test("tomorrow's meeting is not something to open by accident", () => {
    const tomorrow = new Date(2026, 8, 15, 10, 0);
    expect(canJoinEvent(at(tomorrow, 10), today)).toBe(false);
  });

  test('a meeting just after midnight is reachable from the evening before', () => {
    const tonight = new Date(2026, 8, 14, 23, 50);
    const justAfterMidnight = at(new Date(2026, 8, 15), 0, 15);
    expect(canJoinEvent(justAfterMidnight, tonight)).toBe(true);
  });

  test("yesterday's link opens a room nobody is in", () => {
    const yesterday = new Date(2026, 8, 13, 10, 0);
    expect(canJoinEvent(at(yesterday, 10), today)).toBe(false);
  });

  test('an invitation with no video link is not joinable at any time', () => {
    const withoutLink = event({ conference_url: null });
    expect(canJoinEvent(withoutLink, today)).toBe(false);
    expect(joinUnavailableReason(withoutLink, today)).toMatch(/no video link/i);
  });

  test('the reason names what would make the link live', () => {
    const tomorrow = new Date(2026, 8, 15, 10, 0);
    expect(joinUnavailableReason(at(tomorrow, 10), today)).toMatch(/on the day/i);
    const yesterday = new Date(2026, 8, 13, 10, 0);
    expect(joinUnavailableReason(at(yesterday, 10), today)).toMatch(/earlier day/i);
  });
});

describe('isDeclined', () => {
  test('a meeting the user said no to is marked as such', () => {
    expect(isDeclined(event({ attendance: 'declined' }))).toBe(true);
    expect(isDeclined(event({ attendance: 'accepted' }))).toBe(false);
    expect(isDeclined(event({ attendance: 'needs_action' }))).toBe(false);
  });
});

describe('accountColor', () => {
  const connected = ['work@example.com', 'life@gmail.com'];

  test('two accounts are two colours', () => {
    expect(accountColor('work@example.com', connected)).not.toEqual(
      accountColor('life@gmail.com', connected),
    );
  });

  test('a colour follows the account rather than the row it is drawn in', () => {
    expect(accountColor('WORK@example.com', connected)).toEqual(ACCOUNT_COLORS[0]);
    expect(accountColor(' life@gmail.com ', connected)).toEqual(ACCOUNT_COLORS[1]);
  });

  test('an event from a disconnected account still gets a stable colour', () => {
    const first = accountColor('gone@example.com', connected);
    expect(accountColor('gone@example.com', connected)).toEqual(first);
    expect(ACCOUNT_COLORS).toContainEqual(first);
  });

  test('more accounts than colours wraps rather than running off the end', () => {
    const many = Array.from({ length: 8 }, (_, index) => `a${index}@example.com`);
    expect(accountColor('a6@example.com', many)).toEqual(ACCOUNT_COLORS[0]);
  });
});
