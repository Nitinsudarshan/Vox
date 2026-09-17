import React, { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { Search } from 'lucide-react';

import type { KnowledgeGraphData, KnowledgeNode } from '@/types';

import { applyVisibility, bandOf, layoutRings, RING_BANDS, type RingBand } from './graph/ringsLayout';
import {
  ENTRY_ANIMATION_MS,
  hitTestRings,
  prefersReducedMotion,
  renderRings,
} from './graph/ringsRenderer';
import {
  edgeColorFor,
  GHOST_EDGE_COLOR,
  isSuggestedEdge,
  RELAY_COLOR_MAP,
  type CameraState,
  type SimNode,
} from './graph/graphTypes';
import { GraphNodeInspector } from './graph/GraphNodeInspector';

const BAND_LABELS: Record<RingBand, string> = {
  projects: 'Projects',
  areas: 'Areas',
  resources: 'Resources',
  archive: 'Archive',
  uncategorised: 'Unfiled',
};

interface RingsViewProps {
  graphData: KnowledgeGraphData;
  onOpenScribbleEditor?: (id: string) => void;
  onSetPara?: (scribbleId: string, band: RingBand | null) => void;
}

/**
 * The Rings view: PARA as concentric bands, position carrying meaning.
 *
 * The layout is computed from the full node set exactly once per graph
 * change and never from a filtered subset — see `ringsLayout`. Everything
 * the user does here that looks like filtering (search, muting a band)
 * resolves to an opacity map, so nothing ever moves in response to a filter.
 */
export const RingsView: React.FC<RingsViewProps> = ({
  graphData,
  onOpenScribbleEditor,
  onSetPara,
}) => {
  const containerRef = useRef<HTMLDivElement | null>(null);
  const canvasRef = useRef<HTMLCanvasElement | null>(null);

  const [query, setQuery] = useState('');
  const [mutedBands, setMutedBands] = useState<Set<RingBand>>(new Set());
  const [hoveredNodeId, setHoveredNodeId] = useState<string | null>(null);
  const [selectedNodeId, setSelectedNodeId] = useState<string | null>(null);
  const [camera, setCamera] = useState<CameraState>({ x: 0, y: 0, k: 0.42 });

  const panRef = useRef<{ active: boolean; x: number; y: number }>({
    active: false,
    x: 0,
    y: 0,
  });

  /**
   * Entry animation progress, held in a ref rather than state.
   *
   * The loop drives the canvas directly: routing 600ms of frames through
   * `setState` would re-render the whole subtree ~36 times to move pixels
   * React does not own.
   */
  const entryRef = useRef(1);
  const frameRef = useRef<number | null>(null);

  // The layout depends on the graph and nothing else. Not on the filters,
  // not on the viewport, not on the camera — which is what makes the same
  // vault produce the same picture every time it is opened.
  const layout = useMemo(
    () => layoutRings(graphData.nodes, graphData.edges),
    [graphData],
  );

  const nodesById = useMemo(
    () => new Map(graphData.nodes.map((n) => [n.id, n])),
    [graphData.nodes],
  );

  const adjacency = useMemo(() => {
    const adj = new Map<string, Set<string>>();
    for (const edge of graphData.edges) {
      if (!adj.has(edge.source_id)) adj.set(edge.source_id, new Set());
      if (!adj.has(edge.target_id)) adj.set(edge.target_id, new Set());
      adj.get(edge.source_id)!.add(edge.target_id);
      adj.get(edge.target_id)!.add(edge.source_id);
    }
    return adj;
  }, [graphData.edges]);

  const opacity = useMemo(() => {
    const needle = query.trim().toLowerCase();
    return applyVisibility(layout, (id) => {
      const node = nodesById.get(id);
      if (!node) return false;
      if (mutedBands.has(bandOf(node))) return false;
      if (!needle) return true;
      return (
        node.label.toLowerCase().includes(needle) ||
        (node.summary ?? '').toLowerCase().includes(needle)
      );
    });
  }, [layout, nodesById, query, mutedBands]);

  const draw = useCallback(() => {
    const canvas = canvasRef.current;
    const ctx = canvas?.getContext('2d');
    if (!canvas || !ctx) return;

    renderRings({
      canvas,
      ctx,
      layout,
      nodesById,
      edges: graphData.edges,
      opacity,
      adjacency,
      camera,
      hoveredNodeId,
      selectedNodeId,
      entryProgress: entryRef.current,
    });
  }, [layout, nodesById, graphData.edges, opacity, adjacency, camera, hoveredNodeId, selectedNodeId]);

  // The animation loop calls the latest `draw` without depending on its
  // identity, so a hover does not restart the entry animation.
  const drawRef = useRef(draw);
  drawRef.current = draw;

  /**
   * Nodes ease out to their final coordinates once, then the loop stops.
   *
   * No ambient drift and no idle physics: the universe settles and stays
   * settled, which is what makes the layout's determinism visible rather
   * than merely true. Re-runs when the layout changes, because a new vault
   * is a new arrival, not a continuation.
   */
  useEffect(() => {
    if (frameRef.current !== null) {
      cancelAnimationFrame(frameRef.current);
      frameRef.current = null;
    }

    if (prefersReducedMotion()) {
      entryRef.current = 1;
      drawRef.current();
      return;
    }

    entryRef.current = 0;
    const started = performance.now();

    const step = (now: number) => {
      const progress = Math.min(1, (now - started) / ENTRY_ANIMATION_MS);
      entryRef.current = progress;
      drawRef.current();
      if (progress < 1) {
        frameRef.current = requestAnimationFrame(step);
      } else {
        frameRef.current = null;
      }
    };

    frameRef.current = requestAnimationFrame(step);

    return () => {
      if (frameRef.current !== null) {
        cancelAnimationFrame(frameRef.current);
        frameRef.current = null;
      }
    };
  }, [layout]);

  // Size the canvas to its container at device resolution.
  useEffect(() => {
    const canvas = canvasRef.current;
    const container = containerRef.current;
    if (!canvas || !container) return;

    const resize = () => {
      const dpr = window.devicePixelRatio || 1;
      canvas.width = container.clientWidth * dpr;
      canvas.height = container.clientHeight * dpr;
      canvas.style.width = `${container.clientWidth}px`;
      canvas.style.height = `${container.clientHeight}px`;
      draw();
    };

    resize();
    const observer = new ResizeObserver(resize);
    observer.observe(container);
    return () => observer.disconnect();
  }, [draw]);

  useEffect(() => {
    draw();
  }, [draw]);

  const pointerToCanvas = (event: React.PointerEvent | React.MouseEvent) => {
    const canvas = canvasRef.current;
    if (!canvas) return null;
    const rect = canvas.getBoundingClientRect();
    return { x: event.clientX - rect.left, y: event.clientY - rect.top };
  };

  const handlePointerMove = (event: React.PointerEvent<HTMLCanvasElement>) => {
    if (panRef.current.active) {
      const dx = event.clientX - panRef.current.x;
      const dy = event.clientY - panRef.current.y;
      panRef.current = { active: true, x: event.clientX, y: event.clientY };
      setCamera((prev) => ({ ...prev, x: prev.x + dx, y: prev.y + dy }));
      return;
    }

    const point = pointerToCanvas(event);
    const canvas = canvasRef.current;
    if (!point || !canvas) return;
    const dpr = window.devicePixelRatio || 1;
    const hit = hitTestRings(
      layout,
      point.x,
      point.y,
      canvas.width / dpr,
      canvas.height / dpr,
      camera,
      opacity,
    );
    setHoveredNodeId(hit);
  };

  const handlePointerDown = (event: React.PointerEvent<HTMLCanvasElement>) => {
    const point = pointerToCanvas(event);
    const canvas = canvasRef.current;
    if (!point || !canvas) return;
    const dpr = window.devicePixelRatio || 1;
    const hit = hitTestRings(
      layout,
      point.x,
      point.y,
      canvas.width / dpr,
      canvas.height / dpr,
      camera,
      opacity,
    );

    if (hit) {
      // Hover raises; a click pins that state and opens the inspector.
      setSelectedNodeId(hit);
      return;
    }
    panRef.current = { active: true, x: event.clientX, y: event.clientY };
    setSelectedNodeId(null);
  };

  const endPan = () => {
    panRef.current = { active: false, x: 0, y: 0 };
  };

  const handleWheel = (event: React.WheelEvent<HTMLCanvasElement>) => {
    const factor = event.deltaY < 0 ? 1.12 : 1 / 1.12;
    setCamera((prev) => ({ ...prev, k: Math.min(4, Math.max(0.12, prev.k * factor)) }));
  };

  const toggleBand = (band: RingBand) => {
    setMutedBands((prev) => {
      const next = new Set(prev);
      if (next.has(band)) next.delete(band);
      else next.add(band);
      return next;
    });
  };

  /** The inspector speaks `SimNode`; a ring node carries the same identity
   *  plus a position, so adapting is a projection rather than a conversion. */
  const asSimNode = (node: KnowledgeNode): SimNode => {
    const placed = layout.byId.get(node.id);
    return {
      ...node,
      x: placed?.x ?? 0,
      y: placed?.y ?? 0,
      vx: 0,
      vy: 0,
      radius: placed?.size ?? 6,
      color: RELAY_COLOR_MAP[node.node_type] ?? RELAY_COLOR_MAP.default,
    };
  };

  const selectedNode = selectedNodeId ? nodesById.get(selectedNodeId) : undefined;
  const selectedNeighbours = selectedNodeId
    ? [...(adjacency.get(selectedNodeId) ?? [])]
        .map((id) => nodesById.get(id))
        .filter((n): n is KnowledgeNode => Boolean(n))
        .map(asSimNode)
    : [];

  /**
   * The relationship types actually present, for the legend.
   *
   * Only what is on screen: a legend listing every type Vox can store would
   * mostly describe links this vault does not have, which teaches the user
   * to ignore it.
   */
  const edgeLegend = useMemo(() => {
    const kinds = new Map<string, { color: string; suggested: boolean; count: number }>();
    for (const edge of graphData.edges) {
      const suggested = isSuggestedEdge(edge);
      const label = suggested ? 'suggested' : edge.relationship.toLowerCase();
      const existing = kinds.get(label);
      if (existing) {
        existing.count += 1;
        continue;
      }
      kinds.set(label, {
        color: suggested ? GHOST_EDGE_COLOR : edgeColorFor(edge.relationship),
        suggested,
        count: 1,
      });
    }
    return [...kinds.entries()].sort((a, b) => b[1].count - a[1].count);
  }, [graphData.edges]);

  const bandCounts = useMemo(() => {
    const counts = new Map<RingBand, number>(RING_BANDS.map((b) => [b, 0]));
    for (const band of layout.bands) counts.set(band.band, band.count);
    return counts;
  }, [layout]);

  return (
    <div className="flex-1 flex flex-col min-h-0 min-w-0">
      <div className="flex items-center gap-2 px-3 py-2 border-b border-border/60 flex-wrap">
        <div className="relative">
          <Search className="absolute left-2 top-1/2 -translate-y-1/2 w-3.5 h-3.5 text-muted-foreground" />
          <input
            type="search"
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            placeholder="Dim everything but…"
            aria-label="Filter the graph by name"
            className="h-7 w-52 pl-7 pr-2 text-xs bg-background border border-border rounded-md text-foreground placeholder:text-muted-foreground focus:outline-hidden focus:ring-1 focus:ring-ring"
          />
        </div>

        <div className="flex items-center gap-1 flex-wrap">
          {RING_BANDS.map((band) => {
            const isMuted = mutedBands.has(band);
            return (
              <button
                key={band}
                type="button"
                onClick={() => toggleBand(band)}
                aria-pressed={!isMuted}
                className={`h-7 px-2 rounded-md text-xs font-medium border transition-colors ${
                  isMuted
                    ? 'border-border/60 text-muted-foreground/60 bg-transparent'
                    : 'border-border bg-background text-foreground hover:bg-accent'
                }`}
              >
                {BAND_LABELS[band]}
                <span className="ml-1 font-mono text-muted-foreground">
                  {bandCounts.get(band) ?? 0}
                </span>
              </button>
            );
          })}
        </div>

        <p className="text-[11px] text-muted-foreground ml-auto">
          Filtering dims — nothing moves.
        </p>
      </div>

      <div ref={containerRef} className="relative flex-1 min-h-0 overflow-hidden">
        <canvas
          ref={canvasRef}
          className="absolute inset-0 touch-none"
          style={{ cursor: hoveredNodeId ? 'pointer' : 'grab' }}
          onPointerMove={handlePointerMove}
          onPointerDown={handlePointerDown}
          onPointerUp={endPan}
          onPointerLeave={() => {
            endPan();
            setHoveredNodeId(null);
          }}
          onWheel={handleWheel}
        />

        {edgeLegend.length > 0 && (
          <div className="absolute top-3 right-3 bg-card/90 backdrop-blur-xs border border-border/80 rounded-lg px-2.5 py-2 space-y-1 max-w-[190px] pointer-events-none">
            <p className="text-[9px] font-mono uppercase tracking-widest text-muted-foreground">
              Links
            </p>
            {edgeLegend.map(([label, { color, suggested, count }]) => (
              <div key={label} className="flex items-center gap-1.5 text-[10px]">
                <svg width="18" height="6" aria-hidden="true" className="shrink-0">
                  <line
                    x1="0"
                    y1="3"
                    x2="18"
                    y2="3"
                    stroke={color}
                    strokeWidth="2"
                    strokeDasharray={suggested ? '4 3' : undefined}
                  />
                </svg>
                <span className="text-foreground truncate">{label.replace(/_/g, ' ')}</span>
                <span className="ml-auto font-mono text-muted-foreground">{count}</span>
              </div>
            ))}
            <p className="text-[9px] text-muted-foreground pt-0.5 leading-snug">
              Dashed links are guesses, not links you made. Width is confidence.
            </p>
          </div>
        )}

        {selectedNode && (
          <GraphNodeInspector
            selectedNode={asSimNode(selectedNode)}
            neighbors={selectedNeighbours}
            onClose={() => setSelectedNodeId(null)}
            onSelectNode={(id) => setSelectedNodeId(id)}
            onOpenScribbleEditor={onOpenScribbleEditor}
            onSetPara={onSetPara}
          />
        )}
      </div>
    </div>
  );
};
