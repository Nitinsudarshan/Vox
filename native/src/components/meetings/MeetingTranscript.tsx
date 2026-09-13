import React from 'react';
import { Copy, Check, Languages, Loader2 } from 'lucide-react';

import { Button } from '@/components/ui/button';
import { Input } from '@/components/ui/input';
import { channelLabel, formatTimestamp, transcriptToText } from '@/lib/meetings';
import type { TranscriptSegment } from '@/types/meetings';

export type TranscriptViewMode = 'original' | 'romanized' | 'english';

interface MeetingTranscriptProps {
  meetingId?: string;
  segments: TranscriptSegment[];
  /** Scrolls to the newest line as it arrives. On while recording. */
  follow?: boolean;
  emptyMessage?: string;
  /**
   * Where the recording is playing, in seconds. The line covering it is
   * highlighted; undefined means nothing is playing and nothing highlights.
   */
  playheadSeconds?: number;
  /** Set when a line can be clicked to play from there. */
  onSeek?: (seconds: number) => void;
  /** Optional handler to trigger LLM translation of segments into English. */
  onTranslate?: (targetLanguage?: string) => Promise<void> | void;
  isTranslating?: boolean;
}

function hasScriptVariants(segments: TranscriptSegment[]): boolean {
  return segments.some(
    (s) =>
      Boolean(s.original_text) ||
      Boolean(s.romanized_text) ||
      Boolean(s.translated_text) ||
      /[\u0900-\u097F]/.test(s.text),
  );
}

function resolveSegmentText(segment: TranscriptSegment, mode: TranscriptViewMode): string {
  if (mode === 'original') {
    return segment.original_text || segment.text;
  }
  if (mode === 'romanized') {
    return segment.romanized_text || segment.text;
  }
  if (mode === 'english') {
    return segment.translated_text || segment.text;
  }
  return segment.text;
}

/**
 * A meeting's transcript with support for toggling between Original,
 * Romanized (Hinglish/Latin script), and English translation.
 */
