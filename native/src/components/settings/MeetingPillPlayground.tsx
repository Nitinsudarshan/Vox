import React, { useState, useEffect, useRef } from 'react';
import {
  Mic,
  RotateCcw,
  Clock,
  Layers,
  Sparkles,
  Maximize2,
  CheckCircle2,
  AlertTriangle,
  Move,
  Columns2,
  Rows2,
  GripHorizontal,
} from 'lucide-react';

import { Button } from '@/components/ui/button';
import { Switch } from '@/components/ui/switch';
import { Badge } from '@/components/ui/badge';
import {
  MeetingPill,
  type MeetingPillState,
  type MeetingPillOrientation,
} from '@/components/meetings/MeetingPill';

type AudioProfile = 'conversation' | 'mic_only' | 'sys_only' | 'silent';
type BackdropStyle = 'default' | 'dark' | 'light' | 'glass';
type VerticalAlign = 'top' | 'middle' | 'bottom';
type HorizontalAlign = 'left' | 'center' | 'right';

const BAR_COUNT = 20;
const SILENT_BARS = new Array(BAR_COUNT).fill(0);

const POSITION_MATRIX: Array<{
  v: VerticalAlign;
  h: HorizontalAlign;
  label: string;
  shortLabel: string;
}> = [
  { v: 'top', h: 'left', label: 'Top Left', shortLabel: 'TL' },
  { v: 'top', h: 'center', label: 'Top Center', shortLabel: 'TC' },
  { v: 'top', h: 'right', label: 'Top Right', shortLabel: 'TR' },
  { v: 'middle', h: 'left', label: 'Middle Left', shortLabel: 'ML' },
  { v: 'middle', h: 'center', label: 'Center', shortLabel: 'C' },
  { v: 'middle', h: 'right', label: 'Middle Right', shortLabel: 'MR' },
  { v: 'bottom', h: 'left', label: 'Bottom Left', shortLabel: 'BL' },
  { v: 'bottom', h: 'center', label: 'Bottom Center', shortLabel: 'BC' },
  { v: 'bottom', h: 'right', label: 'Bottom Right', shortLabel: 'BR' },
];

/**
 * Interactive Meeting Pill workbench in Developer Settings.
 *
 * Provides a real-time, interactive environment to preview and stress-test the
 * meeting recording pill:
 * - 2 Pill styles: Horizontal (Default) and Vertical.
 * - Screen positioning: 3x3 preset matrix (Top/Middle/Bottom x Left/Center/Right) + Free Movement (drag & drop).
 * - Hover & click interaction directly on the pill (pause, resume, stop).
 * - Multi-state switching (Recording, Paused, Transcribing/Finalizing).
 * - Real-time simulated speech audio waveforms for mic & system channels.
 * - Missing system-audio warning toggle.
 * - Monospace duration layout testing (+1 min, +1 hour) to verify triple-digit timers.
 * - Multi-surface backdrop preview to evaluate contrast against dark & light windows.
 */
