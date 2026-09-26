import React, { useCallback, useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { getCurrentWindow } from '@tauri-apps/api/window';
import { Video, Calendar, Clock, Disc, X, Users, ExternalLink, Loader2 } from 'lucide-react';
import { MeetingReminderPayload, ReminderKind } from '../../types';
import { useOverlayTheme } from '../../lib/overlayTheme';
import { VoxLogo } from '../common/VoxLogo';
import { describeError } from '../../lib/errors';

/**
 * How long the exit animation runs before the window is hidden.
 *
 * The window is hidden by the frontend rather than left to the backend so the
 * card is seen to leave. Hiding a window mid-animation just makes it vanish.
 */
const EXIT_ANIMATION_MS = 180;

/** What the snooze row offers. */
const SNOOZE_OPTIONS = [5, 10, 15] as const;

/**
 * The floating meeting reminder card.
 *
 * An app-owned window rather than an OS toast, because a toast on Windows
 * cannot carry buttons: `tauri-plugin-notification`'s desktop implementation
 * maps title, body and icon and silently drops actions. A reminder you cannot
 * act on is not this feature (`docs/decisions.md` Decision 46 and its
 * reversal).
 *
 * It renders state it is given and reports what was pressed. It owns no
 * recording lifecycle and no queue: every action is one command, and the
 * backend decides what that means.
 *
 * Painted from the app's own theme tokens, light or dark, and it follows the
 * switch live — it is a piece of Vox that happens to be outside its window,
 * and a permanently dark card over a light app reads as some other program's
 * notification. The surface is fully opaque either way, because it floats over
 * a background it cannot know. Nothing casts a shadow: on a transparent window
 * a blurred box-shadow is composited straight onto the desktop, which is the
 * grey halo that used to sit around it.
 */
export const MeetingReminderWindow: React.FC = () => {
  useOverlayTheme();
  const [reminder, setReminder] = useState<MeetingReminderPayload | null>(null);
  const [busy, setBusy] = useState(false);
  const [leaving, setLeaving] = useState(false);
  const [snoozeOpen, setSnoozeOpen] = useState(false);
  const [actionError, setActionError] = useState<string | null>(null);

  // The show protocol, both halves at once: pull whatever is already staged
  // (this window outlives any one reminder, so the event may have been emitted
  // before this view mounted), subscribe for the next one, and report ready so
  // the backend reveals a window that is already painted.
  useEffect(() => {
    let mounted = true;

    const show = (payload: MeetingReminderPayload) => {
      if (!mounted) return;
      setLeaving(false);
      setSnoozeOpen(false);
      setBusy(false);
      setActionError(null);
      setReminder(payload);
    };

    invoke<MeetingReminderPayload | null>('get_pending_meeting_reminder')
      .then((pending) => {
        if (pending) show(pending);
      })
      .catch((err) => console.error('Failed to read the pending reminder:', err));

    const unlisten = listen<MeetingReminderPayload>('meeting-reminder', (event) => {
      if (event.payload) show(event.payload);
    });

    invoke('meeting_reminder_ready').catch((err) =>
      console.error('Failed to report the reminder window ready:', err)
    );

    return () => {
      mounted = false;
      unlisten.then((off) => off());
    };
  }, []);

  // Hovering pauses the backend's auto-dismiss countdown. It lives there rather
  // than here because the countdown has to survive this view re-rendering.
  const setHovered = useCallback((hovered: boolean) => {
    invoke('meeting_reminder_hover_changed', { hovered }).catch(() => {
      /* The countdown is a convenience; failing to pause it is not worth an error. */
    });
  }, []);

  /**
   * Runs one action: plays the card out, hides the window, then invokes.
   *
   * The window goes first so the card is never seen sitting there after it has
   * been answered — a Record press that takes a second to open the recorder
   * would otherwise look like nothing happened.
   */
  const act = async (run: (reminder: MeetingReminderPayload) => Promise<unknown>) => {
    const current = reminder;
    if (!current || busy) return;

    setBusy(true);
    setLeaving(true);
    await new Promise((resolve) => setTimeout(resolve, EXIT_ANIMATION_MS));

    try {
      await getCurrentWindow().hide();
    } catch (err) {
      console.error('Failed to hide the reminder window:', err);
    }
    setReminder(null);
    setLeaving(false);
    setSnoozeOpen(false);

    try {
      await run(current);
    } catch (err) {
      // The card was hidden before the action ran; bring it back with the
      // reason. A Record that failed silently left the user believing the
      // meeting was being captured.
      console.error('Reminder action failed:', err);
      setReminder(current);
      setActionError(describeError(err, 'That did not work.'));
      getCurrentWindow()
        .show()
        .catch((showErr) => console.error('Failed to show the reminder window:', showErr));
    } finally {
      setBusy(false);
    }
  };

  const handleRecord = () =>
    act((r) => invoke('start_meeting_from_reminder', { key: r.key, kind: r.kind }));

  const handleJoin = () =>
    act((r) => invoke('join_meeting_from_reminder', { key: r.key, kind: r.kind }));

  const handleSnooze = (minutes: number) =>
    act((r) => invoke('snooze_meeting_reminder', { key: r.key, kind: r.kind, minutes }));

  const handleDismiss = () =>
    act((r) => invoke('dismiss_meeting_reminder', { key: r.key, kind: r.kind }));

  if (!reminder) {
    return <div className="w-full h-full bg-transparent" />;
  }

  return (
    <div
      className="w-full h-full p-2 bg-transparent select-none box-border"
      onMouseEnter={() => setHovered(true)}
      onMouseLeave={() => setHovered(false)}
    >
      <div
        className={`w-full h-full flex flex-col rounded-lg px-3.5 py-3
                    bg-card border border-border ring-1 ring-indigo-500/20
                    transition-all duration-150 ease-out ${
                      leaving ? 'opacity-0 translate-x-3' : 'opacity-100 translate-x-0'
                    }`}
      >
        <header className="flex items-center justify-between gap-2">
          <div className="flex items-center gap-1.5 min-w-0">
            {/* Whose card this is. It arrives unasked-for, over whatever the
                user is in the middle of, naming a meeting and its guests —
                so the first thing it has to answer is which program is
                asking, before anybody decides whether to trust the buttons. */}
            <VoxLogo expanded className="h-3 w-auto shrink-0" />
            <span aria-hidden="true" className="shrink-0 w-px h-3 bg-border" />
            <ProviderIcon provider={reminder.provider} />
            <span className="text-[11px] font-semibold text-muted-foreground truncate">
              {reminder.provider_name}
            </span>
            <KindBadge kind={reminder.kind} />
          </div>

          <button
            type="button"
            onClick={handleDismiss}
            disabled={busy}
            title="Dismiss this reminder"
            aria-label="Dismiss this reminder"
            className="shrink-0 p-1 rounded-md text-muted-foreground hover:text-foreground
                       hover:bg-foreground/10 transition-colors disabled:opacity-50"
          >
            <X className="w-3.5 h-3.5" />
          </button>
        </header>

        <div className="mt-1.5 min-w-0">
          <h1
            className="text-[13px] font-semibold text-foreground truncate leading-snug"
            title={reminder.title}
          >
            {reminder.title}
          </h1>
          <div className="mt-0.5 flex items-center gap-2 text-[11px] text-muted-foreground">
            <span className="truncate">{reminder.time_label}</span>
            {reminder.participants.length > 0 && (
              <>
                <span aria-hidden="true" className="text-muted-foreground/60">
                  •
                </span>
                <span
                  className="flex items-center gap-1 shrink-0"
                  title={reminder.participants.join(', ')}
                >
                  <Users className="w-3 h-3" aria-hidden="true" />
                  <span>{reminder.participants.length} invited</span>
                </span>
              </>
            )}
          </div>
        </div>

        {/* Actions sit on one row; snooze opens its durations in place rather
            than in a popover, because the window is a fixed size and anything
            escaping it would simply be clipped. */}
        {actionError && (
          <p role="alert" className="mt-1.5 text-[11px] leading-snug text-red-600 dark:text-red-400 line-clamp-2">
            {actionError}
          </p>
        )}
        <div className="mt-auto pt-2.5 flex items-center gap-2 border-t border-border">
          {snoozeOpen ? (
            <>
              <span className="text-[11px] text-muted-foreground shrink-0">Remind me in</span>
              {SNOOZE_OPTIONS.map((minutes) => (
                <button
                  key={minutes}
                  type="button"
                  onClick={() => handleSnooze(minutes)}
                  disabled={busy}
                  className="flex-1 h-7 rounded-md text-[11px] font-medium
                             bg-foreground/10 text-foreground hover:bg-foreground/20
                             transition-colors disabled:opacity-50"
                >
                  {minutes} min
                </button>
              ))}
              <button
                type="button"
                onClick={() => setSnoozeOpen(false)}
                disabled={busy}
                aria-label="Back to reminder actions"
                className="shrink-0 h-7 px-2 rounded-md text-[11px] text-muted-foreground
                           hover:text-foreground hover:bg-foreground/10 transition-colors"
              >
                Back
              </button>
            </>
          ) : (
            <>
              <button
                type="button"
                onClick={handleRecord}
                disabled={busy}
                className="flex-1 h-7 inline-flex items-center justify-center gap-1.5
                           rounded-md text-[11px] font-semibold text-white
                           bg-red-600 hover:bg-red-500 active:scale-[0.98]
                           transition-all disabled:opacity-50"
              >
                {busy ? (
                  <Loader2 className="w-3 h-3 animate-spin" aria-hidden="true" />
                ) : (
                  <Disc className="w-3 h-3 fill-current" aria-hidden="true" />
                )}
                <span>Record</span>
              </button>

              {reminder.can_join && (
                <button
                  type="button"
                  onClick={handleJoin}
                  disabled={busy}
                  title="Open this meeting in your browser"
                  className="flex-1 h-7 inline-flex items-center justify-center gap-1.5
                             rounded-md text-[11px] font-medium text-foreground
                             bg-foreground/10 hover:bg-foreground/20 active:scale-[0.98]
                             transition-all disabled:opacity-50"
                >
                  <ExternalLink className="w-3 h-3" aria-hidden="true" />
                  <span>Join</span>
                </button>
              )}

              <button
                type="button"
                onClick={() => setSnoozeOpen(true)}
                disabled={busy}
                className="flex-1 h-7 inline-flex items-center justify-center gap-1.5
                           rounded-md text-[11px] font-medium text-muted-foreground
                           bg-foreground/5 hover:bg-foreground/10 hover:text-foreground
                           active:scale-[0.98] transition-all disabled:opacity-50"
              >
                <Clock className="w-3 h-3" aria-hidden="true" />
                <span>Snooze</span>
              </button>
            </>
          )}
        </div>
      </div>
    </div>
  );
};

const ProviderIcon: React.FC<{ provider: string }> = ({ provider }) => {
  const isVideoCall = ['google_meet', 'zoom', 'teams', 'webex'].includes(provider.toLowerCase());
  return (
    <span className="shrink-0 p-1 rounded-md bg-foreground/10 flex items-center justify-center">
      {isVideoCall ? (
        <Video className="w-3 h-3 text-indigo-500 dark:text-indigo-300" aria-hidden="true" />
      ) : (
        <Calendar className="w-3 h-3 text-indigo-500 dark:text-indigo-300" aria-hidden="true" />
      )}
    </span>
  );
};

/**
 * Why this card is on screen, in two words.
 *
 * Worth the space: "starts soon" and "nobody is recording this" ask for
 * different things, and a card that looks identical either way makes the user
 * read the timestamp to work out which one it is.
 */
const KindBadge: React.FC<{ kind: ReminderKind }> = ({ kind }) => {
  const styles: Record<ReminderKind, { label: string; className: string }> = {
    upcoming: {
      label: 'Starts soon',
      className:
        'bg-indigo-500/10 text-indigo-600 border-indigo-500/25 dark:bg-indigo-500/15 dark:text-indigo-300 dark:border-indigo-400/25',
    },
    unrecorded: {
      label: 'Not recording',
      className:
        'bg-amber-500/10 text-amber-700 border-amber-500/30 dark:bg-amber-500/15 dark:text-amber-300 dark:border-amber-400/25',
    },
    detected: {
      label: 'Detected',
      className:
        'bg-emerald-500/10 text-emerald-700 border-emerald-500/30 dark:bg-emerald-500/15 dark:text-emerald-300 dark:border-emerald-400/25',
    },
  };
  const { label, className } = styles[kind];

  return (
    <span
      className={`shrink-0 px-1.5 py-px rounded border text-[10px] font-medium leading-4 ${className}`}
    >
      {label}
    </span>
  );
};
