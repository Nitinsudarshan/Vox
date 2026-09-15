/**
 * The Meetings command wrappers.
 *
 * One place where `invoke` names live, so a renamed Rust command breaks at
 * compile time in one file rather than at runtime in six components.
 */

import { invoke } from '@tauri-apps/api/core';
import type {
  Meeting,
  MeetingDetail,
  MeetingDevices,
  MeetingListItem,
  MeetingRecordingStatus,
  MeetingSearchHit,
  MeetingSeries,
  MeetingSeriesSummary,
  MeetingSummary,
  MeetingTemplate,
  ReminderKind,
  ReminderSettings,
  SeriesOccurrence,
  Speaker,
  SpeakerReport,
  TranscriptSegment,
} from '@/types/meetings';
import type { AudioDeviceInfo } from '@/types';

/** What a failed meeting command reports. */
export interface MeetingCommandError {
  code: string;
  message: string;
}

/**
 * Whether an error came back from a meeting command rather than from the
 * bridge. Anything else is a transport failure and has no code to branch on.
 */
export function isMeetingError(error: unknown): error is MeetingCommandError {
  return (
    typeof error === 'object' &&
    error !== null &&
    typeof (error as MeetingCommandError).code === 'string' &&
    typeof (error as MeetingCommandError).message === 'string'
  );
}

/** A message worth showing the user, whatever shape the failure arrived in. */
export function meetingErrorMessage(error: unknown): string {
  if (isMeetingError(error)) return error.message;
  if (error instanceof Error) return error.message;
  if (typeof error === 'string') return error;
  return 'Something went wrong.';
}

export const startMeeting = (
  title?: string,
  captureSystemAudio?: boolean,
  devices?: MeetingDevices,
): Promise<Meeting> => invoke('start_meeting', { title, captureSystemAudio, devices });

/** The saved microphone/output choice for recordings. */
export const getMeetingDevices = (): Promise<MeetingDevices> => invoke('get_meeting_devices');

/**
 * Remembers which devices recordings should open.
 *
 * A dedicated command rather than a whole-settings save: that round trip lets
 * a stale copy of the document overwrite whatever else changed meanwhile.
 */
export const saveMeetingDevices = (devices: MeetingDevices): Promise<void> =>
  invoke('set_meeting_devices', { devices });

/** Microphones, as the recorder's picker lists them. */
export const listInputDevices = (): Promise<AudioDeviceInfo[]> => invoke('get_audio_devices');

/**
 * Output devices, for the loopback source.
 *
 * A separate command because a meeting is the only recording where the output
 * matters: the far end of a call arrives through whichever device the user is
 * listening on.
 */
export const listOutputDevices = (): Promise<AudioDeviceInfo[]> =>
  invoke('get_audio_output_devices');

export const pauseMeeting = (): Promise<void> => invoke('pause_meeting');
export const resumeMeeting = (): Promise<void> => invoke('resume_meeting');
export const stopMeeting = (): Promise<Meeting> => invoke('stop_meeting');

export const getRecordingStatus = (): Promise<MeetingRecordingStatus> =>
  invoke('get_meeting_recording_status');

export const listMeetings = (): Promise<MeetingListItem[]> => invoke('list_meetings');

export const getMeeting = (meetingId: string): Promise<MeetingDetail> =>
  invoke('get_meeting', { meetingId });

export const searchMeetings = (
  query: string,
  limit?: number,
): Promise<MeetingSearchHit[]> => invoke('search_meetings', { query, limit });

export const renameMeeting = (meetingId: string, title: string): Promise<Meeting> =>
  invoke('rename_meeting', { meetingId, title });

export const saveMeetingNotes = (meetingId: string, notes: string): Promise<void> =>
  invoke('save_meeting_notes', { meetingId, notes });

export const deleteMeeting = (meetingId: string): Promise<void> =>
  invoke('delete_meeting', { meetingId });

export const openMeetingFolder = (meetingId: string): Promise<void> =>
  invoke('open_meeting_folder', { meetingId });

export const listTemplates = (): Promise<MeetingTemplate[]> =>
  invoke('list_meeting_templates');

export const generateSummary = (args: {
  meetingId: string;
  templateId?: string;
  language?: string;
  instructions?: string;
  force?: boolean;
}): Promise<void> => invoke('generate_meeting_summary', args);

export const cancelSummary = (meetingId: string): Promise<boolean> =>
  invoke('cancel_meeting_summary', { meetingId });

export const getSummary = (meetingId: string): Promise<MeetingSummary | null> =>
  invoke('get_meeting_summary', { meetingId });

export const saveSummary = (
  meetingId: string,
  markdown: string,
): Promise<MeetingSummary> => invoke('save_meeting_summary', { meetingId, markdown });

// --- reminders -----------------------------------------------------------

/** When Vox announces that a meeting is about to start. */
export const getReminderSettings = (): Promise<ReminderSettings> =>
  invoke('get_meeting_reminder_settings');

