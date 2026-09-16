import React from 'react';

/**
 * The pill's visual identity, kept in one small file so it can be redrawn
 * without touching recording logic — `MeetingRecordingOverlay` stays a state
 * machine and this stays a drawing.
 */

interface MeetingPillWaveformProps {
  /** Microphone levels, newest last. Clamped to 0..1. */
  mic: number[];
  /** System-audio levels, newest last, on the same timeline as `mic`. */
  sys: number[];
  /** Flattens the bars and drops them to the idle colour. */
  muted?: boolean;
  /** Layout orientation: horizontal (default) or vertical dual-channel meter. */
  orientation?: 'horizontal' | 'vertical';
}

/** Half the waveform's height or width: mic on one side of centreline, meeting on the other. */
const HALF_PX = 11;
/** Every bar keeps this much thickness/height, so silence is a line rather than a gap. */
const MIN_BAR_PX = 2;
/** Below this a bar is drawn idle, so room tone does not tint the whole meter. */
const AUDIBLE = 0.02;

/**
 * Bar colours, one pair per theme.
 *
 * The idle bar has to read as "quiet", not as "off": a dark idle bar on the
 * light card is heavier than a live one, which inverts the whole meter.
 */
const MIC_LIVE = 'bg-indigo-500 dark:bg-indigo-400';
const SYS_LIVE = 'bg-sky-500 dark:bg-sky-400';
const IDLE = 'bg-neutral-300 dark:bg-neutral-700';

const barPx = (level: number) =>
  Math.max(MIN_BAR_PX, Math.round(Math.max(0, Math.min(1, level)) * HALF_PX));

/**
 * The live level meter: one mirrored waveform, and the only moving part.
 *
 * Both channels are drawn on a single strip sharing one timeline — the
 * microphone on one half of the centreline, the meeting's audio on the other — so "who is
 * talking right now" is legible without a legend explaining which half is
 * which.
 */
export const MeetingPillWaveform: React.FC<MeetingPillWaveformProps> = ({
  mic,
  sys,
  muted = false,
  orientation = 'horizontal',
}) => {
  if (orientation === 'vertical') {
    // 12 bars vertically to fit nicely in a vertical pill
    const verticalMic = mic.slice(-12);
    const verticalSys = sys.slice(-12);

    return (
      <div
        className="flex flex-col items-center justify-center gap-[2px]"
        style={{ width: HALF_PX * 2 + 2 }}
        aria-hidden="true"
      >
        {verticalMic.map((micLevel, index) => {
          const sysLevel = verticalSys[index] ?? 0;
          const micLive = !muted && micLevel > AUDIBLE;
          const sysLive = !muted && sysLevel > AUDIBLE;
          return (
            <div
              key={index}
              className="flex items-center justify-center gap-[2px] h-[3px]"
            >
              <span
                className={`h-full rounded-full transition-[width] duration-75 ease-out ${
                  micLive ? MIC_LIVE : IDLE
                }`}
                style={{ width: muted ? MIN_BAR_PX : barPx(micLevel) }}
              />
              <span
                className={`h-full rounded-full transition-[width] duration-75 ease-out ${
                  sysLive ? SYS_LIVE : IDLE
                }`}
                style={{ width: muted ? MIN_BAR_PX : barPx(sysLevel) }}
              />
            </div>
          );
        })}
      </div>
    );
  }

  return (
    <div
      className="flex items-center gap-[2px]"
      style={{ height: HALF_PX * 2 + 2 }}
      aria-hidden="true"
    >
      {mic.map((micLevel, index) => {
        const sysLevel = sys[index] ?? 0;
        const micLive = !muted && micLevel > AUDIBLE;
        const sysLive = !muted && sysLevel > AUDIBLE;
        return (
          <div
            key={index}
            className="flex flex-col items-center justify-center gap-[2px] w-[3px]"
          >
            <span
              className={`w-full rounded-full transition-[height] duration-75 ease-out ${
                micLive ? MIC_LIVE : IDLE
              }`}
              style={{ height: muted ? MIN_BAR_PX : barPx(micLevel) }}
            />
            <span
              className={`w-full rounded-full transition-[height] duration-75 ease-out ${
                sysLive ? SYS_LIVE : IDLE
              }`}
              style={{ height: muted ? MIN_BAR_PX : barPx(sysLevel) }}
            />
          </div>
        );
      })}
    </div>
  );
};
