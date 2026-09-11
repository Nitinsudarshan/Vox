import React from 'react';
import { convertFileSrc } from '@tauri-apps/api/core';
import { Pause, Play, Rewind, FastForward, AlertTriangle } from 'lucide-react';

import { Button } from '@/components/ui/button';
import { formatTimestamp } from '@/lib/meetings';

interface MeetingAudioPlayerProps {
  /** Absolute path to the merged recording. */
  audioPath: string;
  /** Seconds to jump to. Bumping `seekNonce` replays the same position. */
  seekTo?: { seconds: number; nonce: number } | null;
  /** Fires as playback moves, so the transcript can follow along. */
  onTimeChange?: (seconds: number) => void;
}

/** How far the skip buttons move, in seconds. */
const SKIP_SECONDS = 10;

const PLAYBACK_RATES = [1, 1.25, 1.5, 2] as const;

/**
 * Plays a meeting's recording, and is the other half of click-to-seek in the
 * transcript.
 *
 * The player exists for one question: "did they really say that?". Reading a
 * transcript line and being unable to hear it is the gap this closes, so the
 * controls are the ones that serve that — scrub, skip, and a speed control for
 * the re-listen — and not a waveform.
 *
 * `convertFileSrc` turns the recording's path into an `asset:` URL the webview
 * can range-request. That only works because the app grants the asset scope
 * for the vault's meetings directory at startup; if the grant failed, the
 * element errors and this says so rather than presenting a control that does
 * nothing.
 */
export const MeetingAudioPlayer: React.FC<MeetingAudioPlayerProps> = ({
  audioPath,
  seekTo,
  onTimeChange,
}) => {
  const audioRef = React.useRef<HTMLAudioElement>(null);
  const [playing, setPlaying] = React.useState(false);
  const [current, setCurrent] = React.useState(0);
  const [duration, setDuration] = React.useState(0);
  const [rate, setRate] = React.useState<number>(1);
  const [failed, setFailed] = React.useState(false);

  const src = React.useMemo(() => {
    try {
      return convertFileSrc(audioPath);
    } catch {
      return '';
    }
  }, [audioPath]);

  // A new meeting is a new file: reset rather than carry the last one's clock.
  React.useEffect(() => {
    setPlaying(false);
    setCurrent(0);
    setDuration(0);
    setFailed(false);
  }, [src]);

  React.useEffect(() => {
    if (!seekTo || !audioRef.current) return;
    audioRef.current.currentTime = seekTo.seconds;
    setCurrent(seekTo.seconds);
    void audioRef.current.play().catch(() => setPlaying(false));
    // `nonce` is in the dependency list so clicking the same line twice seeks
    // twice; `seconds` alone would make the second click a no-op.
  }, [seekTo?.seconds, seekTo?.nonce]);

  React.useEffect(() => {
    if (audioRef.current) audioRef.current.playbackRate = rate;
  }, [rate]);

  const toggle = () => {
    const element = audioRef.current;
    if (!element) return;
    if (element.paused) void element.play().catch(() => setFailed(true));
    else element.pause();
  };

  const skip = (delta: number) => {
    const element = audioRef.current;
    if (!element) return;
    const next = Math.min(Math.max(0, element.currentTime + delta), duration || Infinity);
    element.currentTime = next;
    setCurrent(next);
  };

  if (failed) {
    return (
      <p className="flex items-center gap-2 text-[11px] text-muted-foreground">
        <AlertTriangle className="w-3.5 h-3.5 shrink-0 text-amber-500" />
        This recording could not be played. The file may have been moved or removed.
      </p>
    );
  }

  return (
    <div className="flex items-center gap-2">
      <audio
        ref={audioRef}
        src={src}
        preload="metadata"
        onPlay={() => setPlaying(true)}
        onPause={() => setPlaying(false)}
        onEnded={() => setPlaying(false)}
        onError={() => setFailed(true)}
        onLoadedMetadata={(event) => {
          const value = event.currentTarget.duration;
          setDuration(Number.isFinite(value) ? value : 0);
        }}
        onTimeUpdate={(event) => {
          const seconds = event.currentTarget.currentTime;
          setCurrent(seconds);
          onTimeChange?.(seconds);
        }}
        data-testid="meeting-audio"
      />

      <Button
        size="icon"
        variant="ghost"
        className="h-8 w-8 shrink-0"
        onClick={() => skip(-SKIP_SECONDS)}
        aria-label={`Back ${SKIP_SECONDS} seconds`}
      >
        <Rewind className="w-3.5 h-3.5" />
      </Button>
      <Button
        size="icon"
        variant="secondary"
        className="h-8 w-8 shrink-0"
        onClick={toggle}
        aria-label={playing ? 'Pause recording' : 'Play recording'}
      >
        {playing ? <Pause className="w-4 h-4" /> : <Play className="w-4 h-4" />}
      </Button>
      <Button
        size="icon"
        variant="ghost"
        className="h-8 w-8 shrink-0"
        onClick={() => skip(SKIP_SECONDS)}
        aria-label={`Forward ${SKIP_SECONDS} seconds`}
      >
        <FastForward className="w-3.5 h-3.5" />
      </Button>

      <span className="text-[10px] font-mono text-muted-foreground tabular-nums shrink-0">
        {formatTimestamp(current)}
      </span>
      <input
        type="range"
        min={0}
        max={duration || 0}
        step={0.1}
        value={Math.min(current, duration || 0)}
        onChange={(event) => {
          const next = Number(event.target.value);
          if (audioRef.current) audioRef.current.currentTime = next;
          setCurrent(next);
        }}
        aria-label="Seek recording"
        disabled={duration === 0}
        className="flex-1 min-w-0 h-1 accent-primary cursor-pointer"
      />
      <span className="text-[10px] font-mono text-muted-foreground tabular-nums shrink-0">
        {formatTimestamp(duration)}
      </span>

      <Button
        size="sm"
        variant="ghost"
        className="h-7 px-2 text-[11px] font-mono shrink-0"
        onClick={() => setRate(PLAYBACK_RATES[(PLAYBACK_RATES.indexOf(rate as 1) + 1) % PLAYBACK_RATES.length])}
        aria-label="Playback speed"
      >
        {rate}×
      </Button>
    </div>
  );
};
