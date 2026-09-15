import React from 'react';
import { invoke } from '@tauri-apps/api/core';
import { MonitorSpeaker, FileText, Sparkles, Languages, FolderOpen, Mic } from 'lucide-react';

import { Switch } from '@/components/ui/switch';
import { Card } from '@/components/ui/card';
import * as meetings from '@/lib/meetings';
import type { MeetingTemplate, MeetingDevices } from '@/types/meetings';
import type { AudioDeviceInfo } from '@/types';
import { DevicePicker } from '@/components/meetings/MeetingRecorder';
import { ReminderSettingsCard } from './ReminderSettingsCard';

/** The languages a report can be translated into, matching the Rust table. */
const SUMMARY_LANGUAGES: Array<{ code: string; label: string }> = [
  { code: '', label: 'English (no translation)' },
  { code: 'ar', label: 'Arabic' },
  { code: 'de', label: 'German' },
  { code: 'es', label: 'Spanish' },
  { code: 'fr', label: 'French' },
  { code: 'hi', label: 'Hindi' },
  { code: 'it', label: 'Italian' },
  { code: 'ja', label: 'Japanese' },
  { code: 'ko', label: 'Korean' },
  { code: 'mr', label: 'Marathi' },
  { code: 'nl', label: 'Dutch' },
  { code: 'pt', label: 'Portuguese' },
  { code: 'ru', label: 'Russian' },
  { code: 'ta', label: 'Tamil' },
  { code: 'te', label: 'Telugu' },
  { code: 'tr', label: 'Turkish' },
  { code: 'zh', label: 'Chinese' },
];

interface MeetingSettingsShape {
  captureSystemAudio: boolean;
  defaultTemplateId: string;
  summaryLanguage: string;
  autoSummarize: boolean;
}

const DEFAULTS: MeetingSettingsShape = {
  captureSystemAudio: true,
  defaultTemplateId: 'general',
  summaryLanguage: '',
  autoSummarize: false,
};

/**
 * Meetings preferences.
 *
 * Reads and writes the whole `AppSettings` object, as every other settings
 * view here does — the backend's save is a whole-document replace, so a
 * partial write would silently reset whatever the user last changed elsewhere.
 */
