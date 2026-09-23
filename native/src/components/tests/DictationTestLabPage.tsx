import React, { useState, useEffect, useRef } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import {
  Mic,
  Square,
  FlaskConical,
  Copy,
  Check,
  Clock,
  Zap,
  FileText,
  AlertTriangle,
  History,
  Download,
  Trash2,
  ChevronRight,
  ExternalLink,
  ChevronDown,
  RotateCcw,
  Sparkles,
  Info,
  Activity,
} from 'lucide-react';
import { Button } from '@/components/ui/button';
import { Input } from '@/components/ui/input';
import { Badge } from '@/components/ui/badge';
import {
  Tooltip,
  TooltipContent,
  TooltipProvider,
  TooltipTrigger,
} from '@/components/ui/tooltip';
import { cn } from '@/lib/utils';
import type {
  AvailableModelTarget,
  CleanupStyleInfo,
  DictationTestRun,
  DictationTestSummary,
  DictationTestModelResult,
  DictationTestProgressPayload,
  RunDictationTestRequest,
  DiffSpan,
} from '@/types/benchmark';

export const DictationTestLabPage: React.FC = () => {
  // Available hardware/engine discovery
  const [availableModels, setAvailableModels] = useState<AvailableModelTarget[]>([]);
  const [availableCleanupStyles, setAvailableCleanupStyles] = useState<CleanupStyleInfo[]>([]);
  const [loadingDiscovery, setLoadingDiscovery] = useState(true);

  // User selections
  const [selectedTargetIds, setSelectedTargetIds] = useState<string[]>([]);
  const [selectedCleanupStyles, setSelectedCleanupStyles] = useState<string[]>(['raw', 'faithful', 'clean', 'polished', 'concise']);
  const [productionCleanupStyle, setProductionCleanupStyle] = useState<string>('faithful');
  const [referenceTranscript, setReferenceTranscript] = useState('');
  const [testLabel, setTestLabel] = useState('');

  // Recording & Execution state
  const [isRecording, setIsRecording] = useState(false);
  const [recordingSeconds, setRecordingSeconds] = useState(0);
  const [isProcessing, setIsProcessing] = useState(false);
  const [processingStatus, setProcessingStatus] = useState<string>('');
  const [progressiveResults, setProgressiveResults] = useState<DictationTestModelResult[]>([]);
  const [currentRun, setCurrentRun] = useState<DictationTestRun | null>(null);
  const [errorMessage, setErrorMessage] = useState<string | null>(null);

  // History & Detail View
  const [historyList, setHistoryList] = useState<DictationTestSummary[]>([]);
  const [showHistory, setShowHistory] = useState(false);
  const [selectedModelTab, setSelectedModelTab] = useState<string | null>(null);
  const [copiedKey, setCopiedKey] = useState<string | null>(null);

  const recordingTimerRef = useRef<NodeJS.Timeout | null>(null);
  const isRecordingRef = useRef(false);

  // Leaving the page mid-recording must release the recorder, or hotkey
  // dictation and meetings fail with an active session until restart.
  useEffect(() => {
    isRecordingRef.current = isRecording;
  }, [isRecording]);

  useEffect(() => {
    return () => {
      if (isRecordingRef.current) {
        invoke('cancel_dictation_test_recording').catch(() => {
          // Non-fatal: nothing more we can do after unmount
        });
      }
    };
  }, []);

  // 1. Initial Discovery
  useEffect(() => {
    const fetchDiscovery = async () => {
      setLoadingDiscovery(true);
      try {
        const [models, styles] = await Promise.all([
          invoke<AvailableModelTarget[]>('get_available_test_models'),
          invoke<CleanupStyleInfo[]>('get_available_cleanup_styles'),
        ]);

        setAvailableModels(models);
        setAvailableCleanupStyles(styles);

        // Default behaviour per product requirement: ALL available (installed & compiled-in) models selected by default
        const installedIds = models
          .filter((m) => m.installed && m.compiled_in)
          .map((m) => m.target_id);
        setSelectedTargetIds(installedIds);

        // Cleanup default
        const defaultStyles = styles.filter((s) => s.is_default).map((s) => s.id);
        if (defaultStyles.length > 0) {
          setSelectedCleanupStyles(defaultStyles);
        }
      } catch (err: any) {
        setErrorMessage(err?.message || 'Failed to discover speech models');
      } finally {
        setLoadingDiscovery(false);
      }
    };

    fetchDiscovery();
    loadHistory();
  }, []);

  // 2. Listen for progressive benchmark updates
  useEffect(() => {
    const unlistenPromise = listen<DictationTestProgressPayload>(
      'dictation-test-progress',
      (event) => {
        const { current_model_index, total_models, completed_model } = event.payload;
        setProcessingStatus(`Testing ${current_model_index} / ${total_models} models...`);
        setProgressiveResults((prev) => {
          const filtered = prev.filter((p) => p.target_id !== completed_model.target_id);
          return [...filtered, completed_model];
        });
      }
    );

    return () => {
      unlistenPromise.then((unlisten) => unlisten());
    };
  }, []);

  const loadHistory = async () => {
    try {
      const history = await invoke<DictationTestSummary[]>('get_dictation_test_history');
      setHistoryList(history);
    } catch {
      // Non-fatal
    }
  };

  // Recording Timer
  useEffect(() => {
    if (isRecording) {
      setRecordingSeconds(0);
      recordingTimerRef.current = setInterval(() => {
        setRecordingSeconds((prev) => prev + 1);
      }, 1000);
    } else {
      if (recordingTimerRef.current) clearInterval(recordingTimerRef.current);
    }
    return () => {
      if (recordingTimerRef.current) clearInterval(recordingTimerRef.current);
    };
  }, [isRecording]);

  // Model Selection Toggles
  const toggleModel = (targetId: string) => {
    setSelectedTargetIds((prev) =>
      prev.includes(targetId) ? prev.filter((id) => id !== targetId) : [...prev, targetId]
    );
  };

  const selectAllInstalledModels = () => {
    const installed = availableModels
      .filter((m) => m.installed && m.compiled_in)
      .map((m) => m.target_id);
    setSelectedTargetIds(installed);
  };

  const deselectAllModels = () => {
    setSelectedTargetIds([]);
  };

  const toggleCleanupStyle = (styleId: string) => {
    setSelectedCleanupStyles((prev) =>
      prev.includes(styleId) ? prev.filter((s) => s !== styleId) : [...prev, styleId]
    );
  };

  // Start Benchmark Dictation
  const handleStartRecording = async () => {
    setErrorMessage(null);
    if (selectedTargetIds.length === 0) {
      setErrorMessage('Please select at least one speech model to benchmark.');
      return;
    }

    try {
      await invoke('start_dictation_test_recording');
      setIsRecording(true);
      setCurrentRun(null);
      setProgressiveResults([]);
    } catch (err: any) {
      setErrorMessage(err?.message || 'Failed to start test recording');
    }
  };

  // Stop Recording and Fan Out to Models
  const handleStopRecording = async () => {
    if (!isRecording) return;
    setIsRecording(false);
    setIsProcessing(true);
    setProcessingStatus(`Initiating benchmark across ${selectedTargetIds.length} models...`);

    const requestPayload: RunDictationTestRequest = {
      selected_target_ids: selectedTargetIds,
      selected_cleanup_styles: selectedCleanupStyles,
      production_cleanup_style: productionCleanupStyle,
      reference_transcript: referenceTranscript.trim() ? referenceTranscript.trim() : null,
      test_label: testLabel.trim() ? testLabel.trim() : null,
    };

    try {
      const run = await invoke<DictationTestRun>('stop_dictation_test_recording', {
        request: requestPayload,
      });
      setCurrentRun(run);
      setProgressiveResults(run.model_results);
      if (run.model_results.length > 0) {
        setSelectedModelTab(run.model_results[0].target_id);
      }
      loadHistory();
    } catch (err: any) {
      setErrorMessage(err?.message || 'Benchmarking failed');
    } finally {
      setIsProcessing(false);
      setProcessingStatus('');
    }
  };

  // Copy text helper
  const handleCopy = (text: string, key: string) => {
    navigator.clipboard.writeText(text);
    setCopiedKey(key);
    setTimeout(() => setCopiedKey(null), 2000);
  };

  // Explicit Injection
  const handleInject = async (text: string) => {
    try {
      await invoke('inject_dictation_test_result', { text });
    } catch (err: any) {
      setErrorMessage(err?.message || 'Failed to inject text');
    }
  };

  // Export report
  const handleExport = async (format: 'markdown' | 'json') => {
    if (!currentRun) return;
    try {
      const report = await invoke<string>('export_dictation_test_report', {
        testId: currentRun.test_id,
        format,
      });

      const blob = new Blob([report], { type: format === 'json' ? 'application/json' : 'text/markdown' });
      const url = URL.createObjectURL(blob);
      const a = document.createElement('a');
      a.href = url;
      a.download = `dictation_benchmark_${currentRun.test_id.slice(0, 8)}.${format === 'json' ? 'json' : 'md'}`;
      a.click();
      URL.revokeObjectURL(url);
    } catch (err: any) {
      setErrorMessage(err?.message || 'Failed to export report');
    }
  };

  // Load Past Run
  const handleSelectHistoryRun = async (testId: string) => {
    try {
      const run = await invoke<DictationTestRun | null>('get_dictation_test_run', { testId });
      if (run) {
        setCurrentRun(run);
        setProgressiveResults(run.model_results);
        if (run.model_results.length > 0) {
          setSelectedModelTab(run.model_results[0].target_id);
        }
        setShowHistory(false);
      }
    } catch (err: any) {
      setErrorMessage(err?.message || 'Failed to load test run');
    }
  };

  const handleDeleteHistoryRun = async (testId: string, e: React.MouseEvent) => {
    e.stopPropagation();
    try {
      await invoke('delete_dictation_test_run', { testId });
      loadHistory();
      if (currentRun?.test_id === testId) {
        setCurrentRun(null);
        setProgressiveResults([]);
      }
    } catch (err: any) {
      setErrorMessage(err?.message || 'Failed to delete test run');
    }
  };

  const activeModelResult = progressiveResults.find((m) => m.target_id === selectedModelTab) || progressiveResults[0];

  return (
    <div className="flex-1 min-w-0 max-w-7xl mx-auto w-full pb-16 space-y-6 text-foreground animate-in fade-in duration-200">
      {/* 1. Header & Navigation Context */}
      <div className="flex flex-col sm:flex-row sm:items-center justify-between gap-4 border-b border-border pb-5">
        <div>
          <div className="flex items-center gap-2 text-xs font-medium text-muted-foreground mb-1">
            <span>Tests</span>
            <ChevronRight className="w-3 h-3" />
            <span className="text-foreground font-semibold">Dictation Test Lab</span>
          </div>
          <h1 className="text-2xl font-bold tracking-tight text-foreground flex items-center gap-2.5">
            <FlaskConical className="w-6 h-6 text-amber-500" />
            Dictation Test Lab
          </h1>
          <p className="text-xs text-muted-foreground mt-0.5">
            Record once. Test every available model. Compare speed, accuracy and cleanup behaviour side-by-side.
          </p>
        </div>

        <div className="flex items-center gap-2">
          <Button
            variant="outline"
            size="sm"
            onClick={() => setShowHistory(!showHistory)}
            className="text-xs gap-1.5"
          >
            <History className="w-3.5 h-3.5" />
            <span>Test History</span>
            {historyList.length > 0 && (
              <Badge variant="secondary" className="px-1.5 py-0 text-[10px] ml-1">
                {historyList.length}
              </Badge>
            )}
          </Button>

          {currentRun && (
            <TooltipProvider>
              <Tooltip>
                <TooltipTrigger asChild>
                  <Button
                    variant="outline"
                    size="sm"
                    onClick={() => handleExport('markdown')}
                    className="text-xs gap-1.5"
                  >
                    <Download className="w-3.5 h-3.5" />
                    <span>Export MD</span>
                  </Button>
                </TooltipTrigger>
                <TooltipContent>Download Markdown benchmark report</TooltipContent>
              </Tooltip>
            </TooltipProvider>
          )}
        </div>
      </div>

      {/* Error notification banner */}
      {errorMessage && (
        <div className="p-3 bg-rose-500/10 border border-rose-500/30 rounded-lg flex items-center justify-between text-xs text-rose-600 dark:text-rose-400">
          <div className="flex items-center gap-2">
            <AlertTriangle className="w-4 h-4 shrink-0" />
            <span>{errorMessage}</span>
          </div>
          <Button variant="ghost" size="sm" onClick={() => setErrorMessage(null)} className="h-6 px-2 text-xs">
            Dismiss
          </Button>
        </div>
      )}

      {/* History Drawer / Modal */}
      {showHistory && (
        <div className="p-4 bg-card border border-border rounded-xl shadow-md space-y-3 animate-in fade-in duration-150">
          <div className="flex items-center justify-between border-b border-border/60 pb-2">
            <h3 className="text-sm font-semibold text-foreground flex items-center gap-2">
              <History className="w-4 h-4 text-muted-foreground" />
              Past Benchmarking Runs
            </h3>
            <Button variant="ghost" size="sm" onClick={() => setShowHistory(false)} className="h-7 text-xs">
              Close
            </Button>
          </div>

          {historyList.length === 0 ? (
            <div className="py-6 text-center text-xs text-muted-foreground">
              No previous dictation test runs saved. Complete a test above to record measurements.
            </div>
          ) : (
            <div className="grid grid-cols-1 md:grid-cols-2 lg:grid-cols-3 gap-3 max-h-80 overflow-y-auto pr-1">
              {historyList.map((item) => (
                <div
                  key={item.test_id}
                  onClick={() => handleSelectHistoryRun(item.test_id)}
                  className="p-3 bg-muted/40 hover:bg-muted/80 border border-border/80 rounded-lg cursor-pointer transition-all flex flex-col justify-between gap-2"
                >
                  <div className="flex items-start justify-between gap-2">
                    <div>
                      <div className="text-xs font-semibold text-foreground">
                        {item.label || item.created_at_formatted}
                      </div>
                      <div className="text-[11px] text-muted-foreground">
                        {item.created_at_formatted} · {item.audio_duration_seconds.toFixed(1)}s audio
                      </div>
                    </div>
                    <Button
                      variant="ghost"
                      size="icon"
                      onClick={(e) => handleDeleteHistoryRun(item.test_id, e)}
                      className="h-6 w-6 text-muted-foreground hover:text-rose-500"
                      title="Delete run"
                    >
                      <Trash2 className="w-3.5 h-3.5" />
                    </Button>
                  </div>

                  <div className="flex items-center gap-2 text-[11px] text-muted-foreground pt-1 border-t border-border/40">
                    <span>{item.successful_models_count} models</span>
                    {item.fastest_total_ms && (
                      <span className="font-mono text-emerald-600 dark:text-emerald-400">
                        {item.fastest_total_ms}ms total
                      </span>
                    )}
                  </div>
                </div>
              ))}
            </div>
          )}
        </div>
      )}

      {/* 2. Available Models Section */}
      <div className="p-5 bg-card border border-border rounded-xl shadow-xs space-y-4">
        <div className="flex items-center justify-between flex-wrap gap-2">
          <div>
            <h2 className="text-sm font-semibold uppercase tracking-wider text-muted-foreground flex items-center gap-2">
              <span>Available Recognizers</span>
              <Badge variant="outline" className="text-xs font-normal">
                {selectedTargetIds.length} / {availableModels.filter((m) => m.installed && m.compiled_in).length} selected
              </Badge>
            </h2>
            <p className="text-xs text-muted-foreground mt-0.5">
              Select which speech recognizers and model variants will process the recorded audio.
            </p>
          </div>

          <div className="flex items-center gap-2">
            <Button
              variant="ghost"
              size="sm"
              onClick={selectAllInstalledModels}
              className="text-xs h-7 px-2.5"
            >
              Select All
            </Button>
            <Button
              variant="ghost"
              size="sm"
              onClick={deselectAllModels}
              className="text-xs h-7 px-2.5 text-muted-foreground"
            >
              Clear
            </Button>
          </div>
        </div>

        {loadingDiscovery ? (
          <div className="py-8 text-center text-xs text-muted-foreground flex items-center justify-center gap-2">
            <div className="w-3.5 h-3.5 border-2 border-primary border-t-transparent rounded-full animate-spin" />
            Discovering installed models and recognizer engines...
          </div>
        ) : (
          <div className="grid grid-cols-1 md:grid-cols-2 lg:grid-cols-3 gap-3">
            {availableModels.map((target) => {
              const isSelected = selectedTargetIds.includes(target.target_id);
              const isUsable = target.installed && target.compiled_in;

              return (
                <div
                  key={target.target_id}
                  onClick={() => isUsable && toggleModel(target.target_id)}
                  className={cn(
                    "p-3.5 rounded-lg border transition-all select-none flex flex-col justify-between gap-3",
                    !isUsable
                      ? "opacity-50 bg-muted/20 border-border/40 cursor-not-allowed"
                      : isSelected
                      ? "bg-amber-500/5 border-amber-500/40 shadow-xs cursor-pointer"
                      : "bg-muted/30 hover:bg-muted/60 border-border cursor-pointer"
                  )}
                >
                  <div className="flex items-start justify-between gap-2">
                    <div className="space-y-1">
                      <div className="flex items-center gap-1.5">
                        <input
                          type="checkbox"
                          checked={isSelected}
                          disabled={!isUsable}
                          onChange={() => {}}
                          className="rounded border-border text-amber-500 focus:ring-amber-500 h-3.5 w-3.5"
                        />
                        <span className="font-semibold text-xs text-foreground">
                          {target.model_name}
                        </span>
                      </div>
                      <div className="text-[11px] font-mono text-muted-foreground pl-5">
                        {target.model_filename}
                      </div>
                    </div>

                    <div className="flex flex-col items-end gap-1">
                      <Badge variant={isUsable ? 'secondary' : 'outline'} className="text-[10px] px-1.5 py-0">
                        {target.engine_display_name}
                      </Badge>
                      {target.tier && (
                        <span className="text-[10px] text-muted-foreground font-medium">
                          {target.tier}
                        </span>
                      )}
                    </div>
                  </div>

                  <p className="text-[11px] text-muted-foreground leading-relaxed pl-5">
                    {target.blurb}
                  </p>

                  <div className="flex items-center gap-2 pt-2 border-t border-border/40 text-[10px] text-muted-foreground pl-5">
                    {target.parameters_millions && (
                      <span>{target.parameters_millions}M params</span>
                    )}
                    {target.size_bytes && (
                      <span>· {(target.size_bytes / (1024 * 1024)).toFixed(0)} MB</span>
                    )}
                    <span>· {target.capabilities.local ? 'Local' : 'Cloud'}</span>
                    {!isUsable && <span className="text-rose-500 font-medium">· Not installed</span>}
                  </div>
                </div>
              );
            })}
          </div>
        )}
      </div>

      {/* 3. Test Controls & Recording Bar */}
      <div className="p-5 bg-card border border-border rounded-xl shadow-xs space-y-4">
        <h2 className="text-sm font-semibold uppercase tracking-wider text-muted-foreground">
          Test Controls & Configuration
        </h2>

        {/* Optional Context Inputs */}
        <div className="grid grid-cols-1 md:grid-cols-2 gap-4">
          <div className="space-y-1.5">
            <label className="text-xs font-medium text-foreground flex items-center gap-1.5">
              <span>Test Label (Optional)</span>
              <span className="text-muted-foreground text-[11px]">(e.g. English baseline, Fast speech, Hinglish)</span>
            </label>
            <Input
              placeholder="e.g. English baseline"
              value={testLabel}
              onChange={(e) => setTestLabel(e.target.value)}
              className="text-xs h-8"
              disabled={isRecording || isProcessing}
            />
          </div>

          <div className="space-y-1.5">
            <label className="text-xs font-medium text-foreground flex items-center gap-1.5">
              <span>Reference Transcript (Optional)</span>
              <span className="text-muted-foreground text-[11px]">(Paste exact text spoken to calculate WER/CER)</span>
            </label>
            <Input
              placeholder="Paste intended words here to compute Word Error Rate..."
              value={referenceTranscript}
              onChange={(e) => setReferenceTranscript(e.target.value)}
              className="text-xs h-8"
              disabled={isRecording || isProcessing}
            />
          </div>
        </div>

        {/* Cleanup Styles Selection */}
        <div className="space-y-2 pt-2 border-t border-border/60">
          <label className="text-xs font-medium text-foreground">
            Cleanup Styles to Evaluate
          </label>
          <div className="flex flex-wrap items-center gap-2">
            {availableCleanupStyles.map((style) => {
              const isChecked = selectedCleanupStyles.includes(style.id);
              return (
                <button
                  key={style.id}
                  type="button"
                  onClick={() => toggleCleanupStyle(style.id)}
                  disabled={isRecording || isProcessing}
                  className={cn(
                    "px-3 py-1.5 rounded-lg text-xs font-medium border transition-colors flex items-center gap-1.5 cursor-pointer",
                    isChecked
                      ? "bg-primary/10 border-primary text-primary"
                      : "bg-muted/40 border-border text-muted-foreground hover:bg-muted/80 hover:text-foreground"
                  )}
                  title={style.description}
                >
                  <input
                    type="checkbox"
                    checked={isChecked}
                    onChange={() => {}}
                    className="rounded border-border text-primary focus:ring-primary h-3 w-3 pointer-events-none"
                  />
                  <span>{style.display_name}</span>
                </button>
              );
            })}
          </div>
        </div>

        {/* Production Cleanup Style Selector */}
        <div className="flex flex-col sm:flex-row sm:items-center justify-between gap-3 p-3 bg-muted/20 border border-border/80 rounded-lg">
          <div className="space-y-0.5">
            <div className="text-xs font-semibold text-foreground flex items-center gap-1.5">
              <span>Production Cleanup Path</span>
              <Badge variant="outline" className="text-[10px] font-mono">Real E2E Target</Badge>
            </div>
            <p className="text-[11px] text-muted-foreground">
              Select which cleanup style represents the production Universal Dictation pipeline. This isolates single-path E2E latency without multi-style benchmark overhead.
            </p>
          </div>

          <div className="flex items-center gap-2 shrink-0">
            <select
              value={productionCleanupStyle}
              onChange={(e) => setProductionCleanupStyle(e.target.value)}
              disabled={isRecording || isProcessing}
              aria-label="Production Cleanup Path"
              className="h-8 text-xs font-medium rounded-md border border-border bg-background px-3 py-1 text-foreground focus:outline-none focus:ring-1 focus:ring-primary cursor-pointer"
            >
              <option value="all">All Styles (Compare All Paths)</option>
              {availableCleanupStyles.map((s) => (
                <option key={s.id} value={s.id}>
                  {s.display_name}
                </option>
              ))}
            </select>
          </div>
        </div>

        {/* Prominent Test Dictation Button */}
        <div className="pt-4 flex flex-col items-center justify-center gap-3">
          {!isRecording && !isProcessing && (
            <Button
              size="lg"
              onClick={handleStartRecording}
              className="h-14 px-8 text-sm font-semibold rounded-full bg-amber-600 hover:bg-amber-500 text-white shadow-lg transition-transform hover:scale-105 active:scale-95 flex items-center gap-3"
            >
              <Mic className="w-5 h-5" />
              <span>Test Dictation</span>
            </Button>
          )}

          {isRecording && (
            <Button
              size="lg"
              onClick={handleStopRecording}
              variant="destructive"
              className="h-14 px-8 text-sm font-semibold rounded-full shadow-lg animate-pulse flex items-center gap-3"
            >
              <Square className="w-5 h-5 fill-current" />
              <span>Stop Recording ({recordingSeconds}s)</span>
            </Button>
          )}

          {isProcessing && (
            <div className="flex flex-col items-center gap-2 text-xs text-muted-foreground animate-in fade-in">
              <div className="flex items-center gap-2">
                <div className="w-4 h-4 border-2 border-primary border-t-transparent rounded-full animate-spin" />
                <span className="font-medium text-foreground">{processingStatus || 'Benchmarking models...'}</span>
              </div>
              <p className="text-[11px] text-muted-foreground">Evaluating speech recognizers across selected configurations...</p>
            </div>
          )}

          <p className="text-[11px] text-muted-foreground text-center">
            {isRecording
              ? "Speaking naturally. Press 'Stop Recording' to fan out across all selected models."
              : "Single-recording rule: Exactly one audio buffer will be captured and tested against all selected models."}
          </p>
        </div>
      </div>

      {/* 4. Results Section */}
      {progressiveResults.length > 0 && (
        <div className="space-y-6 animate-in fade-in duration-300">
          {/* Summary Comparison Table */}
          <div className="p-5 bg-card border border-border rounded-xl shadow-xs space-y-4">
            <div className="flex items-center justify-between flex-wrap gap-2">
              <div>
                <h2 className="text-sm font-semibold uppercase tracking-wider text-muted-foreground flex items-center gap-2">
                  <Activity className="w-4 h-4 text-primary" />
                  <span>Model Benchmark Comparison</span>
                  <Badge variant="secondary" className="text-xs font-normal">
                    {progressiveResults.length} models
                  </Badge>
                </h2>
                <p className="text-xs text-muted-foreground mt-0.5">
                  Direct head-to-head comparison on the identical captured audio. Click any row for in-depth trace.
                </p>
              </div>

              {currentRun?.created_at_formatted && (
                <Badge variant="outline" className="text-[11px] font-mono">
                  {currentRun.created_at_formatted}
                </Badge>
              )}
            </div>

            <div className="overflow-x-auto border border-border rounded-lg">
              <table className="w-full text-left text-xs border-collapse">
                <thead>
                  <tr className="bg-muted/50 border-b border-border text-muted-foreground font-semibold">
                    <th className="py-2.5 px-3">Model</th>
                    <th className="py-2.5 px-3 text-right">Model Load</th>
                    <th className="py-2.5 px-3 text-right">Queue</th>
                    <th className="py-2.5 px-3 text-right">STT</th>
                    {productionCleanupStyle === 'all' ? (
                      availableCleanupStyles.map((s) => (
                        <th key={s.id} className="py-2.5 px-3 text-right font-bold text-foreground">
                          {s.display_name} E2E
                        </th>
                      ))
                    ) : (
                      <>
                        <th className="py-2.5 px-3 text-right capitalize">{productionCleanupStyle}</th>
                        <th className="py-2.5 px-3 text-right font-bold text-foreground">Production E2E</th>
                      </>
                    )}
                    <th className="py-2.5 px-3 text-right">RTF</th>
                    {referenceTranscript.trim() && <th className="py-2.5 px-3 text-right">WER</th>}
                    {referenceTranscript.trim() && <th className="py-2.5 px-3 text-right">CER</th>}
                    <th className="py-2.5 px-3 text-center">Status</th>
                  </tr>
                </thead>
                <tbody className="divide-y divide-border">
                  {progressiveResults.map((m) => {
                    const prodCleanupMs = m.cleanup_results[productionCleanupStyle]?.duration_ms ?? (productionCleanupStyle === 'raw' ? 0 : (m.timings.production_cleanup_duration_ms ?? 0));
                    const prodE2eMs = m.timings.production_e2e_by_style?.[productionCleanupStyle] ?? (
                      m.timings.recording_to_audio_ready_ms +
                      m.timings.model_load_ms +
                      m.timings.stt_execution_ms +
                      m.timings.stt_to_text_available_ms +
                      prodCleanupMs
                    );

                    return (
                      <tr
                        key={m.target_id}
                        onClick={() => setSelectedModelTab(m.target_id)}
                        className={cn(
                          "cursor-pointer transition-colors hover:bg-muted/40",
                          selectedModelTab === m.target_id && "bg-amber-500/5 font-medium"
                        )}
                      >
                        <td className="py-2.5 px-3">
                          <div className="font-semibold text-foreground">{m.model_name}</div>
                          <div className="text-[10px] font-mono text-muted-foreground">
                            {m.config.model_filename}
                          </div>
                        </td>
                        <td className="py-2.5 px-3 text-right font-mono">
                          <span>{m.timings.model_load_ms} ms</span>
                          {m.timings.is_cold_load && (
                            <span className="ml-1 text-[10px] text-amber-500 font-sans" title="Cold load from disk">(cold)</span>
                          )}
                        </td>
                        <td className="py-2.5 px-3 text-right font-mono text-muted-foreground">
                          {m.timings.queue_wait_ms} ms
                        </td>
                        <td className="py-2.5 px-3 text-right font-mono font-medium text-foreground">
                          {m.timings.stt_execution_ms} ms
                        </td>
                        {productionCleanupStyle === 'all' ? (
                          availableCleanupStyles.map((s) => {
                            const e2e = m.timings.production_e2e_by_style?.[s.id] ?? (
                              m.timings.recording_to_audio_ready_ms +
                              m.timings.model_load_ms +
                              m.timings.stt_execution_ms +
                              m.timings.stt_to_text_available_ms +
                              (m.cleanup_results[s.id]?.duration_ms ?? 0)
                            );
                            return (
                              <td key={s.id} className="py-2.5 px-3 text-right font-mono font-bold text-primary">
                                {e2e} ms
                              </td>
                            );
                          })
                        ) : (
                          <>
                            <td className="py-2.5 px-3 text-right font-mono text-muted-foreground">
                              {prodCleanupMs} ms
                            </td>
                            <td className="py-2.5 px-3 text-right font-mono font-bold text-primary">
                              {prodE2eMs} ms
                            </td>
                          </>
                        )}
                        <td className="py-2.5 px-3 text-right font-mono">
                          <span
                            className={cn(
                              "px-1.5 py-0.5 rounded text-[11px]",
                              m.timings.rtf < 1.0
                                ? "text-emerald-600 dark:text-emerald-400 bg-emerald-500/10"
                                : "text-amber-600 dark:text-amber-400 bg-amber-500/10"
                            )}
                          >
                            {m.timings.rtf.toFixed(2)}x
                          </span>
                        </td>
                        {referenceTranscript.trim() && (
                          <td className="py-2.5 px-3 text-right font-mono font-medium">
                            {m.accuracy ? `${(m.accuracy.wer * 100).toFixed(1)}%` : '—'}
                          </td>
                        )}
                        {referenceTranscript.trim() && (
                          <td className="py-2.5 px-3 text-right font-mono text-muted-foreground">
                            {m.accuracy ? `${(m.accuracy.cer * 100).toFixed(1)}%` : '—'}
                          </td>
                        )}
                        <td className="py-2.5 px-3 text-center">
                          {m.success ? (
                            <Check className="w-4 h-4 text-emerald-500 mx-auto" />
                          ) : (
                            <span title={m.error || 'Failed'} className="inline-flex">
                              <AlertTriangle className="w-4 h-4 text-rose-500 mx-auto" />
                            </span>
                          )}
                        </td>
                      </tr>
                    );
                  })}
                </tbody>
              </table>
            </div>

            <div className="flex flex-wrap items-center justify-between text-[11px] text-muted-foreground gap-2 pt-1">
              <div className="flex items-center gap-1">
                <Info className="w-3.5 h-3.5 shrink-0" />
                <span>Production E2E = Audio Prep + Model Load + STT + {productionCleanupStyle} Cleanup. Excludes multi-model benchmark queue wait.</span>
              </div>
              <div>
                <span>RTF &lt; 1.0x indicates processing finished faster than real-time audio.</span>
              </div>
            </div>
          </div>

          {/* Timing Timeline Visualization */}
          <div className="p-5 bg-card border border-border rounded-xl shadow-xs space-y-6">
            <div>
              <h2 className="text-sm font-semibold uppercase tracking-wider text-muted-foreground flex items-center gap-2">
                <Clock className="w-4 h-4 text-muted-foreground" />
                Production E2E Latency Breakdown
              </h2>
              <p className="text-xs text-muted-foreground mt-0.5">
                Exact component decomposition for each model on the isolated production path (Prep → Load → STT → {productionCleanupStyle}).
              </p>
            </div>

            <div className="space-y-4">
              {progressiveResults.map((m) => {
                const prodCleanupMs = m.cleanup_results[productionCleanupStyle]?.duration_ms ?? m.timings.production_cleanup_duration_ms ?? 0;
                const e2eTotal = Math.max(m.timings.production_e2e_ms ?? m.timings.total_latency_ms, 1);
                const prepPct = ((m.timings.recording_to_audio_ready_ms / e2eTotal) * 100).toFixed(1);
                const loadPct = ((m.timings.model_load_ms / e2eTotal) * 100).toFixed(1);
                const sttPct = ((m.timings.stt_execution_ms / e2eTotal) * 100).toFixed(1);
                const cleanPct = ((prodCleanupMs / e2eTotal) * 100).toFixed(1);

                return (
                  <div key={m.target_id} className="space-y-1.5">
                    <div className="flex items-center justify-between text-xs">
                      <span className="font-semibold text-foreground flex items-center gap-2">
                        {m.model_name}
                        {m.timings.is_cold_load && (
                          <Badge variant="outline" className="text-[10px] text-amber-500 py-0">Cold Load</Badge>
                        )}
                      </span>
                      <span className="font-mono font-semibold text-primary">
                        Production E2E: {e2eTotal} ms
                      </span>
                    </div>

                    <div className="h-6 w-full bg-muted rounded-md overflow-hidden flex text-[10px] text-white font-mono select-none">
                      {m.timings.recording_to_audio_ready_ms > 0 && (
                        <div
                          style={{ width: `${prepPct}%` }}
                          className="bg-slate-500 flex items-center justify-center px-1 truncate"
                          title={`Audio Prep: ${m.timings.recording_to_audio_ready_ms}ms`}
                        >
                          prep {m.timings.recording_to_audio_ready_ms}ms
                        </div>
                      )}
                      {m.timings.model_load_ms > 0 && (
                        <div
                          style={{ width: `${loadPct}%` }}
                          className="bg-amber-600 flex items-center justify-center px-1 truncate"
                          title={`Model Load: ${m.timings.model_load_ms}ms`}
                        >
                          load {m.timings.model_load_ms}ms
                        </div>
                      )}
                      {m.timings.stt_execution_ms > 0 && (
                        <div
                          style={{ width: `${sttPct}%` }}
                          className="bg-blue-600 flex items-center justify-center px-1 truncate font-semibold"
                          title={`STT: ${m.timings.stt_execution_ms}ms`}
                        >
                          STT {m.timings.stt_execution_ms}ms
                        </div>
                      )}
                      {prodCleanupMs > 0 && (
                        <div
                          style={{ width: `${cleanPct}%` }}
                          className="bg-emerald-600 flex items-center justify-center px-1 truncate font-semibold"
                          title={`${productionCleanupStyle}: ${prodCleanupMs}ms`}
                        >
                          {productionCleanupStyle} {prodCleanupMs}ms
                        </div>
                      )}
                    </div>
                  </div>
                );
              })}
            </div>

            {/* Legend */}
            <div className="flex flex-wrap items-center gap-4 text-[11px] text-muted-foreground pt-2 border-t border-border/60">
              <div className="flex items-center gap-1.5">
                <span className="w-2.5 h-2.5 rounded-full bg-slate-500" />
                <span>Audio Preparation</span>
              </div>
              <div className="flex items-center gap-1.5">
                <span className="w-2.5 h-2.5 rounded-full bg-amber-600" />
                <span>Model Load (Cold)</span>
              </div>
              <div className="flex items-center gap-1.5">
                <span className="w-2.5 h-2.5 rounded-full bg-blue-600" />
                <span>STT Execution</span>
              </div>
              <div className="flex items-center gap-1.5">
                <span className="w-2.5 h-2.5 rounded-full bg-emerald-600" />
                <span>{productionCleanupStyle} Cleanup</span>
              </div>
            </div>
          </div>

          {/* Individual Model Detail View */}
          {activeModelResult && (
            <div className="p-5 bg-card border border-border rounded-xl shadow-xs space-y-5">
              {/* Tab Selector */}
              <div className="flex items-center gap-2 border-b border-border pb-3 overflow-x-auto">
                {progressiveResults.map((m) => (
                  <button
                    key={m.target_id}
                    type="button"
                    onClick={() => setSelectedModelTab(m.target_id)}
                    className={cn(
                      "px-3.5 py-1.5 rounded-lg text-xs font-semibold whitespace-nowrap transition-colors flex items-center gap-2 cursor-pointer",
                      selectedModelTab === m.target_id
                        ? "bg-amber-500 text-white shadow-xs"
                        : "bg-muted/40 hover:bg-muted text-muted-foreground hover:text-foreground"
                    )}
                  >
                    <span>{m.model_name}</span>
                    <span className="font-mono text-[10px] opacity-80">
                      {m.timings.production_e2e_ms ?? m.timings.total_latency_ms}ms
                    </span>
                  </button>
                ))}
              </div>

              {/* Model Header & Config */}
              <div className="flex flex-col md:flex-row md:items-center justify-between gap-3 bg-muted/20 p-3.5 rounded-lg border border-border/60">
                <div className="space-y-0.5">
                  <h3 className="text-sm font-bold text-foreground flex items-center gap-2">
                    {activeModelResult.model_name}
                    <Badge variant="outline" className="font-mono text-[10px]">
                      {activeModelResult.config.backend}
                    </Badge>
                  </h3>
                  <div className="text-xs text-muted-foreground">
                    Model: <code className="font-mono text-foreground">{activeModelResult.config.model_filename}</code> ·
                    Strategy: <code className="font-mono text-foreground">{activeModelResult.config.decoding_strategy}</code> ·
                    Language: <code className="font-mono text-foreground">{activeModelResult.config.language || 'Auto'}</code>
                  </div>
                </div>

                <div className="flex items-center gap-2">
                  <Button
                    variant="outline"
                    size="sm"
                    onClick={() => handleInject(activeModelResult.raw_transcript)}
                    className="text-xs gap-1.5"
                    title="Paste directly into current cursor target"
                  >
                    <ExternalLink className="w-3.5 h-3.5" />
                    <span>Inject Raw</span>
                  </Button>
                </div>
              </div>

              {/* Key Metrics Cards */}
              <div className="grid grid-cols-2 sm:grid-cols-4 gap-3">
                <div className="p-3 bg-background border border-primary/30 rounded-lg shadow-xs">
                  <div className="text-[10px] uppercase font-semibold text-primary">Production E2E</div>
                  <div className="text-xl font-extrabold font-mono text-primary mt-0.5">
                    {activeModelResult.timings.production_e2e_ms ?? activeModelResult.timings.total_latency_ms} ms
                  </div>
                  <div className="text-[10px] text-muted-foreground mt-0.5 truncate">
                    Prep + Load + STT + {activeModelResult.timings.production_cleanup_style || productionCleanupStyle}
                  </div>
                </div>

                <div className="p-3 bg-background border border-border rounded-lg shadow-xs">
                  <div className="text-[10px] uppercase font-semibold text-muted-foreground">STT Inference</div>
                  <div className="text-xl font-bold font-mono text-blue-500 mt-0.5">
                    {activeModelResult.timings.stt_execution_ms} ms
                  </div>
                  <div className="text-[10px] text-muted-foreground mt-0.5">
                    Pure engine decode
                  </div>
                </div>

                <div className="p-3 bg-background border border-border rounded-lg shadow-xs">
                  <div className="text-[10px] uppercase font-semibold text-muted-foreground">Real-Time Factor</div>
                  <div className="text-xl font-bold font-mono text-foreground mt-0.5">
                    {activeModelResult.timings.rtf.toFixed(2)}x
                  </div>
                  <div className="text-[10px] text-muted-foreground mt-0.5">
                    {activeModelResult.timings.rtf < 1.0 ? 'Faster than real-time' : 'Slower than real-time'}
                  </div>
                </div>

                <div className="p-3 bg-background border border-border rounded-lg shadow-xs">
                  <div className="text-[10px] uppercase font-semibold text-muted-foreground">Benchmark Wall Clock</div>
                  <div className="text-xl font-bold font-mono text-foreground mt-0.5">
                    {activeModelResult.timings.benchmark_wall_clock_ms ?? activeModelResult.timings.total_latency_ms} ms
                  </div>
                  <div className="text-[10px] text-muted-foreground mt-0.5">
                    Queue wait: {activeModelResult.timings.queue_wait_ms} ms
                  </div>
                </div>
              </div>

              {/* Production Path Breakdown */}
              <div className="p-4 bg-muted/20 border border-border rounded-lg space-y-3">
                <div className="flex items-center justify-between">
                  <h4 className="text-xs font-semibold text-foreground uppercase tracking-wider flex items-center gap-1.5">
                    <Clock className="w-3.5 h-3.5 text-muted-foreground" />
                    <span>Production-Path Latency Breakdown</span>
                  </h4>
                  <Badge variant="outline" className="text-[10px] font-mono">
                    Single-Path Component Sum
                  </Badge>
                </div>

                <div className="grid grid-cols-2 sm:grid-cols-6 gap-2.5">
                  <div className="p-2.5 bg-background border border-border rounded-md">
                    <div className="text-[10px] text-muted-foreground">Audio Prep</div>
                    <div className="text-sm font-bold font-mono text-foreground mt-0.5">
                      {activeModelResult.timings.recording_to_audio_ready_ms} ms
                    </div>
                  </div>

                  <div className="p-2.5 bg-background border border-border rounded-md">
                    <div className="text-[10px] text-muted-foreground">Model Load</div>
                    <div className="text-sm font-bold font-mono text-foreground mt-0.5 flex items-center gap-1">
                      <span>{activeModelResult.timings.model_load_ms} ms</span>
                      {activeModelResult.timings.is_cold_load && (
                        <span className="text-[9px] text-amber-500 font-sans font-medium">(cold)</span>
                      )}
                    </div>
                  </div>

                  <div className="p-2.5 bg-background border border-border rounded-md">
                    <div className="text-[10px] text-muted-foreground">Lock Wait</div>
                    <div className="text-sm font-bold font-mono text-foreground mt-0.5">
                      {activeModelResult.timings.lock_wait_ms ?? 0} ms
                    </div>
                  </div>

                  <div className="p-2.5 bg-background border border-border rounded-md">
                    <div className="text-[10px] text-muted-foreground">STT Inference</div>
                    <div className="text-sm font-bold font-mono text-blue-500 mt-0.5">
                      {activeModelResult.timings.stt_execution_ms} ms
                    </div>
                  </div>

                  <div className="p-2.5 bg-background border border-border rounded-md">
                    <div className="text-[10px] text-muted-foreground capitalize">
                      {activeModelResult.timings.production_cleanup_style || (productionCleanupStyle === 'all' ? 'faithful' : productionCleanupStyle)} Cleanup
                    </div>
                    <div className="text-sm font-bold font-mono text-emerald-500 mt-0.5">
                      {activeModelResult.timings.production_cleanup_duration_ms ?? activeModelResult.cleanup_results[productionCleanupStyle]?.duration_ms ?? 0} ms
                    </div>
                  </div>

                  <div className="p-2.5 bg-primary/5 border border-primary/30 rounded-md">
                    <div className="text-[10px] font-semibold text-primary">Production E2E</div>
                    <div className="text-sm font-bold font-mono text-primary mt-0.5">
                      {activeModelResult.timings.production_e2e_ms ?? activeModelResult.timings.total_latency_ms} ms
                    </div>
                  </div>
                </div>

                {activeModelResult.timings.production_e2e_by_style && Object.keys(activeModelResult.timings.production_e2e_by_style).length > 0 && (
                  <div className="pt-2 border-t border-border/40 space-y-1.5">
                    <div className="text-[11px] font-semibold text-foreground">All Evaluated Production Paths:</div>
                    <div className="grid grid-cols-2 sm:grid-cols-5 gap-2">
                      {Object.entries(activeModelResult.timings.production_e2e_by_style).map(([style, e2e]) => (
                        <div key={style} className="p-2 bg-muted/40 rounded border border-border/60 text-xs space-y-0.5">
                          <div className="font-semibold capitalize text-foreground flex items-center justify-between">
                            <span>{style}</span>
                            <span className="text-[10px] text-muted-foreground font-normal">
                              {activeModelResult.cleanup_results[style]?.duration_ms ?? 0}ms
                            </span>
                          </div>
                          <div className="font-mono text-primary font-bold">{e2e} ms</div>
                        </div>
                      ))}
                    </div>
                  </div>
                )}

                <div className="flex flex-wrap items-center gap-4 text-[11px] text-muted-foreground pt-2 border-t border-border/40">
                  <div>
                    <span className="font-medium text-foreground">Benchmark Queue Wait: </span>
                    <span className="font-mono">{activeModelResult.timings.queue_wait_ms} ms</span>
                  </div>
                  <div>
                    <span className="font-medium text-foreground">Recognizer Call Duration: </span>
                    <span className="font-mono">{activeModelResult.timings.recognizer_call_duration_ms ?? activeModelResult.timings.stt_execution_ms} ms</span>
                  </div>
                  <div>
                    <span className="font-medium text-foreground">All Cleanups Benchmark Time: </span>
                    <span className="font-mono">{activeModelResult.timings.cleanup_benchmark_total_ms ?? activeModelResult.timings.total_cleanup_ms} ms</span>
                  </div>
                  <div>
                    <span className="font-medium text-foreground">Model Benchmark Wall Clock: </span>
                    <span className="font-mono">{activeModelResult.timings.benchmark_wall_clock_ms ?? activeModelResult.timings.total_latency_ms} ms</span>
                  </div>
                </div>
              </div>

              {/* Raw STT Transcription */}
              <div className="space-y-2">
                <div className="flex items-center justify-between">
                  <label className="text-xs font-semibold uppercase tracking-wider text-muted-foreground flex items-center gap-2">
                    <FileText className="w-3.5 h-3.5" />
                    Raw Speech Recognition Output
                    <Badge variant="secondary" className="text-[10px] font-mono">
                      {activeModelResult.word_count} words · {activeModelResult.char_count} chars
                    </Badge>
                  </label>

                  <Button
                    variant="ghost"
                    size="sm"
                    onClick={() => handleCopy(activeModelResult.raw_transcript, `raw_${activeModelResult.target_id}`)}
                    className="h-7 text-xs gap-1.5 text-muted-foreground hover:text-foreground"
                  >
                    {copiedKey === `raw_${activeModelResult.target_id}` ? (
                      <>
                        <Check className="w-3.5 h-3.5 text-emerald-500" />
                        <span>Copied!</span>
                      </>
                    ) : (
                      <>
                        <Copy className="w-3.5 h-3.5" />
                        <span>Copy Raw</span>
                      </>
                    )}
                  </Button>
                </div>

                <div className="p-4 bg-muted/30 border border-border rounded-lg text-xs leading-relaxed font-sans select-text whitespace-pre-wrap">
                  {activeModelResult.raw_transcript || <span className="text-muted-foreground italic">(Empty / No speech produced)</span>}
                </div>
              </div>

              {/* Accuracy breakdown if reference exists */}
              {activeModelResult.accuracy && (
                <div className="p-4 bg-muted/20 border border-border rounded-lg space-y-2">
                  <div className="flex items-center justify-between">
                    <h4 className="text-xs font-semibold text-foreground uppercase tracking-wider">
                      Accuracy Assessment (vs Reference)
                    </h4>
                    <span className="text-[10px] text-muted-foreground">
                      Normalized Levenshtein alignment (case & punctuation ignored)
                    </span>
                  </div>
                  <div className="grid grid-cols-2 sm:grid-cols-4 gap-3 pt-1">
                    <div className="p-2.5 bg-background border border-border rounded-md">
                      <div className="text-[10px] text-muted-foreground">Word Error Rate (WER)</div>
                      <div className="text-base font-bold font-mono text-foreground">
                        {(activeModelResult.accuracy.wer * 100).toFixed(1)}%
                      </div>
                    </div>
                    <div className="p-2.5 bg-background border border-border rounded-md">
                      <div className="text-[10px] text-muted-foreground">Character Error Rate (CER)</div>
                      <div className="text-base font-bold font-mono text-foreground">
                        {(activeModelResult.accuracy.cer * 100).toFixed(1)}%
                      </div>
                    </div>
                    <div className="p-2.5 bg-background border border-border rounded-md">
                      <div className="text-[10px] text-muted-foreground">Substitutions / Deletions</div>
                      <div className="text-xs font-semibold font-mono text-foreground mt-1">
                        {activeModelResult.accuracy.substitutions} sub / {activeModelResult.accuracy.deletions} del
                      </div>
                    </div>
                    <div className="p-2.5 bg-background border border-border rounded-md">
                      <div className="text-[10px] text-muted-foreground">Insertions</div>
                      <div className="text-xs font-semibold font-mono text-foreground mt-1">
                        {activeModelResult.accuracy.insertions} ins
                      </div>
                    </div>
                  </div>
                </div>
              )}

              {/* Cleanup Results & Diff */}
              {Object.keys(activeModelResult.cleanup_results).length > 0 && (
                <div className="space-y-4 pt-2">
                  <div className="flex items-center justify-between flex-wrap gap-2">
                    <h4 className="text-xs font-semibold uppercase tracking-wider text-muted-foreground flex items-center gap-2">
                      <Sparkles className="w-3.5 h-3.5 text-amber-500" />
                      Cleanup Styles Benchmark Results
                    </h4>
                    <span className="text-[11px] text-muted-foreground font-mono">
                      Cumulative Cleanup Time: {activeModelResult.timings.cleanup_benchmark_total_ms ?? activeModelResult.timings.total_cleanup_ms} ms
                    </span>
                  </div>

                  <div className="grid grid-cols-1 gap-3.5">
                    {Object.entries(activeModelResult.cleanup_results).map(([styleKey, clean]) => (
                      <div
                        key={styleKey}
                        className="p-4 bg-muted/20 border border-border rounded-lg space-y-2.5"
                      >
                        <div className="flex items-center justify-between flex-wrap gap-2">
                          <div className="flex items-center gap-2">
                            <span className="font-semibold text-xs text-foreground">
                              {clean.style_display_name}
                            </span>
                            <Badge variant="secondary" className="text-[10px] font-mono">
                              {clean.duration_ms} ms
                            </Badge>
                            {clean.queue_wait_ms !== undefined && clean.queue_wait_ms > 0 && (
                              <span className="text-[10px] text-muted-foreground font-mono" title="Queue wait behind preceding cleanup styles">
                                (queue: {clean.queue_wait_ms}ms)
                              </span>
                            )}
                            {styleKey === (activeModelResult.timings.production_cleanup_style || productionCleanupStyle) && (
                              <Badge variant="default" className="text-[10px] bg-primary text-primary-foreground font-medium">
                                Production Path
                              </Badge>
                            )}
                            {clean.changed ? (
                              <Badge variant="outline" className="text-[10px] text-amber-600 dark:text-amber-400 border-amber-500/30">
                                Modified Text
                              </Badge>
                            ) : (
                              <Badge variant="outline" className="text-[10px] text-muted-foreground">
                                Unchanged
                              </Badge>
                            )}
                          </div>

                          <div className="flex items-center gap-2">
                            <Button
                              variant="ghost"
                              size="sm"
                              onClick={() => handleCopy(clean.cleaned_text, `${clean.style}_${activeModelResult.target_id}`)}
                              className="h-6 text-xs gap-1 text-muted-foreground hover:text-foreground"
                            >
                              {copiedKey === `${clean.style}_${activeModelResult.target_id}` ? (
                                <>
                                  <Check className="w-3 h-3 text-emerald-500" />
                                  <span>Copied</span>
                                </>
                              ) : (
                                <>
                                  <Copy className="w-3 h-3" />
                                  <span>Copy</span>
                                </>
                              )}
                            </Button>

                            <Button
                              variant="outline"
                              size="sm"
                              onClick={() => handleInject(clean.cleaned_text)}
                              className="h-6 px-2 text-[11px] gap-1"
                            >
                              <span>Inject</span>
                            </Button>
                          </div>
                        </div>

                        {/* Word-level diff display */}
                        <div className="p-3 bg-background border border-border rounded-md text-xs leading-relaxed select-text whitespace-pre-wrap">
                          {clean.diff_spans.map((span, idx) => {
                            if (span.kind === 'added') {
                              return (
                                <span
                                  key={idx}
                                  className="bg-emerald-500/20 text-emerald-700 dark:text-emerald-300 font-medium px-0.5 rounded"
                                  title="Added by cleanup"
                                >
                                  {span.text}
                                </span>
                              );
                            } else if (span.kind === 'removed') {
                              return (
                                <span
                                  key={idx}
                                  className="bg-rose-500/20 text-rose-600 dark:text-rose-400 line-through px-0.5 rounded"
                                  title="Removed by cleanup"
                                >
                                  {span.text}
                                </span>
                              );
                            }
                            return <span key={idx}>{span.text}</span>;
                          })}
                        </div>
                      </div>
                    ))}
                  </div>
                </div>
              )}
            </div>
          )}
        </div>
      )}
    </div>
  );
};
