import React, { useState, useEffect } from 'react';
import { HomePage } from './components/home/HomePage';
import { VoiceNotePage } from './components/voicenotes/VoiceNotePage';
import { MeetingsPage } from './components/meetings/MeetingsPage';
import { ScribbleViewer } from './components/scribble/ScribbleViewer';
import { FilesPage } from './components/files/FilesPage';
import { CapturesPage } from './components/captures/CapturesPage';
import { KnowledgeGraphPage } from './components/knowledge/KnowledgeGraphPage';
import { TodosPage } from './components/todos/TodosPage';

import { ProviderSettings, type SettingsSection } from './components/settings/ProviderSettings';
import { DiagnosticsPage } from './components/diagnostics/DiagnosticsPage';
import { ThemeToggle } from './components/ThemeToggle';
import { ChangelogModal } from './components/common/ChangelogModal';
import { WelcomeModal } from './components/common/WelcomeModal';
import { AccountExplanationModal } from './components/common/AccountExplanationModal';
import { RelayAccount, RelayProfile, DeveloperSettings, AppSettings, MainTabType } from './types';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { NativeSidebar } from './components/common/NativeSidebar';
import { AppHeader } from './components/common/AppHeader';
import { CloseConfirmationDialog } from './components/common/CloseConfirmationDialog';
import { PageHeader } from './components/common/PageHeader';
import { TooltipProvider } from '@/components/ui/tooltip';
import type { CaptureMethod } from './components/captures/CaptureHubPage';

export type { MainTabType };

const TAB_LABELS: Record<MainTabType, string> = {
  home: 'Home',
  capture: 'Voice Notes',
  meetings: 'Meetings',
  scribble: 'Scribbles',
  todos: 'TODOs',
  graph: 'Knowledge Graph',
  files: 'Files & Docs',
  captures: 'Web Capture',
  diagnostics: 'Diagnostics',
  settings: 'Settings',
};


