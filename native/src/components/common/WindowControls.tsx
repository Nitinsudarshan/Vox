import React, { useState, useEffect } from 'react';
import { Minus, EyeOff, X, Square } from 'lucide-react';
import { invoke } from '@tauri-apps/api/core';
import { getCurrentWindow } from '@tauri-apps/api/window';
import {
  Tooltip,
  TooltipContent,
  TooltipProvider,
  TooltipTrigger,
} from '@/components/ui/tooltip';

interface WindowControlsProps {
  onCloseClick: () => void;
  className?: string;
}

/**
 * Clean, borderless window controls for Vox desktop header:
 * - Minimize: Minimizes to OS taskbar.
 * - Maximize / Restore: Toggles maximized window state.
 * - Hide: Hides window to background while keeping Dictation Pill and shortcuts alive.
 * - Close: Prompts confirmation before terminating the app.
 */
export const WindowControls: React.FC<WindowControlsProps> = ({
  onCloseClick,
  className = '',
}) => {
  const [isMaximized, setIsMaximized] = useState(false);

  useEffect(() => {
    let mounted = true;
    const checkMaximized = async () => {
      try {
        const res = await invoke<boolean>('is_main_window_maximized');
        if (mounted) setIsMaximized(res);
      } catch {
        try {
          const max = await getCurrentWindow().isMaximized();
          if (mounted) setIsMaximized(max);
        } catch {
          // ignore
        }
      }
    };

    void checkMaximized();
    window.addEventListener('resize', checkMaximized);
    return () => {
      mounted = false;
      window.removeEventListener('resize', checkMaximized);
    };
  }, []);

  const handleMinimize = async (e: React.MouseEvent) => {
    e.stopPropagation();
    try {
      await invoke('minimize_main_window');
    } catch {
      try {
        await getCurrentWindow().minimize();
      } catch (err) {
        console.warn('Failed to minimize window:', err);
      }
    }
  };

  const handleToggleMaximize = async (e: React.MouseEvent) => {
    e.stopPropagation();
    try {
      const res = await invoke<boolean>('toggle_maximize_main_window');
      setIsMaximized(res);
    } catch {
      try {
        const win = getCurrentWindow();
        await win.toggleMaximize();
        const max = await win.isMaximized();
        setIsMaximized(max);
      } catch (err) {
        console.warn('Failed to toggle maximize window:', err);
      }
    }
  };

  const handleHide = async (e: React.MouseEvent) => {
    e.stopPropagation();
    try {
      await invoke('hide_main_window');
    } catch {
      try {
        await getCurrentWindow().hide();
      } catch (err) {
        console.warn('Failed to hide window:', err);
      }
    }
  };

  const handleClose = (e: React.MouseEvent) => {
    e.stopPropagation();
    onCloseClick();
  };

  return (
    <TooltipProvider delayDuration={200}>
      <div
        className={`flex items-center gap-1 ${className}`}
        role="group"
        aria-label="Window Controls"
      >
        {/* Minimize Button */}
        <Tooltip>
          <TooltipTrigger asChild>
            <button
              type="button"
              onClick={handleMinimize}
              className="group h-7 w-7 rounded-md text-muted-foreground hover:text-foreground hover:bg-muted/80 active:scale-95 flex items-center justify-center cursor-pointer transition-colors focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
              aria-label="Minimize"
            >
              <Minus className="w-3.5 h-3.5 stroke-[2]" />
            </button>
          </TooltipTrigger>
          <TooltipContent side="bottom" sideOffset={6}>
            <span>Minimize</span>
          </TooltipContent>
        </Tooltip>

        {/* Maximize / Restore Button */}
        <Tooltip>
          <TooltipTrigger asChild>
            <button
              type="button"
              onClick={handleToggleMaximize}
              className="group h-7 w-7 rounded-md text-muted-foreground hover:text-foreground hover:bg-muted/80 active:scale-95 flex items-center justify-center cursor-pointer transition-colors focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
              aria-label={isMaximized ? 'Restore' : 'Maximize'}
            >
              {isMaximized ? (
                <svg
                  className="w-3.5 h-3.5 stroke-[1.6]"
                  viewBox="0 0 16 16"
                  fill="none"
                  stroke="currentColor"
                >
                  <rect x="4.5" y="1.5" width="9" height="9" rx="1" />
                  <path d="M2.5 5.5v7a1 1 0 001 1h7" strokeLinecap="round" />
                </svg>
              ) : (
                <Square className="w-3.5 h-3.5 stroke-[1.6]" />
              )}
            </button>
          </TooltipTrigger>
          <TooltipContent side="bottom" sideOffset={6}>
            <span>{isMaximized ? 'Restore Down' : 'Maximize'}</span>
          </TooltipContent>
        </Tooltip>

        {/* Hide Button */}
        <Tooltip>
          <TooltipTrigger asChild>
            <button
              type="button"
              onClick={handleHide}
              className="group h-7 w-7 rounded-md text-muted-foreground hover:text-foreground hover:bg-muted/80 active:scale-95 flex items-center justify-center cursor-pointer transition-colors focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
              aria-label="Hide Vox"
            >
              <EyeOff className="w-3.5 h-3.5 stroke-[1.8]" />
            </button>
          </TooltipTrigger>
          <TooltipContent side="bottom" sideOffset={6}>
            <span>Hide Vox (runs in background)</span>
          </TooltipContent>
        </Tooltip>

        {/* Close Button */}
        <Tooltip>
          <TooltipTrigger asChild>
            <button
              type="button"
              onClick={handleClose}
              className="group h-7 w-7 rounded-md text-muted-foreground hover:text-destructive hover:bg-destructive/15 active:bg-destructive active:text-destructive-foreground active:scale-95 flex items-center justify-center cursor-pointer transition-colors focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
              aria-label="Close Vox"
            >
              <X className="w-3.5 h-3.5 stroke-[2]" />
            </button>
          </TooltipTrigger>
          <TooltipContent side="bottom" sideOffset={6}>
            <span>Close Vox</span>
          </TooltipContent>
        </Tooltip>
      </div>
    </TooltipProvider>
  );
};
