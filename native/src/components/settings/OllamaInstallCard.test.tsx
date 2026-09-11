import { describe, test, expect, vi, beforeEach } from 'vitest';
import React from 'react';
import { render, screen, waitFor, fireEvent } from '@testing-library/react';
import { invoke } from '@tauri-apps/api/core';

import { OllamaInstallCard } from './OllamaInstallCard';

const plan = {
  url: 'https://ollama.com/download/OllamaSetup.exe',
  filename: 'OllamaSetup.exe',
  kind: 'installer' as const,
  approx_bytes: 750_000_000,
  description: 'Downloads Ollama’s installer and opens it.',
};

function mockBackend(answers: Record<string, unknown> = {}) {
  const defaults: Record<string, unknown> = {
    ensure_local_llm_ready: { state: 'not_installed' },
    get_ollama_install_plan: plan,
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

describe('OllamaInstallCard', () => {
  beforeEach(() => {
    vi.mocked(invoke).mockReset();
  });

  test('offers the install when Ollama is missing, with what it will cost', async () => {
    mockBackend();
    render(<OllamaInstallCard />);

    expect(await screen.findByText('Ollama is not installed')).toBeInTheDocument();
    expect(screen.getByRole('button', { name: /install ollama/i })).toBeInTheDocument();
    expect(screen.getByText(/About 715 MB/)).toBeInTheDocument();
  });

  test('says nothing at all once Ollama is running', async () => {
    // A settings page that keeps offering to install something already
    // installed is noise.
    mockBackend({ ensure_local_llm_ready: { state: 'running' } });
    const { container } = render(<OllamaInstallCard />);
    await waitFor(() => expect(vi.mocked(invoke)).toHaveBeenCalledWith('ensure_local_llm_ready'));
    await waitFor(() => expect(container).toBeEmptyDOMElement());
  });

  test('a host that does not answer is reported as itself, not as missing', async () => {
    mockBackend({
      ensure_local_llm_ready: { state: 'unreachable', message: 'http://box:11434 is unreachable' },
    });
    render(<OllamaInstallCard />);

    expect(await screen.findByText('Ollama is not responding')).toBeInTheDocument();
    expect(screen.getByText(/http:\/\/box:11434 is unreachable/)).toBeInTheDocument();
  });

  test('pressing install asks the backend to do it', async () => {
    mockBackend();
    render(<OllamaInstallCard />);

    fireEvent.click(await screen.findByRole('button', { name: /install ollama/i }));
    await waitFor(() => expect(vi.mocked(invoke)).toHaveBeenCalledWith('install_ollama'));
  });

  test('a failed install says why instead of spinning', async () => {
    mockBackend({ install_ollama: new Error('the Ollama download server returned HTTP 503') });
    render(<OllamaInstallCard />);

    fireEvent.click(await screen.findByRole('button', { name: /install ollama/i }));
    expect(await screen.findByText(/returned HTTP 503/)).toBeInTheDocument();
  });

  test('a platform with no plan points at ollama.com rather than a dead button', async () => {
    mockBackend({ get_ollama_install_plan: null });
    render(<OllamaInstallCard />);

    expect(await screen.findByText(/get it from ollama\.com/i)).toBeInTheDocument();
    expect(screen.queryByRole('button', { name: /install ollama/i })).not.toBeInTheDocument();
  });
});
