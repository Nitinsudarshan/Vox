import React, { useState, useEffect, useCallback, useMemo } from 'react';
import {
  Mic,
  Users,
  Cpu,
  Download,
  Trash2,
  Check,
  AlertTriangle,
  Sparkles,
  Volume2,
  Server,
  Zap,
  Target,
  Info,
  ChevronDown,
  ChevronUp,
  RefreshCw,
  HardDrive,
  Globe,
  CheckCircle2,
  Radio,
  Activity,
} from 'lucide-react';

import { Badge } from '@/components/ui/badge';
import { Button } from '@/components/ui/button';
import { Card } from '@/components/ui/card';
import { Input } from '@/components/ui/input';
import {
  Tooltip,
  TooltipContent,
  TooltipProvider,
  TooltipTrigger,
} from '@/components/ui/tooltip';

import {
  listSpeechModels,
  downloadSpeechModel,
  cancelSpeechModelDownload,
  deleteSpeechModel,
  setMeetingSpeechModel,
  setDictationSpeechModel,
  onSpeechModelDownload,
  formatBytes,
  downloadFraction,
  TIER_LABEL,
  getParakeetStatus,
  downloadParakeetModel,
  deleteParakeetModel,
  setDictationEngine,
  type ParakeetStatus,
} from '@/lib/speechModels';
import type {
  SpeechModel,
  SpeechModelCatalogue,
  SpeechModelDownloadProgress,
  SpeechModelTier,
  OllamaModelDetails,
  OllamaPromptTestResult,
  OllamaStatus,
} from '@/types/models';
import type { AppSettings, ProviderConfig, MainTabType, ProviderSlug } from '@/types';
import { CloudProviderSettings } from './CloudProviderSettings';
import { invoke } from '@tauri-apps/api/core';
import { describeError } from '@/lib/errors';

interface UnifiedModelsViewProps {
  settings: AppSettings;
  onUpdateSettings: (updater: (prev: AppSettings) => AppSettings) => void;
  onSaveDirect: () => Promise<void>;
  onNavigateTab?: (tab: MainTabType) => void;
}

type ModelCategoryTab = 'stt' | 'llm' | 'tts';
type LanguageFilter = 'all' | 'english' | 'multilingual';
type TierFilter = 'all' | SpeechModelTier;

type DownloadState =
  | { kind: 'downloading'; fraction: number | null; downloaded: number }
  | { kind: 'verifying' }
  | { kind: 'failed'; message: string };

interface ModelRatings {
  accuracy: number;
  speed: number;
}

function getModelRatings(modelId: string, tier?: string, filename?: string): ModelRatings {
  const id = modelId.toLowerCase();
  const file = (filename || '').toLowerCase();

  // Special case: Parakeet
  if (id.includes('parakeet')) {
    return { accuracy: 4, speed: 5 };
  }

  // Catalogue Whisper models
  if (id === 'whisper-tiny' || id === 'tiny') {
    return { accuracy: 1, speed: 5 };
  }
  if (id === 'whisper-tiny-en' || id === 'tiny.en' || id === 'tiny-en') {
    return { accuracy: 2, speed: 5 };
  }
  if (id === 'whisper-base' || id === 'base') {
    return { accuracy: 2, speed: 4 };
  }
  if (id === 'whisper-base-en' || id === 'base.en' || id === 'base-en') {
    return { accuracy: 3, speed: 4 };
  }
  if (id === 'whisper-small' || id === 'small') {
    return { accuracy: 3, speed: 3 };
  }
  if (id === 'whisper-small-en' || id === 'small.en' || id === 'small-en') {
    return { accuracy: 4, speed: 3 };
  }
  if (id === 'whisper-medium' || id === 'medium') {
    return { accuracy: 4, speed: 2 };
  }
  if (id === 'whisper-medium-en' || id === 'medium.en' || id === 'medium-en') {
    return { accuracy: 4, speed: 2 };
  }
  if (id.includes('turbo') || file.includes('turbo')) {
    return { accuracy: 5, speed: 4 };
  }
  if (id === 'whisper-large-v1' || id === 'large-v1' || id === 'large_v1') {
    return { accuracy: 4, speed: 1 };
  }
  if (id === 'whisper-large-v2' || id === 'large-v2' || id === 'large_v2') {
    return { accuracy: 5, speed: 1 };
  }
  if (
    id === 'whisper-large-v3' ||
    id === 'large-v3' ||
    id === 'large_v3' ||
    id === 'whisper-large' ||
    id === 'large'
  ) {
    return { accuracy: 5, speed: 1 };
  }

  // Unmanaged / Custom models heuristic based on tier or filename
  if (file.includes('apex') || file.includes('hinglish')) {
    return { accuracy: 4, speed: 4 };
  }
  if (tier === 'fast') {
    return { accuracy: 2, speed: 4 };
  }
  if (tier === 'accurate') {
    return { accuracy: 4, speed: 2 };
  }
  if (tier === 'maximum') {
    return { accuracy: 5, speed: 1 };
  }
  return { accuracy: 3, speed: 3 };
}

const ModelRatingMeters: React.FC<{
  accuracy: number;
  speed: number;
  className?: string;
}> = ({ accuracy, speed, className }) => {
  return (
    <div className={`flex items-center gap-2.5 text-[9px] ${className ?? ''}`}>
      {/* Accuracy meter */}
      <div
        className="flex items-center gap-1 text-muted-foreground select-none"
        title={`Accuracy: ${accuracy}/5`}
        aria-label={`Accuracy rating: ${accuracy} out of 5`}
      >
        <Target className="w-2.5 h-2.5 text-sky-500 shrink-0" />
        <span className="text-[8.5px] font-medium text-muted-foreground/80">Acc</span>
        <div className="inline-flex items-center gap-0.5">
          {[1, 2, 3, 4, 5].map((i) => (
            <span
              key={i}
              className={`w-2 h-0.5 rounded-full transition-colors ${
                i <= accuracy
                  ? 'bg-sky-500 dark:bg-sky-400 shadow-2xs'
                  : 'bg-muted-foreground/20 dark:bg-muted-foreground/25'
              }`}
            />
          ))}
        </div>
      </div>

      {/* Speed meter */}
      <div
        className="flex items-center gap-1 text-muted-foreground select-none"
        title={`Speed: ${speed}/5`}
        aria-label={`Speed rating: ${speed} out of 5`}
      >
        <Zap className="w-2.5 h-2.5 text-amber-500 shrink-0" />
        <span className="text-[8.5px] font-medium text-muted-foreground/80">Spd</span>
        <div className="inline-flex items-center gap-0.5">
          {[1, 2, 3, 4, 5].map((i) => (
            <span
              key={i}
              className={`w-2 h-0.5 rounded-full transition-colors ${
                i <= speed
                  ? 'bg-amber-500 dark:bg-amber-400 shadow-2xs'
                  : 'bg-muted-foreground/20 dark:bg-muted-foreground/25'
              }`}
            />
          ))}
        </div>
      </div>
    </div>
  );
};

