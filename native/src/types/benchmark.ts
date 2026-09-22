/**
 * Dictation Test Lab benchmarking types.
 */

export interface AvailableModelTarget {
  target_id: string;
  engine_id: string;
  engine_display_name: string;
  model_name: string;
  model_filename: string;
  model_path: string;
  installed: boolean;
  compiled_in: boolean;
  capabilities: {
    batch: boolean;
    streaming: boolean;
    segment_timestamps: 'yes' | 'no' | 'unknown';
    word_timestamps: 'yes' | 'no' | 'unknown';
    language: { kind: 'fixed'; language: string } | { kind: 'selectable'; detects: boolean };
    translation: 'yes' | 'no' | 'unknown';
    no_speech_evidence: boolean;
    partial_transcripts: boolean;
    local: boolean;
    requires_network: boolean;
  };
  language_support: { kind: 'fixed'; language: string } | { kind: 'selectable'; detects: boolean };
  parameters_millions?: number | null;
  size_bytes?: number | null;
  tier?: string | null;
  blurb: string;
}

export interface CleanupStyleInfo {
  id: string;
  display_name: string;
  description: string;
  is_default: boolean;
}

export interface DiffSpan {
  kind: 'same' | 'removed' | 'added';
  text: string;
}

export interface CleanupResult {
  style: string;
  style_display_name: string;
  cleaned_text: string;
  changed: boolean;
  queue_wait_ms?: number;
  duration_ms: number;
  raw_char_count: number;
  raw_word_count: number;
  cleaned_char_count: number;
  cleaned_word_count: number;
  diff_spans: DiffSpan[];
  error?: string | null;
}

export interface DictationModelTimings {
  recording_to_audio_ready_ms: number;
  queue_wait_ms: number;
  model_load_ms: number;
  is_cold_load: boolean;
  stt_execution_ms: number;
  stt_to_text_available_ms: number;
  lock_wait_ms?: number;
  recognizer_call_duration_ms?: number;
  production_cleanup_style?: string;
  production_cleanup_duration_ms?: number;
  production_e2e_ms?: number;
  production_e2e_by_style?: Record<string, number>;
  cleanup_benchmark_total_ms?: number;
  benchmark_wall_clock_ms?: number;
  total_cleanup_ms: number;
  total_latency_ms: number;
  rtf: number;
  timeline_queue_start_ms?: number;
  timeline_model_start_ms?: number;
  timeline_stt_start_ms?: number;
  timeline_stt_end_ms?: number;
  timeline_model_end_ms?: number;
  recording_stop_ts: string;
  audio_ready_ts: string;
  model_start_ts?: string;
  stt_start_ts: string;
  stt_end_ts: string;
  text_available_ts: string;
  test_complete_ts: string;
  time_to_first_partial_ms?: number | null;
  time_to_first_stable_partial_ms?: number | null;
  streaming_finalization_ms?: number | null;
}

export interface ModelExecutionConfig {
  engine: string;
  model_name: string;
  model_filename: string;
  language?: string | null;
  task: string;
  decoding_strategy: string;
  temperature: number;
  initial_prompt?: string | null;
  threads?: number | null;
  backend: string;
}

export interface AccuracyMetrics {
  reference: string;
  hypothesis: string;
  word_count: number;
  substitutions: number;
  deletions: number;
  insertions: number;
  wer: number;
  cer: number;
  technical_term_accuracy?: number | null;
  technical_term_errors: Array<{ expected: string; actual_found?: string | null }>;
}

export interface DictationTestModelResult {
  target_id: string;
  engine_id: string;
  model_name: string;
  config: ModelExecutionConfig;
  success: boolean;
  error?: string | null;
  raw_transcript: string;
  cleanup_results: Record<string, CleanupResult>;
  timings: DictationModelTimings;
  accuracy?: AccuracyMetrics | null;
  char_count: number;
  word_count: number;
  segment_count: number;
}

export interface BenchmarkAudioMetadata {
  audio_path: string;
  filename: string;
  duration_seconds: f32_alias;
  sample_rate: number;
  channels: number;
  format: string;
  sample_count: number;
}

type f32_alias = number;

export interface BenchmarkEnvironmentMetadata {
  os: string;
  vox_version: string;
  platform: string;
  cleanup_provider: string;
  cleanup_model: string;
}

export interface DictationTestRun {
  schema_version: number;
  test_id: string;
  label?: string | null;
  created_at_epoch_ms: number;
  created_at_formatted: string;
  audio: BenchmarkAudioMetadata;
  environment: BenchmarkEnvironmentMetadata;
  reference_transcript?: string | null;
  selected_models: string[];
  selected_cleanup_styles: string[];
  model_results: DictationTestModelResult[];
  total_run_duration_ms: number;
}

export interface DictationTestSummary {
  test_id: string;
  created_at_epoch_ms: number;
  created_at_formatted: string;
  label?: string | null;
  audio_duration_seconds: number;
  models_tested_count: number;
  successful_models_count: number;
  fastest_model_name?: string | null;
  fastest_total_ms?: number | null;
  cleanup_styles: string[];
  has_reference_transcript: boolean;
}

export interface DictationTestProgressPayload {
  test_id: string;
  current_model_index: number;
  total_models: number;
  completed_model: DictationTestModelResult;
}

export interface RunDictationTestRequest {
  selected_target_ids: string[];
  selected_cleanup_styles: string[];
  production_cleanup_style?: string | null;
  reference_transcript?: string | null;
  test_label?: string | null;
  language_override?: string | null;
  concurrency?: number | null;
}
