import React from 'react';
import { ChevronDown, ChevronRight, FileAudio, Mic } from 'lucide-react';

import { formatDuration, formatClockTime } from '@/lib/meetings';
import type { MeetingListItem } from '@/types/meetings';

interface MeetingDayGroupProps {
  label: string;
  items: MeetingListItem[];
  /** Whether the day starts open. Recent days do; older ones are a heading. */
  defaultOpen: boolean;
  onSelect: (meetingId: string) => void;
}

/**
 * One day of recorded meetings, open or collapsed.
 *
 * A collapsed day is a heading and a count, which is the useful thing an old
 * day says: "there were four of these". The list used to render every day it
 * had ever recorded fully expanded, so last month's four-meeting Tuesday cost
 * the same vertical space as today and pushed today off the screen.
 *
 * The count is on the collapsed row rather than on every row, because on an
 * open day you can see how many there are.
 */
export const MeetingDayGroup: React.FC<MeetingDayGroupProps> = ({
  label,
  items,
  defaultOpen,
  onSelect,
}) => {
  const [open, setOpen] = React.useState(defaultOpen);

  // Follows the default when the list re-groups around it — a search that
  // narrows to one old meeting should not hide it behind a closed heading.
  React.useEffect(() => setOpen(defaultOpen), [defaultOpen]);

  const heading = (
    <h2 className="text-xs font-semibold text-muted-foreground uppercase tracking-wide">
      {label}
    </h2>
  );

  if (defaultOpen) {
    return (
      <section className="mb-6">
        <div className="mb-1.5">{heading}</div>
        <MeetingRows items={items} onSelect={onSelect} />
      </section>
    );
  }

  return (
    <section className="mb-4">
      <button
        type="button"
        onClick={() => setOpen((current) => !current)}
        aria-expanded={open}
        className="w-full flex items-center gap-2 rounded-lg px-3 py-1.5 -mx-3 hover:bg-accent/60 transition-colors cursor-pointer"
      >
        {open ? (
          <ChevronDown className="w-3.5 h-3.5 text-muted-foreground shrink-0" />
        ) : (
          <ChevronRight className="w-3.5 h-3.5 text-muted-foreground shrink-0" />
        )}
        {heading}
        <span className="text-[11px] text-muted-foreground tabular-nums">
          {items.length} {items.length === 1 ? 'meeting' : 'meetings'}
        </span>
      </button>
      {open && (
        <div className="mt-1.5">
          <MeetingRows items={items} onSelect={onSelect} />
        </div>
      )}
    </section>
  );
};

/** The day's meetings, one row each. */
const MeetingRows: React.FC<{
  items: MeetingListItem[];
  onSelect: (meetingId: string) => void;
}> = ({ items, onSelect }) => (
  <ul>
    {items.map((meeting) => (
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
          {/* The two states worth knowing before opening a meeting, and only
              those two: speech that was never transcribed, and a recording
              that caught one side of the call. Both are reasons the report
              will be wrong, which is not something to discover after reading
              it. Everything else that used to sit here — the report badge, the
              transcribing badge, the source icon repeated as a label — said
              nothing the row did not already show. */}
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
              {formatClockTime(meeting.created_at)} · {formatDuration(meeting.duration_seconds)}
            </span>
          </span>
        </button>
      </li>
    ))}
  </ul>
);