/**
 * Raises a sample reminder, through the real path.
 *
 * Reminders only fire in the last few minutes before a meeting, so the only
 * way to find out whether they work at all was to have a meeting and wait for
 * it — and one that never appears looks exactly like a day with nothing due.
 */
export const sendTestReminder = (kind: ReminderKind): Promise<void> =>
  invoke('send_test_meeting_reminder', { kind });

/** Saves them. Unknown lead times are dropped rather than rejected. */
export const saveReminderSettings = (
  reminders: ReminderSettings,
): Promise<ReminderSettings> => invoke('set_meeting_reminder_settings', { reminders });

// --- recurring meeting series -------------------------------------------

/** Every recurring meeting, most recently active first. */
export const listMeetingSeries = (): Promise<MeetingSeriesSummary[]> =>
  invoke('list_meeting_series');

/** The recordings in one series, oldest first — a series reads forwards. */
export const getMeetingSeries = (seriesId: string): Promise<SeriesOccurrence[]> =>
  invoke('get_meeting_series', { seriesId });

/** Creates a series by hand, for recordings no calendar event covers. */
export const createMeetingSeries = (title: string): Promise<MeetingSeries> =>
  invoke('create_meeting_series', { title });

/** Renames a series. The name then survives every later sync. */
export const renameMeetingSeries = (
  seriesId: string,
  title: string,
): Promise<MeetingSeries[]> => invoke('rename_meeting_series', { seriesId, title });

/** Forgets a series. Its recordings stay; they stop being in one. */
export const deleteMeetingSeries = (seriesId: string): Promise<void> =>
  invoke('delete_meeting_series', { seriesId });

/** Puts a recording in a series, or `null` to take it out of the one it is in. */
export const setMeetingSeries = (
  meetingId: string,
  seriesId: string | null,
): Promise<Meeting> => invoke('set_meeting_series', { meetingId, seriesId });

export const promoteToScribble = (meetingId: string): Promise<{ id: string }> =>
  invoke('promote_meeting_to_scribble', { meetingId });

export const audioExtensions = (): Promise<string[]> => invoke('meeting_audio_extensions');

/** Opens the OS file picker. Resolves to `null` when the user cancels. */
export const pickAudioFile = (): Promise<string | null> =>
  invoke('pick_meeting_audio_file');

export const importAudio = (path: string, title?: string): Promise<Meeting> =>
  invoke('import_meeting_audio', { path, title });

/**
 * What one re-transcription may override about the saved settings.
 *
 * All optional. A re-transcription with no overrides repeats what settings
 * say — which is the right default for "the model finished downloading" and
 * the wrong one for "this transcript is wrong", hence the dialog that fills
 * these in.
 */
export interface RetranscribeOverrides {
  /** ISO code to pin, or `'auto'` to force detection. */
  language?: string;
  /** A speech-model catalogue id, for this run only. */
  modelId?: string;
  /** `'fast' | 'balanced' | 'quality'`. */
  preset?: string;
  /** Whether the run also decodes an English rendering of every line. */
  englishTrack?: boolean;
}

export const retranscribeMeeting = (
  meetingId: string,
  overrides?: RetranscribeOverrides,
): Promise<Meeting> => invoke('retranscribe_meeting', { meetingId, overrides });

export const cancelImport = (key: string): Promise<boolean> =>
  invoke('cancel_meeting_import', { key });

export const translateTranscript = (
  meetingId: string,
  targetLanguage?: string,
): Promise<TranscriptSegment[]> =>
  invoke('translate_meeting_transcript', { meetingId, targetLanguage });

/**
 * Decodes the recording again in Whisper's translate mode.
 *
 * The primary route to an English view, and not a model call over the finished
 * transcript: Whisper reads the audio, so it is not limited by how good the
 * native-script transcript turned out. Leaves that transcript untouched.
 * Resolves to the number of lines that gained English.
 */
export const generateEnglishTrack = (
  meetingId: string,
  overrides?: RetranscribeOverrides,
): Promise<number> => invoke('generate_meeting_english_track', { meetingId, overrides });

/**
 * Works out who spoke, and returns a voice sample for each so the user can put
 * a name to them.
 */
export const detectSpeakers = (meetingId: string): Promise<SpeakerReport> =>
  invoke('detect_meeting_speakers', { meetingId });

/** Puts a name to one of the voices detection found. */
export const renameSpeaker = (
  meetingId: string,
  speakerId: string,
  label: string,
): Promise<Speaker[]> => invoke('rename_meeting_speaker', { meetingId, speakerId, label });

/**
 * Fills in the Latin-script view of a transcript.
 *
 * Deterministic and offline — no provider, no model, no network. Separate from
 * `translateTranscript` because the Romanized view must keep working on a
 * machine where translation cannot run at all.
 */
export const romanizeTranscript = (meetingId: string): Promise<TranscriptSegment[]> =>
  invoke('romanize_meeting_transcript', { meetingId });

