import React, { useState } from 'react';
import { Square, Loader2, Pause, Play, AlertTriangle } from 'lucide-react';

import { VoxLogo } from '@/components/common/VoxLogo';

import { MeetingPillWaveform } from './MeetingPillMark';

export type MeetingPillState = 'recording' | 'paused' | 'transcribing';
export type MeetingPillOrientation = 'horizontal' | 'vertical';

export interface MeetingPillProps {
  /** Current recording activity state. */
  state: MeetingPillState;
  /** Total elapsed recording time in seconds. */
  elapsedSeconds: number;
  /** Pill orientation style: horizontal (default) or vertical. */
  orientation?: MeetingPillOrientation;
  /** Whether system audio is captured. If false, shows an amber warning. */
  isSystemAudioActive?: boolean;
  /** Microphone level samples (0..1) for the top half of the waveform. */
  micLevels?: number[];
  /** System audio level samples (0..1) for the bottom half of the waveform. */
  sysLevels?: number[];
  /** Whether an async action (pause/stop) is currently in flight. */
  isBusy?: boolean;
  /** Controlled hover state from parent (e.g. window-level hover). */
  isHovered?: boolean;
  /** Always keep control buttons visible without requiring mouse hover. */
  forceExpanded?: boolean;
  /** Emitted when hover state changes. */
  onHoverChange?: (hovered: boolean) => void;
  /** Triggered when the pause/resume action button is clicked. */
  onTogglePause?: () => void;
  /** Triggered when the stop recording action button is clicked. */
  onStop?: () => void;
  /** Optional container class name. */
  className?: string;
}

const WAVEFORM_BAR_COUNT = 20;
const SILENT_LEVELS: number[] = new Array(WAVEFORM_BAR_COUNT).fill(0);

/**
 * Formats a duration in seconds into `MM:SS` or `HH:MM:SS`.
 */
export const formatMeetingTimer = (totalSeconds: number): string => {
  const hrs = Math.floor(totalSeconds / 3600);
  const mins = Math.floor((totalSeconds % 3600) / 60);
  const secs = totalSeconds % 60;
  if (hrs > 0) {
    return `${String(hrs).padStart(2, '0')}:${String(mins).padStart(2, '0')}:${String(secs).padStart(2, '0')}`;
  }
  return `${String(mins).padStart(2, '0')}:${String(secs).padStart(2, '0')}`;
};

/**
 * The floating meeting recording capsule.
 *
 * One self-contained surface holding:
 * - Vox logo mark for unmistakable program identity.
 * - Recording status indicator (pulsing red for recording, amber for paused, indigo for finalizing).
 * - Elapsed recorded time formatted in tabular figures.
 * - Live mirrored audio waveform (microphone above centreline, system audio below; or mirrored left/right in vertical).
 * - Controls that open inside the pill rather than floating alongside.
 * - Supports both 'horizontal' (default) and 'vertical' capsule orientations.
 */
