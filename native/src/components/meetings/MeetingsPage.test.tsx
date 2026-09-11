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
    list_speech_models: {
      models_dir: '/models',
      models: [
        {
          id: 'whisper-small',
          name: 'Small',
          filename: 'ggml-small.bin',
          path: '/models/ggml-small.bin',
          size_bytes: 487_601_967,
          installed: true,
          managed: true,
          multilingual: true,
          parameters_millions: 244,
          tier: 'balanced',
          blurb: 'The default.',
        },
      ],
      active_meeting_model: 'whisper-small',
      active_dictation_model: 'whisper-small',
      recommended_meeting_model: 'whisper-large-v3-turbo',
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
  test('names the model a recording will be transcribed with', async () => {
    mockBackend();
    render(<MeetingsPage />);
    expect(await screen.findByText(/transcribing with small/i)).toBeInTheDocument();
  });

  test('offers to install a model instead of only saying one is missing', async () => {
    // The failure this replaces: pressing Record produced "Install one under
    // Settings › Speech" and nothing else, for a section that did not exist.
    mockBackend({
      list_speech_models: {
        models_dir: '/models',
        models: [
          {
            id: 'whisper-large-v3-turbo',
            name: 'Large v3 Turbo',
            filename: 'ggml-large-v3-turbo.bin',
            path: '/models/ggml-large-v3-turbo.bin',
            size_bytes: 1_624_555_275,
            installed: false,
            managed: true,
            multilingual: true,
            parameters_millions: 809,
            tier: 'accurate',
            blurb: 'Best meeting model for most machines.',
          },
        ],
        active_meeting_model: null,
        active_dictation_model: null,
        recommended_meeting_model: 'whisper-large-v3-turbo',
      },
    });
    render(<MeetingsPage />);

    expect(await screen.findByText('Recording needs a speech model')).toBeInTheDocument();
    const install = screen.getByRole('button', { name: /install large v3 turbo/i });

    fireEvent.click(install);
    await waitFor(() => {
      expect(vi.mocked(invoke)).toHaveBeenCalledWith('download_speech_model', {
        id: 'whisper-large-v3-turbo',
      });
    });
  });

  test('links to Settings › Speech for a different model', async () => {
    const onOpenSpeechSettings = vi.fn();
    mockBackend();
    render(<MeetingsPage onOpenSpeechSettings={onOpenSpeechSettings} />);

    fireEvent.click(await screen.findByRole('button', { name: /change/i }));
    expect(onOpenSpeechSettings).toHaveBeenCalled();
  });
  test('plays a stored recording and seeks to the line that was clicked', async () => {
    mockBackend({
      get_meeting: {
        meeting: meeting({ audio_path: '/vault/meetings/meeting-1/audio/recording.wav' }),
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
          {
            sequence: 1,
            text: 'yes, go ahead',
            start_seconds: 65,
            end_seconds: 68,
            channel: 'system',
            no_speech_prob: 0.02,
            recorded_at: '2026-09-10T09:01:05Z',
          },
        ],
        summary: null,
        notes: '',
      },
    });
    render(<MeetingsPage />);

    fireEvent.click(await screen.findByText('Weekly sync'));

    const audio = (await screen.findByTestId('meeting-audio')) as HTMLAudioElement;
    expect(screen.getByRole('button', { name: /play recording/i })).toBeInTheDocument();

    // The timestamp on a line is the seek control — the transcript is how you
    // get to a moment, not a separate scrubbing exercise.
    fireEvent.click(screen.getByRole('button', { name: /play from 01:05/i }));
    await waitFor(() => expect(audio.currentTime).toBe(65));
  });

  test('shows transcript and report together on a wide window', async () => {
    mockBackend({
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
        summary: {
          markdown: '# Weekly sync\n\nWe agreed to ship.',
          status: 'completed',
          template_id: 'general',
          generated_at: '2026-09-10T10:00:00Z',
          model: 'llama3.2:latest',
          language: '',
          error: null,
        },
        notes: '',
      },
    });
    render(<MeetingsPage />);

    fireEvent.click(await screen.findByText('Weekly sync'));

    const both = await screen.findByRole('tab', { name: /both/i });
    expect(both).toHaveAttribute('aria-selected', 'true');
    expect(screen.getByText('shall we start')).toBeInTheDocument();
    expect(screen.getByText(/we agreed to ship\./i)).toBeInTheDocument();
  });

  test('offers no seek control for a meeting with no saved recording', async () => {
    mockBackend();
    render(<MeetingsPage />);

    fireEvent.click(await screen.findByText('Weekly sync'));
    await screen.findByText('shall we start');

    expect(screen.queryByTestId('meeting-audio')).not.toBeInTheDocument();
    expect(screen.queryByRole('button', { name: /play from/i })).not.toBeInTheDocument();
  });
});
