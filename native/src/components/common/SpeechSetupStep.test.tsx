import { describe, test, expect, vi, beforeEach } from 'vitest';
import React from 'react';
import { render, screen, waitFor, fireEvent } from '@testing-library/react';
import { invoke } from '@tauri-apps/api/core';

import { SpeechSetupStep } from './SpeechSetupStep';
import type { SpeechModel } from '@/types/models';

const model = (overrides: Partial<SpeechModel> = {}): SpeechModel => ({
  id: 'whisper-large-v3-turbo',
  name: 'Large v3 Turbo',
  filename: 'ggml-large-v3-turbo.bin',
  path: '/models/ggml-large-v3-turbo.bin',
  size_bytes: 1_624_555_275,
  installed: false,
  managed: true,
  multilingual: true,
  parameters_millions: 809,
  tier: 'accurate',
  blurb: 'Best meeting model for most machines.',
  ...overrides,
});

function mockBackend(answers: Record<string, unknown> = {}) {
  const defaults: Record<string, unknown> = {
    get_audio_devices: [{ name: 'Yeti Stereo Microphone', is_default: true }],
    list_speech_models: {
      models_dir: '/models',
      models: [model()],
      active_meeting_model: null,
      active_dictation_model: null,
      recommended_meeting_model: 'whisper-large-v3-turbo',
    },
  };
  const merged = { ...defaults, ...answers };
  vi.mocked(invoke).mockImplementation(async (command: string) => {
    if (command in merged) {
      const answer = merged[command];
      if (answer instanceof Error) throw answer;
      return answer;
    }
    return undefined;
  });
}

describe('SpeechSetupStep', () => {
  beforeEach(() => {
    vi.mocked(invoke).mockReset();
  });

  test('reports what was found rather than asserting what is needed', async () => {
    mockBackend();
    render(<SpeechSetupStep onDone={vi.fn()} />);

    expect(await screen.findByText('1 microphone found')).toBeInTheDocument();
    expect(screen.getByText('No speech model installed yet')).toBeInTheDocument();
  });

  test('says so when there is no microphone at all', async () => {
    mockBackend({ get_audio_devices: [] });
    render(<SpeechSetupStep onDone={vi.fn()} />);
    expect(await screen.findByText('No microphone found')).toBeInTheDocument();
  });

  test('downloads the recommended model', async () => {
    mockBackend();
    render(<SpeechSetupStep onDone={vi.fn()} />);

    fireEvent.click(await screen.findByRole('button', { name: /download large v3 turbo/i }));

    await waitFor(() => {
      expect(vi.mocked(invoke)).toHaveBeenCalledWith('download_speech_model', {
        id: 'whisper-large-v3-turbo',
      });
    });
  });

  test('lets someone on a metered connection get into the app', async () => {
    const onDone = vi.fn();
    mockBackend();
    render(<SpeechSetupStep onDone={onDone} />);

    fireEvent.click(await screen.findByRole('button', { name: /skip for now/i }));
    expect(onDone).toHaveBeenCalled();
    expect(screen.getByText(/settings › speech/i)).toBeInTheDocument();
  });

  test('offers no download and a plain finish once a model is present', async () => {
    mockBackend({
      list_speech_models: {
        models_dir: '/models',
        models: [model({ installed: true })],
        active_meeting_model: 'whisper-large-v3-turbo',
        active_dictation_model: null,
        recommended_meeting_model: 'whisper-large-v3-turbo',
      },
    });
    const onDone = vi.fn();
    render(<SpeechSetupStep onDone={onDone} />);

    expect(
      await screen.findByText('Speech model ready — Large v3 Turbo'),
    ).toBeInTheDocument();
    expect(screen.queryByRole('button', { name: /download/i })).not.toBeInTheDocument();

    fireEvent.click(screen.getByRole('button', { name: /start using vox/i }));
    expect(onDone).toHaveBeenCalled();
  });
});
