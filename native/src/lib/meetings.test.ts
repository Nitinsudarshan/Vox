import { describe, test, expect } from 'vitest';
import {
  channelLabel,
  formatDuration,
  formatTimestamp,
  isMeetingError,
  meetingErrorMessage,
  summaryIsRunning,
  transcriptToText,
} from './meetings';
import type { MeetingSummary, TranscriptSegment } from '@/types/meetings';

const segment = (
  sequence: number,
  channel: TranscriptSegment['channel'],
  start: number,
  text: string,
): TranscriptSegment => ({
  sequence,
  text,
  start_seconds: start,
  end_seconds: start + 2,
  channel,
  no_speech_prob: 0.01,
  recorded_at: '2026-01-01T00:00:00Z',
});

describe('formatTimestamp', () => {
  test('grows a leading hour only when there is one', () => {
    expect(formatTimestamp(0)).toBe('00:00');
    expect(formatTimestamp(9.6)).toBe('00:09');
    expect(formatTimestamp(75)).toBe('01:15');
    expect(formatTimestamp(3661)).toBe('1:01:01');
  });

  test('never renders a negative clock', () => {
    expect(formatTimestamp(-5)).toBe('00:00');
  });
});

describe('formatDuration', () => {
  test('reads in the unit that suits the length', () => {
    expect(formatDuration(38)).toBe('38 sec');
    expect(formatDuration(42 * 60)).toBe('42 min');
    expect(formatDuration(65 * 60)).toBe('1 h 05 min');
  });

  test('a meeting with no duration shows a dash rather than "0 sec"', () => {
    expect(formatDuration(0)).toBe('—');
    expect(formatDuration(Number.NaN)).toBe('—');
  });
});

describe('channelLabel', () => {
  test('says who rather than which device', () => {
    expect(channelLabel('microphone')).toBe('You');
    expect(channelLabel('system')).toBe('Others');
    expect(channelLabel('mixed')).toBe('Speaker');
  });
});

describe('transcriptToText', () => {
  test('names the speaker only when it changes', () => {
    const text = transcriptToText([
      segment(0, 'microphone', 0, 'shall we start'),
      segment(1, 'microphone', 3, 'everyone here'),
      segment(2, 'system', 6, 'yes go ahead'),
    ]);
    expect(text.split('\n')).toEqual([
      '[00:00] You: shall we start',
      '[00:03] everyone here',
      '[00:06] Others: yes go ahead',
    ]);
  });

  test('skips blank lines rather than emitting empty turns', () => {
    const text = transcriptToText([
      segment(0, 'mixed', 0, 'one'),
      segment(1, 'mixed', 2, '   '),
      segment(2, 'mixed', 4, 'two'),
    ]);
    expect(text.split('\n')).toHaveLength(2);
  });

  test('an empty transcript produces an empty string', () => {
    expect(transcriptToText([])).toBe('');
  });
});

describe('summaryIsRunning', () => {
  test('is true only while a run is in flight', () => {
    const base = { status: 'pending' } as MeetingSummary;
    expect(summaryIsRunning(base)).toBe(true);
    expect(summaryIsRunning({ ...base, status: 'processing' })).toBe(true);
    expect(summaryIsRunning({ ...base, status: 'completed' })).toBe(false);
    expect(summaryIsRunning({ ...base, status: 'failed' })).toBe(false);
    expect(summaryIsRunning(null)).toBe(false);
    expect(summaryIsRunning(undefined)).toBe(false);
  });
});

describe('meeting errors', () => {
  test('a command error is recognised and its message used', () => {
    const error = { code: 'MEETING_NO_SPEECH_MODEL', message: 'Install one first.' };
    expect(isMeetingError(error)).toBe(true);
    expect(meetingErrorMessage(error)).toBe('Install one first.');
  });

  test('anything else still produces something worth showing', () => {
    expect(isMeetingError(new Error('boom'))).toBe(false);
    expect(meetingErrorMessage(new Error('boom'))).toBe('boom');
    expect(meetingErrorMessage('plain string')).toBe('plain string');
    expect(meetingErrorMessage(undefined)).toBe('Something went wrong.');
  });
});
