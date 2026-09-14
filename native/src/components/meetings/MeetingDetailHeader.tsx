import React from 'react';
import {
  ArrowLeft,
  Check,
  FolderOpen,
  Languages,
  MoreHorizontal,
  Pencil,
  RotateCcw,
  Trash2,
  Users,
  X,
} from 'lucide-react';

import { Button } from '@/components/ui/button';
import { Input } from '@/components/ui/input';
import { PageHeader } from '@/components/common/PageHeader';
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from '@/components/ui/dropdown-menu';
import { MeetingAudioPlayer } from './MeetingAudioPlayer';
import { formatDuration } from '@/lib/meetings';
import type { MeetingDetail, Speaker } from '@/types/meetings';

interface MeetingDetailHeaderProps {
  detail: MeetingDetail;
  speakers: Speaker[];
  live: boolean;
  busy: boolean;
  onBack: () => void;
  editingTitle: boolean;
  onEditTitle: (editing: boolean) => void;
  onRename: (title: string) => void;
  showSpeakers: boolean;
  onToggleSpeakers: () => void;
  /** Asks for the delete confirmation; it does not delete. */
  onRequestDelete: () => void;
  onOpenFolder: () => void;
  onRetranscribe: () => void;
  onTranslateTranscript?: (targetLanguage?: string) => Promise<void> | void;
  onDetectSpeakers: () => Promise<void> | void;
  /** Seconds to jump the player to, and the nonce that replays the same spot. */
  seekTo: { seconds: number; nonce: number } | null;
  onTimeChange: (seconds: number) => void;
}

/**
 * The banner, while a meeting is open.
 *
 * The page's banner used to be about recording whatever was on screen, so
 * reading a transcript meant looking at a "Start meeting" button the whole
 * time — an invitation to start a second meeting from inside the first. Here
 * the banner is about the meeting being read: its name, who was in it, how
 * long it ran, its recording, and the actions that apply to it.
 *
 * Everything that used to sit in a strip between the banner and the transcript
 * — the back button, the overflow menu, the meta line, the player — is in the
 * banner now. That strip was four rows tall and none of it was the meeting.
 */
export const MeetingDetailHeader: React.FC<MeetingDetailHeaderProps> = ({
  detail,
  speakers,
  live,
  busy,
  onBack,
  editingTitle,
  onEditTitle,
  onRename,
  showSpeakers,
  onToggleSpeakers,
  onRequestDelete,
  onOpenFolder,
  onRetranscribe,
  onTranslateTranscript,
  onDetectSpeakers,
  seekTo,
  onTimeChange,
}) => {
  const { meeting, segments } = detail;
  const [titleDraft, setTitleDraft] = React.useState(meeting.title);

  React.useEffect(() => {
    setTitleDraft(meeting.title);
  }, [meeting.id, meeting.title]);

  const commitTitle = () => {
    const next = titleDraft.trim();
    if (next && next !== meeting.title) onRename(next);
    onEditTitle(false);
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
    meeting.source === 'imported' ? 'Imported' : null,
    !meeting.system_audio_captured && meeting.source === 'recorded' ? 'Microphone only' : null,
  ]
    .filter(Boolean)
    .join(' · ');

  // Who was in the room, as far as Vox knows: the voices it separated, once
  // somebody has put names to them. An unnamed one is "Speaker 2", which says
  // nothing the transcript's own labels do not already say.
  const named = speakers
    .map((speaker) => speaker.label?.trim())
    .filter((label): label is string => Boolean(label));

  return (
    <PageHeader
      variant="banner"
      glowColor="emerald"
      compact
      leading={
        <Button
          variant="ghost"
          size="icon"
          onClick={onBack}
          aria-label="All meetings"
          title="All meetings"
          className="h-8 w-8 text-muted-foreground"
        >
          <ArrowLeft className="w-4 h-4" />
        </Button>
      }
      title={
        editingTitle ? (
          <span className="flex items-center gap-1.5">
            <Input
              value={titleDraft}
              onChange={(event) => setTitleDraft(event.target.value)}
              onKeyDown={(event) => {
                if (event.key === 'Enter') commitTitle();
                if (event.key === 'Escape') {
                  setTitleDraft(meeting.title);
                  onEditTitle(false);
                }
              }}
              aria-label="Meeting title"
              className="h-8 text-sm font-normal max-w-sm"
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
                onEditTitle(false);
              }}
              aria-label="Cancel rename"
            >
              <X className="w-4 h-4" />
            </Button>
          </span>
        ) : (
          meeting.title
        )
      }
      description={
        <span className="flex flex-wrap items-center gap-x-2 gap-y-0.5">
          <span>{meta}</span>
          {named.length > 0 && (
            <>
              <span aria-hidden>·</span>
              <span className="text-foreground/80">{named.join(', ')}</span>
            </>
          )}
          {live && (
            <span className="inline-flex items-center gap-1.5 text-destructive">
              <span className="w-1.5 h-1.5 rounded-full bg-destructive animate-pulse" />
              Recording
            </span>
          )}
        </span>
      }
      footer={
        meeting.audio_path && !live ? (
          <MeetingAudioPlayer
            audioPath={meeting.audio_path}
            seekTo={seekTo}
            onTimeChange={onTimeChange}
          />
        ) : undefined
      }
    >
      <Button
        variant="ghost"
        size="sm"
        onClick={onToggleSpeakers}
        aria-pressed={showSpeakers}
        className="h-8 gap-1.5 text-xs text-muted-foreground"
      >
        <Users className="w-3.5 h-3.5" />
        Speakers
      </Button>

      <DropdownMenu>
        <DropdownMenuTrigger asChild>
          <Button variant="ghost" size="icon" className="h-8 w-8" aria-label="More actions">
            <MoreHorizontal className="w-4 h-4" />
          </Button>
        </DropdownMenuTrigger>
        <DropdownMenuContent align="end" className="w-56">
          <DropdownMenuItem onSelect={() => onEditTitle(true)}>
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
          {/* "Add to graph" is deliberately not here. It belongs to the report,
              it sits on the report panel next to Edit, and having it in both
              places made the menu read as though it applied to the meeting
              rather than to the summary. */}
          <DropdownMenuItem onSelect={onOpenFolder}>
            <FolderOpen className="w-3.5 h-3.5" />
            Open folder
          </DropdownMenuItem>
          <DropdownMenuSeparator />
          <DropdownMenuItem
            onSelect={onRequestDelete}
            disabled={busy || live}
            className="text-destructive focus:text-destructive"
          >
            <Trash2 className="w-3.5 h-3.5" />
            Delete
          </DropdownMenuItem>
        </DropdownMenuContent>
      </DropdownMenu>
    </PageHeader>
  );
};
