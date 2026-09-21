import React, { useCallback, useEffect, useId, useMemo, useRef, useState } from 'react';
import { Maximize2, Minus, Move, Plus, Scan } from 'lucide-react';

import { Button } from '@/components/ui/button';
import {
  MIN_LEGIBLE_SCALE,
  PANEL_MAX_HEIGHT,
  clampPan,
  formatScale,
  initialPan,
  initialScale,
  isOverflowing,
  panStep,
  panelHeight,
  parseSvgSize,
  zoomAbout,
  zoomBy,
  ZOOM_STEP,
  type Pan,
  type Size,
} from '@/lib/diagramViewport';
import { sanitizeSvgMarkup } from '@/lib/svgSafety';
import { cn } from '@/lib/utils';

interface DiagramViewportProps {
  /** Rendered SVG markup. Sanitized here, so no caller has to remember to. */
  svg: string;
  /** What the diagram is, for the accessible name — "Flowchart (LR)". */
  label: string;
  /** `panel` sits in the document and sizes to its diagram; `stage` fills a dialog. */
  variant?: 'panel' | 'stage';
  /** Supplied by the panel variant to open the full-screen view. */
  onExpand?: () => void;
  className?: string;
}

/**
 * A diagram you can move around in.
 *
 * Summaries produce flowcharts several times wider than the panel they land
 * in. Scaling those to fit makes a picture of a diagram — every label below
 * body-text size — so past a point this stops shrinking and starts panning
 * instead: the diagram opens at a legible size and the reader moves through
 * it with the mouse, the arrow keys, or full screen.
 *
 * All the arithmetic lives in `@/lib/diagramViewport`, which is where it is
 * tested: jsdom has no layout, so a test rendering this component would
 * measure zeroes and prove nothing.
 */
