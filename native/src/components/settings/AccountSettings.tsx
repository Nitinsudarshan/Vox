import React, { useState, useEffect } from 'react';
import { invoke } from '@tauri-apps/api/core';
import {
  RelayAccount,
  RelayProfile,
  InstallationInfo,
  UpdateInfo,
  AppSettings,
} from '../../types';
import {
  User,
  ShieldCheck,
  HardDrive,
  CheckCircle2,
  RefreshCw,
  Copy,
  Check,
  LogOut,
  Sparkles,
  AlertCircle,
  Laptop,
  Save,
  Info,
  X,
} from 'lucide-react';
import { Button } from '@/components/ui/button';
import { Input } from '@/components/ui/input';
import { Badge } from '@/components/ui/badge';
import { Switch } from '@/components/ui/switch';

interface AccountSettingsProps {
  settings: AppSettings;
  onUpdateSettings: (updater: (prev: AppSettings) => AppSettings) => void;
  onOpenExplanation?: () => void;
}

export const AccountSettings: React.FC<AccountSettingsProps> = ({
  settings,
  onUpdateSettings,
  onOpenExplanation,
}) => {
  const [account, setAccount] = useState<RelayAccount | null>(null);
  const [profile, setProfile] = useState<RelayProfile | null>(null);
  const [installation, setInstallation] = useState<InstallationInfo | null>(null);
  const [updateInfo, setUpdateInfo] = useState<UpdateInfo | null>(null);
  const [displayNameInput, setDisplayNameInput] = useState('');
  const [savingName, setSavingName] = useState(false);
  const [savedNameSuccess, setSavedNameSuccess] = useState(false);
  const [loading, setLoading] = useState(true);
  const [signingIn, setSigningIn] = useState(false);
  const [checkingUpdate, setCheckingUpdate] = useState(false);
  const [showSignOutConfirm, setShowSignOutConfirm] = useState(false);
  const [copiedId, setCopiedId] = useState(false);
  const [copiedDiagnostics, setCopiedDiagnostics] = useState(false);
  const [showHybridModal, setShowHybridModal] = useState(false);
  const [errorMsg, setErrorMsg] = useState<string | null>(null);

  const loadData = async () => {
    try {
      setLoading(true);
      const [acc, prof, inst] = await Promise.all([
        invoke<RelayAccount>('get_account_state'),
        invoke<RelayProfile>('get_relay_profile'),
        invoke<InstallationInfo>('get_installation_info'),
      ]);
      setAccount(acc);
      setProfile(prof);
      setInstallation(inst);
      if (prof?.display_name) {
        setDisplayNameInput(prof.display_name);
      }
    } catch (err) {
      console.error('Failed to load account/installation state:', err);
    } finally {
      setLoading(false);
    }
  };

  useEffect(() => {
    loadData();
  }, []);

  const handleSaveDisplayName = async (e?: React.FormEvent) => {
    if (e) e.preventDefault();
    const trimmed = displayNameInput.trim();
    if (!trimmed) return;

    try {
      setSavingName(true);
      const updated = await invoke<RelayProfile>('update_profile_display_name', {
        displayName: trimmed,
      });
      setProfile(updated);
      window.dispatchEvent(new CustomEvent('relay-profile-changed', { detail: updated }));
      setSavedNameSuccess(true);
      setTimeout(() => setSavedNameSuccess(false), 2500);
    } catch (err) {
      console.error('Failed to update display name:', err);
    } finally {
      setSavingName(false);
    }
  };

  const handleSignIn = async () => {
    try {
      setSigningIn(true);
      setErrorMsg(null);
      const acc = await invoke<RelayAccount>('start_google_sign_in');
      setAccount(acc);
      const updatedProfile = await invoke<RelayProfile>('get_relay_profile');
      setProfile(updatedProfile);
      if (updatedProfile.display_name) {
        setDisplayNameInput(updatedProfile.display_name);
      }
      window.dispatchEvent(new CustomEvent('relay-account-changed', { detail: acc }));
      window.dispatchEvent(new CustomEvent('relay-profile-changed', { detail: updatedProfile }));
      if (onOpenExplanation) {
        onOpenExplanation();
      }
    } catch (err: unknown) {
      console.error('Google Sign-In failed:', err);
      const raw = typeof err === 'string' ? err : (err as { message?: string })?.message || '';
      if (raw.toLowerCase().includes('configured') || raw.toLowerCase().includes('not configured')) {
        setErrorMsg('Sign-in with Google is not configured for this Vox installation.');
      } else {
        setErrorMsg('Google Sign-In could not be completed. Please try again.');
      }
    } finally {
      setSigningIn(false);
    }
  };

  const handleSignOut = async () => {
    try {
      setErrorMsg(null);
      const acc = await invoke<RelayAccount>('sign_out_account');
      setAccount(acc);
      const updatedProfile = await invoke<RelayProfile>('get_relay_profile');
      setProfile(updatedProfile);
      window.dispatchEvent(new CustomEvent('relay-account-changed', { detail: acc }));
      window.dispatchEvent(new CustomEvent('relay-profile-changed', { detail: updatedProfile }));
      setShowSignOutConfirm(false);
    } catch (err: unknown) {
      console.error('Sign-out failed:', err);
      const msg = typeof err === 'string' ? err : (err as { message?: string })?.message || 'Sign-out failed.';
      setErrorMsg(msg);
    }
  };

  const handleCheckUpdates = async () => {
    try {
      setCheckingUpdate(true);
      const info = await invoke<UpdateInfo>('check_for_updates');
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
    onUpdateSettings((prev) => ({
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

  return (
    <div className="space-y-6 pt-6 border-t border-border/60 animate-in fade-in duration-200">
      {/* Header & Invariant Statement */}
      <div className="flex items-center justify-between">
        <div>
          <p className="font-mono text-[10px] font-bold text-muted-foreground uppercase tracking-widest mb-1">
            ACCOUNT & IDENTITY
          </p>
          <h2 className="text-lg font-bold text-foreground">Profile, Cloud Connection & Diagnostics</h2>
        </div>
        <Badge variant="outline" className="text-[10px] font-mono border-primary/30 text-primary bg-primary/5 uppercase">
          {account?.authenticated ? 'Google Connected' : 'Local Mode'}
        </Badge>
      </div>

      {errorMsg && (
        <div className="p-3.5 rounded-lg border border-destructive/30 bg-destructive/10 text-destructive text-xs flex items-center justify-between gap-2.5">
          <div className="flex items-center gap-2">
            <AlertCircle className="w-4 h-4 shrink-0" />
            <p>{errorMsg}</p>
          </div>
          <button
            type="button"
            onClick={() => setErrorMsg(null)}
            className="p-1 hover:bg-destructive/20 rounded text-destructive cursor-pointer"
            aria-label="Dismiss error"
          >
            <X className="w-3.5 h-3.5" />
          </button>
        </div>
      )}

      {/* Responsive 3-Column Card Grid (Matching General Settings Style) */}
      <div className="grid grid-cols-1 md:grid-cols-2 xl:grid-cols-3 gap-4">
        {/* Card 1: Personalization (Display Name) */}
        <div className="p-4 rounded-lg border border-border bg-card space-y-4 flex flex-col justify-between">
          <div className="space-y-3">
            <div className="flex items-center gap-2">
              <User className="w-4 h-4 text-primary" />
              <div>
                <p className="text-xs font-semibold text-foreground">Personalization</p>
                <p className="text-[11px] text-muted-foreground">What Vox calls you locally</p>
              </div>
            </div>

            <form onSubmit={handleSaveDisplayName} className="space-y-2">
              <Input
                value={displayNameInput}
                onChange={(e) => setDisplayNameInput(e.target.value)}
                placeholder="Enter your name (e.g. Nitin)"
                className="h-8 text-xs bg-muted/40"
              />
              <div className="flex items-center justify-between">
                <p className="text-[10px] text-muted-foreground">Stored only on this device.</p>
                {savedNameSuccess && (
                  <Badge variant="secondary" className="text-[10px] gap-1 bg-emerald-500/10 text-emerald-500 border-emerald-500/20">
                    <Check className="w-3 h-3" />
                    <span>Saved</span>
                  </Badge>
                )}
              </div>
            </form>
          </div>

          <div className="pt-3 border-t border-border/60">
            <Button
              type="button"
              size="sm"
              variant="outline"
              className="w-full text-xs h-8 gap-1.5"
              onClick={handleSaveDisplayName}
              disabled={savingName || !displayNameInput.trim() || displayNameInput.trim() === profile?.display_name}
            >
              {savingName ? <RefreshCw className="w-3.5 h-3.5 animate-spin" /> : <Save className="w-3.5 h-3.5" />}
              <span>Save Name</span>
            </Button>
          </div>
        </div>

        {/* Card 2: Account Connection */}
        <div className="p-4 rounded-lg border border-border bg-card space-y-4 flex flex-col justify-between">
          <div className="space-y-3">
            <div className="flex items-center gap-2">
              <ShieldCheck className="w-4 h-4 text-primary" />
              <div>
                <p className="text-xs font-semibold text-foreground">Account Status</p>
                <p className="text-[11px] text-muted-foreground">Sync identity & cloud features</p>
              </div>
            </div>

            <div className="p-2.5 rounded-lg bg-muted/40 border border-border/80 flex items-center gap-3">
              {account?.authenticated && account.profile_image ? (
                <img
                  src={account.profile_image}
                  alt="Profile"
                  referrerPolicy="no-referrer"
                  className="w-8 h-8 rounded-full border border-primary/30 object-cover shrink-0"
                  onError={(e) => { (e.currentTarget as HTMLImageElement).style.display = 'none'; }}
                />
              ) : (
                <div className="w-8 h-8 rounded-full bg-primary/10 border border-primary/20 text-primary flex items-center justify-center text-xs font-bold shrink-0">
                  {profile?.display_name && profile.display_name !== 'Local User'
                    ? profile.display_name.charAt(0).toUpperCase()
                    : <User className="w-4 h-4" />}
                </div>
              )}
              <div className="min-w-0 overflow-hidden">
                <p className="text-xs font-medium text-foreground truncate">
                  {account?.authenticated ? account.display_name || account.email : 'Local User'}
                </p>
                <p className="text-[10px] font-mono text-muted-foreground truncate">
                  {account?.authenticated ? account.email : '100% offline local mode'}
                </p>
              </div>
            </div>

            {showSignOutConfirm && (
              <div className="p-2.5 rounded-lg border border-destructive/40 bg-destructive/5 space-y-2">
                <p className="text-[11px] text-muted-foreground leading-snug">
                  Disconnect account? Local data remains untouched on this device.
                </p>
                <div className="flex items-center gap-1.5">
                  <Button size="sm" variant="destructive" className="text-xs h-7 px-2.5" onClick={handleSignOut}>
                    Confirm
                  </Button>
                  <Button size="sm" variant="ghost" className="text-xs h-7 px-2.5" onClick={() => setShowSignOutConfirm(false)}>
                    Cancel
                  </Button>
                </div>
              </div>
            )}
          </div>

          <div className="pt-3 border-t border-border/60">
            {account?.authenticated ? (
              <Button
                type="button"
                variant="outline"
                size="sm"
                className="w-full text-xs h-8 text-destructive hover:text-destructive border-destructive/30 gap-1.5"
                onClick={() => setShowSignOutConfirm(true)}
              >
                <LogOut className="w-3.5 h-3.5" />
                <span>Disconnect Account</span>
              </Button>
            ) : (
              <Button
                type="button"
                size="sm"
                className="w-full text-xs h-8 font-semibold gap-2 bg-primary hover:bg-primary/90 text-primary-foreground shadow-xs"
                onClick={handleSignIn}
                disabled={signingIn}
              >
                {signingIn ? <RefreshCw className="w-3.5 h-3.5 animate-spin" /> : (
                  <svg className="w-3.5 h-3.5" viewBox="0 0 24 24">
                    <path fill="currentColor" d="M22.56 12.25c0-.78-.07-1.53-.2-2.25H12v4.26h5.92c-.26 1.37-1.04 2.53-2.21 3.31v2.77h3.57c2.08-1.92 3.28-4.74 3.28-8.09z" />
                    <path fill="currentColor" d="M12 23c2.97 0 5.46-.98 7.28-2.66l-3.57-2.77c-.98.66-2.23 1.06-3.71 1.06-2.86 0-5.29-1.93-6.16-4.53H2.18v2.84C3.99 20.53 7.7 23 12 23z" />
                    <path fill="currentColor" d="M5.84 14.09c-.22-.66-.35-1.36-.35-2.09s.13-1.43.35-2.09V7.06H2.18C1.43 8.55 1 10.22 1 12s.43 3.45 1.18 4.94l2.85-2.22.81-.63z" />
                    <path fill="currentColor" d="M12 5.38c1.62 0 3.06.56 4.21 1.64l3.15-3.15C17.45 2.09 14.97 1 12 1 7.7 1 3.99 3.47 2.18 7.06l3.66 2.84c.87-2.6 3.3-4.52 6.16-4.52z" />
                  </svg>
                )}
                <span>{signingIn ? 'Connecting…' : 'Sign in with Google'}</span>
              </Button>
            )}
          </div>
        </div>

        {/* Card 3: Operating Mode & Hybrid */}
        <div className="p-4 rounded-lg border border-border bg-card space-y-4 flex flex-col justify-between">
          <div className="space-y-3">
            <div className="flex items-center gap-2">
              <HardDrive className="w-4 h-4 text-primary" />
              <div>
                <p className="text-xs font-semibold text-foreground">Operating Mode</p>
                <p className="text-[11px] text-muted-foreground">Local-first data governance</p>
              </div>
            </div>

            <div className="p-2.5 rounded-lg bg-muted/40 border border-border/80 space-y-1.5">
              <div className="flex items-center justify-between">
                <span className="text-[10px] uppercase font-mono text-muted-foreground">Storage Model</span>
                <Badge variant="outline" className="text-[10px] font-mono text-emerald-600 dark:text-emerald-400 border-emerald-500/30">
                  Local Only
                </Badge>
              </div>
              <p className="text-[11px] text-muted-foreground leading-snug">
                Notes, vectors, and voice recordings reside exclusively on this computer.
              </p>
            </div>
          </div>

          <div className="pt-3 border-t border-border/60">
            <Button
              type="button"
              size="sm"
              variant="outline"
              className="w-full text-xs h-8 gap-1.5"
              onClick={() => setShowHybridModal(true)}
            >
              <Sparkles className="w-3.5 h-3.5 text-primary" />
              <span>Explore Hybrid Mode</span>
            </Button>
          </div>
        </div>

        {/* Card 4: Application Version & Updates */}
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

        {/* Card 5: Installation Identity */}
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

        {/* Card 6: Diagnostics & Bug Reporting */}
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

      {/* Hybrid Mode Preview Modal */}
      {showHybridModal && (
        <div className="fixed inset-0 z-50 flex items-center justify-center bg-background/80 backdrop-blur-xs p-4 animate-in fade-in-50">
          <div className="w-full max-w-md bg-card border border-border rounded-lg p-6 shadow-2xl space-y-6">
            <div className="text-center space-y-2">
              <div className="w-12 h-12 rounded-lg bg-primary/10 border border-primary/20 text-primary flex items-center justify-center mx-auto mb-2">
                <Sparkles className="w-6 h-6" />
              </div>
              <h3 className="text-lg font-bold text-foreground">Vox Hybrid</h3>
              <p className="text-xs text-muted-foreground max-w-xs mx-auto">
                Local-first speed with selective, user-controlled cloud synchronization.
              </p>
            </div>

            <div className="space-y-2.5 text-xs text-muted-foreground">
              <div className="flex items-center gap-2.5 p-2 rounded-lg bg-muted/40">
                <CheckCircle2 className="w-4 h-4 text-emerald-500 shrink-0" />
                <span>Full Google Calendar real-time synchronization</span>
              </div>
              <div className="flex items-center gap-2.5 p-2 rounded-lg bg-muted/40">
                <CheckCircle2 className="w-4 h-4 text-emerald-500 shrink-0" />
                <span>Cross-device access across Windows and macOS</span>
              </div>
              <div className="flex items-center gap-2.5 p-2 rounded-lg bg-muted/40">
                <CheckCircle2 className="w-4 h-4 text-emerald-500 shrink-0" />
                <span>Encrypted cloud backup with granular sync controls</span>
              </div>
              <div className="flex items-center gap-2.5 p-2 rounded-lg bg-muted/40">
                <CheckCircle2 className="w-4 h-4 text-emerald-500 shrink-0" />
                <span>Zero manual migrations: seamless transition from Local</span>
              </div>
            </div>

            <div className="pt-2 border-t border-border/40 flex items-center justify-between">
              <span className="text-xs text-muted-foreground">Coming in Hybrid Release</span>
              <Button size="sm" onClick={() => setShowHybridModal(false)}>
                Got it
              </Button>
            </div>
          </div>
        </div>
      )}
    </div>
  );
};
