import { act, render, screen, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { listen } from '@tauri-apps/api/event';
import { resolveTheme, useOverlayTheme } from './overlayTheme';

/** Every listener the component registered, by event name. */
const listeners = new Map<string, (event: { payload: unknown }) => void>();

function Probe() {
  const theme = useOverlayTheme();
  return <span data-testid="theme">{theme}</span>;
}

/**
 * The overlays are separate webviews, so they learn the theme rather than
 * inheriting it. Getting that wrong is not subtle: a dark pill pinned over a
 * light app for the length of a meeting reads as some other program's window,
 * which is the one thing a recording indicator must never look like.
 */
describe('useOverlayTheme', () => {
  beforeEach(() => {
    listeners.clear();
    localStorage.clear();
    document.documentElement.classList.remove('dark');
    vi.mocked(listen).mockImplementation((async (event: string, handler: unknown) => {
      listeners.set(event, handler as (e: { payload: unknown }) => void);
      return () => listeners.delete(event);
    }) as unknown as typeof listen);
  });

  afterEach(() => {
    document.documentElement.classList.remove('dark');
  });

  it('starts in the theme the main window saved', () => {
    localStorage.setItem('vox-theme', 'dark');

    render(<Probe />);

    expect(screen.getByTestId('theme')).toHaveTextContent('dark');
    expect(document.documentElement.classList.contains('dark')).toBe(true);
  });

  it('stays light when nothing has been saved and the system is light', () => {
    render(<Probe />);

    expect(screen.getByTestId('theme')).toHaveTextContent('light');
    expect(document.documentElement.classList.contains('dark')).toBe(false);
  });

  it('follows the switch while it is on screen', async () => {
    localStorage.setItem('vox-theme', 'light');
    render(<Probe />);
    expect(screen.getByTestId('theme')).toHaveTextContent('light');

    // What `ThemeToggle` does: write, then announce.
    localStorage.setItem('vox-theme', 'dark');
    act(() => listeners.get('vox-theme-changed')?.({ payload: 'dark' }));

    await waitFor(() =>
      expect(document.documentElement.classList.contains('dark')).toBe(true),
    );
    expect(screen.getByTestId('theme')).toHaveTextContent('dark');
  });

  it('still reads the key Relay wrote, for a profile carried over', () => {
    localStorage.setItem('relay-theme', 'dark');

    expect(resolveTheme()).toBe('dark');
  });
});
