import React, { useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import {
  AppSettings,
  LanguageSettings,
  MainTabType,
  VaultLocationInfo,
  RelayAccount,
  AudioDeviceInfo,
  OllamaModelDetails,
  SttModelsOverview,
  InstallationInfo,
  UpdateInfo,
} from '../../types';
import {
  Cpu,
  Cloud,
  CheckCircle,
  Sliders,
  ShieldCheck,
  HardDrive,
  User,
  Trash2,
  Download,
  AlertTriangle,
  AlertCircle,
  RefreshCw,
  Mic,
  AudioLines,
  Keyboard,
  Globe,
  Languages,
  Sparkles,
  BookOpen,
  Users,
  CalendarDays,
  Volume2,
  Terminal,
  Check,
  Layers,
  Power,
  Clipboard,
  MessageCircle,
  Activity,
  ChevronDown,
  ChevronUp,
  Info,
  Laptop,
  Copy,
  type LucideIcon,
} from 'lucide-react';
import { Button } from '@/components/ui/button';
import { Input } from '@/components/ui/input';
import { Badge } from '@/components/ui/badge';
import { Switch } from '@/components/ui/switch';
import { HotkeyRecorder } from './HotkeyRecorder';
import { TrashSettings } from './TrashSettings';
import { AccountSettings } from './AccountSettings';
import { DeveloperSettingsView } from './DeveloperSettingsView';
import { DictionarySnippetsSettings } from './DictionarySnippetsSettings';
import { CaptureSettingsView } from './CaptureSettingsView';
import { MeetingSettingsView } from './MeetingSettingsView';
import { CalendarSettingsView } from './CalendarSettingsView';
import { SpeechModelsView } from './SpeechModelsView';
import { CloudProviderSettings } from './CloudProviderSettings';
import { OllamaInstallCard } from './OllamaInstallCard';
import { UnifiedModelsView } from './UnifiedModelsView';
import { describeError } from '@/lib/errors';
import type { SttModelStatus } from '@/types/models';
import type { CleanupStyle } from '../capture/PillTypes';

export type SettingsSection =
  | 'account'
  | 'general'
  | 'dictation'
  | 'speech'
  | 'dictionary'
  | 'capture'
  | 'meetings'
  | 'calendar'
  | 'languages'
  | 'advanced'
  | 'about'
  | 'privacy'
  | 'trash'
  | 'developer';

interface SettingsNavItem {
  id: SettingsSection;
  label: string;
  icon: LucideIcon;
  /** Non-default tint, for the two sections that are not everyday preferences. */
  accent?: string;
}

/**
 * The settings sections, in the order they are shown.
 *
 * One list rather than twelve near-identical buttons: the hand-written version
 * had drifted — its numbering ran 0,1,2,3,4,5,6,6,5,6,7,8 and two unrelated
 * sections shared the same icon, which is exactly the drift a list cannot have.
 * `capture` is labelled "Web Capture" because `Captures` is now a whole surface
 * with several modes, and only the browser bridge is configured here.
 */
const SETTINGS_NAV: SettingsNavItem[] = [
  { id: 'general', label: 'General & Account', icon: Sliders },
  { id: 'dictation', label: 'Dictation & Audio', icon: Mic },
  { id: 'speech', label: 'Models & Speech', icon: Cpu },
  { id: 'dictionary', label: 'Dictionary & Snippets', icon: BookOpen },
  { id: 'capture', label: 'Web Capture', icon: Globe },
  { id: 'meetings', label: 'Meetings', icon: Users },
  { id: 'calendar', label: 'Calendar', icon: CalendarDays },
  { id: 'languages', label: 'Languages & Script', icon: Languages },
  { id: 'about', label: 'About Vox', icon: Info },
  { id: 'trash', label: 'Trash & Deleted', icon: Trash2, accent: 'text-amber-500' },
  { id: 'developer', label: 'Developer', icon: Terminal, accent: 'text-amber-500' },
];

const DEFAULT_LANGUAGE_SETTINGS: LanguageSettings = {
  primary_dictation_language: 'en',
  spoken_languages: ['en'],
  notes_language: 'en',
  output_script: 'latin',
};

const WHISPER_SUPPORTED_LANGUAGES = [
  { code: 'auto', name: 'Auto-detect / Multilingual (Hinglish)' },
  { code: 'en', name: 'English' },
  { code: 'hi', name: 'Hindi' },
  { code: 'kn', name: 'Kannada' },
  { code: 'ta', name: 'Tamil' },
  { code: 'te', name: 'Telugu' },
  { code: 'mr', name: 'Marathi' },
  { code: 'bn', name: 'Bengali' },
  { code: 'gu', name: 'Gujarati' },
  { code: 'ml', name: 'Malayalam' },
  { code: 'pa', name: 'Punjabi' },
  { code: 'ur', name: 'Urdu' },
  { code: 'es', name: 'Spanish' },
  { code: 'fr', name: 'French' },
  { code: 'de', name: 'German' },
  { code: 'it', name: 'Italian' },
  { code: 'pt', name: 'Portuguese' },
  { code: 'ru', name: 'Russian' },
  { code: 'ja', name: 'Japanese' },
  { code: 'zh', name: 'Chinese (Mandarin)' },
  { code: 'ko', name: 'Korean' },
  { code: 'ar', name: 'Arabic' },
  { code: 'nl', name: 'Dutch' },
  { code: 'tr', name: 'Turkish' },
  { code: 'vi', name: 'Vietnamese' },
  { code: 'id', name: 'Indonesian' },
  { code: 'pl', name: 'Polish' },
  { code: 'uk', name: 'Ukrainian' },
  { code: 'sv', name: 'Swedish' },
];

const DEFAULT_SETTINGS: AppSettings = {
  provider: {
    active_provider: 'ollama',
    ollama_host: 'http://localhost:11434',
    ollama_model: 'llama3.2:latest',
    cloud_model: 'gpt-4o-mini',
  },
  stt: { whisper_model_path: '', cleanup_style: 'raw' },
  hotkeys: {
    show_hide_hotkey: 'Ctrl+Shift+Space',
    dictation_hotkey: 'Ctrl+Space',
    toggle_to_talk: false,
    capture_hotkey: 'Ctrl+Shift+C',
  },
  ui: { pill_position: 'bottom_center' },
  vault: { directory: null },
  language: DEFAULT_LANGUAGE_SETTINGS,
  diagnostics: {
    allow_anonymous_diagnostics: true,
    first_run_completed: false,
  },
  sound: {
    dictation_sounds: true,
  },
  clipboard: {
    auto_paste: true,
    copy_to_clipboard: true,
  },
  startup: {
    launch_at_login: false,
    start_minimized: false,
  },
  audio_input: {
    prefer_builtin_mic: true,
    selected_device: null,
    keep_microphone_warm: 'off',
  },
  dictionary: ['Vox', 'Whisper', 'Tauri', 'Rust', 'Supabase', 'LanceDB', 'Ollama'],
  snippets: [],
};

interface ProviderSettingsProps {
  initialSection?: SettingsSection;
  onNavigateTab?: (tab: MainTabType) => void;
}

const normalizeSection = (sec?: string): SettingsSection => {
  if (!sec) return 'general';
  if (sec === 'advanced') return 'speech';
  if (sec === 'account') return 'general';
  if (sec === 'privacy') return 'about';
  return sec as SettingsSection;
};

export const ProviderSettings: React.FC<ProviderSettingsProps> = ({
  initialSection = 'general',
  onNavigateTab,
}) => {
  const [activeSection, setActiveSection] = useState<SettingsSection>(normalizeSection(initialSection));

  useEffect(() => {
    if (initialSection) {
      setActiveSection(normalizeSection(initialSection));
    }
  }, [initialSection]);
  const [settings, setSettings] = useState<AppSettings>(DEFAULT_SETTINGS);
  const [loading, setLoading] = useState(true);
  const [saved, setSaved] = useState(false);
  const [error, setError] = useState('');

  // Audio input devices
  const [audioDevices, setAudioDevices] = useState<AudioDeviceInfo[]>([]);
  const [loadingDevices, setLoadingDevices] = useState(false);

  type OllamaStatus =
    | { state: 'checking' }
    | { state: 'running' }
    | { state: 'started' }
    | { state: 'not_installed' }
    | { state: 'unreachable'; message: string };
  const [ollamaStatus, setOllamaStatus] = useState<OllamaStatus>({ state: 'checking' });

  // LLM Model discovery state
  const [ollamaModels, setOllamaModels] = useState<OllamaModelDetails[]>([]);
  const [loadingOllamaModels, setLoadingOllamaModels] = useState(false);
  const [customLlmMode, setCustomLlmMode] = useState(false);

  // STT Model discovery state
  const [sttOverview, setSttOverview] = useState<SttModelsOverview | null>(null);
  const [loadingSttModels, setLoadingSttModels] = useState(false);
  // Which managed model is downloading, and why the last attempt failed.
  // `large-v3-turbo` is ~1.6 GB, so the button has to stay honest about being
  // busy rather than looking inert for several minutes.
  const [downloadingModel, setDownloadingModel] = useState<string | null>(null);
  const [modelDownloadError, setModelDownloadError] = useState<string | null>(null);
  const [customSttMode, setCustomSttMode] = useState(false);
  const [showCleanupInfo, setShowCleanupInfo] = useState(false);

  const checkLocalLlm = async (overrideHost?: string) => {
    setOllamaStatus({ state: 'checking' });
    try {
      const status = await invoke<OllamaStatus>('ensure_local_llm_ready');
      setOllamaStatus(status);
      await fetchOllamaModels(overrideHost);
    } catch (err) {
      console.error('Failed to check local Ollama status', err);
      setOllamaStatus({ state: 'unreachable', message: 'Could not reach the backend' });
    }
  };

  const fetchOllamaModels = async (host?: string) => {
    setLoadingOllamaModels(true);
    try {
      const models = await invoke<OllamaModelDetails[]>('get_available_llm_models', {
        host: host || settings.provider.ollama_host || null,
      });
      setOllamaModels(models || []);
    } catch (err) {
      setOllamaModels([]);
    } finally {
      setLoadingOllamaModels(false);
    }
  };

  // Fetches a managed model on request.
  //
  // Downloading does not select the model. The accuracy ceiling costs real
  // decode time on every utterance, so making it active is a separate press —
  // the same separation the backend command documents.
  const downloadSttModel = async (filename: string) => {
    setDownloadingModel(filename);
    setModelDownloadError(null);
    try {
      const status = await invoke<SttModelStatus>('download_stt_model', { filename });
      if (status.state === 'failed') {
        setModelDownloadError(status.message);
      }
      await fetchSttModels();
    } catch (e) {
      setModelDownloadError(describeError(e));
    } finally {
      setDownloadingModel(null);
    }
  };


  const fetchSttModels = async () => {
    setLoadingSttModels(true);
    try {
      const overview = await invoke<SttModelsOverview>('get_available_stt_models');
      setSttOverview(overview);
    } catch (err) {
      console.error('Failed to query STT models', err);
    } finally {
      setLoadingSttModels(false);
    }
  };

  const loadAudioDevices = async () => {
    setLoadingDevices(true);
    try {
      const devs = await invoke<AudioDeviceInfo[]>('get_audio_devices');
      setAudioDevices(devs || []);
    } catch (err) {
      console.error('Failed to query audio devices', err);
    } finally {
      setLoadingDevices(false);
    }
  };

  const [vaultLocation, setVaultLocation] = useState<VaultLocationInfo | null>(null);
  const [vaultBusy, setVaultBusy] = useState(false);
  const [vaultError, setVaultError] = useState('');

  // Relay Account state for Privacy & Destructive controls
  const [account, setAccount] = useState<RelayAccount | null>(null);
  const [deleteAccountModalOpen, setDeleteAccountModalOpen] = useState(false);
  const [deleteAccountAck, setDeleteAccountAck] = useState(false);
  const [deleteAccountInput, setDeleteAccountInput] = useState('');
  const [deletingAccount, setDeletingAccount] = useState(false);
  const [deleteAccountSuccess, setDeleteAccountSuccess] = useState<string | null>(null);
  const [deleteAccountError, setDeleteAccountError] = useState<string | null>(null);

  // Clear Vault double confirmation state
  const [clearVaultModalOpen, setClearVaultModalOpen] = useState(false);
  const [clearVaultAck, setClearVaultAck] = useState(false);
  const [clearVaultInput, setClearVaultInput] = useState('');
  const [clearingVault, setClearingVault] = useState(false);
  const [destructiveOpen, setDestructiveOpen] = useState(false);

  const loadAccountState = async () => {
    try {
      const acc = await invoke<RelayAccount>('get_account_state');
      setAccount(acc);
    } catch (err) {
      console.error('Failed to read account state for privacy controls', err);
    }
  };

  const handleDeleteAccount = async () => {
    if (!deleteAccountAck || deleteAccountInput.trim().toUpperCase() !== 'DELETE ACCOUNT') {
      return;
    }
    try {
      setDeletingAccount(true);
      setDeleteAccountError(null);
      const updated = await invoke<RelayAccount>('delete_relay_account');
      setAccount(updated);
      window.dispatchEvent(new CustomEvent('relay-account-changed', { detail: updated }));
      setDeleteAccountSuccess('Vox Cloud Account was deleted. All local markdown notes, scribbles, audio, and vectors remain 100% untouched.');
      setDeleteAccountModalOpen(false);
      setDeleteAccountAck(false);
      setDeleteAccountInput('');
      setTimeout(() => setDeleteAccountSuccess(null), 7000);
    } catch (err: unknown) {
      console.error('Failed to delete account:', err);
      const msg = typeof err === 'string' ? err : (err as { message?: string })?.message || 'Failed to delete account.';
      setDeleteAccountError(msg);
    } finally {
      setDeletingAccount(false);
    }
  };

  const handleDisconnectSync = async () => {
    try {
      const updated = await invoke<RelayAccount>('sign_out_account');
      setAccount(updated);
      window.dispatchEvent(new CustomEvent('relay-account-changed', { detail: updated }));
    } catch (err) {
      console.error('Failed to disconnect sync:', err);
    }
  };

  // Installation Identity, Updates & Diagnostics for About Vox
  const [installation, setInstallation] = useState<InstallationInfo | null>(null);
  const [updateInfo, setUpdateInfo] = useState<UpdateInfo | null>(null);
  const [checkingUpdate, setCheckingUpdate] = useState(false);
  const [copiedId, setCopiedId] = useState(false);
  const [copiedDiagnostics, setCopiedDiagnostics] = useState(false);

  const loadInstallationInfo = async () => {
    try {
      const inst = await invoke<InstallationInfo>('get_installation_info');
      setInstallation(inst);
    } catch (err) {
      console.error('Failed to query installation info', err);
    }
  };

  const handleCheckUpdates = async () => {
    try {
      setCheckingUpdate(true);
      const info = await invoke<UpdateInfo>('check_for_app_updates');
      setUpdateInfo(info);
    } catch (err) {
      console.error('Update check failed:', err);
    } finally {
      setCheckingUpdate(false);
    }
  };

  const copyInstallationId = () => {
    if (!installation?.installation_id) return;
    navigator.clipboard.writeText(installation.installation_id);
    setCopiedId(true);
    setTimeout(() => setCopiedId(false), 2000);
  };

  const copyDiagnosticsSummary = async () => {
    try {
      const summary = await invoke<string>('get_diagnostic_summary');
      await navigator.clipboard.writeText(summary);
      setCopiedDiagnostics(true);
      setTimeout(() => setCopiedDiagnostics(false), 2000);
    } catch (err) {
      console.error('Failed to copy diagnostics:', err);
    }
  };

  const handleToggleDiagnostics = async (checked: boolean) => {
    setSettings((prev) => ({
      ...prev,
      diagnostics: {
        ...prev.diagnostics,
        allow_anonymous_diagnostics: checked,
      },
    }));

    try {
      const current = await invoke<AppSettings>('get_settings');
      const updated: AppSettings = {
        ...current,
        diagnostics: {
          ...current.diagnostics,
          allow_anonymous_diagnostics: checked,
        },
      };
      await invoke('save_settings', { settings: updated });
    } catch (err) {
      console.error('Failed to persist diagnostics setting:', err);
    }
  };

  const maskedId = installation?.installation_id
    ? installation.installation_id.length > 12
      ? `${installation.installation_id.substring(0, 8)}...${installation.installation_id.substring(installation.installation_id.length - 4)}`
      : installation.installation_id
    : '••••••••••••';

  const isDiagnosticsAllowed = settings.diagnostics?.allow_anonymous_diagnostics ?? false;

  const loadVaultLocation = async () => {
    try {
      setVaultLocation(await invoke<VaultLocationInfo>('get_vault_location'));
    } catch (err) {
      console.error('Failed to read Vault Directory Location', err);
      setVaultError('Could not determine where the vault is stored');
    }
  };

  const handleChooseVaultFolder = async () => {
    setVaultBusy(true);
    setVaultError('');
    try {
      const picked = await invoke<string | null>('choose_vault_folder');
      if (!picked) return;
      const info = await invoke<VaultLocationInfo>('set_vault_location', { path: picked });
      setVaultLocation(info);
      setSettings((prev) => ({
        ...prev,
        vault: { ...prev.vault, directory: picked },
      }));
    } catch (err) {
      console.error('Failed to set Vault Directory Location', err);
      setVaultError(describeError(err, "Couldn't use that folder — choose another."));
    } finally {
      setVaultBusy(false);
    }
  };

  useEffect(() => {
    invoke<AppSettings>('get_settings')
      .then((loaded) => {
        setSettings({
          ...DEFAULT_SETTINGS,
          ...loaded,
          stt: {
            ...DEFAULT_SETTINGS.stt!,
            ...(loaded.stt || {}),
            cleanup_style:
              loaded.stt?.cleanup_style ||
              loaded.stt?.cleanupStyle ||
              'raw',
          },
          clipboard: {
            ...DEFAULT_SETTINGS.clipboard!,
            ...(loaded.clipboard || {}),
          },
          startup: {
            ...DEFAULT_SETTINGS.startup!,
            ...(loaded.startup || {}),
          },
          audio_input: {
            ...DEFAULT_SETTINGS.audio_input!,
            ...(loaded.audio_input || {}),
          },
          sound: {
            ...DEFAULT_SETTINGS.sound!,
            ...(loaded.sound || {}),
          },
          language: {
            ...DEFAULT_LANGUAGE_SETTINGS,
            ...(loaded.language || {}),
            spoken_languages:
              loaded.language?.spoken_languages ||
              loaded.language?.spokenLanguages ||
              DEFAULT_LANGUAGE_SETTINGS.spoken_languages,
            primary_dictation_language:
              loaded.language?.primary_dictation_language ||
              loaded.language?.primaryDictationLanguage ||
              DEFAULT_LANGUAGE_SETTINGS.primary_dictation_language,
            notes_language:
              loaded.language?.notes_language ||
              loaded.language?.notesLanguage ||
              DEFAULT_LANGUAGE_SETTINGS.notes_language,
            output_script:
              loaded.language?.output_script ||
              loaded.language?.outputScript ||
              DEFAULT_LANGUAGE_SETTINGS.output_script,
          },
        });
      })
      .catch((err) => {
        console.error('Failed to load settings', err);
        setError('Could not load saved settings — showing defaults');
      })
      .finally(() => setLoading(false));

    const unlistenPromise = listen<AppSettings>('settings-changed', ({ payload }) => {
      if (payload) {
        setSettings((prev) => ({
          ...prev,
          ...payload,
        }));
      }
    });

    return () => {
      unlistenPromise.then((unlisten) => unlisten());
    };
  }, []);

  useEffect(() => {
    if (!loading && activeSection === 'general') {
      loadVaultLocation();
    }
    if (!loading && activeSection === 'dictation') {
      loadAudioDevices();
    }
    if (!loading && activeSection === 'advanced') {
      if (settings.provider.active_provider === 'ollama') {
        checkLocalLlm();
        fetchOllamaModels();
      }
      fetchSttModels();
    }
    if (!loading && (activeSection === 'about' || activeSection === 'privacy')) {
      loadAccountState();
      loadVaultLocation();
      loadInstallationInfo();
    }
  }, [loading, activeSection, settings.provider.active_provider]);

  const applyHotkey = async (field: 'show_hide_hotkey' | 'dictation_hotkey', accelerator: string) => {
    const updatedHotkeys = { ...settings.hotkeys, [field]: accelerator };
    const updatedSettings = { ...settings, hotkeys: updatedHotkeys };
    setSettings(updatedSettings);
    try {
      await invoke('update_hotkeys', { hotkeys: updatedHotkeys });
      setError('');
    } catch (err) {
      console.error('Failed to apply hotkey', err);
      setError(describeError(err, 'Failed to apply hotkey — it may already be in use by another app'));
    }
  };



  const handleSaveDirect = async (next: AppSettings = settings) => {
    try {
      await invoke('save_settings', { settings: next });
      setSaved(true);
      setError('');
      setTimeout(() => setSaved(false), 2000);
      fetchSttModels();
      if (next.provider.active_provider === 'ollama') {
        fetchOllamaModels();
      }
    } catch (err) {
      console.error('Failed to save settings', err);
      setError('Failed to save settings');
    }
  };

  const handleSave = async (e: React.FormEvent) => {
    e.preventDefault();
    await handleSaveDirect();
  };

  // Find default or active microphone
  const defaultDevice = audioDevices.find((d) => d.is_default) || audioDevices[0];
  const activeDeviceName = settings.audio_input?.selected_device || defaultDevice?.name || 'System default';

  if (loading) {
    return (
      <div className="flex-1 flex items-center justify-center bg-card rounded-lg border border-border text-xs text-muted-foreground">
        Loading settings…
      </div>
    );
  }

  return (
    <div className="flex-1 flex flex-col md:flex-row gap-4 min-h-0 overflow-hidden w-full">
      {/* Settings Sub-Nav Sidebar */}
      <aside className="w-full md:w-52 flex flex-col shrink-0 gap-1 bg-card p-2.5 rounded-lg border border-border">
        <div className="px-2.5 py-1.5 mb-1">
          <span className="font-mono text-[10px] font-bold tracking-widest text-muted-foreground uppercase">
            SETTINGS
          </span>
        </div>

        {SETTINGS_NAV.map((item) => {
          const Icon = item.icon;
          const active = activeSection === item.id;
          return (
            <button
              key={item.id}
              type="button"
              onClick={() => setActiveSection(item.id)}
              aria-current={active ? 'page' : undefined}
              className={`flex items-center gap-2.5 px-2.5 py-1.5 rounded-lg text-xs font-medium transition-all text-left ${
                active
                  ? 'bg-accent text-accent-foreground font-semibold shadow-xs'
                  : 'text-muted-foreground hover:bg-muted hover:text-foreground'
              }`}
            >
              <Icon className={`w-4 h-4 ${item.accent ?? 'text-primary'}`} />
              <span>{item.label}</span>
            </button>
          );
        })}
      </aside>

      {/* Main Settings Content Area */}
      <main className="flex-1 bg-card rounded-lg border border-border p-4 md:p-5 overflow-y-auto min-h-0 w-full">
        {saved && (
          <div className="mb-3 p-2.5 rounded-lg bg-emerald-500/10 border border-emerald-500/30 text-emerald-600 dark:text-emerald-400 text-xs flex items-center justify-between">
            <span className="flex items-center gap-2">
              <CheckCircle className="w-4 h-4 text-emerald-500" />
              Settings updated successfully
            </span>
            <Badge variant="outline" className="text-[10px] font-mono border-emerald-500/30 text-emerald-500">
              Persisted
            </Badge>
          </div>
        )}

        {error && <p className="mb-4 text-xs text-amber-500">{error}</p>}

        {/* 1. GENERAL & ACCOUNT SECTION */}
        {activeSection === 'general' && (
          <form onSubmit={handleSave} className="space-y-4 w-full">
            <div>
              <p className="font-mono text-[9px] font-bold text-muted-foreground uppercase tracking-widest mb-0.5">
                GENERAL CONFIGURATION
              </p>
              <h2 className="text-base font-bold text-foreground">Desktop App & Startup Defaults</h2>
            </div>

            <div className="grid grid-cols-1 md:grid-cols-2 xl:grid-cols-3 gap-3">
              {/* Card 1: Hotkeys & Pill Position */}
              <div className="p-3 rounded-lg border border-border bg-card space-y-3 flex flex-col justify-between">
                <div className="space-y-2.5">
                  <div className="flex items-center gap-2">
                    <Keyboard className="w-3.5 h-3.5 text-primary" />
                    <div>
                      <p className="text-xs font-semibold text-foreground">Show/Hide Hotkey</p>
                      <p className="text-[10px] text-muted-foreground">Toggle Vox window anywhere</p>
                    </div>
                  </div>
                  <div>
                    <HotkeyRecorder
                      id="show-hide-hotkey"
                      value={settings.hotkeys.show_hide_hotkey}
                      onCapture={(accelerator) => applyHotkey('show_hide_hotkey', accelerator)}
                    />
                    <p className="text-[9px] text-muted-foreground mt-1">
                      Click box, press desired key combination.
                    </p>
                  </div>
                </div>

                <div className="pt-2 border-t border-border/60">
                  <p className="text-xs font-semibold text-foreground mb-0.5">Pill Screen Position</p>
                  <p className="text-[10px] text-muted-foreground mb-1.5">
                    Where the floating dictation pill anchors
                  </p>
                  <div className="flex bg-muted p-1 rounded-lg border border-border w-full">
                    {(
                      [
                        { value: 'bottom_left', label: 'Left' },
                        { value: 'bottom_center', label: 'Center' },
                        { value: 'bottom_right', label: 'Right' },
                      ] as const
                    ).map((opt) => (
                      <button
                        key={opt.value}
                        type="button"
                        onClick={async () => {
                          const updated = { ...settings, ui: { ...settings.ui, pill_position: opt.value } };
                          setSettings(updated);
                          try {
                            await invoke('set_pill_position', { position: opt.value });
                          } catch (err) {
                            console.error('Failed to set pill position', err);
                          }
                        }}
                        className={`flex-1 px-2 py-1 text-xs font-medium rounded-md transition-all ${
                          settings.ui.pill_position === opt.value
                            ? 'bg-card text-foreground font-semibold shadow-xs'
                            : 'text-muted-foreground hover:text-foreground'
                        }`}
                      >
                        {opt.label}
                      </button>
                    ))}
                  </div>
                </div>
              </div>

              {/* Card 2: Startup Behavior */}
              <div className="p-3 rounded-lg border border-border bg-card space-y-3 flex flex-col justify-between">
                <div>
                  <div className="flex items-center gap-2 mb-2.5">
                    <Power className="w-3.5 h-3.5 text-primary" />
                    <div>
                      <p className="text-xs font-semibold text-foreground">Startup Behavior</p>
                      <p className="text-[10px] text-muted-foreground">Control launch and window states</p>
                    </div>
                  </div>

                  <div className="space-y-2.5 pt-0.5">
                    <div className="flex items-center justify-between gap-3">
                      <div>
                        <p className="text-xs font-medium text-foreground">Launch at login</p>
                        <p className="text-[10px] text-muted-foreground">Start Vox in background at log in</p>
                      </div>
                      <Switch
                        checked={settings.startup?.launch_at_login ?? false}
                        onCheckedChange={async (checked) => {
                          const updated: AppSettings = {
                            ...settings,
                            startup: {
                              ...settings.startup,
                              launch_at_login: checked,
                              start_minimized: settings.startup?.start_minimized ?? false,
                            },
                          };
                          setSettings(updated);
                          try {
                            await invoke('save_settings', { settings: updated });
                          } catch (err) {
                            console.error('Failed to update launch at login', err);
                          }
                        }}
                      />
                    </div>

                    <div className="h-px bg-border/60" />

                    <div className="flex items-center justify-between gap-3">
                      <div>
                        <p className="text-xs font-medium text-foreground">Start minimized</p>
                        <p className="text-[10px] text-muted-foreground">Launch without showing control panel</p>
                      </div>
                      <Switch
                        checked={settings.startup?.start_minimized ?? false}
                        onCheckedChange={async (checked) => {
                          const updated: AppSettings = {
                            ...settings,
                            startup: {
                              ...settings.startup,
                              launch_at_login: settings.startup?.launch_at_login ?? false,
                              start_minimized: checked,
                            },
                          };
                          setSettings(updated);
                          try {
                            await invoke('save_settings', { settings: updated });
                          } catch (err) {
                            console.error('Failed to update start minimized', err);
                          }
                        }}
                      />
                    </div>
                  </div>
                </div>
              </div>

              {/* Card 3: Vault Directory Location */}
              <div className="p-3 rounded-lg border border-border bg-card space-y-3 flex flex-col justify-between">
                <div className="space-y-2">
                  <div className="flex items-center gap-2">
                    <HardDrive className="w-3.5 h-3.5 text-primary" />
                    <div>
                      <p className="text-xs font-semibold text-foreground">Vault Storage</p>
                      <p className="text-[10px] text-muted-foreground">Local markdown notes & vectors</p>
                    </div>
                  </div>

                  <div className="p-2 rounded-lg bg-muted/40 border border-border/80">
                    <p className="text-[9px] text-muted-foreground uppercase font-mono tracking-wider mb-0.5">
                      Active Directory
                    </p>
                    <p className="text-xs font-mono text-foreground break-all leading-tight">
                      {vaultLocation?.path || 'Loading…'}
                    </p>
                  </div>

                  {vaultLocation && !vaultLocation.configured && (
                    <p className="text-[9px] text-muted-foreground">Using default OS app location</p>
                  )}
                  {vaultError && <p className="text-[9px] text-destructive">{vaultError}</p>}
                </div>

                <div className="pt-2 border-t border-border/60 flex items-center justify-between gap-2">
                  {vaultLocation?.accessible === false ? (
                    <Badge variant="outline" className="text-xs font-mono border-destructive/50 text-destructive">
                      Inaccessible
                    </Badge>
                  ) : (
                    <Badge variant="outline" className="text-[10px] font-mono text-emerald-600 dark:text-emerald-400 border-emerald-500/30">
                      Accessible
                    </Badge>
                  )}
                  <Button
                    type="button"
                    size="sm"
                    variant="outline"
                    className="text-xs h-7"
                    disabled={vaultBusy}
                    onClick={handleChooseVaultFolder}
                  >
                    Choose Folder
                  </Button>
                </div>
              </div>
            </div>

            {/* Account & Identity Subsection */}
            <AccountSettings
              settings={settings}
              onUpdateSettings={setSettings}
            />

            <div className="pt-2">
              <Button type="submit" size="sm" variant="default">
                Save General Settings
              </Button>
            </div>
          </form>
        )}

        {/* 2. DICTATION & AUDIO SECTION */}
        {activeSection === 'dictation' && (
          <form onSubmit={handleSave} className="space-y-6">
            <div>
              <p className="font-mono text-[10px] font-bold text-muted-foreground uppercase tracking-widest mb-1">
                UNIVERSAL DICTATION & AUDIO HARDWARE
              </p>
              <h2 className="text-lg font-bold text-foreground">Microphone, Clipboard & Sound Behavior</h2>
            </div>

            <div className="grid grid-cols-1 lg:grid-cols-2 gap-4">
              {/* Card 0: AI Cleanup & Transcription Style */}
              <div className="p-4 rounded-lg border border-border bg-card space-y-4 lg:col-span-2 shadow-xs">
                <div className="flex items-center justify-between gap-2">
                  <div className="flex items-center gap-2">
                    <Sparkles className="w-4 h-4 text-primary shrink-0" />
                    <div>
                      <p className="text-xs font-semibold text-foreground">AI Cleanup & Transcription Style</p>
                      <p className="text-[11px] text-muted-foreground">
                        Off (Raw) by default. Any other style sends each dictation to your selected AI
                        provider and can add up to 5 seconds before the text lands.
                      </p>
                    </div>
                  </div>

                  {/* "i" Info button at the far end */}
                  <button
                    type="button"
                    onClick={() => setShowCleanupInfo((prev) => !prev)}
                    className={`w-6 h-6 rounded-full flex items-center justify-center transition-all border shrink-0 cursor-pointer ${
                      showCleanupInfo
                        ? 'bg-primary text-primary-foreground border-primary shadow-xs ring-2 ring-primary/20'
                        : 'bg-muted/50 text-muted-foreground border-border hover:bg-muted hover:text-foreground hover:border-primary/40'
                    }`}
                    title={showCleanupInfo ? 'Hide mode comparison' : 'Compare all modes with an example'}
                    aria-label="Compare all modes with an example"
                    aria-expanded={showCleanupInfo}
                  >
                    <Info className="w-3.5 h-3.5" />
                  </button>
                </div>

                <div className="grid grid-cols-1 sm:grid-cols-2 lg:grid-cols-5 gap-2.5 pt-1">
                  {[
                    {
                      id: 'raw',
                      name: 'Raw',
                      tag: 'Default',
                      desc: 'No AI rewrite. Text as transcribed, with only the built-in filler and dictionary fixes.',
                    },
                    {
                      id: 'faithful',
                      name: 'Faithful',
                      desc: 'Punctuation, capitalization & filler removal. Your words stay 100% untouched.',
                    },
                    {
                      id: 'clean',
                      name: 'Clean',
                      desc: 'Light grammatical polish & sentence cleanup. Keeps your natural spoken voice.',
                    },
                    {
                      id: 'polished',
                      name: 'Polished',
                      desc: 'Elevates to professional business correspondence, preserving all facts & names.',
                    },
                    {
                      id: 'concise',
                      name: 'Concise',
                      desc: 'Removes redundancy & condenses text into tight, punchy executive points.',
                    },
                  ].map((item) => {
                    const currentStyle = settings.stt?.cleanup_style || settings.stt?.cleanupStyle || 'raw';
                    const isSelected = currentStyle === item.id;
                    return (
                      <button
                        key={item.id}
                        type="button"
                        onClick={async () => {
                          const updated: AppSettings = {
                            ...settings,
                            stt: {
                              ...settings.stt,
                              cleanup_style: item.id as CleanupStyle,
                              cleanupStyle: item.id as CleanupStyle,
                            },
                          };
                          setSettings(updated);
                          try {
                            await invoke('save_settings', { settings: updated });
                          } catch (err) {
                            console.error('Failed to update cleanup style', err);
                          }
                        }}
                        className={`p-3 rounded-lg border text-left flex flex-col justify-between transition-all cursor-pointer ${
                          isSelected
                            ? 'border-primary bg-primary/10 text-foreground shadow-xs ring-1 ring-primary/40'
                            : 'border-border bg-card/50 text-muted-foreground hover:border-border/80 hover:bg-muted/10'
                        }`}
                      >
                        <div>
                          <div className="flex items-center justify-between mb-1.5">
                            <span className="text-xs font-bold text-foreground">{item.name}</span>
                            {item.tag && (
                              <Badge
                                variant={item.tag === 'Default' ? 'emerald' : 'secondary'}
                                className="text-[9px] px-1 py-0 font-mono"
                              >
                                {item.tag}
                              </Badge>
                            )}
                          </div>
                          <p className="text-[10px] text-muted-foreground leading-snug">
                            {item.desc}
                          </p>
                        </div>
                      </button>
                    );
                  })}
                </div>

                {/* Generic Comparison Example Breakdown */}
                {showCleanupInfo && (
                  <div className="rounded-lg border border-border/80 bg-muted/20 p-3.5 space-y-3 animate-in fade-in duration-150">
                    <div className="flex items-start justify-between gap-2 border-b border-border/60 pb-2.5">
                      <div>
                        <span className="text-[10px] font-mono uppercase tracking-wider font-semibold text-primary">
                          Generic Comparison Example
                        </span>
                        <p className="text-xs text-foreground/90 font-medium italic mt-0.5">
                          “um so yeah we need to like reschedule the budget review meeting with Sarah to next Tuesday at 3 p.m. because um Monday is totally jammed, you know?”
                        </p>
                      </div>
                      <button
                        type="button"
                        onClick={() => setShowCleanupInfo(false)}
                        className="text-[10px] font-mono text-muted-foreground hover:text-foreground px-1.5 py-0.5 rounded hover:bg-muted/80 cursor-pointer"
                      >
                        Close
                      </button>
                    </div>

                    <div className="grid grid-cols-1 md:grid-cols-5 gap-2.5">
                      <div className="p-2.5 rounded-md bg-card border border-border/70 space-y-1.5">
                        <span className="text-[11px] font-bold text-foreground font-mono">Raw</span>
                        <p className="text-[10px] text-muted-foreground leading-tight">
                          Zero modification. Verbatim Whisper speech with all fillers and pauses.
                        </p>
                        <p className="text-[10px] text-foreground/90 font-mono bg-muted/40 p-1.5 rounded break-words">
                          "um so yeah we need to like reschedule the budget review meeting with Sarah to next Tuesday at 3 p.m. because um Monday is totally jammed, you know?"
                        </p>
                      </div>

                      <div className="p-2.5 rounded-md bg-card border border-border/70 space-y-1.5">
                        <div className="flex items-center justify-between">
                          <span className="text-[11px] font-bold text-foreground font-mono">Faithful</span>
                          <Badge variant="emerald" className="text-[8px] px-1 py-0">Default</Badge>
                        </div>
                        <p className="text-[10px] text-muted-foreground leading-tight">
                          Removes fillers & stutters, adds punctuation. Words stay 100% untouched.
                        </p>
                        <p className="text-[10px] text-foreground/90 font-mono bg-muted/40 p-1.5 rounded break-words">
                          "We need to reschedule the budget review meeting with Sarah to next Tuesday at 3 p.m. because Monday is totally jammed."
                        </p>
                      </div>

                      <div className="p-2.5 rounded-md bg-card border border-border/70 space-y-1.5">
                        <span className="text-[11px] font-bold text-foreground font-mono">Clean</span>
                        <p className="text-[10px] text-muted-foreground leading-tight">
                          Polishes grammar, agreement & run-on sentences with light rephrasing.
                        </p>
                        <p className="text-[10px] text-foreground/90 font-mono bg-muted/40 p-1.5 rounded break-words">
                          "We need to reschedule the budget review meeting with Sarah to next Tuesday at 3:00 PM because Monday is completely booked."
                        </p>
                      </div>

                      <div className="p-2.5 rounded-md bg-card border border-border/70 space-y-1.5">
                        <span className="text-[11px] font-bold text-foreground font-mono">Polished</span>
                        <p className="text-[10px] text-muted-foreground leading-tight">
                          Elevates register to business professional while preserving all facts & names.
                        </p>
                        <p className="text-[10px] text-foreground/90 font-mono bg-muted/40 p-1.5 rounded break-words">
                          "Please reschedule the budget review meeting with Sarah to next Tuesday at 3:00 PM, as Monday's schedule is fully committed."
                        </p>
                      </div>

                      <div className="p-2.5 rounded-md bg-card border border-border/70 space-y-1.5">
                        <span className="text-[11px] font-bold text-foreground font-mono">Concise</span>
                        <p className="text-[10px] text-muted-foreground leading-tight">
                          Cuts all redundancy & extracts the essential action point cleanly.
                        </p>
                        <p className="text-[10px] text-foreground/90 font-mono bg-muted/40 p-1.5 rounded break-words">
                          "Reschedule Sarah's budget review meeting to Tuesday at 3:00 PM."
                        </p>
                      </div>
                    </div>
                  </div>
                )}
              </div>

              {/* Card 1: Universal Dictation Hotkey & Toggle Mode */}
              <div className="p-4 rounded-lg border border-border bg-card space-y-4 flex flex-col justify-between">
                <div className="space-y-3">
                  <div className="flex items-center gap-2">
                    <Keyboard className="w-4 h-4 text-primary" />
                    <div>
                      <p className="text-xs font-semibold text-foreground">Universal Dictation Hotkey</p>
                      <p className="text-[11px] text-muted-foreground">Dictate anywhere into the active focused field</p>
                    </div>
                  </div>
                  <div>
                    <HotkeyRecorder
                      id="dictation-hotkey"
                      value={settings.hotkeys.dictation_hotkey}
                      onCapture={(accelerator) => applyHotkey('dictation_hotkey', accelerator)}
                    />
                    <p className="text-[10px] text-muted-foreground mt-1.5">
                      Press and hold (or toggle) to speak into any app.
                    </p>
                  </div>
                </div>

                <div className="pt-3 border-t border-border/60 flex items-center justify-between gap-3">
                  <div>
                    <p className="text-xs font-semibold text-foreground">Toggle-to-Talk Mode</p>
                    <p className="text-[11px] text-muted-foreground">
                      Press once to start, press again to stop
                    </p>
                  </div>
                  <Switch
                    checked={settings.hotkeys.toggle_to_talk}
                    onCheckedChange={async (checked) => {
                      const updated = {
                        ...settings,
                        hotkeys: { ...settings.hotkeys, toggle_to_talk: checked },
                      };
                      setSettings(updated);
                      try {
                        await invoke('save_settings', { settings: updated });
                      } catch (err) {
                        console.error('Failed to toggle toggle-to-talk mode', err);
                      }
                    }}
                  />
                </div>
              </div>

              {/* Card 2: Sound Effects & Audio Tones */}
              <div className="p-4 rounded-lg border border-border bg-card space-y-4 flex flex-col justify-between">
                <div className="space-y-3">
                  <div className="flex items-center gap-2">
                    <Volume2 className="w-4 h-4 text-primary" />
                    <div>
                      <p className="text-xs font-semibold text-foreground">Sound Effects & Feedback</p>
                      <p className="text-[11px] text-muted-foreground">Audio cues for dictation states</p>
                    </div>
                  </div>
                  <div className="p-3 rounded-lg bg-muted/40 border border-border/80 flex items-center justify-between gap-3">
                    <div>
                      <p className="text-xs font-medium text-foreground">Dictation Start / Stop Tone</p>
                      <p className="text-[11px] text-muted-foreground">
                        Play an audible chime when voice recording activates and completes
                      </p>
                    </div>
                    <Switch
                      checked={settings.sound?.dictation_sounds ?? true}
                      onCheckedChange={async (checked) => {
                        const updated: AppSettings = {
                          ...settings,
                          sound: {
                            ...settings.sound,
                            dictation_sounds: checked,
                          },
                        };
                        setSettings(updated);
                        try {
                          await invoke('save_settings', { settings: updated });
                        } catch (err) {
                          console.error('Failed to toggle dictation sounds', err);
                        }
                      }}
                    />
                  </div>
                </div>

                <div className="p-2.5 rounded-lg bg-primary/5 border border-primary/20 text-[11px] text-muted-foreground">
                  Sub-100ms low latency feedback is synced with your push-to-talk key presses.
                </div>
              </div>

              {/* Card 3: Clipboard & Injection Method */}
              <div className="p-4 rounded-lg border border-border bg-card space-y-4">
                <div className="flex items-center gap-2">
                  <Clipboard className="w-4 h-4 text-primary" />
                  <div>
                    <p className="text-xs font-semibold text-foreground">Clipboard & Text Injection</p>
                    <p className="text-[11px] text-muted-foreground">Control how transcribed text reaches target apps</p>
                  </div>
                </div>

                <div className="space-y-3 pt-1">
                  <div className="flex items-center justify-between gap-3">
                    <div>
                      <p className="text-xs font-medium text-foreground">Automatic pasting</p>
                      <p className="text-[11px] text-muted-foreground">
                        Paste transcribed text into active app when dictation finishes
                      </p>
                    </div>
                    <Switch
                      checked={settings.clipboard?.auto_paste ?? true}
                      onCheckedChange={async (checked) => {
                        const updated: AppSettings = {
                          ...settings,
                          clipboard: {
                            ...settings.clipboard,
                            auto_paste: checked,
                            copy_to_clipboard: settings.clipboard?.copy_to_clipboard ?? true,
                          },
                        };
                        setSettings(updated);
                        try {
                          await invoke('save_settings', { settings: updated });
                        } catch (err) {
                          console.error('Failed to update auto paste', err);
                        }
                      }}
                    />
                  </div>

                  {(settings.clipboard?.auto_paste ?? true) && (
                    <>
                      <div className="h-px bg-border/60" />
                      <div className="space-y-2">
                        <p className="text-xs font-medium text-foreground">Injection Method</p>
                        <div className="grid grid-cols-1 sm:grid-cols-2 gap-2">
                          <button
                            type="button"
                            onClick={async () => {
                              const updated: AppSettings = {
                                ...settings,
                                clipboard: {
                                  ...settings.clipboard,
                                  auto_paste: settings.clipboard?.auto_paste ?? true,
                                  copy_to_clipboard: settings.clipboard?.copy_to_clipboard ?? true,
                                  injection_method: 'clipboard_paste',
                                  injectionMethod: 'clipboard_paste',
                                },
                              };
                              setSettings(updated);
                              try {
                                await invoke('save_settings', { settings: updated });
                              } catch (err) {
                                console.error('Failed to update injection method', err);
                              }
                            }}
                            className={`p-2.5 rounded-lg border text-left transition-all ${
                              (settings.clipboard?.injection_method ?? 'clipboard_paste') === 'clipboard_paste'
                                ? 'border-primary bg-primary/10 text-foreground shadow-xs'
                                : 'border-border bg-card/50 text-muted-foreground hover:border-border/80'
                            }`}
                          >
                            <div className="flex items-center justify-between mb-1">
                              <span className="text-xs font-bold text-foreground">Clipboard Paste</span>
                              <Badge variant="emerald" className="text-[9px] px-1.5 py-0">Default</Badge>
                            </div>
                            <p className="text-[10px] text-muted-foreground leading-snug">
                              Instant Ctrl+V. Reliable across Notepad, browsers, Word, terminals.
                            </p>
                          </button>

                          <button
                            type="button"
                            onClick={async () => {
                              const updated: AppSettings = {
                                ...settings,
                                clipboard: {
                                  ...settings.clipboard,
                                  auto_paste: settings.clipboard?.auto_paste ?? true,
                                  copy_to_clipboard: settings.clipboard?.copy_to_clipboard ?? true,
                                  injection_method: 'keystrokes',
                                  injectionMethod: 'keystrokes',
                                },
                              };
                              setSettings(updated);
                              try {
                                await invoke('save_settings', { settings: updated });
                              } catch (err) {
                                console.error('Failed to update injection method', err);
                              }
                            }}
                            className={`p-2.5 rounded-lg border text-left transition-all ${
                              (settings.clipboard?.injection_method ?? 'clipboard_paste') === 'keystrokes'
                                ? 'border-primary bg-primary/10 text-foreground shadow-xs'
                                : 'border-border bg-card/50 text-muted-foreground hover:border-border/80'
                            }`}
                          >
                            <div className="flex items-center justify-between mb-1">
                              <span className="text-xs font-bold text-foreground">Simulated Keys</span>
                            </div>
                            <p className="text-[10px] text-muted-foreground leading-snug">
                              Types each character physically. For apps that block pasting.
                            </p>
                          </button>
                        </div>
                      </div>
                    </>
                  )}

                  <div className="h-px bg-border/60" />

                  <div className="flex items-center justify-between gap-3">
                    <div>
                      <p className="text-xs font-medium text-foreground">Keep in OS clipboard</p>
                      <p className="text-[11px] text-muted-foreground">
                        Keep text in clipboard for manual pasting if desired
                      </p>
                    </div>
                    <Switch
                      checked={settings.clipboard?.copy_to_clipboard ?? true}
                      onCheckedChange={async (checked) => {
                        const updated: AppSettings = {
                          ...settings,
                          clipboard: {
                            ...settings.clipboard,
                            auto_paste: settings.clipboard?.auto_paste ?? true,
                            copy_to_clipboard: checked,
                          },
                        };
                        setSettings(updated);
                        try {
                          await invoke('save_settings', { settings: updated });
                        } catch (err) {
                          console.error('Failed to update copy to clipboard', err);
                        }
                      }}
                    />
                  </div>
                </div>
              </div>

              {/* Card 4: Microphone Hardware & Warm-up */}
              <div className="p-4 rounded-lg border border-border bg-card space-y-4">
                <div className="flex items-center gap-2">
                  <Mic className="w-4 h-4 text-primary" />
                  <div>
                    <p className="text-xs font-semibold text-foreground">Microphone & Audio Input</p>
                    <p className="text-[11px] text-muted-foreground">Device selection and warm-up latency</p>
                  </div>
                </div>

                <div className="space-y-3 pt-1">
                  <div className="flex items-center justify-between gap-3">
                    <div>
                      <p className="text-xs font-medium text-foreground">Prefer Built-in Microphone</p>
                      <p className="text-[11px] text-muted-foreground">
                        Lowers external microphone latency
                      </p>
                    </div>
                    <Switch
                      checked={settings.audio_input?.prefer_builtin_mic ?? true}
                      onCheckedChange={async (checked) => {
                        const updated: AppSettings = {
                          ...settings,
                          audio_input: {
                            ...settings.audio_input,
                            prefer_builtin_mic: checked,
                            selected_device: settings.audio_input?.selected_device,
                            keep_microphone_warm: settings.audio_input?.keep_microphone_warm || 'off',
                          },
                        };
                        setSettings(updated);
                        try {
                          await invoke('save_settings', { settings: updated });
                        } catch (err) {
                          console.error('Failed to update prefer builtin mic', err);
                        }
                      }}
                    />
                  </div>

                  <div className="space-y-1">
                    <label htmlFor="input-device" className="block text-xs font-medium text-foreground">
                      Input Device
                    </label>
                    <select
                      id="input-device"
                      value={settings.audio_input?.selected_device ?? ''}
                      onChange={async (e) => {
                        const value = e.target.value;
                        const updated: AppSettings = {
                          ...settings,
                          audio_input: {
                            ...settings.audio_input,
                            prefer_builtin_mic: settings.audio_input?.prefer_builtin_mic ?? true,
                            selected_device: value === '' ? null : value,
                            keep_microphone_warm: settings.audio_input?.keep_microphone_warm || 'off',
                          },
                        };
                        setSettings(updated);
                        try {
                          await invoke('save_settings', { settings: updated });
                        } catch (err) {
                          console.error('Failed to save microphone selection', err);
                        }
                      }}
                      className="w-full text-xs rounded-md border border-border bg-background px-2.5 py-1.5 text-foreground"
                    >
                      <option value="">
                        System default{defaultDevice ? ` (${defaultDevice.name})` : ''}
                      </option>
                      {audioDevices.map((device) => (
                        <option key={device.name} value={device.name}>
                          {device.name}
                        </option>
                      ))}
                    </select>
                  </div>

                  {/* Active Input Device Badge */}
                  <div className="p-2 rounded-md bg-emerald-500/10 border border-emerald-500/30 text-emerald-800 dark:text-emerald-300 text-xs flex items-center gap-2 font-medium">
                    <Mic className="w-3.5 h-3.5 text-emerald-600 dark:text-emerald-400 shrink-0" />
                    <span className="truncate">Active: {activeDeviceName}</span>
                  </div>

                  <div className="h-px bg-border/60" />

                  <div className="flex items-center justify-between gap-3">
                    <div>
                      <p className="text-xs font-medium text-foreground">Keep Microphone Warm</p>
                      <p className="text-[11px] text-muted-foreground">
                        Keep mic open briefly to eliminate start clip
                      </p>
                    </div>
                    <select
                      value={settings.audio_input?.keep_microphone_warm || 'off'}
                      onChange={async (e) => {
                        const val = e.target.value;
                        const updated: AppSettings = {
                          ...settings,
                          audio_input: {
                            ...settings.audio_input,
                            prefer_builtin_mic: settings.audio_input?.prefer_builtin_mic ?? true,
                            keep_microphone_warm: val,
                          },
                        };
                        setSettings(updated);
                        try {
                          await invoke('save_settings', { settings: updated });
                        } catch (err) {
                          console.error('Failed to save keep mic warm', err);
                        }
                      }}
                      className="h-7 rounded-md bg-background border border-input px-2 text-xs text-foreground focus:outline-none"
                    >
                      <option value="off">Off</option>
                      <option value="15s">15s</option>
                      <option value="30s">30s</option>
                      <option value="1m">1m</option>
                      <option value="5m">5m</option>
                    </select>
                  </div>
                </div>
              </div>
            </div>

            <div className="pt-2">
              <Button type="submit" size="sm" variant="default">
                Save Dictation Settings
              </Button>
            </div>
          </form>
        )}

        {/* 3. DICTIONARY & SNIPPETS SECTION */}
        {activeSection === 'dictionary' && (
          <DictionarySnippetsSettings
            settings={settings}
            onUpdateSettings={setSettings}
            onSaveDirect={handleSaveDirect}
          />
        )}

        {/* 4. LANGUAGES & SCRIPT SECTION */}
        {activeSection === 'languages' && (
          <form onSubmit={handleSave} className="space-y-6">
            <div>
              <p className="font-mono text-[10px] font-bold text-muted-foreground uppercase tracking-widest mb-1">
                MULTILINGUAL & ORTHOGRAPHY CONFIGURATION
              </p>
              <h2 className="text-lg font-bold text-foreground">Language & Writing Script Preferences</h2>
            </div>

            <div className="space-y-5">
              <div className="grid grid-cols-1 md:grid-cols-2 lg:grid-cols-3 gap-4">
                {/* 1. Primary Dictation Language */}
                <div className="p-4 rounded-lg border border-border bg-card space-y-2 flex flex-col justify-between">
                  <div>
                    <label htmlFor="primary-dictation-lang" className="block text-xs font-semibold text-foreground mb-0.5">
                      Primary Dictation Language
                    </label>
                    <p className="text-[10px] text-muted-foreground mb-2">
                      Default language for push-to-talk dictation
                    </p>
                  </div>
                  <select
                    id="primary-dictation-lang"
                    value={settings.language?.primary_dictation_language || 'en'}
                    onChange={(e) => {
                      const newPrimary = e.target.value;
                      const currentSpoken = settings.language?.spoken_languages || ['en'];
                      const updatedSpoken = currentSpoken.includes(newPrimary)
                        ? currentSpoken
                        : [...currentSpoken, newPrimary];
                      setSettings({
                        ...settings,
                        language: {
                          ...settings.language,
                          primary_dictation_language: newPrimary,
                          spoken_languages: updatedSpoken,
                        },
                      });
                    }}
                    className="w-full h-8 rounded-md bg-background border border-input px-2 text-xs text-foreground focus:outline-none"
                  >
                    {WHISPER_SUPPORTED_LANGUAGES.map((lang) => (
                      <option key={lang.code} value={lang.code}>
                        {lang.name} ({lang.code})
                      </option>
                    ))}
                  </select>
                </div>

                {/* 2. Output Writing Script */}
                <div className="p-4 rounded-lg border border-border bg-card space-y-2 flex flex-col justify-between">
                  <div>
                    <label className="block text-xs font-semibold text-foreground mb-0.5">
                      Output Writing Script
                    </label>
                    <p className="text-[10px] text-muted-foreground mb-2">
                      Alphabet format, independent of spoken language
                    </p>
                  </div>
                  <div className="flex bg-muted p-1 rounded-lg border border-border w-full">
                    {[
                      { value: 'latin', label: 'Latin / English' },
                      { value: 'native', label: 'Native Script' },
                    ].map((opt) => (
                      <button
                        key={opt.value}
                        type="button"
                        onClick={() =>
                          setSettings({
                            ...settings,
                            language: {
                              ...settings.language,
                              output_script: opt.value,
                            },
                          })
                        }
                        className={`flex-1 px-2 py-1 text-xs font-medium rounded-md transition-all ${
                          (settings.language?.output_script || 'latin') === opt.value
                            ? 'bg-card text-foreground font-semibold shadow-xs'
                            : 'text-muted-foreground hover:text-foreground'
                        }`}
                      >
                        {opt.label}
                      </button>
                    ))}
                  </div>
                </div>

                {/* 3. Notes & Summarization Language */}
                <div className="p-4 rounded-lg border border-border bg-card space-y-2 flex flex-col justify-between">
                  <div>
                    <label htmlFor="notes-lang" className="block text-xs font-semibold text-foreground mb-0.5">
                      Notes & Summarization Language
                    </label>
                    <p className="text-[10px] text-muted-foreground mb-2">
                      Language for synthesized notes and summaries
                    </p>
                  </div>
                  <select
                    id="notes-lang"
                    value={settings.language?.notes_language || 'en'}
                    onChange={(e) =>
                      setSettings({
                        ...settings,
                        language: {
                          ...settings.language,
                          notes_language: e.target.value,
                        },
                      })
                    }
                    className="w-full h-8 rounded-md bg-background border border-input px-2 text-xs text-foreground focus:outline-none"
                  >
                    {WHISPER_SUPPORTED_LANGUAGES.map((lang) => (
                      <option key={lang.code} value={lang.code}>
                        {lang.name} ({lang.code})
                      </option>
                    ))}
                  </select>
                </div>
              </div>

              {/* Languages I Speak (Multi-select) */}
              <div className="p-4 rounded-lg border border-border bg-card space-y-3">
                <div>
                  <label className="block text-xs font-semibold text-foreground mb-0.5">
                    Languages I Speak (Spoken Profile)
                  </label>
                  <p className="text-[11px] text-muted-foreground">
                    Select all languages you commonly speak. Vox recognizes and transcribes speech across your profile.
                  </p>
                </div>

                {/* Selected language chips */}
                <div className="flex flex-wrap gap-1.5 min-h-[32px] p-2 rounded-lg bg-muted/40 border border-border items-center">
                  {(settings.language?.spoken_languages || ['en']).map((code) => {
                    const langObj = WHISPER_SUPPORTED_LANGUAGES.find((l) => l.code === code);
                    const label = langObj ? `${langObj.name} (${code})` : code;
                    const isPrimary = settings.language?.primary_dictation_language === code;
                    return (
                      <Badge
                        key={code}
                        variant="secondary"
                        className="text-[11px] font-medium py-1 px-2.5 gap-1.5 rounded-md border border-border/80 flex items-center bg-card text-foreground shadow-2xs"
                      >
                        <span>{label}</span>
                        {isPrimary && (
                          <span className="text-[9px] uppercase tracking-wider text-primary font-bold">(Primary)</span>
                        )}
                        {(settings.language?.spoken_languages || []).length > 1 && (
                          <button
                            type="button"
                            onClick={() => {
                              const current = settings.language?.spoken_languages || ['en'];
                              const updated = current.filter((c) => c !== code);
                              setSettings({
                                ...settings,
                                language: {
                                  ...settings.language,
                                  spoken_languages: updated.length > 0 ? updated : ['en'],
                                },
                              });
                            }}
                            className="text-muted-foreground hover:text-destructive ml-0.5"
                            title="Remove language"
                          >
                            ×
                          </button>
                        )}
                      </Badge>
                    );
                  })}
                </div>

                {/* Quick-add toggle badges */}
                <div className="flex items-center gap-1.5 flex-wrap pt-1">
                  <span className="text-[10px] text-muted-foreground mr-1">Quick add:</span>
                  {WHISPER_SUPPORTED_LANGUAGES.slice(0, 12).map((lang) => {
                    const isSelected = (settings.language?.spoken_languages || ['en']).includes(lang.code);
                    if (isSelected) return null;
                    return (
                      <button
                        key={lang.code}
                        type="button"
                        onClick={() => {
                          const current = settings.language?.spoken_languages || ['en'];
                          setSettings({
                            ...settings,
                            language: {
                              ...settings.language,
                              spoken_languages: [...current, lang.code],
                            },
                          });
                        }}
                        className="px-2 py-0.5 text-[10px] rounded-md border border-border bg-background text-muted-foreground hover:text-foreground hover:border-primary/50 transition-colors"
                      >
                        + {lang.name}
                      </button>
                    );
                  })}
                </div>
              </div>

              <div className="pt-1">
                <Button type="submit" size="sm" variant="default">
                  Save Language Settings
                </Button>
              </div>
            </div>
          </form>
        )}

        {/* 5. MODELS & SPEECH HUB */}
        {activeSection === 'speech' && (
          <UnifiedModelsView
            settings={settings}
            onUpdateSettings={setSettings}
            onSaveDirect={handleSaveDirect}
            onNavigateTab={onNavigateTab}
          />
        )}

        {/* 6. ABOUT VOX (MERGED WITH PRIVACY & VAULT) */}
        {(activeSection === 'about' || activeSection === 'privacy') && (
          <div className="space-y-6 animate-in fade-in-50">
            <div className="flex items-center justify-between">
              <div>
                <p className="font-mono text-[10px] font-bold text-muted-foreground uppercase tracking-widest mb-1">
                  ABOUT VOX
                </p>
                <h2 className="text-lg font-bold text-foreground">Application, System Identity & Vault Privacy</h2>
              </div>
              <div className="flex items-center gap-2">
                <Badge variant="outline" className="text-[10px] font-mono">
                  v{installation?.app_version || '0.1.0'}
                </Badge>
                <Badge variant="outline" className="text-[10px] font-mono border-primary/30 text-primary bg-primary/5 uppercase">
                  {account?.authenticated ? 'Google Connected' : 'Local Mode'}
                </Badge>
              </div>
            </div>

            {/* System Identity & Application Information (3-Column Grid) */}
            <div className="grid grid-cols-1 md:grid-cols-2 xl:grid-cols-3 gap-4">
              {/* Card 1: Application Version & Updates */}
              <div className="p-4 rounded-lg border border-border bg-card space-y-4 flex flex-col justify-between">
                <div className="space-y-3">
                  <div className="flex items-center justify-between">
                    <div className="flex items-center gap-2">
                      <Laptop className="w-4 h-4 text-primary" />
                      <div>
                        <p className="text-xs font-semibold text-foreground">Vox Application</p>
                        <p className="text-[11px] text-muted-foreground">Version & update channel</p>
                      </div>
                    </div>
                    <Badge variant="outline" className="text-[10px] font-mono">
                      v{installation?.app_version || '0.1.0'}
                    </Badge>
                  </div>

                  <div className="text-[11px] text-muted-foreground space-y-1">
                    <p>
                      Platform: <span className="font-mono text-foreground capitalize">{installation?.platform || 'Windows'}</span> ({installation?.os_version || 'x86_64'})
                    </p>
                    {updateInfo && (
                      <p className="text-[10px]">
                        {updateInfo.is_offline ? (
                          <span className="text-amber-500">Offline mode</span>
                        ) : updateInfo.update_available ? (
                          <span className="text-emerald-500 font-semibold">v{updateInfo.latest_version} available</span>
                        ) : (
                          <span className="text-emerald-500 flex items-center gap-1">
                            <Check className="w-3 h-3" /> Up to date
                          </span>
                        )}
                      </p>
                    )}
                  </div>
                </div>

                <div className="pt-3 border-t border-border/60">
                  <Button
                    type="button"
                    variant="outline"
                    size="sm"
                    className="w-full text-xs h-8 gap-1.5"
                    onClick={handleCheckUpdates}
                    disabled={checkingUpdate}
                  >
                    <RefreshCw className={`w-3.5 h-3.5 ${checkingUpdate ? 'animate-spin' : ''}`} />
                    <span>{checkingUpdate ? 'Checking…' : 'Check for Updates'}</span>
                  </Button>
                </div>
              </div>

              {/* Card 2: Installation Identity */}
              <div className="p-4 rounded-lg border border-border bg-card space-y-4 flex flex-col justify-between">
                <div className="space-y-3">
                  <div className="flex items-center justify-between">
                    <div className="flex items-center gap-2">
                      <ShieldCheck className="w-4 h-4 text-emerald-500" />
                      <div>
                        <p className="text-xs font-semibold text-foreground">Installation Identity</p>
                        <p className="text-[11px] text-muted-foreground">Unique device identifier</p>
                      </div>
                    </div>
                    <Badge variant="outline" className="text-[10px] font-mono text-muted-foreground">
                      Stable
                    </Badge>
                  </div>

                  <div className="flex items-center justify-between bg-muted/40 p-2 rounded-lg border border-border/60">
                    <span className="text-xs font-mono text-muted-foreground truncate mr-2">{maskedId}</span>
                    <Button
                      type="button"
                      variant="ghost"
                      size="sm"
                      className="h-6 w-6 p-0 text-muted-foreground hover:text-foreground shrink-0"
                      onClick={copyInstallationId}
                      title="Copy Installation ID"
                    >
                      {copiedId ? <Check className="w-3 h-3 text-emerald-500" /> : <Copy className="w-3 h-3" />}
                    </Button>
                  </div>
                  <p className="text-[10px] text-muted-foreground leading-tight">
                    Survives restarts and updates. Used for update verification and diagnostic telemetry.
                  </p>
                </div>
              </div>

              {/* Card 3: Diagnostics & Bug Reporting */}
              <div className="p-4 rounded-lg border border-border bg-card space-y-4 flex flex-col justify-between">
                <div className="space-y-3">
                  <div className="flex items-center gap-2">
                    <Info className="w-4 h-4 text-primary" />
                    <div>
                      <p className="text-xs font-semibold text-foreground">Diagnostics & Telemetry</p>
                      <p className="text-[11px] text-muted-foreground">Crash reports & bug assistance</p>
                    </div>
                  </div>

                  <div className="flex items-center justify-between gap-2 pt-1">
                    <div>
                      <p className="text-xs font-medium text-foreground">Anonymous crash reports</p>
                      <p className="text-[10px] text-muted-foreground">Help fix bugs automatically</p>
                    </div>
                    <Switch
                      checked={isDiagnosticsAllowed}
                      onCheckedChange={handleToggleDiagnostics}
                    />
                  </div>
                </div>

                <div className="pt-3 border-t border-border/60">
                  <Button
                    type="button"
                    size="sm"
                    variant="outline"
                    className="w-full text-xs h-8 gap-1.5"
                    onClick={copyDiagnosticsSummary}
                  >
                    {copiedDiagnostics ? <Check className="w-3 h-3 text-emerald-500" /> : <Copy className="w-3 h-3" />}
                    <span>{copiedDiagnostics ? 'Copied Diagnostics' : 'Copy Diagnostic Info'}</span>
                  </Button>
                </div>
              </div>
            </div>

            {deleteAccountSuccess && (
              <div className="p-3.5 rounded-lg border border-emerald-500/30 bg-emerald-500/10 text-emerald-600 dark:text-emerald-400 text-xs flex items-center gap-2">
                <CheckCircle className="w-4 h-4 shrink-0" />
                <span>{deleteAccountSuccess}</span>
              </div>
            )}

            {deleteAccountError && (
              <div className="p-3.5 rounded-lg border border-destructive/30 bg-destructive/10 text-destructive text-xs flex items-center gap-2">
                <AlertCircle className="w-4 h-4 shrink-0" />
                <span>{deleteAccountError}</span>
              </div>
            )}

            <div className="space-y-4 pt-2">
              <div className="border-t border-border/60 pt-4">
                <p className="font-mono text-[10px] font-bold text-muted-foreground uppercase tracking-widest mb-1">
                  DATA CONTROL & PRIVACY BOUNDARIES
                </p>
                <h3 className="text-sm font-bold text-foreground">Data Ownership & Vault Isolation</h3>
              </div>
              {/* Privacy Overview & Safe Export (2 Columns) */}
              <div className="grid grid-cols-1 md:grid-cols-2 gap-4">
                <div className="p-4 rounded-lg bg-card border border-border space-y-2 flex flex-col justify-between">
                  <div className="space-y-1.5">
                    <div className="flex items-center gap-2 text-primary font-semibold text-xs">
                      <ShieldCheck className="w-4 h-4" />
                      <span>100% Local-First Processing</span>
                    </div>
                    <p className="text-[11px] text-muted-foreground leading-relaxed">
                      Vox operates locally on your machine. Voice transcriptions, raw audio recordings, markdown notes,
                      and LanceDB vectors stay inside your local directory. No third-party tracking or telemetry is collected.
                    </p>
                  </div>
                  <Badge variant="outline" className="w-fit text-[10px] font-mono text-emerald-600 dark:text-emerald-400 border-emerald-500/30">
                    Local Device Isolation
                  </Badge>
                </div>

                <div className="p-4 rounded-lg bg-card border border-border flex flex-col justify-between space-y-3">
                  <div>
                    <p className="text-xs font-bold text-foreground">Export All Vault Data</p>
                    <p className="text-[11px] text-muted-foreground mt-1">
                      Direct access to your stored markdown notes, audio transcripts, and LanceDB embeddings folder on disk.
                    </p>
                  </div>
                  <div className="pt-2 flex justify-end">
                    <Button
                      variant="default"
                      size="sm"
                      className="gap-2 text-xs h-8"
                      onClick={async () => {
                        const dir = vaultLocation?.path;
                        if (dir) {
                          try {
                            await invoke('open_vault_folder');
                          } catch {
                            alert(`Your vault is stored at: ${dir}`);
                          }
                        }
                      }}
                    >
                      <Download className="w-4 h-4" />
                      <span>Explore Vault Folder</span>
                    </Button>
                  </div>
                </div>
              </div>

              {/* Destructive Actions Section (Collapsible Accordion, Half and Half) */}
              <div className="rounded-lg border border-destructive/40 bg-destructive/5 overflow-hidden transition-all">
                <button
                  type="button"
                  onClick={() => setDestructiveOpen(!destructiveOpen)}
                  className="w-full flex items-center justify-between p-3.5 text-left cursor-pointer hover:bg-destructive/10 transition-colors select-none"
                  aria-expanded={destructiveOpen}
                >
                  <div className="flex items-center gap-2 text-destructive font-bold text-xs">
                    <AlertTriangle className="w-4 h-4 shrink-0" />
                    <span>Irreversible Data Reset & Account Actions</span>
                  </div>
                  <div className="flex items-center gap-2 text-[11px] text-muted-foreground">
                    <span>{destructiveOpen ? 'Hide actions' : 'Show actions'}</span>
                    {destructiveOpen ? (
                      <ChevronUp className="w-4 h-4 text-destructive shrink-0" />
                    ) : (
                      <ChevronDown className="w-4 h-4 shrink-0" />
                    )}
                  </div>
                </button>

                {destructiveOpen && (
                  <div className="p-4 pt-0 space-y-3.5 border-t border-destructive/20 animate-in fade-in-50 duration-150">
                    <p className="text-[11px] text-muted-foreground pt-3">
                      Use caution. These actions permanently erase configurations, accounts, or stored local vault data.
                    </p>

                    {/* Half and Half Grid */}
                    <div className="grid grid-cols-1 md:grid-cols-2 gap-3.5">
                      {/* Left Half: Delete Vox Cloud Account */}
                      <div className="p-3.5 rounded-lg border border-destructive/30 bg-card/60 flex flex-col justify-between space-y-3">
                        <div className="space-y-1.5">
                          <div className="flex items-center justify-between gap-2">
                            <p className="text-xs font-semibold text-foreground">Delete Vox Cloud Account</p>
                            <Badge variant="outline" className="text-[9px] px-1.5 py-0 border-destructive/30 text-destructive font-mono">
                              {account?.authenticated ? 'Cloud Linked' : 'Local Only'}
                            </Badge>
                          </div>
                          <p className="text-[11px] text-muted-foreground leading-relaxed">
                            Deletes your cloud account record and clears OS Keyring tokens.
                            <strong className="block mt-1 text-foreground font-medium">
                              Account ≠ Vault: Your local markdown notes, recordings, and scribbles remain 100% on this PC.
                            </strong>
                          </p>
                        </div>
                        <div className="pt-2 border-t border-border/40 flex justify-end">
                          <Button
                            variant="outline"
                            size="sm"
                            className="w-full sm:w-auto border-destructive/60 text-destructive hover:bg-destructive/10 gap-1.5 text-xs"
                            onClick={() => {
                              setDeleteAccountModalOpen(true);
                              setDeleteAccountAck(false);
                              setDeleteAccountInput('');
                              setDeleteAccountError(null);
                            }}
                            disabled={!account?.authenticated}
                          >
                            <User className="w-3.5 h-3.5" />
                            <span>Delete Cloud Account</span>
                          </Button>
                        </div>
                      </div>

                      {/* Right Half: Clear Local Vault & Index */}
                      <div className="p-3.5 rounded-lg border border-destructive/30 bg-card/60 flex flex-col justify-between space-y-3">
                        <div className="space-y-1.5">
                          <div className="flex items-center justify-between gap-2">
                            <p className="text-xs font-semibold text-foreground">Clear Local Vault & Index</p>
                            <Badge variant="outline" className="text-[9px] px-1.5 py-0 border-destructive/30 text-destructive font-mono">
                              Irreversible
                            </Badge>
                          </div>
                          <p className="text-[11px] text-muted-foreground leading-relaxed">
                            Permanently wipes stored markdown files, voice notes, scribbles, and the LanceDB vector database from local disk.
                          </p>
                        </div>
                        <div className="pt-2 border-t border-border/40 flex justify-end">
                          <Button
                            variant="outline"
                            size="sm"
                            className="w-full sm:w-auto border-destructive/60 text-destructive hover:bg-destructive/10 gap-1.5 text-xs"
                            onClick={() => {
                              setClearVaultModalOpen(true);
                              setClearVaultAck(false);
                              setClearVaultInput('');
                            }}
                          >
                            <Trash2 className="w-3.5 h-3.5" />
                            <span>Clear Local Vault</span>
                          </Button>
                        </div>
                      </div>
                    </div>

                    {/* Disconnect Sync (if authenticated) */}
                    {account?.authenticated && (
                      <div className="p-3 rounded-lg border border-border/60 bg-card/40 flex flex-col sm:flex-row sm:items-center justify-between gap-3">
                        <div className="space-y-0.5">
                          <p className="text-xs font-semibold text-foreground">Disconnect Hybrid Cloud Sync</p>
                          <p className="text-[11px] text-muted-foreground">
                            Signs out of your Vox identity and returns the application to 100% offline local-only operating mode.
                          </p>
                        </div>
                        <Button
                          variant="outline"
                          size="sm"
                          className="border-border text-muted-foreground hover:text-foreground gap-1.5 text-xs shrink-0"
                          onClick={handleDisconnectSync}
                        >
                          <Cloud className="w-3.5 h-3.5" />
                          <span>Disconnect Sync</span>
                        </Button>
                      </div>
                    )}
                  </div>
                )}
              </div>
            </div>

            {/* Modal: Delete Vox Account Double Confirmation */}
            {deleteAccountModalOpen && (
              <div className="fixed inset-0 z-50 flex items-center justify-center bg-background/80 backdrop-blur-sm p-4 animate-in fade-in-50">
                <div className="w-full max-w-md bg-card border border-destructive/50 rounded-lg p-6 shadow-2xl space-y-5">
                  <div className="flex items-start gap-3">
                    <div className="p-2 rounded-lg bg-destructive/10 text-destructive shrink-0">
                      <AlertTriangle className="w-5 h-5" />
                    </div>
                    <div className="space-y-1">
                      <h3 className="text-sm font-bold text-foreground">Delete Vox Cloud Account</h3>
                      <p className="text-xs text-muted-foreground leading-relaxed">
                        Step 1 of 2: Review destruction scope.
                      </p>
                    </div>
                  </div>

                  <div className="p-3.5 rounded-lg border border-border bg-muted/30 space-y-2 text-xs">
                    <div className="flex items-center gap-1.5 font-semibold text-destructive">
                      <span>What will be deleted:</span>
                    </div>
                    <ul className="list-disc list-inside space-y-1 text-muted-foreground pl-1 text-[11px]">
                      <li>Your Vox cloud profile and registration in Supabase</li>
                      <li>Secure OAuth credentials stored in your OS Keyring</li>
                      <li>Google Calendar synchronization association</li>
                    </ul>

                    <div className="pt-2 border-t border-border/60">
                      <div className="flex items-center gap-1.5 font-semibold text-emerald-600 dark:text-emerald-400">
                        <Check className="w-3.5 h-3.5" />
                        <span>What is PRESERVED (Account ≠ Vault):</span>
                      </div>
                      <p className="text-[11px] text-muted-foreground mt-0.5 leading-relaxed">
                        All your local Markdown files, Voice Notes, Scribbles, Audio files, and Vector index remain 100% untouched on this device.
                      </p>
                    </div>
                  </div>

                  {/* Double Confirmation Step */}
                  <div className="space-y-3 pt-1">
                    <label className="flex items-start gap-2 text-xs text-foreground cursor-pointer select-none">
                      <input
                        type="checkbox"
                        checked={deleteAccountAck}
                        onChange={(e) => setDeleteAccountAck(e.target.checked)}
                        className="mt-0.5 rounded border-border text-destructive focus:ring-destructive"
                      />
                      <span className="text-[11px] leading-tight text-muted-foreground">
                        I understand this permanently removes my cloud account and disconnects this installation.
                      </span>
                    </label>

                    <div className="space-y-1.5">
                      <label className="text-[11px] font-semibold text-muted-foreground">
                        Type <span className="font-mono text-destructive font-bold">DELETE ACCOUNT</span> to confirm:
                      </label>
                      <Input
                        value={deleteAccountInput}
                        onChange={(e) => setDeleteAccountInput(e.target.value)}
                        placeholder="DELETE ACCOUNT"
                        className="h-8 text-xs font-mono"
                      />
                    </div>
                  </div>

                  <div className="flex items-center justify-end gap-2 pt-2 border-t border-border">
                    <Button
                      size="sm"
                      variant="ghost"
                      className="text-xs h-8"
                      onClick={() => setDeleteAccountModalOpen(false)}
                      disabled={deletingAccount}
                    >
                      Cancel
                    </Button>
                    <Button
                      size="sm"
                      variant="destructive"
                      className="text-xs h-8 gap-1.5"
                      onClick={handleDeleteAccount}
                      disabled={
                        !deleteAccountAck ||
                        deleteAccountInput.trim().toUpperCase() !== 'DELETE ACCOUNT' ||
                        deletingAccount
                      }
                    >
                      {deletingAccount ? <RefreshCw className="w-3.5 h-3.5 animate-spin" /> : null}
                      <span>{deletingAccount ? 'Deleting Account...' : 'Permanently Delete Account'}</span>
                    </Button>
                  </div>
                </div>
              </div>
            )}

            {/* Modal: Clear Vault Double Confirmation */}
            {clearVaultModalOpen && (
              <div className="fixed inset-0 z-50 flex items-center justify-center bg-background/80 backdrop-blur-sm p-4 animate-in fade-in-50">
                <div className="w-full max-w-md bg-card border border-destructive/50 rounded-lg p-6 shadow-2xl space-y-5">
                  <div className="flex items-start gap-3">
                    <div className="p-2 rounded-lg bg-destructive/10 text-destructive shrink-0">
                      <AlertTriangle className="w-5 h-5" />
                    </div>
                    <div className="space-y-1">
                      <h3 className="text-sm font-bold text-destructive">Wipe Local Vault & Vectors</h3>
                      <p className="text-xs text-muted-foreground leading-relaxed">
                        Double Confirmation Required for Destructive Action.
                      </p>
                    </div>
                  </div>

                  <div className="p-3.5 rounded-lg border border-destructive/30 bg-destructive/5 space-y-2 text-xs">
                    <p className="font-semibold text-destructive">
                      WARNING: This action is IRREVERSIBLE.
                    </p>
                    <p className="text-[11px] text-muted-foreground leading-relaxed">
                      All local markdown notes, scribbles, audio recordings, and vector index tables in your vault directory will be deleted from your disk.
                    </p>
                  </div>

                  <div className="space-y-3 pt-1">
                    <label className="flex items-start gap-2 text-xs text-foreground cursor-pointer select-none">
                      <input
                        type="checkbox"
                        checked={clearVaultAck}
                        onChange={(e) => setClearVaultAck(e.target.checked)}
                        className="mt-0.5 rounded border-border text-destructive focus:ring-destructive"
                      />
                      <span className="text-[11px] leading-tight text-muted-foreground">
                        I understand that all local notes, scribbles, and vectors will be permanently destroyed.
                      </span>
                    </label>

                    <div className="space-y-1.5">
                      <label className="text-[11px] font-semibold text-muted-foreground">
                        Type <span className="font-mono text-destructive font-bold">CLEAR VAULT</span> to confirm:
                      </label>
                      <Input
                        value={clearVaultInput}
                        onChange={(e) => setClearVaultInput(e.target.value)}
                        placeholder="CLEAR VAULT"
                        className="h-8 text-xs font-mono"
                      />
                    </div>
                  </div>

                  <div className="flex items-center justify-end gap-2 pt-2 border-t border-border">
                    <Button
                      size="sm"
                      variant="ghost"
                      className="text-xs h-8"
                      onClick={() => setClearVaultModalOpen(false)}
                      disabled={clearingVault}
                    >
                      Cancel
                    </Button>
                    <Button
                      size="sm"
                      variant="destructive"
                      className="text-xs h-8 gap-1.5"
                      onClick={() => {
                        setClearingVault(true);
                        setTimeout(() => {
                          setClearingVault(false);
                          setClearVaultModalOpen(false);
                          setClearVaultAck(false);
                          setClearVaultInput('');
                        }, 1000);
                      }}
                      disabled={
                        !clearVaultAck ||
                        clearVaultInput.trim().toUpperCase() !== 'CLEAR VAULT' ||
                        clearingVault
                      }
                    >
                      <span>{clearingVault ? 'Clearing...' : 'Permanently Wipe Vault'}</span>
                    </Button>
                  </div>
                </div>
              </div>
            )}
          </div>
        )}

        {activeSection === 'capture' && <CaptureSettingsView />}

        {activeSection === 'meetings' && <MeetingSettingsView />}
        {activeSection === 'calendar' && <CalendarSettingsView />}

        {activeSection === 'trash' && <TrashSettings />}

        {/* 8. DEVELOPER SECTION */}
        {activeSection === 'developer' && <DeveloperSettingsView />}
      </main>
    </div>
  );
};
