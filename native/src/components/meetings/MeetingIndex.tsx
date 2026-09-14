import React from 'react';
import { Search, Import, CalendarDays, RefreshCw, Loader2 } from 'lucide-react';

import { Button } from '@/components/ui/button';
import { Input } from '@/components/ui/input';
import { groupByDay } from '@/lib/meetings';
import { AgendaSection } from './calendar/AgendaSection';
import { MeetingDayGroup } from './MeetingDayGroup';
import type { MeetingListItem } from '@/types/meetings';
import type { CalendarAccount, CalendarEvent, DayAgenda } from '@/types/calendar';

interface MeetingIndexProps {
  meetings: MeetingListItem[];
  loading: boolean;
  query: string;
  onQueryChange: (value: string) => void;
  onSelect: (meetingId: string) => void;
  onImport: () => void;
  busy: boolean;
  /** Upcoming days from the connected calendars. Empty when none are. */
  agenda: DayAgenda[];
  /** The connected accounts, so each event can be coloured by the one it came from. */
  accounts: CalendarAccount[];
  agendaSyncing: boolean;
  /** Re-reads every connected calendar. */
  onSyncCalendars: () => void;
  /** Opens an event's video link in the browser. */
  onJoin: (event: CalendarEvent) => void;
  /** Opens Settings › Calendar, when nothing is connected yet. */
  onConnectCalendar?: () => void;
}

/**
 * Days of recordings left open. Everything older is a heading and a count.
 *
 * Two, because "today and yesterday" is the span a person is still working
 * inside. A list that renders every day it has ever recorded pushes today off
 * the screen by the second week.
 */
const DAYS_EXPANDED = 2;

/**
 * The meetings index: what is coming up, then what has been recorded.
 *
 * One column, full width, grouped by day. The version this replaces put a
 * 320-pixel list beside a detail panel and rendered both at once, so a
 * transcript was read through a column narrower than a phone while the list it
 * sat next to showed six rows. A list is for choosing; reading happens after
 * the choice, with the whole window.
 */
export const MeetingIndex: React.FC<MeetingIndexProps> = ({
  meetings,
  loading,
  query,
  onQueryChange,
  onSelect,
  onImport,
  busy,
  agenda,
  accounts,
  agendaSyncing,
  onSyncCalendars,
  onJoin,
  onConnectCalendar,
}) => {
  const days = React.useMemo(() => groupByDay(meetings), [meetings]);
  const hasCalendar = accounts.length > 0;

  return (
    <div className="flex flex-col min-h-0 h-full">
      <div className="shrink-0 flex items-center gap-2 pb-4">
        <div className="relative flex-1 max-w-md">
          <Search className="absolute left-2.5 top-1/2 -translate-y-1/2 w-3.5 h-3.5 text-muted-foreground" />
          <Input
            value={query}
            onChange={(event) => onQueryChange(event.target.value)}
            placeholder="Search meetings…"
            aria-label="Search meetings"
            className="h-9 text-sm pl-8"
          />
        </div>
        <Button
          variant="ghost"
          size="sm"
          onClick={onImport}
          disabled={busy}
          className="h-9 gap-2 text-xs text-muted-foreground"
        >
          <Import className="w-3.5 h-3.5" />
          Import
        </Button>
        {/* Here rather than only in Settings › Calendar. Noticing a meeting is
            missing happens on this page, and a trip through Settings to fix it
            is three clicks away from where the problem is. */}
        {hasCalendar && (
          <Button
            variant="ghost"
            size="sm"
            onClick={onSyncCalendars}
            disabled={agendaSyncing}
            className="h-9 gap-2 text-xs text-muted-foreground"
          >
            {agendaSyncing ? (
              <Loader2 className="w-3.5 h-3.5 animate-spin" />
            ) : (
              <RefreshCw className="w-3.5 h-3.5" />
            )}
            {agendaSyncing ? 'Syncing…' : 'Sync'}
          </Button>
        )}
      </div>

      <div className="flex-1 min-h-0 overflow-y-auto">
        <AgendaSection
          agenda={agenda}
          accounts={accounts}
          syncing={agendaSyncing}
          onOpenNotes={onSelect}
          onJoin={onJoin}
        />

        {loading ? (
          <p className="text-sm text-muted-foreground py-10 text-center">Loading meetings…</p>
        ) : meetings.length === 0 ? (
          <EmptyState
            searching={query.trim().length > 0}
            calendarConnected={(agenda ?? []).length > 0}
            onConnectCalendar={onConnectCalendar}
          />
        ) : (
          days.map((day, index) => (
            <MeetingDayGroup
              key={day.label}
              label={day.label}
              items={day.items}
              // A search is already a filter; collapsing its results behind a
              // heading would hide what the user just asked for.
              defaultOpen={index < DAYS_EXPANDED || query.trim().length > 0}
              onSelect={onSelect}
            />
          ))
        )}
      </div>
    </div>
  );
};

/** What the page says when there is nothing in it. */
const EmptyState: React.FC<{
  searching: boolean;
  calendarConnected: boolean;
  onConnectCalendar?: () => void;
}> = ({ searching, calendarConnected, onConnectCalendar }) => {
  if (searching) {
    return (
      <p className="text-sm text-muted-foreground py-10 text-center">
        No meetings match that search.
      </p>
    );
  }
  return (
    <div className="py-14 text-center">
      <p className="text-sm font-medium text-foreground">No meetings yet</p>
      <p className="text-xs text-muted-foreground mt-1">
        Record one, or import an existing recording.
      </p>
      {!calendarConnected && onConnectCalendar && (
        <Button
          variant="outline"
          size="sm"
          onClick={onConnectCalendar}
          className="mt-4 gap-2 text-xs"
        >
          <CalendarDays className="w-3.5 h-3.5" />
          Connect a calendar
        </Button>
      )}
    </div>
  );
};
