import { describe, test, expect, vi, beforeEach } from 'vitest';
import React from 'react';
import { render, screen, waitFor, fireEvent } from '@testing-library/react';
import { invoke } from '@tauri-apps/api/core';

import { MeetingsPage } from './MeetingsPage';
import type { MeetingListItem, MeetingRecordingStatus } from '@/types/meetings';

const idleStatus: MeetingRecordingStatus = {
  active: false,
  elapsed_seconds: 0,
  microphone_active: false,
  system_audio_active: false,
  microphone_heard: false,
  system_audio_heard: false,
  segments_queued: 0,
  segments_completed: 0,
  segments_dropped: 0,
};

const meeting = (overrides: Partial<MeetingListItem> = {}): MeetingListItem => ({
  id: 'meeting-1',
  title: 'Weekly sync',
  created_at: '2026-09-10T09:00:00Z',
  updated_at: '2026-09-10T09:45:00Z',
  state: 'completed',
  source: 'recorded',
  duration_seconds: 2_700,
  system_audio_captured: true,
  segment_count: 120,
  dropped_segments: 0,
  tags: [],
  has_summary: true,
  summary_status: 'completed',
  preview: 'we agreed to ship the migration',
  ...overrides,
});

/** Routes each command to a canned answer, so a test only states what differs. */
function mockBackend(overrides: Record<string, unknown> = {}) {
  const defaults: Record<string, unknown> = {
    get_meeting_recording_status: idleStatus,
    list_meetings: [meeting()],
    list_meeting_templates: [
      { id: 'general', name: 'General Meeting', description: '', sections: [], custom: false },
      { id: 'standup', name: 'Daily Standup', description: '', sections: [], custom: false },
    ],
    get_meeting: {
      meeting: meeting(),
      segments: [
        {
          sequence: 0,
          text: 'shall we start',
          start_seconds: 0,
          end_seconds: 2,
          channel: 'microphone',
          no_speech_prob: 0.01,
          recorded_at: '2026-09-10T09:00:00Z',
        },
      ],
      summary: null,
      notes: '',
    },
  };
  const answers = { ...defaults, ...overrides };
  vi.mocked(invoke).mockImplementation(async (command: string) => {
    if (command in answers) {
      const answer = answers[command];
      if (answer instanceof Error) throw answer;
      return answer;
    }
    return undefined;
  });
}

describe('MeetingsPage', () => {
  beforeEach(() => {
    vi.mocked(invoke).mockReset();
  });

  test('lists meetings from the vault', async () => {
    mockBackend();
    render(<MeetingsPage />);
    expect(await screen.findByText('Weekly sync')).toBeInTheDocument();
    expect(screen.getByText(/we agreed to ship the migration/)).toBeInTheDocument();
  });

  test('offers to record when nothing is recording', async () => {
    mockBackend();
    render(<MeetingsPage />);
    expect(await screen.findByRole('button', { name: /start recording/i })).toBeInTheDocument();
    expect(screen.queryByRole('button', { name: /^stop$/i })).not.toBeInTheDocument();
  });

  test('shows the live controls and both level meters while recording', async () => {
    mockBackend({
      get_meeting_recording_status: {
        ...idleStatus,
        active: true,
        meeting_id: 'meeting-1',
        title: 'Weekly sync',
        state: 'recording',
        elapsed_seconds: 65,
        microphone_active: true,
        microphone_heard: true,
        system_audio_active: true,
      },
    });
    render(<MeetingsPage />);

    expect(await screen.findByRole('button', { name: /^stop$/i })).toBeInTheDocument();
    expect(screen.getByRole('button', { name: /^pause$/i })).toBeInTheDocument();
    expect(screen.getByRole('meter', { name: /microphone level/i })).toBeInTheDocument();
    expect(screen.getByRole('meter', { name: /system audio level/i })).toBeInTheDocument();
    expect(screen.getByText('01:05')).toBeInTheDocument();
  });

  test('warns when system audio could not be captured', async () => {
    mockBackend({
      get_meeting_recording_status: {
        ...idleStatus,
        active: true,
        meeting_id: 'meeting-1',
        state: 'recording',
        microphone_active: true,
        warning: 'System audio could not be captured.',
      },
    });
    render(<MeetingsPage />);
    expect(await screen.findByText(/System audio could not be captured/)).toBeInTheDocument();
  });

  test('opening a meeting shows its transcript with the speaker label', async () => {
    mockBackend();
    render(<MeetingsPage />);

    fireEvent.click(await screen.findByText('Weekly sync'));

    expect(await screen.findByText('shall we start')).toBeInTheDocument();
    expect(screen.getByText('You')).toBeInTheDocument();
  });

  test('a meeting with no report offers to generate one', async () => {
    mockBackend();
    render(<MeetingsPage />);
    fireEvent.click(await screen.findByText('Weekly sync'));

    fireEvent.click(await screen.findByRole('tab', { name: /report/i }));

    expect(await screen.findByRole('button', { name: /^generate$/i })).toBeInTheDocument();
    expect(screen.getByLabelText('Summary template')).toBeInTheDocument();
  });

  test('a failed start is reported rather than swallowed', async () => {
    mockBackend({
      start_meeting: Object.assign(new Error('no model'), {
        code: 'MEETING_NO_SPEECH_MODEL',
        message: 'No speech model is installed.',
      }),
    });
    render(<MeetingsPage />);

    fireEvent.click(await screen.findByRole('button', { name: /start recording/i }));

    expect(await screen.findByText('No speech model is installed.')).toBeInTheDocument();
  });

  test('filtering narrows the list without touching the backend again', async () => {
    mockBackend({
      list_meetings: [meeting(), meeting({ id: 'meeting-2', title: 'Pricing review', preview: '' })],
    });
    render(<MeetingsPage />);
    await screen.findByText('Weekly sync');

    fireEvent.change(screen.getByLabelText('Search meetings'), {
      target: { value: 'pricing' },
    });

    await waitFor(() => expect(screen.queryByText('Weekly sync')).not.toBeInTheDocument());
    expect(screen.getByText('Pricing review')).toBeInTheDocument();
  });

  test('an empty vault says so instead of showing an empty list', async () => {
    mockBackend({ list_meetings: [] });
    render(<MeetingsPage />);
    expect(await screen.findByText('No meetings yet')).toBeInTheDocument();
  });

  test('a microphone-only meeting is marked as such in the list', async () => {
    mockBackend({ list_meetings: [meeting({ system_audio_captured: false })] });
    render(<MeetingsPage />);
    expect(await screen.findByText('Mic only')).toBeInTheDocument();
  });

  test('dropped segments are surfaced rather than hidden', async () => {
    mockBackend({ list_meetings: [meeting({ dropped_segments: 3 })] });
    render(<MeetingsPage />);
    expect(await screen.findByText('3 lost')).toBeInTheDocument();
  });
});
