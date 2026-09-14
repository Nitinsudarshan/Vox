import { describe, test, expect, vi, beforeEach, afterEach } from 'vitest';
import React from 'react';
import { render, screen, waitFor, fireEvent, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { invoke } from '@tauri-apps/api/core';

import { MeetingsPage } from './MeetingsPage';
import { MeetingSettingsView } from '../settings/MeetingSettingsView';
import type { MeetingListItem, MeetingRecordingStatus } from '@/types/meetings';
import type { CalendarAccount, CalendarEvent } from '@/types/calendar';

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

/** A connected Google account, as `list_calendar_accounts` returns one. */
const account = (email: string, overrides: Partial<CalendarAccount> = {}): CalendarAccount => ({
  email,
  display_name: null,
  enabled: true,
  calendar_ids: [],
  last_synced_at: null,
  last_error: null,
  ...overrides,
});

/** An event on the fixed test clock's own day, at `hour` local. */
const calendarEvent = (
  hour: number,
  overrides: Partial<CalendarEvent> = {},
): CalendarEvent => {
  const start = new Date(2026, 8, 14, hour, 0);
  return {
    id: `evt-${hour}`,
    account_email: 'me@work.com',
    calendar_id: 'primary',
    title: 'Scrum Call',
    start: start.toISOString(),
    end: new Date(start.getTime() + 30 * 60_000).toISOString(),
    all_day: false,
    conference_url: 'https://meet.google.com/abc-defg-hij',
    attendees: [],
    attendance: 'accepted',
    ...overrides,
  };
};

/** `YYYY-MM-DD` for an event, the way the backend groups days. */
const dayOf = (event: CalendarEvent): string => {
  const start = new Date(event.start);
  return `${start.getFullYear()}-${String(start.getMonth() + 1).padStart(2, '0')}-${String(
    start.getDate(),
  ).padStart(2, '0')}`;
};

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
      speakers: [],
    },
    get_meeting_devices: { microphone: null, system_audio: null },
    get_audio_devices: [
      { name: 'Yeti Stereo Microphone', is_default: true },
      { name: 'Headset (WH-1000XM4 Hands-Free AG Audio)', is_default: false },
    ],
    get_audio_output_devices: [{ name: 'Speakers (Realtek)', is_default: true }],
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

/**
 * A meeting with a recording on disk.
 *
 * Several actions genuinely need one — transcribing again has nothing to read
 * without it, and a voice sample has nothing to play — and the menu disables
 * them when it is missing. A fixture without `audio_path` therefore tests the
 * disabled path, which is not what these tests are about.
 */
function recordedMeetingDetail(overrides: Record<string, unknown> = {}) {
  return {
    meeting: meeting({ audio_path: '/vault/meetings/meeting-1/audio.wav' }),
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
    speakers: [],
    ...overrides,
  };
}

/**
 * Opens the overflow menu and chooses one item.
 *
 * `userEvent` rather than `fireEvent` because Radix menus are driven by
 * pointer events rather than by `click`, and a bare `fireEvent.click` leaves
 * the menu shut — which reads in a failure as a missing feature rather than as
 * a missing event.
 */
