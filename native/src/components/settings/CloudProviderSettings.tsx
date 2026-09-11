import React from 'react';
import { Cloud, KeyRound, Server, ShieldAlert } from 'lucide-react';

import { Input } from '@/components/ui/input';
import type { ProviderConfig } from '@/types';

interface CloudProviderSettingsProps {
  provider: ProviderConfig;
  onChange: (next: ProviderConfig) => void;
}

/**
 * Provider slugs that are not Ollama, and what to show for each.
 *
 * A table rather than a chain of ternaries. The version this replaces had the
 * model presets written as nested conditionals inside JSX, which is workable
 * for three providers and is the reason adding a fourth meant touching four
 * unrelated expressions.
 */
interface CloudProviderSpec {
  id: string;
  label: string;
  /** Models offered as one-click presets. The field stays free-text. */
  models: string[];
  keyPlaceholder: string;
  /** Where to get a key, in words — no link, because links rot. */
  keyHint: string;
}

const CLOUD_PROVIDERS: CloudProviderSpec[] = [
  {
    id: 'cloud_openai',
    label: 'OpenAI',
    models: ['gpt-4o-mini', 'gpt-4o', 'o3-mini'],
    keyPlaceholder: 'sk-…',
    keyHint: 'From platform.openai.com → API keys.',
  },
  {
    id: 'cloud_anthropic',
    label: 'Anthropic',
    models: ['claude-sonnet-4-5', 'claude-3-5-haiku-20241022', 'claude-3-opus-20240229'],
    keyPlaceholder: 'sk-ant-…',
    keyHint: 'From console.anthropic.com → API keys.',
  },
  {
    id: 'cloud_gemini',
    label: 'Google Gemini',
    models: ['gemini-2.0-flash', 'gemini-1.5-pro', 'gemini-1.5-flash'],
    keyPlaceholder: 'AIza…',
    keyHint: 'From aistudio.google.com → Get API key.',
  },
  {
    id: 'groq',
    label: 'Groq',
    models: ['llama-3.3-70b-versatile', 'llama-3.1-8b-instant', 'mixtral-8x7b-32768'],
    keyPlaceholder: 'gsk_…',
    keyHint: 'From console.groq.com → API keys.',
  },
  {
    id: 'openrouter',
    label: 'OpenRouter',
    models: ['openai/gpt-4o-mini', 'anthropic/claude-sonnet-4.5', 'meta-llama/llama-3.3-70b-instruct'],
    keyPlaceholder: 'sk-or-…',
    keyHint: 'From openrouter.ai → Keys.',
  },
  {
    id: 'custom_openai',
    label: 'Custom endpoint',
    models: [],
    keyPlaceholder: 'Optional',
    keyHint: 'Most local servers need no key. Leave it empty if yours does not.',
  },
];

