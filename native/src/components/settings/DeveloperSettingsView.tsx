import React, { useState, useEffect } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { DeveloperSettings } from '../../types';
import { Terminal, RefreshCw, Bell, SearchCode } from 'lucide-react';
import { Button } from '@/components/ui/button';
import type { ConferencingWindowMatch, ReminderKind } from '@/types/meetings';
import { Switch } from '@/components/ui/switch';
import { Badge } from '@/components/ui/badge';

/**
 * Every kind of meeting reminder, with when it fires and what it is for.
 *
 * The list is the point. Three separate messages, each with its own actions,
 * is a design decision that stays invisible until one of them turns up — and
 * the one that matters most, a meeting running with nothing being captured,
 * is the failure this app exists to prevent and the only one that is silent
 * by nature.
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
  const [detected, setDetected] = useState<ConferencingWindowMatch[] | null>(null);

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

  /**
   * What window detection can see right now.
   *
   * The `detected` reminder is the one kind no calendar can vouch for, so
   * &ldquo;nothing is open&rdquo; and &ldquo;detection is blind on this
   * machine&rdquo; are otherwise the same silence. Rendered in place rather
   * than in a dialog: the result is read against what is actually on screen,
   * and a modal would cover the very windows being counted.
   */
  const handleCheckDetection = async () => {
    setReminderResult(null);
    try {
      setDetected(await invoke<ConferencingWindowMatch[]>('debug_detect_conferencing_windows'));
    } catch (err) {
      console.error('Failed to run window detection:', err);
      setDetected(null);
      setReminderResult(String(err));
    }
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
            one takes — a card in a window of its own, above whatever you are working in, never
            an in-app panel and never a Windows toast. A real one fires only in the few specific
            minutes around a meeting, which otherwise makes &ldquo;nothing appeared&rdquo; and
            &ldquo;nothing was due&rdquo; impossible to tell apart.
          </p>
          <div className="pt-1 text-[11px] text-muted-foreground/80">
            Nothing is written and no calendar is read: the sample meeting is invented here.
            Window detection, below, only lists what is already on screen.
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

        <div className="pt-1">
          <Button
            variant="outline"
            size="sm"
            onClick={() => void handleCheckDetection()}
            className="text-xs h-8 gap-1.5 border-purple-500/30 hover:bg-purple-500/10 hover:text-purple-500"
          >
            <SearchCode className="w-3.5 h-3.5" />
            Check window detection
          </Button>
        </div>

        {reminderResult && (
          <p role="status" className="text-[11px] text-muted-foreground">
            {reminderResult}
          </p>
        )}

        {detected && (
          <div role="status" className="space-y-1.5">
            <p className="text-[11px] font-medium text-foreground">
              {detected.length === 0
                ? 'No conferencing windows on screen.'
                : `${detected.length} conferencing ${
                    detected.length === 1 ? 'window' : 'windows'
                  } on screen:`}
            </p>
            {detected.map((match) => (
              <div
                key={`${match.provider}:${match.raw_title}`}
                className="flex items-start justify-between gap-3 p-2 rounded-md border border-border/60 bg-background/50"
              >
                <div className="min-w-0">
                  {/* A window title is somebody else's text. Shown, never
                      interpreted — `rules/untrusted-input.md`. */}
                  <p className="text-[11px] font-medium text-foreground truncate">{match.title}</p>
                  <p className="text-[10px] font-mono text-muted-foreground truncate">
                    {match.raw_title}
                  </p>
                  {/* The answer to "it sees the call and still says nothing".
                      Silence from a gate and silence from a broken feature are
                      otherwise the same silence. */}
                  {match.blocked_by ? (
                    <p className="text-[10px] text-amber-600 dark:text-amber-400 mt-1">
                      No reminder: {match.blocked_by}
                    </p>
                  ) : (
                    <p className="text-[10px] text-emerald-600 dark:text-emerald-400 mt-1">
                      Would raise a reminder.
                    </p>
                  )}
                </div>
                <Badge variant="outline" className="text-[10px] shrink-0 font-mono">
                  {match.provider} · {match.source} · {match.confidence.toFixed(2)}
                </Badge>
              </div>
            ))}
          </div>
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
