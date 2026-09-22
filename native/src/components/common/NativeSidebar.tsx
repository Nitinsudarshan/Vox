import React from 'react';
import {
  Mic,
  Users,
  Sparkles,
  Settings,
  ShieldCheck,
  Activity,
  ChevronsUpDown,
  Database,
  Cloud,
  Sparkle,
  Sliders,
  Network,
  ListChecks,
  FlaskConical,
} from 'lucide-react';

import { Badge } from '@/components/ui/badge';
import {
  Tooltip,
  TooltipContent,
  TooltipProvider,
  TooltipTrigger,
} from '@/components/ui/tooltip';
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuGroup,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from '@/components/ui/dropdown-menu';
import type { MainTabType, RelayAccount, RelayProfile } from '@/types';

/** The sidebar navigates the same tabs `App` routes — one list, in `types/navigation.ts`. */
export type TabType = MainTabType;

interface NativeSidebarProps {
  isOpen: boolean;
  onToggle: () => void;
  activeTab: TabType;
  setActiveTab: (tab: TabType) => void;
  account: RelayAccount | null;
  profile: RelayProfile | null;
  appVersion: string;

  onOpenChangelog: () => void;
  onOpenWelcome: () => void;
  onOpenExplanation: () => void;
}

export const NativeSidebar: React.FC<NativeSidebarProps> = ({
  isOpen,
  onToggle: _onToggle,
  activeTab,
  setActiveTab,
  account,
  profile,
  appVersion,

  onOpenChangelog,
  onOpenWelcome,
  onOpenExplanation,
}) => {
  const [activeWorkspace, setActiveWorkspace] = React.useState<'local' | 'cloud'>(
    account?.authenticated ? 'cloud' : 'local'
  );

  React.useEffect(() => {
    if (account?.authenticated) {
      setActiveWorkspace('cloud');
    }
  }, [account?.authenticated]);

  interface NavItemConfig {
    id: TabType;
    label: string;
    icon: React.ComponentType<{ className?: string }>;
    color: string;
    activeBg: string;
  }

  const primaryNavItems: NavItemConfig[] = [
    {
      id: 'capture',
      label: 'Voice Notes',
      icon: Mic,
      color: 'text-emerald-500',
      activeBg: 'bg-sidebar-accent text-sidebar-accent-foreground',
    },
    {
      id: 'meetings',
      label: 'Meetings',
      icon: Users,
      color: 'text-rose-500',
      activeBg: 'bg-sidebar-accent text-sidebar-accent-foreground',
    },
    {
      id: 'scribble',
      label: 'Scribbles',
      icon: Sparkles,
      color: 'text-amber-500',
      activeBg: 'bg-sidebar-accent text-sidebar-accent-foreground',
    },
    {
      id: 'todos',
      label: 'TODOs',
      icon: ListChecks,
      color: 'text-sky-500',
      activeBg: 'bg-sidebar-accent text-sidebar-accent-foreground',
    },
    {
      id: 'graph',
      label: 'Knowledge Graph',
      icon: Network,
      color: 'text-blue-500',
      activeBg: 'bg-sidebar-accent text-sidebar-accent-foreground',
    },
  ];

  const systemNavItems: NavItemConfig[] = [
    {
      id: 'tests',
      label: 'Tests',
      icon: FlaskConical,
      color: 'text-amber-500 dark:text-amber-400',
      activeBg: 'bg-sidebar-accent text-sidebar-accent-foreground',
    },
    {
      id: 'settings',
      label: 'Settings',
      icon: Settings,
      color: 'text-muted-foreground',
      activeBg: 'bg-sidebar-accent text-sidebar-accent-foreground',
    },
    {
      id: 'diagnostics',
      label: 'Diagnostics',
      icon: Activity,
      color: 'text-violet-400',
      activeBg: 'bg-sidebar-accent text-sidebar-accent-foreground',
    },
  ];

  const displayName = profile?.display_name || account?.display_name || 'Local User';
  const storageMode = activeWorkspace === 'cloud' ? 'Hybrid Cloud' : 'Local Vault';
  const initial = displayName && displayName !== 'Local User' ? displayName.charAt(0).toUpperCase() : 'R';

  const renderNavButton = (item: NavItemConfig) => {
    const Icon = item.icon;
    const isActive = activeTab === item.id;

    const button = (
      <button
        key={item.id}
        type="button"
        onClick={() => setActiveTab(item.id)}
        className={`flex items-center rounded-lg text-xs font-medium transition-colors cursor-pointer overflow-hidden ${
          isOpen
            ? 'w-full h-9 px-2.5 py-1.5'
            : 'size-8 justify-center p-0 shrink-0'
        } ${
          isActive
            ? `${item.activeBg} font-semibold shadow-xs`
            : 'text-muted-foreground hover:bg-sidebar-accent/50 hover:text-sidebar-foreground'
        }`}
        aria-label={item.label}
      >
        <Icon className={`w-4 h-4 shrink-0 ${item.color}`} />
        {isOpen && (
          <div className="flex items-center justify-between flex-1 min-w-0 ml-2.5 transition-all duration-300 ease-[cubic-bezier(0.4,0,0.2,1)] overflow-hidden whitespace-nowrap">
            <span className="truncate">{item.label}</span>
            {isActive && (
              <span className="w-1.5 h-1.5 rounded-full bg-primary shrink-0 ml-2" />
            )}
          </div>
        )}
      </button>
    );

    if (!isOpen) {
      return (
        <Tooltip key={item.id}>
          <TooltipTrigger asChild>{button}</TooltipTrigger>
          <TooltipContent side="right" sideOffset={10}>
            <span>{item.label}</span>
          </TooltipContent>
        </Tooltip>
      );
    }

    return button;
  };

  return (
    <TooltipProvider delayDuration={150}>
      <aside
        className={`relative transition-[width] duration-300 ease-[cubic-bezier(0.4,0,0.2,1)] bg-sidebar border-r border-sidebar-border flex flex-col shrink-0 select-none z-20 h-full overflow-hidden ${
          isOpen ? 'w-64' : 'w-12 items-center'
        }`}
      >
        {/* Navigation & Quick Links Body */}
        <div className={`flex-1 w-full overflow-y-auto overflow-x-hidden flex flex-col ${isOpen ? 'px-3 py-3' : 'px-2 py-3 items-center'}`}>
          {/* Primary Navigation */}
          <nav className="w-full space-y-1 shrink-0 flex flex-col items-center">
            {primaryNavItems.map(renderNavButton)}
          </nav>

          {/* Visual Separator between Primary and System */}
          <div className={`w-full border-t border-sidebar-border ${isOpen ? 'my-2.5' : 'my-2'} shrink-0`} />

          {/* System Navigation */}
          <nav className="w-full space-y-1 shrink-0 flex flex-col items-center">
            {systemNavItems.map(renderNavButton)}
          </nav>
        </div>


        {/* User Footer Card & Popover Menu (sidebar-07 NavUser Pattern) */}
        <div className={`mt-auto w-full border-t border-sidebar-border flex flex-col items-center shrink-0 ${isOpen ? 'p-3 pt-2.5' : 'p-2 pt-2.5'}`}>
          <DropdownMenu>
            <DropdownMenuTrigger asChild>
              <button
                type="button"
                className={`flex items-center transition-all cursor-pointer group shadow-2xs overflow-hidden ${
                  isOpen
                    ? 'w-full h-10 rounded-lg bg-card/60 hover:bg-card border border-sidebar-border p-1.5 mb-2'
                    : 'size-8 rounded-full border border-border bg-card flex items-center justify-center p-0 mb-2 hover:ring-2 hover:ring-primary/40 shadow-xs'
                }`}
                title="Account & Session Settings"
              >
                {isOpen ? (
                  <>
                    <div className="flex aspect-square size-7 items-center justify-center rounded-full shrink-0 overflow-hidden">
                      {account?.authenticated && account.profile_image ? (
                        <img
                          src={account.profile_image}
                          alt="Profile"
                          referrerPolicy="no-referrer"
                          className="w-full h-full object-cover rounded-full"
                          onError={(e) => { (e.currentTarget as HTMLImageElement).style.display = 'none'; }}
                        />
                      ) : (
                        <div className="w-full h-full rounded-full bg-primary/20 text-primary font-bold flex items-center justify-center text-xs">
                          {initial}
                        </div>
                      )}
                    </div>

                    <div className="flex items-center flex-1 min-w-0 ml-2 transition-all duration-300 ease-[cubic-bezier(0.4,0,0.2,1)] overflow-hidden whitespace-nowrap">
                      <div className="grid flex-1 leading-tight min-w-0 text-left">
                        <span className="text-xs font-bold text-sidebar-foreground truncate group-hover:text-primary transition-colors">
                          {displayName}
                        </span>
                        <span className="text-[10px] text-muted-foreground truncate font-mono">
                          {storageMode}
                        </span>
                      </div>
                      <ChevronsUpDown className="w-3.5 h-3.5 text-muted-foreground shrink-0 ml-1" />
                    </div>
                  </>
                ) : (
                  <div className="size-full rounded-full flex items-center justify-center overflow-hidden">
                    {account?.authenticated && account.profile_image ? (
                      <img
                        src={account.profile_image}
                        alt="Profile"
                        referrerPolicy="no-referrer"
                        className="size-full object-cover rounded-full"
                        onError={(e) => { (e.currentTarget as HTMLImageElement).style.display = 'none'; }}
                      />
                    ) : (
                      <div className="size-full rounded-full bg-primary/20 text-primary font-bold flex items-center justify-center text-xs">
                        {initial}
                      </div>
                    )}
                  </div>
                )}
              </button>
            </DropdownMenuTrigger>

            <DropdownMenuContent
              side={isOpen ? 'top' : 'right'}
              align={isOpen ? 'end' : 'start'}
              sideOffset={8}
              className="w-64 p-1.5"
            >
              <DropdownMenuLabel className="p-0 font-normal">
                <div className="flex items-center gap-2.5 px-2 py-2 text-left text-xs rounded-lg bg-muted/40 mb-1">
                  <div className="w-7 h-7 rounded-full bg-primary/20 text-primary font-bold flex items-center justify-center text-xs shrink-0">
                    {initial}
                  </div>
                  <div className="grid flex-1 leading-tight min-w-0">
                    <div className="flex items-center justify-between gap-1">
                      <span className="font-bold truncate">{displayName}</span>
                      <Badge
                        variant="outline"
                        className="text-[9px] font-mono px-1 py-0 border-primary/30 text-primary"
                      >
                        {account?.authenticated ? 'Google' : 'Local'}
                      </Badge>
                    </div>
                    <span className="text-[10px] text-muted-foreground font-mono truncate">
                      {storageMode}
                    </span>
                  </div>
                </div>
              </DropdownMenuLabel>

              <DropdownMenuGroup>
                <DropdownMenuLabel className="text-[10px] text-muted-foreground uppercase font-mono tracking-wider px-2 py-1">
                  Storage & Workspace
                </DropdownMenuLabel>
                <DropdownMenuItem
                  onClick={() => setActiveWorkspace('local')}
                  className="gap-2.5 cursor-pointer py-1.5"
                >
                  <div className="flex size-6 items-center justify-center rounded-md border bg-background">
                    <Database className="size-3.5 text-emerald-500" />
                  </div>
                  <div className="flex flex-col">
                    <span className="font-semibold text-xs">Local Vault Only</span>
                    <span className="text-[10px] text-muted-foreground">Markdown & Audio on Disk</span>
                  </div>
                  {activeWorkspace === 'local' && (
                    <span className="ml-auto text-[10px] font-bold text-primary">✓</span>
                  )}
                </DropdownMenuItem>

                <DropdownMenuItem
                  onClick={() => setActiveWorkspace('cloud')}
                  className="gap-2.5 cursor-pointer py-1.5"
                >
                  <div className="flex size-6 items-center justify-center rounded-md border bg-background">
                    <Cloud className="size-3.5 text-blue-500" />
                  </div>
                  <div className="flex flex-col">
                    <span className="font-semibold text-xs">Hybrid Cloud Sync</span>
                    <span className="text-[10px] text-muted-foreground">Supabase Multi-Device</span>
                  </div>
                  {activeWorkspace === 'cloud' && (
                    <span className="ml-auto text-[10px] font-bold text-primary">✓</span>
                  )}
                </DropdownMenuItem>

                <DropdownMenuSeparator />

                <DropdownMenuItem
                  onClick={() => setActiveTab('settings')}
                  className="gap-2.5 cursor-pointer py-2 text-xs"
                >
                  <Sliders className="w-3.5 h-3.5 text-muted-foreground" />
                  <span>Account & App Preferences</span>
                </DropdownMenuItem>

                <DropdownMenuItem
                  onClick={onOpenChangelog}
                  className="gap-2.5 cursor-pointer py-2 text-xs"
                >
                  <Activity className="w-3.5 h-3.5 text-primary" />
                  <span>Release Notes (v{appVersion})</span>
                </DropdownMenuItem>

                <DropdownMenuItem
                  onClick={onOpenExplanation}
                  className="gap-2.5 cursor-pointer py-2 text-xs"
                >
                  <ShieldCheck className="w-3.5 h-3.5 text-emerald-500" />
                  <span>Privacy & Trust Architecture</span>
                </DropdownMenuItem>

                <DropdownMenuItem
                  onClick={onOpenWelcome}
                  className="gap-2.5 cursor-pointer py-2 text-xs"
                >
                  <Sparkle className="w-3.5 h-3.5 text-amber-500" />
                  <span>Onboarding & Setup Guide</span>
                </DropdownMenuItem>
              </DropdownMenuGroup>
            </DropdownMenuContent>
          </DropdownMenu>

          {/* Footer Status Indicators (Expanded Only) */}
          {isOpen && (
            <div className="w-full flex items-center justify-between text-[10px] text-muted-foreground px-1 pt-1 overflow-hidden whitespace-nowrap">
              <div className="flex items-center gap-1.5 font-medium text-emerald-500">
                <ShieldCheck className="w-3.5 h-3.5 shrink-0" />
                <span className="truncate">{account?.authenticated ? 'Hybrid Mode' : 'Local Vault'}</span>
              </div>
              <button
                type="button"
                onClick={onOpenChangelog}
                className="flex items-center gap-1 font-mono hover:text-primary transition-colors cursor-pointer group shrink-0"
                title="View Release Notes & Changelog"
              >
                <Activity className="w-3 h-3 text-primary group-hover:animate-pulse shrink-0" />
                <span className="underline decoration-dotted underline-offset-2">v{appVersion}</span>
              </button>
            </div>
          )}
        </div>
      </aside>
    </TooltipProvider>
  );
};
