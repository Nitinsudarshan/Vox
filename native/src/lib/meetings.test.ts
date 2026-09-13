import { describe, test, expect } from 'vitest';
import {
  channelLabel,
  dayLabel,
  formatDuration,
  formatTimestamp,
  groupByDay,
  isMeetingError,
  meetingErrorMessage,
  speakerLabel,
  summaryIsRunning,
  transcriptToText,
} from './meetings';
import type { MeetingSummary, Speaker, TranscriptSegment } from '@/types/meetings';

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

describe('dayLabel', () => {
  const now = new Date('2026-09-14T12:00:00');

  test('names today and yesterday rather than dating them', () => {
    expect(dayLabel('2026-09-14T09:00:00', now)).toBe('Today');
    expect(dayLabel('2026-09-13T23:30:00', now)).toBe('Yesterday');
  });

  test('a late-night meeting belongs to the day it happened on', () => {
    // Compared on calendar days, not elapsed hours: 23:00 yesterday is
    // "Yesterday" at noon today, not "13 hours ago".
    expect(dayLabel('2026-09-13T23:00:00', now)).toBe('Yesterday');
    expect(dayLabel('2026-09-14T00:30:00', now)).toBe('Today');
  });

  test('names the weekday inside the last week', () => {
    expect(dayLabel('2026-09-10T09:00:00', now)).toBe('Thursday');
  });

  test('dates anything older', () => {
    const label = dayLabel('2026-07-01T09:00:00', now);
    expect(label).toMatch(/Jul/);
    expect(label).not.toBe('Today');
  });

  test('an unparseable date is labelled rather than rendered as Invalid Date', () => {
    expect(dayLabel('not a date', now)).toBe('Undated');
  });
});

describe('groupByDay', () => {
  const now = new Date('2026-09-14T12:00:00');

  test('runs of the same day become one group, in the order given', () => {
    const items = [
      { id: 'a', created_at: '2026-09-14T11:00:00' },
      { id: 'b', created_at: '2026-09-14T09:00:00' },
      { id: 'c', created_at: '2026-09-13T16:00:00' },
    ];
    const groups = groupByDay(items, now);
    expect(groups.map((group) => group.label)).toEqual(['Today', 'Yesterday']);
    expect(groups[0].items.map((item) => item.id)).toEqual(['a', 'b']);
  });

  test('the backend order is preserved rather than re-sorted', () => {
    // A second sort here is a second answer to "which is newest" that can
    // disagree with the one the list was built from.
    const items = [
      { id: 'older', created_at: '2026-09-14T08:00:00' },
      { id: 'newer', created_at: '2026-09-14T11:00:00' },
    ];
    expect(groupByDay(items, now)[0].items.map((item) => item.id)).toEqual(['older', 'newer']);
  });

  test('nothing in produces nothing out', () => {
    expect(groupByDay([], now)).toEqual([]);
  });
});

describe('speakerLabel', () => {
  const line = (overrides: Partial<TranscriptSegment> = {}): TranscriptSegment => ({
    ...segment(0, 'system', 0, 'hello'),
    ...overrides,
  });

  const payal: Speaker = {
    id: 'speaker-1',
    label: 'Payal',
    named_by_user: true,
    channel: 'system',
    sample_start_seconds: 4,
    sample_end_seconds: 12,
    segment_count: 3,
    speaking_seconds: 40,
  };

  test('an attributed line carries the name the user gave', () => {
    expect(speakerLabel(line({ speaker_id: 'speaker-1' }), [payal])).toBe('Payal');
  });

  test('an unattributed line falls back to the capture channel', () => {
    // The channel is measured rather than inferred, so it is always available:
    // less than a name, and more than nothing.
    expect(speakerLabel(line(), [payal])).toBe('Others');
    expect(speakerLabel(line({ channel: 'microphone' }), [payal])).toBe('You');
  });

  test('a line pointing at a speaker that no longer exists falls back too', () => {
    // Detection re-run with fewer groups must not leave lines blank.
    expect(speakerLabel(line({ speaker_id: 'speaker-9' }), [payal])).toBe('Others');
  });

  test('no speakers at all is the ordinary case before detection has run', () => {
    expect(speakerLabel(line())).toBe('Others');
  });
});
