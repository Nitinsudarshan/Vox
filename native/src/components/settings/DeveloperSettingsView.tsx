import React, { useState, useEffect } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { DeveloperSettings } from '../../types';
import { Terminal, RefreshCw, Bell } from 'lucide-react';
import { Button } from '@/components/ui/button';
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
      'A scheduled meeting is about to start. Raised five minutes ahead of anything on a connected calendar.',
  },
  {
    kind: 'unrecorded',
    offset: 't+5',
    label: 'Running unrecorded',
    description:
      'A scheduled meeting has started and nothing is recording it — and the call is actually open on screen.',
  },
  {
    kind: 'detected',
    offset: 'ad-hoc',
    label: 'Detected call',
    description:
      'A conferencing window is open that the calendar knows nothing about. Window detection is a Windows capability.',
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

  /**
   * Raises one mock reminder through `NotificationService::show` itself.
   *
   * The real path, not a preview of it: a shortcut here would prove nothing
   * about the card users actually see.
   */
  const handleTestReminder = async (kind: ReminderKind) => {
    setReminderResult(null);
    try {
      await invoke('trigger_mock_meeting_reminder', { kind });
      setReminderResult('Raised. The card is in the top-right corner of your screen.');
    } catch (err) {
      console.error(`Failed to trigger a mock ${kind} reminder:`, err);
      setReminderResult(String(err));
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
                onClick={() => void handleTestReminder(entry.kind)}
                className="shrink-0 text-xs"
              >
                Send
              </Button>
            </li>
          ))}
        </ul>

        {reminderResult && (
          <p role="status" className="text-[11px] text-muted-foreground">
            {reminderResult}
          </p>
        )}
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
