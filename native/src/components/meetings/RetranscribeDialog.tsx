import React from 'react';
import { Loader2, RotateCcw } from 'lucide-react';

import { Button } from '@/components/ui/button';
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from '@/components/ui/dialog';
import { listSpeechModels } from '@/lib/speechModels';
import type { SpeechModel } from '@/types/models';
import { formatTimestamp, type RetranscribeOverrides } from '@/lib/meetings';
import type { ImportProgress } from '@/types/meetings';

/**
 * Languages worth pinning a decode to.
 *
 * Not every language Whisper knows — a list of ninety-nine is a list nobody
 * reads. These are the ones Vox's own speech settings offer, plus "Detect
 * automatically", which is what the decoder does when nothing is pinned.
 */
const LANGUAGES: Array<{ code: string; label: string }> = [
  { code: '', label: 'Use my language settings' },
  { code: 'auto', label: 'Detect automatically' },
  { code: 'en', label: 'English' },
  { code: 'hi', label: 'Hindi' },
  { code: 'bn', label: 'Bengali' },
  { code: 'mr', label: 'Marathi' },
  { code: 'ta', label: 'Tamil' },
  { code: 'te', label: 'Telugu' },
  { code: 'kn', label: 'Kannada' },
  { code: 'gu', label: 'Gujarati' },
  { code: 'ml', label: 'Malayalam' },
  { code: 'ur', label: 'Urdu' },
  { code: 'de', label: 'German' },
  { code: 'es', label: 'Spanish' },
  { code: 'fr', label: 'French' },
  { code: 'it', label: 'Italian' },
  { code: 'pt', label: 'Portuguese' },
  { code: 'nl', label: 'Dutch' },
  { code: 'ru', label: 'Russian' },
  { code: 'ja', label: 'Japanese' },
  { code: 'zh', label: 'Chinese' },
];

const PRESETS: Array<{ value: string; label: string; blurb: string }> = [
  {
    value: 'quality',
    label: 'Best',
    blurb: 'Beam search, nothing dropped. Slowest, and what a bad transcript needs.',
  },
  { value: 'balanced', label: 'Balanced', blurb: 'A narrower beam. Roughly half the time.' },
  { value: 'fast', label: 'Fast', blurb: 'Greedy decoding. For checking a change, not for keeping.' },
];

interface RetranscribeDialogProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  onRun: (overrides: RetranscribeOverrides) => Promise<void> | void;
  running: boolean;
  /** Whether the meeting's transcript holds a script the user cannot read. */
  suggestEnglishTrack: boolean;
  /** Progress reports emitted while re-transcription runs. */
  progress?: ImportProgress | null;
}

/**
 * The options a re-transcription runs with.
 *
 * A dialog rather than a button because of what re-transcribing is *for*.
 * Nobody presses it when the transcript is fine; they press it because the
 * transcript is wrong — and running it again on the settings that produced the
 * wrong transcript produces the wrong transcript again. The three choices here
 * are the three that actually change the answer: which language the decoder is
 * told to expect, how big a model reads the audio, and how hard it looks.
 */
