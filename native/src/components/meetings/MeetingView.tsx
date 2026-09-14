import React from 'react';
import { AlertTriangle, Columns2 } from 'lucide-react';

import { Button } from '@/components/ui/button';
import { MeetingTranscript } from './MeetingTranscript';
import { MeetingSummaryPanel } from './MeetingSummaryPanel';
import { MeetingDetailHeader } from './MeetingDetailHeader';
import { SpeakerPanel } from './SpeakerPanel';
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
 * Everything that is not the document lives in the banner
 * ([`MeetingDetailHeader`]): the name, the meta, the recording, the actions.
 * What is left here is a row of tabs and the document itself, which is what a
 * person opened the meeting to read. The version before this one put a back
 * button, an action row, a title, a meta line and an audio player between the
 * page banner and the first line of transcript — five rows of chrome above the
 * thing being read.
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
  const [confirmingDelete, setConfirmingDelete] = React.useState(false);
  const [showSpeakers, setShowSpeakers] = React.useState(false);

  React.useEffect(() => {
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

  return (
    <div className="flex flex-col min-h-0 h-full">
      <MeetingDetailHeader
        detail={detail}
        speakers={speakers}
        live={live}
        busy={busy}
        onBack={onBack}
        editingTitle={editingTitle}
        onEditTitle={setEditingTitle}
        onRename={onRename}
        showSpeakers={showSpeakers}
        onToggleSpeakers={() => setShowSpeakers((open) => !open)}
        onRequestDelete={() => setConfirmingDelete(true)}
        onOpenFolder={onOpenFolder}
        onRetranscribe={onRetranscribe}
        onTranslateTranscript={onTranslateTranscript}
        onDetectSpeakers={onDetectSpeakers}
        seekTo={seekTo}
        onTimeChange={setPlayhead}
      />

      <div className="shrink-0">
        {confirmingDelete && (
          <div className="flex items-center gap-2 mb-3 rounded-lg border border-destructive/40 bg-destructive/5 px-3 py-2">
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
          <p className="flex items-start gap-2 text-[11px] text-amber-600 dark:text-amber-400 mb-2">
            <AlertTriangle className="w-3.5 h-3.5 shrink-0 mt-0.5" />
            <span>
              {meeting.dropped_segments} segment(s) of speech could not be transcribed. The
              recording still contains them — transcribing again with a smaller model will pick
              them up.
            </span>
          </p>
        )}

        {meeting.error && (
          <p className="flex items-start gap-2 text-[11px] text-amber-600 dark:text-amber-400 mb-2">
            <AlertTriangle className="w-3.5 h-3.5 shrink-0 mt-0.5" />
            <span>{meeting.error}</span>
          </p>
        )}

        {showSpeakers && (
          <div className="mb-3">
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

        <div className="flex gap-1 border-b border-border" role="tablist">
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
      </div>

      <div
        className={`flex-1 min-h-0 pt-3 ${
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