export const MeetingSettingsView: React.FC = () => {
  const [settings, setSettings] = React.useState<MeetingSettingsShape>(DEFAULTS);
  const [templates, setTemplates] = React.useState<MeetingTemplate[]>([]);
  const [devices, setDevices] = React.useState<MeetingDevices>({});
  const [inputs, setInputs] = React.useState<AudioDeviceInfo[]>([]);
  const [outputs, setOutputs] = React.useState<AudioDeviceInfo[]>([]);
  const [loaded, setLoaded] = React.useState(false);
  const [error, setError] = React.useState<string | null>(null);

  React.useEffect(() => {
    void (async () => {
      try {
        const all = await invoke<Record<string, unknown>>('get_settings');
        const stored = (all?.meetings ?? {}) as Partial<MeetingSettingsShape>;
        setSettings({ ...DEFAULTS, ...stored });
      } catch (err) {
        setError(meetings.meetingErrorMessage(err));
      } finally {
        setLoaded(true);
      }
      try {
        setTemplates(await meetings.listTemplates());
      } catch {
        setTemplates([]);
      }
      void meetings.listInputDevices().then(setInputs).catch(() => setInputs([]));
      void meetings.listOutputDevices().then(setOutputs).catch(() => setOutputs([]));
      void meetings
        .getMeetingDevices()
        .then((saved) => setDevices(saved ?? {}))
        .catch(() => undefined);
    })();
  }, []);

  const update = async (patch: Partial<MeetingSettingsShape>) => {
    const next = { ...settings, ...patch };
    setSettings(next);
    try {
      const all = await invoke<Record<string, unknown>>('get_settings');
      await invoke('save_settings', { settings: { ...all, meetings: next } });
      setError(null);
    } catch (err) {
      setError(meetings.meetingErrorMessage(err));
    }
  };

  const handleDevicesChange = async (next: MeetingDevices) => {
    setDevices(next);
    try {
      await meetings.saveMeetingDevices(next);
      setError(null);
    } catch (err) {
      setError(meetings.meetingErrorMessage(err));
    }
  };

  if (!loaded) {
    return <p className="text-xs text-muted-foreground">Loading…</p>;
  }

  return (
    <div className="space-y-4 w-full">
      <div>
        <h2 className="text-lg font-semibold text-foreground">Meetings</h2>
        <p className="text-xs text-muted-foreground mt-1">
          Recording, transcription and report defaults. Everything here runs on this machine;
          only report generation reaches a model, and only the one you have configured.
        </p>
      </div>

      {error && <p className="text-xs text-destructive">{error}</p>}

      {/* Full width: the lead-time row does not fit a half column, and this is
          the setting a person comes here for after missing a meeting. */}
      <ReminderSettingsCard />

      <div className="grid grid-cols-1 lg:grid-cols-2 gap-4">
        {/* Card 1: Audio Input & Capture Devices */}
        <Card className="p-4 space-y-3 flex flex-col justify-between">
          <div>
            <h3 className="text-sm font-medium text-foreground">Audio Input & Capture Devices</h3>
            <p className="text-xs text-muted-foreground mt-0.5">
              Select the audio sources used when recording a meeting.
            </p>
          </div>
          <div className="grid grid-cols-1 sm:grid-cols-2 gap-3 pt-1">
            <DevicePicker
              icon={Mic}
              label="Microphone"
              devices={inputs}
              value={devices.microphone ?? ''}
              defaultLabel="System default"
              onChange={(name) =>
                void handleDevicesChange({ ...devices, microphone: name === '' ? null : name })
              }
            />
            <DevicePicker
              icon={MonitorSpeaker}
              label="System audio from"
              devices={outputs}
              value={devices.system_audio ?? ''}
              defaultLabel="Default output"
              onChange={(name) =>
                void handleDevicesChange({ ...devices, system_audio: name === '' ? null : name })
              }
            />
          </div>
        </Card>

        {/* Card 2: Capture Options */}
        <Card className="p-4 space-y-4 flex flex-col justify-between">
          <label className="flex items-start justify-between gap-4 cursor-pointer">
            <span className="flex items-start gap-3 min-w-0">
              <MonitorSpeaker className="w-4 h-4 text-muted-foreground shrink-0 mt-0.5" />
              <span className="min-w-0">
                <span className="block text-sm font-medium text-foreground">
                  Record system audio
                </span>
                <span className="block text-xs text-muted-foreground mt-0.5">
                  Captures what this machine plays as well as your microphone, so the other
                  people on a call are transcribed too. Off means only your side is recorded.
                </span>
              </span>
            </span>
            <Switch
              checked={settings.captureSystemAudio}
              onCheckedChange={(checked) => void update({ captureSystemAudio: checked })}
              aria-label="Record system audio"
            />
          </label>

          <div className="h-px bg-border/60" />

          <label className="flex items-start justify-between gap-4 cursor-pointer">
            <span className="flex items-start gap-3 min-w-0">
              <Sparkles className="w-4 h-4 text-muted-foreground shrink-0 mt-0.5" />
              <span className="min-w-0">
                <span className="block text-sm font-medium text-foreground">
                  Write the report automatically
                </span>
                <span className="block text-xs text-muted-foreground mt-0.5">
                  Starts generating as soon as a recording stops. Summarising a long meeting on
                  a local model takes minutes of full-speed inference, so this is off until you
                  ask for it.
                </span>
              </span>
            </span>
            <Switch
              checked={settings.autoSummarize}
              onCheckedChange={(checked) => void update({ autoSummarize: checked })}
              aria-label="Write the report automatically"
            />
          </label>
        </Card>

        {/* Card 3: Report Shape & Language */}
        <Card className="p-4 space-y-4 flex flex-col justify-between">
          <div className="flex items-start gap-3">
            <FileText className="w-4 h-4 text-muted-foreground shrink-0 mt-0.5" />
            <div className="flex-1 min-w-0">
              <label
                htmlFor="meeting-default-template"
                className="block text-sm font-medium text-foreground"
              >
                Default report template
              </label>
              <p className="text-xs text-muted-foreground mt-0.5 mb-2">
                Which shape a new report takes. You can pick a different one per meeting.
              </p>
              <select
                id="meeting-default-template"
                value={settings.defaultTemplateId}
                onChange={(event) => void update({ defaultTemplateId: event.target.value })}
                className="h-8 w-full rounded-lg border border-border bg-card px-2 text-xs text-foreground"
              >
                {templates.map((template) => (
                  <option key={template.id} value={template.id}>
                    {template.name}
                    {template.custom ? ' (yours)' : ''}
                  </option>
                ))}
              </select>
            </div>
          </div>

          <div className="h-px bg-border/60" />

          <div className="flex items-start gap-3">
            <Languages className="w-4 h-4 text-muted-foreground shrink-0 mt-0.5" />
            <div className="flex-1 min-w-0">
              <label
                htmlFor="meeting-summary-language"
                className="block text-sm font-medium text-foreground"
              >
                Report language
              </label>
              <p className="text-xs text-muted-foreground mt-0.5 mb-2">
                Reports are written in English and translated afterwards for structure consistency.
              </p>
              <select
                id="meeting-summary-language"
                value={settings.summaryLanguage}
                onChange={(event) => void update({ summaryLanguage: event.target.value })}
                className="h-8 w-full rounded-lg border border-border bg-card px-2 text-xs text-foreground"
              >
                {SUMMARY_LANGUAGES.map((language) => (
                  <option key={language.code} value={language.code}>
                    {language.label}
                  </option>
                ))}
              </select>
            </div>
          </div>
        </Card>

        {/* Card 4: Custom Templates Guide */}
        <Card className="p-4 flex flex-col justify-between space-y-3">
          <div className="flex items-start gap-3">
            <FolderOpen className="w-4 h-4 text-primary shrink-0 mt-0.5" />
            <div className="min-w-0">
              <p className="text-sm font-medium text-foreground">Your own templates</p>
              <p className="text-xs text-muted-foreground mt-1 leading-relaxed">
                Drop a JSON file in <code className="font-mono bg-muted px-1 py-0.5 rounded">meeting-templates/</code> inside
                Vox&apos;s config folder to add a report shape or replace one of the built-ins.
                A file whose <code className="font-mono bg-muted px-1 py-0.5 rounded">id</code> matches a built-in
                template replaces it; any other id adds a new one without needing an app rebuild.
              </p>
            </div>
          </div>
          <div className="p-2.5 rounded-lg bg-muted/40 border border-border text-[11px] text-muted-foreground font-mono">
            Location: %APPDATA%/Vox/meeting-templates
          </div>
        </Card>
      </div>
    </div>
  );
};
