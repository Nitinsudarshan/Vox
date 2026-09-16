import { render, screen, fireEvent } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, expect, it, vi } from 'vitest';

import { MeetingPill, formatMeetingTimer } from './MeetingPill';

describe('formatMeetingTimer', () => {
  it('formats seconds into MM:SS when under 1 hour', () => {
    expect(formatMeetingTimer(0)).toBe('00:00');
    expect(formatMeetingTimer(45)).toBe('00:45');
    expect(formatMeetingTimer(599)).toBe('09:59');
    expect(formatMeetingTimer(3599)).toBe('59:59');
  });

  it('formats seconds into HH:MM:SS when 1 hour or more', () => {
    expect(formatMeetingTimer(3600)).toBe('01:00:00');
    expect(formatMeetingTimer(3665)).toBe('01:01:05');
    expect(formatMeetingTimer(7322)).toBe('02:02:02');
  });
});

describe('MeetingPill', () => {
  it('renders resting recording state with timer and waveform', () => {
    render(
      <MeetingPill
        state="recording"
        elapsedSeconds={42}
      />
    );

    expect(screen.getByTestId('meeting-pill')).toBeInTheDocument();
    expect(screen.getByTestId('elapsed-timer')).toHaveTextContent('00:42');
    expect(screen.getByTestId('meeting-waveform')).toBeInTheDocument();
    expect(screen.queryByTestId('transcribing-loader')).not.toBeInTheDocument();
    expect(screen.queryByTestId('sys-audio-warning')).not.toBeInTheDocument();
    expect(screen.queryByTestId('meeting-pill-controls')).not.toBeInTheDocument();
  });

  it('renders paused state with amber status indicator', () => {
    render(
      <MeetingPill
        state="paused"
        elapsedSeconds={120}
      />
    );

    const statusDot = screen.getByTestId('status-indicator').querySelector('.bg-amber-400');
    expect(statusDot).toBeInTheDocument();
    expect(screen.getByTestId('elapsed-timer')).toHaveTextContent('02:00');
  });

  it('renders transcribing state with spinning loader', () => {
    render(
      <MeetingPill
        state="transcribing"
        elapsedSeconds={300}
      />
    );

    expect(screen.getByTestId('transcribing-loader')).toBeInTheDocument();
    expect(screen.queryByTestId('meeting-waveform')).not.toBeInTheDocument();
    // Controls should not show when transcribing even if hovered
    expect(screen.queryByTestId('meeting-pill-controls')).not.toBeInTheDocument();
  });

  it('displays warning triangle when system audio is inactive', () => {
    const { rerender } = render(
      <MeetingPill
        state="recording"
        elapsedSeconds={10}
        isSystemAudioActive={false}
      />
    );

    expect(screen.getByTestId('sys-audio-warning')).toBeInTheDocument();

    rerender(
      <MeetingPill
        state="recording"
        elapsedSeconds={10}
        isSystemAudioActive={true}
      />
    );

    expect(screen.queryByTestId('sys-audio-warning')).not.toBeInTheDocument();
  });

  it('reveals controls on mouse hover and fires onHoverChange', () => {
    const onHoverChange = vi.fn();
    render(
      <MeetingPill
        state="recording"
        elapsedSeconds={15}
        onHoverChange={onHoverChange}
      />
    );

    const pill = screen.getByTestId('meeting-pill');
    expect(screen.queryByTestId('meeting-pill-controls')).not.toBeInTheDocument();

    fireEvent.mouseEnter(pill);
    expect(onHoverChange).toHaveBeenCalledWith(true);
    expect(screen.getByTestId('meeting-pill-controls')).toBeInTheDocument();

    fireEvent.mouseLeave(pill);
    expect(onHoverChange).toHaveBeenCalledWith(false);
    expect(screen.queryByTestId('meeting-pill-controls')).not.toBeInTheDocument();
  });

  it('keeps controls open when forceExpanded is true', () => {
    render(
      <MeetingPill
        state="recording"
        elapsedSeconds={15}
        forceExpanded={true}
      />
    );

    expect(screen.getByTestId('meeting-pill-controls')).toBeInTheDocument();
  });

  it('triggers onTogglePause when pause button is clicked', async () => {
    const onTogglePause = vi.fn();
    render(
      <MeetingPill
        state="recording"
        elapsedSeconds={15}
        forceExpanded={true}
        onTogglePause={onTogglePause}
      />
    );

    const pauseBtn = screen.getByRole('button', { name: /pause recording/i });
    await userEvent.click(pauseBtn);

    expect(onTogglePause).toHaveBeenCalledTimes(1);
  });

  it('triggers onStop when stop button is clicked', async () => {
    const onStop = vi.fn();
    render(
      <MeetingPill
        state="recording"
        elapsedSeconds={15}
        forceExpanded={true}
        onStop={onStop}
      />
    );

    const stopBtn = screen.getByRole('button', { name: /stop and save this meeting/i });
    await userEvent.click(stopBtn);

    expect(onStop).toHaveBeenCalledTimes(1);
  });

  it('renders vertical orientation style with vertical layout and controls', () => {
    render(
      <MeetingPill
        state="recording"
        elapsedSeconds={25}
        orientation="vertical"
        forceExpanded={true}
      />
    );

    const pill = screen.getByTestId('meeting-pill');
    expect(pill).toHaveAttribute('data-orientation', 'vertical');
    expect(pill.className).toContain('flex-col');
    expect(screen.getByTestId('meeting-pill-controls')).toBeInTheDocument();
  });
});
