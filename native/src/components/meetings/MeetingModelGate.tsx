import React from 'react';
import { AlertTriangle, ChevronRight, Cpu, Download, Settings2, X } from 'lucide-react';

import { Button } from '@/components/ui/button';
import { Card } from '@/components/ui/card';
import { cn } from '@/lib/utils';
import {
  cancelSpeechModelDownload,
  downloadFraction,
  downloadSpeechModel,
  formatBytes,
  listSpeechModels,
  onSpeechModelDownload,
} from '@/lib/speechModels';
import type { SpeechModel, SpeechModelCatalogue } from '@/types/models';

interface MeetingModelGateProps {
  /** Opens Settings › Speech, for the user who wants the full catalogue. */
  onOpenSpeechSettings?: () => void;
  /** Called once a model lands, so the page can retry whatever was blocked. */
  onModelInstalled?: () => void;
  /** Whether to render only the model status line, the install card, or both. Default: 'all'. */
  mode?: 'all' | 'status-only' | 'card-only';
  className?: string;
}

/**
 * Says whether a recording can be transcribed, and fixes it when it cannot.
 *
 * Before this, pressing Record on a machine with no model produced a sentence
 * telling the user to visit a settings section that did not exist, and nothing
 * else happened. The gap was never the message: it was that the fix lived
 * somewhere else, and the place it pointed at was wrong.
 *
 * So the install is here, on the surface that needs it, and the component is
 * otherwise nearly silent — one line naming the model a recording will use,
 * because "which model transcribed this" is the first question asked of a
 * disappointing transcript.
 */
export const MeetingModelGate: React.FC<MeetingModelGateProps> = ({
  onOpenSpeechSettings,
  onModelInstalled,
  mode = 'all',
  className,
}) => {
  const [catalogue, setCatalogue] = React.useState<SpeechModelCatalogue | null>(null);
  const [progress, setProgress] = React.useState<{ fraction: number | null; id: string } | null>(
    null,
  );
  const [error, setError] = React.useState<string | null>(null);

  const refresh = React.useCallback(async () => {
    try {
      setCatalogue(await listSpeechModels());
    } catch {
      // A catalogue that cannot be read is not worth a banner of its own —
      // pressing Record will report the real failure.
      setCatalogue(null);
    }
  }, []);

  React.useEffect(() => {
    void refresh();
  }, [refresh]);

  React.useEffect(() => {
    const unlisten = onSpeechModelDownload((event) => {
      switch (event.state) {
        case 'downloading':
          setProgress({ id: event.id, fraction: downloadFraction(event) });
          break;
        case 'verifying':
          setProgress({ id: event.id, fraction: 1 });
          break;
        case 'failed':
          setProgress(null);
          setError(event.message);
          break;
        case 'ready':
          setProgress(null);
          setError(null);
          void refresh().then(() => onModelInstalled?.());
          break;
        case 'cancelled':
          setProgress(null);
          break;
      }
    });
    return () => {
      void unlisten.then((off) => off());
    };
  }, [refresh, onModelInstalled]);

  if (!catalogue) return null;

  const installed = catalogue.models.filter((model) => model.installed);
  const active = catalogue.models.find((m) => m.id === catalogue.active_meeting_model);

  if (installed.length > 0) {
    if (mode === 'card-only') return null;
    return (
      <Button
        type="button"
        variant="outline"
        size="sm"
        onClick={onOpenSpeechSettings}
        disabled={!onOpenSpeechSettings}
        className={cn(
          "h-8 px-2.5 gap-1.5 text-xs font-medium border-border/80 bg-background/60 backdrop-blur-xs text-foreground hover:bg-accent hover:text-accent-foreground shadow-2xs transition-all",
          className
        )}
        title="Change active speech model"
        aria-label={`Change speech model. Current: ${active ? active.name : 'Default model'}`}
      >
        <Cpu className="w-3.5 h-3.5 text-muted-foreground shrink-0" />
        <span className="text-foreground font-medium truncate max-w-[220px] sm:max-w-[280px]">
          {`Transcribing with ${active ? active.name : 'the best installed model'}`}
        </span>
        {onOpenSpeechSettings && (
          <>
            <span className="text-muted-foreground/70 font-normal">· change</span>
            <ChevronRight className="w-3 h-3 text-muted-foreground/60 shrink-0" />
          </>
        )}
      </Button>
    );
  }

  if (mode === 'status-only') return null;

  const recommended: SpeechModel | undefined =
    catalogue.models.find((m) => m.id === catalogue.recommended_meeting_model) ??
    catalogue.models.find((m) => m.managed);

  const percent = progress?.fraction === null ? null : Math.round((progress?.fraction ?? 0) * 100);

  return (
    <Card className={`p-4 border-amber-500/40 bg-amber-500/5 ${className ?? ''}`}>
      <div className="flex items-start gap-3">
        <AlertTriangle className="w-4 h-4 text-amber-500 shrink-0 mt-0.5" />
        <div className="min-w-0 flex-1">
          <p className="text-sm font-medium text-foreground">
            Recording needs a speech model
          </p>
          <p className="text-xs text-muted-foreground mt-1">
            Without one a meeting would record audio and produce no transcript. The model runs
            on this machine — it is downloaded once and no audio ever leaves.
          </p>

          {error && <p className="text-xs text-destructive mt-2">{error}</p>}

          {progress ? (
            <div className="mt-3">
              <div className="h-1.5 rounded-full bg-muted overflow-hidden">
                <div
                  className="h-full bg-primary transition-[width] duration-300"
                  style={{ width: percent === null ? '100%' : `${percent}%` }}
                />
              </div>
              <div className="flex items-center justify-between gap-3 mt-1.5">
                <p className="text-[11px] text-muted-foreground">
                  {percent === null ? 'Downloading…' : `Downloading… ${percent}%`}
                </p>
                <Button
                  size="sm"
                  variant="ghost"
                  className="gap-1.5 h-7"
                  onClick={() => void cancelSpeechModelDownload(progress.id)}
                >
                  <X className="w-3.5 h-3.5" />
                  Cancel
                </Button>
              </div>
            </div>
          ) : (
            <div className="flex flex-wrap items-center gap-2 mt-3">
              {recommended && (
                <Button
                  size="sm"
                  className="gap-2"
                  onClick={() => {
                    setError(null);
                    setProgress({ id: recommended.id, fraction: null });
                    void downloadSpeechModel(recommended.id).catch((err) => {
                      setProgress(null);
                      setError(err instanceof Error ? err.message : String(err));
                    });
                  }}
                >
                  <Download className="w-3.5 h-3.5" />
                  Install {recommended.name} ({formatBytes(recommended.size_bytes)})
                </Button>
              )}
              {onOpenSpeechSettings && (
                <Button size="sm" variant="outline" className="gap-2" onClick={onOpenSpeechSettings}>
                  <Settings2 className="w-3.5 h-3.5" />
                  Choose a different model
                </Button>
              )}
            </div>
          )}
        </div>
      </div>
    </Card>
  );
};
