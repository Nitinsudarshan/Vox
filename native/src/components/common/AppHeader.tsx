import React, { useEffect, useRef } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { getCurrentWindow } from '@tauri-apps/api/window';
import { Sidebar as SidebarIcon, ChevronRight } from 'lucide-react';
import { Button } from '@/components/ui/button';
import { ThemeToggle } from '@/components/ThemeToggle';
import { WindowControls } from './WindowControls';
import { VoxLogo } from './VoxLogo';
import type { MainTabType } from '@/types';

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
  tests: 'Tests',
  settings: 'Settings',
};

interface AppHeaderProps {
  sidebarOpen: boolean;
  onToggleSidebar: () => void;
  activeTab: MainTabType;
  onNavigateHome: () => void;
  onCloseClick: () => void;
  className?: string;
}

/**
 * Single unified desktop window chrome and navigation header for Vox.
 *
 * Borderless, product-native header integrating:
 * - Native window dragging via `data-tauri-drag-region`
 * - Sidebar expand/collapse toggle
 * - Vox brand identity & breadcrumbs
 * - Light/Dark theme toggle
 * - Clean desktop window controls (Minimize, Hide, Close)
 */
export const AppHeader: React.FC<AppHeaderProps> = ({
  sidebarOpen,
  onToggleSidebar,
  activeTab,
  onNavigateHome,
  onCloseClick,
  className = '',
}) => {
  // Intercept OS-level close events (e.g. Alt+F4) to present confirmation dialog.
  // Registered once: the handler reads the latest `onCloseClick` through a
  // ref, because the parent passes a new function on every render and each
  // re-registration could outlive its cleanup while still pending.
  const onCloseClickRef = useRef(onCloseClick);
  onCloseClickRef.current = onCloseClick;
  useEffect(() => {
    let disposed = false;
    let unlisten: (() => void) | undefined;
    try {
      const win = getCurrentWindow();
      if (win && typeof win.onCloseRequested === 'function') {
        void win
          .onCloseRequested((event) => {
            event.preventDefault();
            onCloseClickRef.current();
          })
          .then((fn) => {
            if (disposed) fn();
            else unlisten = fn;
          });
      }
    } catch {
      // Browser fallback or mock environment
    }

    return () => {
      disposed = true;
      if (unlisten) unlisten();
    };
  }, []);

  const handleHeaderDoubleClick = async (e: React.MouseEvent) => {
    if ((e.target as HTMLElement).closest('button, a, input, select')) return;
    try {
      await invoke('toggle_maximize_main_window');
    } catch {
      try {
        await getCurrentWindow().toggleMaximize();
      } catch (err) {
        console.warn('Failed to toggle maximize:', err);
      }
    }
  };

  return (
    <header
      data-tauri-drag-region
      onDoubleClick={handleHeaderDoubleClick}
      className={`h-11 bg-sidebar border-b border-border px-3 flex items-center justify-between shrink-0 select-none z-30 transition-colors ${className}`}
    >
      {/* Left: Navigation Menu Toggle + Vox Identity & Breadcrumbs */}
      <div className="flex items-center gap-2.5 min-w-0" data-tauri-drag-region>
        <Button
          variant="ghost"
          size="icon"
          className="h-7 w-7 text-muted-foreground hover:text-foreground shrink-0 cursor-pointer focus-visible:ring-1 focus-visible:ring-ring"
          onClick={onToggleSidebar}
          aria-label="Toggle Sidebar Navigation"
          title={sidebarOpen ? 'Collapse sidebar' : 'Expand sidebar'}
        >
          <SidebarIcon className="w-3.5 h-3.5" />
        </Button>

        <div className="h-3.5 w-px bg-border/80 shrink-0" />

        <nav
          aria-label="Breadcrumb"
          className="flex items-center gap-1.5 text-xs text-muted-foreground font-mono uppercase tracking-wider truncate"
        >
          <button
            type="button"
            onClick={onNavigateHome}
            className="flex items-center h-5 max-h-5 hover:opacity-80 transition-opacity cursor-pointer focus:outline-none focus-visible:ring-1 focus-visible:ring-ring rounded-xs px-1 py-0.5 -mx-1 shrink-0"
            aria-label="Navigate to Vox Home"
          >
            <VoxLogo expanded className="h-4 max-h-4 w-auto shrink-0" />
          </button>

          {activeTab !== 'home' && (
            <>
              <ChevronRight className="w-3 h-3 text-muted-foreground/50 shrink-0" />
              <span className="font-semibold text-foreground truncate">
                {TAB_LABELS[activeTab]}
              </span>
            </>
          )}
        </nav>
      </div>

      {/* Center: Draggable Window Chrome Region */}
      <div
        data-tauri-drag-region
        className="flex-1 h-full min-w-[24px] cursor-default"
      />

      {/* Right: Theme Toggle + Window Controls */}
      <div className="flex items-center gap-1 shrink-0" data-tauri-drag-region>
        <ThemeToggle />

        <div className="h-3.5 w-px bg-border/80 mx-1 shrink-0" />

        <WindowControls onCloseClick={onCloseClick} />
      </div>
    </header>
  );
};
