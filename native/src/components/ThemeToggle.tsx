import React, { useState, useEffect } from 'react';
import { Sun, Moon } from 'lucide-react';
import { emit } from '@tauri-apps/api/event';
import { invoke } from '@tauri-apps/api/core';
import { applyThemeWithoutTransition } from '@/lib/utils';

export type ThemeMode = 'light' | 'dark';

export const ThemeToggle: React.FC = () => {
  const [theme, setTheme] = useState<ThemeMode>(() => {
    const saved = localStorage.getItem('vox-theme') ?? localStorage.getItem('relay-theme');
    if (saved === 'light' || saved === 'dark') {
      return saved;
    }
    if (typeof window !== 'undefined' && window.matchMedia?.('(prefers-color-scheme: dark)')?.matches) {
      return 'dark';
    }
    return 'light';
  });

  useEffect(() => {
    applyThemeWithoutTransition(theme === 'dark');
    localStorage.setItem('vox-theme', theme);
    emit('vox-theme-changed', theme).catch(() => {});
    emit('relay-theme-changed', theme).catch(() => {});
    invoke('update_taskbar_theme_icon').catch(() => {});
  }, [theme]);

  useEffect(() => {
    if (typeof window === 'undefined' || !window.matchMedia) return;
    const media = window.matchMedia('(prefers-color-scheme: dark)');
    const handleMediaChange = () => {
      invoke('update_taskbar_theme_icon').catch(() => {});
    };
    media.addEventListener?.('change', handleMediaChange);
    return () => {
      media.removeEventListener?.('change', handleMediaChange);
    };
  }, []);

  const toggleTheme = () => {
    setTheme((prev) => (prev === 'dark' ? 'light' : 'dark'));
  };

  return (
    <button
      type="button"
      onClick={toggleTheme}
      className="group h-8 w-8 rounded-lg border border-border bg-card hover:bg-muted/80 text-foreground flex items-center justify-center cursor-pointer shadow-xs active:scale-95 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring select-none"
      aria-label={theme === 'dark' ? 'Switch to light mode' : 'Switch to dark mode'}
      title={theme === 'dark' ? 'Switch to light mode' : 'Switch to dark mode'}
    >
      {theme === 'dark' ? (
        <Sun
          key="sun"
          className="w-4 h-4 text-amber-400 group-hover:text-amber-300 transition-transform duration-200 transform group-hover:rotate-45 group-hover:scale-110"
        />
      ) : (
        <Moon
          key="moon"
          className="w-4 h-4 text-indigo-500 group-hover:text-indigo-600 transition-transform duration-200 transform group-hover:-rotate-12 group-hover:scale-110"
        />
      )}
    </button>
  );
};
