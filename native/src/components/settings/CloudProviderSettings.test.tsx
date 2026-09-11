import { describe, test, expect, vi } from 'vitest';
import React from 'react';
import { render, screen, fireEvent } from '@testing-library/react';

import { CloudProviderSettings } from './CloudProviderSettings';
import type { ProviderConfig } from '@/types';

const config = (overrides: Partial<ProviderConfig> = {}): ProviderConfig => ({
  active_provider: 'cloud_openai',
  ollama_host: 'http://localhost:11434',
  ollama_model: 'llama3.2:latest',
  cloud_model: 'gpt-4o-mini',
  ...overrides,
});

describe('CloudProviderSettings', () => {
  test('offers every provider, not only the original three', () => {
    render(<CloudProviderSettings provider={config()} onChange={vi.fn()} />);

    for (const label of [
      'OpenAI',
      'Anthropic',
      'Google Gemini',
      'Groq',
      'OpenRouter',
      'Custom endpoint',
    ]) {
      expect(screen.getByRole('button', { name: label })).toBeInTheDocument();
    }
  });

  test('a key is stored against the provider it was typed for', () => {
    const onChange = vi.fn();
    render(
      <CloudProviderSettings
        provider={config({ active_provider: 'groq', provider_keys: { cloud_openai: 'openai-key' } })}
        onChange={onChange}
      />,
    );

    fireEvent.change(screen.getByLabelText('API key'), { target: { value: 'gsk_new' } });

    expect(onChange).toHaveBeenCalledWith(
      expect.objectContaining({
        provider_keys: { cloud_openai: 'openai-key', groq: 'gsk_new' },
      }),
    );
  });

  test('shows the key saved for the active provider, not another one', () => {
    render(
      <CloudProviderSettings
        provider={config({
          active_provider: 'groq',
          provider_keys: { cloud_openai: 'openai-key', groq: 'groq-key' },
        })}
        onChange={vi.fn()}
      />,
    );
    expect(screen.getByLabelText('API key')).toHaveValue('groq-key');
  });

  test('falls back to the single key an older install saved', () => {
    render(
      <CloudProviderSettings
        provider={config({ cloud_api_key: 'legacy-key' })}
        onChange={vi.fn()}
      />,
    );
    expect(screen.getByLabelText('API key')).toHaveValue('legacy-key');
  });

  test('a custom endpoint gets a URL field and says whether it is local', () => {
    const { rerender } = render(
      <CloudProviderSettings
        provider={config({
          active_provider: 'custom_openai',
          custom_openai_endpoint: 'http://localhost:1234/v1',
        })}
        onChange={vi.fn()}
      />,
    );
    expect(screen.getByLabelText('Endpoint URL')).toHaveValue('http://localhost:1234/v1');
    expect(screen.getByText(/transcripts stay here/i)).toBeInTheDocument();

    rerender(
      <CloudProviderSettings
        provider={config({
          active_provider: 'custom_openai',
          custom_openai_endpoint: 'https://models.example.com/v1',
        })}
        onChange={vi.fn()}
      />,
    );
    expect(screen.getByText(/are sent to that server/i)).toBeInTheDocument();
  });

  test('warns before a key would go unencrypted to a remote endpoint', () => {
    render(
      <CloudProviderSettings
        provider={config({
          active_provider: 'custom_openai',
          custom_openai_endpoint: 'http://models.example.com/v1',
          provider_keys: { custom_openai: 'secret' },
        })}
        onChange={vi.fn()}
      />,
    );
    expect(screen.getByText(/would be sent\s+unencrypted/i)).toBeInTheDocument();
  });

  test('plain HTTP on this machine is not warned about', () => {
    render(
      <CloudProviderSettings
        provider={config({
          active_provider: 'custom_openai',
          custom_openai_endpoint: 'http://localhost:1234/v1',
          provider_keys: { custom_openai: 'secret' },
        })}
        onChange={vi.fn()}
      />,
    );
    expect(screen.queryByText(/unencrypted/i)).not.toBeInTheDocument();
  });

  test('the custom endpoint writes its own model field, not the cloud one', () => {
    const onChange = vi.fn();
    render(
      <CloudProviderSettings
        provider={config({ active_provider: 'custom_openai' })}
        onChange={onChange}
      />,
    );

    fireEvent.change(screen.getByLabelText('Model'), { target: { value: 'qwen2.5-7b-instruct' } });

    expect(onChange).toHaveBeenCalledWith(
      expect.objectContaining({ custom_openai_model: 'qwen2.5-7b-instruct' }),
    );
    expect(onChange).not.toHaveBeenCalledWith(
      expect.objectContaining({ cloud_model: 'qwen2.5-7b-instruct' }),
    );
  });

  test('switching provider moves the model off the previous vendor', () => {
    const onChange = vi.fn();
    render(<CloudProviderSettings provider={config()} onChange={onChange} />);

    fireEvent.click(screen.getByRole('button', { name: 'Groq' }));

    expect(onChange).toHaveBeenCalledWith(
      expect.objectContaining({
        active_provider: 'groq',
        cloud_model: 'llama-3.3-70b-versatile',
      }),
    );
  });
});