export const MeetingTranscript: React.FC<MeetingTranscriptProps> = ({
  meetingId: _meetingId,
  segments,
  follow = false,
  emptyMessage = 'Nothing has been transcribed yet.',
  playheadSeconds,
  onSeek,
  onTranslate,
  isTranslating = false,
}) => {
  const [filter, setFilter] = React.useState('');
  const [copied, setCopied] = React.useState(false);
  const [viewMode, setViewMode] = React.useState<TranscriptViewMode>('original');
  const endRef = React.useRef<HTMLDivElement>(null);
  const activeRef = React.useRef<HTMLDivElement>(null);

  const showVariants = React.useMemo(() => hasScriptVariants(segments), [segments]);
  const hasAnyEnglishTranslation = React.useMemo(
    () => segments.some((s) => Boolean(s.translated_text)),
    [segments],
  );

  // Pick the most informative view mode by default when translations or romanization exist
  React.useEffect(() => {
    if (segments.some((s) => Boolean(s.translated_text))) {
      setViewMode('english');
    } else if (segments.some((s) => Boolean(s.romanized_text))) {
      setViewMode('romanized');
    }
  }, [segments]);

  React.useEffect(() => {
    if (follow) endRef.current?.scrollIntoView({ behavior: 'smooth', block: 'end' });
  }, [follow, segments.length]);

  const needle = filter.trim().toLowerCase();
  const visible = React.useMemo(
    () =>
      needle
        ? segments.filter((segment) =>
            resolveSegmentText(segment, viewMode).toLowerCase().includes(needle),
          )
        : segments,
    [segments, needle, viewMode],
  );

  const activeSequence = React.useMemo(() => {
    if (playheadSeconds === undefined) return null;
    let found: number | null = null;
    for (const segment of segments) {
      if (segment.start_seconds <= playheadSeconds) found = segment.sequence;
      else break;
    }
    return found;
  }, [segments, playheadSeconds]);

  React.useEffect(() => {
    if (activeSequence === null) return;
    activeRef.current?.scrollIntoView({ behavior: 'smooth', block: 'nearest' });
  }, [activeSequence]);

  const handleCopy = async () => {
    try {
      await navigator.clipboard.writeText(
        transcriptToText(segments, (s) => resolveSegmentText(s, viewMode)),
      );
      setCopied(true);
      setTimeout(() => setCopied(false), 2000);
    } catch {
      // A denied clipboard is not worth an error dialog; the button simply
      // does not confirm.
    }
  };

  return (
    <div className="flex flex-col min-h-0 h-full">
      <div className="flex items-center gap-2 mb-3 shrink-0 flex-wrap">
        <Input
          value={filter}
          onChange={(event) => setFilter(event.target.value)}
          placeholder="Find in transcript…"
          className="h-8 text-xs flex-1 min-w-[140px]"
          aria-label="Find in transcript"
        />

        {showVariants && (
          <div
            className="flex bg-muted p-0.5 rounded-lg border border-border shrink-0"
            role="group"
            aria-label="Script and translation mode"
          >
            <button
              type="button"
              onClick={() => setViewMode('original')}
              className={`px-2 py-1 text-[11px] font-medium rounded-md transition-all cursor-pointer ${
                viewMode === 'original'
                  ? 'bg-card text-foreground font-semibold shadow-xs'
                  : 'text-muted-foreground hover:text-foreground'
              }`}
            >
              Original
            </button>
            <button
              type="button"
              onClick={() => setViewMode('romanized')}
              className={`px-2 py-1 text-[11px] font-medium rounded-md transition-all cursor-pointer ${
                viewMode === 'romanized'
                  ? 'bg-card text-foreground font-semibold shadow-xs'
                  : 'text-muted-foreground hover:text-foreground'
              }`}
            >
              Romanized
            </button>
            <button
              type="button"
              onClick={() => {
                setViewMode('english');
                if (!hasAnyEnglishTranslation && onTranslate) {
                  onTranslate('English');
                }
              }}
              className={`px-2 py-1 text-[11px] font-medium rounded-md transition-all cursor-pointer inline-flex items-center gap-1 ${
                viewMode === 'english'
                  ? 'bg-card text-foreground font-semibold shadow-xs'
                  : 'text-muted-foreground hover:text-foreground'
              }`}
            >
              {isTranslating && viewMode === 'english' ? (
                <Loader2 className="w-3 h-3 animate-spin" />
              ) : null}
              English
            </button>
          </div>
        )}

        {showVariants && onTranslate && !hasAnyEnglishTranslation && (
          <Button
            variant="outline"
            size="sm"
            onClick={() => onTranslate('English')}
            disabled={isTranslating}
            className="gap-1.5 shrink-0 text-xs h-8"
            title="Translate transcript into English"
          >
            {isTranslating ? (
              <Loader2 className="w-3.5 h-3.5 animate-spin" />
            ) : (
              <Languages className="w-3.5 h-3.5" />
            )}
            {isTranslating ? 'Translating…' : 'Translate'}
          </Button>
        )}

        <Button
          variant="outline"
          size="sm"
          onClick={handleCopy}
          disabled={segments.length === 0}
          className="gap-1.5 shrink-0 h-8 text-xs"
        >
          {copied ? <Check className="w-3.5 h-3.5" /> : <Copy className="w-3.5 h-3.5" />}
          {copied ? 'Copied' : 'Copy'}
        </Button>
      </div>

      <div className="flex-1 min-h-0 overflow-y-auto pr-1 space-y-2">
        {visible.length === 0 ? (
          <p className="text-xs text-muted-foreground py-6 text-center">
            {segments.length === 0 ? emptyMessage : 'No lines match that search.'}
          </p>
        ) : (
          visible.map((segment, index) => {
            const previous = visible[index - 1];
            const showSpeaker = !previous || previous.channel !== segment.channel;
            const active = segment.sequence === activeSequence;
            const displayText = resolveSegmentText(segment, viewMode);
            return (
              <div key={segment.sequence} className={showSpeaker ? 'pt-2 first:pt-0' : ''}>
                {showSpeaker && (
                  <p
                    className={`text-[11px] font-semibold mb-0.5 ${
                      segment.channel === 'microphone'
                        ? 'text-emerald-600 dark:text-emerald-400'
                        : segment.channel === 'system'
                          ? 'text-blue-600 dark:text-blue-400'
                          : 'text-muted-foreground'
                    }`}
                  >
                    {channelLabel(segment.channel)}
                  </p>
                )}
                <div
                  ref={active ? activeRef : undefined}
                  className={`flex gap-2.5 rounded-md -mx-1 px-1 py-0.5 transition-colors ${
                    active ? 'bg-accent' : ''
                  }`}
                >
                  {onSeek ? (
                    <button
                      type="button"
                      onClick={() => onSeek(segment.start_seconds)}
                      aria-label={`Play from ${formatTimestamp(segment.start_seconds)}`}
                      className="text-[10px] font-mono text-muted-foreground/70 hover:text-foreground pt-0.5 shrink-0 tabular-nums cursor-pointer"
                    >
                      {formatTimestamp(segment.start_seconds)}
                    </button>
                  ) : (
                    <span className="text-[10px] font-mono text-muted-foreground/70 pt-0.5 shrink-0 tabular-nums">
                      {formatTimestamp(segment.start_seconds)}
                    </span>
                  )}
                  <p className="text-sm text-foreground leading-relaxed">{displayText}</p>
                </div>
              </div>
            );
          })
        )}
        <div ref={endRef} />
      </div>
    </div>
  );
};
