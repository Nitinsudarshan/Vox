export type ModelStatus =
  | 'ready'
  | 'active'
  | 'available'
  | 'missing'
  | 'unavailable'
  | 'checking';

export interface ModelDescriptor {
  name: string;
  displayName: string;
  provider: 'ollama' | 'cloud_openai' | 'cloud_gemini' | 'cloud_anthropic' | 'whisper';
  type: 'llm' | 'stt';
  installed: boolean;
  available: boolean;
  usable: boolean;
  active: boolean;
  status: ModelStatus;
  details?: string;
  sizeBytes?: number;
  path?: string;
  capabilities?: string[];
}

export interface OllamaModelDetails {
  name: string;
  model: string;
  size?: number | null;
  digest?: string | null;
  modified_at?: string | null;
  parameter_size?: string | null;
  quantization_level?: string | null;
  format?: string | null;
  family?: string | null;
}

export interface OllamaPromptTestResult {
  success: boolean;
  latency_ms: number;
  response?: string | null;
  error?: string | null;
  model: string;
}

export interface SttModelInfo {
  name: string;
  filename: string;
  path: string;
  size_bytes: number;
  exists: boolean;
  is_managed: boolean;
  profile?: 'fast' | 'accurate' | 'custom' | null;
  status: 'ready' | 'available' | 'missing' | string;
}

export interface SttModelsOverview {
  active_model_name: string;
  active_model_path: string;
  active_profile: 'fast' | 'accurate' | 'custom' | string;
  models_dir: string;
  models: SttModelInfo[];
}

export interface SttModelTestResult {
  success: boolean;
  path: string;
  size_bytes: number;
  latency_ms: number;
  error?: string | null;
}

/** What the retained STT decode history says, in the terms the open audio
 *  decisions are posed in. Counts and percentiles only — the history itself
 *  holds no transcript. */
export interface SttDecodeSummary {
  decodes: number;
  failed: number;
  rms_p10: number;
  rms_median: number;
  peak_p10: number;
  peak_median: number;
  quiet_decodes: number;
  pinned_decodes: number;
  auto_detected_decodes: number;
  pinned_words_per_second_median: number;
  auto_words_per_second_median: number;
  starved_decodes: number;
}

/**
 * What a speech model costs to run, in the terms someone choosing one cares
 * about. Mirrors `capture::models::ModelTier`.
 */
export type SpeechModelTier = 'fast' | 'balanced' | 'accurate' | 'maximum';

/** One entry of the speech-model catalogue, crossed with what is on disk. */
export interface SpeechModel {
  id: string;
  name: string;
  filename: string;
  path: string;
  /** Bytes on disk when installed; the catalogue's estimate when not. */
  size_bytes: number;
  installed: boolean;
  /** False for a `.bin` the user dropped into the models folder themselves. */
  managed: boolean;
  multilingual: boolean;
  parameters_millions: number;
  tier: SpeechModelTier;
  blurb: string;
}

/** The whole catalogue, plus what each surface will actually use. */
export interface SpeechModelCatalogue {
  models_dir: string;
  models: SpeechModel[];
  active_meeting_model: string | null;
  active_dictation_model: string | null;
  recommended_meeting_model: string;
}

/**
 * A download's progress, as it arrives on the `speech-model-download` event.
 *
 * `total_bytes` is null when the server sent no `Content-Length` — rare on
 * Hugging Face, but a progress bar that assumes otherwise shows NaN.
 */
export type SpeechModelDownloadProgress =
  | { state: 'downloading'; id: string; downloaded_bytes: number; total_bytes: number | null }
  | { state: 'verifying'; id: string }
  | { state: 'ready'; id: string; path: string }
  | { state: 'failed'; id: string; message: string }
  | { state: 'cancelled'; id: string };
