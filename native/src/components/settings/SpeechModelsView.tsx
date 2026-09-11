import React from 'react';
import {
  AlertTriangle,
  Check,
  Download,
  FolderOpen,
  HardDrive,
  Loader2,
  Mic,
  Trash2,
  X,
} from 'lucide-react';

import { Badge } from '@/components/ui/badge';
import { Button } from '@/components/ui/button';
import { Card } from '@/components/ui/card';
import {
  cancelSpeechModelDownload,
  deleteSpeechModel,
  downloadFraction,
  downloadSpeechModel,
  formatBytes,
  listSpeechModels,
  onSpeechModelDownload,
  setMeetingSpeechModel,
  TIER_LABEL,
} from '@/lib/speechModels';
import type {
  SpeechModel,
  SpeechModelCatalogue,
  SpeechModelDownloadProgress,
  SpeechModelTier,
} from '@/types/models';

/**
 * Settings › Speech — the speech-model manager.
 *
 * This section exists because the app already told users to come here. A
 * meeting that could not find a model failed with "Install one under Settings ›
 * Speech", and there was no such section: the three models the app could fetch
 * were listed inside "AI Models & STT", with no progress while a 1.6 GB file
 * downloaded, no way to remove one, and no way to say which model a meeting
 * should use.
 *
 * The list is grouped by tier rather than shown flat. Twelve Whisper models
 * presented as one column asks every user to understand the difference between
 * `medium.en` and `large-v2` before they can record anything; grouped, the
 * answer to "which one" is the recommended row at the top and the rest is
 * there for whoever wants it.
 */

const TIER_ORDER: SpeechModelTier[] = ['fast', 'balanced', 'accurate', 'maximum'];

const TIER_BLURB: Record<SpeechModelTier, string> = {
  fast: 'Lowest latency. Best for push-to-talk dictation, weakest on accents.',
  balanced: 'The working default — usable accuracy at a latency dictation can afford.',
  accurate: 'Noticeably better on accented and code-switched speech. Best for meetings.',
  maximum: 'The accuracy ceiling, and slow enough on CPU to be a background job.',
};

/** A download in flight, keyed by model id. */
type DownloadState =
  | { kind: 'downloading'; fraction: number | null; downloaded: number }
  | { kind: 'verifying' }
  | { kind: 'failed'; message: string };

