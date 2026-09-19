import { describe, test, expect, vi, beforeEach } from 'vitest';
import React from 'react';
import { render, screen, fireEvent } from '@testing-library/react';

import { RetranscribeDialog } from './RetranscribeDialog';
import * as speechModels from '@/lib/speechModels';

vi.mock('@/lib/speechModels', () => ({
  listSpeechModels: vi.fn(),
}));

describe('RetranscribeDialog', () => {
  beforeEach(() => {
    vi.mocked(speechModels.listSpeechModels).mockResolvedValue({
      models: [
        {
          id: 'whisper-base',
          name: 'Whisper Base',
          size_bytes: 140_000_000,
          multilingual: true,
          installed: true,
        },
      ],
    } as any);
  });

  test('renders options and allows submitting re-transcription', async () => {
    const onRun = vi.fn();
    render(
      <RetranscribeDialog
        open={true}
        onOpenChange={vi.fn()}
        onRun={onRun}
        running={false}
        suggestEnglishTrack={false}
      />,
    );

    expect(await screen.findByRole('heading', { name: /transcribe again/i })).toBeInTheDocument();
    const button = screen.getByRole('button', { name: /^transcribe again$/i });
    expect(button).toBeInTheDocument();
    expect(button).not.toBeDisabled();

    fireEvent.click(button);
    expect(onRun).toHaveBeenCalledWith({
      language: '',
      modelId: '',
      preset: 'quality',
      englishTrack: false,
    });
  });

  test('disables inputs and shows progress section and button percentage while running', async () => {
    const { rerender } = render(
      <RetranscribeDialog
        open={true}
        onOpenChange={vi.fn()}
        onRun={vi.fn()}
        running={true}
        suggestEnglishTrack={false}
        progress={null}
      />,
    );

    // Wait for model list to load
    await screen.findByRole('heading', { name: /transcribe again/i });

    // Button shows initial running text
    expect(screen.getByRole('button', { name: /^transcribing…$/i })).toBeDisabled();
    expect(screen.getByRole('button', { name: /cancel/i })).toBeDisabled();
    expect(screen.getByLabelText(/transcription language/i)).toBeDisabled();
    expect(screen.getByLabelText(/speech model/i)).toBeDisabled();

    // Now emit progress with 45%
    rerender(
      <RetranscribeDialog
        open={true}
        onOpenChange={vi.fn()}
        onRun={vi.fn()}
        running={true}
        suggestEnglishTrack={false}
        progress={{
          meeting_id: 'meeting-1',
          stage: 'Transcribing',
          processed_seconds: 45,
          total_seconds: 100,
          fraction: 0.45,
          segments: 5,
        }}
      />,
    );

    // Button updates to show percentage
    expect(screen.getByRole('button', { name: /transcribing… 45%/i })).toBeInTheDocument();

    // Progress card renders percentage, time, and segment count
    expect(screen.getByText('45%')).toBeInTheDocument();
    expect(screen.getByText('00:45 / 01:40')).toBeInTheDocument();
    expect(screen.getByText(/5 segments/)).toBeInTheDocument();
  });

  test('displays translating stage and percentage when English pass runs', async () => {
    render(
      <RetranscribeDialog
        open={true}
        onOpenChange={vi.fn()}
        onRun={vi.fn()}
        running={true}
        suggestEnglishTrack={true}
        progress={{
          meeting_id: 'meeting-1',
          stage: 'Translating',
          processed_seconds: 70,
          total_seconds: 100,
          fraction: 0.7,
          segments: 10,
        }}
      />,
    );

    await screen.findByRole('heading', { name: /transcribe again/i });
    expect(screen.getByRole('button', { name: /translating… 70%/i })).toBeInTheDocument();
    expect(screen.getByText('70%')).toBeInTheDocument();
    expect(screen.getAllByText(/translating…/i).length).toBeGreaterThanOrEqual(1);
    expect(screen.getByText(/10 segments/)).toBeInTheDocument();
  });
});
