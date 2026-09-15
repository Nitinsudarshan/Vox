import { describe, test, expect, vi, beforeEach, afterEach } from 'vitest';
import React from 'react';
import { render, screen, waitFor, fireEvent } from '@testing-library/react';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';

import { ReminderOverlay } from './ReminderOverlay';
import { MEETING_REMINDER_EVENT, type MeetingReminder } from '@/types/meetings';

/** The handler the component registered, so a test can fire a reminder at it. */
let deliver: ((event: { payload: MeetingReminder }) => void) | null = null;

const reminder = (overrides: Partial<MeetingReminder> = {}): MeetingReminder => ({
  key: 'me@work.com|evt-1|5',
  event_id: 'evt-1',
  account_email: 'me@work.com',
  title: 'Weekly Sync',
  start: new Date(Date.now() + 3 * 60_000).toISOString(),
  conference_url: 'https://meet.google.com/abc-defg-hij',
  location: null,
  meeting_id: null,
  minutes_until: 3,
  guest_count: 2,
  ...overrides,
});

describe('ReminderOverlay', () => {
  beforeEach(() => {
    deliver = null;
    vi.mocked(invoke).mockReset();
    vi.mocked(invoke).mockResolvedValue(undefined);
    vi.mocked(listen).mockImplementation(((name: string, handler: unknown) => {
      if (name === MEETING_REMINDER_EVENT) {
        deliver = handler as (event: { payload: MeetingReminder }) => void;
      }
      return Promise.resolve(() => undefined);
    }) as never);
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  test('says nothing until a meeting is actually due', () => {
    render(<ReminderOverlay />);
    expect(screen.queryByRole('region', { name: /meeting reminders/i })).not.toBeInTheDocument();
  });

  test('a due meeting is announced with how long is left and who is in it', async () => {
    render(<ReminderOverlay />);
    await waitFor(() => expect(deliver).not.toBeNull());
    deliver?.({ payload: reminder() });

    expect(await screen.findByText('Weekly Sync')).toBeInTheDocument();
    expect(screen.getByText(/starts in 3 minutes/i)).toBeInTheDocument();
    expect(screen.getByText(/2 guests/i)).toBeInTheDocument();
  });

  test('a meeting that has already begun says so rather than counting down', async () => {
    render(<ReminderOverlay />);
    await waitFor(() => expect(deliver).not.toBeNull());
    deliver?.({
      payload: reminder({
        key: 'me@work.com|evt-1|0',
        start: new Date(Date.now() - 2 * 60_000).toISOString(),
        minutes_until: -2,
      }),
    });

    expect(await screen.findByText(/started 2 minutes ago/i)).toBeInTheDocument();
  });

  test('joining opens the link through the backend, which validates it', async () => {
    render(<ReminderOverlay />);
    await waitFor(() => expect(deliver).not.toBeNull());
    deliver?.({ payload: reminder() });

    fireEvent.click(await screen.findByRole('button', { name: /^join$/i }));
    await waitFor(() =>
      expect(vi.mocked(invoke)).toHaveBeenCalledWith('open_calendar_link', {
        url: 'https://meet.google.com/abc-defg-hij',
      }),
    );
    // Acting on a reminder is done with it.
    await waitFor(() => expect(screen.queryByText('Weekly Sync')).not.toBeInTheDocument());
  });

  test('join and record opens the call and starts recording it', async () => {
    render(<ReminderOverlay />);
    await waitFor(() => expect(deliver).not.toBeNull());
    deliver?.({ payload: reminder() });

    fireEvent.click(await screen.findByRole('button', { name: /join and record/i }));
    await waitFor(() =>
      expect(vi.mocked(invoke)).toHaveBeenCalledWith('open_calendar_link', {
        url: 'https://meet.google.com/abc-defg-hij',
      }),
    );
    await waitFor(() =>
      expect(vi.mocked(invoke)).toHaveBeenCalledWith(
        'start_meeting',
        expect.objectContaining({ title: 'Weekly Sync' }),
      ),
    );
    await waitFor(() => expect(screen.queryByText('Weekly Sync')).not.toBeInTheDocument());
  });

  test('the window hides itself once the last reminder is gone', async () => {
    // A transparent always-on-top window left up with nothing in it still
    // swallows clicks meant for whatever is underneath.
    render(<ReminderOverlay />);
    await waitFor(() => expect(deliver).not.toBeNull());
    deliver?.({ payload: reminder() });
    await screen.findByText('Weekly Sync');

    fireEvent.click(screen.getByRole('button', { name: /dismiss the reminder/i }));
    await waitFor(() =>
      expect(vi.mocked(invoke)).toHaveBeenCalledWith('dismiss_meeting_reminders'),
    );
  });

  test('the window is sized from what was laid out rather than from a guess', async () => {
    render(<ReminderOverlay />);
    await waitFor(() => expect(deliver).not.toBeNull());
    deliver?.({ payload: reminder() });

    await waitFor(() =>
      expect(vi.mocked(invoke)).toHaveBeenCalledWith(
        'resize_meeting_reminders',
        expect.objectContaining({ height: expect.any(Number) }),
      ),
    );
  });

  test('a meeting with no video link offers recording and nothing to join', async () => {
    render(<ReminderOverlay />);
    await waitFor(() => expect(deliver).not.toBeNull());
    deliver?.({ payload: reminder({ conference_url: null, location: 'Room 4' }) });

    expect(await screen.findByText('Room 4')).toBeInTheDocument();
    expect(screen.queryByRole('button', { name: /^join$/i })).not.toBeInTheDocument();
    expect(screen.queryByRole('button', { name: /join and record/i })).not.toBeInTheDocument();
    expect(screen.getByRole('button', { name: /^record$/i })).toBeInTheDocument();
  });

  test('dismissing one is final for that reminder', async () => {
    render(<ReminderOverlay />);
    await waitFor(() => expect(deliver).not.toBeNull());
    deliver?.({ payload: reminder() });

    fireEvent.click(await screen.findByRole('button', { name: /dismiss the reminder/i }));
    await waitFor(() => expect(screen.queryByText('Weekly Sync')).not.toBeInTheDocument());
  });

  test('the same meeting announced again replaces its card rather than stacking', async () => {
    // A meeting legitimately arrives twice — at ten minutes and then at one.
    render(<ReminderOverlay />);
    await waitFor(() => expect(deliver).not.toBeNull());

    deliver?.({ payload: reminder({ key: 'me@work.com|evt-1|10', minutes_until: 9 }) });
    deliver?.({
      payload: reminder({
        key: 'me@work.com|evt-1|1',
        start: new Date(Date.now() + 60_000).toISOString(),
        minutes_until: 1,
      }),
    });

    expect(await screen.findAllByText('Weekly Sync')).toHaveLength(1);
    expect(screen.getByText(/starts in 1 minute$/i)).toBeInTheDocument();
  });

  test('two different meetings both get a card', async () => {
    render(<ReminderOverlay />);
    await waitFor(() => expect(deliver).not.toBeNull());

    deliver?.({ payload: reminder() });
    deliver?.({
      payload: reminder({
        key: 'me@gmail.com|evt-2|5',
        event_id: 'evt-2',
        account_email: 'me@gmail.com',
        title: 'Dentist',
      }),
    });

    expect(await screen.findByText('Weekly Sync')).toBeInTheDocument();
    expect(screen.getByText('Dentist')).toBeInTheDocument();
  });
});
