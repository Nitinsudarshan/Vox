import React from 'react';
import { ChevronDown, ChevronRight, MapPin, Video } from 'lucide-react';

import {
  accountColor,
  attendeeSummary,
  canJoinEvent,
  eventTime,
  isDeclined,
  joinUnavailableReason,
} from '@/lib/calendar';
import type { CalendarEvent } from '@/types/calendar';

interface AgendaEventRowProps {
  event: CalendarEvent;
  /** Connected account addresses, in order, so the colour is stable. */
  connectedAccounts: string[];
  /** Whether this meeting has already finished. Dims the row rather than hiding it. */
  done?: boolean;
  /** Opens the recording Vox already has for this event. */
  onOpenNotes: (meetingId: string) => void;
  /** Opens the video link in the browser. */
  onJoin: (event: CalendarEvent) => void;
  /** Fixed "now" for tests; the live clock otherwise. */
  now?: Date;
}

/**
 * One calendar event, as a row in the agenda.
 *
 * Four things this row says that the version before it did not, each of which
 * was a question its reader had to leave Vox to answer:
 *
 * - **Which account it came from**, as a colour. With two calendars connected
 *   every row looked alike, and "is this the work one?" is the first question
 *   a merged agenda raises.
 * - **Whether you said no.** A declined invitation is struck through, the way
 *   Google Calendar draws it — and stays joinable, because declining a series
 *   and dropping into one of its meetings is a normal thing to do.
 * - **Whether it has already happened.** A finished meeting stays on the day,
 *   dimmed. Deleting it the moment it ends makes "did the 10:30 happen, and did
 *   I record it?" unanswerable at 11, which is exactly when it is asked.
 * - **Whether you can join from here**, as a button that is only live when
 *   pressing it would take you somewhere real.
 * - **What the invitation actually says**, on demand.
 */
export const AgendaEventRow: React.FC<AgendaEventRowProps> = ({
  event,
  connectedAccounts,
  done = false,
  onOpenNotes,
  onJoin,
  now,
}) => {
  const [expanded, setExpanded] = React.useState(false);

  const colour = accountColor(event.account_email, connectedAccounts);
  const guests = attendeeSummary(event);
  const declined = isDeclined(event);
  const joinable = canJoinEvent(event, now);
  const details = event.description?.trim() || '';
  const hasDetails = details.length > 0 || Boolean(event.location);

  return (
    // Dimmed rather than hidden or struck through: a finished meeting is still
    // a fact about the day, and the strike-through already means "declined".
    <li className={done ? 'opacity-55' : undefined}>
      <div className="flex items-baseline gap-3">
        <span className="text-[11px] font-mono text-muted-foreground tabular-nums w-16 shrink-0">
          {eventTime(event)}
        </span>

        <span
          className={`w-1.5 h-1.5 rounded-full shrink-0 self-center ${colour.dot}`}
          title={event.account_email}
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
          {declined && (
            <span className="text-[11px] text-muted-foreground ml-2">Declined</span>
          )}
          {guests && <span className="text-[11px] text-muted-foreground ml-2">{guests}</span>}
          {/* The address is the only thing that names the account for a
              screen reader — a coloured dot says nothing out loud. */}
          <span className="sr-only">{` from ${event.account_email}`}</span>
        </span>

        <span className="shrink-0 flex items-center gap-2">
          {hasDetails && (
            <button
              type="button"
              onClick={() => setExpanded((open) => !open)}
              aria-expanded={expanded}
              aria-label={`${expanded ? 'Hide' : 'Show'} details for ${event.title}`}
              className="text-muted-foreground hover:text-foreground transition-colors cursor-pointer"
            >
              {expanded ? (
                <ChevronDown className="w-3.5 h-3.5" />
              ) : (
                <ChevronRight className="w-3.5 h-3.5" />
              )}
            </button>
          )}

          {event.meeting_id && (
            <button
              type="button"
              onClick={() => onOpenNotes(event.meeting_id as string)}
              className="text-[11px] text-primary hover:underline cursor-pointer"
            >
              Notes
            </button>
          )}

          {event.conference_url &&
            (joinable ? (
              <button
                type="button"
                onClick={() => onJoin(event)}
                className="inline-flex items-center gap-1 text-[11px] font-medium text-primary hover:underline cursor-pointer"
              >
                <Video className="w-3.5 h-3.5" />
                Join
              </button>
            ) : (
              // Not a disabled button: there is nothing to press and nothing
              // to wait for, so this is a mark saying the meeting has a link,
              // with the reason it is not live on hover.
              <span
                className="inline-flex items-center gap-1 text-[11px] text-muted-foreground/70"
                title={joinUnavailableReason(event, now)}
              >
                <Video className="w-3.5 h-3.5" />
              </span>
            ))}
        </span>
      </div>

      {expanded && (
        <div className="mt-1.5 ml-[4.75rem] mr-2 rounded-lg border border-border bg-muted/40 px-3 py-2 space-y-1.5">
          {event.location && (
            <p className="flex items-start gap-1.5 text-[11px] text-muted-foreground">
              <MapPin className="w-3 h-3 shrink-0 mt-0.5" />
              <span>{event.location}</span>
            </p>
          )}
          {details && (
            // Plain text, deliberately. The Rust side flattens Google's HTML
            // on the way in (`rules/untrusted-input.md`): an invitation
            // somebody else wrote is shown, never interpreted.
            <p className="text-[11px] text-muted-foreground whitespace-pre-wrap leading-relaxed">
              {details}
            </p>
          )}
        </div>
      )}
    </li>
  );
};