/** Whether a URL points at this machine. Mirrors the Rust check. */
function isLoopback(url: string): boolean {
  const rest = url.includes('://') ? url.slice(url.indexOf('://') + 3) : url;
  const hostPort = rest.split(/[/?#]/)[0] ?? '';
  const host = hostPort.replace(/^\[|\]$/g, '').replace(/:\d+$/, '');
  return ['localhost', '127.0.0.1', '::1', '0.0.0.0'].includes(host) || host.startsWith('127.');
}

/**
 * Everything about a non-Ollama provider: which one, its key, and its model.
 *
 * Extracted from the 2,400-line settings component because it grew from three
 * providers to six. Keys are stored per provider (`provider_keys`), so
 * switching from OpenAI to Groq to compare them no longer means pasting a key,
 * losing it, and pasting the first one back.
 */
export const CloudProviderSettings: React.FC<CloudProviderSettingsProps> = ({
  provider,
  onChange,
}) => {
  const active = CLOUD_PROVIDERS.find((p) => p.id === provider.active_provider);
  const isCustom = provider.active_provider === 'custom_openai';
  const keys = provider.provider_keys ?? {};
  // The legacy single key is read as a fallback, exactly as the backend does,
  // so an install that predates per-provider keys shows the key it is using.
  const currentKey = keys[provider.active_provider] ?? provider.cloud_api_key ?? '';
  const endpoint = provider.custom_openai_endpoint ?? '';

  const setKey = (value: string) =>
    onChange({
      ...provider,
      provider_keys: { ...keys, [provider.active_provider]: value },
    });

  const insecure =
    isCustom &&
    endpoint.startsWith('http://') &&
    !isLoopback(endpoint) &&
    currentKey.trim() !== '';

  return (
    <div className="space-y-4">
      <div className="space-y-1.5">
        <label className="block text-xs font-medium text-foreground">Provider</label>
        <div className="grid grid-cols-2 sm:grid-cols-3 gap-2">
          {CLOUD_PROVIDERS.map((spec) => (
            <button
              key={spec.id}
              type="button"
              onClick={() =>
                onChange({
                  ...provider,
                  active_provider: spec.id as ProviderConfig['active_provider'],
                  // Moving to a provider offers its own first preset, so the
                  // model field is never left naming another vendor's model.
                  ...(spec.models[0] ? { cloud_model: spec.models[0] } : {}),
                })
              }
              className={`p-2 rounded-lg border text-xs font-semibold transition-all ${
                provider.active_provider === spec.id
                  ? 'border-primary bg-primary/10 text-foreground'
                  : 'border-border bg-card/50 text-muted-foreground hover:border-border/80'
              }`}
            >
              {spec.label}
            </button>
          ))}
        </div>
      </div>

      {isCustom && (
        <div className="space-y-1.5">
          <label
            htmlFor="custom-openai-endpoint"
            className="flex items-center gap-1.5 text-xs font-medium text-foreground"
          >
            <Server className="w-3.5 h-3.5" />
            Endpoint URL
          </label>
          <Input
            id="custom-openai-endpoint"
            value={endpoint}
            onChange={(event) =>
              onChange({ ...provider, custom_openai_endpoint: event.target.value })
            }
            placeholder="http://localhost:1234/v1"
            className="text-xs font-mono"
          />
          <p className="text-[11px] text-muted-foreground">
            Any server speaking the OpenAI chat-completions API — vLLM, LM Studio, LiteLLM,
            Azure OpenAI. Vox appends <code className="font-mono">/chat/completions</code>.
          </p>
          {endpoint !== '' && (
            <p className="flex items-center gap-1.5 text-[11px] text-muted-foreground">
              <Cloud className="w-3 h-3 shrink-0" />
              {isLoopback(endpoint)
                ? 'On this machine — transcripts stay here.'
                : 'Not on this machine — transcripts are sent to that server.'}
            </p>
          )}
        </div>
      )}

      <div className="space-y-1.5">
        <label
          htmlFor="cloud-api-key"
          className="flex items-center gap-1.5 text-xs font-medium text-foreground"
        >
          <KeyRound className="w-3.5 h-3.5" />
          API key
        </label>
        <Input
          id="cloud-api-key"
          type="password"
          value={currentKey}
          onChange={(event) => setKey(event.target.value)}
          placeholder={active?.keyPlaceholder ?? 'sk-…'}
          className="text-xs font-mono"
        />
        <p className="text-[11px] text-muted-foreground">
          {active?.keyHint} Kept per provider, so switching between them does not lose it.
        </p>
        {insecure && (
          <p className="flex items-start gap-1.5 text-[11px] text-destructive">
            <ShieldAlert className="w-3.5 h-3.5 shrink-0 mt-px" />
            This endpoint is plain HTTP and not on this machine, so the key would be sent
            unencrypted. Vox will refuse the request — use https://, or clear the key.
          </p>
        )}
      </div>

      <div className="space-y-2">
        <label htmlFor="cloud-model-custom" className="block text-xs font-medium text-foreground">
          Model
        </label>
        {active && active.models.length > 0 && (
          <div className="grid grid-cols-1 sm:grid-cols-3 gap-2">
            {active.models.map((name) => (
              <button
                key={name}
                type="button"
                onClick={() => onChange({ ...provider, cloud_model: name })}
                className={`p-2 rounded-lg border text-xs font-mono transition-all break-all ${
                  provider.cloud_model === name
                    ? 'border-primary bg-primary/10 text-foreground font-bold'
                    : 'border-border bg-card/50 text-muted-foreground hover:border-border/80'
                }`}
              >
                {name}
              </button>
            ))}
          </div>
        )}
        <Input
          id="cloud-model-custom"
          value={(isCustom ? provider.custom_openai_model : provider.cloud_model) ?? ''}
          onChange={(event) =>
            onChange(
              isCustom
                ? { ...provider, custom_openai_model: event.target.value }
                : { ...provider, cloud_model: event.target.value },
            )
          }
          placeholder={isCustom ? 'Model name your server expects' : 'Custom model name…'}
          className="text-xs font-mono"
        />
        {isCustom && (
          <p className="text-[11px] text-muted-foreground">
            Many local servers ignore this and answer from whatever they loaded; vLLM does not.
          </p>
        )}
      </div>
    </div>
  );
};

/** Exposed so the settings view can name providers without a second list. */
export { CLOUD_PROVIDERS };
export type { CloudProviderSpec };
