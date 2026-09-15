import React from 'react';
import { Loader2 } from 'lucide-react';

import { agendaDayLabel, isPast, isTodayOrLater } from '@/lib/calendar';
import type { CalendarAccount, CalendarEvent, DayAgenda } from '@/types/calendar';
import { AccountLegend } from './AccountLegend';
import { AgendaEventRow } from './AgendaEventRow';

/** Days of the agenda shown inline. The rest is what the calendar view is for. */
const DAYS_SHOWN = 2;

interface AgendaSectionProps {
  agenda: DayAgenda[];
  accounts: CalendarAccount[];
  syncing: boolean;
  /** Opens one meeting in full. The row is a single target. */
  onOpenEvent: (event: CalendarEvent) => void;
  /** Fixed "now" for tests; the live clock otherwise. */
  now?: Date;
}

/**
 * The day, across every connected calendar.
 *
 * Two days, from today — including the ones that have already finished, dimmed
 * rather than dropped. A schedule that deletes a meeting the moment it ends is
 * a schedule you cannot check yourself against: "did the 10:30 happen, and did
 * I record it?" is asked at 11, and the row that answers it had just
 * disappeared. Finished rows keep their join link too, which is what a call
 * that resumes on the same link needs.
 *
 * Everything before today is cut: the cache holds a month of history so
 * recordings can be matched to meetings, and none of it belongs above today's
 * schedule. Everything further out lives in the calendar view, which is a
 * calendar rather than a preview.
 */
export const AgendaSection: React.FC<AgendaSectionProps> = ({
  agenda,
  accounts,
  syncing,
  onOpenEvent,
  now,
}) => {
  const connectedAccounts = React.useMemo(
    () => accounts.filter((account) => account.enabled).map((account) => account.email),
    [accounts],
  );

  const days = React.useMemo(() => {
    const clock = now ?? new Date();
    return (agenda ?? [])
      .filter((day) => isTodayOrLater(day.date, clock) && day.events.length > 0)
      .slice(0, DAYS_SHOWN);
  }, [agenda, now]);

  if (days.length === 0) return null;

  return (
    <section className="mb-8">
      <div className="flex items-center justify-between gap-3 mb-2">
        <h2 className="text-xs font-semibold text-muted-foreground uppercase tracking-wide flex items-center gap-1.5">
          Your day
          {syncing && <Loader2 className="w-3 h-3 animate-spin" />}
        </h2>
        <AccountLegend accounts={accounts} />
      </div>

      <div className="rounded-xl border border-border divide-y divide-border">
        {days.map((day) => (
          <div key={day.date} className="p-3">
            <p className="text-[11px] font-medium text-muted-foreground mb-2">
              {agendaDayLabel(day.date, now)}
            </p>
            <ul className="space-y-2">
              {day.events.map((event) => (
                <AgendaEventRow
                  key={`${event.account_email}-${event.id}`}
                  event={event}
                  connectedAccounts={connectedAccounts}
                  done={isPast(event, now ?? new Date())}
                  onOpen={onOpenEvent}
                />
              ))}
            </ul>
          </div>
        ))}
      </div>
    </section>
  );
};
