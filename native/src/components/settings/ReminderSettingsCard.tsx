import React from 'react';
import { AlertTriangle, Bell } from 'lucide-react';

import { Card } from '@/components/ui/card';
import { Switch } from '@/components/ui/switch';
import * as meetings from '@/lib/meetings';
import {
  REMINDER_LEAD_CHOICES,
  type ReminderSettings,
} from '@/types/meetings';

/** What each lead time is called. `0` is the start itself. */
const LEAD_LABELS: Record<number, string> = {
  15: '15 min before',
  10: '10 min before',
  5: '5 min before',
  1: '1 min before',
  0: 'When it starts',
};

const DEFAULTS: ReminderSettings = {
  enabled: true,
  lead_minutes: [5, 0],
  only_with_link: false,
  include_declined: false,
  nudge_when_not_recording: true,
};

/**
 * When Vox says a meeting is about to start.
 *
 * Its own command rather than a whole-`AppSettings` round trip, for the reason
 * the device picker has one: a checkbox that carries the entire settings
 * document back lets a stale copy overwrite whatever else changed meanwhile.
 *
 * The lead times are a fixed set rather than a minutes box. These are the
 * intervals people mean, and a free-form field invites "0.5" and "90" — one of
 * which is not a number of minutes and the other of which is a different
 * feature. Picking several is normal and does not produce several
 * notifications: the backend treats them as buckets and announces a meeting
 * once, in the tightest one it has reached.
 */
export const ReminderSettingsCard: React.FC = () => {
  const [settings, setSettings] = React.useState<ReminderSettings>(DEFAULTS);
  const [loaded, setLoaded] = React.useState(false);
  const [error, setError] = React.useState<string | null>(null);

  React.useEffect(() => {
    void meetings
      .getReminderSettings()
      .then((stored) => setSettings({ ...DEFAULTS, ...(stored ?? {}) }))
      .catch(() => undefined)
      .finally(() => setLoaded(true));
  }, []);

  const update = async (patch: Partial<ReminderSettings>) => {
    const next = { ...settings, ...patch };
    // Optimistic: a switch must not lag the click. A failed write costs the
    // preference sticking, and the message below says so.
    setSettings(next);
    try {
      const saved = await meetings.saveReminderSettings(next);
      if (saved) setSettings(saved);
      setError(null);
    } catch (err) {
      setError(meetings.meetingErrorMessage(err));
    }
  };

  const toggleLead = (lead: number) => {
    const chosen = settings.lead_minutes.includes(lead)
      ? settings.lead_minutes.filter((value) => value !== lead)
      : [...settings.lead_minutes, lead];
    void update({ lead_minutes: chosen });
  };

  if (!loaded) return null;

  return (
    <Card className="p-4 space-y-3">
      <div className="flex items-start justify-between gap-4">
        <div className="min-w-0">
          <p className="text-sm font-medium text-foreground flex items-center gap-2">
            <Bell className="w-3.5 h-3.5 text-muted-foreground" />
            Meeting reminders
          </p>
          <p className="text-xs text-muted-foreground mt-1 leading-relaxed max-w-xl">
            Vox says something before a meeting on a connected calendar starts. Each meeting is
            announced once, at the closest lead time it reaches — picking several does not mean
            several notifications. The reminder appears in a small window of its own, above
            whatever you are working in, so it reaches you without Vox being on screen. All-day
            events, meetings already well under way, and ones you have declined are never
            announced.
          </p>
        </div>
        <Switch
          checked={settings.enabled}
          onCheckedChange={(enabled) => void update({ enabled })}
          aria-label="Remind me before a meeting starts"
        />
      </div>

      {error && (
        <p className="flex items-start gap-2 text-xs text-destructive">
          <AlertTriangle className="w-3.5 h-3.5 shrink-0 mt-0.5" />
          <span>{error}</span>
        </p>
      )}

      {settings.enabled && (
        <>
          <fieldset className="space-y-1.5">
            <legend className="text-xs font-medium text-foreground">Tell me</legend>
            <div className="flex flex-wrap gap-1.5">
              {REMINDER_LEAD_CHOICES.map((lead) => {
                const chosen = settings.lead_minutes.includes(lead);
                return (
                  <button
                    key={lead}
                    type="button"
                    role="switch"
                    aria-checked={chosen}
                    onClick={() => toggleLead(lead)}
                    className={`h-7 px-2.5 rounded-md border text-xs transition-colors cursor-pointer ${
                      chosen
                        ? 'border-primary bg-primary/10 text-foreground'
                        : 'border-border text-muted-foreground hover:text-foreground'
                    }`}
                  >
                    {LEAD_LABELS[lead]}
                  </button>
                );
              })}
            </div>
            {settings.lead_minutes.length === 0 && (
              <p className="text-[11px] text-amber-600 dark:text-amber-400">
                Nothing chosen, so nothing will be announced.
              </p>
            )}
          </fieldset>

          <ReminderToggle
            label="Only meetings with a video link"
            description="Leaves out rooms and phone calls."
            checked={settings.only_with_link}
            onChange={(only_with_link) => void update({ only_with_link })}
          />
          <ReminderToggle
            label="Include meetings I declined"
            description="Off by default — declining says you are not going."
            checked={settings.include_declined}
            onChange={(include_declined) => void update({ include_declined })}
          />
          <ReminderToggle
            label="Tell me when a meeting is not being recorded"
            description="Five minutes into a meeting with nothing being captured. The failure this app exists to prevent, and the only one that is silent on its own."
            checked={settings.nudge_when_not_recording}
            onChange={(nudge_when_not_recording) => void update({ nudge_when_not_recording })}
          />
        </>
      )}
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
