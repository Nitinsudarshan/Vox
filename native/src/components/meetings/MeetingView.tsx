import React from 'react';
import {
  AlertTriangle,
  ArrowLeft,
  Check,
  Columns2,
  FolderOpen,
  Languages,
  MoreHorizontal,
  Network,
  Pencil,
  RotateCcw,
  Trash2,
  Users,
  X,
} from 'lucide-react';

import { Button } from '@/components/ui/button';
import { Input } from '@/components/ui/input';
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from '@/components/ui/dropdown-menu';
import { MeetingTranscript } from './MeetingTranscript';
import { MeetingSummaryPanel } from './MeetingSummaryPanel';
import { MeetingAudioPlayer } from './MeetingAudioPlayer';
import { SpeakerPanel } from './SpeakerPanel';
import { formatDuration } from '@/lib/meetings';
import type {
  MeetingDetail as MeetingDetailData,
  MeetingTemplate,
  SummaryProgress,
} from '@/types/meetings';

interface MeetingViewProps {
  detail: MeetingDetailData;
  templates: MeetingTemplate[];
  templateId: string;
  onTemplateChange: (id: string) => void;
  summaryProgress: SummaryProgress | null;
  live: boolean;
  busy: boolean;
  onBack: () => void;
  onRename: (title: string) => void;
  onDelete: () => void;
  onOpenFolder: () => void;
  onRetranscribe: () => void;
  onGenerateSummary: (force: boolean) => void;
  onCancelSummary: () => void;
  onSaveSummary: (markdown: string) => void;
  onPromote: () => void;
  onOpenProviderSettings?: () => void;
  onTranslateTranscript?: (targetLanguage?: string) => Promise<void> | void;
  isTranslatingTranscript?: boolean;
  onDetectSpeakers: () => Promise<void> | void;
  onRenameSpeaker: (speakerId: string, label: string) => Promise<void> | void;
  detectingSpeakers: boolean;
}

/** Which pane is showing. `split` appears only where it fits. */
type Pane = 'summary' | 'transcript' | 'split';

/** Below this the split becomes two unreadable columns, so it becomes tabs. */
const SPLIT_MIN_WIDTH = 1024;

/**
 * One meeting, as a page rather than as a panel.
 *
 * The version this replaces lived in the right-hand half of a two-column
 * layout, under a page header, a recorder card, a model-gate card and a row of
 * icon buttons — so the report a user came to read got about a third of the
 * window, and everything competing for the rest was chrome. Here the meeting
 * *is* the page: a title, one line of context, and the document. Everything
 * that is not reading the meeting has moved into the overflow menu, where it
 * costs one click and no permanent space.
 */
