import React from 'react';
import { invoke } from '@tauri-apps/api/core';
import { AlertTriangle, Bell } from 'lucide-react';

import { Card } from '@/components/ui/card';
import { Switch } from '@/components/ui/switch';
import * as meetings from '@/lib/meetings';

/**
 * Which meeting reminders Vox is allowed to raise.
 *
 * Mirrors `calendar::reminders::ReminderSettings`. One switch per kind rather
 * than one for the feature: they are different evidence about different
 * situations, and somebody who wants to be told their meeting is unrecorded
 * does not necessarily want to be told a Zoom window is open.
 */
interface ReminderShape {
  remind_before_meeting: boolean;
  remind_if_unrecorded: boolean;
  remind_on_detection: boolean;
}

const DEFAULTS: ReminderShape = {
  remind_before_meeting: true,
  remind_if_unrecorded: true,
  remind_on_detection: true,
};

export const ReminderSettingsCard: React.FC = () => {
  const [settings, setSettings] = React.useState<ReminderShape>(DEFAULTS);
  const [loaded, setLoaded] = React.useState(false);
  const [error, setError] = React.useState<string | null>(null);

  React.useEffect(() => {
    void (async () => {
      try {
        const all = await invoke<Record<string, unknown>>('get_settings');
        const stored = ((all?.meetings as Record<string, unknown>)?.reminders ??
          {}) as Partial<ReminderShape>;
        setSettings({ ...DEFAULTS, ...stored });
      } catch {
        // Defaults are the honest fallback: they are what the backend uses too.
      } finally {
        setLoaded(true);
      }
    })();
  }, []);

  const update = async (patch: Partial<ReminderShape>) => {
    const next = { ...settings, ...patch };
    // Optimistic: a switch must not lag the click. A failed write costs the
    // preference sticking, and the message below says so.
    setSettings(next);
    try {
      const all = await invoke<Record<string, unknown>>('get_settings');
      const meetingsBlock = (all?.meetings ?? {}) as Record<string, unknown>;
      await invoke('save_settings', {
        settings: { ...all, meetings: { ...meetingsBlock, reminders: next } },
      });
      setError(null);
    } catch (err) {
      setError(meetings.meetingErrorMessage(err));
    }
  };

  if (!loaded) return null;

  return (
    <Card className="p-4 space-y-3">
      <div className="min-w-0">
        <p className="text-sm font-medium text-foreground flex items-center gap-2">
          <Bell className="w-3.5 h-3.5 text-muted-foreground" />
          Meeting reminders
        </p>
        <p className="text-xs text-muted-foreground mt-1 leading-relaxed max-w-xl">
          A card in a small window of its own, above whatever you are working in, carrying
          Record, Join and Snooze. Hovering it pauses its countdown; it plays itself out
          otherwise.
        </p>
      </div>

      {error && (
        <p className="flex items-start gap-2 text-xs text-destructive">
          <AlertTriangle className="w-3.5 h-3.5 shrink-0 mt-0.5" />
          <span>{error}</span>
        </p>
      )}

      <ReminderToggle
        label="Before a meeting starts"
        description="Five minutes ahead of anything scheduled on a connected calendar."
        checked={settings.remind_before_meeting}
        onChange={(remind_before_meeting) => void update({ remind_before_meeting })}
      />
      <div className="h-px bg-border/60" />
      <ReminderToggle
        label="When a meeting is running unrecorded"
        description="A few minutes after a scheduled start, and only while the call is actually open on screen — a meeting you left is not one you forgot to record."
        checked={settings.remind_if_unrecorded}
        onChange={(remind_if_unrecorded) => void update({ remind_if_unrecorded })}
      />
      <div className="h-px bg-border/60" />
      <ReminderToggle
        label="When a call is detected that is not in the calendar"
        description="An ad-hoc call somebody pulled you into — the meeting with no calendar entry, which nothing else here can catch. Reads window titles, which never leave this machine, and a call with no topic in its title has to persist before it interrupts you."
        checked={settings.remind_on_detection}
        onChange={(remind_on_detection) => void update({ remind_on_detection })}
      />
    </Card>
  );
};

const ReminderToggle: React.FC<{
  label: string;
  description: string;
  checked: boolean;
  onChange: (value: boolean) => void;
}> = ({ label, description, checked, onChange }) => (
  <div className="flex items-start justify-between gap-4">
    <div className="min-w-0">
      <p className="text-xs font-medium text-foreground">{label}</p>
      <p className="text-[11px] text-muted-foreground mt-0.5">{description}</p>
    </div>
    <Switch checked={checked} onCheckedChange={onChange} aria-label={label} />
  </div>
);