export const SpeechModelsView: React.FC = () => {
  const [catalogue, setCatalogue] = React.useState<SpeechModelCatalogue | null>(null);
  const [loading, setLoading] = React.useState(true);
  const [downloads, setDownloads] = React.useState<Record<string, DownloadState>>({});
  const [busyId, setBusyId] = React.useState<string | null>(null);
  const [error, setError] = React.useState<string | null>(null);

  const refresh = React.useCallback(async () => {
    try {
      setCatalogue(await listSpeechModels());
      setError(null);
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      setLoading(false);
    }
  }, []);

  React.useEffect(() => {
    void refresh();
  }, [refresh]);

  // One listener for every model's progress. The event carries the id, so a
  // per-row subscription would be twelve listeners doing the same filtering.
  React.useEffect(() => {
    const unlisten = onSpeechModelDownload((progress: SpeechModelDownloadProgress) => {
      setDownloads((prev) => {
        const next = { ...prev };
        switch (progress.state) {
          case 'downloading':
            next[progress.id] = {
              kind: 'downloading',
              fraction: downloadFraction(progress),
              downloaded: progress.downloaded_bytes,
            };
            break;
          case 'verifying':
            next[progress.id] = { kind: 'verifying' };
            break;
          case 'failed':
            next[progress.id] = { kind: 'failed', message: progress.message };
            break;
          case 'ready':
          case 'cancelled':
            delete next[progress.id];
            break;
        }
        return next;
      });
      if (progress.state === 'ready' || progress.state === 'cancelled') {
        void refresh();
      }
    });
    return () => {
      void unlisten.then((off) => off());
    };
  }, [refresh]);

  const download = async (id: string) => {
    setDownloads((prev) => ({
      ...prev,
      [id]: { kind: 'downloading', fraction: null, downloaded: 0 },
    }));
    try {
      await downloadSpeechModel(id);
    } catch (err) {
      // The event stream reports the reason; this catch keeps a transport
      // failure from leaving the row spinning forever.
      setDownloads((prev) => ({
        ...prev,
        [id]: { kind: 'failed', message: err instanceof Error ? err.message : String(err) },
      }));
    }
    await refresh();
  };

  const remove = async (model: SpeechModel) => {
    setBusyId(model.id);
    try {
      await deleteSpeechModel(model.id);
      await refresh();
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      setBusyId(null);
    }
  };

  const useForMeetings = async (id: string | null) => {
    setBusyId(id ?? '__auto__');
    try {
      await setMeetingSpeechModel(id);
      await refresh();
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      setBusyId(null);
    }
  };

  if (loading) {
    return <p className="text-xs text-muted-foreground">Loading speech models…</p>;
  }

  const models = catalogue?.models ?? [];
  const installedCount = models.filter((m) => m.installed).length;
  const recommended = models.find((m) => m.id === catalogue?.recommended_meeting_model);
  const activeMeeting = catalogue?.active_meeting_model ?? null;

  return (
    <div className="space-y-4 max-w-3xl">
      <div>
        <h2 className="text-lg font-semibold text-foreground">Speech</h2>
        <p className="text-xs text-muted-foreground mt-1">
          The models that turn audio into text. Everything here runs on this machine — a model
          is downloaded once and never sends audio anywhere.
        </p>
      </div>

      {error && (
        <p className="text-xs text-destructive flex items-start gap-1.5">
          <AlertTriangle className="w-3.5 h-3.5 shrink-0 mt-px" />
          {error}
        </p>
      )}

      {installedCount === 0 && (
        <Card className="p-4 border-amber-500/40 bg-amber-500/5">
          <div className="flex items-start gap-3">
            <AlertTriangle className="w-4 h-4 text-amber-500 shrink-0 mt-0.5" />
            <div className="min-w-0 flex-1">
              <p className="text-sm font-medium text-foreground">No speech model installed</p>
              <p className="text-xs text-muted-foreground mt-1">
                Dictation and meeting recording both need one. {recommended?.name} is the
                recommended starting point — it is the best accuracy most machines can decode
                at a sensible speed.
              </p>
              {recommended && (
                <Button
                  size="sm"
                  className="mt-3 gap-2"
                  disabled={Boolean(downloads[recommended.id])}
                  onClick={() => void download(recommended.id)}
                >
                  <Download className="w-3.5 h-3.5" />
                  Download {recommended.name} ({formatBytes(recommended.size_bytes)})
                </Button>
              )}
            </div>
          </div>
        </Card>
      )}

      <Card className="p-4 space-y-3">
        <div className="flex items-start justify-between gap-4">
          <div className="min-w-0">
            <p className="text-sm font-medium text-foreground">Model used for meetings</p>
            <p className="text-xs text-muted-foreground mt-0.5">
              Kept separate from dictation on purpose: push-to-talk is waiting for the words
              and cannot afford a large model, while a meeting is decoded in the background
              and can.
            </p>
          </div>
        </div>
        <div className="flex flex-wrap items-center gap-2">
          <Button
            size="sm"
            variant={activeMeeting === null ? 'default' : 'outline'}
            disabled={busyId !== null}
            onClick={() => void useForMeetings(null)}
          >
            Best installed
          </Button>
          {models
            .filter((m) => m.installed)
            .map((model) => (
              <Button
                key={model.id}
                size="sm"
                variant={activeMeeting === model.id ? 'default' : 'outline'}
                disabled={busyId !== null}
                onClick={() => void useForMeetings(model.id)}
              >
                {model.name}
              </Button>
            ))}
        </div>
      </Card>

      {TIER_ORDER.map((tier) => {
        const rows = models.filter((m) => m.managed && m.tier === tier);
        if (rows.length === 0) return null;
        return (
          <section key={tier} className="space-y-2">
            <div>
              <h3 className="text-sm font-semibold text-foreground">{TIER_LABEL[tier]}</h3>
              <p className="text-xs text-muted-foreground">{TIER_BLURB[tier]}</p>
            </div>
            <div className="space-y-2">
              {rows.map((model) => (
                <ModelRow
                  key={model.id}
                  model={model}
                  download={downloads[model.id]}
                  isActiveForMeetings={activeMeeting === model.id}
                  isRecommended={model.id === catalogue?.recommended_meeting_model}
                  busy={busyId === model.id}
                  onDownload={() => void download(model.id)}
                  onCancel={() => void cancelSpeechModelDownload(model.id)}
                  onDelete={() => void remove(model)}
                />
              ))}
            </div>
          </section>
        );
      })}

      {models.some((m) => !m.managed) && (
        <section className="space-y-2">
          <div>
            <h3 className="text-sm font-semibold text-foreground">Added by hand</h3>
            <p className="text-xs text-muted-foreground">
              GGML files found in the models folder that Vox did not download. Selectable, but
              Vox cannot say how good they are or replace them if they go missing.
            </p>
          </div>
          <div className="space-y-2">
            {models
              .filter((m) => !m.managed)
              .map((model) => (
                <ModelRow
                  key={model.id}
                  model={model}
                  isActiveForMeetings={activeMeeting === model.id}
                  busy={busyId === model.id}
                  onDownload={() => undefined}
                  onCancel={() => undefined}
                  onDelete={() => void remove(model)}
                />
              ))}
          </div>
        </section>
      )}

      {catalogue && (
        <p className="text-[11px] text-muted-foreground flex items-center gap-1.5 pt-1">
          <FolderOpen className="w-3 h-3 shrink-0" />
          <span className="font-mono break-all">{catalogue.models_dir}</span>
        </p>
      )}
    </div>
  );
};

