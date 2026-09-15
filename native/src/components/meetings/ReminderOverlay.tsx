import React from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { Bell, CircleAlert, MapPin, Radio, Users, Video, X } from 'lucide-react';

import { Button } from '@/components/ui/button';
import * as calendar from '@/lib/calendar';
import * as meetings from '@/lib/meetings';
import { MEETING_REMINDER_EVENT, type MeetingReminder } from '@/types/meetings';

/** How many reminders stack before the oldest is dropped. */
const MAX_VISIBLE = 3;

/** How long a reminder sits there before it goes on its own, in ms. */
const DISMISS_AFTER_MS = 90_000;

/**
 * The whole content of the meeting-reminder window.
 *
 * This renders in a window of its own — undecorated, transparent, always on
 * top, off the taskbar — rather than inside Vox or as a Windows toast. Inside
 * Vox it would only reach somebody already looking at Vox, which is precisely
 * not the person about to miss a meeting; a Windows toast goes to the
 * notification centre, cannot carry a Join button that does the right thing,
 * and is silenced by Focus Assist without saying so.
 *
 * The window is built hidden at startup and lives for the session, so this
 * component mounts once and is listening long before the first reminder. It
 * tells Rust how tall it is after every render that changes the stack, and
 * asks to be hidden once the last card is gone — a transparent always-on-top
 * window left up with nothing in it still swallows clicks.
 */
export const ReminderOverlay: React.FC = () => {
  const [reminders, setReminders] = React.useState<MeetingReminder[]>([]);
  const stackRef = React.useRef<HTMLDivElement>(null);

  React.useEffect(() => {
    const subscription = listen<MeetingReminder>(MEETING_REMINDER_EVENT, (event) => {
      setReminders((current) => {
        // The key is per bucket, so the same meeting can legitimately arrive
        // twice — at ten minutes and then at one. Re-announcing replaces the
        // older card rather than stacking a second copy of one meeting.
        const withoutSameEvent = current.filter(
          (existing) =>
            !(
              existing.event_id === event.payload.event_id &&
              existing.account_email === event.payload.account_email
            ),
        );
        return [...withoutSameEvent, event.payload].slice(-MAX_VISIBLE);
      });
    });
    return () => {
      void subscription.then((off) => off());
    };
  }, []);

  // The window is sized from what was actually laid out, not from a guess at
  // how tall a card is: a long meeting title wraps and a short one does not.
  React.useEffect(() => {
    if (reminders.length === 0) {
      void invoke('dismiss_meeting_reminders').catch(() => undefined);
      return;
    }
    const height = (stackRef.current?.offsetHeight ?? 0) + 24;
    void invoke('resize_meeting_reminders', { height }).catch(() => undefined);
  }, [reminders]);

  const dismiss = React.useCallback((key: string) => {
    setReminders((current) => current.filter((reminder) => reminder.key !== key));
  }, []);

  if (reminders.length === 0) return null;

  return (
    <div
      ref={stackRef}
      className="flex flex-col gap-2 p-3"
      role="region"
      aria-label="Meeting reminders"
    >
      {reminders.map((reminder) => (
        <ReminderCard
          key={reminder.key}
          reminder={reminder}
          onDismiss={() => dismiss(reminder.key)}
        />
      ))}
    </div>
  );
};

/** How long is left, recomputed as the card sits there. */
function useCountdown(start: string, initialMinutes: number): number {
  const [minutes, setMinutes] = React.useState(initialMinutes);

  React.useEffect(() => {
    const startsAt = new Date(start).getTime();
    if (Number.isNaN(startsAt)) return undefined;

    // Recomputed rather than counted down from the value the backend sent: a
    // card that sits through a laptop sleeping should show the real time left,
    // not the one it was born with.
    const update = () => setMinutes(Math.round((startsAt - Date.now()) / 60_000));
    update();
    const timer = window.setInterval(update, 15_000);
    return () => window.clearInterval(timer);
  }, [start]);

  return minutes;
}

