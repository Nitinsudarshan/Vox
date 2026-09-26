import { act, render, screen } from '@testing-library/react';
import { listen, type EventCallback } from '@tauri-apps/api/event';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { DictationPill } from './DictationPill';

type CapturePayload = { active: boolean; mode?: string | null; status: string; message?: string | null };

let emitCaptureState: (payload: CapturePayload) => void;

beforeEach(() => {
  vi.useFakeTimers();
  vi.mocked(listen).mockImplementation(async (event, handler) => {
    if (event === 'capture-state-changed') {
      emitCaptureState = (payload) =>
        act(() => (handler as EventCallback<CapturePayload>)({ event, id: 0, payload }));
    }
    return () => {};
  });
});

afterEach(() => {
  vi.useRealTimers();
  vi.mocked(listen).mockReset();
});

describe('DictationPill', () => {
  it('stays recording when a new dictation starts before the last result collapses', () => {
    render(<DictationPill />);

    emitCaptureState({ active: false, mode: 'dictation', status: 'SUCCESS' });
    expect(screen.getByText('Text inserted')).toBeInTheDocument();

    emitCaptureState({ active: true, mode: 'dictation', status: 'LISTENING' });
    act(() => {
      vi.advanceTimersByTime(3000);
    });

    // The previous session's collapse timer must not hide a live recording.
    expect(screen.queryByText('Click to dictate')).not.toBeInTheDocument();
    expect(screen.queryByText('Text inserted')).not.toBeInTheDocument();
  });

  it('finishes a todo recorded from another surface instead of staying on Transcribing', () => {
    render(<DictationPill />);

    emitCaptureState({ active: true, mode: 'todo', status: 'LISTENING' });
    emitCaptureState({ active: false, mode: 'todo', status: 'TRANSCRIBING' });
    expect(screen.getByText('Transcribing...')).toBeInTheDocument();

    emitCaptureState({ active: false, mode: 'todo', status: 'SUCCESS' });
    expect(screen.getByText('Todo added')).toBeInTheDocument();

    act(() => {
      vi.advanceTimersByTime(2500);
    });
    expect(screen.queryByText('Transcribing...')).not.toBeInTheDocument();
    expect(screen.getByText('Click to dictate')).toBeInTheDocument();
  });
});
