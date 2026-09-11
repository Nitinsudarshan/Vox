import React from 'react';
import { Copy, Check } from 'lucide-react';

import { Button } from '@/components/ui/button';
import { Input } from '@/components/ui/input';
import { channelLabel, formatTimestamp, transcriptToText } from '@/lib/meetings';
import type { TranscriptSegment } from '@/types/meetings';

interface MeetingTranscriptProps {
  segments: TranscriptSegment[];
  /** Scrolls to the newest line as it arrives. On while recording. */
  follow?: boolean;
  emptyMessage?: string;
}

/**
 * A meeting's transcript.
 *
 * Lines are grouped by speaker the way a transcript reads: the label appears
 * when the channel changes, not on every line.
 */
export const MeetingTranscript: React.FC<MeetingTranscriptProps> = ({
  segments,
  follow = false,
  emptyMessage = 'Nothing has been transcribed yet.',
}) => {
  const [filter, setFilter] = React.useState('');
  const [copied, setCopied] = React.useState(false);
  const endRef = React.useRef<HTMLDivElement>(null);

  React.useEffect(() => {
    if (follow) endRef.current?.scrollIntoView({ behavior: 'smooth', block: 'end' });
  }, [follow, segments.length]);

  const needle = filter.trim().toLowerCase();
  const visible = React.useMemo(
    () =>
      needle
        ? segments.filter((segment) => segment.text.toLowerCase().includes(needle))
        : segments,
    [segments, needle],
  );

  const handleCopy = async () => {
    try {
      await navigator.clipboard.writeText(transcriptToText(segments));
      setCopied(true);
      setTimeout(() => setCopied(false), 2000);
    } catch {
      // A denied clipboard is not worth an error dialog; the button simply
      // does not confirm.
    }
  };

  return (
    <div className="flex flex-col min-h-0 h-full">
      <div className="flex items-center gap-2 mb-3 shrink-0">
        <Input
          value={filter}
          onChange={(event) => setFilter(event.target.value)}
          placeholder="Find in transcript…"
          className="h-8 text-xs"
          aria-label="Find in transcript"
        />
        <Button
          variant="outline"
          size="sm"
          onClick={handleCopy}
          disabled={segments.length === 0}
          className="gap-1.5 shrink-0"
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
                <div className="flex gap-2.5">
                  <span className="text-[10px] font-mono text-muted-foreground/70 pt-0.5 shrink-0 tabular-nums">
                    {formatTimestamp(segment.start_seconds)}
                  </span>
                  <p className="text-sm text-foreground leading-relaxed">{segment.text}</p>
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
