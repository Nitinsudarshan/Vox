import React from 'react';
import { Loader2 } from 'lucide-react';

import { agendaDayLabel, isPast } from '@/lib/calendar';
import type { CalendarAccount, CalendarEvent, DayAgenda } from '@/types/calendar';
import { AccountLegend } from './AccountLegend';
import { AgendaEventRow } from './AgendaEventRow';

/** Days of the agenda shown inline. The rest is what the calendar view is for. */
const DAYS_SHOWN = 2;

interface AgendaSectionProps {
  agenda: DayAgenda[];
  accounts: CalendarAccount[];
  syncing: boolean;
  onOpenNotes: (meetingId: string) => void;
  onJoin: (event: CalendarEvent) => void;
  /** Fixed "now" for tests; the live clock otherwise. */
  now?: Date;
}

/**
 * What is coming up, across every connected calendar.
 *
 * Deliberately short: two days, and only what has not finished. A calendar is
 * useful on this page for what is about to happen — what already happened is
 * the recording list underneath, which is the real record. Everything further
 * out lives in the calendar view, which is a calendar rather than a preview.
 */
export const AgendaSection: React.FC<AgendaSectionProps> = ({
  agenda,
  accounts,
  syncing,
  onOpenNotes,
  onJoin,
  now,
}) => {
  const connectedAccounts = React.useMemo(
    () => accounts.filter((account) => account.enabled).map((account) => account.email),
    [accounts],
  );

  const upcoming = React.useMemo(() => {
    const clock = now ?? new Date();
    return (agenda ?? [])
      .map((day) => ({ ...day, events: day.events.filter((event) => !isPast(event, clock)) }))
      .filter((day) => day.events.length > 0)
      .slice(0, DAYS_SHOWN);
  }, [agenda, now]);

  if (upcoming.length === 0) return null;

  return (
    <section className="mb-8">
      <div className="flex items-center justify-between gap-3 mb-2">
        <h2 className="text-xs font-semibold text-muted-foreground uppercase tracking-wide flex items-center gap-1.5">
          Coming up
          {syncing && <Loader2 className="w-3 h-3 animate-spin" />}
        </h2>
        <AccountLegend accounts={accounts} />
      </div>

      <div className="rounded-xl border border-border divide-y divide-border">
        {upcoming.map((day) => (
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
                  onOpenNotes={onOpenNotes}
                  onJoin={onJoin}
                  now={now}
                />
              ))}
            </ul>
          </div>
        ))}
      </div>
    </section>
  );
};
