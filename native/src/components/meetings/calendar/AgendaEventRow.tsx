import React from 'react';
import { FileText, Video } from 'lucide-react';

import { accountColor, attendeeSummary, eventTime, isDeclined } from '@/lib/calendar';
import type { CalendarEvent } from '@/types/calendar';

interface AgendaEventRowProps {
  event: CalendarEvent;
  /** Connected account addresses, in order, so the colour is stable. */
  connectedAccounts: string[];
  /** Whether this meeting has already finished. Dims the row rather than hiding it. */
  done?: boolean;
  /** Opens everything about this meeting, and the actions that apply to it. */
  onOpen: (event: CalendarEvent) => void;
}

/**
 * One calendar event, as a row in the agenda.
 *
 * The whole row is one target. It used to carry its own actions — a chevron
 * that expanded the notes, a Join, a Notes — three small buttons in a line,
 * none of them the obvious thing to press and all of them competing with the
 * meeting's name for the click. The name is the thing people aim at, so the
 * name is what opens it, and the actions live in the dialog behind it where
 * there is room for them to be labelled.
 *
 * What stays on the row is what tells you whether to open it at all: when it
 * is, which calendar it came from (the colour), who else is on it, whether you
 * said no (struck through, the way Google draws it), whether it has already
 * happened (dimmed rather than dropped — "did the 10:30 happen?" is asked at
 * 11), and two marks: it has a video link, and Vox already has a recording
 * of it.
 */
export const AgendaEventRow: React.FC<AgendaEventRowProps> = ({
  event,
  connectedAccounts,
  done = false,
  onOpen,
}) => {
  const colour = accountColor(event.account_email, connectedAccounts);
  const guests = attendeeSummary(event);
  const declined = isDeclined(event);

  return (
    <li className={done ? 'opacity-55' : undefined}>
      <button
        type="button"
        onClick={() => onOpen(event)}
        className="w-full text-left flex items-baseline gap-3 rounded-lg px-2 py-1.5 -mx-2 hover:bg-accent/60 transition-colors cursor-pointer"
      >
        <span className="text-[11px] font-mono text-muted-foreground tabular-nums w-16 shrink-0">
          {eventTime(event)}
        </span>

        <span
          className={`w-1.5 h-1.5 rounded-full shrink-0 self-center ${colour.dot}`}
          aria-hidden
        />

        <span className="min-w-0 flex-1">
          <span
            className={`text-sm ${
              declined ? 'line-through text-muted-foreground' : 'text-foreground'
            }`}
          >
            {event.title}
          </span>
          {declined && <span className="text-[11px] text-muted-foreground ml-2">Declined</span>}
          {guests && <span className="text-[11px] text-muted-foreground ml-2">{guests}</span>}
          {/* The colour says which account out loud to nobody. */}
          <span className="sr-only">{` from ${event.account_email}`}</span>
        </span>

        <span className="shrink-0 flex items-center gap-2 text-muted-foreground/70">
          {event.meeting_id && <FileText className="w-3.5 h-3.5" aria-label="Has notes" />}
          {event.conference_url && <Video className="w-3.5 h-3.5" aria-label="Video call" />}
        </span>
      </button>
    </li>
  );
};
