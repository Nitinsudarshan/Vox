import React from 'react';
import { CalendarDays, MapPin, Radio, Users, Video } from 'lucide-react';

import { Button } from '@/components/ui/button';
import { Dialog, DialogContent, DialogHeader, DialogTitle } from '@/components/ui/dialog';
import {
  accountColor,
  canJoinEvent,
  eventDuration,
  eventTime,
  isDeclined,
  joinUnavailableReason,
} from '@/lib/calendar';
import type { Attendance, CalendarEvent } from '@/types/calendar';

/** How each answer to an invitation reads. */
const ATTENDANCE_LABELS: Record<Attendance, string | null> = {
  accepted: 'Going',
  declined: 'Not going',
  tentative: 'Maybe',
  needs_action: 'Not answered',
  // Their own event, or one with no guest list. Saying "unknown" about a
  // meeting somebody put in their own calendar helps nobody.
  unknown: null,
};

interface EventDetailsDialogProps {
  event: CalendarEvent | null;
  onOpenChange: (open: boolean) => void;
  /** Connected account addresses, in order, so the colour is stable. */
  connectedAccounts: string[];
  onJoin: (event: CalendarEvent) => void;
  /** Opens the call and starts recording it. */
  onJoinAndRecord: (event: CalendarEvent) => void;
  /** Opens the recording Vox already has for this event. */
  onOpenNotes: (meetingId: string) => void;
  /** Fixed "now" for tests; the live clock otherwise. */
  now?: Date;
}

/**
 * One meeting, in full.
 *
 * The agenda row used to carry the actions itself, which made it a row of
 * small targets: a chevron that expanded the notes, a Join, a Notes, each
 * three millimetres wide and none of them the obvious thing to press. The row
 * is now one target — the meeting — and everything about it lives here, where
 * there is room for the guest list to be a list rather than "+14".
 */
export const EventDetailsDialog: React.FC<EventDetailsDialogProps> = ({
  event,
  onOpenChange,
  connectedAccounts,
  onJoin,
  onJoinAndRecord,
  onOpenNotes,
  now,
}) => {
  if (!event) return null;

  const colour = accountColor(event.account_email, connectedAccounts);
  const joinable = canJoinEvent(event, now);
  const declined = isDeclined(event);
  const attendance = ATTENDANCE_LABELS[event.attendance];
  const duration = eventDuration(event);
  const guests = event.attendees.filter((attendee) => !attendee.is_self);
  const details = event.description?.trim() || '';
  const when = new Date(event.start);

  return (
    <Dialog open onOpenChange={onOpenChange}>
      <DialogContent className="max-w-lg max-h-[85vh] flex flex-col">
        <DialogHeader className="shrink-0">
          <DialogTitle
            className={declined ? 'line-through text-muted-foreground' : undefined}
          >
            {event.title}
          </DialogTitle>
          <div className="flex flex-wrap items-center gap-x-2 gap-y-1 pt-1.5 text-xs text-muted-foreground">
            <span className="inline-flex items-center gap-1.5">
              <CalendarDays className="w-3.5 h-3.5" />
              {Number.isNaN(when.getTime())
                ? event.start
                : when.toLocaleDateString(undefined, {
                    weekday: 'long',
                    day: 'numeric',
                    month: 'long',
                  })}
            </span>
            <span aria-hidden>·</span>
            <span>
              {eventTime(event)}
              {duration && ` · ${duration}`}
            </span>
            {attendance && (
              <>
                <span aria-hidden>·</span>
                <span className={declined ? 'text-destructive' : undefined}>{attendance}</span>
              </>
            )}
          </div>
          <p className="flex items-center gap-1.5 pt-1 text-[11px] text-muted-foreground">
            <span className={`w-1.5 h-1.5 rounded-full ${colour.dot}`} aria-hidden />
            {event.account_email}
          </p>
        </DialogHeader>

        <div className="flex-1 min-h-0 overflow-y-auto space-y-3 pr-1">
          {event.location && (
            <p className="flex items-start gap-2 text-xs text-foreground">
              <MapPin className="w-3.5 h-3.5 shrink-0 mt-0.5 text-muted-foreground" />
              <span>{event.location}</span>
            </p>
          )}

          {guests.length > 0 && (
            <div>
              <p className="flex items-center gap-2 text-xs font-medium text-foreground mb-1.5">
                <Users className="w-3.5 h-3.5 text-muted-foreground" />
                {guests.length} {guests.length === 1 ? 'guest' : 'guests'}
              </p>
              {/* The whole list, not "+14". This is the surface with room
                  for it, and "who else is on this call" is most of why
                  somebody opens a meeting before joining it. */}
              <ul className="space-y-0.5 pl-5">
                {guests.map((guest) => (
                  <li
                    key={guest.email}
                    className="text-[11px] text-muted-foreground flex items-baseline gap-2"
                  >
                    <span className="truncate">
                      {guest.display_name?.trim() || guest.email}
                    </span>
                    {guest.organizer && (
                      <span className="text-muted-foreground/70 shrink-0">organiser</span>
                    )}
                    {guest.response === 'declined' && (
                      <span className="text-muted-foreground/70 shrink-0">declined</span>
                    )}
                  </li>
                ))}
              </ul>
            </div>
          )}

          {details && (
            // Plain text. The Rust side flattens Google's HTML on the way in
            // (`rules/untrusted-input.md`): an invitation somebody else wrote
            // is shown, never interpreted.
            <div>
              <p className="text-xs font-medium text-foreground mb-1">Notes on the invitation</p>
              <p className="text-[11px] text-muted-foreground whitespace-pre-wrap leading-relaxed">
                {details}
              </p>
            </div>
          )}
        </div>

        <div className="shrink-0 flex flex-wrap items-center gap-2 pt-1">
          {event.conference_url ? (
            <>
              <Button
                size="sm"
                onClick={() => onJoin(event)}
                disabled={!joinable}
                title={joinable ? undefined : joinUnavailableReason(event, now)}
                className="gap-1.5"
              >
                <Video className="w-3.5 h-3.5" />
                Join
              </Button>
              <Button
                size="sm"
                variant="outline"
                onClick={() => onJoinAndRecord(event)}
                disabled={!joinable}
                title={joinable ? undefined : joinUnavailableReason(event, now)}
                className="gap-1.5"
              >
                <Radio className="w-3.5 h-3.5" />
                Join and record
              </Button>
            </>
          ) : (
            <Button
              size="sm"
              variant="outline"
              onClick={() => onJoinAndRecord(event)}
              className="gap-1.5"
            >
              <Radio className="w-3.5 h-3.5" />
              Record
            </Button>
          )}

          {event.meeting_id && (
            <Button
              size="sm"
              variant="ghost"
              onClick={() => onOpenNotes(event.meeting_id as string)}
              className="text-xs"
            >
              Open notes
            </Button>
          )}

          {!joinable && event.conference_url && (
            <span className="text-[11px] text-muted-foreground">
              {joinUnavailableReason(event, now)}
            </span>
          )}
        </div>
      </DialogContent>
    </Dialog>
  );
};
