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

/**
 * Which pass produced a transcript.
 *
 * They optimize for different things over the same audio: a live pass races a
 * clock and must average faster than real time, a final pass has no clock and
 * can afford a wider beam and a bigger model.
 */
export type TranscriptionPass = 'live' | 'final';

/** What produced the transcript currently on disk. */
export interface TranscriptProvenance {
  pass: TranscriptionPass;
  /** Recognizer id — `whisper`, `parakeet`. */
  engine: string;
  /** Model filename, never its path. */
  model: string;
  language?: string | null;
  /** Decode profile, in the engine's own terms. */
  profile: string;
  completed_at: string;
}

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
  /** Which `Speaker` this line was attributed to, if any. */
  speaker_id?: string | null;
}

/**
 * One person Vox believes spoke during a meeting.
 *
 * A proposal, not an assertion: the grouping comes from acoustic statistics,
 * not a trained speaker model, so every speaker carries a span of the
 * recording the user can play in order to put a name to the voice.
 */
export interface Speaker {
  id: string;
  label: string;
  /** Whether `label` is the user's word or Vox's placeholder. */
  named_by_user: boolean;
  channel: SegmentChannel;
  sample_start_seconds: number;
  sample_end_seconds: number;
  segment_count: number;
  speaking_seconds: number;
}

/** What a speaker-detection run found. */
export interface SpeakerReport {
  speakers: Speaker[];
  attributed: number;
  unattributed: number;
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
  /**
   * What produced the transcript currently on disk. Absent for a meeting
   * transcribed before this was recorded, and for one still recording.
   */
  transcript?: TranscriptProvenance | null;
  language?: string | null;
  mic_device?: string | null;
  /** False means only this machine's microphone was recorded. */
  system_audio_captured: boolean;
  segment_count: number;
  /** Segments the pipeline queued but could not decode. */
  dropped_segments: number;
  error?: string | null;
  tags: string[];
  /**
   * The recurring meeting this recording is one of, once it is known.
   *
   * Written during a calendar sync from the event the recording matched, or
   * set by hand. `null` for a one-off.
   */
  series_id?: string | null;
}

/** Where a recurring meeting's identity came from. */
export type SeriesSource = 'google' | 'manual';

/** A recurring meeting. */
export interface MeetingSeries {
  /** `google:<recurringEventId>` or `manual:<slug>-<timestamp>`. */
  id: string;
  title: string;
  source: SeriesSource;
  created_at: string;
  /** Whether the user named it, so a sync leaves the name alone. */
  renamed_by_user: boolean;
}

/** A series as the picker lists one. */
export interface MeetingSeriesSummary extends MeetingSeries {
  occurrence_count: number;
  latest_at?: string | null;
}

/** One recording's place in its series. */
export interface SeriesOccurrence {
  meeting_id: string;
  title: string;
  created_at: string;
  duration_seconds: number;
  has_summary: boolean;
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
  /**
   * Empty until detection runs, which is not an error — and absent entirely
   * for a meeting stored before speakers existed, which is why this is
   * optional rather than merely empty.
   */
  speakers?: Speaker[];
  /**
   * How the recording and the transcription went. Absent for a meeting
   * recorded before diagnostics existed, or one that never finished — both
   * ordinary, so the surface renders nothing rather than claiming zeroes.
   */
  diagnostics?: MeetingDiagnostics | null;
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
  recordingWarning: 'meeting-recording-warning',
  summaryProgress: 'meeting-summary-progress',
  importProgress: 'meeting-import-progress',
  translationProgress: 'meeting-translation-progress',
} as const;

/**
 * Something went wrong with the *recording*, as distinct from the transcript.
 *
 * Kept separate because a transcript can be regenerated from a recording and a
 * recording cannot be regenerated from anything, so these are the more serious
 * of the two and must not be mixed in with decoder complaints.
 */
export interface RecordingWarning {
  /** `microphone`, `system_audio`, `audio_storage` or `audio_shed`. */
  kind: string;
  message: string;
}

/** How far a transcript translation has got. */
export interface TranslationProgress {
  meeting_id: string;
  completed: number;
  total: number;
  fraction: number;
}

// --- reminders -----------------------------------------------------------

/** The Tauri event a reminder card arrives on. */
export const MEETING_REMINDER_EVENT = 'meeting-reminder';

/**
 * Why a reminder exists.
 *
 * Mirrors `native/src-tauri/src/calendar/reminders/mod.rs::ReminderKind`.
 * - `upcoming` — a scheduled meeting is about to start
 * - `unrecorded` — a scheduled meeting has started and nothing is recording it
 * - `detected` — a conferencing call is on screen the calendar knows nothing about
 */
export type ReminderKind = 'upcoming' | 'unrecorded' | 'detected';

/**
 * One conferencing window detection can see on screen.
 *
 * Mirrors `calendar::reminders::detection::WindowMatch`. Only Settings ›
 * Developer reads this: the reminder pipeline weighs these itself and hands
 * the frontend a finished card, never the raw sightings.
 */