export const MeetingPillPlayground: React.FC = () => {
  const [style, setStyle] = useState<MeetingPillOrientation>('horizontal');
  const [verticalAlign, setVerticalAlign] = useState<VerticalAlign>('middle');
  const [horizontalAlign, setHorizontalAlign] = useState<HorizontalAlign>('center');
  const [isFreeMovement, setIsFreeMovement] = useState<boolean>(false);
  const [freePos, setFreePos] = useState<{ x: number; y: number }>({ x: 50, y: 50 });
  const [isDragging, setIsDragging] = useState<boolean>(false);

  const [state, setState] = useState<MeetingPillState>('recording');
  const [elapsedSec, setElapsedSec] = useState<number>(42);
  const [isSystemAudioActive, setIsSystemAudioActive] = useState<boolean>(true);
  const [isSimulatingAudio, setIsSimulatingAudio] = useState<boolean>(true);
  const [audioProfile, setAudioProfile] = useState<AudioProfile>('conversation');
  const [forceExpanded, setForceExpanded] = useState<boolean>(false);
  const [backdrop, setBackdrop] = useState<BackdropStyle>('default');
  const [micLevels, setMicLevels] = useState<number[]>(SILENT_BARS);
  const [sysLevels, setSysLevels] = useState<number[]>(SILENT_BARS);
  const [lastAction, setLastAction] = useState<string | null>(null);

  const viewportRef = useRef<HTMLDivElement>(null);
  const pillContainerRef = useRef<HTMLDivElement>(null);
  const dragStartRef = useRef<{
    startMouseX: number;
    startMouseY: number;
    initialCenterX: number;
    initialCenterY: number;
  } | null>(null);
  const actionTimeoutRef = useRef<ReturnType<typeof setTimeout> | null>(null);

  const notifyAction = (msg: string) => {
    setLastAction(msg);
    if (actionTimeoutRef.current) clearTimeout(actionTimeoutRef.current);
    actionTimeoutRef.current = setTimeout(() => setLastAction(null), 3500);
  };

  // Timer simulation
  useEffect(() => {
    if (state !== 'recording') return;
    const interval = setInterval(() => {
      setElapsedSec((prev) => prev + 1);
    }, 1000);
    return () => clearInterval(interval);
  }, [state]);

  // Audio level generator simulating natural human speech cadence
  useEffect(() => {
    if (state === 'paused' || state === 'transcribing' || !isSimulatingAudio) {
      setMicLevels(SILENT_BARS);
      setSysLevels(SILENT_BARS);
      return;
    }

    let phase = 0;
    const interval = setInterval(() => {
      phase += 0.25;

      const micSpeechBurst = Math.sin(phase * 0.7) > 0.1 ? Math.abs(Math.sin(phase * 1.5)) : 0.05;
      const sysSpeechBurst = Math.cos(phase * 0.5) > 0.2 ? Math.abs(Math.cos(phase * 1.8)) : 0.05;

      const newMic =
        audioProfile === 'conversation' || audioProfile === 'mic_only'
          ? Math.min(1, Math.max(0.02, micSpeechBurst * (0.4 + Math.random() * 0.6)))
          : 0.02;

      const newSys =
        (audioProfile === 'conversation' || audioProfile === 'sys_only') && isSystemAudioActive
          ? Math.min(1, Math.max(0.02, sysSpeechBurst * (0.35 + Math.random() * 0.55)))
          : 0.02;

      setMicLevels((prev) => [...prev.slice(1), newMic]);
      setSysLevels((prev) => [...prev.slice(1), newSys]);
    }, 80);

    return () => clearInterval(interval);
  }, [state, isSimulatingAudio, audioProfile, isSystemAudioActive]);

  // Ensure pill stays strictly within viewport bounds even when styles, dimensions or expansion change
  useEffect(() => {
    const clampToBounds = () => {
      if (!isFreeMovement || !viewportRef.current || !pillContainerRef.current) return;
      const vRect = viewportRef.current.getBoundingClientRect();
      const pRect = pillContainerRef.current.getBoundingClientRect();
      if (vRect.width === 0 || vRect.height === 0) return;

      const edgeMargin = 10;
      const halfW = pRect.width / 2;
      const halfH = pRect.height / 2;

      let minPctX = Math.ceil(((edgeMargin + halfW) / vRect.width) * 100);
      let maxPctX = Math.floor(((vRect.width - edgeMargin - halfW) / vRect.width) * 100);
      let minPctY = Math.ceil(((edgeMargin + halfH) / vRect.height) * 100);
      let maxPctY = Math.floor(((vRect.height - edgeMargin - halfH) / vRect.height) * 100);

      if (minPctX > maxPctX) {
        minPctX = 50;
        maxPctX = 50;
      }
      if (minPctY > maxPctY) {
        minPctY = 50;
        maxPctY = 50;
      }

      const clampedX = Math.max(minPctX, Math.min(maxPctX, freePos.x));
      const clampedY = Math.max(minPctY, Math.min(maxPctY, freePos.y));

      if (clampedX !== freePos.x || clampedY !== freePos.y) {
        setFreePos({ x: clampedX, y: clampedY });
      }
    };

    clampToBounds();
    window.addEventListener('resize', clampToBounds);
    return () => window.removeEventListener('resize', clampToBounds);
  }, [style, forceExpanded, isFreeMovement, freePos.x, freePos.y]);

  // Pill action handlers
  const handleTogglePause = () => {
    if (state === 'recording') {
      setState('paused');
      notifyAction('Paused recording via pill control.');
    } else if (state === 'paused') {
      setState('recording');
      notifyAction('Resumed recording via pill control.');
    }
  };

  const handleStop = () => {
    setState('transcribing');
    notifyAction('Stopped recording — pill entered transcribing state.');
  };

  const handleResetToDefault = () => {
    setStyle('horizontal');
    setVerticalAlign('middle');
    setHorizontalAlign('center');
    setIsFreeMovement(false);
    setFreePos({ x: 50, y: 50 });
    setState('recording');
    setElapsedSec(42);
    setIsSystemAudioActive(true);
    setIsSimulatingAudio(true);
    setAudioProfile('conversation');
    setForceExpanded(false);
    setBackdrop('default');
    notifyAction('Reset workbench to defaults.');
  };

  // Dragging logic for Free Movement with strict boundary clamping
  const handlePointerDown = (e: React.PointerEvent) => {
    if ((e.target as HTMLElement).closest('button')) return;
    setIsDragging(true);
    setIsFreeMovement(true);

    const viewport = viewportRef.current;
    const pill = pillContainerRef.current;
    if (viewport && pill) {
      const vRect = viewport.getBoundingClientRect();
      const pRect = pill.getBoundingClientRect();
      const initialCenterX = pRect.left + pRect.width / 2 - vRect.left;
      const initialCenterY = pRect.top + pRect.height / 2 - vRect.top;
      dragStartRef.current = {
        startMouseX: e.clientX,
        startMouseY: e.clientY,
        initialCenterX,
        initialCenterY,
      };
    } else {
      const vWidth = viewport?.clientWidth || 600;
      const vHeight = viewport?.clientHeight || 300;
      dragStartRef.current = {
        startMouseX: e.clientX,
        startMouseY: e.clientY,
        initialCenterX: (freePos.x / 100) * vWidth,
        initialCenterY: (freePos.y / 100) * vHeight,
      };
    }
  };

  // Window-level pointer listeners so dragging continues smoothly and stays strictly clamped
  useEffect(() => {
    if (!isDragging) return;

    const handlePointerMove = (e: PointerEvent) => {
      const viewport = viewportRef.current;
      const pill = pillContainerRef.current;
      if (!viewport || !dragStartRef.current) return;

      const vRect = viewport.getBoundingClientRect();
      const pRect = pill?.getBoundingClientRect() || {
        width: style === 'vertical' ? 48 : 260,
        height: style === 'vertical' ? 180 : 44,
      };

      // Strict edge margin: pill edges never touch or cross the viewport boundary
      const edgeMargin = 10;
      const halfW = pRect.width / 2;
      const halfH = pRect.height / 2;

      const minCenterX = edgeMargin + halfW;
      const maxCenterX = Math.max(minCenterX, vRect.width - edgeMargin - halfW);
      const minCenterY = edgeMargin + halfH;
      const maxCenterY = Math.max(minCenterY, vRect.height - edgeMargin - halfH);

      // Delta from initial grab position
      const deltaX = e.clientX - dragStartRef.current.startMouseX;
      const deltaY = e.clientY - dragStartRef.current.startMouseY;
      const rawCenterX = dragStartRef.current.initialCenterX + deltaX;
      const rawCenterY = dragStartRef.current.initialCenterY + deltaY;

      // Strictly clamp within center boundaries
      const clampedCenterX = Math.max(minCenterX, Math.min(maxCenterX, rawCenterX));
      const clampedCenterY = Math.max(minCenterY, Math.min(maxCenterY, rawCenterY));

      // Calculate safe percentage limits ensuring pixel coordinates cannot exceed margins
      let minPctX = Math.ceil(((edgeMargin + halfW) / vRect.width) * 100);
      let maxPctX = Math.floor(((vRect.width - edgeMargin - halfW) / vRect.width) * 100);
      let minPctY = Math.ceil(((edgeMargin + halfH) / vRect.height) * 100);
      let maxPctY = Math.floor(((vRect.height - edgeMargin - halfH) / vRect.height) * 100);

      if (minPctX > maxPctX) {
        minPctX = 50;
        maxPctX = 50;
      }
      if (minPctY > maxPctY) {
        minPctY = 50;
        maxPctY = 50;
      }

      const rawPctX = (clampedCenterX / vRect.width) * 100;
      const rawPctY = (clampedCenterY / vRect.height) * 100;

      const newX = Math.max(minPctX, Math.min(maxPctX, Math.round(rawPctX)));
      const newY = Math.max(minPctY, Math.min(maxPctY, Math.round(rawPctY)));

      setFreePos({ x: newX, y: newY });
    };

    const handlePointerUp = () => {
      setIsDragging(false);
      dragStartRef.current = null;
      notifyAction(`Pill moved freely and clamped safely to (${freePos.x}%, ${freePos.y}%).`);
    };

    window.addEventListener('pointermove', handlePointerMove);
    window.addEventListener('pointerup', handlePointerUp);
    return () => {
      window.removeEventListener('pointermove', handlePointerMove);
      window.removeEventListener('pointerup', handlePointerUp);
    };
  }, [isDragging, style, freePos.x, freePos.y]);

  // Viewport flexbox mapping classes
  const getViewportPositionClasses = () => {
    let vClass = 'justify-center';
    if (verticalAlign === 'top') vClass = 'justify-start';
    if (verticalAlign === 'bottom') vClass = 'justify-end';

    let hClass = 'items-center';
    if (horizontalAlign === 'left') hClass = 'items-start';
    if (horizontalAlign === 'right') hClass = 'items-end';

    return `${vClass} ${hClass}`;
  };

  // Backdrop styling classes
  const getBackdropClass = () => {
    switch (backdrop) {
      case 'dark':
        return 'bg-neutral-950 border-neutral-800 text-neutral-100';
      case 'light':
        return 'bg-neutral-100 border-neutral-300 text-neutral-900';
      case 'glass':
        return 'bg-gradient-to-br from-indigo-950/40 via-purple-950/30 to-background border-indigo-500/20';
      default:
        return 'bg-background/70 border-border/80';
    }
  };

  return (
    <div className="p-5 rounded-lg border border-border/80 bg-card/60 backdrop-blur-xs space-y-5">
      {/* Header */}
      <div className="flex items-start justify-between gap-4">
        <div className="space-y-1">
          <div className="flex items-center gap-2">
            <Sparkles className="w-4 h-4 text-indigo-400" />
            <h3 className="text-sm font-semibold text-foreground">Meeting Pill Workbench</h3>
            <Badge
              variant="outline"
              className="text-[10px] font-mono border-indigo-500/30 text-indigo-400 bg-indigo-500/5 uppercase"
            >
              Styles & Free Movement
            </Badge>
          </div>
          <p className="text-xs text-muted-foreground leading-relaxed max-w-xl">
            Interactive desktop meeting pill preview. Switch between horizontal and vertical styles,
            select snap presets, or drag the pill freely across the screen.
          </p>
        </div>

        <Button
          variant="outline"
          size="sm"
          onClick={handleResetToDefault}
          className="text-xs h-7 gap-1.5 shrink-0 text-muted-foreground hover:text-foreground"
          title="Reset to defaults"
        >
          <RotateCcw className="w-3 h-3" />
          Reset
        </Button>
      </div>

      {/* Pill Style & Screen Positioning Bar */}
      <div className="grid grid-cols-1 lg:grid-cols-12 gap-3 p-3.5 rounded-lg border border-border/60 bg-background/50">
        {/* Style Selector */}
        <div className="lg:col-span-4 space-y-2">
          <span className="text-xs font-semibold text-foreground flex items-center gap-1.5">
            <Rows2 className="w-3.5 h-3.5 text-muted-foreground" />
            Pill Style
          </span>
          <div className="flex items-center gap-1.5">
            <Button
              variant={style === 'horizontal' ? 'default' : 'outline'}
              size="sm"
              onClick={() => setStyle('horizontal')}
              className="text-xs h-8 flex-1 gap-1.5"
            >
              <Rows2 className="w-3.5 h-3.5 rotate-90" />
              Horizontal (Default)
            </Button>
            <Button
              variant={style === 'vertical' ? 'default' : 'outline'}
              size="sm"
              onClick={() => setStyle('vertical')}
              className="text-xs h-8 flex-1 gap-1.5"
            >
              <Columns2 className="w-3.5 h-3.5" />
              Vertical
            </Button>
          </div>
        </div>

        {/* Positioning Matrix & Free Movement */}
        <div className="lg:col-span-8 space-y-2">
          <div className="flex items-center justify-between">
            <span className="text-xs font-semibold text-foreground flex items-center gap-1.5">
              <Move className="w-3.5 h-3.5 text-muted-foreground" />
              Screen Position{' '}
              <span className="font-mono text-[11px] text-muted-foreground font-normal">
                {isFreeMovement
                  ? `(free: X ${freePos.x}%, Y ${freePos.y}%)`
                  : `(${verticalAlign} · ${horizontalAlign})`}
              </span>
            </span>

            <Button
              variant={isFreeMovement ? 'default' : 'outline'}
              size="sm"
              onClick={() => setIsFreeMovement(true)}
              className="text-[11px] h-6 px-2.5 gap-1 font-medium"
              title="Drag and position the pill anywhere on the screen"
            >
              <GripHorizontal className="w-3 h-3" />
              Free Movement
            </Button>
          </div>

          {/* Mini 3x3 Monitor Grid */}
          <div className="grid grid-cols-3 gap-1.5 max-w-sm">
            {POSITION_MATRIX.map((pos) => {
              const isSelected = !isFreeMovement && verticalAlign === pos.v && horizontalAlign === pos.h;
              return (
                <button
                  key={`${pos.v}-${pos.h}`}
                  type="button"
                  onClick={() => {
                    setIsFreeMovement(false);
                    setVerticalAlign(pos.v);
                    setHorizontalAlign(pos.h);
                  }}
                  className={`text-[11px] py-1 px-2 rounded border font-mono transition-colors cursor-pointer ${
                    isSelected
                      ? 'bg-primary text-primary-foreground border-primary font-bold shadow-xs'
                      : 'bg-card/80 border-border/70 text-muted-foreground hover:bg-accent hover:text-foreground'
                  }`}
                  title={`${pos.label} (${pos.v} / ${pos.h})`}
                >
                  {pos.shortLabel} · {pos.label}
                </button>
              );
            })}
          </div>
        </div>
      </div>

      {/* Simulated Desktop Screen Stage */}
      <div className="space-y-1.5">
        <div className="flex items-center justify-between text-[11px] text-muted-foreground px-1">
          <span className="flex items-center gap-1.5">
            <span className="inline-block w-2 h-2 rounded-full bg-emerald-500/70" />
            Simulated Desktop Screen (1920 × 1080)
          </span>
          <span className="font-mono">
            Anchor:{' '}
            {isFreeMovement
              ? `free_${freePos.x}_${freePos.y}`
              : `${verticalAlign}_${horizontalAlign}`}{' '}
            · Style: {style}
          </span>
        </div>

        <div
          ref={viewportRef}
          className={`relative min-h-[320px] flex flex-col p-6 rounded-lg border transition-all duration-200 overflow-hidden shadow-inner ${
            isDragging ? 'cursor-grabbing select-none' : ''
          } ${getBackdropClass()}`}
        >
          {/* Surface Indicator */}
          <div className="absolute top-2.5 left-3 flex items-center gap-1.5 text-[10px] font-mono opacity-50 select-none">
            <Layers className="w-3 h-3" />
            <span>Surface: {backdrop}</span>
          </div>

          {/* Desktop Anchor Bounds or Free Movement Container */}
          {isFreeMovement ? (
            <div
              ref={pillContainerRef}
              style={{
                position: 'absolute',
                left: `${freePos.x}%`,
                top: `${freePos.y}%`,
                transform: 'translate(-50%, -50%)',
              }}
              onPointerDown={handlePointerDown}
              className={`cursor-grab touch-none ${isDragging ? 'cursor-grabbing' : ''}`}
              title="Click and drag anywhere on the pill to reposition freely"
            >
              <div className="group relative">
                <div className="absolute -top-3.5 left-1/2 -translate-x-1/2 opacity-0 group-hover:opacity-80 transition-opacity text-[9px] font-mono bg-background/90 px-1.5 py-0.5 rounded shadow-xs pointer-events-none flex items-center gap-1 border border-border">
                  <GripHorizontal className="w-2.5 h-2.5" />
                  Drag
                </div>
                <MeetingPill
                  orientation={style}
                  state={state}
                  elapsedSeconds={elapsedSec}
                  isSystemAudioActive={isSystemAudioActive}
                  micLevels={micLevels}
                  sysLevels={sysLevels}
                  forceExpanded={forceExpanded}
                  onTogglePause={handleTogglePause}
                  onStop={handleStop}
                />
              </div>
            </div>
          ) : (
            <div
              className={`w-full h-full flex-1 flex flex-col ${getViewportPositionClasses()} transition-all duration-200`}
            >
              <div
                onPointerDown={handlePointerDown}
                className="cursor-grab active:cursor-grabbing touch-none group relative"
                title="Click and drag to switch to Free Movement"
              >
                <div className="absolute -top-3.5 left-1/2 -translate-x-1/2 opacity-0 group-hover:opacity-80 transition-opacity text-[9px] font-mono bg-background/90 px-1.5 py-0.5 rounded shadow-xs pointer-events-none flex items-center gap-1 border border-border">
                  <GripHorizontal className="w-2.5 h-2.5" />
                  Drag freely
                </div>
                <MeetingPill
                  orientation={style}
                  state={state}
                  elapsedSeconds={elapsedSec}
                  isSystemAudioActive={isSystemAudioActive}
                  micLevels={micLevels}
                  sysLevels={sysLevels}
                  forceExpanded={forceExpanded}
                  onTogglePause={handleTogglePause}
                  onStop={handleStop}
                />
              </div>
            </div>
          )}

          {isFreeMovement && (
            <div className="absolute bottom-2.5 right-3 text-[10px] font-mono text-muted-foreground/60 select-none flex items-center gap-1.5 pointer-events-none">
              <span className="w-1.5 h-1.5 rounded-full bg-emerald-500/70" />
              Viewport boundaries locked (cannot exit screen)
            </div>
          )}

          <p className="text-[11px] text-muted-foreground/60 text-center mt-auto pt-4 select-none">
            {isFreeMovement
              ? `Free Movement active (${freePos.x}%, ${freePos.y}%). Click and drag the pill anywhere, or pick a preset above to snap.`
              : 'Hover the pill to expand controls. Drag the pill to move freely, or click any position preset above.'}
          </p>
        </div>
      </div>

      {/* Action Feedback status */}
      {lastAction && (
        <div
          role="status"
          className="flex items-center gap-2 text-[11px] font-medium text-emerald-500 bg-emerald-500/10 border border-emerald-500/20 px-3 py-1.5 rounded-md"
        >
          <CheckCircle2 className="w-3.5 h-3.5 shrink-0" />
          <span>{lastAction}</span>
        </div>
      )}

      {/* Interactive Testing Controls Grid */}
      <div className="grid grid-cols-1 md:grid-cols-2 gap-4 pt-1">
        {/* State Selection */}
        <div className="p-3.5 rounded-md border border-border/60 bg-background/40 space-y-2.5">
          <label className="text-xs font-semibold text-foreground flex items-center gap-1.5">
            <span className="w-2 h-2 rounded-full bg-indigo-500" />
            Capsule State
          </label>
          <div className="flex flex-wrap gap-1.5">
            <Button
              variant={state === 'recording' ? 'default' : 'outline'}
              size="sm"
              onClick={() => setState('recording')}
              className="text-xs h-7 gap-1.5"
            >
              <span className="w-1.5 h-1.5 rounded-full bg-red-400 animate-ping" />
              Recording
            </Button>
            <Button
              variant={state === 'paused' ? 'default' : 'outline'}
              size="sm"
              onClick={() => setState('paused')}
              className="text-xs h-7 gap-1.5"
            >
              <span className="w-1.5 h-1.5 rounded-full bg-amber-400" />
              Paused
            </Button>
            <Button
              variant={state === 'transcribing' ? 'default' : 'outline'}
              size="sm"
              onClick={() => setState('transcribing')}
              className="text-xs h-7 gap-1.5"
            >
              <span className="w-1.5 h-1.5 rounded-full bg-indigo-400" />
              Transcribing
            </Button>
          </div>
        </div>

        {/* Timer & Duration Tester */}
        <div className="p-3.5 rounded-md border border-border/60 bg-background/40 space-y-2.5">
          <label className="text-xs font-semibold text-foreground flex items-center gap-1.5">
            <Clock className="w-3.5 h-3.5 text-muted-foreground" />
            Duration Layout ({elapsedSec}s)
          </label>
          <div className="flex items-center gap-1.5">
            <Button
              variant="outline"
              size="sm"
              onClick={() => setElapsedSec(0)}
              className="text-xs h-7"
            >
              00:00
            </Button>
            <Button
              variant="outline"
              size="sm"
              onClick={() => setElapsedSec((prev) => prev + 60)}
              className="text-xs h-7"
            >
              +1 min
            </Button>
            <Button
              variant="outline"
              size="sm"
              onClick={() => setElapsedSec((prev) => prev + 3600)}
              className="text-xs h-7"
              title="Test layout with 1+ hour meeting timer"
            >
              +1 hour
            </Button>
          </div>
        </div>

        {/* Audio Simulation Options */}
        <div className="p-3.5 rounded-md border border-border/60 bg-background/40 space-y-3">
          <div className="flex items-center justify-between">
            <label className="text-xs font-semibold text-foreground flex items-center gap-1.5">
              <Mic className="w-3.5 h-3.5 text-muted-foreground" />
              Simulate Live Audio
            </label>
            <Switch
              checked={isSimulatingAudio}
              onCheckedChange={setIsSimulatingAudio}
              aria-label="Toggle audio level simulation"
            />
          </div>

          <div className="flex items-center gap-1.5">
            <span className="text-[11px] text-muted-foreground mr-1">Activity:</span>
            <Button
              variant={audioProfile === 'conversation' ? 'secondary' : 'outline'}
              size="sm"
              disabled={!isSimulatingAudio}
              onClick={() => setAudioProfile('conversation')}
              className="text-[11px] h-6 px-2"
            >
              Both
            </Button>
            <Button
              variant={audioProfile === 'mic_only' ? 'secondary' : 'outline'}
              size="sm"
              disabled={!isSimulatingAudio}
              onClick={() => setAudioProfile('mic_only')}
              className="text-[11px] h-6 px-2"
            >
              Mic only
            </Button>
            <Button
              variant={audioProfile === 'sys_only' ? 'secondary' : 'outline'}
              size="sm"
              disabled={!isSimulatingAudio}
              onClick={() => setAudioProfile('sys_only')}
              className="text-[11px] h-6 px-2"
            >
              System only
            </Button>
          </div>
        </div>

        {/* Visual Inspection & Edge Cases */}
        <div className="p-3.5 rounded-md border border-border/60 bg-background/40 space-y-3">
          <div className="flex items-center justify-between">
            <label className="text-xs font-medium text-foreground flex items-center gap-1.5">
              <Maximize2 className="w-3.5 h-3.5 text-muted-foreground" />
              Force expand controls
            </label>
            <Switch
              checked={forceExpanded}
              onCheckedChange={setForceExpanded}
              aria-label="Force expand meeting pill controls"
            />
          </div>

          <div className="flex items-center justify-between">
            <label className="text-xs font-medium text-foreground flex items-center gap-1.5">
              <AlertTriangle className="w-3.5 h-3.5 text-amber-500" />
              Missing system audio
            </label>
            <Switch
              checked={!isSystemAudioActive}
              onCheckedChange={(checked) => setIsSystemAudioActive(!checked)}
              aria-label="Toggle missing system audio warning"
            />
          </div>
        </div>
      </div>

      {/* Surface Backdrop Selector */}
      <div className="flex items-center justify-between pt-1 border-t border-border/40">
        <span className="text-xs text-muted-foreground flex items-center gap-1.5">
          <Layers className="w-3.5 h-3.5" />
          Test against surface:
        </span>
        <div className="flex items-center gap-1.5">
          {(['default', 'dark', 'light', 'glass'] as BackdropStyle[]).map((mode) => (
            <Button
              key={mode}
              variant={backdrop === mode ? 'secondary' : 'outline'}
              size="sm"
              onClick={() => setBackdrop(mode)}
              className="text-[11px] h-6 px-2.5 capitalize"
            >
              {mode}
            </Button>
          ))}
        </div>
      </div>
    </div>
  );
};