interface ModelRowProps {
  model: SpeechModel;
  download?: DownloadState;
  isActiveForMeetings: boolean;
  isRecommended?: boolean;
  busy: boolean;
  onDownload: () => void;
  onCancel: () => void;
  onDelete: () => void;
}

/** One model: what it is, what it costs, and the one action it currently offers. */
const ModelRow: React.FC<ModelRowProps> = ({
  model,
  download,
  isActiveForMeetings,
  isRecommended,
  busy,
  onDownload,
  onCancel,
  onDelete,
}) => {
  const percent =
    download?.kind === 'downloading' && download.fraction !== null
      ? Math.round(download.fraction * 100)
      : null;

  return (
    <Card className="p-3">
      <div className="flex items-start gap-3">
        <div className="min-w-0 flex-1">
          <div className="flex flex-wrap items-center gap-1.5">
            <span className="text-sm font-medium text-foreground">{model.name}</span>
            {model.installed && (
              <Badge variant="secondary" className="gap-1">
                <Check className="w-3 h-3" />
                Installed
              </Badge>
            )}
            {isActiveForMeetings && (
              <Badge variant="secondary" className="gap-1">
                <Mic className="w-3 h-3" />
                Meetings
              </Badge>
            )}
            {isRecommended && !model.installed && <Badge>Recommended</Badge>}
            {!model.multilingual && <Badge variant="outline">English only</Badge>}
          </div>
          <p className="text-xs text-muted-foreground mt-1">{model.blurb}</p>
          <p className="text-[11px] text-muted-foreground mt-1 flex items-center gap-1.5">
            <HardDrive className="w-3 h-3 shrink-0" />
            {formatBytes(model.size_bytes)}
            {model.parameters_millions > 0 && <> · {model.parameters_millions}M parameters</>}
          </p>

          {download?.kind === 'downloading' && (
            <div className="mt-2">
              <div className="h-1.5 rounded-full bg-muted overflow-hidden">
                <div
                  className="h-full bg-primary transition-[width] duration-300"
                  style={{ width: percent === null ? '100%' : `${percent}%` }}
                />
              </div>
              <p className="text-[11px] text-muted-foreground mt-1">
                {percent === null
                  ? `Downloading… ${formatBytes(download.downloaded)}`
                  : `Downloading… ${percent}% of ${formatBytes(model.size_bytes)}`}
              </p>
            </div>
          )}
          {download?.kind === 'verifying' && (
            <p className="text-[11px] text-muted-foreground mt-2">Verifying…</p>
          )}
          {download?.kind === 'failed' && (
            <p className="text-[11px] text-destructive mt-2">{download.message}</p>
          )}
        </div>

        <div className="shrink-0">
          {download?.kind === 'downloading' || download?.kind === 'verifying' ? (
            <Button size="sm" variant="outline" className="gap-1.5" onClick={onCancel}>
              <X className="w-3.5 h-3.5" />
              Cancel
            </Button>
          ) : model.installed ? (
            <Button
              size="sm"
              variant="ghost"
              className="gap-1.5 text-muted-foreground hover:text-destructive"
              disabled={busy}
              onClick={onDelete}
            >
              {busy ? (
                <Loader2 className="w-3.5 h-3.5 animate-spin" />
              ) : (
                <Trash2 className="w-3.5 h-3.5" />
              )}
              Remove
            </Button>
          ) : (
            <Button size="sm" variant="outline" className="gap-1.5" onClick={onDownload}>
              <Download className="w-3.5 h-3.5" />
              Download
            </Button>
          )}
        </div>
      </div>
    </Card>
  );
};
