import React from 'react';
import { FileAudio, Mic, Search, Import, Video, CalendarDays, Loader2 } from 'lucide-react';

import { Button } from '@/components/ui/button';
import { Input } from '@/components/ui/input';
import { formatDuration, formatClockTime, groupByDay } from '@/lib/meetings';
import { agendaDayLabel, attendeeSummary, eventTime, isPast } from '@/lib/calendar';
import type { MeetingListItem } from '@/types/meetings';
import type { DayAgenda } from '@/types/calendar';

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
  agendaSyncing: boolean;
  /** Opens Settings › Calendar, when nothing is connected yet. */
  onConnectCalendar?: () => void;
}

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
  agendaSyncing,
  onConnectCalendar,
}) => {
  const days = React.useMemo(() => groupByDay(meetings), [meetings]);

  // Only days that still have something unfinished on them. A calendar is
  // useful here for what is about to happen; what already happened is the
  // recording list below, which is the real record.
  const upcoming = React.useMemo(() => {
    const now = new Date();
    return (agenda ?? [])
      .map((day) => ({ ...day, events: day.events.filter((event) => !isPast(event, now)) }))
      .filter((day) => day.events.length > 0)
      .slice(0, 2);
  }, [agenda]);

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
      </div>

      <div className="flex-1 min-h-0 overflow-y-auto">
        {upcoming.length > 0 && (
          <section className="mb-8">
            <h2 className="text-xs font-semibold text-muted-foreground uppercase tracking-wide mb-2 flex items-center gap-1.5">
              Coming up
              {agendaSyncing && <Loader2 className="w-3 h-3 animate-spin" />}
            </h2>
            <div className="rounded-xl border border-border divide-y divide-border">
              {upcoming.map((day) => (
                <div key={day.date} className="p-3">
                  <p className="text-[11px] font-medium text-muted-foreground mb-2">
                    {agendaDayLabel(day.date)}
                  </p>
                  <ul className="space-y-2">
                    {day.events.map((event) => {
                      const guests = attendeeSummary(event);
                      return (
                        <li
                          key={`${event.account_email}-${event.id}`}
                          className="flex items-baseline gap-3"
                        >
                          <span className="text-[11px] font-mono text-muted-foreground tabular-nums w-16 shrink-0">
                            {eventTime(event)}
                          </span>
                          <span className="min-w-0 flex-1">
                            <span className="text-sm text-foreground">{event.title}</span>
                            {guests && (
                              <span className="text-[11px] text-muted-foreground ml-2">
                                {guests}
                              </span>
                            )}
                          </span>
                          {event.meeting_id ? (
                            <button
                              type="button"
                              onClick={() => onSelect(event.meeting_id as string)}
                              className="text-[11px] text-primary hover:underline shrink-0 cursor-pointer"
                            >
                              Notes
                            </button>
                          ) : (
                            event.conference_url && (
                              <Video className="w-3.5 h-3.5 text-muted-foreground shrink-0" />
                            )
                          )}
                        </li>
                      );
                    })}
                  </ul>
                </div>
              ))}
            </div>
          </section>
        )}

        {loading ? (
          <p className="text-sm text-muted-foreground py-10 text-center">Loading meetings…</p>
        ) : meetings.length === 0 ? (
          <EmptyState
            searching={query.trim().length > 0}
            calendarConnected={(agenda ?? []).length > 0}
            onConnectCalendar={onConnectCalendar}
          />
        ) : (
          days.map((day) => (
            <section key={day.label} className="mb-6">
              <h2 className="text-xs font-semibold text-muted-foreground uppercase tracking-wide mb-1.5">
                {day.label}
              </h2>
              <ul>
                {day.items.map((meeting) => (
                  <li key={meeting.id}>
                    <button
                      type="button"
                      onClick={() => onSelect(meeting.id)}
                      className="w-full text-left flex items-baseline gap-3 rounded-lg px-3 py-2.5 -mx-3 hover:bg-accent/60 transition-colors cursor-pointer"
                    >
                      <span className="shrink-0 pt-0.5">
                        {meeting.source === 'imported' ? (
                          <FileAudio className="w-3.5 h-3.5 text-muted-foreground" />
                        ) : (
                          <Mic className="w-3.5 h-3.5 text-muted-foreground" />
                        )}
                      </span>
                      <span className="min-w-0 flex-1">
                        <span className="block text-sm font-medium text-foreground truncate">
                          {meeting.title}
                        </span>
                        <span className="block text-[11px] text-muted-foreground truncate mt-0.5">
                          {meeting.preview || `${meeting.segment_count} lines`}
                        </span>
                      </span>
                      {/* The two states worth knowing before opening a
                          meeting, and only those two: speech that was never
                          transcribed, and a recording that caught one side of
                          the call. Both are reasons the report will be wrong,
                          which is not something to discover after reading it.
                          Everything else that used to sit here — the report
                          badge, the transcribing badge, the source icon
                          repeated as a label — said nothing the row did not
                          already show. */}
                      <span className="shrink-0 flex items-baseline gap-2 text-[11px] tabular-nums">
                        {meeting.dropped_segments > 0 && (
                          <span className="text-amber-600 dark:text-amber-400">
                            {meeting.dropped_segments} lost
                          </span>
                        )}
                        {!meeting.system_audio_captured && meeting.source === 'recorded' && (
                          <span className="text-muted-foreground/70">Mic only</span>
                        )}
                        <span className="text-muted-foreground">
                          {formatClockTime(meeting.created_at)} ·{' '}
                          {formatDuration(meeting.duration_seconds)}
                        </span>
                      </span>
                    </button>
                  </li>
                ))}
              </ul>
            </section>
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
