import React from 'react';
import {
  FolderOpen,
  Trash2,
  RotateCcw,
  AlertTriangle,
  Check,
  X,
  Pencil,
} from 'lucide-react';

import { Button } from '@/components/ui/button';
import { Badge } from '@/components/ui/badge';
import { Input } from '@/components/ui/input';
import { MeetingTranscript } from './MeetingTranscript';
import { MeetingSummaryPanel } from './MeetingSummaryPanel';
import { formatDuration } from '@/lib/meetings';
import type {
  MeetingDetail as MeetingDetailData,
  MeetingTemplate,
  SummaryProgress,
} from '@/types/meetings';

interface MeetingDetailProps {
  detail: MeetingDetailData;
  templates: MeetingTemplate[];
  templateId: string;
  onTemplateChange: (id: string) => void;
  summaryProgress: SummaryProgress | null;
  live: boolean;
  busy: boolean;
  onRename: (title: string) => void;
  onDelete: () => void;
  onOpenFolder: () => void;
  onRetranscribe: () => void;
  onGenerateSummary: (force: boolean) => void;
  onCancelSummary: () => void;
  onSaveSummary: (markdown: string) => void;
  onPromote: () => void;
}

type Pane = 'transcript' | 'summary';

/** One meeting: its header, its transcript, and its report. */
export const MeetingDetail: React.FC<MeetingDetailProps> = ({
  detail,
  templates,
  templateId,
  onTemplateChange,
  summaryProgress,
  live,
  busy,
  onRename,
  onDelete,
  onOpenFolder,
  onRetranscribe,
  onGenerateSummary,
  onCancelSummary,
  onSaveSummary,
  onPromote,
}) => {
  const { meeting, segments, summary } = detail;
  const [pane, setPane] = React.useState<Pane>('transcript');
  const [editingTitle, setEditingTitle] = React.useState(false);
  const [titleDraft, setTitleDraft] = React.useState(meeting.title);
  const [confirmingDelete, setConfirmingDelete] = React.useState(false);

  React.useEffect(() => {
    setTitleDraft(meeting.title);
    setEditingTitle(false);
    setConfirmingDelete(false);
  }, [meeting.id, meeting.title]);

  // A live recording shows its transcript arriving; a finished one opens on
  // the report when there is one to read.
  React.useEffect(() => {
    setPane(live || !summary?.markdown ? 'transcript' : 'summary');
  }, [meeting.id, live, summary?.markdown]);

  const commitTitle = () => {
    const next = titleDraft.trim();
    if (next && next !== meeting.title) onRename(next);
    setEditingTitle(false);
  };

  return (
    <div className="flex flex-col min-h-0 h-full">
      <header className="shrink-0 pb-3 border-b border-border">
        <div className="flex items-start gap-2">
          <div className="flex-1 min-w-0">
            {editingTitle ? (
              <div className="flex items-center gap-1.5">
                <Input
                  value={titleDraft}
                  onChange={(event) => setTitleDraft(event.target.value)}
                  onKeyDown={(event) => {
                    if (event.key === 'Enter') commitTitle();
                    if (event.key === 'Escape') {
                      setTitleDraft(meeting.title);
                      setEditingTitle(false);
                    }
                  }}
                  aria-label="Meeting title"
                  className="h-8 text-sm"
                  autoFocus
                />
                <Button size="icon" variant="ghost" onClick={commitTitle} aria-label="Save title">
                  <Check className="w-4 h-4" />
                </Button>
                <Button
                  size="icon"
                  variant="ghost"
                  onClick={() => {
                    setTitleDraft(meeting.title);
                    setEditingTitle(false);
                  }}
                  aria-label="Cancel rename"
                >
                  <X className="w-4 h-4" />
                </Button>
              </div>
            ) : (
              <div className="flex items-center gap-1.5 min-w-0">
                <h2 className="text-base font-semibold text-foreground truncate">
                  {meeting.title}
                </h2>
                <Button
                  size="icon"
                  variant="ghost"
                  onClick={() => setEditingTitle(true)}
                  aria-label="Rename meeting"
                  className="shrink-0 h-7 w-7"
                >
                  <Pencil className="w-3.5 h-3.5" />
                </Button>
              </div>
            )}
            <p className="text-[11px] text-muted-foreground mt-0.5">
              {new Date(meeting.created_at).toLocaleString()} ·{' '}
              {formatDuration(meeting.duration_seconds)} · {meeting.segment_count} lines
            </p>
          </div>

          <div className="flex items-center gap-1 shrink-0">
            <Button
              size="icon"
              variant="ghost"
              onClick={onOpenFolder}
              aria-label="Open meeting folder"
              className="h-8 w-8"
            >
              <FolderOpen className="w-4 h-4" />
            </Button>
            <Button
              size="icon"
              variant="ghost"
              onClick={onRetranscribe}
              disabled={busy || live || !meeting.audio_path}
              aria-label="Transcribe again"
              title={
                meeting.audio_path
                  ? 'Transcribe the recording again with the current model'
                  : 'This meeting has no saved recording'
              }
              className="h-8 w-8"
            >
              <RotateCcw className="w-4 h-4" />
            </Button>
            {confirmingDelete ? (
              <>
                <Button size="sm" variant="destructive" onClick={onDelete} disabled={busy}>
                  Delete
                </Button>
                <Button size="sm" variant="ghost" onClick={() => setConfirmingDelete(false)}>
                  Keep
                </Button>
              </>
            ) : (
              <Button
                size="icon"
                variant="ghost"
                onClick={() => setConfirmingDelete(true)}
                disabled={busy || live}
                aria-label="Delete meeting"
                className="h-8 w-8"
              >
                <Trash2 className="w-4 h-4" />
              </Button>
            )}
          </div>
        </div>

        {confirmingDelete && (
          <p className="text-[11px] text-destructive mt-2">
            This removes the transcript, the report and the audio recording. It cannot be
            undone.
          </p>
        )}

        <div className="flex flex-wrap items-center gap-1.5 mt-2">
          {!meeting.system_audio_captured && meeting.source === 'recorded' && (
            <Badge variant="outline" className="text-[10px]">
              Microphone only
            </Badge>
          )}
          {meeting.source === 'imported' && (
            <Badge variant="outline" className="text-[10px]">
              Imported
            </Badge>
          )}
          {meeting.dropped_segments > 0 && (
            <Badge variant="destructive" className="text-[10px] gap-1">
              <AlertTriangle className="w-2.5 h-2.5" />
              {meeting.dropped_segments} segment(s) not transcribed
            </Badge>
          )}
        </div>

        {meeting.error && (
          <p className="flex items-start gap-2 text-[11px] text-amber-600 dark:text-amber-400 mt-2">
            <AlertTriangle className="w-3.5 h-3.5 shrink-0 mt-0.5" />
            <span>{meeting.error}</span>
          </p>
        )}

        <div className="flex gap-1 mt-3" role="tablist">
          {(['transcript', 'summary'] as const).map((value) => (
            <button
              key={value}
              type="button"
              role="tab"
              aria-selected={pane === value}
              onClick={() => setPane(value)}
              className={`px-3 h-7 rounded-lg text-xs font-medium transition-colors ${
                pane === value
                  ? 'bg-accent text-accent-foreground'
                  : 'text-muted-foreground hover:bg-accent/50'
              }`}
            >
              {value === 'transcript' ? 'Transcript' : 'Report'}
            </button>
          ))}
        </div>
      </header>

      <div className="flex-1 min-h-0 pt-3">
        {pane === 'transcript' ? (
          <MeetingTranscript
            segments={segments}
            follow={live}
            emptyMessage={
              live ? 'Listening…' : 'Nothing was transcribed for this meeting.'
            }
          />
        ) : (
          <MeetingSummaryPanel
            summary={summary ?? null}
            templates={templates}
            templateId={templateId}
            onTemplateChange={onTemplateChange}
            progress={summaryProgress}
            hasTranscript={segments.length > 0}
            onGenerate={onGenerateSummary}
            onCancel={onCancelSummary}
            onSave={onSaveSummary}
            onPromote={onPromote}
          />
        )}
      </div>
    </div>
  );
};
