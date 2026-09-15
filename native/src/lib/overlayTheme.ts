import React from 'react';
import { listen } from '@tauri-apps/api/event';

/** What the main window writes its choice to, and the key Relay used before it. */
const THEME_KEYS = ['vox-theme', 'relay-theme'];

/** The events `ThemeToggle` emits app-wide when the user flips the switch. */
const THEME_EVENTS = ['vox-theme-changed', 'relay-theme-changed'];

export type OverlayTheme = 'light' | 'dark';

/**
 * The theme the main window is currently in.
 *
 * `localStorage` is shared across every webview in the app, so the overlay
 * reads the same value the main window wrote rather than being told it — which
 * matters because an overlay can be created long after the choice was made,
 * and a reminder window that has been hidden for six hours must come back in
 * the theme the user is looking at now.
 */
export function resolveTheme(): OverlayTheme {
  for (const key of THEME_KEYS) {
    try {
      const saved = localStorage.getItem(key);
      if (saved === 'light' || saved === 'dark') return saved;
    } catch {
      // Private mode, or storage disabled. The system preference below still
      // gives an answer, and a wrong one is better than an unstyled window.
    }
  }
  if (typeof window !== 'undefined' && window.matchMedia) {
    return window.matchMedia('(prefers-color-scheme: dark)').matches ? 'dark' : 'light';
  }
  return 'light';
}

/**
 * Keeps an overlay window in the app's theme, for as long as it is mounted.
 *
 * Every surface Vox puts on the desktop is a piece of the app that happens to
 * be outside its window, so a dark pill floating over a light app reads as
 * something else's notification. The class goes on `documentElement`, which is
 * what the `--background`/`--foreground` tokens key off, so components style
 * themselves with the same tokens the rest of the app uses.
 */
export function useOverlayTheme(): OverlayTheme {
  const [theme, setTheme] = React.useState<OverlayTheme>(resolveTheme);

  React.useEffect(() => {
    document.documentElement.classList.toggle('dark', theme === 'dark');
  }, [theme]);

  React.useEffect(() => {
    const apply = () => setTheme(resolveTheme());

    // Flipping the switch in the main window has to reach here immediately:
    // the pill is on screen throughout, and a meeting is exactly when
    // somebody dims their screen for the evening.
    const unlisten = THEME_EVENTS.map((event) => listen(event, apply));

    // A second window in the same app writing `localStorage` fires this, and
    // it is the fallback if an event is ever missed.
    window.addEventListener('storage', apply);

    // Only meaningful while nothing is saved, but harmless otherwise: `apply`
    // re-reads the saved value first either way.
    const media = window.matchMedia?.('(prefers-color-scheme: dark)');
    media?.addEventListener('change', apply);

    return () => {
      unlisten.forEach((pending) => void pending.then((off) => off()).catch(() => {}));
      window.removeEventListener('storage', apply);
      media?.removeEventListener('change', apply);
    };
  }, []);

  return theme;
}
