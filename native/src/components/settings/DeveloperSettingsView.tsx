import React, { useState, useEffect } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { DeveloperSettings } from '../../types';
import { Terminal, RefreshCw, Bell } from 'lucide-react';
import { Button } from '@/components/ui/button';
import * as meetings from '@/lib/meetings';
import type { ReminderKind } from '@/types/meetings';
import { Switch } from '@/components/ui/switch';
import { Badge } from '@/components/ui/badge';

/**
 * Every kind of meeting reminder, with what each one is for.
 *
 * The list is the point. Three separate messages is a design decision that is
 * otherwise invisible until one of them turns up, and the third — a meeting
 * running with nothing being captured — is the failure this app exists to
 * prevent and the only one that is silent by nature.
 */
const REMINDER_KINDS: Array<{
  kind: ReminderKind;
  offset: string;
  label: string;
  description: string;
}> = [
  {
    kind: 'upcoming',
    offset: 't−5',
    label: 'Before it starts',
    description:
      'Fires at the closest chosen lead time — 15, 10, 5 or 1 minute out. Offers Join and Join and record.',
  },
  {
    kind: 'starting',
    offset: 't',
    label: 'Starting now',
    description:
      'At the start, and for up to five minutes after if Vox was closed through it. Same actions.',
  },
  {
    kind: 'not_recording',
    offset: 't+5',
    label: 'Under way, nothing being recorded',
    description:
      'Five minutes in, when no recording is running and Vox has none of this meeting. Leads with Record now.',
  },
];

export const DeveloperSettingsView: React.FC = () => {
  const [devSettings, setDevSettings] = useState<DeveloperSettings>({
    force_onboarding_on_launch: false,
  });
  const [loading, setLoading] = useState(true);
  const [saving, setSaving] = useState(false);
  const [, setSavedFeedback] = useState(false);
  const [reminderResult, setReminderResult] = useState<string | null>(null);

  useEffect(() => {
    loadDevSettings();
  }, []);

  const loadDevSettings = async () => {
    try {
      setLoading(true);
      const res = await invoke<DeveloperSettings>('get_developer_settings');
      setDevSettings(res);
    } catch (err) {
      console.error('Failed to load developer settings:', err);
    } finally {
      setLoading(false);
    }
  };

  const handleToggleForceOnboarding = async (checked: boolean) => {
    try {
      setSaving(true);
      const res = await invoke<DeveloperSettings>('set_developer_force_onboarding', {
        enabled: checked,
      });
      setDevSettings(res);
      setSavedFeedback(true);
      setTimeout(() => setSavedFeedback(false), 2000);
    } catch (err) {
      console.error('Failed to update developer onboarding setting:', err);
    } finally {
      setSaving(false);
    }
  };

  const handleTestReminder = async (kinds: ReminderKind[]) => {
    setReminderResult(null);
    try {
      for (const kind of kinds) {
        await meetings.sendTestReminder(kind);
      }
      setReminderResult(
        kinds.length === 1
          ? 'Sent. The reminder window should be in the corner of your screen.'
          : 'Sent all three. They stack in the reminder window, newest at the bottom.',
      );
    } catch (err) {
      setReminderResult(meetings.meetingErrorMessage(err));
    }
    setTimeout(() => setReminderResult(null), 8000);
  };

  return (
    <div className="space-y-6">
      <div className="border-b border-border/40 pb-5">
        <div className="flex items-center gap-3 mb-1.5">
          <div className="flex items-center gap-2">
            <Terminal className="w-5 h-5 text-amber-500" />
            <h2 className="text-xl font-bold tracking-tight text-foreground">Developer Settings</h2>
          </div>
          <Badge variant="outline" className="text-[10px] font-mono border-amber-500/30 text-amber-500 bg-amber-500/5 uppercase">
            Internal / Testing
          </Badge>
        </div>
        <p className="text-xs text-muted-foreground leading-relaxed max-w-2xl">
          Diagnostic overrides for testing Vox lifecycle, transitions, and onboarding workflows.
          <strong className="text-foreground ml-1">These switches do not delete your saved notes, scribbles, or authentication credentials.</strong>
        </p>
      </div>

      {/* Meeting reminder smoke test */}
      <div className="p-5 rounded-lg border border-border/80 bg-card/60 backdrop-blur-xs space-y-3">
        <div className="space-y-1">
          <div className="flex items-center gap-2">
            <Bell className="w-4 h-4 text-muted-foreground" />
            <h3 className="text-sm font-semibold text-foreground">Meeting reminders</h3>
          </div>
          <p className="text-xs text-muted-foreground leading-relaxed max-w-2xl">
            There are three, and they are different messages with different actions rather than
            one message at three times. Each button raises its kind through the same path a real
            one takes — the reminder window, not an in-app panel and not a Windows toast.
            Reminders otherwise only fire in a few specific minutes around a meeting, which made
            &ldquo;nothing appeared&rdquo; and &ldquo;nothing was due&rdquo; impossible to tell
            apart.
          </p>
          <div className="pt-1 text-[11px] text-muted-foreground/80">
            Nothing is written and no calendar is read: the sample meeting is invented here.
          </div>
        </div>

        <ul className="space-y-2">
          {REMINDER_KINDS.map((entry) => (
            <li
              key={entry.kind}
              className="flex items-start justify-between gap-4 rounded-md border border-border/60 px-3 py-2"
            >
              <div className="min-w-0">
                <p className="text-xs font-medium text-foreground">
                  <span className="font-mono text-[11px] text-muted-foreground mr-2">
                    {entry.offset}
                  </span>
                  {entry.label}
                </p>
                <p className="text-[11px] text-muted-foreground mt-0.5">{entry.description}</p>
              </div>
              <Button
                variant="outline"
                size="sm"
                onClick={() => void handleTestReminder([entry.kind])}
                className="shrink-0 text-xs"
              >
                Send
              </Button>
            </li>
          ))}
        </ul>

        <div className="flex items-center gap-2">
          <Button
            variant="secondary"
            size="sm"
            onClick={() => void handleTestReminder(REMINDER_KINDS.map((entry) => entry.kind))}
            className="gap-2 text-xs"
          >
            <Bell className="w-3.5 h-3.5" />
            Send all three
          </Button>
          {reminderResult && (
            <p role="status" className="text-[11px] text-muted-foreground">
              {reminderResult}
            </p>
          )}
        </div>
      </div>

      {/* Onboarding Replay Override Section */}
      <div className="p-5 rounded-lg border border-border/80 bg-card/60 backdrop-blur-xs space-y-4">
        <div className="flex items-start justify-between gap-4">
          <div className="space-y-1">
            <div className="flex items-center gap-2">
              <h3 className="text-sm font-semibold text-foreground">Show onboarding on every launch</h3>
            </div>
            <p className="text-xs text-muted-foreground leading-relaxed max-w-xl">
              Development testing only. Replays onboarding every time Vox starts without deleting your saved profile or data.
            </p>
            <div className="pt-1 text-[11px] text-muted-foreground/80">
              When enabled, Vox will present the 2-step onboarding modal (Personalization name prompt and Google/Local selection) on every startup for iterative UX testing.
            </div>
          </div>

          <div className="flex items-center gap-2 shrink-0 pt-0.5">
            {saving && <RefreshCw className="w-3.5 h-3.5 animate-spin text-muted-foreground" />}
            <Switch
              checked={devSettings.force_onboarding_on_launch}
              onCheckedChange={handleToggleForceOnboarding}
              disabled={loading || saving}
            />
          </div>
        </div>
      </div>
    </div>
  );
};
