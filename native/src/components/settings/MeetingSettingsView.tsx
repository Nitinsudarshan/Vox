import React from 'react';
import { invoke } from '@tauri-apps/api/core';
import {
  MonitorSpeaker,
  FileText,
  Sparkles,
  Languages,
  FolderOpen,
  Mic,
  Move,
  Rows2,
  Columns2,
  GripHorizontal,
} from 'lucide-react';

import { Switch } from '@/components/ui/switch';
import { Button } from '@/components/ui/button';
import { Card } from '@/components/ui/card';
import { Badge } from '@/components/ui/badge';
import * as meetings from '@/lib/meetings';
import type { MeetingTemplate, MeetingDevices } from '@/types/meetings';
import type { AudioDeviceInfo } from '@/types';
import { DevicePicker } from '@/components/meetings/MeetingRecorder';
import { ReminderSettingsCard } from './ReminderSettingsCard';
import { MeetingPill, type MeetingPillOrientation } from '@/components/meetings/MeetingPill';

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

export const PILL_POSITIONS: Array<{ id: string; label: string; short: string }> = [
  { id: 'top_left', label: 'Top Left', short: 'TL' },
  { id: 'top_center', label: 'Top Center', short: 'TC' },
  { id: 'top_right', label: 'Top Right', short: 'TR' },
  { id: 'middle_left', label: 'Middle Left', short: 'ML' },
  { id: 'middle_center', label: 'Center', short: 'C' },
  { id: 'middle_right', label: 'Middle Right', short: 'MR' },
  { id: 'bottom_left', label: 'Bottom Left', short: 'BL' },
  { id: 'bottom_center', label: 'Bottom Center', short: 'BC' },
  { id: 'bottom_right', label: 'Bottom Right', short: 'BR' },
];

interface MeetingSettingsShape {
  captureSystemAudio: boolean;
  defaultTemplateId: string;
  summaryLanguage: string;
  autoSummarize: boolean;
  pillStyle?: MeetingPillOrientation;
  pillPosition?: string;
  pillFreeX?: number;
  pillFreeY?: number;
}

const DEFAULTS: MeetingSettingsShape = {
  captureSystemAudio: true,
  defaultTemplateId: 'general',
  summaryLanguage: '',
  autoSummarize: false,
  pillStyle: 'horizontal',
  pillPosition: 'middle_right',
  pillFreeX: 85,
  pillFreeY: 50,
};

