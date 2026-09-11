import { describe, test, expect, vi, beforeEach } from 'vitest';
import React from 'react';
import { render, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { invoke } from '@tauri-apps/api/core';

import { SpeechModelsView } from './SpeechModelsView';
import type { SpeechModel, SpeechModelCatalogue } from '@/types/models';

const model = (overrides: Partial<SpeechModel> = {}): SpeechModel => ({
  id: 'whisper-small',
  name: 'Small',
  filename: 'ggml-small.bin',
  path: '/models/ggml-small.bin',
  size_bytes: 487_601_967,
  installed: false,
  managed: true,
  multilingual: true,
  parameters_millions: 244,
  tier: 'balanced',
  blurb: 'The default.',
  ...overrides,
});

const catalogue = (overrides: Partial<SpeechModelCatalogue> = {}): SpeechModelCatalogue => ({
  models_dir: '/home/u/.vox/models',
  models: [
    model({ id: 'whisper-base', name: 'Base', filename: 'ggml-base.bin', tier: 'fast' }),
    model(),
    model({
      id: 'whisper-large-v3-turbo',
      name: 'Large v3 Turbo',
      filename: 'ggml-large-v3-turbo.bin',
      tier: 'accurate',
      size_bytes: 1_624_555_275,
    }),
  ],
  active_meeting_model: null,
  active_dictation_model: null,
  recommended_meeting_model: 'whisper-large-v3-turbo',
  ...overrides,
});

/**
 * The card for one model.
 *
 * Scoped rather than looked up by text: an installed model's name appears
 * twice — once on its row, and once as a button in the "used for meetings"
 * picker — so a bare `findByText` is ambiguous exactly when a model is present,
 * which is the case most of these tests are about.
 */
async function findModelRow(name: string): Promise<HTMLElement> {
  const matches = await screen.findAllByText(name);
  const row = matches
    .map((node) => node.closest('div.p-3'))
    .find((card): card is HTMLElement => card !== null);
  if (!row) throw new Error(`no model row for '${name}'`);
  return row;
}

function mockBackend(answers: Record<string, unknown> = {}) {
  vi.mocked(invoke).mockImplementation(async (command: string) => {
    if (command in answers) {
      const answer = answers[command];
      if (answer instanceof Error) throw answer;
      return answer;
    }
    return undefined;
  });
}

describe('SpeechModelsView', () => {
  beforeEach(() => {
    vi.mocked(invoke).mockReset();
  });

  test('warns when nothing is installed and offers the recommended model', async () => {
    mockBackend({ list_speech_models: catalogue() });
    render(<SpeechModelsView />);

    expect(await screen.findByText('No speech model installed')).toBeInTheDocument();
    expect(
      screen.getByRole('button', { name: /download large v3 turbo \(1\.5 gb\)/i }),
    ).toBeInTheDocument();
  });

  test('does not warn once a model is on disk', async () => {
    mockBackend({
      list_speech_models: catalogue({
        models: [model({ installed: true })],
        active_meeting_model: 'whisper-small',
      }),
    });
    render(<SpeechModelsView />);

    await findModelRow('Small');
    expect(screen.queryByText('No speech model installed')).not.toBeInTheDocument();
  });

  test('downloads the model whose row was pressed', async () => {
    const user = userEvent.setup();
    mockBackend({ list_speech_models: catalogue() });
    render(<SpeechModelsView />);

    const row = await findModelRow('Base');
    await user.click(within(row).getByRole('button', { name: /download/i }));

    await waitFor(() => {
      expect(vi.mocked(invoke)).toHaveBeenCalledWith('download_speech_model', {
        id: 'whisper-base',
      });
    });
  });

  test('offers removal, not a download, for an installed model', async () => {
    mockBackend({
      list_speech_models: catalogue({
        models: [model({ id: 'whisper-base', name: 'Base', tier: 'fast', installed: true })],
      }),
    });
    render(<SpeechModelsView />);

    const row = await findModelRow('Base');
    expect(within(row).getByRole('button', { name: /remove/i })).toBeInTheDocument();
    expect(within(row).queryByRole('button', { name: /download/i })).not.toBeInTheDocument();
  });

  test('choosing a meeting model sends its id, and Best installed sends null', async () => {
    const user = userEvent.setup();
    mockBackend({
      list_speech_models: catalogue({
        models: [model({ installed: true })],
        active_meeting_model: null,
      }),
    });
    render(<SpeechModelsView />);

    await user.click(await screen.findByRole('button', { name: /^small$/i }));
    await waitFor(() => {
      expect(vi.mocked(invoke)).toHaveBeenCalledWith('set_meeting_speech_model', {
        id: 'whisper-small',
      });
    });

    await user.click(screen.getByRole('button', { name: /best installed/i }));
    await waitFor(() => {
      expect(vi.mocked(invoke)).toHaveBeenCalledWith('set_meeting_speech_model', { id: null });
    });
  });

  test('lists a hand-placed model separately from the catalogue', async () => {
    mockBackend({
      list_speech_models: catalogue({
        models: [
          model({ installed: true }),
          model({
            id: 'my-finetune.bin',
            name: 'my-finetune.bin',
            filename: 'my-finetune.bin',
            managed: false,
            installed: true,
          }),
        ],
      }),
    });
    render(<SpeechModelsView />);

    expect(await screen.findByText('Added by hand')).toBeInTheDocument();
  });
});
