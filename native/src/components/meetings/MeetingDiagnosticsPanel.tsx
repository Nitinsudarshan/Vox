import React, { useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { AlertTriangle, Gauge } from 'lucide-react';

import { Button } from '@/components/ui/button';
import type { MeetingDiagnostics, SegmentDiagnostics } from '@/types/meetings';

interface MeetingDiagnosticsPanelProps {
  meetingId: string;
  diagnostics: MeetingDiagnostics;
}

/** How many of the slowest segments the drill-down shows. */
const SLOWEST_SHOWN = 8;

/** Seconds, rendered the way a person reads a duration. */
const duration = (seconds: number): string => {
  if (seconds < 1) return `${Math.round(seconds * 1000)} ms`;
  if (seconds < 60) return `${seconds.toFixed(1)} s`;
  return `${Math.floor(seconds / 60)}m ${Math.round(seconds % 60)}s`;
};

const percent = (fraction: number): string => `${Math.round(fraction * 100)}%`;

/**
 * A row of one measurement, with the unit attached to the number rather than
 * to a column header — these are read one at a time, not scanned.
 */
const Stat: React.FC<{ label: string; value: string; hint?: string }> = ({
  label,
  value,
  hint,
}) => (
  <div className="flex items-baseline justify-between gap-3 py-1">
    <span className="text-[11px] text-muted-foreground">{label}</span>
    <span className="text-[11px] font-medium tabular-nums" title={hint}>
      {value}
    </span>
  </div>
);

/**
 * Why this meeting's transcript looks the way it does.
 *
 * Split into capture and transcription rather than one health score, because
 * they fail for different reasons and are fixed by different actions: a
 * microphone that heard nothing is a device problem, a backlog is a model
 * problem, and one number covering both tells you neither.
 *
 * Shows nothing it did not measure. There is no confidence figure here because
 * Whisper does not report one.
 */
export const MeetingDiagnosticsPanel: React.FC<MeetingDiagnosticsPanelProps> = ({
  meetingId,
  diagnostics,
}) => {
  const { capture, transcription } = diagnostics;
  const [segments, setSegments] = useState<SegmentDiagnostics[] | null>(null);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);

  // Loaded on request rather than with the meeting: the rollup answers
  // "was this slow", and this answers "which segments", which is only worth
  // a thousand records once the first answer is yes.
  const loadSegments = async () => {
    setLoading(true);
    setLoadError(null);
    try {
      const loaded = await invoke<SegmentDiagnostics[]>('get_meeting_segment_diagnostics', {
        meetingId,
      });
      setSegments(loaded);
    } catch (error) {
      setLoadError(error instanceof Error ? error.message : String(error));
    } finally {
      setLoading(false);
    }
  };

  const slowest = segments
    ? [...segments]
        .sort((a, b) => b.decode_ms + b.queue_wait_ms - (a.decode_ms + a.queue_wait_ms))
        .slice(0, SLOWEST_SHOWN)
    : [];

  const wrongDevice =
    (capture.microphone_opened && !capture.microphone_heard) ||
    (capture.system_audio_opened && !capture.system_audio_heard);
  const audioIncomplete =
    !capture.audio_checkpoints_written || capture.checkpoint_failures > 0;
  const audioLost = capture.audio_lost_seconds > 0;
  const lostSpeech = transcription.segments_dropped + transcription.segments_failed;
  const behind = transcription.pipeline_rtf > 1;

  const coverage =
    capture.recording_seconds > 0
      ? transcription.transcribed_seconds / capture.recording_seconds
      : 0;
  const speechCoverage =
    transcription.speech_seconds > 0
      ? transcription.transcribed_seconds / transcription.speech_seconds
      : 0;

  const rejections = Object.entries(transcription.rejections).sort(
    (a, b) => b[1] - a[1],
  );

  return (
    <section
      className="rounded-lg border border-border/60 bg-muted/20 p-3"
      aria-label="Meeting diagnostics"
    >
      <header className="flex items-center gap-2 mb-2">
        <Gauge className="w-3.5 h-3.5 text-muted-foreground" />
        <h3 className="text-[11px] font-medium">How this meeting was transcribed</h3>
      </header>

      {audioIncomplete && (
        <p className="flex items-start gap-2 text-[11px] text-destructive mb-2">
          <AlertTriangle className="w-3.5 h-3.5 shrink-0 mt-0.5" />
          <span>
            {capture.audio_checkpoints_written
              ? `${capture.checkpoint_failures} checkpoint write(s) failed, so the recording has gaps.`
              : 'No audio was saved for this meeting. The transcript is all that exists of it.'}
          </span>
        </p>
      )}

      {audioLost && (
        <p className="flex items-start gap-2 text-[11px] text-destructive mb-2">
          <AlertTriangle className="w-3.5 h-3.5 shrink-0 mt-0.5" />
          <span>
            {duration(capture.audio_lost_seconds)} of audio never reached the writer, because it
            arrived faster than the disk took it. That much of the recording is missing.
          </span>
        </p>
      )}

      {wrongDevice && (
        <p className="flex items-start gap-2 text-[11px] text-amber-600 dark:text-amber-400 mb-2">
          <AlertTriangle className="w-3.5 h-3.5 shrink-0 mt-0.5" />
          <span>
            {capture.microphone_opened && !capture.microphone_heard
              ? 'The microphone was open and never heard anything — usually the wrong device.'
              : 'System audio was open and never heard anything, so the far end was not recorded.'}
          </span>
        </p>
      )}

      {behind && (
        <p className="flex items-start gap-2 text-[11px] text-amber-600 dark:text-amber-400 mb-2">
          <AlertTriangle className="w-3.5 h-3.5 shrink-0 mt-0.5" />
          <span>
            Transcription ran slower than the meeting ({transcription.pipeline_rtf.toFixed(2)}×
            real time), so a backlog built up. A smaller speech model keeps up.
          </span>
        </p>
      )}

      <div className="grid gap-x-6 sm:grid-cols-2">
        <div>
          <p className="text-[10px] uppercase tracking-wide text-muted-foreground mb-1">
            Recording
          </p>
          <Stat label="Length" value={duration(capture.recording_seconds)} />
          <Stat
            label="Microphone"
            value={
              !capture.microphone_opened
                ? 'not opened'
                : capture.microphone_heard
                  ? 'heard'
                  : 'silent'
            }
          />
          <Stat
            label="System audio"
            value={
              !capture.system_audio_opened
                ? 'not opened'
                : capture.system_audio_heard
                  ? 'heard'
                  : 'silent'
            }
          />
          <Stat
            label="Audio saved"
            value={capture.audio_checkpoints_written ? 'yes' : 'no'}
          />
        </div>

        <div>
          <p className="text-[10px] uppercase tracking-wide text-muted-foreground mb-1">
            Transcription
          </p>
          <Stat label="Model" value={diagnostics.model || 'unknown'} />
          <Stat
            label="Speed"
            value={`${transcription.pipeline_rtf.toFixed(2)}× clock`}
            hint="Decode time against the length of the recording. Below 1.0 keeps up; above it a backlog grows."
          />
          <Stat
            label="Coverage"
            value={`${percent(coverage)} of recording`}
            hint={`${percent(speechCoverage)} of what the segmenter called speech produced text.`}
          />
          <Stat
            label="Slowest segment"
            value={`${transcription.worst_decode_rtf.toFixed(2)}× (#${transcription.worst_decode_sequence})`}
          />
          <Stat
            label="Finalization p95"
            value={duration(transcription.finalization_p95_ms / 1000)}
            hint="From the segmenter closing a span to its text existing."
          />
          <Stat label="Peak queue" value={`${transcription.peak_queue_depth}`} />
          <Stat
            label="Wait after stop"
            value={
              transcription.drain_completed
                ? duration(transcription.drain_seconds)
                : `${duration(transcription.drain_seconds)} (gave up)`
            }
          />
        </div>
      </div>

      {lostSpeech > 0 && (
        <p className="text-[11px] text-amber-600 dark:text-amber-400 mt-2">
          {transcription.segments_dropped} segment(s) never reached the decoder and{' '}
          {transcription.segments_failed} failed in it. The recording still has that speech.
        </p>
      )}

      {rejections.length > 0 && (
        <p className="text-[11px] text-muted-foreground mt-2">
          Screened out:{' '}
          {rejections.map(([reason, count]) => `${reason} (${count})`).join(', ')}. Whisper
          invents fluent text over silence; these were discarded rather than transcribed.
        </p>
      )}

      {transcription.model_reloads > 0 && (
        <p className="text-[11px] text-muted-foreground mt-2">
          The speech model was reloaded {transcription.model_reloads} time(s) — another surface
          was using it, which costs {duration(transcription.model_load_ms_total / 1000)}.
        </p>
      )}

      <div className="mt-2">
        {segments === null ? (
          <Button size="sm" variant="ghost" onClick={loadSegments} disabled={loading}>
            {loading ? 'Loading…' : 'Show the slowest segments'}
          </Button>
        ) : slowest.length === 0 ? (
          <p className="text-[11px] text-muted-foreground">
            No per-segment records were kept for this meeting.
          </p>
        ) : (
          <table className="w-full text-[11px] tabular-nums">
            <caption className="sr-only">
              The {SLOWEST_SHOWN} segments that took longest, by queue wait plus decode
            </caption>
            <thead>
              <tr className="text-muted-foreground text-left">
                <th scope="col" className="font-normal pr-2">#</th>
                <th scope="col" className="font-normal pr-2">at</th>
                <th scope="col" className="font-normal pr-2">audio</th>
                <th scope="col" className="font-normal pr-2">queue</th>
                <th scope="col" className="font-normal pr-2">decode</th>
                <th scope="col" className="font-normal">outcome</th>
              </tr>
            </thead>
            <tbody>
              {slowest.map((segment) => (
                <tr key={segment.sequence}>
                  <td className="pr-2">{segment.sequence}</td>
                  <td className="pr-2">{duration(segment.start_seconds)}</td>
                  <td className="pr-2">
                    {duration(Math.max(0, segment.end_seconds - segment.start_seconds))}
                  </td>
                  <td className="pr-2">{segment.queue_wait_ms} ms</td>
                  <td className="pr-2">{segment.decode_ms} ms</td>
                  <td className="text-muted-foreground">
                    {segment.status === 'kept'
                      ? `${segment.text_chars} chars`
                      : (segment.rejection ?? segment.error ?? segment.status)}
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        )}
        {loadError && (
          <p className="text-[11px] text-destructive mt-1">
            Those records could not be read: {loadError}
          </p>
        )}
      </div>
    </section>
  );
};
