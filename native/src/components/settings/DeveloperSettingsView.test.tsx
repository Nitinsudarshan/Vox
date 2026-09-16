import { render, screen, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { invoke } from '@tauri-apps/api/core';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { DeveloperSettingsView } from './DeveloperSettingsView';

/**
 * This panel is how anybody checks that reminders work at all.
 *
 * A reminder fires in a few specific minutes around a meeting and nowhere
 * else, so without these buttons the only way to see one is to wait for a real
 * meeting and hope. That makes the panel itself load-bearing: a Send button
 * that looks right and sends the wrong kind — or nothing — hides exactly the
 * breakage it exists to surface.
 */
describe('DeveloperSettingsView', () => {
  beforeEach(() => {
    vi.mocked(invoke).mockReset();
    vi.mocked(invoke).mockImplementation(async (command: string) => {
      if (command === 'get_developer_settings') return { force_onboarding_on_launch: false };
      return undefined;
    });
  });

  async function renderPanel() {
    render(<DeveloperSettingsView />);
    await screen.findByText('Meeting reminders');
  }

  it('lists every reminder kind with when it fires', async () => {
    await renderPanel();

    expect(screen.getByText('Before it starts')).toBeInTheDocument();
    expect(screen.getByText('Running unrecorded')).toBeInTheDocument();
    expect(screen.getByText('Detected call')).toBeInTheDocument();
    expect(screen.getByText('t−5')).toBeInTheDocument();
    expect(screen.getByText('t+5')).toBeInTheDocument();
    expect(screen.getByText('ad-hoc')).toBeInTheDocument();
  });

  it.each([
    ['Before it starts', 'upcoming'],
    ['Running unrecorded', 'unrecorded'],
    ['Detected call', 'detected'],
  ])('sends a real %s reminder through the notification service', async (label, kind) => {
    await renderPanel();
    const row = screen.getByText(label).closest('li') as HTMLElement;

    await userEvent.click(within(row).getByRole('button', { name: 'Send' }));

    expect(invoke).toHaveBeenCalledWith('trigger_mock_meeting_reminder', { kind });
    expect(await screen.findByRole('status')).toHaveTextContent(/top-right/i);
  });

  it('says so when a reminder could not be raised', async () => {
    vi.mocked(invoke).mockImplementation(async (command: string) => {
      if (command === 'get_developer_settings') return { force_onboarding_on_launch: false };
      throw new Error('reminder window is gone');
    });
    await renderPanel();
    const row = screen.getByText('Before it starts').closest('li') as HTMLElement;

    await userEvent.click(within(row).getByRole('button', { name: 'Send' }));

    expect(await screen.findByRole('status')).toHaveTextContent(/reminder window is gone/);
  });

  it('lists the conferencing windows detection can see', async () => {
    vi.mocked(invoke).mockImplementation(async (command: string) => {
      if (command === 'get_developer_settings') return { force_onboarding_on_launch: false };
      if (command === 'debug_detect_conferencing_windows') {
        return [
          {
            provider: 'google_meet',
            title: 'Placement review',
            raw_title: 'Placement review - Google Meet',
            source: 'window_title',
            confidence: 0.9,
            blocked_by: null,
          },
        ];
      }
      return undefined;
    });
    await renderPanel();

    await userEvent.click(screen.getByRole('button', { name: /check window detection/i }));

    expect(await screen.findByText('Placement review')).toBeInTheDocument();
    expect(screen.getByText('Placement review - Google Meet')).toBeInTheDocument();
    expect(screen.getByText('google_meet · window_title · 0.90')).toBeInTheDocument();
    expect(screen.getByText('Would raise a reminder.')).toBeInTheDocument();
  });

  it('says why a call it can see would still raise nothing', async () => {
    // The question the button exists for. A gate's silence and a broken
    // feature's silence are the same silence without this.
    vi.mocked(invoke).mockImplementation(async (command: string) => {
      if (command === 'get_developer_settings') return { force_onboarding_on_launch: false };
      if (command === 'debug_detect_conferencing_windows') {
        return [
          {
            provider: 'zoom',
            title: 'Zoom Meeting',
            raw_title: 'Zoom Meeting',
            source: 'window_class',
            confidence: 0.55,
            blocked_by: 'Detected-call reminders are switched off in Settings.',
          },
        ];
      }
      return undefined;
    });
    await renderPanel();

    await userEvent.click(screen.getByRole('button', { name: /check window detection/i }));

    expect(
      await screen.findByText(/No reminder: Detected-call reminders are switched off/),
    ).toBeInTheDocument();
  });

  /**
   * An empty list is a result, not a non-answer: it is what separates "no call
   * is open" from "detection cannot see anything on this machine", which is
   * the whole reason the button exists.
   */
  it('distinguishes nothing detected from nothing happening', async () => {
    vi.mocked(invoke).mockImplementation(async (command: string) => {
      if (command === 'get_developer_settings') return { force_onboarding_on_launch: false };
      if (command === 'debug_detect_conferencing_windows') return [];
      return undefined;
    });
    await renderPanel();

    await userEvent.click(screen.getByRole('button', { name: /check window detection/i }));

    expect(await screen.findByText('No conferencing windows on screen.')).toBeInTheDocument();
  });

  it('renders the interactive meeting pill workbench', async () => {
    await renderPanel();

    expect(screen.getByText('Meeting Pill Workbench')).toBeInTheDocument();
    expect(screen.getByTestId('meeting-pill')).toBeInTheDocument();
    expect(screen.getByRole('switch', { name: /force expand meeting pill controls/i })).toBeInTheDocument();
    expect(screen.getByRole('switch', { name: /toggle missing system audio warning/i })).toBeInTheDocument();
  });

  it('allows switching pill states and toggles in the playground', async () => {
    await renderPanel();

    // Switch to Paused state via workbench button
    await userEvent.click(screen.getByRole('button', { name: 'Paused' }));
    expect(screen.getByTestId('status-indicator').querySelector('.bg-amber-400')).toBeInTheDocument();

    // Toggle missing system audio
    await userEvent.click(screen.getByRole('switch', { name: /toggle missing system audio warning/i }));
    expect(screen.getByTestId('sys-audio-warning')).toBeInTheDocument();

    // Force expand controls
    await userEvent.click(screen.getByRole('switch', { name: /force expand meeting pill controls/i }));
    expect(screen.getByTestId('meeting-pill-controls')).toBeInTheDocument();
  });

  it('allows interacting directly with the pill action buttons in the playground', async () => {
    await renderPanel();

    // Force expand so buttons are accessible
    await userEvent.click(screen.getByRole('switch', { name: /force expand meeting pill controls/i }));

    // Click Pause button on the pill
    const pauseBtn = screen.getByRole('button', { name: /pause recording/i });
    await userEvent.click(pauseBtn);

    // Pill should now be paused (amber indicator, play button appears)
    expect(screen.getByTestId('status-indicator').querySelector('.bg-amber-400')).toBeInTheDocument();
    expect(screen.getByRole('button', { name: /resume recording/i })).toBeInTheDocument();

    // Click Resume button on the pill
    await userEvent.click(screen.getByRole('button', { name: /resume recording/i }));
    expect(screen.getByTestId('status-indicator').querySelector('.bg-red-500')).toBeInTheDocument();

    // Click Stop button on the pill
    await userEvent.click(screen.getByRole('button', { name: /stop and save this meeting/i }));
    expect(screen.getByTestId('transcribing-loader')).toBeInTheDocument();
  });

  it('allows switching between horizontal and vertical styles and selecting screen positions', async () => {
    await renderPanel();

    // Default style is horizontal
    expect(screen.getByTestId('meeting-pill')).toHaveAttribute('data-orientation', 'horizontal');

    // Switch to vertical style
    await userEvent.click(screen.getByRole('button', { name: /vertical/i }));
    expect(screen.getByTestId('meeting-pill')).toHaveAttribute('data-orientation', 'vertical');

    // Switch back to horizontal
    await userEvent.click(screen.getByRole('button', { name: /horizontal/i }));
    expect(screen.getByTestId('meeting-pill')).toHaveAttribute('data-orientation', 'horizontal');

    // Click Top Right position in the grid
    await userEvent.click(screen.getByTitle(/Top Right/i));
    expect(screen.getByText(/Anchor: top_right/i)).toBeInTheDocument();

    // Select Free Movement mode
    await userEvent.click(screen.getByRole('button', { name: /free movement/i }));
    expect(screen.getByText(/Viewport boundaries locked \(cannot exit screen\)/i)).toBeInTheDocument();
    expect(screen.getByText(/Free Movement active/i)).toBeInTheDocument();
  });
});

