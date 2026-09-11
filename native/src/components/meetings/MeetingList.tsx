import React from 'react';
import { Mic, FileAudio, AlertTriangle, Clock, Sparkles } from 'lucide-react';

import { Badge } from '@/components/ui/badge';
import { formatDuration } from '@/lib/meetings';
import type { MeetingListItem } from '@/types/meetings';

interface MeetingListProps {
  meetings: MeetingListItem[];
  selectedId: string | null;
  onSelect: (meetingId: string) => void;
  loading: boolean;
}

/** The meeting index: newest first, with enough on each row to pick one. */
export const MeetingList: React.FC<MeetingListProps> = ({
  meetings,
  selectedId,
  onSelect,
  loading,
}) => {
  if (loading) {
    return <p className="text-xs text-muted-foreground py-6 text-center">Loading meetings…</p>;
  }
  if (meetings.length === 0) {
    return (
      <div className="py-10 text-center">
        <p className="text-sm font-medium text-foreground">No meetings yet</p>
        <p className="text-xs text-muted-foreground mt-1">
          Record one, or import an existing recording.
        </p>
      </div>
    );
  }

  return (
    <ul className="space-y-1.5">
      {meetings.map((meeting) => {
        const selected = meeting.id === selectedId;
        return (
          <li key={meeting.id}>
            <button
              type="button"
              onClick={() => onSelect(meeting.id)}
              aria-current={selected ? 'true' : undefined}
              className={`w-full text-left rounded-lg border p-3 transition-colors ${
                selected
                  ? 'border-primary/40 bg-accent'
                  : 'border-border hover:bg-accent/50'
              }`}
            >
              <div className="flex items-start gap-2">
                {meeting.source === 'imported' ? (
                  <FileAudio className="w-3.5 h-3.5 text-muted-foreground shrink-0 mt-0.5" />
                ) : (
                  <Mic className="w-3.5 h-3.5 text-muted-foreground shrink-0 mt-0.5" />
                )}
                <div className="min-w-0 flex-1">
                  <p className="text-sm font-medium text-foreground truncate">{meeting.title}</p>
                  <p className="text-[11px] text-muted-foreground mt-0.5">
                    {new Date(meeting.created_at).toLocaleString()} ·{' '}
                    {formatDuration(meeting.duration_seconds)}
                  </p>
                  {meeting.preview && (
                    <p className="text-[11px] text-muted-foreground/80 mt-1 line-clamp-2">
                      {meeting.preview}
                    </p>
                  )}
                  <div className="flex flex-wrap items-center gap-1.5 mt-1.5">
                    {meeting.has_summary && (
                      <Badge variant="emerald" className="text-[10px] gap-1">
                        <Sparkles className="w-2.5 h-2.5" />
                        Report
                      </Badge>
                    )}
                    {meeting.state === 'transcribing' && (
                      <Badge variant="amber" className="text-[10px] gap-1">
                        <Clock className="w-2.5 h-2.5" />
                        Transcribing
                      </Badge>
                    )}
                    {!meeting.system_audio_captured && meeting.source === 'recorded' && (
                      <Badge variant="outline" className="text-[10px]">
                        Mic only
                      </Badge>
                    )}
                    {meeting.dropped_segments > 0 && (
                      <Badge variant="destructive" className="text-[10px] gap-1">
                        <AlertTriangle className="w-2.5 h-2.5" />
                        {meeting.dropped_segments} lost
                      </Badge>
                    )}
                  </div>
                </div>
              </div>
            </button>
          </li>
        );
      })}
    </ul>
  );
};
