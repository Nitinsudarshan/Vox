import React from 'react';
import { listen } from '@tauri-apps/api/event';
import { Mic, MonitorSpeaker, Pause, Play, Square, AlertTriangle, Radio } from 'lucide-react';

import { Button } from '@/components/ui/button';
import { Badge } from '@/components/ui/badge';
import { Card } from '@/components/ui/card';
import {
  MEETING_EVENTS,
  type MeetingLevels,
  type MeetingRecordingStatus,
} from '@/types/meetings';
import { formatTimestamp } from '@/lib/meetings';

interface MeetingRecorderProps {
  status: MeetingRecordingStatus;
  busy: boolean;
  onStart: () => void;
  onPause: () => void;
  onResume: () => void;
  onStop: () => void;
}

/**
 * The recording control surface: start/pause/stop, live level meters for both
 * capture channels, and the counters that say whether transcription is keeping
 * up.
 *
 * The meters are the point of the two-channel display. A meeting where the
 * microphone bar never moves is one where the wrong input device is open —
 * which is otherwise only discovered when the transcript turns out to be empty.
 */
export const MeetingRecorder: React.FC<MeetingRecorderProps> = ({
  status,
  busy,
  onStart,
  onPause,
  onResume,
  onStop,
}) => {
  const [levels, setLevels] = React.useState<MeetingLevels>({ mic: 0, system: 0 });

  React.useEffect(() => {
    const unlisten = listen<MeetingLevels>(MEETING_EVENTS.level, (event) => {
      setLevels(event.payload);
    });
    return () => {
      void unlisten.then((off) => off());
    };
  }, []);

  const isPaused = status.state === 'paused';
  const isRecording = status.active;
  const backlog = Math.max(0, status.segments_queued - status.segments_completed);

  if (!isRecording) {
    return (
      <Card className="p-5 flex flex-col sm:flex-row sm:items-center gap-4">
        <div className="flex-1 min-w-0">
          <h2 className="text-sm font-semibold text-foreground">Record a meeting</h2>
          <p className="text-xs text-muted-foreground mt-1">
            Captures your microphone and this machine&apos;s audio together, transcribes on
            device, and keeps both in your vault.
          </p>
        </div>
        <Button onClick={onStart} disabled={busy} className="gap-2 shrink-0">
          <Radio className="w-4 h-4" />
          Start recording
        </Button>
      </Card>
    );
  }

  return (
    <Card className="p-5 space-y-4 border-destructive/40">
      <div className="flex flex-col sm:flex-row sm:items-center gap-3">
        <div className="flex items-center gap-3 flex-1 min-w-0">
          <span
            className={`w-2.5 h-2.5 rounded-full shrink-0 ${
              isPaused ? 'bg-amber-500' : 'bg-destructive animate-pulse'
            }`}
            aria-hidden
          />
          <div className="min-w-0">
            <p className="text-sm font-semibold text-foreground truncate">
              {status.title ?? 'Recording'}
            </p>
            <p className="text-xs text-muted-foreground font-mono">
              {formatTimestamp(status.elapsed_seconds)}
              {isPaused && ' · paused'}
            </p>
          </div>
        </div>
        <div className="flex items-center gap-2 shrink-0">
          {isPaused ? (
            <Button variant="outline" size="sm" onClick={onResume} disabled={busy} className="gap-1.5">
              <Play className="w-3.5 h-3.5" />
              Resume
            </Button>
          ) : (
            <Button variant="outline" size="sm" onClick={onPause} disabled={busy} className="gap-1.5">
              <Pause className="w-3.5 h-3.5" />
              Pause
            </Button>
          )}
          <Button variant="destructive" size="sm" onClick={onStop} disabled={busy} className="gap-1.5">
            <Square className="w-3.5 h-3.5" />
            Stop
          </Button>
        </div>
      </div>

      <div className="grid gap-2.5">
        <LevelMeter
          icon={Mic}
          label="Microphone"
          level={isPaused ? 0 : levels.mic}
          active={status.microphone_active}
          heard={status.microphone_heard}
        />
        <LevelMeter
          icon={MonitorSpeaker}
          label="System audio"
          level={isPaused ? 0 : levels.system}
          active={status.system_audio_active}
          heard={status.system_audio_heard}
        />
      </div>

      {status.warning && (
        <p className="flex items-start gap-2 text-xs text-amber-600 dark:text-amber-400">
          <AlertTriangle className="w-3.5 h-3.5 shrink-0 mt-0.5" />
          <span>{status.warning}</span>
        </p>
      )}

      <div className="flex flex-wrap items-center gap-2 text-[11px] text-muted-foreground">
        <Badge variant="outline" className="font-mono text-[10px]">
          {status.segments_completed} transcribed
        </Badge>
        {backlog > 0 && (
          <Badge variant="amber" className="font-mono text-[10px]">
            {backlog} waiting
          </Badge>
        )}
        {status.segments_dropped > 0 && (
          <Badge variant="destructive" className="font-mono text-[10px]">
            {status.segments_dropped} dropped
          </Badge>
        )}
      </div>
    </Card>
  );
};

interface LevelMeterProps {
  icon: React.ComponentType<{ className?: string }>;
  label: string;
  level: number;
  active: boolean;
  heard: boolean;
}

/**
 * One capture channel's bar.
 *
 * `active` is whether the stream is bound; `heard` is whether anything above
 * the silence floor has ever arrived on it. Both false means the device is not
 * open; active but never heard is the "wrong microphone" case, and is called
 * out in words rather than left to the bar.
 */
const LevelMeter: React.FC<LevelMeterProps> = ({ icon: Icon, label, level, active, heard }) => (
  <div className="flex items-center gap-3">
    <Icon className={`w-4 h-4 shrink-0 ${active ? 'text-foreground' : 'text-muted-foreground/50'}`} />
    <div className="flex-1 min-w-0">
      <div className="flex items-center justify-between mb-1">
        <span className="text-[11px] font-medium text-muted-foreground">{label}</span>
        {!active && <span className="text-[10px] text-muted-foreground">not captured</span>}
        {active && !heard && (
          <span className="text-[10px] text-amber-600 dark:text-amber-400">nothing heard yet</span>
        )}
      </div>
      <div
        className="h-1.5 rounded-full bg-muted overflow-hidden"
        role="meter"
        aria-label={`${label} level`}
        aria-valuenow={Math.round(level * 100)}
        aria-valuemin={0}
        aria-valuemax={100}
      >
        <div
          className={`h-full rounded-full transition-[width] duration-75 ${
            active ? 'bg-emerald-500' : 'bg-muted-foreground/30'
          }`}
          style={{ width: `${Math.min(100, Math.max(0, level * 100))}%` }}
        />
      </div>
    </div>
  </div>
);