export interface ConferencingWindowMatch {
  /** `google_meet`, `zoom`, `teams`, `webex` or `other`. */
  provider: string;
  /** The title with the app's own chrome stripped off. */
  title: string;
  /** Exactly what the window reported, for telling a bad match from a bad title. */
  raw_title: string;
  /** Which signal saw it: `window_title` or `window_class`. */
  source: string;
  /** How specific the match is, 0–1. A bare "Zoom Meeting" scores low. */
  confidence: number;
  /**
   * Why this window would not raise a reminder right now; absent when it
   * would. The same gates the real loop applies, in the same order, so a
   * reminder that never arrives can be told from one that was never due.
   */
  blocked_by?: string | null;
}

/** What the reminder card is given. Sanitized in Rust; nothing else crosses. */
export interface MeetingReminderPayload {
  /** `cal:<event id>` or `win:<provider>:<title>`. Never a recording's id. */
  key: string;
  kind: ReminderKind;
  title: string;
  provider: string;
  provider_name: string;
  /** When the meeting starts, in words — "Starts in 4 minutes". */
  time_label: string;
  participants: string[];
  /** Whether there is a conferencing link behind the Join button. */
  can_join: boolean;
}

// --- diagnostics ---------------------------------------------------------

/** What happened to one decoded segment. */
export type SegmentStatus = 'kept' | 'discarded' | 'failed';

/**
 * Why a turn ended. `ceiling` means it was cut mid-speech and the next
 * segment continues the same sentence.
 */
export type TurnEnd = 'silence' | 'ceiling' | 'flush';

/**
 * One segment's journey from the segmenter to the transcript.
 *
 * Every duration is milliseconds and they are deliberately separate: a single
 * total cannot tell "the model is slow" from "something else held the model"
 * from "the queue was behind", and those need three different fixes.
 *
 * There is no confidence field. What Whisper reports is `no_speech_prob`, and
 * an engine that reports none sends none rather than a stand-in.
 */
export interface SegmentDiagnostics {
  version: number;
  /** Joins to `TranscriptSegment.sequence`, including for segments that never produced one. */
  sequence: number;
  start_seconds: number;
  end_seconds: number;
  channel: SegmentChannel;
  forced_split: boolean;
  /** Why the turn ended. */
  end_reason: TurnEnd;
  /**
   * Quiet the segmenter waited through before judging the turn over. The part
   * of a line's latency that no decoder can remove.
   */
  hangover_ms: number;
  voiced_seconds: number;
  total_seconds: number;
  no_speech_prob?: number | null;
  queue_wait_ms: number;
  lock_wait_ms: number;
  model_load_ms: number;
  model_reloaded: boolean;
  decode_ms: number;
  post_ms: number;
  persist_ms: number;
  model: string;
  language?: string | null;
  expensive_script_profile: boolean;
  audio_ctx?: number | null;
  status: SegmentStatus;
  /** The `speech_health` reason key, for a discarded segment. */
  rejection?: string | null;
  error?: string | null;
  text_chars: number;
  queue_depth_after: number;
}

/**
 * How the recording itself went.
 *
 * Separate from {@link TranscriptionHealth} because they fail for different
 * reasons: a microphone that heard nothing is a device problem, a backlog is a
 * model problem. `opened && !heard` is the signature of the wrong device.
 */
export interface CaptureHealth {
  recording_seconds: number;
  microphone_opened: boolean;
  system_audio_opened: boolean;
  microphone_heard: boolean;
  system_audio_heard: boolean;
  audio_checkpoints_written: boolean;
  checkpoint_failures: number;
  /**
   * Audio that was captured and never reached the writer. Should be 0 on any
   * ordinary recording; non-zero is a hole in the recording itself, which is
   * worse than anything on the transcription side because nothing can
   * regenerate it.
   */
  audio_lost_seconds: number;
}

/** How transcription went. */
export interface TranscriptionHealth {
  segments_emitted: number;
  segments_kept: number;
  segments_discarded: number;
  segments_failed: number;
  /** Speech that exists in the recording and not in the transcript. */
  segments_dropped: number;
  peak_queue_depth: number;
  speech_seconds: number;
  transcribed_seconds: number;
  mean_decode_rtf: number;
  worst_decode_rtf: number;
  worst_decode_sequence: number;
  /**
   * Decode time against wall-clock recording length. Above 1.0 the backlog
   * grows; below it nothing accumulates.
   */
  pipeline_rtf: number;
  finalization_p50_ms: number;
  finalization_p95_ms: number;
  finalization_max_ms: number;
  /**
   * Median quiet waited through before a turn was judged over. Read beside the
   * finalization percentiles: together they say how much of the wait is the
   * decoder and how much is the hangover, and no model shrinks the second.
   */
  hangover_p50_ms: number;
  /** Turns cut at the ceiling — each is a sentence split across two lines. */
  segments_forced_split: number;
  lock_wait_ms_total: number;
  model_load_ms_total: number;
  model_reloads: number;
  drain_seconds: number;
  /** False when the drain gave up on a backlog it could not clear. */
  drain_completed: boolean;
  /** Discarded segments by `speech_health` reason key. */
  rejections: Record<string, number>;
}

/** One meeting's diagnostics rollup. */
export interface MeetingDiagnostics {
  version: number;
  meeting_id: string;
  completed_at: string;
  /** Model filename, never its path. */
  model: string;
  language?: string | null;
  strategy: string;
  threads: string;
  /** Each decode-profile change, as [first segment on the new profile, is the cheaper one]. */
  profile_switches: [number, boolean][];
  segments_on_cheap_profile: number;
  capture: CaptureHealth;
  transcription: TranscriptionHealth;
}