export const MeetingPill: React.FC<MeetingPillProps> = ({
  state,
  elapsedSeconds,
  orientation = 'horizontal',
  isSystemAudioActive = true,
  micLevels = SILENT_LEVELS,
  sysLevels = SILENT_LEVELS,
  isBusy = false,
  isHovered,
  forceExpanded = false,
  onHoverChange,
  onTogglePause,
  onStop,
  className = '',
}) => {
  const [internalHover, setInternalHover] = useState(false);

  const activeHover = isHovered !== undefined ? isHovered : internalHover;
  const isPaused = state === 'paused';
  const isFinalizing = state === 'transcribing';
  const isRecording = state === 'recording';
  const isVertical = orientation === 'vertical';
  const showControls = (activeHover || forceExpanded) && !isFinalizing;

  const handleMouseEnter = () => {
    setInternalHover(true);
    onHoverChange?.(true);
  };

  const handleMouseLeave = () => {
    setInternalHover(false);
    onHoverChange?.(false);
  };

  return (
    <div
      onMouseEnter={handleMouseEnter}
      onMouseLeave={handleMouseLeave}
      className={`inline-flex items-center rounded-lg bg-card border border-border ring-1 ring-indigo-500/20
                 transition-all duration-150 select-none shadow-xs ${
                   isVertical
                     ? 'flex-col gap-2 w-12 py-2.5 px-1.5 rounded-2xl'
                     : 'gap-2.5 h-11 pl-3 pr-2'
                 } ${className}`}
      data-testid="meeting-pill"
      data-orientation={orientation}
    >
      {/* Brand mark */}
      <VoxLogo className="w-4 h-4 shrink-0" />

      {/* Status dot & elapsed duration */}
      <div
        className={`flex items-center shrink-0 ${
          isVertical ? 'flex-col gap-1 text-center' : 'gap-2'
        }`}
      >
        <span className="relative flex w-2 h-2" data-testid="status-indicator">
          {isRecording && (
            <span className="absolute inline-flex w-2 h-2 rounded-full bg-red-500 opacity-70 animate-ping" />
          )}
          <span
            className={`relative inline-flex w-2 h-2 rounded-full ${
              isPaused ? 'bg-amber-400' : isFinalizing ? 'bg-indigo-400' : 'bg-red-500'
            }`}
          />
        </span>
        <span
          className={`font-mono leading-none font-medium tabular-nums text-foreground ${
            isVertical ? 'text-[11px] tracking-tight' : 'text-[13px]'
          }`}
          data-testid="elapsed-timer"
        >
          {formatMeetingTimer(elapsedSeconds)}
        </span>
      </div>

      {/* System audio inactive warning */}
      {!isSystemAudioActive && (
        <span
          className="flex items-center shrink-0 text-amber-400"
          title="Only this machine's microphone is being recorded"
          aria-label="Only this machine's microphone is being recorded"
          data-testid="sys-audio-warning"
        >
          <AlertTriangle className="w-3.5 h-3.5" />
        </span>
      )}

      {/* Waveform / Finalizing loader */}
      {isFinalizing ? (
        <span
          className="flex items-center shrink-0 text-muted-foreground"
          data-testid="transcribing-loader"
        >
          <Loader2 className="w-4 h-4 animate-spin" />
        </span>
      ) : (
        <div
          className="shrink-0"
          title={isVertical ? 'You on left, meeting on right' : 'You above the line, the meeting below it'}
          data-testid="meeting-waveform"
        >
          <MeetingPillWaveform
            mic={micLevels}
            sys={sysLevels}
            muted={isPaused}
            orientation={orientation}
          />
        </div>
      )}

      {/* Action controls (open inside the pill) */}
      {showControls && (
        <div
          className={`flex items-center shrink-0 ${
            isVertical ? 'flex-col gap-1 pt-1.5 border-t border-border/40' : 'gap-1'
          }`}
          data-testid="meeting-pill-controls"
        >
          <button
            type="button"
            onClick={onTogglePause}
            disabled={isBusy}
            title={isPaused ? 'Resume recording' : 'Pause recording'}
            aria-label={isPaused ? 'Resume recording' : 'Pause recording'}
            className="grid place-items-center w-7 h-7 rounded-md text-muted-foreground
                       hover:bg-foreground/10 hover:text-foreground
                       disabled:opacity-40 disabled:cursor-not-allowed
                       focus-visible:outline focus-visible:outline-2 focus-visible:outline-indigo-400
                       cursor-pointer transition-colors"
          >
            {isPaused ? (
              <Play className="w-3.5 h-3.5 fill-current" />
            ) : (
              <Pause className="w-3.5 h-3.5 fill-current" />
            )}
          </button>
          <button
            type="button"
            onClick={onStop}
            disabled={isBusy}
            title="Stop and save this meeting"
            aria-label="Stop and save this meeting"
            className="grid place-items-center w-7 h-7 rounded-md text-white bg-red-500
                       hover:bg-red-600 disabled:opacity-40 disabled:cursor-not-allowed
                       focus-visible:outline focus-visible:outline-2 focus-visible:outline-indigo-400
                       cursor-pointer transition-colors"
          >
            <Square className="w-3 h-3 fill-current" />
          </button>
        </div>
      )}
    </div>
  );
};