export const RetranscribeDialog: React.FC<RetranscribeDialogProps> = ({
  open,
  onOpenChange,
  onRun,
  running,
  suggestEnglishTrack,
  progress,
}) => {
  const [language, setLanguage] = React.useState('');
  const [modelId, setModelId] = React.useState('');
  const [preset, setPreset] = React.useState('quality');
  const [englishTrack, setEnglishTrack] = React.useState(suggestEnglishTrack);
  const [models, setModels] = React.useState<SpeechModel[]>([]);

  React.useEffect(() => {
    if (!open) return;
    setEnglishTrack(suggestEnglishTrack);
    void listSpeechModels()
      .then((catalogue) => setModels(catalogue.models.filter((model) => model.installed)))
      .catch(() => setModels([]));
  }, [open, suggestEnglishTrack]);

  const multilingualOnly = models.filter((model) => model.multilingual);
  const chosenPreset = PRESETS.find((option) => option.value === preset);
  const percent =
    progress?.fraction != null
      ? Math.min(100, Math.max(0, Math.round(progress.fraction * 100)))
      : null;
  const stage = progress?.stage || 'Transcribing';

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="max-w-md">
        <DialogHeader>
          <DialogTitle>Transcribe again</DialogTitle>
          <DialogDescription>
            The recording is decoded from scratch and replaces the current transcript. The audio
            is untouched.
          </DialogDescription>
        </DialogHeader>

        <div className="space-y-4">
          <Field
            label="Language"
            hint="Whisper re-detects the language every thirty seconds. On a call that switches between two languages that can land on the wrong one mid-meeting, which comes back as fluent nonsense rather than as an error. Pinning stops it guessing."
          >
            <select
              value={language}
              onChange={(event) => setLanguage(event.target.value)}
              disabled={running}
              aria-label="Transcription language"
              className="h-9 w-full rounded-md border border-border bg-card px-2 text-sm text-foreground disabled:opacity-50"
            >
              {LANGUAGES.map((option) => (
                <option key={option.code || 'default'} value={option.code}>
                  {option.label}
                </option>
              ))}
            </select>
          </Field>

          <Field
            label="Model"
            hint="A bigger model is the single largest difference on speech that is not English."
          >
            <select
              value={modelId}
              onChange={(event) => setModelId(event.target.value)}
              disabled={running}
              aria-label="Speech model"
              className="h-9 w-full rounded-md border border-border bg-card px-2 text-sm text-foreground disabled:opacity-50"
            >
              <option value="">Use my speech settings</option>
              {multilingualOnly.map((model) => (
                <option key={model.id} value={model.id}>
                  {model.name}
                </option>
              ))}
            </select>
          </Field>

          <Field label="Quality" hint={chosenPreset?.blurb}>
            <div className="flex rounded-md border border-border bg-muted p-0.5" role="group">
              {PRESETS.map((option) => (
                <button
                  key={option.value}
                  type="button"
                  onClick={() => setPreset(option.value)}
                  disabled={running}
                  aria-pressed={preset === option.value}
                  className={`flex-1 h-7 rounded text-xs font-medium transition-colors cursor-pointer disabled:cursor-not-allowed disabled:opacity-50 ${
                    preset === option.value
                      ? 'bg-card text-foreground shadow-xs'
                      : 'text-muted-foreground hover:text-foreground'
                  }`}
                >
                  {option.label}
                </button>
              ))}
            </div>
          </Field>

          <label
            className={`flex items-start gap-2.5 ${
              running ? 'cursor-not-allowed opacity-60' : 'cursor-pointer'
            }`}
          >
            <input
              type="checkbox"
              checked={englishTrack}
              disabled={running}
              onChange={(event) => setEnglishTrack(event.target.checked)}
              className="mt-0.5 accent-primary"
            />
            <span className="min-w-0">
              <span className="block text-sm text-foreground">Also produce an English version</span>
              <span className="block text-[11px] text-muted-foreground leading-relaxed mt-0.5">
                Decodes every line a second time with Whisper translating to English. Roughly
                doubles the time, and is better than translating the transcript afterwards
                because it reads the audio rather than a transcript that may already be wrong.
              </span>
            </span>
          </label>

          {running && (
            <div className="rounded-lg border border-border bg-muted/40 p-3 space-y-2 mt-2">
              <div className="flex items-center justify-between text-xs">
                <span className="font-medium text-foreground flex items-center gap-1.5">
                  <Loader2 className="w-3.5 h-3.5 animate-spin text-primary" />
                  {stage}…
                </span>
                <span className="font-semibold tabular-nums text-foreground">
                  {percent != null ? `${percent}%` : 'Starting…'}
                </span>
              </div>
              <div className="h-1.5 w-full rounded-full bg-muted overflow-hidden">
                <div
                  className="h-full rounded-full bg-primary transition-all duration-300"
                  style={{
                    width: percent != null ? `${percent}%` : '15%',
                  }}
                />
              </div>
              <div className="flex items-center justify-between text-[11px] text-muted-foreground">
                <span>
                  {progress?.processed_seconds != null && progress.total_seconds
                    ? `${formatTimestamp(progress.processed_seconds)} / ${formatTimestamp(progress.total_seconds)}`
                    : progress?.processed_seconds != null && progress.processed_seconds > 0
                      ? `${formatTimestamp(progress.processed_seconds)} processed`
                      : 'Reading audio…'}
                </span>
                {progress?.segments ? (
                  <span>
                    {progress.segments} segment{progress.segments === 1 ? '' : 's'}
                  </span>
                ) : null}
              </div>
            </div>
          )}
        </div>

        <DialogFooter>
          <Button variant="ghost" onClick={() => onOpenChange(false)} disabled={running}>
            Cancel
          </Button>
          <Button
            onClick={() => void onRun({ language, modelId, preset, englishTrack })}
            disabled={running}
            className="gap-2"
          >
            {running ? (
              <Loader2 className="w-3.5 h-3.5 animate-spin" />
            ) : (
              <RotateCcw className="w-3.5 h-3.5" />
            )}
            {running
              ? percent != null
                ? `${stage}… ${percent}%`
                : `${stage}…`
              : 'Transcribe again'}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
};

const Field: React.FC<{ label: string; hint?: string; children: React.ReactNode }> = ({
  label,
  hint,
  children,
}) => (
  <div>
    <p className="text-sm font-medium text-foreground mb-1">{label}</p>
    {children}
    {hint && <p className="text-[11px] text-muted-foreground leading-relaxed mt-1">{hint}</p>}
  </div>
);
