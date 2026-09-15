import React from 'react';
import { AlertTriangle, Loader2, Plus, Repeat } from 'lucide-react';

import { Button } from '@/components/ui/button';
import { Input } from '@/components/ui/input';
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
} from '@/components/ui/dialog';
import * as meetings from '@/lib/meetings';
import { meetingErrorMessage } from '@/lib/meetings';
import type { MeetingSeriesSummary } from '@/types/meetings';

interface SeriesDialogProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  meetingId: string;
  meetingTitle: string;
  /** The series this meeting is in, if any. */
  currentSeriesId?: string | null;
  /** Fires once the membership has changed, so the caller can re-read. */
  onChanged: () => void;
}

/**
 * Putting one recording into a recurring meeting, by hand.
 *
 * Needed for two cases the calendar cannot answer. Imported audio and ad-hoc
 * recordings have no event and never will, so the only honest way to group
 * them is somebody saying so. And a recurrence that was deleted and recreated
 * in Google gets a new id, which genuinely makes the old meetings a different
 * series until a person says otherwise — this is where they say it.
 *
 * Reads its own list rather than taking one as a prop: it is opened rarely,
 * the list is small, and a stale list here would quietly assign a meeting to a
 * series that no longer exists.
 */
export const SeriesDialog: React.FC<SeriesDialogProps> = ({
  open,
  onOpenChange,
  meetingId,
  meetingTitle,
  currentSeriesId,
  onChanged,
}) => {
  const [series, setSeries] = React.useState<MeetingSeriesSummary[]>([]);
  const [loading, setLoading] = React.useState(true);
  const [busy, setBusy] = React.useState(false);
  const [newTitle, setNewTitle] = React.useState('');
  const [error, setError] = React.useState<string | null>(null);

  React.useEffect(() => {
    if (!open) return;
    setError(null);
    setNewTitle('');
    setLoading(true);
    void meetings
      .listMeetingSeries()
      .then((all) => setSeries(all ?? []))
      .catch(() => setSeries([]))
      .finally(() => setLoading(false));
  }, [open]);

  const run = async (action: () => Promise<void>) => {
    setBusy(true);
    setError(null);
    try {
      await action();
      onChanged();
      onOpenChange(false);
    } catch (err) {
      setError(meetingErrorMessage(err));
    } finally {
      setBusy(false);
    }
  };

  const assign = (seriesId: string | null) =>
    run(async () => {
      await meetings.setMeetingSeries(meetingId, seriesId);
    });

  const createAndAssign = () =>
    run(async () => {
      const created = await meetings.createMeetingSeries(newTitle.trim());
      await meetings.setMeetingSeries(meetingId, created.id);
    });

  const canCreate = newTitle.trim().length > 0 && !busy;

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="max-w-md">
        <DialogHeader>
          <DialogTitle>Add to a series</DialogTitle>
          <p className="text-xs text-muted-foreground pt-1">
            Groups “{meetingTitle}” with the other meetings of a recurring call, so they can
            be read in order.
          </p>
        </DialogHeader>

        {error && (
          <p className="flex items-start gap-2 text-xs text-destructive">
            <AlertTriangle className="w-3.5 h-3.5 shrink-0 mt-0.5" />
            <span>{error}</span>
          </p>
        )}

        <div className="max-h-56 overflow-y-auto -mx-1 px-1">
          {loading ? (
            <p className="text-xs text-muted-foreground py-4 text-center">Loading series…</p>
          ) : series.length === 0 ? (
            <p className="text-xs text-muted-foreground py-4 text-center">
              No series yet. Name one below.
            </p>
          ) : (
            <ul className="space-y-1">
              {series.map((entry) => {
                const current = entry.id === currentSeriesId;
                return (
                  <li key={entry.id}>
                    <button
                      type="button"
                      onClick={() => assign(entry.id)}
                      disabled={busy || current}
                      className={`w-full text-left flex items-center gap-2 rounded-md px-2 py-1.5 text-xs transition-colors ${
                        current
                          ? 'bg-accent/60 text-foreground cursor-default'
                          : 'hover:bg-accent/50 text-foreground cursor-pointer'
                      }`}
                    >
                      <Repeat className="w-3.5 h-3.5 text-muted-foreground shrink-0" />
                      <span className="flex-1 min-w-0 truncate">{entry.title}</span>
                      <span className="text-[11px] text-muted-foreground tabular-nums shrink-0">
                        {current ? 'Current' : entry.occurrence_count}
                      </span>
                    </button>
                  </li>
                );
              })}
            </ul>
          )}
        </div>

        <div className="flex items-center gap-2 pt-1">
          <Input
            value={newTitle}
            onChange={(event) => setNewTitle(event.target.value)}
            onKeyDown={(event) => {
              if (event.key === 'Enter' && canCreate) void createAndAssign();
            }}
            placeholder="New series name…"
            aria-label="New series name"
            className="h-8 text-xs"
          />
          <Button size="sm" onClick={() => void createAndAssign()} disabled={!canCreate}>
            {busy ? <Loader2 className="w-3.5 h-3.5 animate-spin" /> : <Plus className="w-3.5 h-3.5" />}
            Create
          </Button>
        </div>

        {currentSeriesId && (
          <Button
            variant="ghost"
            size="sm"
            onClick={() => assign(null)}
            disabled={busy}
            className="text-xs text-muted-foreground self-start"
          >
            Remove from this series
          </Button>
        )}
      </DialogContent>
    </Dialog>
  );
};
