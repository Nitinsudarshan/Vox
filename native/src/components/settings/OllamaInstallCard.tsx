import React from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { AlertTriangle, Check, Download, HardDrive } from 'lucide-react';

import { Button } from '@/components/ui/button';
import { Card } from '@/components/ui/card';
import { formatBytes } from '@/lib/speechModels';
import { describeError } from '@/lib/errors';

/** What installing Ollama on this machine involves. Mirrors the Rust struct. */
interface InstallPlan {
  url: string;
  filename: string;
  kind: 'installer' | 'archive';
  approx_bytes: number;
  description: string;
}

type InstallProgress =
  | { state: 'downloading'; downloaded_bytes: number; total_bytes: number | null }
  | { state: 'preparing' }
  | { state: 'awaiting_user' }
  | { state: 'ready' }
  | { state: 'failed'; message: string };

type OllamaStatus =
  | { state: 'running' }
  | { state: 'started' }
  | { state: 'not_installed' }
  | { state: 'unreachable'; message: string };

/**
 * Installs Ollama, when it is missing.
 *
 * The state this replaces was a dead end: `OllamaStatus::NotInstalled` reached
 * the UI as "not installed" and nothing else, so a user who wanted summaries
 * on their own machine had to work out on their own that Ollama existed and
 * where to get it.
 *
 * Renders nothing at all once Ollama is running. A settings page that keeps
 * offering to install something already installed is noise.
 */
export const OllamaInstallCard: React.FC = () => {
  const [status, setStatus] = React.useState<OllamaStatus | null>(null);
  const [plan, setPlan] = React.useState<InstallPlan | null>(null);
  const [progress, setProgress] = React.useState<InstallProgress | null>(null);

  const refresh = React.useCallback(async () => {
    try {
      setStatus(await invoke<OllamaStatus>('ensure_local_llm_ready'));
    } catch {
      setStatus(null);
    }
  }, []);

  React.useEffect(() => {
    void refresh();
    void invoke<InstallPlan | null>('get_ollama_install_plan')
      .then(setPlan)
      .catch(() => setPlan(null));
  }, [refresh]);

  React.useEffect(() => {
    const unlisten = listen<InstallProgress>('ollama-install', (event) => {
      setProgress(event.payload);
      if (event.payload.state === 'ready') void refresh();
    });
    return () => {
      void unlisten.then((off) => off());
    };
  }, [refresh]);

  if (!status || status.state === 'running' || status.state === 'started') return null;

  const percent =
    progress?.state === 'downloading' && progress.total_bytes
      ? Math.round((progress.downloaded_bytes / progress.total_bytes) * 100)
      : null;
  const busy =
    progress?.state === 'downloading' || progress?.state === 'preparing';

  return (
    <Card className="p-4 border-amber-500/40 bg-amber-500/5">
      <div className="flex items-start gap-3">
        <AlertTriangle className="w-4 h-4 text-amber-500 shrink-0 mt-0.5" />
        <div className="min-w-0 flex-1">
          <p className="text-sm font-medium text-foreground">
            {status.state === 'not_installed'
              ? 'Ollama is not installed'
              : 'Ollama is not responding'}
          </p>
          <p className="text-xs text-muted-foreground mt-1">
            {status.state === 'not_installed'
              ? 'It runs language models on this machine, which is how meeting reports are written without sending a transcript anywhere.'
              : status.message}
          </p>

          {progress?.state === 'awaiting_user' && (
            <p className="text-xs text-foreground mt-2">
              The installer is open. Finish it there and Vox will pick Ollama up — no restart
              needed.
            </p>
          )}
          {progress?.state === 'ready' && (
            <p className="flex items-center gap-1.5 text-xs text-emerald-600 dark:text-emerald-400 mt-2">
              <Check className="w-3.5 h-3.5" />
              Installed.
            </p>
          )}
          {progress?.state === 'failed' && (
            <p className="text-xs text-destructive mt-2">{progress.message}</p>
          )}

          {busy && (
            <div className="mt-3">
              <div className="h-1.5 rounded-full bg-muted overflow-hidden">
                <div
                  className="h-full bg-primary transition-[width] duration-300"
                  style={{ width: percent === null ? '100%' : `${percent}%` }}
                />
              </div>
              <p className="text-[11px] text-muted-foreground mt-1">
                {progress?.state === 'preparing'
                  ? 'Preparing…'
                  : percent === null
                    ? 'Downloading…'
                    : `Downloading… ${percent}%`}
              </p>
            </div>
          )}

          {plan && !busy && progress?.state !== 'ready' && (
            <div className="mt-3 space-y-1.5">
              <Button
                size="sm"
                className="gap-2"
                onClick={() => {
                  setProgress({ state: 'downloading', downloaded_bytes: 0, total_bytes: null });
                  void invoke('install_ollama').catch((err) =>
                    setProgress({
                      state: 'failed',
                      message: describeError(err),
                    }),
                  );
                }}
              >
                <Download className="w-3.5 h-3.5" />
                Install Ollama
              </Button>
              <p className="flex items-center gap-1.5 text-[11px] text-muted-foreground">
                <HardDrive className="w-3 h-3 shrink-0" />
                About {formatBytes(plan.approx_bytes)} · {plan.description}
              </p>
            </div>
          )}
          {!plan && (
            <p className="text-xs text-muted-foreground mt-2">
              Vox cannot install it on this platform — get it from ollama.com.
            </p>
          )}
        </div>
      </div>
    </Card>
  );
};
