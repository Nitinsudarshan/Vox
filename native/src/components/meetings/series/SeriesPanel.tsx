import React from 'react';
import { CircleDot, FileText, Loader2, Repeat } from 'lucide-react';

import { formatDuration } from '@/lib/meetings';
import type { SeriesOccurrence } from '@/types/meetings';

interface SeriesPanelProps {
  /** The series' name, as it should be shown. */
  title: string;
  occurrences: SeriesOccurrence[];
  loading: boolean;
  /** The meeting currently open, so it can be marked rather than linked. */
  currentMeetingId: string;
  onOpen: (meetingId: string) => void;
}

/**
 * Every meeting in this series, oldest first.
 *
 * Forwards, deliberately, against the meetings list's own newest-first order:
 * a list of what happened lately reads backwards, and a series reads forwards
 * — "what did we decide in week one, and what has changed since" starts at
 * week one.
 *
 * What each row carries is what tells you whether it is worth opening: when it
 * was, how long it ran, and whether there is a report to read. A row with no
 * report is a meeting whose transcript was never turned into anything, which
 * is the most useful thing to know about it from here.
 */
export const SeriesPanel: React.FC<SeriesPanelProps> = ({
  title,
  occurrences,
  loading,
  currentMeetingId,
  onOpen,
}) => (
  <div className="rounded-lg border border-border bg-card p-3">
    <div className="flex items-center gap-2 mb-2">
      <Repeat className="w-3.5 h-3.5 text-muted-foreground shrink-0" />
      <h3 className="text-xs font-semibold text-foreground">{title}</h3>
      <span className="text-[11px] text-muted-foreground tabular-nums">
        {occurrences.length} {occurrences.length === 1 ? 'meeting' : 'meetings'}
      </span>
      {loading && <Loader2 className="w-3 h-3 animate-spin text-muted-foreground" />}
    </div>

    {!loading && occurrences.length === 0 ? (
      <p className="text-[11px] text-muted-foreground">
        Nothing else has been recorded in this series yet.
      </p>
    ) : (
      <ol className="space-y-0.5">
        {occurrences.map((occurrence, index) => {
          const current = occurrence.meeting_id === currentMeetingId;
          const when = new Date(occurrence.created_at);
          return (
            <li key={occurrence.meeting_id}>
              <button
                type="button"
                onClick={() => onOpen(occurrence.meeting_id)}
                disabled={current}
                aria-current={current ? 'true' : undefined}
                className={`w-full text-left flex items-baseline gap-2 rounded-md px-2 py-1 -mx-1 text-[11px] transition-colors ${
                  current
                    ? 'bg-accent/60 text-foreground cursor-default'
                    : 'text-muted-foreground hover:bg-accent/40 hover:text-foreground cursor-pointer'
                }`}
              >
                <span className="tabular-nums w-5 shrink-0 text-right">{index + 1}</span>
                <span className="shrink-0">
                  {current ? (
                    <CircleDot className="w-3 h-3" />
                  ) : (
                    <span className="inline-block w-3" />
                  )}
                </span>
                <span className="flex-1 min-w-0 truncate">
                  {Number.isNaN(when.getTime())
                    ? occurrence.title
                    : when.toLocaleDateString(undefined, {
                        weekday: 'short',
                        day: 'numeric',
                        month: 'short',
                      })}
                </span>
                <span className="shrink-0 tabular-nums">
                  {formatDuration(occurrence.duration_seconds)}
                </span>
                <span className="shrink-0 w-3.5">
                  {occurrence.has_summary && (
                    <FileText className="w-3 h-3" aria-label="Has a report" />
                  )}
                </span>
              </button>
            </li>
          );
        })}
      </ol>
    )}
  </div>
);
