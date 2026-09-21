import React from 'react';
import { ChevronRight, Mic } from 'lucide-react';

import { Button } from '@/components/ui/button';
import { cn } from '@/lib/utils';
import {
  getParakeetStatus,
  listSpeechModels,
  onSpeechModelDownload,
  type ParakeetStatus,
} from '@/lib/speechModels';
import type { SpeechModelCatalogue } from '@/types/models';

export interface DictationModelGateProps {
  /** Opens Settings › Speech / Models & Speech so the user can switch or download models. */
  onOpenSpeechSettings?: () => void;
  className?: string;
}

/**
 * Displays which speech model is actively used for Universal Dictation and Voice Notes,
 * matching the header pattern from Meetings.
 */
export const DictationModelGate: React.FC<DictationModelGateProps> = ({
  onOpenSpeechSettings,
  className,
}) => {
  const [catalogue, setCatalogue] = React.useState<SpeechModelCatalogue | null>(null);
  const [parakeetStatus, setParakeetStatus] = React.useState<ParakeetStatus | null>(null);

  const refresh = React.useCallback(async () => {
    try {
      const [cat, parakeet] = await Promise.all([
        listSpeechModels(),
        getParakeetStatus().catch(() => null),
      ]);
      setCatalogue(cat);
      setParakeetStatus(parakeet);
    } catch {
      setCatalogue(null);
    }
  }, []);

  React.useEffect(() => {
    void refresh();
  }, [refresh]);

  React.useEffect(() => {
    const unlisten = onSpeechModelDownload((event) => {
      if (event.state === 'ready') {
        void refresh();
      }
    });
    return () => {
      void unlisten.then((off) => off());
    };
  }, [refresh]);

  // Determine active model display name
  const modelName = React.useMemo(() => {
    if (parakeetStatus?.active_for_dictation) {
      return 'NVIDIA Parakeet';
    }
    if (catalogue?.active_dictation_model) {
      const found = catalogue.models.find((m) => m.id === catalogue.active_dictation_model);
      return found ? found.name : catalogue.active_dictation_model;
    }
    return 'Base';
  }, [parakeetStatus, catalogue]);

  return (
    <Button
      type="button"
      variant="outline"
      size="sm"
      onClick={onOpenSpeechSettings}
      disabled={!onOpenSpeechSettings}
      className={cn(
        "h-8 px-2.5 gap-1.5 text-xs font-medium border-border/80 bg-background/60 backdrop-blur-xs text-foreground hover:bg-accent hover:text-accent-foreground shadow-2xs transition-all",
        onOpenSpeechSettings && "cursor-pointer",
        className
      )}
      title="Change active speech model for dictation"
      aria-label={`Change dictation speech model. Current: ${modelName}`}
    >
      <Mic className="w-3.5 h-3.5 text-emerald-500 shrink-0" />
      <span className="text-foreground font-medium truncate max-w-[200px] sm:max-w-[260px]">
        {`Dictating with ${modelName}`}
      </span>
      {onOpenSpeechSettings && (
        <>
          <span className="text-muted-foreground/70 font-normal">· change</span>
          <ChevronRight className="w-3 h-3 text-muted-foreground/60 shrink-0" />
        </>
      )}
    </Button>
  );
};
