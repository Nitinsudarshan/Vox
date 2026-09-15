import React from 'react';
import { ChevronLeft, ChevronRight, FileText, Loader2, RefreshCw, Video } from 'lucide-react';

import { Button } from '@/components/ui/button';
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
} from '@/components/ui/dialog';
import { accountColor, eventTime, isDeclined } from '@/lib/calendar';
import type { CalendarAccount, CalendarEvent, DayAgenda } from '@/types/calendar';
import { AccountLegend } from './AccountLegend';

/** Days in the grid. A week is the unit people plan in. */
const WEEK_DAYS = 7;

interface CalendarDialogProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  agenda: DayAgenda[];
  accounts: CalendarAccount[];
  syncing: boolean;
  onSync: () => void;
  /** Opens one meeting in full, the same dialog the agenda opens. */
  onOpenEvent: (event: CalendarEvent) => void;
  /** Fixed "now" for tests; the live clock otherwise. */
  now?: Date;
}

/**
 * The whole calendar, a week at a time.
 *
 * The agenda on the meetings page answers "what is next"; this answers "what
 * does the week look like", which is a different question and the reason the
 * two are not the same surface. Every connected account is in here at once,
 * told apart by the colour on each event's left edge and named in the key —
 * work and life merged into one week is the entire point of connecting more
 * than one calendar.
 *
 * Read-only, like everything else Vox does with a calendar: this shows a week
 * and opens links, and there is nothing in it that writes to Google.
 */
export const CalendarDialog: React.FC<CalendarDialogProps> = ({
  open,
  onOpenChange,
  agenda,
  accounts,
  syncing,
  onSync,
  onOpenEvent,
  now,
}) => {
  const today = React.useMemo(() => startOfDay(now ?? new Date()), [now]);
  const [weekStart, setWeekStart] = React.useState(() => startOfWeek(today));

  // Opening the dialog lands on the current week, whatever week it was left
  // on. Coming back a day later to last Monday is not what anybody meant.
  React.useEffect(() => {
    if (open) setWeekStart(startOfWeek(today));
  }, [open, today]);

  const connectedAccounts = React.useMemo(
    () => accounts.filter((account) => account.enabled).map((account) => account.email),
    [accounts],
  );

  const byDate = React.useMemo(() => {
    const map = new Map<string, CalendarEvent[]>();
    for (const day of agenda ?? []) map.set(day.date, day.events);
    return map;
  }, [agenda]);

  const days = React.useMemo(
    () =>
      Array.from({ length: WEEK_DAYS }, (_, offset) => {
        const date = new Date(
          weekStart.getFullYear(),
          weekStart.getMonth(),
          weekStart.getDate() + offset,
        );
        return { date, key: dateKey(date), events: byDate.get(dateKey(date)) ?? [] };
      }),
    [weekStart, byDate],
  );

  const shiftWeeks = (weeks: number) =>
    setWeekStart(
      (current) =>
        new Date(current.getFullYear(), current.getMonth(), current.getDate() + weeks * WEEK_DAYS),
    );

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="max-w-6xl w-[95vw] h-[85vh] flex flex-col p-0 gap-0">
        <DialogHeader className="shrink-0 px-5 py-4 border-b border-border">
          <DialogTitle>Calendar</DialogTitle>
          <div className="flex flex-wrap items-center justify-between gap-3 pt-2">
            <div className="flex items-center gap-1">
              <Button
                variant="ghost"
                size="icon"
                className="h-7 w-7"
                onClick={() => shiftWeeks(-1)}
                aria-label="Previous week"
              >
                <ChevronLeft className="w-4 h-4" />
              </Button>
              <Button
                variant="ghost"
                size="sm"
                className="h-7 text-xs"
                onClick={() => setWeekStart(startOfWeek(today))}
              >
                This week
              </Button>
              <Button
                variant="ghost"
                size="icon"
                className="h-7 w-7"
                onClick={() => shiftWeeks(1)}
                aria-label="Next week"
              >
                <ChevronRight className="w-4 h-4" />
              </Button>
              <span className="text-xs text-muted-foreground ml-2">{weekLabel(days)}</span>
            </div>

            <div className="flex items-center gap-3">
              <AccountLegend accounts={accounts} />
              <Button
                variant="ghost"
                size="sm"
                onClick={onSync}
                disabled={syncing}
                className="h-7 gap-1.5 text-xs text-muted-foreground"
              >
                {syncing ? (
                  <Loader2 className="w-3.5 h-3.5 animate-spin" />
                ) : (
                  <RefreshCw className="w-3.5 h-3.5" />
                )}
                {syncing ? 'Syncing…' : 'Sync'}
              </Button>
            </div>
          </div>
        </DialogHeader>

        <div className="flex-1 min-h-0 overflow-auto p-3">
          <div className="grid grid-cols-1 sm:grid-cols-2 lg:grid-cols-7 gap-2 min-h-full">
            {days.map((day) => (
              <section
                key={day.key}
                className={`rounded-lg border p-2 min-h-24 ${
                  day.key === dateKey(today)
                    ? 'border-primary/50 bg-primary/5'
                    : 'border-border'
                }`}
              >
                <h3 className="text-[11px] font-medium text-muted-foreground mb-2 flex items-baseline justify-between gap-1">
                  <span>
                    {day.date.toLocaleDateString(undefined, { weekday: 'short' })}{' '}
                    <span className="text-foreground">{day.date.getDate()}</span>
                  </span>
                  {day.events.length > 0 && (
                    <span className="tabular-nums">{day.events.length}</span>
                  )}
                </h3>
                <ul className="space-y-1.5">
                  {day.events.map((event) => (
                    <CalendarDayEvent
                      key={`${event.account_email}-${event.id}`}
                      event={event}
                      connectedAccounts={connectedAccounts}
                      onOpen={onOpenEvent}
                    />
                  ))}
                </ul>
              </section>
            ))}
          </div>
        </div>
      </DialogContent>
    </Dialog>
  );
};