const getPreviewPositionClasses = (position?: string): string => {
  switch (position) {
    case 'top_left':
      return 'justify-start items-start';
    case 'top_center':
      return 'justify-start items-center';
    case 'top_right':
      return 'justify-start items-end';
    case 'middle_left':
      return 'justify-center items-start';
    case 'middle_center':
      return 'justify-center items-center';
    case 'bottom_left':
      return 'justify-end items-start';
    case 'bottom_center':
      return 'justify-end items-center';
    case 'bottom_right':
      return 'justify-end items-end';
    case 'middle_right':
    default:
      return 'justify-center items-end';
  }
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

  const previewViewportRef = React.useRef<HTMLDivElement>(null);
  const previewPillRef = React.useRef<HTMLDivElement>(null);
  const [isPreviewDragging, setIsPreviewDragging] = React.useState(false);
  const previewDragStartRef = React.useRef<{
    startMouseX: number;
    startMouseY: number;
    initialCenterX: number;
    initialCenterY: number;
  } | null>(null);

  const handlePreviewPointerDown = (e: React.PointerEvent) => {
    if ((e.target as HTMLElement).closest('button')) return;
    setIsPreviewDragging(true);

    const viewport = previewViewportRef.current;
    const pill = previewPillRef.current;
    if (viewport && pill) {
      const vRect = viewport.getBoundingClientRect();
      const pRect = pill.getBoundingClientRect();
      previewDragStartRef.current = {
        startMouseX: e.clientX,
        startMouseY: e.clientY,
        initialCenterX: pRect.left + pRect.width / 2 - vRect.left,
        initialCenterY: pRect.top + pRect.height / 2 - vRect.top,
      };
    }
  };

  React.useEffect(() => {
    if (!isPreviewDragging) return;

    const handlePointerMove = (e: PointerEvent) => {
      const viewport = previewViewportRef.current;
      const pill = previewPillRef.current;
      if (!viewport || !previewDragStartRef.current) return;

      const vRect = viewport.getBoundingClientRect();
      const pRect = pill?.getBoundingClientRect() || { width: 140, height: 40 };

      // Strictly ensure the pill edges never cross preview boundaries
      const edgeMargin = 6;
      const halfW = pRect.width / 2;
      const halfH = pRect.height / 2;

      const minCenterX = edgeMargin + halfW;
      const maxCenterX = Math.max(minCenterX, vRect.width - edgeMargin - halfW);
      const minCenterY = edgeMargin + halfH;
      const maxCenterY = Math.max(minCenterY, vRect.height - edgeMargin - halfH);

      const deltaX = e.clientX - previewDragStartRef.current.startMouseX;
      const deltaY = e.clientY - previewDragStartRef.current.startMouseY;
      const rawCenterX = previewDragStartRef.current.initialCenterX + deltaX;
      const rawCenterY = previewDragStartRef.current.initialCenterY + deltaY;

      const clampedCenterX = Math.max(minCenterX, Math.min(maxCenterX, rawCenterX));
      const clampedCenterY = Math.max(minCenterY, Math.min(maxCenterY, rawCenterY));

      let minPctX = Math.ceil(((edgeMargin + halfW) / vRect.width) * 100);
      let maxPctX = Math.floor(((vRect.width - edgeMargin - halfW) / vRect.width) * 100);
      let minPctY = Math.ceil(((edgeMargin + halfH) / vRect.height) * 100);
      let maxPctY = Math.floor(((vRect.height - edgeMargin - halfH) / vRect.height) * 100);

      if (minPctX > maxPctX) {
        minPctX = 50;
        maxPctX = 50;
      }
      if (minPctY > maxPctY) {
        minPctY = 50;
        maxPctY = 50;
      }

      const rawPctX = (clampedCenterX / vRect.width) * 100;
      const rawPctY = (clampedCenterY / vRect.height) * 100;

      const newX = Math.max(minPctX, Math.min(maxPctX, Math.round(rawPctX)));
      const newY = Math.max(minPctY, Math.min(maxPctY, Math.round(rawPctY)));

      setSettings((prev) => ({ ...prev, pillFreeX: newX, pillFreeY: newY }));
    };

    const handlePointerUp = () => {
      setIsPreviewDragging(false);
      previewDragStartRef.current = null;
      void update({ pillFreeX: settings.pillFreeX ?? 85, pillFreeY: settings.pillFreeY ?? 50 });
    };

    window.addEventListener('pointermove', handlePointerMove);
    window.addEventListener('pointerup', handlePointerUp);
    return () => {
      window.removeEventListener('pointermove', handlePointerMove);
      window.removeEventListener('pointerup', handlePointerUp);
    };
  }, [isPreviewDragging, settings.pillFreeX, settings.pillFreeY]);

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

      {/* Card: Meeting Recording Pill Style & Position */}
      <Card className="p-4 space-y-4">
        <div className="flex items-start justify-between gap-4">
          <div>
            <div className="flex items-center gap-2">
              <h3 className="text-sm font-medium text-foreground">Meeting Recording Pill</h3>
              <Badge variant="outline" className="text-[10px] font-mono border-indigo-500/30 text-indigo-400 bg-indigo-500/5 uppercase">
                On-Screen Overlay
              </Badge>
            </div>
            <p className="text-xs text-muted-foreground mt-0.5">
              Choose the floating capsule style and screen anchor position during active meeting recordings.
            </p>
          </div>
          <span className="text-[11px] font-mono text-muted-foreground/80 shrink-0">
            {settings.pillStyle === 'vertical' ? 'Vertical' : 'Horizontal'} · {settings.pillPosition === 'free' ? `free (${settings.pillFreeX ?? 85}%, ${settings.pillFreeY ?? 50}%)` : (settings.pillPosition ?? 'middle_right')}
          </span>
        </div>

        <div className="grid grid-cols-1 md:grid-cols-12 gap-4 pt-1">
          {/* Controls Column */}
          <div className="md:col-span-7 space-y-4">
            {/* Style Selector */}
            <div className="space-y-1.5">
              <label className="text-xs font-semibold text-foreground flex items-center gap-1.5">
                <Rows2 className="w-3.5 h-3.5 text-muted-foreground" />
                Pill Style
              </label>
              <div className="flex items-center gap-2 max-w-sm">
                <Button
                  variant={settings.pillStyle === 'horizontal' ? 'default' : 'outline'}
                  size="sm"
                  onClick={() => void update({ pillStyle: 'horizontal' })}
                  className="text-xs h-8 flex-1 gap-1.5"
                >
                  <Rows2 className="w-3.5 h-3.5 rotate-90" />
                  Horizontal (Default)
                </Button>
                <Button
                  variant={settings.pillStyle === 'vertical' ? 'default' : 'outline'}
                  size="sm"
                  onClick={() => void update({ pillStyle: 'vertical' })}
                  className="text-xs h-8 flex-1 gap-1.5"
                >
                  <Columns2 className="w-3.5 h-3.5" />
                  Vertical
                </Button>
              </div>
            </div>

            {/* Position Selector */}
            <div className="space-y-1.5">
              <div className="flex items-center justify-between max-w-sm">
                <label className="text-xs font-semibold text-foreground flex items-center gap-1.5">
                  <Move className="w-3.5 h-3.5 text-muted-foreground" />
                  Screen Position
                </label>
                <Button
                  variant={settings.pillPosition === 'free' ? 'default' : 'outline'}
                  size="sm"
                  onClick={() => void update({ pillPosition: 'free' })}
                  className="text-[11px] h-6 px-2 gap-1"
                >
                  <GripHorizontal className="w-3 h-3" />
                  Free Movement
                </Button>
              </div>

              {/* 3x3 Grid */}
              <div className="grid grid-cols-3 gap-1.5 max-w-sm">
                {PILL_POSITIONS.map((pos) => {
                  const isSelected = settings.pillPosition !== 'free' && settings.pillPosition === pos.id;
                  return (
                    <button
                      key={pos.id}
                      type="button"
                      onClick={() => void update({ pillPosition: pos.id })}
                      className={`text-[11px] py-1 px-2 rounded border font-mono transition-colors cursor-pointer ${
                        isSelected
                          ? 'bg-primary text-primary-foreground border-primary font-bold shadow-xs'
                          : 'bg-card/80 border-border/70 text-muted-foreground hover:bg-accent hover:text-foreground'
                      }`}
                      title={pos.label}
                    >
                      {pos.short} · {pos.label}
                    </button>
                  );
                })}
              </div>

              {settings.pillPosition === 'free' && (
                <div className="pt-2 border-t border-border/40 space-y-1.5">
                  <div className="flex items-center justify-between text-[11px] text-muted-foreground">
                    <span className="flex items-center gap-1 font-medium text-foreground">
                      <GripHorizontal className="w-3 h-3 text-muted-foreground" />
                      Free Position
                    </span>
                    <Button
                      variant="ghost"
                      size="sm"
                      onClick={() => void update({ pillFreeX: 85, pillFreeY: 50 })}
                      className="h-5 px-1.5 text-[10px] text-muted-foreground hover:text-foreground"
                    >
                      Reset (85%, 50%)
                    </Button>
                  </div>
                  <div className="grid grid-cols-2 gap-2 text-[11px]">
                    <div className="flex items-center justify-between bg-muted/40 px-2 py-1 rounded border border-border/50">
                      <span className="text-muted-foreground">X:</span>
                      <span className="font-mono font-medium">{settings.pillFreeX ?? 85}%</span>
                    </div>
                    <div className="flex items-center justify-between bg-muted/40 px-2 py-1 rounded border border-border/50">
                      <span className="text-muted-foreground">Y:</span>
                      <span className="font-mono font-medium">{settings.pillFreeY ?? 50}%</span>
                    </div>
                  </div>
                  <p className="text-[10px] text-muted-foreground/70 flex items-center gap-1">
                    <span className="w-1.5 h-1.5 rounded-full bg-emerald-500/70 shrink-0" />
                    Drag pill in preview. Viewport boundary locked.
                  </p>
                </div>
              )}
            </div>
          </div>

          {/* Mini Preview Column */}
          <div className="md:col-span-5 flex flex-col">
            <div className="text-[11px] text-muted-foreground mb-1.5 flex items-center justify-between">
              <span>Preview</span>
              <span className="font-mono text-[10px] opacity-70">
                {settings.pillPosition === 'free'
                  ? `free (${settings.pillFreeX ?? 85}%, ${settings.pillFreeY ?? 50}%)`
                  : (settings.pillPosition ?? 'middle_right')}
              </span>
            </div>

            <div
              ref={previewViewportRef}
              className={`relative flex-1 min-h-[170px] rounded-lg border border-border/80 bg-background/60 overflow-hidden flex flex-col p-4 select-none ${
                isPreviewDragging ? 'cursor-grabbing' : ''
              }`}
            >
              {settings.pillPosition === 'free' ? (
                <div
                  ref={previewPillRef}
                  onPointerDown={handlePreviewPointerDown}
                  style={{
                    position: 'absolute',
                    left: `${Math.max(15, Math.min(85, settings.pillFreeX ?? 85))}%`,
                    top: `${Math.max(15, Math.min(85, settings.pillFreeY ?? 50))}%`,
                    transform: 'translate(-50%, -50%)',
                  }}
                  className={`cursor-grab touch-none ${isPreviewDragging ? 'cursor-grabbing' : ''}`}
                  title="Drag to reposition freely (clamped to screen)"
                >
                  <div className="group relative">
                    <div className="absolute -top-3 left-1/2 -translate-x-1/2 opacity-0 group-hover:opacity-80 transition-opacity text-[8px] font-mono bg-background/90 px-1 py-0.2 rounded shadow-xs pointer-events-none flex items-center gap-0.5 border border-border">
                      <GripHorizontal className="w-2 h-2" />
                      Drag
                    </div>
                    <MeetingPill
                      orientation={settings.pillStyle ?? 'horizontal'}
                      state="recording"
                      elapsedSeconds={42}
                    />
                  </div>
                </div>
              ) : (
                <div className={`w-full h-full flex-1 flex flex-col ${getPreviewPositionClasses(settings.pillPosition)}`}>
                  <MeetingPill
                    orientation={settings.pillStyle ?? 'horizontal'}
                    state="recording"
                    elapsedSeconds={42}
                  />
                </div>
              )}
            </div>
          </div>
        </div>
      </Card>

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
