import React from 'react';
import {
  AlertTriangle,
  CalendarDays,
  Check,
  Loader2,
  Plus,
  RefreshCw,
  Trash2,
} from 'lucide-react';

import { Button } from '@/components/ui/button';
import { Card } from '@/components/ui/card';
import { Switch } from '@/components/ui/switch';
import * as calendar from '@/lib/calendar';
import type { CalendarAccount } from '@/types/calendar';

/**
 * Google Calendar, as many accounts as the user keeps.
 *
 * The list is the feature. Work lives in one Google account and life in
 * another, the two never merge, and "what am I doing today" is the union of
 * them — so this page is built around adding a second account being normal
 * rather than exceptional, and around each one being switchable without being
 * signed out.
 *
 * Read-only access is stated rather than implied. It is the difference between
 * a reasonable ask and one people decline, and Vox genuinely has no reason to
 * be able to change a calendar.
 */
export const CalendarSettingsView: React.FC = () => {
  const [accounts, setAccounts] = React.useState<CalendarAccount[]>([]);
  const [loading, setLoading] = React.useState(true);
  const [connecting, setConnecting] = React.useState(false);
  const [syncing, setSyncing] = React.useState(false);
  const [error, setError] = React.useState<string | null>(null);

  const refresh = React.useCallback(async () => {
    try {
      setAccounts((await calendar.listCalendarAccounts()) ?? []);
    } catch {
      setAccounts([]);
    } finally {
      setLoading(false);
    }
  }, []);

  React.useEffect(() => {
    void refresh();
  }, [refresh]);

  const message = (err: unknown) =>
    typeof err === 'object' && err !== null && 'message' in err
      ? String((err as { message: unknown }).message)
      : String(err);

  const connect = async () => {
    setConnecting(true);
    setError(null);
    try {
      setAccounts((await calendar.connectCalendarAccount()) ?? []);
      await sync();
    } catch (err) {
      setError(message(err));
    } finally {
      setConnecting(false);
    }
  };

  const sync = async () => {
    setSyncing(true);
    try {
      setAccounts((await calendar.syncCalendars()) ?? []);
    } catch (err) {
      setError(message(err));
    } finally {
      setSyncing(false);
    }
  };

  const disconnect = async (email: string) => {
    try {
      setAccounts((await calendar.disconnectCalendarAccount(email)) ?? []);
    } catch (err) {
      setError(message(err));
    }
  };

  const toggle = async (email: string, enabled: boolean) => {
    try {
      setAccounts((await calendar.setCalendarAccountEnabled(email, enabled)) ?? []);
    } catch (err) {
      setError(message(err));
    }
  };

  return (
    <div className="space-y-4">
      <div>
        <h2 className="text-base font-semibold text-foreground">Calendar</h2>
        <p className="text-xs text-muted-foreground mt-1 leading-relaxed max-w-2xl">
          Vox reads your Google Calendars so a recording can carry the meeting&apos;s real name
          and guest list, and so the day shows which meetings have notes. Connect as many
          accounts as you use — work and personal are merged into one day rather than kept apart.
          Access is <span className="text-foreground">read-only</span>: Vox cannot create, change
          or delete anything in your calendar.
        </p>
      </div>

      {error && (
        <p className="flex items-start gap-2 text-xs text-destructive">
          <AlertTriangle className="w-3.5 h-3.5 shrink-0 mt-0.5" />
          <span>{error}</span>
        </p>
      )}

      {loading ? (
        <p className="text-xs text-muted-foreground">Loading accounts…</p>
      ) : accounts.length === 0 ? (
        <Card className="p-6 text-center">
          <CalendarDays className="w-6 h-6 text-muted-foreground mx-auto mb-2" />
          <p className="text-sm font-medium text-foreground">No calendars connected</p>
          <p className="text-xs text-muted-foreground mt-1 max-w-sm mx-auto leading-relaxed">
            Without one, meetings are named by the time they were recorded and Vox has no way to
            know who was invited.
          </p>
          <Button onClick={() => void connect()} disabled={connecting} className="mt-4 gap-2">
            {connecting ? (
              <Loader2 className="w-3.5 h-3.5 animate-spin" />
            ) : (
              <Plus className="w-3.5 h-3.5" />
            )}
            {connecting ? 'Waiting for Google…' : 'Connect a Google account'}
          </Button>
        </Card>
      ) : (
        <>
          <Card className="divide-y divide-border">
            {accounts.map((account) => (
              <div key={account.email} className="flex items-center gap-3 p-3">
                <Switch
                  checked={account.enabled}
                  onCheckedChange={(next) => void toggle(account.email, next)}
                  aria-label={`Show events from ${account.email}`}
                />
                <div className="min-w-0 flex-1">
                  <p className="text-sm font-medium text-foreground truncate">
                    {account.display_name || account.email}
                  </p>
                  <p className="text-[11px] text-muted-foreground truncate">
                    {account.display_name ? `${account.email} · ` : ''}
                    {account.last_error ? (
                      <span className="text-destructive">{account.last_error}</span>
                    ) : account.last_synced_at ? (
                      <>Synced {new Date(account.last_synced_at).toLocaleString()}</>
                    ) : (
                      'Not synced yet'
                    )}
                  </p>
                </div>
                <Button
                  size="icon"
                  variant="ghost"
                  onClick={() => void disconnect(account.email)}
                  aria-label={`Disconnect ${account.email}`}
                  title="Disconnect. This removes its events too."
                  className="h-8 w-8 shrink-0"
                >
                  <Trash2 className="w-4 h-4" />
                </Button>
              </div>
            ))}
          </Card>

          <div className="flex items-center gap-2">
            <Button
              variant="outline"
              size="sm"
              onClick={() => void connect()}
              disabled={connecting}
              className="gap-2 text-xs"
            >
              {connecting ? (
                <Loader2 className="w-3.5 h-3.5 animate-spin" />
              ) : (
                <Plus className="w-3.5 h-3.5" />
              )}
              Add another account
            </Button>
            <Button
              variant="ghost"
              size="sm"
              onClick={() => void sync()}
              disabled={syncing}
              className="gap-2 text-xs"
            >
              {syncing ? (
                <Loader2 className="w-3.5 h-3.5 animate-spin" />
              ) : (
                <RefreshCw className="w-3.5 h-3.5" />
              )}
              {syncing ? 'Syncing…' : 'Sync now'}
            </Button>
          </div>

          <p className="flex items-start gap-2 text-[11px] text-muted-foreground leading-relaxed">
            <Check className="w-3.5 h-3.5 shrink-0 mt-0.5 text-emerald-600 dark:text-emerald-400" />
            <span>
              Events are cached on this machine so the day renders without waiting for Google.
              Turning an account off hides its events and keeps it signed in; disconnecting
              removes its events and its credentials.
            </span>
          </p>
        </>
      )}
    </div>
  );
};
