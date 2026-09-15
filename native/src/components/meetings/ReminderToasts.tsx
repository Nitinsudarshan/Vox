import React from 'react';
import { listen } from '@tauri-apps/api/event';
import { Bell, MapPin, Radio, Users, Video, X } from 'lucide-react';

import { Button } from '@/components/ui/button';
import * as calendar from '@/lib/calendar';
import { MEETING_REMINDER_EVENT, type MeetingReminder } from '@/types/meetings';

/** How many reminders stack before the oldest is dropped. */
const MAX_VISIBLE = 3;

/** How long a reminder sits there before it goes on its own, in ms. */
const DISMISS_AFTER_MS = 90_000;

interface ReminderToastsProps {
  /** Takes the user to the Meetings page with a recording running. */
  onStartMeeting: () => void;
}

/**
 * What a meeting reminder looks like inside Vox.
 *
 * The OS toast is the other half of this and is not a substitute for it. A
 * Windows toast reaches somebody working in another window, which is exactly
 * when they would otherwise miss the meeting; it also disappears into a
 * notification centre, cannot carry a Join button that does the right thing,
 * and looks like Windows. This one can be acted on: join the call, or start
 * recording it, in one click from wherever they are in the app.
 *
 * Mounted at the app root rather than on the Meetings page, because a reminder
 * that only appears on the page you were already looking at reminds nobody.
 *
 * Dismissal is local and final for that reminder: the backend fires each
 * bucket once and never re-notifies, so there is nothing to tell it.
 */
export const ReminderToasts: React.FC<ReminderToastsProps> = ({ onStartMeeting }) => {
  const [reminders, setReminders] = React.useState<MeetingReminder[]>([]);

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

  const dismiss = React.useCallback((key: string) => {
    setReminders((current) => current.filter((reminder) => reminder.key !== key));
  }, []);

  if (reminders.length === 0) return null;

  return (
    <div
      className="fixed bottom-4 right-4 z-50 flex flex-col gap-2 w-80 max-w-[calc(100vw-2rem)]"
      role="region"
      aria-label="Meeting reminders"
    >
      {reminders.map((reminder) => (
        <ReminderCard
          key={reminder.key}
          reminder={reminder}
          onDismiss={() => dismiss(reminder.key)}
          onStartMeeting={onStartMeeting}
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
  onStartMeeting: () => void;
}> = ({ reminder, onDismiss, onStartMeeting }) => {
  const minutes = useCountdown(reminder.start, reminder.minutes_until);

  React.useEffect(() => {
    const timer = window.setTimeout(onDismiss, DISMISS_AFTER_MS);
    return () => window.clearTimeout(timer);
  }, [onDismiss]);

  const when =
    minutes > 1
      ? `in ${minutes} minutes`
      : minutes === 1
        ? 'in 1 minute'
        : minutes === 0
          ? 'now'
          : `${-minutes} minute${minutes === -1 ? '' : 's'} ago`;

  return (
    <div className="rounded-lg border border-border bg-card shadow-lg p-3 animate-in slide-in-from-bottom-2">
      <div className="flex items-start gap-2">
        <Bell className="w-3.5 h-3.5 text-primary shrink-0 mt-0.5" />
        <div className="min-w-0 flex-1">
          <p className="text-sm font-medium text-foreground leading-snug">{reminder.title}</p>
          <p className="text-[11px] text-muted-foreground mt-0.5">
            {minutes < 0 ? `Started ${when}` : `Starts ${when}`}
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
        {reminder.conference_url && (
          <Button
            size="sm"
            onClick={() => {
              void calendar.openCalendarLink(reminder.conference_url as string).catch(() => undefined);
              onDismiss();
            }}
            className="h-7 gap-1.5 text-xs"
          >
            <Video className="w-3.5 h-3.5" />
            Join
          </Button>
        )}
        <Button
          size="sm"
          variant="outline"
          onClick={() => {
            onStartMeeting();
            onDismiss();
          }}
          className="h-7 gap-1.5 text-xs"
        >
          <Radio className="w-3.5 h-3.5" />
          Record
        </Button>
      </div>
    </div>
  );
};
