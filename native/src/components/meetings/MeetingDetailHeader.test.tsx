import { render, screen } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';

import { MeetingDetailHeader } from './MeetingDetailHeader';
import type { Meeting, MeetingDetail, TranscriptProvenance } from '@/types/meetings';

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn(), convertFileSrc: (p: string) => p }));

const meeting = (overrides: Partial<Meeting> = {}): Meeting => ({
  id: 'meeting-1',
  title: 'Standup',
  created_at: '2026-09-19T09:00:00Z',
  updated_at: '2026-09-19T09:30:00Z',
  state: 'completed',
  source: 'recorded',
  duration_seconds: 1800,
  audio_path: null,
  language: 'en',
  mic_device: null,
  system_audio_captured: true,
  segment_count: 40,
  dropped_segments: 0,
  error: null,
  tags: [],
  series_id: null,
  ...overrides,
});

const provenance = (
  overrides: Partial<TranscriptProvenance> = {},
): TranscriptProvenance => ({
  pass: 'live',
  engine: 'whisper',
  model: 'ggml-small.bin',
  language: 'en',
  profile: 'BeamSearch',
  completed_at: '2026-09-19T09:31:00Z',
  ...overrides,
});

const detail = (m: Meeting): MeetingDetail => ({
  meeting: m,
  segments: [],
  summary: null,
  notes: '',
  speakers: [],
});

const renderHeader = (m: Meeting, live = false) =>
  render(
    <MeetingDetailHeader
      detail={detail(m)}
      speakers={[]}
      live={live}
      busy={false}
      onBack={vi.fn()}
      editingTitle={false}
      onEditTitle={vi.fn()}
      onRename={vi.fn()}
      showSpeakers={false}
      onToggleSpeakers={vi.fn()}
      showSeries={false}
      onToggleSeries={vi.fn()}
      onAddToSeries={vi.fn()}
      onRequestDelete={vi.fn()}
      onOpenFolder={vi.fn()}
      onRetranscribe={vi.fn()}
      onDetectSpeakers={vi.fn()}
      seekTo={null}
      onTimeChange={vi.fn()}
    />,
  );

describe('MeetingDetailHeader transcript state', () => {
  it('says a transcript came from the live pass', () => {
    renderHeader(meeting({ transcript: provenance({ pass: 'live' }) }));
    expect(screen.getByText('Live transcript')).toBeInTheDocument();
  });

  it('says a transcript came from the final pass', () => {
    // The distinction the whole stage exists for: a live transcript raced a
    // clock and a final one did not, so "this reads worse than last time"
    // has somewhere to start.
    renderHeader(
      meeting({
        transcript: provenance({ pass: 'final', model: 'ggml-large-v3-turbo.bin' }),
      }),
    );
    expect(screen.getByText('Final transcript')).toBeInTheDocument();
  });

  it('names the model, language and profile behind the pass', () => {
    renderHeader(
      meeting({
        transcript: provenance({ pass: 'final', model: 'ggml-large-v3-turbo.bin', language: 'hi' }),
      }),
    );
    expect(screen.getByText('Final transcript')).toHaveAttribute(
      'title',
      'whisper · ggml-large-v3-turbo.bin · hi · BeamSearch',
    );
  });

  it('shows work in progress rather than a stale pass while decoding', () => {
    renderHeader(
      meeting({ state: 'transcribing', transcript: provenance({ pass: 'live' }) }),
    );
    expect(screen.getByText('Transcribing…')).toBeInTheDocument();
    expect(screen.queryByText('Live transcript')).not.toBeInTheDocument();
  });

  it('claims nothing about a meeting recorded before passes were recorded', () => {
    renderHeader(meeting({ transcript: null }));
    expect(screen.queryByText('Live transcript')).not.toBeInTheDocument();
    expect(screen.queryByText('Final transcript')).not.toBeInTheDocument();
  });

  it('shows the recording indicator rather than a pass while recording', () => {
    renderHeader(meeting({ state: 'recording' }), true);
    expect(screen.getByText('Recording')).toBeInTheDocument();
    expect(screen.queryByText(/transcript$/i)).not.toBeInTheDocument();
  });
});
