import React, { useCallback, useEffect, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';

import { useOverlayTheme } from '@/lib/overlayTheme';

import {
  MEETING_EVENTS,
  type MeetingLevels,
  type MeetingRecordingStatus,
} from '../../types/meetings';
import {
  MeetingPill,
  type MeetingPillState,
  type MeetingPillOrientation,
} from './MeetingPill';

/**
 * Samples held in the waveform, one per bar.
 *
 * The backend emits a level pair every 40 ms and each event shifts the history
 * by one bar, so this is a little under a second of audio on screen — long
 * enough to read as a wave, short enough that it tracks the voice rather than
 * lagging behind it.
 */
const WAVEFORM_BAR_COUNT = 20;
const SILENT_LEVELS: number[] = new Array(WAVEFORM_BAR_COUNT).fill(0);

/**
 * States in which a recording is no longer occupying the pill.
 *
 * Vox reports one recording status rather than a session object, so "nothing
 * is running" is `active: false` — these cover a status that is still being
 * reported while its recording winds down.
 */
const TERMINAL_STATES = ['completed', 'failed'];

/**
 * How often the pill reconciles against the backend.
 *
 * The pill is a subscriber, never the owner, of recording state — but events
 * alone are not enough to stay correct: the overlay's webview can be created
 * after a session has already started (missing the start event) and it survives
 * across sessions, so a single missed event used to leave it showing a stale
 * session whose timer kept counting between meetings. Reconciling on a timer
 * makes every such divergence self-healing within one interval.
 */
const RECONCILE_INTERVAL_MS = 1000;
const TIMER_TICK_MS = 250;

/**
 * The floating meeting recording pill.
 *
 * One self-contained surface: the status dot, the elapsed time, and the live
 * waveform are inside it, and hovering opens pause and stop inside it too
 * rather than floating them alongside. Nothing is attached to the pill; the
 * pill is the whole object.
 *
 * A meeting runs for an hour, and for that hour this sits on top of whatever the
 * user is actually working in — so it stays narrow at rest and the window grows
 * only while the controls are open. The window is transparent but still takes
 * clicks, so unused width is an invisible dead zone, not free space.
 *
 * Painted from the app's own theme tokens, light or dark, and it follows the
 * switch live. This is a piece of Vox that happens to be outside its window,
 * and a permanently dark pill over a light app reads as some other program's
 * notification. Nothing casts a shadow: on a transparent window a blurred
 * box-shadow is composited straight onto the desktop, which is the grey halo
 * that used to sit around it.
 */
export const MeetingRecordingOverlay: React.FC = () => {
  useOverlayTheme();
  const [session, setSession] = useState<MeetingRecordingStatus | null>(null);
  const [elapsedSec, setElapsedSec] = useState<number>(0);
  const [micLevels, setMicLevels] = useState<number[]>(SILENT_LEVELS);
  const [sysLevels, setSysLevels] = useState<number[]>(SILENT_LEVELS);
  const [isBusy, setIsBusy] = useState<boolean>(false);
  const [isHovered, setIsHovered] = useState<boolean>(false);
  const [pillStyle, setPillStyle] = useState<MeetingPillOrientation>('horizontal');

  useEffect(() => {
    void invoke<Record<string, unknown>>('get_settings')
      .then((all) => {
        const stored = (all?.meetings ?? {}) as { pillStyle?: MeetingPillOrientation };
        if (stored.pillStyle) setPillStyle(stored.pillStyle);
      })
      .catch(() => undefined);

    const unlisten = listen<{ meetings?: { pillStyle?: MeetingPillOrientation } }>(
      'settings-changed',
      (event) => {
        if (event.payload?.meetings?.pillStyle) {
          setPillStyle(event.payload.meetings.pillStyle);
        }
      },
    );

    return () => {
      void unlisten.then((f) => f());
    };
  }, []);

  /**
   * Backend-reported recorded duration plus the local instant it arrived.
   * Elapsed time is interpolated from this rather than from `started_at`, so the
   * display excludes paused intervals and can never drift away from the
   * recording itself.
   */
  const durationAnchor = useRef<{ seconds: number; at: number } | null>(null);

  const applySession = useCallback((next: MeetingRecordingStatus | null) => {
    if (!next || !next.active || TERMINAL_STATES.includes(next.state ?? '')) {
      durationAnchor.current = null;
      setSession(null);
      setElapsedSec(0);
      setMicLevels(SILENT_LEVELS);
      setSysLevels(SILENT_LEVELS);
      return;
    }

    durationAnchor.current = {
      seconds: next.elapsed_seconds || 0,
      at: performance.now(),
    };
    setSession(next);
  }, []);

  // Reconcile with the backend on mount and on an interval, and react
  // immediately to state changes in between.
  useEffect(() => {
    let cancelled = false;

    const reconcile = async () => {
      try {
        const active = await invoke<MeetingRecordingStatus | null>(
          'get_meeting_recording_status',
        );
        if (!cancelled) {
          applySession(active);
        }
      } catch (err) {
        console.error('Failed to reconcile active meeting:', err);
      }
    };

    reconcile();
    const poll = setInterval(reconcile, RECONCILE_INTERVAL_MS);

    const unlistenState = listen<MeetingRecordingStatus>(MEETING_EVENTS.state, (event) => {
      applySession(event.payload);
    });

    const unlistenLevels = listen<MeetingLevels>(MEETING_EVENTS.level, (event) => {
      const { mic: mic_level, system: sys_level } = event.payload;
      // Both channels shift the same way — newest on the right — so a bar's top
      // and bottom halves are the same instant. Opposing directions would put
      // the two meters on different timelines and the mirrored wave would be
      // showing something that never happened.
      setMicLevels((prev) => [...prev.slice(1), Math.min(1, mic_level || 0)]);
      setSysLevels((prev) => [...prev.slice(1), Math.min(1, sys_level || 0)]);
    });

    return () => {
      cancelled = true;
      clearInterval(poll);
      unlistenState.then((f) => f());
      unlistenLevels.then((f) => f());
    };
  }, [applySession]);

  // Interpolate between reconciliations, and only while actually recording.
  const isRecording = Boolean(session?.active) && session?.state !== 'paused';
  useEffect(() => {
    const update = () => {
      const anchor = durationAnchor.current;
      if (!anchor) {
        setElapsedSec(0);
        return;
      }
      const drift = isRecording ? (performance.now() - anchor.at) / 1000 : 0;
      setElapsedSec(Math.max(0, Math.floor(anchor.seconds + drift)));
    };

    update();
    if (!isRecording) {
      return;
    }
    const timer = setInterval(update, TIMER_TICK_MS);
    return () => clearInterval(timer);
  }, [isRecording, session?.meeting_id, session?.state, session?.elapsed_seconds]);

  // The window has to grow before the controls can be seen, so hover state is
  // pushed to the backend rather than handled in CSS alone.
  const setExpanded = useCallback((expanded: boolean) => {
    setIsHovered(expanded);
    invoke('set_meeting_overlay_expanded', { expanded }).catch((err) => {
      console.error('Failed to resize the meeting pill:', err);
    });
  }, []);

  // Never leave the window expanded behind a pill that has gone away.
  useEffect(() => {
    if (!session && isHovered) {
      setExpanded(false);
    }
  }, [session, isHovered, setExpanded]);

  const isPaused = session?.state === 'paused';
  const isFinalizing = session?.state === 'transcribing';

  const handleTogglePause = async () => {
    if (!session || isBusy || isFinalizing) return;
    setIsBusy(true);
    try {
      const command = isPaused ? 'resume_meeting' : 'pause_meeting';
      await invoke(command);
      applySession(await invoke<MeetingRecordingStatus | null>('get_meeting_recording_status'));
    } catch (err) {
      console.error('Failed to toggle meeting pause:', err);
    } finally {
      setIsBusy(false);
    }
  };

  const handleStop = async () => {
    if (!session || isBusy || isFinalizing) return;
    setIsBusy(true);
    try {
      await invoke('stop_meeting');
      applySession(null);
    } catch (err) {
      console.error('Failed to stop meeting recording:', err);
    } finally {
      setIsBusy(false);
    }
  };

  if (!session) {
    return <div className="w-full h-full bg-transparent" />;
  }

  const pillState: MeetingPillState = isFinalizing
    ? 'transcribing'
    : isPaused
      ? 'paused'
      : 'recording';

  return (
    <div
      className="w-full h-full flex items-center justify-end pr-1 bg-transparent select-none"
      onMouseEnter={() => setExpanded(true)}
      onMouseLeave={() => setExpanded(false)}
    >
      <MeetingPill
        orientation={pillStyle}
        state={pillState}
        elapsedSeconds={elapsedSec}
        isSystemAudioActive={session.system_audio_active ?? true}
        micLevels={micLevels}
        sysLevels={sysLevels}
        isBusy={isBusy}
        isHovered={isHovered}
        onTogglePause={handleTogglePause}
        onStop={handleStop}
      />
    </div>
  );
};