async function chooseFromMenu(name: RegExp) {
  const user = userEvent.setup();
  await user.click(await screen.findByRole('button', { name: /more actions/i }));
  await user.click(await screen.findByRole('menuitem', { name }));
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

  test('offers to start a meeting when none is running', async () => {
    mockBackend();
    render(<MeetingsPage />);
    expect(await screen.findByRole('button', { name: /start meeting/i })).toBeInTheDocument();
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

    fireEvent.click(await screen.findByRole('button', { name: /start meeting/i }));

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
  test('offers a microphone and an output device in meeting settings', async () => {
    mockBackend();
    render(<MeetingSettingsView />);

    const mic = (await screen.findByLabelText('Microphone')) as HTMLSelectElement;
    const system = screen.getByLabelText('System audio from') as HTMLSelectElement;

    // The first option is not a device: it means "resolve this the way every
    // other surface does", which is what the app keeps doing until asked not to.
    expect(mic.value).toBe('');
    expect(within(mic).getByRole('option', { name: /system default/i })).toBeInTheDocument();
    expect(
      within(mic).getByRole('option', { name: /yeti stereo microphone \(default\)/i }),
    ).toBeInTheDocument();
    expect(within(system).getByRole('option', { name: /speakers \(realtek\)/i })).toBeInTheDocument();
  });

  test('remembers the chosen device and records with it', async () => {
    mockBackend();
    const { unmount } = render(<MeetingSettingsView />);

    const mic = await screen.findByLabelText('Microphone');
    fireEvent.change(mic, { target: { value: 'Yeti Stereo Microphone' } });

    await waitFor(() => {
      expect(vi.mocked(invoke)).toHaveBeenCalledWith('set_meeting_devices', {
        devices: { microphone: 'Yeti Stereo Microphone', system_audio: null },
      });
    });
    unmount();

    // Now mock the saved devices returned to MeetingsPage
    mockBackend({
      get_meeting_devices: { microphone: 'Yeti Stereo Microphone', system_audio: null },
    });
    render(<MeetingsPage />);
    const startBtn = await screen.findByRole('button', { name: /start meeting/i });
    fireEvent.click(startBtn);
    await waitFor(() => {
      expect(vi.mocked(invoke)).toHaveBeenCalledWith('start_meeting', {
        title: undefined,
        captureSystemAudio: undefined,
        devices: { microphone: 'Yeti Stereo Microphone', system_audio: null },
      });
    });
  });

  test('says a saved device is gone rather than quietly reading as the default in meeting settings', async () => {
    mockBackend({
      get_meeting_devices: { microphone: 'Unplugged USB Mic', system_audio: null },
    });
    render(<MeetingSettingsView />);

    const mic = (await screen.findByLabelText('Microphone')) as HTMLSelectElement;
    await waitFor(() => expect(mic.value).toBe('Unplugged USB Mic'));
    expect(screen.getByText(/not connected — recording will fall back/i)).toBeInTheDocument();
  });

  test('names the devices a running recording opened', async () => {
    mockBackend({
      get_meeting_recording_status: {
        ...idleStatus,
        active: true,
        meeting_id: 'meeting-1',
        state: 'recording',
        microphone_active: true,
        microphone_heard: false,
        system_audio_active: true,
        system_audio_heard: true,
        devices: {
          microphone: 'Yeti Stereo Microphone',
          system_audio: 'Speakers (Realtek)',
        },
      },
    });
    render(<MeetingsPage />);

    // "Nothing heard yet" is how a wrong device is noticed; the name is what
    // makes it actionable without leaving the app.
    expect(await screen.findByText(/· Yeti Stereo Microphone/)).toBeInTheDocument();
    expect(screen.getByText(/· Speakers \(Realtek\)/)).toBeInTheDocument();
    expect(screen.getByText('nothing heard yet')).toBeInTheDocument();
  });
  test('a report that has no provider offers the settings that fix it', async () => {
    // The failure this replaces: three attempts, a minute of backoff, and
    // "error sending request for url (http://localhost:11434/api/generate)"
    // stored as the meeting's report status.
    const onOpenProviderSettings = vi.fn();
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
          markdown: null,
          status: 'failed',
          template_id: 'general',
          generated_at: null,
          model: null,
          language: '',
          error:
            'Ollama is not installed on this machine, so there is no local model to write the report with. Install it from ollama.com, or choose a different provider under Settings › AI Models & STT.',
        },
        notes: '',
      },
    });
    render(<MeetingsPage onOpenProviderSettings={onOpenProviderSettings} />);

    fireEvent.click(await screen.findByText('Weekly sync'));
    expect(await screen.findByText(/Ollama is not installed/)).toBeInTheDocument();

    fireEvent.click(screen.getByRole('button', { name: /open settings/i }));
    expect(onOpenProviderSettings).toHaveBeenCalled();
  });

  test('a model failure is shown as a failure, with no settings to open', async () => {
    const onOpenProviderSettings = vi.fn();
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
          markdown: null,
          status: 'failed',
          template_id: 'general',
          generated_at: null,
          model: 'llama3.2:latest',
          language: '',
          error: 'the model returned nothing',
        },
        notes: '',
      },
    });
    render(<MeetingsPage onOpenProviderSettings={onOpenProviderSettings} />);

    fireEvent.click(await screen.findByText('Weekly sync'));
    expect(await screen.findByText(/the model returned nothing/)).toBeInTheDocument();
    expect(screen.queryByRole('button', { name: /open settings/i })).not.toBeInTheDocument();
  });

  test('the index is one column: opening a meeting replaces the list', async () => {
    // The whole point of the rework. The version this replaces rendered the
    // list and the meeting side by side, so a transcript was read through a
    // 320-pixel column with six list rows beside it.
    mockBackend();
    render(<MeetingsPage />);

    fireEvent.click(await screen.findByText('Weekly sync'));
    expect(await screen.findByRole('button', { name: /all meetings/i })).toBeInTheDocument();
    expect(
      screen.queryByRole('textbox', { name: /search meetings/i }),
    ).not.toBeInTheDocument();
  });

  test('going back returns to the list', async () => {
    mockBackend();
    render(<MeetingsPage />);

    fireEvent.click(await screen.findByText('Weekly sync'));
    fireEvent.click(await screen.findByRole('button', { name: /all meetings/i }));
    expect(
      await screen.findByRole('textbox', { name: /search meetings/i }),
    ).toBeInTheDocument();
  });

  test('meetings are grouped under the day they happened on', async () => {
    const today = new Date();
    mockBackend({
      list_meetings: [meeting({ id: 'm-today', title: 'Today call', created_at: today.toISOString() })],
    });
    render(<MeetingsPage />);
    expect(await screen.findByText('Today')).toBeInTheDocument();
  });

  test('secondary actions live in the overflow menu rather than a row of icons', async () => {
    mockBackend({ get_meeting: recordedMeetingDetail() });
    render(<MeetingsPage />);
    fireEvent.click(await screen.findByText('Weekly sync'));

    // Nothing destructive is one stray click away from a reader.
    expect(screen.queryByRole('menuitem', { name: /delete/i })).not.toBeInTheDocument();

    const user = userEvent.setup();
    await user.click(await screen.findByRole('button', { name: /more actions/i }));
    expect(await screen.findByRole('menuitem', { name: /transcribe again/i })).toBeInTheDocument();
    expect(screen.getByRole('menuitem', { name: /find speakers/i })).toBeInTheDocument();
    expect(screen.getByRole('menuitem', { name: /delete/i })).toBeInTheDocument();
  });

  test('transcribing again asks what to change instead of repeating the settings that failed', async () => {
    mockBackend({ get_meeting: recordedMeetingDetail() });
    render(<MeetingsPage />);
    fireEvent.click(await screen.findByText('Weekly sync'));
    await chooseFromMenu(/transcribe again/i);

    // Scoped to the dialog: "speech model" also names a control on the page
    // behind it, and an unscoped query would pass on the wrong one.
    const dialog = within(await screen.findByRole('dialog'));
    expect(dialog.getByLabelText(/transcription language/i)).toBeInTheDocument();
    expect(dialog.getByLabelText(/speech model/i)).toBeInTheDocument();
    // Nothing has run yet: opening the dialog is not the action.
    expect(vi.mocked(invoke)).not.toHaveBeenCalledWith('retranscribe_meeting', expect.anything());
  });

  test('a re-transcription carries the choices the dialog collected', async () => {
    mockBackend({ get_meeting: recordedMeetingDetail() });
    render(<MeetingsPage />);
    fireEvent.click(await screen.findByText('Weekly sync'));
    await chooseFromMenu(/transcribe again/i);

    fireEvent.change(await screen.findByLabelText(/transcription language/i), {
      target: { value: 'hi' },
    });
    fireEvent.click(screen.getByRole('button', { name: /^transcribe again$/i }));

    await waitFor(() =>
      expect(vi.mocked(invoke)).toHaveBeenCalledWith('retranscribe_meeting', {
        meetingId: 'meeting-1',
        overrides: { language: 'hi', modelId: '', preset: 'quality' },
      }),
    );
  });

  test('a transcript in another script offers the English view without being asked twice', async () => {
    mockBackend({
      get_meeting: {
        meeting: meeting(),
        segments: [
          {
            sequence: 0,
            text: 'क्या आप सुन सकते हैं',
            romanized_text: 'kya aap sun sakate hain',
            translated_text: 'can you hear me',
            start_seconds: 0,
            end_seconds: 2,
            channel: 'microphone',
            no_speech_prob: 0.01,
            recorded_at: '2026-09-10T09:00:00Z',
          },
        ],
        summary: null,
        notes: '',
        speakers: [],
      },
    });
    render(<MeetingsPage />);
    fireEvent.click(await screen.findByText('Weekly sync'));

    // It opens on English, because that is the view the reader can read.
    expect(await screen.findByText('can you hear me')).toBeInTheDocument();

    fireEvent.click(screen.getByRole('button', { name: /^romanized$/i }));
    expect(await screen.findByText('kya aap sun sakate hain')).toBeInTheDocument();

    fireEvent.click(screen.getByRole('button', { name: /^original$/i }));
    expect(await screen.findByText('क्या आप सुन सकते हैं')).toBeInTheDocument();
  });

  test('an all-English transcript is not cluttered with script views it has no use for', async () => {
    mockBackend();
    render(<MeetingsPage />);
    fireEvent.click(await screen.findByText('Weekly sync'));

    expect(await screen.findByText('shall we start')).toBeInTheDocument();
    expect(screen.queryByRole('button', { name: /^romanized$/i })).not.toBeInTheDocument();
  });

  test('detected speakers name the lines they said', async () => {
    mockBackend({
      get_meeting: {
        meeting: meeting(),
        segments: [
          {
            sequence: 0,
            text: 'shall we start',
            start_seconds: 0,
            end_seconds: 2,
            channel: 'system',
            no_speech_prob: 0.01,
            recorded_at: '2026-09-10T09:00:00Z',
            speaker_id: 'speaker-1',
          },
        ],
        summary: null,
        notes: '',
        speakers: [
          {
            id: 'speaker-1',
            label: 'Payal',
            named_by_user: true,
            channel: 'system',
            sample_start_seconds: 4,
            sample_end_seconds: 12,
            segment_count: 1,
            speaking_seconds: 2,
          },
        ],
      },
    });
    render(<MeetingsPage />);
    fireEvent.click(await screen.findByText('Weekly sync'));

    // Named twice on purpose: once as the line's speaker, and once in the
    // banner as somebody who was in the meeting.
    expect(await screen.findAllByText('Payal')).toHaveLength(2);
    expect(screen.queryByText('Others')).not.toBeInTheDocument();
  });

  test('a speaker can be played back and renamed', async () => {
    mockBackend({
      get_meeting: {
        meeting: meeting({ audio_path: '/vault/meetings/meeting-1/audio.wav' }),
        segments: [
          {
            sequence: 0,
            text: 'shall we start',
            start_seconds: 0,
            end_seconds: 2,
            channel: 'system',
            no_speech_prob: 0.01,
            recorded_at: '2026-09-10T09:00:00Z',
            speaker_id: 'speaker-1',
          },
        ],
        summary: null,
        notes: '',
        speakers: [
          {
            id: 'speaker-1',
            label: 'Speaker 1',
            named_by_user: false,
            channel: 'system',
            sample_start_seconds: 4,
            sample_end_seconds: 12,
            segment_count: 1,
            speaking_seconds: 2,
          },
        ],
      },
    });
    render(<MeetingsPage />);
    fireEvent.click(await screen.findByText('Weekly sync'));
    fireEvent.click(await screen.findByRole('button', { name: /^speakers$/i }));

    // A sample to hear is what makes a wrong grouping cost one rename.
    expect(await screen.findByRole('button', { name: /play speaker 1 at/i })).toBeInTheDocument();

    fireEvent.click(screen.getByTitle('Rename this speaker'));
    const field = await screen.findByLabelText(/name for speaker 1/i);
    fireEvent.change(field, { target: { value: 'Soni' } });
    fireEvent.keyDown(field, { key: 'Enter' });

    await waitFor(() =>
      expect(vi.mocked(invoke)).toHaveBeenCalledWith('rename_meeting_speaker', {
        meetingId: 'meeting-1',
        speakerId: 'speaker-1',
        label: 'Soni',
      }),
    );
  });

  test('a meeting being read is what the banner is about, not starting another', async () => {
    // Offering to start a second meeting from inside the first is an
    // invitation to a mistake.
    mockBackend({ get_meeting: recordedMeetingDetail() });
    render(<MeetingsPage />);
    expect(await screen.findByRole('button', { name: /start meeting/i })).toBeInTheDocument();

    fireEvent.click(await screen.findByText('Weekly sync'));
    await screen.findByRole('button', { name: /all meetings/i });
    expect(screen.queryByRole('button', { name: /start meeting/i })).not.toBeInTheDocument();
    // The banner is the meeting: its name, its recording, its actions.
    expect(screen.getByRole('heading', { name: 'Weekly sync' })).toBeInTheDocument();
    expect(screen.getByRole('button', { name: /play recording/i })).toBeInTheDocument();
    expect(screen.getByRole('button', { name: /more actions/i })).toBeInTheDocument();
  });

  test('a live recording keeps its stop control while its transcript is watched', async () => {
    // The banner changing must not take the only way to stop the recording
    // with it.
    mockBackend({
      get_meeting_recording_status: {
        ...idleStatus,
        active: true,
        meeting_id: 'meeting-1',
        elapsed_seconds: 42,
      },
      get_meeting: recordedMeetingDetail(),
    });
    render(<MeetingsPage />);

    fireEvent.click(await screen.findByText('Weekly sync'));
    expect(await screen.findByRole('button', { name: /stop/i })).toBeInTheDocument();
  });

  test('adding a report to the graph is offered by the report, not by the meeting menu', async () => {
    mockBackend({
      get_meeting: recordedMeetingDetail({
        summary: {
          markdown: '# Notes\n\nWe agreed to ship.',
          status: 'completed',
          template_id: 'general',
          model: 'llama3.2:latest',
          chunk_count: 1,
          processing_ms: 900,
        },
      }),
    });
    render(<MeetingsPage />);
    fireEvent.click(await screen.findByText('Weekly sync'));

    const user = userEvent.setup();
    await user.click(await screen.findByRole('button', { name: /more actions/i }));
    expect(screen.queryByRole('menuitem', { name: /add to graph/i })).not.toBeInTheDocument();
    await user.keyboard('{Escape}');
    expect(screen.getByRole('button', { name: /add to graph/i })).toBeInTheDocument();
  });

  test('the calendar shows what is coming up and links to notes already recorded', async () => {
    const tomorrow = new Date(Date.now() + 86_400_000);
    const date = `${tomorrow.getFullYear()}-${String(tomorrow.getMonth() + 1).padStart(2, '0')}-${String(tomorrow.getDate()).padStart(2, '0')}`;
    mockBackend({
      get_calendar_agenda: [
        {
          date,
          events: [
            {
              id: 'evt-1',
              account_email: 'me@work.com',
              calendar_id: 'primary',
              title: 'Alumna Growth Team Daily Sync',
              start: `${date}T10:45:00.000Z`,
              end: `${date}T11:00:00.000Z`,
              all_day: false,
              attendees: [
                { email: 'payal@navgurukul.org', display_name: 'Payal', response: 'accepted', is_self: false, organizer: true },
              ],
              attendance: 'accepted',
              meeting_id: 'meeting-1',
            },
          ],
        },
      ],
    });
    render(<MeetingsPage />);

    expect(await screen.findByText('Coming up')).toBeInTheDocument();
    expect(screen.getByText('Alumna Growth Team Daily Sync')).toBeInTheDocument();
    expect(screen.getByText('Payal')).toBeInTheDocument();

    // A recorded meeting is reachable from the calendar row that produced it.
    fireEvent.click(screen.getByRole('button', { name: /^notes$/i }));
    expect(await screen.findByRole('button', { name: /all meetings/i })).toBeInTheDocument();
  });

  test('no connected calendar leaves the index with no calendar chrome at all', async () => {
    mockBackend({ get_calendar_agenda: [] });
    render(<MeetingsPage />);
    await screen.findByText('Weekly sync');
    expect(screen.queryByText('Coming up')).not.toBeInTheDocument();
  });

  describe('the agenda', () => {
    // A fixed clock. Every one of these tests turns on what day an event is
    // on relative to now, which is not a thing to leave to whatever time the
    // suite happens to run at.
    beforeEach(() => {
      vi.useFakeTimers({ shouldAdvanceTime: true });
      vi.setSystemTime(new Date(2026, 8, 14, 16, 0, 0));
    });
    afterEach(() => {
      vi.useRealTimers();
    });

    /** Puts `events` on the agenda, each under its own day. */
    const withAgenda = (events: CalendarEvent[], overrides: Record<string, unknown> = {}) => {
      const days = new Map<string, CalendarEvent[]>();
      for (const event of events) {
        const date = dayOf(event);
        days.set(date, [...(days.get(date) ?? []), event]);
      }
      mockBackend({
        get_calendar_agenda: [...days.entries()].map(([date, items]) => ({
          date,
          events: items,
        })),
        list_calendar_accounts: [account('me@work.com')],
        ...overrides,
      });
    };

    test('a meeting on today can be joined without leaving Vox', async () => {
      withAgenda([calendarEvent(18, { title: 'Campus Learning Weekly Check-in' })]);
      render(<MeetingsPage />);

      fireEvent.click(await screen.findByRole('button', { name: /^join$/i }));
      await waitFor(() =>
        expect(vi.mocked(invoke)).toHaveBeenCalledWith('open_calendar_link', {
          url: 'https://meet.google.com/abc-defg-hij',
        }),
      );
    });

    test('a meeting earlier today is still joinable, because a call can resume', async () => {
      withAgenda([
        calendarEvent(10, {
          title: 'Morning standup',
          // Still running at 16:00, so it is on the agenda at all.
          end: new Date(2026, 8, 14, 17, 0).toISOString(),
        }),
      ]);
      render(<MeetingsPage />);
      expect(await screen.findByRole('button', { name: /^join$/i })).toBeInTheDocument();
    });

    test("a meeting on another day shows it has a link but does not offer to open it", async () => {
      const tomorrow = new Date(2026, 8, 15, 10, 0);
      withAgenda([
        calendarEvent(10, {
          id: 'evt-tomorrow',
          title: 'Tomorrow sync',
          start: tomorrow.toISOString(),
          end: new Date(tomorrow.getTime() + 30 * 60_000).toISOString(),
        }),
      ]);
      render(<MeetingsPage />);

      await screen.findByText('Tomorrow sync');
      expect(screen.queryByRole('button', { name: /^join$/i })).not.toBeInTheDocument();
      expect(screen.getByTitle(/opens on the day/i)).toBeInTheDocument();
    });

    test('a meeting the user declined is struck through and stays joinable', async () => {
      // Declining a series and dropping into one of its meetings is normal, so
      // the strike-through is a statement about the invitation, not a lock.
      withAgenda([
        calendarEvent(18, {
          title: 'Campus Learning Weekly Check-in',
          attendance: 'declined',
        }),
      ]);
      render(<MeetingsPage />);

      const title = await screen.findByText('Campus Learning Weekly Check-in');
      expect(title.className).toMatch(/line-through/);
      expect(screen.getByText('Declined')).toBeInTheDocument();
      expect(screen.getByRole('button', { name: /^join$/i })).toBeInTheDocument();
    });

    test('two connected calendars are named in a key, so a row can be placed', async () => {
      withAgenda(
        [
          calendarEvent(18, { title: 'Work sync', account_email: 'me@work.com' }),
          calendarEvent(19, {
            id: 'evt-life',
            title: 'Dentist',
            account_email: 'me@gmail.com',
          }),
        ],
        {
          list_calendar_accounts: [
            account('me@work.com', { display_name: 'Work' }),
            account('me@gmail.com', { display_name: 'Personal' }),
          ],
        },
      );
      render(<MeetingsPage />);

      expect(await screen.findByText('Work')).toBeInTheDocument();
      expect(screen.getByText('Personal')).toBeInTheDocument();
    });

    test('one calendar needs no colour key', async () => {
      withAgenda([calendarEvent(18)], {
        list_calendar_accounts: [account('me@work.com', { display_name: 'Work' })],
      });
      render(<MeetingsPage />);
      await screen.findByText('Scrum Call');
      expect(screen.queryByText('Work')).not.toBeInTheDocument();
    });

    test("the invitation's own notes are readable without opening the calendar", async () => {
      withAgenda([
        calendarEvent(18, {
          description: 'Agenda:\n• Budget\n• Hiring',
          location: 'Room 4',
        }),
      ]);
      render(<MeetingsPage />);

      fireEvent.click(await screen.findByRole('button', { name: /show details for scrum call/i }));
      expect(await screen.findByText(/Agenda:/)).toBeInTheDocument();
      expect(screen.getByText('Room 4')).toBeInTheDocument();
    });

    test('an invitation with nothing written on it offers nothing to expand', async () => {
      withAgenda([calendarEvent(18)]);
      render(<MeetingsPage />);
      await screen.findByText('Scrum Call');
      expect(screen.queryByRole('button', { name: /show details/i })).not.toBeInTheDocument();
    });

    test('the calendar can be re-synced from the meetings page', async () => {
      // Noticing a meeting is missing happens here, not in Settings.
      withAgenda([calendarEvent(18)]);
      render(<MeetingsPage />);

      await screen.findByText('Scrum Call');
      vi.mocked(invoke).mockClear();
      fireEvent.click(screen.getByRole('button', { name: /^sync$/i }));
      await waitFor(() =>
        expect(vi.mocked(invoke)).toHaveBeenCalledWith('sync_calendars'),
      );
    });

    test('the week can be opened from the banner, with every account in it', async () => {
      withAgenda(
        [
          calendarEvent(18, { title: 'Work sync', account_email: 'me@work.com' }),
          calendarEvent(19, {
            id: 'evt-life',
            title: 'Dentist',
            account_email: 'me@gmail.com',
          }),
        ],
        {
          list_calendar_accounts: [
            account('me@work.com', { display_name: 'Work' }),
            account('me@gmail.com', { display_name: 'Personal' }),
          ],
        },
      );
      render(<MeetingsPage />);

      fireEvent.click(await screen.findByRole('button', { name: /^calendar$/i }));
      const week = within(await screen.findByRole('dialog'));
      expect(week.getByText('Work sync')).toBeInTheDocument();
      expect(week.getByText('Dentist')).toBeInTheDocument();
      // The key is what makes the colours mean anything.
      expect(week.getByText('Work')).toBeInTheDocument();
      expect(week.getByText('Personal')).toBeInTheDocument();
      expect(week.getByRole('button', { name: /previous week/i })).toBeInTheDocument();
    });

    test('a week with nothing in it is still a week, not an error', async () => {
      withAgenda([calendarEvent(18)]);
      render(<MeetingsPage />);

      fireEvent.click(await screen.findByRole('button', { name: /^calendar$/i }));
      const week = within(await screen.findByRole('dialog'));
      fireEvent.click(week.getByRole('button', { name: /next week/i }));
      expect(week.queryByText('Scrum Call')).not.toBeInTheDocument();
      fireEvent.click(week.getByRole('button', { name: /this week/i }));
      expect(week.getByText('Scrum Call')).toBeInTheDocument();
    });

    test('no connected calendar means no calendar button', async () => {
      mockBackend({ get_calendar_agenda: [], list_calendar_accounts: [] });
      render(<MeetingsPage />);
      await screen.findByText('Weekly sync');
      expect(screen.queryByRole('button', { name: /^calendar$/i })).not.toBeInTheDocument();
    });

    test('with no calendar connected there is nothing to sync', async () => {
      mockBackend({ get_calendar_agenda: [], list_calendar_accounts: [] });
      render(<MeetingsPage />);
      await screen.findByText('Weekly sync');
      expect(screen.queryByRole('button', { name: /^sync$/i })).not.toBeInTheDocument();
    });
  });
});
