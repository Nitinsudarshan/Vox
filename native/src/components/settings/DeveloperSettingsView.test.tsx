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
          },
        ];
      }
      return undefined;
    });
    await renderPanel();

    await userEvent.click(screen.getByRole('button', { name: /check window detection/i }));

    expect(await screen.findByText('Placement review')).toBeInTheDocument();
    expect(screen.getByText('Placement review - Google Meet')).toBeInTheDocument();
    expect(screen.getByText('google_meet · 0.90')).toBeInTheDocument();
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
});