const ReminderCard: React.FC<{
  reminder: MeetingReminder;
  onDismiss: () => void;
}> = ({ reminder, onDismiss }) => {
  const minutes = useCountdown(reminder.start, reminder.minutes_until);

  React.useEffect(() => {
    const timer = window.setTimeout(onDismiss, DISMISS_AFTER_MS);
    return () => window.clearTimeout(timer);
  }, [onDismiss]);

  const nudge = reminder.kind === 'not_recording';

  const when =
    minutes > 1
      ? `in ${minutes} minutes`
      : minutes === 1
        ? 'in 1 minute'
        : minutes === 0
          ? 'now'
          : `${-minutes} minute${minutes === -1 ? '' : 's'} ago`;

  /** The line under the title. The nudge's is a statement, not a countdown. */
  const subtitle = nudge
    ? `Started ${when} · nothing is being recorded`
    : minutes < 0
      ? `Started ${when}`
      : `Starts ${when}`;

  /** Starts recording without opening anything. */
  const record = () => {
    void meetings.startMeeting(reminder.title).catch(() => undefined);
    onDismiss();
  };

  /** Opens the call, and starts recording it when asked. */
  const join = (andRecord: boolean) => {
    if (reminder.conference_url) {
      void calendar.openCalendarLink(reminder.conference_url).catch(() => undefined);
    }
    if (andRecord) {
      void meetings.startMeeting(reminder.title).catch(() => undefined);
    }
    onDismiss();
  };

  return (
    // The nudge is bordered differently on purpose: it is the only one of the
    // three that is about something already going wrong, and it is the one a
    // person needs to pick out of a stack at a glance.
    <div
      className={`rounded-xl border shadow-2xl p-3 ${
        nudge ? 'border-amber-500/60 bg-amber-500/5' : 'border-border bg-card'
      }`}
    >
      <div className="flex items-start gap-2">
        {nudge ? (
          <CircleAlert className="w-3.5 h-3.5 text-amber-500 shrink-0 mt-0.5" />
        ) : (
          <Bell className="w-3.5 h-3.5 text-primary shrink-0 mt-0.5" />
        )}
        <div className="min-w-0 flex-1">
          <p className="text-sm font-medium text-foreground leading-snug">{reminder.title}</p>
          <p
            className={`text-[11px] mt-0.5 ${
              nudge ? 'text-amber-700 dark:text-amber-400' : 'text-muted-foreground'
            }`}
          >
            {subtitle}
          </p>
        </div>
        <button
          type="button"
          onClick={onDismiss}
          aria-label={`Dismiss the reminder for ${reminder.title}`}
          className="text-muted-foreground hover:text-foreground transition-colors shrink-0 cursor-pointer"
        >
          <X className="w-3.5 h-3.5" />
        </button>
      </div>

      {(reminder.location || reminder.guest_count > 0) && (
        <p className="flex flex-wrap items-center gap-x-3 gap-y-0.5 text-[11px] text-muted-foreground mt-1.5 ml-5">
          {reminder.location && (
            <span className="inline-flex items-center gap-1">
              <MapPin className="w-3 h-3" />
              {reminder.location}
            </span>
          )}
          {reminder.guest_count > 0 && (
            <span className="inline-flex items-center gap-1">
              <Users className="w-3 h-3" />
              {reminder.guest_count} {reminder.guest_count === 1 ? 'guest' : 'guests'}
            </span>
          )}
        </p>
      )}

      <div className="flex items-center gap-2 mt-2.5 ml-5">
        {/* The call is already happening, so recording it is the action and
            Join is the afterthought — the other way round from the two
            reminders that arrive before it starts. */}
        {nudge ? (
          <>
            <Button size="sm" onClick={() => record()} className="h-7 gap-1.5 text-xs">
              <Radio className="w-3.5 h-3.5" />
              Record now
            </Button>
            {reminder.conference_url && (
              <Button
                size="sm"
                variant="outline"
                onClick={() => join(false)}
                className="h-7 gap-1.5 text-xs"
              >
                <Video className="w-3.5 h-3.5" />
                Join
              </Button>
            )}
          </>
        ) : reminder.conference_url ? (
          <>
            <Button size="sm" onClick={() => join(false)} className="h-7 gap-1.5 text-xs">
              <Video className="w-3.5 h-3.5" />
              Join
            </Button>
            <Button
              size="sm"
              variant="outline"
              onClick={() => join(true)}
              className="h-7 gap-1.5 text-xs"
            >
              <Radio className="w-3.5 h-3.5" />
              Join and record
            </Button>
          </>
        ) : (
          <Button size="sm" variant="outline" onClick={() => record()} className="h-7 gap-1.5 text-xs">
            <Radio className="w-3.5 h-3.5" />
            Record
          </Button>
        )}
      </div>
    </div>
  );
};