export const MeetingView: React.FC<MeetingViewProps> = ({
  detail,
  templates,
  templateId,
  onTemplateChange,
  summaryProgress,
  live,
  busy,
  onBack,
  onRename,
  onDelete,
  onOpenFolder,
  onRetranscribe,
  onGenerateSummary,
  onCancelSummary,
  onSaveSummary,
  onPromote,
  onOpenProviderSettings,
  onTranslateTranscript,
  isTranslatingTranscript,
  onDetectSpeakers,
  onRenameSpeaker,
  detectingSpeakers,
}) => {
  const { meeting, segments, summary } = detail;
  // Absent for a meeting stored before speaker detection existed.
  const speakers = detail.speakers ?? [];
  const [wide, setWide] = React.useState(
    () => typeof window !== 'undefined' && window.innerWidth >= SPLIT_MIN_WIDTH,
  );
  const [pane, setPane] = React.useState<Pane>('transcript');
  const [playhead, setPlayhead] = React.useState<number | undefined>(undefined);
  const [seekTo, setSeekTo] = React.useState<{ seconds: number; nonce: number } | null>(null);
  const [editingTitle, setEditingTitle] = React.useState(false);
  const [titleDraft, setTitleDraft] = React.useState(meeting.title);
  const [confirmingDelete, setConfirmingDelete] = React.useState(false);
  const [showSpeakers, setShowSpeakers] = React.useState(false);

  React.useEffect(() => {
    setTitleDraft(meeting.title);
    setEditingTitle(false);
    setConfirmingDelete(false);
  }, [meeting.id, meeting.title]);

  React.useEffect(() => {
    const onResize = () => setWide(window.innerWidth >= SPLIT_MIN_WIDTH);
    window.addEventListener('resize', onResize);
    return () => window.removeEventListener('resize', onResize);
  }, []);

  // A live recording shows only its transcript arriving — there is no report
  // to put beside it yet. A finished one opens on the report when there is one
  // to read, which is what a person came for; the transcript is what they
  // check it against.
  const hasReportToShow = Boolean(summary?.markdown) || summary?.status === 'failed';
  React.useEffect(() => {
    if (live || !hasReportToShow) setPane('transcript');
    else setPane(wide ? 'split' : 'summary');
  }, [meeting.id, live, hasReportToShow, wide]);

  React.useEffect(() => {
    setPlayhead(undefined);
    setSeekTo(null);
    setShowSpeakers(false);
  }, [meeting.id]);

  const panes: Array<{ value: Pane; label: string }> = [
    { value: 'summary', label: 'Report' },
    { value: 'transcript', label: 'Transcript' },
    ...(wide ? [{ value: 'split' as Pane, label: 'Both' }] : []),
  ];

  const seekAudio = (seconds: number) =>
    setSeekTo((previous) => ({ seconds, nonce: (previous?.nonce ?? 0) + 1 }));

  const commitTitle = () => {
    const next = titleDraft.trim();
    if (next && next !== meeting.title) onRename(next);
    setEditingTitle(false);
  };

  const recordedAt = new Date(meeting.created_at);
  const meta = [
    Number.isNaN(recordedAt.getTime())
      ? null
      : recordedAt.toLocaleString(undefined, {
          weekday: 'short',
          month: 'short',
          day: 'numeric',
          hour: 'numeric',
          minute: '2-digit',
        }),
    formatDuration(meeting.duration_seconds),
    `${meeting.segment_count} lines`,
    speakers.length > 0 ? `${speakers.length} speakers` : null,
    meeting.source === 'imported' ? 'Imported' : null,
    !meeting.system_audio_captured && meeting.source === 'recorded' ? 'Microphone only' : null,
  ].filter(Boolean) as string[];

  return (
    <div className="flex flex-col min-h-0 h-full">
      <header className="shrink-0">
        <div className="flex items-center justify-between gap-2 mb-4">
          <Button
            variant="ghost"
            size="sm"
            onClick={onBack}
            className="h-8 -ml-2 gap-1.5 text-xs text-muted-foreground"
          >
            <ArrowLeft className="w-3.5 h-3.5" />
            All meetings
          </Button>

          <div className="flex items-center gap-1">
            <Button
              variant="ghost"
              size="sm"
              onClick={() => setShowSpeakers((open) => !open)}
              aria-pressed={showSpeakers}
              className="h-8 gap-1.5 text-xs text-muted-foreground"
            >
              <Users className="w-3.5 h-3.5" />
              Speakers
            </Button>
            <DropdownMenu>
              <DropdownMenuTrigger asChild>
                <Button
                  variant="ghost"
                  size="icon"
                  className="h-8 w-8"
                  aria-label="More actions"
                >
                  <MoreHorizontal className="w-4 h-4" />
                </Button>
              </DropdownMenuTrigger>
              <DropdownMenuContent align="end" className="w-56">
                <DropdownMenuItem onSelect={() => setEditingTitle(true)}>
                  <Pencil className="w-3.5 h-3.5" />
                  Rename
                </DropdownMenuItem>
                <DropdownMenuItem
                  onSelect={() => onRetranscribe()}
                  disabled={busy || live || !meeting.audio_path}
                >
                  <RotateCcw className="w-3.5 h-3.5" />
                  Transcribe again…
                </DropdownMenuItem>
                <DropdownMenuItem
                  onSelect={() => void onTranslateTranscript?.('English')}
                  disabled={busy || live || segments.length === 0}
                >
                  <Languages className="w-3.5 h-3.5" />
                  Translate to English
                </DropdownMenuItem>
                <DropdownMenuItem
                  onSelect={() => void onDetectSpeakers()}
                  disabled={busy || live || !meeting.audio_path}
                >
                  <Users className="w-3.5 h-3.5" />
                  Find speakers
                </DropdownMenuItem>
                <DropdownMenuSeparator />
                <DropdownMenuItem onSelect={onPromote} disabled={!summary?.markdown}>
                  <Network className="w-3.5 h-3.5" />
                  Add to graph
                </DropdownMenuItem>
                <DropdownMenuItem onSelect={onOpenFolder}>
                  <FolderOpen className="w-3.5 h-3.5" />
                  Open folder
                </DropdownMenuItem>
                <DropdownMenuSeparator />
                <DropdownMenuItem
                  onSelect={() => setConfirmingDelete(true)}
                  disabled={busy || live}
                  className="text-destructive focus:text-destructive"
                >
                  <Trash2 className="w-3.5 h-3.5" />
                  Delete
                </DropdownMenuItem>
              </DropdownMenuContent>
            </DropdownMenu>
          </div>
        </div>

        {editingTitle ? (
          <div className="flex items-center gap-1.5 mb-1">
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
              className="h-10 text-lg"
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
          <h1 className="font-serif text-2xl md:text-3xl text-foreground leading-tight">
            {meeting.title}
          </h1>
        )}

        <p className="text-xs text-muted-foreground mt-1.5">{meta.join(' · ')}</p>

        {confirmingDelete && (
          <div className="flex items-center gap-2 mt-3 rounded-lg border border-destructive/40 bg-destructive/5 px-3 py-2">
            <AlertTriangle className="w-3.5 h-3.5 text-destructive shrink-0" />
            <p className="text-[11px] text-destructive flex-1">
              This removes the transcript, the report and the recording. It cannot be undone.
            </p>
            <Button size="sm" variant="destructive" onClick={onDelete} disabled={busy}>
              Delete
            </Button>
            <Button size="sm" variant="ghost" onClick={() => setConfirmingDelete(false)}>
              Keep
            </Button>
          </div>
        )}

        {meeting.dropped_segments > 0 && (
          <p className="flex items-start gap-2 text-[11px] text-amber-600 dark:text-amber-400 mt-2">
            <AlertTriangle className="w-3.5 h-3.5 shrink-0 mt-0.5" />
            <span>
              {meeting.dropped_segments} segment(s) of speech could not be transcribed. The
              recording still contains them — transcribing again with a smaller model will pick
              them up.
            </span>
          </p>
        )}

        {meeting.error && (
          <p className="flex items-start gap-2 text-[11px] text-amber-600 dark:text-amber-400 mt-2">
            <AlertTriangle className="w-3.5 h-3.5 shrink-0 mt-0.5" />
            <span>{meeting.error}</span>
          </p>
        )}

        {showSpeakers && (
          <div className="mt-4">
            <SpeakerPanel
              speakers={speakers}
              detecting={detectingSpeakers}
              canDetect={Boolean(meeting.audio_path) && !live && segments.length > 0}
              onDetect={onDetectSpeakers}
              onRename={onRenameSpeaker}
              onPlaySample={meeting.audio_path && !live ? seekAudio : undefined}
            />
          </div>
        )}

        {meeting.audio_path && !live && (
          <div className="mt-4">
            <MeetingAudioPlayer
              audioPath={meeting.audio_path}
              seekTo={seekTo}
              onTimeChange={setPlayhead}
            />
          </div>
        )}

        <div className="flex gap-1 mt-4 border-b border-border" role="tablist">
          {panes.map(({ value, label }) => (
            <button
              key={value}
              type="button"
              role="tab"
              aria-selected={pane === value}
              onClick={() => setPane(value)}
              className={`px-3 h-8 text-xs font-medium transition-colors inline-flex items-center gap-1.5 border-b-2 -mb-px cursor-pointer ${
                pane === value
                  ? 'border-primary text-foreground'
                  : 'border-transparent text-muted-foreground hover:text-foreground'
              }`}
            >
              {value === 'split' && <Columns2 className="w-3.5 h-3.5" />}
              {label}
            </button>
          ))}
        </div>
      </header>

      <div
        className={`flex-1 min-h-0 pt-4 ${
          pane === 'split' ? 'grid grid-cols-2 gap-6 divide-x divide-border' : ''
        }`}
      >
        {(pane === 'transcript' || pane === 'split') && (
          <MeetingTranscript
            meetingId={meeting.id}
            segments={segments}
            speakers={speakers}
            follow={live}
            playheadSeconds={playhead}
            onSeek={meeting.audio_path && !live ? seekAudio : undefined}
            emptyMessage={live ? 'Listening…' : 'Nothing was transcribed for this meeting.'}
            onTranslate={onTranslateTranscript}
            isTranslating={isTranslatingTranscript}
          />
        )}
        {(pane === 'summary' || pane === 'split') && (
          <div className={pane === 'split' ? 'min-h-0 pl-6' : 'contents'}>
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
              onOpenProviderSettings={onOpenProviderSettings}
            />
          </div>
        )}
      </div>
    </div>
  );
};
