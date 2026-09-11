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
  MeetingSummary,
  MeetingTemplate,
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

export const promoteToScribble = (meetingId: string): Promise<{ id: string }> =>
  invoke('promote_meeting_to_scribble', { meetingId });

export const audioExtensions = (): Promise<string[]> => invoke('meeting_audio_extensions');

/** Opens the OS file picker. Resolves to `null` when the user cancels. */
export const pickAudioFile = (): Promise<string | null> =>
  invoke('pick_meeting_audio_file');

export const importAudio = (path: string, title?: string): Promise<Meeting> =>
  invoke('import_meeting_audio', { path, title });

export const retranscribeMeeting = (meetingId: string): Promise<Meeting> =>
  invoke('retranscribe_meeting', { meetingId });

export const cancelImport = (key: string): Promise<boolean> =>
  invoke('cancel_meeting_import', { key });

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
export function transcriptToText(segments: TranscriptSegment[]): string {
  const lines: string[] = [];
  let lastChannel: string | null = null;
  for (const segment of segments) {
    const text = segment.text.trim();
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

/** Whether a summary run is still in flight. */
export function summaryIsRunning(summary?: MeetingSummary | null): boolean {
  return summary?.status === 'pending' || summary?.status === 'processing';
}