interface CalendarDayEventProps {
  event: CalendarEvent;
  connectedAccounts: string[];
  onOpen: (event: CalendarEvent) => void;
}

/**
 * One event, as a block in its day's column.
 *
 * A single target, like the agenda row: the block is the meeting, and
 * everything you can do with it is in the dialog behind it. A 120-pixel-wide
 * column is no place for three competing buttons.
 */
const CalendarDayEvent: React.FC<CalendarDayEventProps> = ({
  event,
  connectedAccounts,
  onOpen,
}) => {
  const colour = accountColor(event.account_email, connectedAccounts);
  const declined = isDeclined(event);

  return (
    <li>
      <button
        type="button"
        onClick={() => onOpen(event)}
        title={`${event.title} — ${event.account_email}`}
        className={`w-full text-left rounded-md border-l-2 px-2 py-1.5 transition-opacity hover:opacity-80 cursor-pointer ${colour.border} ${colour.soft}`}
      >
        <p className="text-[10px] font-mono text-muted-foreground tabular-nums">
          {eventTime(event)}
        </p>
        <p
          className={`text-[11px] leading-snug ${
            declined ? 'line-through text-muted-foreground' : 'text-foreground'
          }`}
        >
          {event.title}
        </p>
        <span className="sr-only">{`From ${event.account_email}.`}</span>
        {(event.meeting_id || event.conference_url) && (
          <span className="flex items-center gap-1.5 mt-1 text-muted-foreground/70">
            {event.meeting_id && <FileText className="w-3 h-3" aria-label="Has notes" />}
            {event.conference_url && <Video className="w-3 h-3" aria-label="Video call" />}
          </span>
        )}
      </button>
    </li>
  );
};

function startOfDay(when: Date): Date {
  return new Date(when.getFullYear(), when.getMonth(), when.getDate());
}

/**
 * The Monday of `when`'s week.
 *
 * Monday rather than Sunday because a work week is what a meetings calendar is
 * read for, and putting the weekend at both ends of the grid splits it.
 */
function startOfWeek(when: Date): Date {
  const day = when.getDay();
  const backToMonday = (day + 6) % WEEK_DAYS;
  return new Date(when.getFullYear(), when.getMonth(), when.getDate() - backToMonday);
}

/** `YYYY-MM-DD` in the viewer's timezone, matching what the backend groups on. */
function dateKey(when: Date): string {
  return `${when.getFullYear()}-${String(when.getMonth() + 1).padStart(2, '0')}-${String(
    when.getDate(),
  ).padStart(2, '0')}`;
}

/** "8 – 14 Sept", or both months when the week straddles two. */
function weekLabel(days: Array<{ date: Date }>): string {
  if (days.length === 0) return '';
  const first = days[0].date;
  const last = days[days.length - 1].date;
  const month = (value: Date) => value.toLocaleDateString(undefined, { month: 'short' });
  const year =
    first.getFullYear() === last.getFullYear() ? `${first.getFullYear()}` : '';
  if (first.getMonth() === last.getMonth()) {
    return `${first.getDate()} – ${last.getDate()} ${month(first)} ${year}`.trim();
  }
  return `${first.getDate()} ${month(first)} – ${last.getDate()} ${month(last)} ${year}`.trim();
}
