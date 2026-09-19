import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import { MeetingDiagnosticsPanel } from './MeetingDiagnosticsPanel';
import type {
  CaptureHealth,
  MeetingDiagnostics,
  SegmentDiagnostics,
  TranscriptionHealth,
} from '@/types/meetings';

const invoke = vi.hoisted(() => vi.fn());
vi.mock('@tauri-apps/api/core', () => ({ invoke }));

const capture = (overrides: Partial<CaptureHealth> = {}): CaptureHealth => ({
  recording_seconds: 600,
  microphone_opened: true,
  system_audio_opened: true,
  microphone_heard: true,
  system_audio_heard: true,
  audio_checkpoints_written: true,
  checkpoint_failures: 0,
  audio_lost_seconds: 0,
  ...overrides,
});

const transcription = (
  overrides: Partial<TranscriptionHealth> = {},
): TranscriptionHealth => ({
  segments_emitted: 40,
  segments_kept: 38,
  segments_discarded: 2,
  segments_failed: 0,
  segments_dropped: 0,
  peak_queue_depth: 3,
  speech_seconds: 300,
  transcribed_seconds: 290,
  mean_decode_rtf: 0.8,
  worst_decode_rtf: 2.4,
  worst_decode_sequence: 17,
  pipeline_rtf: 0.4,
  finalization_p50_ms: 900,
  finalization_p95_ms: 2400,
  finalization_max_ms: 5000,
  hangover_p50_ms: 400,
  segments_forced_split: 0,
  lock_wait_ms_total: 0,
  model_load_ms_total: 0,
  model_reloads: 0,
  drain_seconds: 4,
  drain_completed: true,
  rejections: {},
  ...overrides,
});

const diagnostics = (
  overrides: Partial<MeetingDiagnostics> = {},
): MeetingDiagnostics => ({
  version: 1,
  meeting_id: 'meeting-1',
  completed_at: '2026-09-19T10:00:00Z',
  model: 'ggml-small.bin',
  language: 'en',
  strategy: 'BeamSearch',
  threads: '8',
  profile_switches: [],
  segments_on_cheap_profile: 0,
  capture: capture(),
  transcription: transcription(),
  ...overrides,
});

const segment = (overrides: Partial<SegmentDiagnostics> = {}): SegmentDiagnostics => ({
  version: 1,
  sequence: 0,
  start_seconds: 0,
  end_seconds: 2,
  channel: 'microphone',
  forced_split: false,
  end_reason: 'silence',
  hangover_ms: 400,
  voiced_seconds: 1.8,
  total_seconds: 2,
  no_speech_prob: 0.05,
  queue_wait_ms: 10,
  lock_wait_ms: 0,
  model_load_ms: 0,
  model_reloaded: false,
  decode_ms: 200,
  post_ms: 2,
  persist_ms: 1,
  model: 'ggml-small.bin',
  language: 'en',
  expensive_script_profile: false,
  audio_ctx: null,
  status: 'kept',
  rejection: null,
  error: null,
  text_chars: 30,
  queue_depth_after: 0,
  ...overrides,
});