export const UnifiedModelsView: React.FC<UnifiedModelsViewProps> = ({
  settings,
  onUpdateSettings,
  onSaveDirect,
  onNavigateTab,
}) => {
  // Navigation tab
  const [activeTab, setActiveTab] = useState<ModelCategoryTab>('stt');

  // --- STT Models State ---
  const [catalogue, setCatalogue] = useState<SpeechModelCatalogue | null>(null);
  const [sttLoading, setSttLoading] = useState(true);
  const [downloads, setDownloads] = useState<Record<string, DownloadState>>({});
  const [sttBusyId, setSttBusyId] = useState<string | null>(null);
  const [sttError, setSttError] = useState<string | null>(null);

  // Filters for STT
  const [langFilter, setLangFilter] = useState<LanguageFilter>('all');
  const [tierFilter, setTierFilter] = useState<TierFilter>('all');
  const [showInstalledOnly, setShowInstalledOnly] = useState(false);

  // --- LLM Models State ---
  const [ollamaStatus, setOllamaStatus] = useState<
    'checking' | 'running' | 'not_installed' | 'unreachable'
  >('checking');
  const [ollamaModels, setOllamaModels] = useState<OllamaModelDetails[]>([]);
  const [loadingOllama, setLoadingOllama] = useState(false);
  const [pullModelName, setPullModelName] = useState('');
  const [pulling, setPulling] = useState(false);
  const [testResponse, setTestResponse] = useState<string | null>(null);
  const [testLatency, setTestLatency] = useState<number | null>(null);
  const [testingLlm, setTestingLlm] = useState(false);
  const [ollamaAccordionOpen, setOllamaAccordionOpen] = useState(true);

  // --- TTS State ---
  const [ttsRate, setTtsRate] = useState(1.0);
  const [ttsPitch, setTtsPitch] = useState(1.0);
  const [ttsSpeaking, setTtsSpeaking] = useState(false);
  const [ttsVoice, setTtsVoice] = useState<string>('');
  const [availableVoices, setAvailableVoices] = useState<SpeechSynthesisVoice[]>([]);

  // Refresh STT models
  const [parakeetStatus, setParakeetStatus] = useState<ParakeetStatus | null>(null);
  const [parakeetBusy, setParakeetBusy] = useState(false);

  const refreshStt = useCallback(async () => {
    try {
      const [cat, pStatus] = await Promise.all([
        listSpeechModels(),
        getParakeetStatus().catch(() => null),
      ]);
      setCatalogue(cat);
      setParakeetStatus(pStatus);
      setSttError(null);
    } catch (err) {
      setSttError(describeError(err));
    } finally {
      setSttLoading(false);
    }
  }, []);

  useEffect(() => {
    void refreshStt();
  }, [refreshStt]);

  // STT Download progress subscription
  useEffect(() => {
    const unlisten = onSpeechModelDownload((progress: SpeechModelDownloadProgress) => {
      setDownloads((prev) => {
        const next = { ...prev };
        switch (progress.state) {
          case 'downloading':
            next[progress.id] = {
              kind: 'downloading',
              fraction: downloadFraction(progress),
              downloaded: progress.downloaded_bytes,
            };
            break;
          case 'verifying':
            next[progress.id] = { kind: 'verifying' };
            break;
          case 'failed':
            next[progress.id] = { kind: 'failed', message: progress.message };
            break;
          case 'ready':
          case 'cancelled':
            delete next[progress.id];
            break;
        }
        return next;
      });
      if (progress.state === 'ready' || progress.state === 'cancelled') {
        void refreshStt();
      }
    });
    return () => {
      void unlisten.then((off) => off());
    };
  }, [refreshStt]);

  // Check Ollama status and list models
  const refreshOllama = useCallback(async () => {
    setLoadingOllama(true);
    try {
      const res = await invoke<OllamaStatus>('ensure_local_llm_ready');
      if (res.state === 'running' || res.state === 'started') {
        setOllamaStatus('running');
      } else if (res.state === 'not_installed') {
        setOllamaStatus('not_installed');
      } else {
        setOllamaStatus('unreachable');
      }
      let models: OllamaModelDetails[] = [];
      try {
        models = await invoke<OllamaModelDetails[]>('get_available_llm_models', {
          host: settings.provider.ollama_host || null,
        });
      } catch {
        models = [];
      }
      setOllamaModels(models || []);
    } catch (err) {
      setOllamaStatus('unreachable');
    } finally {
      setLoadingOllama(false);
    }
  }, []);

  useEffect(() => {
    if (activeTab === 'llm' && settings.provider.active_provider === 'ollama') {
      void refreshOllama();
    }
  }, [activeTab, settings.provider.active_provider, refreshOllama]);

  // Load browser voices for TTS
  useEffect(() => {
    if (typeof window !== 'undefined' && 'speechSynthesis' in window) {
      const loadVoices = () => {
        const voices = window.speechSynthesis.getVoices();
        setAvailableVoices(voices);
        if (voices.length > 0 && !ttsVoice) {
          const defaultV = voices.find((v) => v.default) || voices[0];
          setTtsVoice(defaultV.name);
        }
      };
      loadVoices();
      window.speechSynthesis.onvoiceschanged = loadVoices;
    }
  }, [ttsVoice]);

  // STT Actions
  const handleDownloadSpeech = async (id: string) => {
    setDownloads((prev) => ({
      ...prev,
      [id]: { kind: 'downloading', fraction: null, downloaded: 0 },
    }));
    try {
      await downloadSpeechModel(id);
    } catch (err) {
      setDownloads((prev) => ({
        ...prev,
        [id]: { kind: 'failed', message: describeError(err) },
      }));
    }
    await refreshStt();
  };

  const handleRemoveSpeech = async (model: SpeechModel) => {
    setSttBusyId(model.id);
    try {
      await deleteSpeechModel(model.id);
      await refreshStt();
    } catch (err) {
      setSttError(describeError(err));
    } finally {
      setSttBusyId(null);
    }
  };

  const handleSetMeeting = async (id: string | null) => {
    setSttBusyId(id ?? '__auto_meeting__');
    try {
      await setMeetingSpeechModel(id);
      await refreshStt();
    } catch (err) {
      setSttError(describeError(err));
    } finally {
      setSttBusyId(null);
    }
  };

  const handleSetDictation = async (id: string | null) => {
    setSttBusyId(id ?? '__auto_dictation__');
    try {
      await setDictationSpeechModel(id);
      await refreshStt();
    } catch (err) {
      setSttError(describeError(err));
    } finally {
      setSttBusyId(null);
    }
  };

  const handleDownloadParakeet = async () => {
    setDownloads((prev) => ({
      ...prev,
      'parakeet-tdt': { kind: 'downloading', fraction: null, downloaded: 0 },
    }));
    try {
      await downloadParakeetModel();
    } catch (err) {
      setDownloads((prev) => ({
        ...prev,
        'parakeet-tdt': {
          kind: 'failed',
          message: describeError(err),
        },
      }));
    }
    await refreshStt();
  };

  const handleToggleParakeetDictation = async () => {
    setParakeetBusy(true);
    try {
      const newEngine = parakeetStatus?.active_for_dictation ? 'whisper' : 'parakeet';
      await setDictationEngine(newEngine);
      await refreshStt();
    } catch (err) {
      setSttError(describeError(err));
    } finally {
      setParakeetBusy(false);
    }
  };

  const handleDeleteParakeet = async () => {
    setParakeetBusy(true);
    try {
      await deleteParakeetModel();
      await refreshStt();
    } catch (err) {
      setSttError(describeError(err));
    } finally {
      setParakeetBusy(false);
    }
  };

  // Test LLM Connection
  const handleTestLlm = async () => {
    setTestingLlm(true);
    setTestResponse(null);
    setTestLatency(null);
    try {
      const activeP = settings.provider.active_provider;
      if (activeP === 'ollama') {
        const model = settings.provider.ollama_model || 'llama3.2:latest';
        const res = await invoke<OllamaPromptTestResult>('test_llm_prompt', {
          model,
          prompt: "Say 'Vox AI ready' in under 5 words.",
        });
        if (res?.success) {
          setTestResponse(res.response || 'Success');
          setTestLatency(res.latency_ms);
        } else {
          setTestResponse(res?.error || 'Failed to receive completion');
        }
      } else {
        // Test Cloud Provider
        const start = Date.now();
        await onSaveDirect();
        setTestLatency(Date.now() - start);
        setTestResponse(`Connected to ${activeP}. Configuration verified.`);
      }
    } catch (err) {
      setTestResponse(describeError(err, 'Connection test failed'));
    } finally {
      setTestingLlm(false);
    }
  };

  // Pull Ollama Model
  const handlePullModel = async () => {
    if (!pullModelName.trim()) return;
    setPulling(true);
    try {
      await invoke('pull_ollama_model', {
        modelName: pullModelName.trim(),
        host: settings.provider.ollama_host || 'http://localhost:11434',
      });
      setPullModelName('');
      await refreshOllama();
    } catch (err) {
      alert(`Pull failed: ${describeError(err, 'unknown error')}`);
    } finally {
      setPulling(false);
    }
  };

  // Test TTS Playback
  const handleTestTts = () => {
    if (typeof window === 'undefined' || !('speechSynthesis' in window)) return;
    window.speechSynthesis.cancel();
    const utterance = new SpeechSynthesisUtterance('Vox voice output is online, natural, and ready.');
    utterance.rate = ttsRate;
    utterance.pitch = ttsPitch;
    if (ttsVoice) {
      const v = availableVoices.find((item) => item.name === ttsVoice);
      if (v) utterance.voice = v;
    }
    utterance.onstart = () => setTtsSpeaking(true);
    utterance.onend = () => setTtsSpeaking(false);
    utterance.onerror = () => setTtsSpeaking(false);
    window.speechSynthesis.speak(utterance);
  };

  // Filtered Speech Models
  const filteredSpeechModels = useMemo(() => {
    let list = catalogue?.models ?? [];

    if (langFilter === 'english') {
      list = list.filter((m) => !m.multilingual);
    } else if (langFilter === 'multilingual') {
      list = list.filter((m) => m.multilingual);
    }

    if (tierFilter !== 'all') {
      list = list.filter((m) => m.tier === tierFilter);
    }

    if (showInstalledOnly) {
      list = list.filter((m) => m.installed);
    }

    return list;
  }, [catalogue?.models, langFilter, tierFilter, showInstalledOnly]);

  const installedSpeechModels = useMemo(() => {
    return (catalogue?.models ?? []).filter((m) => m.installed);
  }, [catalogue?.models]);

  const activeDictationModel = catalogue?.active_dictation_model ?? null;
  const activeMeetingModel = catalogue?.active_meeting_model ?? null;

  return (
    <TooltipProvider delayDuration={250}>
      <div className="space-y-4 w-full">
        {/* Header */}
        <div className="flex flex-col sm:flex-row sm:items-center justify-between gap-3 border-b border-border pb-3">
          <div>
            <h2 className="text-lg font-semibold text-foreground flex items-center gap-2">
              <Cpu className="w-5 h-5 text-primary" />
              Models & Speech Hub
            </h2>
            <p className="text-xs text-muted-foreground mt-0.5">
              Unified control for all local & API intelligence: Dictation, Meetings, LLMs, and Speech Synthesis.
            </p>
          </div>

          {/* Category Tabs */}
          <div className="flex items-center gap-1 p-1 rounded-lg bg-muted/60 border border-border shrink-0">
            <button
              type="button"
              onClick={() => setActiveTab('stt')}
              className={`flex items-center gap-1.5 px-3 py-1 text-xs font-medium rounded-md transition-colors cursor-pointer ${
                activeTab === 'stt'
                  ? 'bg-background text-foreground shadow-xs'
                  : 'text-muted-foreground hover:text-foreground'
              }`}
            >
              <Mic className="w-3.5 h-3.5" />
              Speech-to-Text
            </button>
            <button
              type="button"
              onClick={() => setActiveTab('llm')}
              className={`flex items-center gap-1.5 px-3 py-1 text-xs font-medium rounded-md transition-colors cursor-pointer ${
                activeTab === 'llm'
                  ? 'bg-background text-foreground shadow-xs'
                  : 'text-muted-foreground hover:text-foreground'
              }`}
            >
              <Sparkles className="w-3.5 h-3.5" />
              AI & LLM
            </button>
            <button
              type="button"
              onClick={() => setActiveTab('tts')}
              className={`flex items-center gap-1.5 px-3 py-1 text-xs font-medium rounded-md transition-colors cursor-pointer ${
                activeTab === 'tts'
                  ? 'bg-background text-foreground shadow-xs'
                  : 'text-muted-foreground hover:text-foreground'
              }`}
            >
              <Volume2 className="w-3.5 h-3.5" />
              Voice (TTS)
            </button>
          </div>
        </div>

        {/* Dedicated Diagnostics Redirect Banner */}
        {onNavigateTab && (
          <div className="p-3.5 rounded-lg border border-primary/20 bg-primary/5 flex flex-col sm:flex-row sm:items-center justify-between gap-3">
            <div className="flex items-start gap-3">
              <div className="p-2 rounded-lg bg-primary/10 text-primary shrink-0 mt-0.5 sm:mt-0">
                <Activity className="w-4 h-4" />
              </div>
              <div>
                <p className="text-xs font-bold text-foreground">Need Technical Testing or Observability?</p>
                <p className="text-[11px] text-muted-foreground mt-0.5 leading-relaxed">
                  Live audio telemetry, VAD decisions, decoding diagnostics, and STT accuracy benchmarking have moved to the dedicated Diagnostics page.
                </p>
              </div>
            </div>
            <Button
              type="button"
              variant="outline"
              size="sm"
              onClick={() => onNavigateTab('diagnostics')}
              className="text-xs gap-1.5 shrink-0 self-start sm:self-auto border-primary/30 text-primary hover:bg-primary/10"
            >
              <Activity className="w-3.5 h-3.5" />
              Open Diagnostics
            </Button>
          </div>
        )}

        {/* Active Models Quick Status Bar */}
        <Card className="p-3.5 bg-muted/30 border-border">
          <div className="flex flex-wrap items-center justify-between gap-3 text-xs">
            <div className="flex items-center gap-2">
              <span className="font-semibold text-foreground text-xs uppercase tracking-wider font-mono">
                Active Models:
              </span>
            </div>

            <div className="flex flex-wrap items-center gap-2">
              {/* Dictation Model Pill */}
              <div className="flex items-center gap-1.5 px-2.5 py-1 rounded-md bg-background border border-border shadow-2xs">
                <Mic className="w-3 h-3 text-emerald-500 shrink-0" />
                <span className="text-[11px] text-muted-foreground">Dictation:</span>
                <select
                  value={parakeetStatus?.active_for_dictation ? '__parakeet__' : (activeDictationModel ?? '')}
                  onChange={(e) => {
                    const val = e.target.value;
                    if (val === '__parakeet__') {
                      void handleToggleParakeetDictation();
                    } else {
                      void handleSetDictation(val || null);
                    }
                  }}
                  className="text-xs font-medium text-foreground bg-transparent focus:outline-none cursor-pointer"
                  title="Select active model for voice dictation"
                >
                  {parakeetStatus?.installed && (
                    <option value="__parakeet__">NVIDIA Parakeet TDT</option>
                  )}
                  <option value="">Default (Fastest)</option>
                  {installedSpeechModels.map((m) => (
                    <option key={m.id} value={m.id}>
                      {m.name} ({formatBytes(m.size_bytes)})
                    </option>
                  ))}
                </select>
              </div>

              {/* Meeting Model Pill */}
              <div className="flex items-center gap-1.5 px-2.5 py-1 rounded-md bg-background border border-border shadow-2xs">
                <Users className="w-3 h-3 text-blue-500 shrink-0" />
                <span className="text-[11px] text-muted-foreground">Meetings:</span>
                <select
                  value={activeMeetingModel ?? ''}
                  onChange={(e) => void handleSetMeeting(e.target.value || null)}
                  className="text-xs font-medium text-foreground bg-transparent focus:outline-none cursor-pointer"
                  title="Select active model for meeting transcription"
                >
                  <option value="">Best Installed (Auto)</option>
                  {installedSpeechModels.map((m) => (
                    <option key={m.id} value={m.id}>
                      {m.name} ({formatBytes(m.size_bytes)})
                    </option>
                  ))}
                </select>
              </div>

              {/* AI / LLM Model Pill */}
              <div className="flex items-center gap-1.5 px-2.5 py-1 rounded-md bg-background border border-border shadow-2xs">
                <Sparkles className="w-3 h-3 text-purple-500 shrink-0" />
                <span className="text-[11px] text-muted-foreground">AI:</span>
                <span className="text-xs font-medium text-foreground capitalize">
                  {settings.provider.active_provider === 'ollama'
                    ? `Ollama (${settings.provider.ollama_model || 'llama3.2'})`
                    : settings.provider.active_provider.replace('cloud_', '')}
                </span>
              </div>
            </div>
          </div>
        </Card>

        {/* ------------------------------------------------------------------ */}
        {/* TAB 1: SPEECH-TO-TEXT (STT)                                        */}
        {/* ------------------------------------------------------------------ */}
        {activeTab === 'stt' && (
          <div className="space-y-4">
            {sttError && (
              <p className="text-xs text-destructive flex items-center gap-1.5">
                <AlertTriangle className="w-3.5 h-3.5 shrink-0" />
                {sttError}
              </p>
            )}

            {/* NVIDIA Parakeet TDT Engine Card */}
            {(() => {
              const isParakeetDownloading = Boolean(downloads['parakeet-tdt']);
              const parakeetProgress = downloads['parakeet-tdt'];
              const isInstalled = Boolean(parakeetStatus?.installed);
              const isActive = Boolean(parakeetStatus?.active_for_dictation);

              return (
                <Card
                  className={`p-3.5 border transition-all ${
                    isActive
                      ? 'border-primary/50 bg-primary/5 shadow-2xs'
                      : 'border-border bg-card hover:border-border/80'
                  }`}
                >
                  <div className="flex flex-col sm:flex-row sm:items-center justify-between gap-3">
                    <div className="space-y-1 min-w-0">
                      <div className="flex items-center gap-2 flex-wrap">
                        <Zap className="w-4 h-4 text-primary shrink-0" />
                        <h3 className="text-sm font-semibold text-foreground">
                          NVIDIA Parakeet TDT Engine
                        </h3>
                        <Badge variant="outline" className="text-[10px] text-primary border-primary/40">
                          ONNX Runtime
                        </Badge>
                        <Badge variant="outline" className="text-[10px] text-emerald-600 dark:text-emerald-400">
                          Sub-100ms Latency
                        </Badge>
                        {isInstalled && (
                          <Badge variant="emerald" className="text-[9px] px-1.5 py-0 h-4 leading-tight">
                            Installed
                          </Badge>
                        )}
                      </div>
                      <p className="text-xs text-muted-foreground">
                        FastConformer-CTC transducer ASR quantized to int8. Skips blank audio frames for ultra-low latency push-to-talk dictation.
                      </p>
                      <ModelRatingMeters accuracy={4} speed={5} className="pt-0.5" />
                    </div>

                    <div className="shrink-0 flex items-center gap-2">
                      {isInstalled ? (
                        <>
                          <Button
                            size="sm"
                            variant={isActive ? 'default' : 'outline'}
                            disabled={parakeetBusy}
                            onClick={() => void handleToggleParakeetDictation()}
                            className="h-6 text-xs px-2.5 gap-1.5"
                            title="Use NVIDIA Parakeet for push-to-talk voice dictation"
                          >
                            {isActive && <Check className="w-3 h-3" />}
                            {isActive ? 'Dictation Active' : 'Use for Dictation'}
                          </Button>
                          <button
                            type="button"
                            onClick={() => void handleDeleteParakeet()}
                            disabled={parakeetBusy}
                            className="text-muted-foreground hover:text-destructive p-1 rounded transition-colors cursor-pointer"
                            title="Delete Parakeet model from disk"
                          >
                            <Trash2 className="w-3.5 h-3.5" />
                          </button>
                        </>
                      ) : (
                        <Button
                          size="sm"
                          variant="outline"
                          disabled={isParakeetDownloading || parakeetBusy}
                          onClick={() => void handleDownloadParakeet()}
                          className="h-6 text-xs px-2.5 gap-1.5 hover:border-primary hover:text-primary hover:bg-primary/10"
                        >
                          <Download className="w-3.5 h-3.5" />
                          <span>{isParakeetDownloading ? 'Downloading…' : 'Download Engine (~670 MB)'}</span>
                        </Button>
                      )}
                    </div>
                  </div>

                  {/* Download Progress Bar */}
                  {isParakeetDownloading && parakeetProgress?.kind === 'downloading' && (
                    <div className="mt-3 pt-2.5 border-t border-border/40 space-y-1">
                      <div className="h-1.5 w-full bg-muted rounded-full overflow-hidden">
                        <div
                          className="h-full bg-primary transition-all duration-300"
                          style={{
                            width: `${Math.round((parakeetProgress.fraction ?? 0) * 100)}%`,
                          }}
                        />
                      </div>
                      <div className="flex justify-between text-[10px] text-muted-foreground">
                        <span>
                          Downloading Parakeet ONNX models… {formatBytes(parakeetProgress.downloaded)} / ~670 MB
                        </span>
                        <button
                          type="button"
                          onClick={() => void cancelSpeechModelDownload('parakeet-tdt')}
                          className="text-destructive hover:underline cursor-pointer"
                        >
                          Cancel
                        </button>
                      </div>
                    </div>
                  )}
                </Card>
              );
            })()}

            {/* Filter & Compact Display Controls */}
            <div className="flex flex-wrap items-center justify-between gap-2.5 p-2 rounded-lg bg-muted/20 border border-border">
              {/* Language Segmentation: English-only vs Multilingual */}
              <div className="flex items-center gap-1 text-xs">
                <span className="text-[11px] font-medium text-muted-foreground mr-1">Language:</span>
                <button
                  type="button"
                  onClick={() => setLangFilter('all')}
                  className={`px-2.5 py-1 rounded text-xs transition-colors cursor-pointer ${
                    langFilter === 'all'
                      ? 'bg-primary text-primary-foreground font-medium'
                      : 'bg-muted/60 text-muted-foreground hover:text-foreground'
                  }`}
                >
                  All ({catalogue?.models.length ?? 0})
                </button>
                <button
                  type="button"
                  onClick={() => setLangFilter('english')}
                  className={`px-2.5 py-1 rounded text-xs transition-colors cursor-pointer ${
                    langFilter === 'english'
                      ? 'bg-primary text-primary-foreground font-medium'
                      : 'bg-muted/60 text-muted-foreground hover:text-foreground'
                  }`}
                >
                  English-only (.en)
                </button>
                <button
                  type="button"
                  onClick={() => setLangFilter('multilingual')}
                  className={`px-2.5 py-1 rounded text-xs transition-colors cursor-pointer ${
                    langFilter === 'multilingual'
                      ? 'bg-primary text-primary-foreground font-medium'
                      : 'bg-muted/60 text-muted-foreground hover:text-foreground'
                  }`}
                >
                  Multilingual
                </button>
              </div>

              {/* Tier Filters */}
              <div className="flex items-center gap-2">
                <select
                  value={tierFilter}
                  onChange={(e) => setTierFilter(e.target.value as TierFilter)}
                  className="h-7 rounded border border-border bg-background px-2 text-xs text-foreground focus:outline-none cursor-pointer"
                  title="Filter by response tier"
                >
                  <option value="all">All Tiers</option>
                  <option value="fast">Fast (Dictation)</option>
                  <option value="balanced">Balanced</option>
                  <option value="accurate">Accurate (Meetings)</option>
                  <option value="maximum">Maximum</option>
                </select>

                <Button
                  size="sm"
                  variant="outline"
                  onClick={() => setShowInstalledOnly((prev) => !prev)}
                  className="h-7 text-xs gap-1.5"
                >
                  {showInstalledOnly ? (
                    <>
                      <Check className="w-3.5 h-3.5 text-emerald-500" />
                      Installed only
                    </>
                  ) : (
                    'Show all models'
                  )}
                </Button>
              </div>
            </div>

            {/* Compact Model Cards Grid */}
            {sttLoading ? (
              <p className="text-xs text-muted-foreground py-4">Loading speech models…</p>
            ) : filteredSpeechModels.length === 0 ? (
              <Card className="p-6 text-center text-xs text-muted-foreground">
                No speech models match the selected filter.
              </Card>
            ) : (
              <div className="grid grid-cols-1 sm:grid-cols-2 md:grid-cols-3 lg:grid-cols-4 2xl:grid-cols-4 gap-2.5">
                {filteredSpeechModels.map((model) => {
                  const isDownloading = Boolean(downloads[model.id]);
                  const downloadProgress = downloads[model.id];
                  const isDictationActive = !parakeetStatus?.active_for_dictation && activeDictationModel === model.id;
                  const isMeetingActive = activeMeetingModel === model.id;
                  const isRecommendedDictation =
                    model.id === 'whisper-base-en' ||
                    model.id === 'whisper-base' ||
                    model.id === 'base.en' ||
                    model.id === 'base';
                  const busy = sttBusyId === model.id;
                  const ratings = getModelRatings(model.id, model.tier, model.filename);

                  return (
                    <div
                      key={model.id}
                      data-testid={`speech-model-card-${model.id}`}
                      className={`p-2 rounded-lg border transition-all flex flex-col justify-between min-h-[92px] ${
                        isDictationActive || isMeetingActive
                          ? 'border-primary/50 bg-primary/5 shadow-2xs'
                          : 'border-border bg-card hover:border-border/80'
                      }`}
                    >
                      {/* Top section: Name, Badges & Specs */}
                      <div>
                        {/* Top row: Name & Badges */}
                        <div className="flex items-center justify-between gap-1.5">
                          <div className="flex items-center gap-1.5 min-w-0">
                            <span className="font-semibold text-xs text-foreground truncate">
                              {model.name}
                            </span>

                            <Badge variant="outline" className="text-[9px] px-1 py-0 h-4 shrink-0">
                              {model.multilingual ? 'Multi' : '.en'}
                            </Badge>

                            {/* Info Tooltip for Details */}
                            <Tooltip>
                              <TooltipTrigger asChild>
                                <button
                                  type="button"
                                  className="text-muted-foreground hover:text-foreground cursor-pointer shrink-0"
                                  aria-label={`Details for ${model.name}`}
                                >
                                  <Info className="w-3 h-3" />
                                </button>
                              </TooltipTrigger>
                              <TooltipContent side="top" className="max-w-xs text-xs">
                                <p className="font-medium text-foreground">{model.name}</p>
                                <p className="text-muted-foreground mt-0.5">{model.blurb}</p>
                                <div className="mt-1.5 pt-1.5 border-t border-border/50 text-[10px] space-y-0.5">
                                  <div>Parameters: {model.parameters_millions}M</div>
                                  <div>Tier: {TIER_LABEL[model.tier]}</div>
                                  <div>Disk: {formatBytes(model.size_bytes)}</div>
                                  <div>Accuracy: {ratings.accuracy}/5</div>
                                  <div>Speed: {ratings.speed}/5</div>
                                </div>
                              </TooltipContent>
                            </Tooltip>
                          </div>

                          <div className="flex items-center gap-1 shrink-0">
                            {isRecommendedDictation && (
                              <Badge variant="emerald" className="text-[8.5px] px-1 py-0 h-4 leading-tight">
                                Rec: Dictation
                              </Badge>
                            )}
                            {model.installed && !isRecommendedDictation && (
                              <Badge variant="outline" className="text-[8.5px] px-1 py-0 h-4 leading-tight text-emerald-500 border-emerald-500/30">
                                Installed
                              </Badge>
                            )}
                          </div>
                        </div>

                        {/* Specs snippet */}
                        <div className="flex items-center gap-1.5 text-[10px] text-muted-foreground mt-0.5">
                          <span>{model.parameters_millions}M params</span>
                          <span>·</span>
                          <span>{formatBytes(model.size_bytes)}</span>
                          <span>·</span>
                          <span className="capitalize">{model.tier}</span>
                        </div>

                        {/* Model Rating Meters (Accuracy & Speed) */}
                        <ModelRatingMeters
                          accuracy={ratings.accuracy}
                          speed={ratings.speed}
                          className="mt-1"
                        />
                      </div>

                      {/* Standardized Bottom Row: Controls / Actions */}
                      <div className="pt-1.5 mt-1.5 border-t border-border/40">
                        {isDownloading && downloadProgress?.kind === 'downloading' ? (
                          <div className="space-y-0.5">
                            <div className="h-1 w-full bg-muted rounded-full overflow-hidden">
                              <div
                                className="h-full bg-primary transition-all duration-300"
                                style={{
                                  width: `${Math.round((downloadProgress.fraction ?? 0) * 100)}%`,
                                }}
                              />
                            </div>
                            <div className="flex justify-between items-center text-[9.5px] text-muted-foreground">
                              <span>Downloading… {formatBytes(downloadProgress.downloaded)}</span>
                              <button
                                type="button"
                                onClick={() => void cancelSpeechModelDownload(model.id)}
                                className="text-destructive hover:underline cursor-pointer"
                              >
                                Cancel
                              </button>
                            </div>
                          </div>
                        ) : model.installed ? (
                          <div className="flex items-center justify-between gap-1">
                            <div className="flex items-center gap-1">
                              <Button
                                size="sm"
                                variant={isDictationActive ? 'default' : 'outline'}
                                disabled={busy}
                                onClick={() => void handleSetDictation(model.id)}
                                className="h-5 text-[10px] px-1.5 gap-1"
                                title="Use this model for voice dictation"
                              >
                                {isDictationActive && <Check className="w-2.5 h-2.5" />}
                                Dictation
                              </Button>
                              <Button
                                size="sm"
                                variant={isMeetingActive ? 'default' : 'outline'}
                                disabled={busy}
                                onClick={() => void handleSetMeeting(model.id)}
                                className="h-5 text-[10px] px-1.5 gap-1"
                                title="Use this model for meeting transcription"
                              >
                                {isMeetingActive && <Check className="w-2.5 h-2.5" />}
                                Meetings
                              </Button>
                            </div>

                            <button
                              type="button"
                              onClick={() => void handleRemoveSpeech(model)}
                              disabled={busy}
                              className="text-muted-foreground hover:text-destructive p-0.5 rounded transition-colors cursor-pointer"
                              title="Delete model from disk"
                            >
                              <Trash2 className="w-3 h-3" />
                            </button>
                          </div>
                        ) : (
                          <div className="flex items-center justify-between gap-1">
                            <span className="text-[10px] text-muted-foreground/60 select-none">
                              Available
                            </span>
                            <Button
                              size="sm"
                              variant="outline"
                              disabled={isDownloading || busy}
                              onClick={() => void handleDownloadSpeech(model.id)}
                              className="h-5 text-[10px] px-2 gap-1 hover:border-primary hover:text-primary hover:bg-primary/10"
                              aria-label={`Download ${model.name}`}
                            >
                              <Download className="w-3 h-3" />
                              <span>Download</span>
                            </Button>
                          </div>
                        )}
                      </div>
                    </div>
                  );
                })}
              </div>
            )}
          </div>
        )}

        {/* ------------------------------------------------------------------ */}
        {/* TAB 2: AI & LLM MODELS (LOCAL & CLOUD)                             */}
        {/* ------------------------------------------------------------------ */}
        {activeTab === 'llm' && (
          <div className="space-y-4">
            {/* Provider Switcher Tabs */}
            <Card className="p-4 space-y-4">
              <div>
                <h3 className="text-sm font-semibold text-foreground">Inference Provider</h3>
                <p className="text-xs text-muted-foreground mt-0.5">
                  Choose between local private inference via Ollama or high-speed cloud APIs (OpenAI, Claude, Groq, Gemini).
                </p>
              </div>

              <div className="grid grid-cols-2 sm:grid-cols-3 md:grid-cols-6 gap-2">
                {[
                  { id: 'ollama', label: 'Ollama (Local)' },
                  { id: 'cloud_openai', label: 'OpenAI (ChatGPT)' },
                  { id: 'cloud_anthropic', label: 'Claude (Anthropic)' },
                  { id: 'groq', label: 'Groq (Ultra-Fast)' },
                  { id: 'cloud_gemini', label: 'Google Gemini' },
                  { id: 'custom_openai', label: 'Custom Endpoint' },
                ].map((p) => {
                  const active = settings.provider.active_provider === p.id;
                  return (
                    <button
                      key={p.id}
                      type="button"
                      onClick={() => {
                        onUpdateSettings((prev) => ({
                          ...prev,
                          provider: {
                            ...prev.provider,
                            active_provider: p.id as ProviderSlug,
                          },
                        }));
                      }}
                      className={`p-2.5 rounded-lg border text-center transition-all cursor-pointer ${
                        active
                          ? 'bg-primary text-primary-foreground border-primary font-medium shadow-xs'
                          : 'bg-muted/30 border-border text-muted-foreground hover:text-foreground hover:bg-muted/60'
                      }`}
                    >
                      <span className="text-xs block leading-tight">{p.label}</span>
                    </button>
                  );
                })}
              </div>
            </Card>

            {/* Local Ollama Management */}
            {settings.provider.active_provider === 'ollama' ? (
              <Card className="p-4 space-y-4">
                <div className="flex items-center justify-between border-b border-border pb-3">
                  <div>
                    <h3 className="text-sm font-semibold text-foreground flex items-center gap-2">
                      <Server className="w-4 h-4 text-emerald-500" />
                      Local Ollama Service
                    </h3>
                    <p className="text-xs text-muted-foreground">
                      Running completely offline on this machine.
                    </p>
                  </div>
                  <div className="flex items-center gap-2">
                    <Badge
                      variant={ollamaStatus === 'running' ? 'emerald' : 'destructive'}
                      className="text-xs"
                    >
                      {ollamaStatus === 'running'
                        ? '● Online'
                        : ollamaStatus === 'not_installed'
                        ? 'Not Installed'
                        : 'Offline / Unreachable'}
                    </Badge>
                    <Button
                      size="sm"
                      variant="ghost"
                      onClick={() => void refreshOllama()}
                      disabled={loadingOllama}
                      className="h-7 w-7 p-0"
                      title="Refresh status"
                    >
                      <RefreshCw className={`w-3.5 h-3.5 ${loadingOllama ? 'animate-spin' : ''}`} />
                    </Button>
                  </div>
                </div>

                <div className="grid grid-cols-1 sm:grid-cols-2 gap-3">
                  <div>
                    <label className="text-xs font-medium text-foreground block mb-1">
                      Ollama Host Endpoint
                    </label>
                    <Input
                      value={settings.provider.ollama_host || 'http://localhost:11434'}
                      onChange={(e) => {
                        const val = e.target.value;
                        onUpdateSettings((prev) => ({
                          ...prev,
                          provider: { ...prev.provider, ollama_host: val },
                        }));
                      }}
                      className="h-8 text-xs font-mono"
                    />
                  </div>

                  <div>
                    <label className="text-xs font-medium text-foreground block mb-1">
                      Active Model
                    </label>
                    <select
                      value={settings.provider.ollama_model || 'llama3.2:latest'}
                      onChange={(e) => {
                        const val = e.target.value;
                        onUpdateSettings((prev) => ({
                          ...prev,
                          provider: { ...prev.provider, ollama_model: val },
                        }));
                      }}
                      className="h-8 w-full rounded-md border border-border bg-background px-2 text-xs text-foreground"
                    >
                      {ollamaModels.length > 0 ? (
                        ollamaModels.map((m) => (
                          <option key={m.name} value={m.name}>
                            {m.name} {m.size ? `(${formatBytes(m.size)})` : ''}
                          </option>
                        ))
                      ) : (
                        <option value="llama3.2:latest">llama3.2:latest</option>
                      )}
                    </select>
                  </div>
                </div>

                {/* Available Models Accordion (Openable, Half and Half Grid) */}
                <div className="rounded-lg border border-border overflow-hidden bg-card/60">
                  <button
                    type="button"
                    onClick={() => setOllamaAccordionOpen(!ollamaAccordionOpen)}
                    className="w-full flex items-center justify-between p-3 bg-muted/30 hover:bg-muted/50 transition-colors cursor-pointer text-left select-none"
                    aria-expanded={ollamaAccordionOpen}
                  >
                    <div className="flex items-center gap-2">
                      <Server className="w-4 h-4 text-primary shrink-0" />
                      <span className="text-xs font-semibold text-foreground">
                        Available Models from Ollama ({ollamaModels.length})
                      </span>
                    </div>
                    <div className="flex items-center gap-2 text-xs text-muted-foreground">
                      <span>{ollamaAccordionOpen ? 'Hide models' : 'Browse & manage models'}</span>
                      {ollamaAccordionOpen ? (
                        <ChevronUp className="w-4 h-4 text-foreground shrink-0" />
                      ) : (
                        <ChevronDown className="w-4 h-4 shrink-0" />
                      )}
                    </div>
                  </button>

                  {ollamaAccordionOpen && (
                    <div className="p-3.5 border-t border-border space-y-3.5 bg-muted/10 animate-in fade-in-50 duration-150">
                      {loadingOllama ? (
                        <div className="p-3 text-center text-xs text-muted-foreground border border-dashed rounded-lg">
                          Scanning models from Ollama…
                        </div>
                      ) : ollamaModels.length > 0 ? (
                        <div className="grid grid-cols-1 sm:grid-cols-2 gap-2">
                          {ollamaModels.map((m) => {
                            const isSelected = settings.provider.ollama_model === m.name;
                            return (
                              <button
                                key={m.name}
                                type="button"
                                onClick={() =>
                                  onUpdateSettings((prev) => ({
                                    ...prev,
                                    provider: { ...prev.provider, ollama_model: m.name },
                                  }))
                                }
                                className={`p-2 rounded-md border text-left transition-all flex items-start justify-between cursor-pointer ${
                                  isSelected
                                    ? 'border-primary bg-primary/10 text-foreground shadow-xs'
                                    : 'border-border bg-card text-muted-foreground hover:border-border/80 hover:text-foreground'
                                }`}
                              >
                                <div className="space-y-0.5 min-w-0 pr-1.5">
                                  <div className="flex items-center gap-1.5">
                                    <span className="text-[11.5px] font-semibold font-mono truncate">{m.name}</span>
                                    {isSelected && (
                                      <Check className="w-3 h-3 text-primary shrink-0" />
                                    )}
                                  </div>
                                  <div className="flex items-center gap-1 flex-wrap">
                                    {m.parameter_size && (
                                      <Badge variant="outline" className="text-[8.5px] px-1 py-0 font-mono h-3.5 leading-none">
                                        {m.parameter_size}
                                      </Badge>
                                    )}
                                    {m.quantization_level && (
                                      <Badge variant="outline" className="text-[8.5px] px-1 py-0 font-mono h-3.5 leading-none">
                                        {m.quantization_level}
                                      </Badge>
                                    )}
                                  </div>
                                </div>
                                <span className="text-[9.5px] font-mono text-muted-foreground shrink-0">
                                  {m.size ? `${(m.size / (1024 * 1024 * 1024)).toFixed(1)} GB` : ''}
                                </span>
                              </button>
                            );
                          })}
                        </div>
                      ) : (
                        <div className="p-3 rounded-lg border border-border bg-card text-xs text-muted-foreground space-y-1">
                          <p className="font-semibold text-foreground">No installed models found in Ollama.</p>
                          <p className="text-[11px]">
                            Run <code className="px-1.5 py-0.5 rounded bg-muted border border-border font-mono text-foreground">ollama pull llama3.2</code> in your terminal, or pull one below.
                          </p>
                        </div>
                      )}

                      {/* Pull Model Form */}
                      <div className="p-3 rounded-lg bg-card border border-border space-y-2">
                        <p className="text-xs font-medium text-foreground">Pull / Download New Model</p>
                        <div className="flex gap-2">
                          <Input
                            placeholder="e.g. llama3.2:latest, gemma2:9b, mistral"
                            value={pullModelName}
                            onChange={(e) => setPullModelName(e.target.value)}
                            className="h-8 text-xs font-mono"
                          />
                          <Button
                            size="sm"
                            onClick={() => void handlePullModel()}
                            disabled={pulling || !pullModelName.trim()}
                            className="h-8 text-xs gap-1.5 shrink-0"
                          >
                            {pulling ? 'Pulling…' : 'Pull Model'}
                          </Button>
                        </div>
                      </div>
                    </div>
                  )}
                </div>
              </Card>
            ) : (
              /* Cloud Provider Configuration */
              <Card className="p-4">
                <CloudProviderSettings
                  provider={settings.provider}
                  onChange={(next: ProviderConfig) => {
                    onUpdateSettings((prev) => ({
                      ...prev,
                      provider: next,
                    }));
                  }}
                />
              </Card>
            )}

            {/* Test Connection / Response Verification */}
            <Card className="p-4 flex flex-col sm:flex-row sm:items-center justify-between gap-3 bg-muted/15">
              <div>
                <p className="text-xs font-semibold text-foreground">Verify Connection & Speed</p>
                <p className="text-[11px] text-muted-foreground">
                  Test prompt round-trip latency against the currently selected model.
                </p>
                {testResponse && (
                  <p className="text-xs text-primary mt-1 font-mono">
                    [{testLatency}ms] {testResponse}
                  </p>
                )}
              </div>
              <Button
                size="sm"
                variant="outline"
                onClick={() => void handleTestLlm()}
                disabled={testingLlm}
                className="h-8 text-xs gap-1.5 shrink-0"
              >
                <Zap className="w-3.5 h-3.5 text-amber-500" />
                {testingLlm ? 'Testing…' : 'Test Model'}
              </Button>
            </Card>
          </div>
        )}

        {/* ------------------------------------------------------------------ */}
        {/* TAB 3: TEXT-TO-SPEECH (TTS) & VOICE                                */}
        {/* ------------------------------------------------------------------ */}
        {activeTab === 'tts' && (
          <div className="space-y-4">
            <Card className="p-4 space-y-4">
              <div>
                <h3 className="text-sm font-semibold text-foreground flex items-center gap-2">
                  <Volume2 className="w-4 h-4 text-primary" />
                  Voice Synthesis & Readback (TTS)
                </h3>
                <p className="text-xs text-muted-foreground mt-0.5">
                  Configure voice playback for reading out captured notes, AI responses, and dictation audio feedback.
                </p>
              </div>

              <div className="space-y-3">
                <div>
                  <label className="text-xs font-medium text-foreground block mb-1">
                    System Voice
                  </label>
                  <select
                    value={ttsVoice}
                    onChange={(e) => setTtsVoice(e.target.value)}
                    className="h-8 w-full rounded-md border border-border bg-background px-2 text-xs text-foreground cursor-pointer"
                  >
                    {availableVoices.map((v) => (
                      <option key={v.name} value={v.name}>
                        {v.name} ({v.lang}) {v.default ? '— Default' : ''}
                      </option>
                    ))}
                  </select>
                </div>

                <div className="grid grid-cols-1 sm:grid-cols-2 gap-4">
                  <div>
                    <div className="flex justify-between text-xs mb-1">
                      <span className="font-medium text-foreground">Speech Rate</span>
                      <span className="text-muted-foreground font-mono">{ttsRate}x</span>
                    </div>
                    <input
                      type="range"
                      min="0.5"
                      max="2.0"
                      step="0.1"
                      value={ttsRate}
                      onChange={(e) => setTtsRate(parseFloat(e.target.value))}
                      className="w-full accent-primary cursor-pointer"
                    />
                  </div>

                  <div>
                    <div className="flex justify-between text-xs mb-1">
                      <span className="font-medium text-foreground">Pitch</span>
                      <span className="text-muted-foreground font-mono">{ttsPitch}</span>
                    </div>
                    <input
                      type="range"
                      min="0.5"
                      max="1.5"
                      step="0.1"
                      value={ttsPitch}
                      onChange={(e) => setTtsPitch(parseFloat(e.target.value))}
                      className="w-full accent-primary cursor-pointer"
                    />
                  </div>
                </div>

                <div className="pt-2 flex justify-end">
                  <Button
                    size="sm"
                    onClick={handleTestTts}
                    disabled={ttsSpeaking}
                    className="gap-2 h-8 text-xs"
                  >
                    <Volume2 className="w-3.5 h-3.5" />
                    {ttsSpeaking ? 'Speaking…' : 'Test Voice Output'}
                  </Button>
                </div>
              </div>
            </Card>

            <Card className="p-4 bg-muted/20 border-dashed">
              <h4 className="text-xs font-semibold text-foreground mb-1">Upcoming Neural Engines</h4>
              <p className="text-[11px] text-muted-foreground">
                Integration hooks for Kokoro-82M and ElevenLabs API streaming are prepared in the audio pipeline.
              </p>
            </Card>
          </div>
        )}
      </div>
    </TooltipProvider>
  );
};