export const App: React.FC = () => {
  const [activeTab, setActiveTab] = useState<MainTabType>('home');
  const [settingsSection, setSettingsSection] = useState<SettingsSection | undefined>(undefined);
  /** Whether the privacy explanation is owed once onboarding closes. */
  const [explainAfterWelcome, setExplainAfterWelcome] = useState(false);
  /**
   * A capture mode requested from Home. Captures opens straight onto it, and it
   * is cleared by any other navigation so the Capture tab is not sticky.
   */
  const [captureMethod, setCaptureMethod] = useState<CaptureMethod | null>(null);
  /** A scribble the Knowledge Graph asked the workspace to reveal. */
  const [focusScribbleId, setFocusScribbleId] = useState<string | null>(null);
  const [sidebarOpen, setSidebarOpen] = useState(false);
  const [changelogOpen, setChangelogOpen] = useState(false);
  const [appVersion, setAppVersion] = useState<string>('0.9.0');
  const [account, setAccount] = useState<RelayAccount | null>(null);
  const [profile, setProfile] = useState<RelayProfile | null>(null);
  const [settings, setSettings] = useState<AppSettings | null>(null);
  const [welcomeOpen, setWelcomeOpen] = useState(false);
  const [explanationOpen, setExplanationOpen] = useState(false);
  const [closeDialogOpen, setCloseDialogOpen] = useState(false);

  const handleConfirmClose = async () => {
    setCloseDialogOpen(false);
    try {
      await invoke('close_app');
    } catch {
      window.close();
    }
  };

  const handleConfirmHide = async () => {
    setCloseDialogOpen(false);
    try {
      await invoke('hide_main_window');
    } catch {
      try {
        const { getCurrentWindow } = await import('@tauri-apps/api/window');
        await getCurrentWindow().hide();
      } catch (err) {
        console.warn('Failed to hide window:', err);
      }
    }
  };



  /**
   * Every tab change goes through here.
   *
   * A one-shot request from another surface — a capture mode from a Home card, a
   * scribble the graph wants revealed — is cleared on the next navigation, so
   * arriving at a surface from the sidebar never lands on someone else's intent.
   */
  const navigateTo = (
    tab: MainTabType,
    intent: {
      captureMethod?: CaptureMethod;
      focusScribbleId?: string;
      section?: SettingsSection;
    } = {},
  ) => {
    setActiveTab(tab);
    setCaptureMethod(intent.captureMethod ?? null);
    setFocusScribbleId(intent.focusScribbleId ?? null);
    if (tab === 'settings') {
      setSettingsSection(intent.section);
    }
  };

  const refreshAccountAndSettings = async () => {
    try {
      const [ver, acc, prof, devSetts, appSetts] = await Promise.all([
        invoke<string>('get_app_version'),
        invoke<RelayAccount>('get_account_state'),
        invoke<RelayProfile>('get_relay_profile'),
        invoke<DeveloperSettings>('get_developer_settings'),
        invoke<AppSettings>('get_settings'),
      ]);
      if (ver) setAppVersion(ver);
      if (acc) setAccount(acc);
      if (prof) setProfile(prof);
      if (appSetts) setSettings(appSetts);

      // Onboarding visibility: developer override forces replay, or first-run incomplete
      const shouldShowOnboarding = devSetts?.force_onboarding_on_launch || !prof?.onboarding_completed;
      if (shouldShowOnboarding) {
        setWelcomeOpen(true);
      }
    } catch (err) {
      console.warn('Could not load initial profile/settings:', err);
    }
  };

  useEffect(() => {
    refreshAccountAndSettings();

    // Listen for backend Tauri account, profile, settings, & navigation events
    const handleNavigate = (payload: unknown) => {
      if (typeof payload === 'string') {
        if (payload in TAB_LABELS) {
          setActiveTab(payload as MainTabType);
          setCaptureMethod(null);
          setFocusScribbleId(null);
          if (payload === 'settings') {
            setSettingsSection(undefined);
          }
        }
      } else if (payload && typeof payload === 'object') {
        const obj = payload as { tab?: MainTabType; section?: SettingsSection };
        if (obj.tab && obj.tab in TAB_LABELS) {
          setActiveTab(obj.tab);
          setCaptureMethod(null);
          setFocusScribbleId(null);
        }
        if (obj.section) {
          setSettingsSection(obj.section);
        }
      }
    };

    const unlistenNavigate = listen<unknown>('navigate-tab', (event) => {
      if (event.payload) {
        handleNavigate(event.payload);
      }
    });

    const unlistenAccount = listen<RelayAccount>('account-changed', (event) => {
      if (event.payload) {
        setAccount(event.payload);
      }
    });

    const unlistenProfile = listen<RelayProfile>('profile-changed', (event) => {
      if (event.payload) {
        setProfile(event.payload);
      }
    });

    const unlistenSettings = listen<AppSettings>('settings-changed', (event) => {
      if (event.payload) {
        setSettings(event.payload);
      }
    });

    // 3. Listen for DOM custom events
    const handleDomAccountChange = (e: Event) => {
      const customEvent = e as CustomEvent<RelayAccount>;
      if (customEvent.detail) {
        setAccount(customEvent.detail);
      }
    };

    const handleDomProfileChange = (e: Event) => {
      const customEvent = e as CustomEvent<RelayProfile>;
      if (customEvent.detail) {
        setProfile(customEvent.detail);
      }
    };

    const handleDomNavigate = (e: Event) => {
      const customEvent = e as CustomEvent<unknown>;
      if (customEvent.detail) {
        handleNavigate(customEvent.detail);
      }
    };

    window.addEventListener('relay-account-changed', handleDomAccountChange);
    window.addEventListener('relay-profile-changed', handleDomProfileChange);
    window.addEventListener('relay-navigate-tab', handleDomNavigate);

    return () => {
      unlistenNavigate.then((unlisten) => unlisten());
      unlistenAccount.then((unlisten) => unlisten());
      unlistenProfile.then((unlisten) => unlisten());
      unlistenSettings.then((unlisten) => unlisten());
      window.removeEventListener('relay-account-changed', handleDomAccountChange);
      window.removeEventListener('relay-profile-changed', handleDomProfileChange);
      window.removeEventListener('relay-navigate-tab', handleDomNavigate);
    };
  }, []);



  const handleWelcomeGoogle = async (displayName: string) => {
    try {
      await invoke('update_profile_display_name', { displayName });
      const acc = await invoke<RelayAccount>('start_google_sign_in');
      const updatedProfile = await invoke<RelayProfile>('complete_profile_onboarding', {
        displayName,
        accountMode: 'local',
      });
      setProfile(updatedProfile);
      setAccount(acc);
      // The modal stays open for the speech-setup step; it closes from
      // `onFinish`, and the privacy explanation follows that.
      setExplainAfterWelcome(true);
    } catch (err) {
      console.error('Failed to complete Google onboarding:', err);
      throw err;
    }
  };

  const handleWelcomeLocally = async (displayName: string) => {
    try {
      const updatedProfile = await invoke<RelayProfile>('complete_profile_onboarding', {
        displayName,
        accountMode: 'local',
      });
      setProfile(updatedProfile);
    } catch (err) {
      console.error('Failed to complete local onboarding:', err);
      throw err;
    }
  };

  const renderHeroHeader = () => {
    switch (activeTab) {
      case 'home': {
        const name = profile?.display_name && profile.display_name !== 'Local User'
          ? profile.display_name.split(' ')[0]
          : null;
        return (
          <PageHeader
            title={name ? 'Welcome back,' : 'Everything Vox'}
            highlightText={name ? `${name}.` : 'has captured.'}
            description="Start a capture, or pick up what you already said. Read directly from your vault."
            glowColor="emerald"
            compact
          />
        );
      }
      case 'capture':
      case 'meetings':
      case 'settings':
      case 'scribble':
      case 'todos':
      case 'graph':
        return null;
      case 'captures':
        return (
          <PageHeader
            title="Everything you capture,"
            highlightText="as text you own."
            description="Type or paste a thought, or open a page sent from the browser extension."
            glowColor="primary"
            compact
          />
        );
      case 'files':
        return (
          <PageHeader
            title="Imported"
            highlightText="documents & knowledge."
            description="PDF, Word, Markdown and Text files imported into your local vault."
            glowColor="primary"
            compact
          />
        );
      case 'diagnostics':
        return (
          <PageHeader
            title="Inspect & test"
            highlightText="Vox's engines."
            description="Real-time telemetry, audio & VAD inspection, STT accuracy tests, and LLM latency."
            glowColor="purple"
            compact
          />
        );
    }
  };

  return (
    <TooltipProvider delayDuration={200}>
      <div className="flex flex-col h-screen w-screen bg-background text-foreground overflow-hidden font-sans">
      {/* Borderless Product-Native Unified Header */}
      <AppHeader
        sidebarOpen={sidebarOpen}
        onToggleSidebar={() => setSidebarOpen(!sidebarOpen)}
        activeTab={activeTab}
        onNavigateHome={() => navigateTo('home')}
        onCloseClick={() => setCloseDialogOpen(true)}
      />

      {/* Main Workspace Surface: Sidebar + View Content */}
      <div className="flex flex-1 min-h-0 min-w-0 overflow-hidden">
        {/* Navigation Sidebar (sidebar-07 icon-collapsible pattern) */}
        <NativeSidebar
          isOpen={sidebarOpen}
          onToggle={() => setSidebarOpen(!sidebarOpen)}
          activeTab={activeTab}
          setActiveTab={(tab) => navigateTo(tab)}
          account={account}
          profile={profile}
          appVersion={appVersion}
          onOpenChangelog={() => setChangelogOpen(true)}
          onOpenWelcome={() => setWelcomeOpen(true)}
          onOpenExplanation={() => setExplanationOpen(true)}
        />

        {/* View Surface Container */}
        <main className="flex-1 min-w-0 p-4 md:p-6 overflow-y-auto flex flex-col bg-background">
          {renderHeroHeader()}

          {activeTab === 'home' && (
            <HomePage
              account={account}
              settings={settings}
              appVersion={appVersion}
              onNavigate={(tab) => navigateTo(tab)}
              onStartCapture={(method) => navigateTo('captures', { captureMethod: method })}
              onOpenSettings={(section) => navigateTo('settings', { section })}
              onOpenChangelog={() => setChangelogOpen(true)}
            />
          )}

          {activeTab === 'capture' && (
            <VoiceNotePage
              onOpenSpeechSettings={() => navigateTo('settings', { section: 'speech' })}
            />
          )}

          {activeTab === 'meetings' && (
            <MeetingsPage
              onOpenSpeechSettings={() => navigateTo('settings', { section: 'speech' })}
              onOpenProviderSettings={() => navigateTo('settings', { section: 'advanced' })}
              onOpenCalendarSettings={() => navigateTo('settings', { section: 'calendar' })}
            />
          )}

          {activeTab === 'scribble' && (
            <ScribbleViewer
              focusScribbleId={focusScribbleId}
              onStartCapture={() => navigateTo('captures', { captureMethod: 'text' })}
            />
          )}

          {activeTab === 'todos' && (
            <TodosPage onNavigateTab={(tab) => navigateTo(tab)} />
          )}

          {activeTab === 'graph' && (
            <KnowledgeGraphPage
              onOpenScribble={(id) => navigateTo('scribble', { focusScribbleId: id })}
            />
          )}

          {activeTab === 'files' && <FilesPage onNavigateTab={(tab) => navigateTo(tab)} />}

          {activeTab === 'captures' && (
            <CapturesPage
              initialCaptureMethod={captureMethod}
              onNavigateTab={(tab) => navigateTo(tab)}
              onOpenCaptureSettings={() => navigateTo('settings', { section: 'capture' })}
              onOpenScribble={(id) => navigateTo('scribble', { focusScribbleId: id })}
            />
          )}

          {activeTab === 'diagnostics' && (
            <DiagnosticsPage onNavigateTab={(tab) => navigateTo(tab)} />
          )}

          {activeTab === 'settings' && (
            <ProviderSettings
              initialSection={settingsSection}
              onNavigateTab={(tab) => navigateTo(tab)}
            />
          )}
        </main>
      </div>

      {/* Welcome First-Launch Onboarding Modal */}
      <WelcomeModal
        isOpen={welcomeOpen}
        initialDisplayName={profile?.display_name && profile.display_name !== 'Local User' ? profile.display_name : ''}
        onContinueGoogle={handleWelcomeGoogle}
        onContinueLocally={handleWelcomeLocally}
        onFinish={() => {
          setWelcomeOpen(false);
          if (explainAfterWelcome) {
            setExplainAfterWelcome(false);
            setExplanationOpen(true);
          }
        }}
      />

      {/* Account Trust & Privacy Explanation Modal */}
      <AccountExplanationModal
        isOpen={explanationOpen}
        onClose={() => setExplanationOpen(false)}
      />

      {/* Changelog Modal */}
      <ChangelogModal
        open={changelogOpen}
        onClose={() => setChangelogOpen(false)}
        currentVersion={appVersion}
      />

      {/* Close Confirmation Dialog */}
      <CloseConfirmationDialog
        open={closeDialogOpen}
        onOpenChange={setCloseDialogOpen}
        onConfirmClose={handleConfirmClose}
        onHide={handleConfirmHide}
      />
      </div>
    </TooltipProvider>
  );
};