describe('MeetingDiagnosticsPanel', () => {
  beforeEach(() => {
    invoke.mockReset();
  });

  it('reports a healthy run without raising an alarm', () => {
    render(<MeetingDiagnosticsPanel meetingId="meeting-1" diagnostics={diagnostics()} />);

    expect(screen.getByText('ggml-small.bin')).toBeInTheDocument();
    expect(screen.getByText('0.40× clock')).toBeInTheDocument();
    expect(screen.queryByText(/slower than the meeting/)).not.toBeInTheDocument();
    expect(screen.queryByText(/No audio was saved/)).not.toBeInTheDocument();
  });

  it('says outright when no audio was saved', () => {
    // The worst failure in the subsystem, and until diagnostics existed it
    // only appeared in a log file nobody reads.
    render(
      <MeetingDiagnosticsPanel
        meetingId="meeting-1"
        diagnostics={diagnostics({
          capture: capture({ audio_checkpoints_written: false }),
        })}
      />,
    );
    expect(screen.getByText(/No audio was saved for this meeting/)).toBeInTheDocument();
  });

  it('counts checkpoint failures as gaps in the recording', () => {
    render(
      <MeetingDiagnosticsPanel
        meetingId="meeting-1"
        diagnostics={diagnostics({ capture: capture({ checkpoint_failures: 3 }) })}
      />,
    );
    expect(screen.getByText(/3 checkpoint write\(s\) failed/)).toBeInTheDocument();
  });

  it('says how much of the recording is missing when audio was shed', () => {
    // The worst kind of loss: it is in neither the audio nor the transcript,
    // and nothing can regenerate it.
    render(
      <MeetingDiagnosticsPanel
        meetingId="meeting-1"
        diagnostics={diagnostics({ capture: capture({ audio_lost_seconds: 2.5 }) })}
      />,
    );
    expect(screen.getByText(/2.5 s of audio never reached the writer/)).toBeInTheDocument();
  });

  it('names the wrong-device signature rather than showing a silent meter', () => {
    render(
      <MeetingDiagnosticsPanel
        meetingId="meeting-1"
        diagnostics={diagnostics({ capture: capture({ microphone_heard: false }) })}
      />,
    );
    expect(
      screen.getByText(/microphone was open and never heard anything/),
    ).toBeInTheDocument();
  });

  it('warns when transcription could not keep up with the clock', () => {
    render(
      <MeetingDiagnosticsPanel
        meetingId="meeting-1"
        diagnostics={diagnostics({ transcription: transcription({ pipeline_rtf: 1.6 }) })}
      />,
    );
    expect(screen.getByText(/slower than the meeting \(1.60× real time\)/)).toBeInTheDocument();
  });

  it('says speech is still in the recording when segments were lost', () => {
    render(
      <MeetingDiagnosticsPanel
        meetingId="meeting-1"
        diagnostics={diagnostics({
          transcription: transcription({ segments_dropped: 4, segments_failed: 1 }),
        })}
      />,
    );
    expect(screen.getByText(/4 segment\(s\) never reached the decoder/)).toBeInTheDocument();
    expect(screen.getByText(/recording still has that speech/)).toBeInTheDocument();
  });

  it('breaks screened-out segments down by reason', () => {
    render(
      <MeetingDiagnosticsPanel
        meetingId="meeting-1"
        diagnostics={diagnostics({
          transcription: transcription({
            rejections: { phrase_loop: 5, subtitle_filler: 1 },
          }),
        })}
      />,
    );
    expect(screen.getByText(/phrase_loop \(5\), subtitle_filler \(1\)/)).toBeInTheDocument();
  });

  it('reports the hangover next to the latency it is part of', () => {
    // The two have to be read together: no speech model makes the hangover
    // smaller, so "transcription is slow" has two very different answers.
    render(<MeetingDiagnosticsPanel meetingId="meeting-1" diagnostics={diagnostics()} />);
    expect(screen.getByText('400 ms')).toBeInTheDocument();
  });

  it('shows no confidence figure, because Whisper does not report one', () => {
    const { container } = render(
      <MeetingDiagnosticsPanel meetingId="meeting-1" diagnostics={diagnostics()} />,
    );
    expect(container.textContent).not.toMatch(/confidence/i);
  });

  it('loads per-segment records only when asked, slowest first', async () => {
    invoke.mockResolvedValue([
      segment({ sequence: 0, decode_ms: 100 }),
      segment({ sequence: 1, decode_ms: 9000, queue_wait_ms: 500 }),
      segment({ sequence: 2, decode_ms: 300 }),
    ]);

    render(<MeetingDiagnosticsPanel meetingId="meeting-1" diagnostics={diagnostics()} />);
    expect(invoke).not.toHaveBeenCalled();

    await userEvent.click(screen.getByRole('button', { name: /slowest segments/i }));

    expect(invoke).toHaveBeenCalledWith('get_meeting_segment_diagnostics', {
      meetingId: 'meeting-1',
    });
    const rows = await screen.findAllByRole('row');
    // Header plus three records, with the 9-second decode first.
    expect(rows).toHaveLength(4);
    expect(rows[1].textContent).toContain('9000 ms');
  });

  it('shows why a segment produced no text instead of a blank cell', async () => {
    invoke.mockResolvedValue([
      segment({ sequence: 4, status: 'discarded', rejection: 'phrase_loop', text_chars: 0 }),
    ]);
    render(<MeetingDiagnosticsPanel meetingId="meeting-1" diagnostics={diagnostics()} />);
    await userEvent.click(screen.getByRole('button', { name: /slowest segments/i }));
    expect(await screen.findByText('phrase_loop')).toBeInTheDocument();
  });

  it('reports a failed read rather than rendering an empty table', async () => {
    invoke.mockRejectedValue(new Error('diagnostics.jsonl is unreadable'));
    render(<MeetingDiagnosticsPanel meetingId="meeting-1" diagnostics={diagnostics()} />);
    await userEvent.click(screen.getByRole('button', { name: /slowest segments/i }));
    expect(
      await screen.findByText(/diagnostics.jsonl is unreadable/),
    ).toBeInTheDocument();
  });

  it('says so when a meeting kept no per-segment records', async () => {
    invoke.mockResolvedValue([]);
    render(<MeetingDiagnosticsPanel meetingId="meeting-1" diagnostics={diagnostics()} />);
    await userEvent.click(screen.getByRole('button', { name: /slowest segments/i }));
    expect(
      await screen.findByText(/No per-segment records were kept/),
    ).toBeInTheDocument();
  });
});