export const DiagramViewport: React.FC<DiagramViewportProps> = ({
  svg,
  label,
  variant = 'panel',
  onExpand,
  className,
}) => {
  const isStage = variant === 'stage';
  const viewportRef = useRef<HTMLDivElement>(null);
  const hintId = `${useId().replace(/[^a-zA-Z0-9_-]/g, 'd')}-hint`;

  const markup = useMemo(() => sanitizeSvgMarkup(svg), [svg]);
  const diagram = useMemo(() => parseSvgSize(markup), [markup]);

  const [viewport, setViewport] = useState<Size>({ width: 0, height: 0 });
  const [scale, setScale] = useState(MIN_LEGIBLE_SCALE);
  const [pan, setPan] = useState<Pan>({ x: 0, y: 0 });
  const [isAdjusted, setIsAdjusted] = useState(false);
  const [isDragging, setIsDragging] = useState(false);
  const dragOrigin = useRef<{ pointer: Pan; pan: Pan } | null>(null);

  // The measured box the diagram has to live in.
  useEffect(() => {
    const node = viewportRef.current;
    if (!node) return;
    const observer = new ResizeObserver(([entry]) => {
      setViewport({
        width: entry.contentRect.width,
        height: entry.contentRect.height,
      });
    });
    observer.observe(node);
    return () => observer.disconnect();
  }, []);

  /**
   * The box the opening zoom is chosen against.
   *
   * A panel's height follows its diagram, so measuring the panel to decide
   * the zoom that decides the panel's height would be circular. The ceiling
   * stands in for the height instead, and the panel then shrinks to whatever
   * the diagram actually needs.
   */
  const fitBox: Size = useMemo(
    () => ({
      width: viewport.width,
      height: isStage ? viewport.height : PANEL_MAX_HEIGHT,
    }),
    [viewport.width, viewport.height, isStage],
  );

  // A new diagram, or a resize the reader has not overridden, opens fitted.
  useEffect(() => {
    if (isAdjusted || !diagram || fitBox.width <= 0) return;
    const s = initialScale(diagram, fitBox, isStage);
    setScale(s);
    setPan(initialPan(diagram, viewport.height > 0 ? viewport : fitBox, s));
  }, [diagram, fitBox, isAdjusted, isStage, viewport]);

  // A new diagram is a fresh start, whatever was done to the last one.
  useEffect(() => {
    setIsAdjusted(false);
  }, [markup]);

  const applyScale = useCallback(
    (next: number, focus?: Pan) => {
      if (!diagram) return;
      // The focus defaults to the middle of the viewport, which is what a
      // keyboard zoom should hold still; the wheel passes the cursor.
      const centre = focus ?? { x: viewport.width / 2, y: viewport.height / 2 };
      setIsAdjusted(true);
      // Two plain setters rather than a pan computed inside the scale
      // updater: an updater that calls another setter is not pure, and Strict
      // Mode invokes it twice, which would apply the zoom's pan correction
      // twice and walk the diagram sideways on every click in development.
      setPan((currentPan) =>
        clampPan(zoomAbout(currentPan, centre, scale, next), diagram, viewport, next),
      );
      setScale(next);
    },
    [diagram, scale, viewport],
  );

  const movePan = useCallback(
    (dx: number, dy: number) => {
      if (!diagram) return;
      setIsAdjusted(true);
      setPan((current) => clampPan({ x: current.x + dx, y: current.y + dy }, diagram, viewport, scale));
    },
    [diagram, viewport, scale],
  );

  const resetToFit = useCallback(() => {
    if (!diagram) return;
    setIsAdjusted(false);
    const s = initialScale(diagram, fitBox, isStage);
    setScale(s);
    setPan(initialPan(diagram, viewport.height > 0 ? viewport : fitBox, s));
  }, [diagram, fitBox, isStage, viewport]);

  // Wheel is attached by hand because React's is passive, and a passive
  // listener cannot stop the page scrolling under a zoom.
  useEffect(() => {
    const node = viewportRef.current;
    if (!node || !diagram) return;

    const onWheel = (event: WheelEvent) => {
      if (event.ctrlKey || event.metaKey) {
        event.preventDefault();
        const box = node.getBoundingClientRect();
        applyScale(zoomBy(scale, event.deltaY < 0 ? ZOOM_STEP : 1 / ZOOM_STEP), {
          x: event.clientX - box.left,
          y: event.clientY - box.top,
        });
        return;
      }
      if (!isOverflowing(diagram, viewport, scale) && !isStage) return;
      event.preventDefault();
      movePan(-event.deltaX, -event.deltaY);
    };

    node.addEventListener('wheel', onWheel, { passive: false });
    return () => node.removeEventListener('wheel', onWheel);
  }, [applyScale, diagram, isStage, movePan, scale, viewport]);

  const handleKeyDown = (event: React.KeyboardEvent<HTMLDivElement>) => {
    const step = panStep(event.shiftKey);
    switch (event.key) {
      case 'ArrowLeft':
        event.preventDefault();
        movePan(step, 0);
        return;
      case 'ArrowRight':
        event.preventDefault();
        movePan(-step, 0);
        return;
      case 'ArrowUp':
        event.preventDefault();
        movePan(0, step);
        return;
      case 'ArrowDown':
        event.preventDefault();
        movePan(0, -step);
        return;
      case '+':
      case '=':
        event.preventDefault();
        applyScale(zoomBy(scale, ZOOM_STEP));
        return;
      case '-':
      case '_':
        event.preventDefault();
        applyScale(zoomBy(scale, 1 / ZOOM_STEP));
        return;
      case '0':
        event.preventDefault();
        resetToFit();
        return;
      case 'f':
      case 'F':
        if (onExpand) {
          event.preventDefault();
          onExpand();
        }
        return;
      default:
    }
  };

  const handlePointerDown = (event: React.PointerEvent<HTMLDivElement>) => {
    if (event.button !== 0 || !diagram) return;
    event.currentTarget.setPointerCapture(event.pointerId);
    dragOrigin.current = { pointer: { x: event.clientX, y: event.clientY }, pan };
    setIsDragging(true);
  };

  const handlePointerMove = (event: React.PointerEvent<HTMLDivElement>) => {
    const origin = dragOrigin.current;
    if (!origin || !diagram) return;
    setIsAdjusted(true);
    setPan(
      clampPan(
        {
          x: origin.pan.x + (event.clientX - origin.pointer.x),
          y: origin.pan.y + (event.clientY - origin.pointer.y),
        },
        diagram,
        viewport,
        scale,
      ),
    );
  };

  const endDrag = (event: React.PointerEvent<HTMLDivElement>) => {
    if (!dragOrigin.current) return;
    if (event.currentTarget.hasPointerCapture(event.pointerId)) {
      event.currentTarget.releasePointerCapture(event.pointerId);
    }
    dragOrigin.current = null;
    setIsDragging(false);
  };

  const overflowing = diagram ? isOverflowing(diagram, viewport, scale) : false;
  const height = isStage ? undefined : panelHeight(diagram, scale);

  return (
    <div className={cn('flex flex-col', isStage ? 'h-full min-h-0' : 'w-full', className)}>
      <div
        ref={viewportRef}
        role="group"
        tabIndex={0}
        aria-label={`${label}, pannable diagram`}
        aria-describedby={hintId}
        onKeyDown={handleKeyDown}
        onPointerDown={handlePointerDown}
        onPointerMove={handlePointerMove}
        onPointerUp={endDrag}
        onPointerCancel={endDrag}
        style={isStage ? undefined : { height: `${height}px`, minHeight: `${height}px` }}
        className={cn(
          'relative overflow-hidden bg-card/60 outline-hidden select-none touch-none',
          isStage ? 'flex-1 min-h-0' : 'w-full',
          'focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-inset',
          isDragging ? 'cursor-grabbing' : 'cursor-grab',
        )}
      >
        <p id={hintId} className="sr-only">
          Arrow keys pan the diagram, shift and an arrow key pans further, plus and minus zoom,
          zero fits it to the panel
          {onExpand ? ', and F opens it full screen.' : '.'}
        </p>
        <div className="absolute inset-0 overflow-hidden">
          <div
            aria-hidden="true"
            style={{
              width: diagram ? `${diagram.width}px` : undefined,
              height: diagram ? `${diagram.height}px` : undefined,
              transform: `translate(${pan.x}px, ${pan.y}px) scale(${scale})`,
              transformOrigin: '0 0',
            }}
            className="[&>svg]:block [&>svg]:w-full [&>svg]:h-full [&>svg]:max-w-none pointer-events-none select-none"
            dangerouslySetInnerHTML={{ __html: markup }}
          />
        </div>
      </div>

      <div className="flex items-center justify-between gap-2 border-t border-border/60 bg-muted/30 px-2 py-1 select-none">
        <div className="flex items-center gap-1 text-[11px] text-muted-foreground">
          {overflowing || isStage ? (
            <>
              <Move className="h-3 w-3" aria-hidden="true" />
              <span>Drag or use arrow keys</span>
            </>
          ) : (
            <span className="pl-1">Whole diagram shown</span>
          )}
        </div>

        <div className="flex items-center gap-0.5">
          <Button
            size="sm"
            variant="ghost"
            className="h-6 w-6 p-0 text-muted-foreground hover:text-foreground"
            onClick={() => applyScale(zoomBy(scale, 1 / ZOOM_STEP))}
            disabled={!diagram}
            aria-label="Zoom out"
            title="Zoom out (−)"
          >
            <Minus className="h-3 w-3" />
          </Button>
          <span
            aria-live="polite"
            className="min-w-11 text-center font-mono text-[11px] text-muted-foreground"
          >
            {formatScale(scale)}
          </span>
          <Button
            size="sm"
            variant="ghost"
            className="h-6 w-6 p-0 text-muted-foreground hover:text-foreground"
            onClick={() => applyScale(zoomBy(scale, ZOOM_STEP))}
            disabled={!diagram}
            aria-label="Zoom in"
            title="Zoom in (+)"
          >
            <Plus className="h-3 w-3" />
          </Button>
          <Button
            size="sm"
            variant="ghost"
            className="h-6 w-6 p-0 text-muted-foreground hover:text-foreground"
            onClick={resetToFit}
            disabled={!diagram}
            aria-label="Fit diagram to the panel"
            title="Fit (0)"
          >
            <Scan className="h-3 w-3" />
          </Button>
          {onExpand ? (
            <Button
              size="sm"
              variant="ghost"
              className="h-6 w-6 p-0 text-muted-foreground hover:text-foreground"
              onClick={onExpand}
              aria-label="Open the diagram full screen"
              title="Full screen (F)"
            >
              <Maximize2 className="h-3 w-3" />
            </Button>
          ) : null}
        </div>
      </div>
    </div>
  );
};