// --- formatting ---------------------------------------------------------

/** `mm:ss`, or `h:mm:ss` once a meeting passes an hour. */
export function formatTimestamp(seconds: number): string {
  const total = Math.max(0, Math.floor(seconds));
  const hours = Math.floor(total / 3600);
  const minutes = Math.floor((total % 3600) / 60);
  const secs = total % 60;
  const pad = (n: number) => String(n).padStart(2, '0');
  return hours > 0 ? `${hours}:${pad(minutes)}:${pad(secs)}` : `${pad(minutes)}:${pad(secs)}`;
}

/** A duration in words, for a list row: "42 min", "1 h 05 min", "38 sec". */
export function formatDuration(seconds: number): string {
  if (!Number.isFinite(seconds) || seconds <= 0) return '—';
  if (seconds < 60) return `${Math.round(seconds)} sec`;
  const minutes = Math.round(seconds / 60);
  if (minutes < 60) return `${minutes} min`;
  const hours = Math.floor(minutes / 60);
  return `${hours} h ${String(minutes % 60).padStart(2, '0')} min`;
}

/**
 * The name shown against a transcript line.
 *
 * A detected speaker's name where there is one; otherwise the capture channel,
 * which is measured rather than inferred and so is always available.
 */
export function speakerLabel(
  segment: TranscriptSegment,
  speakers: Speaker[] = [],
): string {
  const speaker = segment.speaker_id
    ? speakers.find((candidate) => candidate.id === segment.speaker_id)
    : undefined;
  return speaker ? speaker.label : channelLabel(segment.channel);
}

/** The speaker label shown against a transcript line. */
export function channelLabel(channel: TranscriptSegment['channel']): string {
  switch (channel) {
    case 'microphone':
      return 'You';
    case 'system':
      return 'Others';
    default:
      return 'Speaker';
  }
}

/**
 * A transcript as plain text, for copying out.
 *
 * The speaker label is repeated only when it changes, which is how a
 * transcript reads rather than how a log does.
 */
export function transcriptToText(
  segments: TranscriptSegment[],
  textExtractor?: (segment: TranscriptSegment) => string,
): string {
  const lines: string[] = [];
  let lastChannel: string | null = null;
  for (const segment of segments) {
    const text = (textExtractor ? textExtractor(segment) : segment.text).trim();
    if (!text) continue;
    const stamp = formatTimestamp(segment.start_seconds);
    if (segment.channel !== lastChannel) {
      lines.push(`[${stamp}] ${channelLabel(segment.channel)}: ${text}`);
      lastChannel = segment.channel;
    } else {
      lines.push(`[${stamp}] ${text}`);
    }
  }
  return lines.join('\n');
}

/**
 * The heading a meeting sits under in the index: "Today", "Yesterday", or the
 * date it was recorded.
 *
 * Compared on calendar days in the viewer's own timezone rather than on
 * elapsed hours: a call at 11pm is "Yesterday" at 1am, not "2 hours ago", and
 * someone scanning for "the one from Tuesday" is looking for a date.
 */
export function dayLabel(iso: string, now: Date = new Date()): string {
  const when = new Date(iso);
  if (Number.isNaN(when.getTime())) return 'Undated';

  const midnight = (date: Date) =>
    new Date(date.getFullYear(), date.getMonth(), date.getDate()).getTime();
  const days = Math.round((midnight(now) - midnight(when)) / 86_400_000);

  if (days === 0) return 'Today';
  if (days === 1) return 'Yesterday';
  if (days > 1 && days < 7) {
    return when.toLocaleDateString(undefined, { weekday: 'long' });
  }
  return when.toLocaleDateString(undefined, {
    weekday: 'short',
    month: 'short',
    day: 'numeric',
    ...(when.getFullYear() === now.getFullYear() ? {} : { year: 'numeric' }),
  });
}

/** The clock time a meeting started, for a list row. */
export function formatClockTime(iso: string): string {
  const when = new Date(iso);
  if (Number.isNaN(when.getTime())) return '';
  return when.toLocaleTimeString(undefined, { hour: 'numeric', minute: '2-digit' });
}

/**
 * The index, split into the day headings it renders under.
 *
 * Order is preserved rather than re-sorted: the backend already returns
 * newest first, and a second sort here is a second answer to "which is
 * newest" that can disagree with it.
 */
export function groupByDay<T extends { created_at: string }>(
  items: T[],
  now: Date = new Date(),
): Array<{ label: string; items: T[] }> {
  const groups: Array<{ label: string; items: T[] }> = [];
  for (const item of items) {
    const label = dayLabel(item.created_at, now);
    const last = groups[groups.length - 1];
    if (last && last.label === label) last.items.push(item);
    else groups.push({ label, items: [item] });
  }
  return groups;
}

/** Whether a summary run is still in flight. */
export function summaryIsRunning(summary?: MeetingSummary | null): boolean {
  return summary?.status === 'pending' || summary?.status === 'processing';
}
