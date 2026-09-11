/**
 * The speech-model command wrappers.
 *
 * One place where the `invoke` names live, so a renamed Rust command breaks in
 * this file rather than at runtime in whichever surface happened to call it.
 */

import { invoke } from '@tauri-apps/api/core';
import { listen, type UnlistenFn } from '@tauri-apps/api/event';
import type {
  SpeechModel,
  SpeechModelCatalogue,
  SpeechModelDownloadProgress,
  SpeechModelTier,
} from '@/types/models';

/** The event a download reports progress on. Matches the Rust constant. */
export const SPEECH_MODEL_DOWNLOAD_EVENT = 'speech-model-download';

export const listSpeechModels = (): Promise<SpeechModelCatalogue> =>
  invoke('list_speech_models');

/**
 * Fetches a model. Resolves when the download finishes; watch
 * {@link onSpeechModelDownload} for progress while it runs.
 */
export const downloadSpeechModel = (id: string): Promise<SpeechModel> =>
  invoke('download_speech_model', { id });

export const cancelSpeechModelDownload = (id: string): Promise<boolean> =>
  invoke('cancel_speech_model_download', { id });

export const deleteSpeechModel = (id: string): Promise<boolean> =>
  invoke('delete_speech_model', { id });

/** Chooses the model meetings use. `null` means "whatever is installed". */
export const setMeetingSpeechModel = (id: string | null): Promise<void> =>
  invoke('set_meeting_speech_model', { id });

/** Subscribes to download progress for every model at once. */
export const onSpeechModelDownload = (
  handler: (progress: SpeechModelDownloadProgress) => void,
): Promise<UnlistenFn> =>
  listen<SpeechModelDownloadProgress>(SPEECH_MODEL_DOWNLOAD_EVENT, (event) =>
    handler(event.payload),
  );

/** Bytes as a person reads them. */
export function formatBytes(bytes: number): string {
  if (!Number.isFinite(bytes) || bytes <= 0) return '—';
  if (bytes >= 1024 ** 3) return `${(bytes / 1024 ** 3).toFixed(1)} GB`;
  if (bytes >= 1024 ** 2) return `${Math.round(bytes / 1024 ** 2)} MB`;
  return `${Math.round(bytes / 1024)} KB`;
}

/** The one-word label shown against a tier. */
export const TIER_LABEL: Record<SpeechModelTier, string> = {
  fast: 'Fast',
  balanced: 'Balanced',
  accurate: 'Accurate',
  maximum: 'Maximum',
};

/**
 * How far along a download is, 0–1, or null when the server sent no length.
 *
 * Separate from the component so the "no Content-Length" case is handled in
 * one place: a bar that computes `downloaded / null` renders NaN% and a full
 * bar, which reads as finished.
 */
export function downloadFraction(progress: SpeechModelDownloadProgress): number | null {
  if (progress.state !== 'downloading') return null;
  if (!progress.total_bytes || progress.total_bytes <= 0) return null;
  return Math.min(1, progress.downloaded_bytes / progress.total_bytes);
}
