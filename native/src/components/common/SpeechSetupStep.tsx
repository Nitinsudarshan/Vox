import React from 'react';
import { AlertTriangle, Check, Download, Mic, X } from 'lucide-react';

import { Button } from '@/components/ui/button';
import {
  cancelSpeechModelDownload,
  downloadFraction,
  downloadSpeechModel,
  formatBytes,
  listSpeechModels,
  onSpeechModelDownload,
} from '@/lib/speechModels';
import { listInputDevices } from '@/lib/meetings';
import type { SpeechModel, SpeechModelCatalogue } from '@/types/models';
import { describeError } from '@/lib/errors';

interface SpeechSetupStepProps {
  /** Called when the user is done here, whether or not a model was installed. */
  onDone: () => void;
}

/**
 * The last step of first-run setup: get a speech model onto the machine.
 *
 * Onboarding used to end at "local or account", which left the user in an app
 * that could neither dictate nor record — the first press of either returned
 * an error about a model nobody had told them they needed. The download is
 * hundreds of megabytes and the app is useless without it, so it belongs at
 * the point where the user is already waiting, not behind a failure later.
 *
 * Skippable, and says so. Somebody on a metered connection should be able to
 * get into the app and come back to this; the surfaces that need a model offer
 * the same install when they are reached.
 */
export const SpeechSetupStep: React.FC<SpeechSetupStepProps> = ({ onDone }) => {
  const [catalogue, setCatalogue] = React.useState<SpeechModelCatalogue | null>(null);
  const [microphones, setMicrophones] = React.useState<number | null>(null);
  const [progress, setProgress] = React.useState<{ id: string; fraction: number | null } | null>(
    null,
  );
  const [error, setError] = React.useState<string | null>(null);

  const refresh = React.useCallback(async () => {
    try {
      setCatalogue(await listSpeechModels());
    } catch {
      setCatalogue(null);
    }
  }, []);

  React.useEffect(() => {
    void refresh();
    void listInputDevices()
      .then((devices) => setMicrophones(devices.length))
      .catch(() => setMicrophones(null));
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
          void refresh();
          break;
        case 'cancelled':
          setProgress(null);
          break;
      }
    });
    return () => {
      void unlisten.then((off) => off());
    };
  }, [refresh]);

  const installed = catalogue?.models.filter((model) => model.installed) ?? [];
  const ready = installed.length > 0;
  const recommended: SpeechModel | undefined =
    catalogue?.models.find((m) => m.id === catalogue.recommended_meeting_model) ??
    catalogue?.models.find((m) => m.managed);
  const percent = progress?.fraction === null ? null : Math.round((progress?.fraction ?? 0) * 100);

  return (
    <div className="space-y-5 relative z-10 animate-in fade-in-50">
      <div className="space-y-2">
        {/* Checked rather than asserted: an app that says "you will need a
            microphone" is less useful than one that says whether you have one. */}
        <SetupRow
          ok={microphones !== null && microphones > 0}
          pending={microphones === null}
          icon={Mic}
          title={
            microphones === null
              ? 'Checking for a microphone…'
              : microphones > 0
                ? `${microphones} microphone${microphones === 1 ? '' : 's'} found`
                : 'No microphone found'
          }
          detail={
            microphones === 0
              ? 'Vox records from a microphone. Connect one and it will appear without a restart.'
              : 'Windows asks for permission the first time Vox records.'
          }
        />

        <SetupRow
          ok={ready}
          pending={catalogue === null}
          icon={Download}
          title={
            ready
              ? `Speech model ready — ${installed[0].name}`
              : 'No speech model installed yet'
          }
          detail={
            ready
              ? 'Dictation and meeting recording both use it. Everything is transcribed on this machine.'
              : 'Dictation and meeting recording both need one. It is downloaded once and no audio ever leaves this machine.'
          }
        />
      </div>

      {error && (
        <p className="flex items-start gap-1.5 text-xs text-destructive">
          <AlertTriangle className="w-3.5 h-3.5 shrink-0 mt-px" />
          {error}
        </p>
      )}

      {progress && (
        <div>
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
              variant="ghost"
              size="sm"
              className="h-7 gap-1.5 text-[11px]"
              onClick={() => void cancelSpeechModelDownload(progress.id)}
            >
              <X className="w-3 h-3" />
              Cancel
            </Button>
          </div>
        </div>
      )}

      <div className="space-y-2">
        {!ready && !progress && recommended && (
          <Button
            className="w-full h-11 text-xs font-semibold gap-2"
            onClick={() => {
              setError(null);
              setProgress({ id: recommended.id, fraction: null });
              void downloadSpeechModel(recommended.id).catch((err) => {
                setProgress(null);
                setError(describeError(err));
              });
            }}
          >
            <Download className="w-4 h-4" />
            Download {recommended.name} ({formatBytes(recommended.size_bytes)})
          </Button>
        )}

        <Button
          variant={ready ? 'default' : 'outline'}
          className="w-full h-11 text-xs font-semibold"
          onClick={onDone}
          disabled={Boolean(progress)}
        >
          {ready ? 'Start using Vox' : 'Skip for now'}
        </Button>
        {!ready && (
          <p className="text-[10px] text-center text-muted-foreground">
            You can install one later under Settings › Speech.
          </p>
        )}
      </div>
    </div>
  );
};

interface SetupRowProps {
  ok: boolean;
  pending: boolean;
  icon: React.ComponentType<{ className?: string }>;
  title: string;
  detail: string;
}

/** One checked prerequisite. */
const SetupRow: React.FC<SetupRowProps> = ({ ok, pending, icon: Icon, title, detail }) => (
  <div className="flex items-start gap-3 p-3 rounded-lg border border-border/60 bg-muted/30">
    <span className="shrink-0 mt-0.5">
      {pending ? (
        <Icon className="w-4 h-4 text-muted-foreground" />
      ) : ok ? (
        <Check className="w-4 h-4 text-emerald-500" />
      ) : (
        <AlertTriangle className="w-4 h-4 text-amber-500" />
      )}
    </span>
    <span className="min-w-0">
      <span className="block text-xs font-semibold text-foreground">{title}</span>
      <span className="block text-[11px] text-muted-foreground leading-relaxed mt-0.5">
        {detail}
      </span>
    </span>
  </div>
);
