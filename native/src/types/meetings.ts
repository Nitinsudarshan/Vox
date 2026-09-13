/**
 * The Meetings vocabulary, mirroring `native/src-tauri/src/meetings/model.rs`.
 *
 * Serde writes these shapes verbatim — there is no DTO layer between the store
 * and the window — so a field that changes here has to change there too.
 */

export type MeetingSource = 'recorded' | 'imported';

export type MeetingState =
  | 'recording'
  | 'paused'
  | 'transcribing'
  | 'completed'
  | 'failed';

/**
 * Which capture channel a transcript line came from.
 *
 * Not diarization: the microphone and the system loopback are two streams, so
 * "was that me or them" is known for free. Two remote participants are both
 * `system` and cannot be told apart.
 */
export type SegmentChannel = 'microphone' | 'system' | 'mixed';

export type SummaryStatus =
  | 'pending'
  | 'processing'
  | 'completed'
  | 'failed'
  | 'cancelled';

export interface TranscriptSegment {
  sequence: number;
  text: string;
  start_seconds: number;
  end_seconds: number;
  channel: SegmentChannel;
  /** Whisper's own no-speech probability for this span. Lower is more certain. */
  no_speech_prob: number;
  recorded_at: string;
  original_text?: string | null;
  romanized_text?: string | null;
  translated_text?: string | null;
}

export interface Meeting {
  id: string;
  title: string;
  created_at: string;
  updated_at: string;
  state: MeetingState;
  source: MeetingSource;
  duration_seconds: number;
  audio_path?: string | null;
  transcript_model?: string | null;
  language?: string | null;
  mic_device?: string | null;
  /** False means only this machine's microphone was recorded. */
  system_audio_captured: boolean;
  segment_count: number;
  /** Segments the pipeline queued but could not decode. */
  dropped_segments: number;
  error?: string | null;
  tags: string[];
}

/** A meeting as the list renders it: the record plus two derived fields. */
export interface MeetingListItem extends Meeting {
  has_summary: boolean;
  summary_status?: SummaryStatus | null;
  preview: string;
}

export interface MeetingSummary {
  meeting_id: string;
  status: SummaryStatus;
  template_id: string;
  markdown?: string | null;
  english_markdown?: string | null;
  previous_markdown?: string | null;
  error?: string | null;
  provider?: string | null;
  model?: string | null;
  language?: string | null;
  fingerprint?: string | null;
  chunk_count: number;
  processing_ms: number;
  started_at?: string | null;
  completed_at?: string | null;
}

export interface MeetingDetail {
  meeting: Meeting;
  segments: TranscriptSegment[];
  summary?: MeetingSummary | null;
  notes: string;
}

export interface MeetingSearchHit {
  meeting_id: string;
  meeting_title: string;
  sequence: number;
  start_seconds: number;
  excerpt: string;
}

export type SectionStyle = 'bullets' | 'checklist' | 'prose';

export interface TemplateSection {
  heading: string;
  instruction: string;
  style: SectionStyle;
  required: boolean;
}

export interface MeetingTemplate {
  id: string;
  name: string;
  description: string;
  sections: TemplateSection[];
  /** True for a template the user supplied rather than one Vox ships. */
  custom: boolean;
}

export interface MeetingRecordingStatus {
  active: boolean;
  meeting_id?: string | null;
  title?: string | null;
  state?: MeetingState | null;
  elapsed_seconds: number;
  microphone_active: boolean;
  system_audio_active: boolean;
  /**
   * Whether anything above the silence floor has been heard.
   * `microphone_heard === false` a minute in is the signal that the wrong
   * device is open — the one users otherwise discover at the end.
   */
  microphone_heard: boolean;
  system_audio_heard: boolean;
  segments_queued: number;
  segments_completed: number;
  segments_dropped: number;
  warning?: string | null;
  /**
   * The devices this recording opened. The heard flags above are how a wrong
   * device is noticed; these are what let the surface name it.
   */
  devices?: OpenedDevices;
}

/** Which microphone and output device a recording should open. */
export interface MeetingDevices {
  /** `null`/absent means the shared microphone preference. */
  microphone?: string | null;
  /** `null`/absent means the OS default output. */
  system_audio?: string | null;
}

/** The devices a running recording actually opened. */
export interface OpenedDevices {
  microphone?: string | null;
  system_audio?: string | null;
}

// --- event payloads -----------------------------------------------------

export interface MeetingLevels {
  mic: number;
  system: number;
}

export interface TranscriptionProgress {
  meeting_id: string;
  queued: number;
  completed: number;
  dropped: number;
  in_queue: number;
}

export interface TranscriptionWarning {
  meeting_id: string;
  kind: string;
  message: string;
}

export interface SummaryProgress {
  meeting_id: string;
  status: SummaryStatus;
  stage: string;
  fraction?: number | null;
}

export interface ImportProgress {
  meeting_id: string;
  stage: string;
  processed_seconds: number;
  total_seconds?: number | null;
  fraction?: number | null;
  segments: number;
}

/** Event names emitted by the Rust meetings module. */
export const MEETING_EVENTS = {
  state: 'meeting-state-changed',
  level: 'meeting-audio-level',
  segment: 'meeting-transcript-segment',
  transcriptionProgress: 'meeting-transcription-progress',
  transcriptionWarning: 'meeting-transcription-warning',
  summaryProgress: 'meeting-summary-progress',
  importProgress: 'meeting-import-progress',
} as const;
