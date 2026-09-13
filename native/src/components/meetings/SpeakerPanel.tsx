import React from 'react';
import { Check, Loader2, Mic, Play, Users, X } from 'lucide-react';

import { Button } from '@/components/ui/button';
import { Input } from '@/components/ui/input';
import { formatDuration, formatTimestamp } from '@/lib/meetings';
import type { Speaker } from '@/types/meetings';

interface SpeakerPanelProps {
  speakers: Speaker[];
  detecting: boolean;
  canDetect: boolean;
  onDetect: () => Promise<void> | void;
  onRename: (speakerId: string, label: string) => Promise<void> | void;
  /** Seeks the player to a speaker's sample. Absent while recording. */
  onPlaySample?: (seconds: number) => void;
}

/**
 * Who spoke, and what to call them.
 *
 * Framed as a proposal throughout, because that is what it is: the grouping
 * comes from acoustic statistics rather than a trained speaker model, so it
 * will sometimes merge two people or split one. That is survivable only if
 * fixing it is trivial — hence a play button on every row. The user hears four
 * seconds, recognises the voice, types the name, and the transcript and every
 * future report use it.
 *
 * The alternative shape — assert the names and hide the uncertainty — is worse
 * at exactly the moment it matters, because a confident wrong name in a report
 * is read as fact.
 */
export const SpeakerPanel: React.FC<SpeakerPanelProps> = ({
  speakers,
  detecting,
  canDetect,
  onDetect,
  onRename,
  onPlaySample,
}) => {
  const [editingId, setEditingId] = React.useState<string | null>(null);
  const [draft, setDraft] = React.useState('');

  const startEditing = (speaker: Speaker) => {
    setEditingId(speaker.id);
    setDraft(speaker.label);
  };

  const commit = (speaker: Speaker) => {
    const next = draft.trim();
    if (next && next !== speaker.label) void onRename(speaker.id, next);
    setEditingId(null);
  };

  return (
    <section className="rounded-xl border border-border p-3">
      <div className="flex items-center justify-between gap-2 mb-2">
        <h2 className="text-xs font-semibold text-foreground flex items-center gap-1.5">
          <Users className="w-3.5 h-3.5 text-muted-foreground" />
          Speakers
        </h2>
        <Button
          variant="ghost"
          size="sm"
          onClick={() => void onDetect()}
          disabled={detecting || !canDetect}
          className="h-7 gap-1.5 text-[11px]"
          title={
            canDetect
              ? 'Group the transcript by voice'
              : 'This needs a finished recording with a transcript'
          }
        >
          {detecting && <Loader2 className="w-3 h-3 animate-spin" />}
          {detecting ? 'Listening…' : speakers.length > 0 ? 'Find again' : 'Find speakers'}
        </Button>
      </div>

      {speakers.length === 0 ? (
        <p className="text-[11px] text-muted-foreground leading-relaxed">
          {canDetect
            ? 'Vox can group the transcript by voice and play a few seconds of each one, so you can put names to them. It groups by how voices sound, so it will not always be right — every row can be renamed.'
            : 'Speaker detection needs a finished recording with a transcript.'}
        </p>
      ) : (
        <>
          <ul className="space-y-1">
            {speakers.map((speaker) => (
              <li
                key={speaker.id}
                className="flex items-center gap-2 rounded-lg px-2 py-1.5 hover:bg-accent/50"
              >
                {onPlaySample && (
                  <button
                    type="button"
                    onClick={() => onPlaySample(speaker.sample_start_seconds)}
                    aria-label={`Play ${speaker.label} at ${formatTimestamp(
                      speaker.sample_start_seconds,
                    )}`}
                    title="Play a few seconds of this voice"
                    className="shrink-0 w-6 h-6 rounded-full border border-border flex items-center justify-center text-muted-foreground hover:text-foreground hover:border-foreground/40 transition-colors cursor-pointer"
                  >
                    <Play className="w-3 h-3" />
                  </button>
                )}

                {editingId === speaker.id ? (
                  <>
                    <Input
                      value={draft}
                      onChange={(event) => setDraft(event.target.value)}
                      onKeyDown={(event) => {
                        if (event.key === 'Enter') commit(speaker);
                        if (event.key === 'Escape') setEditingId(null);
                      }}
                      aria-label={`Name for ${speaker.label}`}
                      className="h-7 text-xs flex-1"
                      autoFocus
                    />
                    <Button
                      size="icon"
                      variant="ghost"
                      className="h-6 w-6 shrink-0"
                      onClick={() => commit(speaker)}
                      aria-label="Save name"
                    >
                      <Check className="w-3.5 h-3.5" />
                    </Button>
                    <Button
                      size="icon"
                      variant="ghost"
                      className="h-6 w-6 shrink-0"
                      onClick={() => setEditingId(null)}
                      aria-label="Cancel"
                    >
                      <X className="w-3.5 h-3.5" />
                    </Button>
                  </>
                ) : (
                  <>
                    <button
                      type="button"
                      onClick={() => startEditing(speaker)}
                      className="flex-1 min-w-0 text-left cursor-pointer"
                      title="Rename this speaker"
                    >
                      <span className="text-xs font-medium text-foreground truncate flex items-center gap-1.5">
                        {speaker.channel === 'microphone' && (
                          <Mic className="w-3 h-3 text-emerald-600 dark:text-emerald-400 shrink-0" />
                        )}
                        {speaker.label}
                        {!speaker.named_by_user && (
                          <span className="text-[10px] font-normal text-muted-foreground">
                            — tap to name
                          </span>
                        )}
                      </span>
                    </button>
                    <span className="shrink-0 text-[10px] text-muted-foreground tabular-nums">
                      {speaker.segment_count} lines · {formatDuration(speaker.speaking_seconds)}
                    </span>
                  </>
                )}
              </li>
            ))}
          </ul>
          <p className="text-[10px] text-muted-foreground mt-2 leading-relaxed">
            Grouped by how the voices sound, not by a trained speaker model, so two similar
            voices can end up merged. Names you type are kept if you transcribe again.
          </p>
        </>
      )}
    </section>
  );
};
