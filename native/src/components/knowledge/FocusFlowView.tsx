import React, { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { ChevronRight, Crosshair } from 'lucide-react';

import type { KnowledgeGraphData, KnowledgeNode } from '@/types';

import { layoutFocus, pathToRoot } from './graph/focusLayout';
import { hitTestFocus, renderFocus } from './graph/focusRenderer';
import type { CameraState } from './graph/graphTypes';

interface FocusFlowViewProps {
  graphData: KnowledgeGraphData;
  onOpenScribbleEditor?: (id: string) => void;
}

const DEPTH_CHOICES = [2, 3, 4, 6];

/**
 * Focus & Flow: one root, work branching outward.
 *
 * Clicking a node lights the route it arrived by and dims everything else.
 * That is the feature — it answers "how did I get here" — and it is done by
 * changing brightness, never by moving anything: the layout is a function
 * of the root and the vault alone.
 */
export const FocusFlowView: React.FC<FocusFlowViewProps> = ({
  graphData,
  onOpenScribbleEditor,
}) => {
  const containerRef = useRef<HTMLDivElement | null>(null);
  const canvasRef = useRef<HTMLCanvasElement | null>(null);

  const [rootId, setRootId] = useState<string | null>(null);
  const [depth, setDepth] = useState(4);
  const [hoveredNodeId, setHoveredNodeId] = useState<string | null>(null);
  const [selectedNodeId, setSelectedNodeId] = useState<string | null>(null);
  const [camera, setCamera] = useState<CameraState>({ x: 0, y: 0, k: 0.8 });
  const panRef = useRef<{ active: boolean; x: number; y: number }>({
    active: false,
    x: 0,
    y: 0,
  });

  const nodesById = useMemo(
    () => new Map(graphData.nodes.map((n) => [n.id, n])),
    [graphData.nodes],
  );

  /**
   * Candidate roots, best first.
   *
   * Ranked by PageRank because the most connected thing in the vault is the
   * most useful default place to start reading from — landing on an orphan
   * would show an empty tree and look broken.
   */
  const rootCandidates = useMemo(
    () =>
      [...graphData.nodes]
        .filter((n) => (n.degree ?? 0) > 0)
        .sort((a, b) => (b.pagerank ?? 0) - (a.pagerank ?? 0))
        .slice(0, 50),
    [graphData.nodes],
  );

  const effectiveRoot = rootId ?? rootCandidates[0]?.id ?? null;

  const layout = useMemo(
    () => layoutFocus(effectiveRoot, graphData.nodes, graphData.edges, depth),
    [effectiveRoot, graphData, depth],
  );

  // A selection from a previous root is not a selection in this tree.
  useEffect(() => {
    setSelectedNodeId((current) => (current && layout.byId.has(current) ? current : null));
  }, [layout]);

  const path = useMemo(() => pathToRoot(layout, selectedNodeId), [layout, selectedNodeId]);

  const draw = useCallback(() => {
    const canvas = canvasRef.current;
    const ctx = canvas?.getContext('2d');
    if (!canvas || !ctx) return;

    renderFocus({
      canvas,
      ctx,
      layout,
      nodesById,
      camera,
      hoveredNodeId,
      selectedNodeId,
      path,
    });
  }, [layout, nodesById, camera, hoveredNodeId, selectedNodeId, path]);

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

  const locate = (event: React.PointerEvent<HTMLCanvasElement>) => {
    const canvas = canvasRef.current;
    if (!canvas) return null;
    const rect = canvas.getBoundingClientRect();
    const dpr = window.devicePixelRatio || 1;
    return hitTestFocus(
      layout,
      event.clientX - rect.left,
      event.clientY - rect.top,
      canvas.height / dpr,
      camera,
    );
  };

  const handlePointerMove = (event: React.PointerEvent<HTMLCanvasElement>) => {
    if (panRef.current.active) {
      const dx = event.clientX - panRef.current.x;
      const dy = event.clientY - panRef.current.y;
      panRef.current = { active: true, x: event.clientX, y: event.clientY };
      setCamera((prev) => ({ ...prev, x: prev.x + dx, y: prev.y + dy }));
      return;
    }
    setHoveredNodeId(locate(event));
  };

  const handlePointerDown = (event: React.PointerEvent<HTMLCanvasElement>) => {
    const hit = locate(event);
    if (hit) {
      setSelectedNodeId(hit);
      return;
    }
    panRef.current = { active: true, x: event.clientX, y: event.clientY };
    setSelectedNodeId(null);
  };

  const pathNodes = path
    .map((id) => nodesById.get(id))
    .filter((n): n is KnowledgeNode => Boolean(n));

  /** Reading order for the fallback list: shallowest first, then by the
   *  vertical position the canvas draws them in. */
  const orderedForKeyboard = useMemo(
    () => [...layout.nodes].sort((a, b) => a.depth - b.depth || a.y - b.y),
    [layout],
  );

  return (
    <div className="flex-1 flex flex-col min-h-0 min-w-0">
      <div className="flex items-center gap-2 px-3 py-2 border-b border-border/60 flex-wrap">
        <label className="flex items-center gap-1.5 text-xs text-muted-foreground">
          <Crosshair className="w-3.5 h-3.5" />
          <span>Rooted on</span>
          <select
            value={effectiveRoot ?? ''}
            onChange={(e) => setRootId(e.target.value || null)}
            aria-label="Root node"
            className="h-7 max-w-56 px-2 text-xs bg-background border border-border rounded-md text-foreground focus:outline-hidden focus:ring-1 focus:ring-ring"
          >
            {rootCandidates.map((candidate) => (
              <option key={candidate.id} value={candidate.id}>
                {candidate.label}
              </option>
            ))}
          </select>
        </label>

        <label className="flex items-center gap-1.5 text-xs text-muted-foreground">
          <span>Depth</span>
          <select
            value={depth}
            onChange={(e) => setDepth(Number(e.target.value))}
            aria-label="How many hops to follow"
            className="h-7 px-2 text-xs bg-background border border-border rounded-md text-foreground focus:outline-hidden focus:ring-1 focus:ring-ring"
          >
            {DEPTH_CHOICES.map((d) => (
              <option key={d} value={d}>
                {d}
              </option>
            ))}
          </select>
        </label>

        <p className="text-[11px] text-muted-foreground ml-auto">
          {selectedNodeId
            ? 'Showing the route from the root. Click empty space to clear.'
            : 'Click a node to trace how you got there.'}
        </p>
      </div>

      {pathNodes.length > 0 && (
        <nav
          aria-label="Path from root"
          className="flex items-center gap-1 px-3 py-1.5 border-b border-border/60 overflow-x-auto"
        >
          {pathNodes.map((node, index) => (
            <React.Fragment key={node.id}>
              {index > 0 && (
                <ChevronRight className="w-3 h-3 text-muted-foreground shrink-0" aria-hidden />
              )}
              <button
                type="button"
                onClick={() => setSelectedNodeId(node.id)}
                onDoubleClick={() =>
                  node.node_type === 'scribble' && onOpenScribbleEditor?.(node.id)
                }
                className="text-[11px] px-1.5 py-0.5 rounded-md text-foreground hover:bg-accent whitespace-nowrap"
              >
                {node.label}
              </button>
            </React.Fragment>
          ))}
        </nav>
      )}

      <div ref={containerRef} className="relative flex-1 min-h-0 overflow-hidden">
        {layout.nodes.length === 0 ? (
          <p className="absolute inset-0 flex items-center justify-center text-xs text-muted-foreground">
            Nothing connects to this root yet.
          </p>
        ) : (
          <canvas
            ref={canvasRef}
            className="absolute inset-0 touch-none"
            style={{ cursor: hoveredNodeId ? 'pointer' : 'grab' }}
            onPointerMove={handlePointerMove}
            onPointerDown={handlePointerDown}
            onPointerUp={() => {
              panRef.current = { active: false, x: 0, y: 0 };
            }}
            onPointerLeave={() => {
              panRef.current = { active: false, x: 0, y: 0 };
              setHoveredNodeId(null);
            }}
            onWheel={(e) =>
              setCamera((prev) => ({
                ...prev,
                k: Math.min(3, Math.max(0.2, prev.k * (e.deltaY < 0 ? 1.12 : 1 / 1.12))),
              }))
            }
          >
            {/*
              The canvas's fallback content: the accessible representation of
              a picture nothing but a pointer can otherwise reach. Browsers
              do not paint this, but it is in the accessibility tree and its
              buttons take focus, so the tree is navigable by keyboard and
              legible to a screen reader — and tracing a path does not
              require being able to see or click the canvas.
            */}
            <ul aria-label="Nodes in this tree">
              {orderedForKeyboard.map((placed) => {
                const node = nodesById.get(placed.id);
                return (
                  <li key={placed.id}>
                    <button type="button" onClick={() => setSelectedNodeId(placed.id)}>
                      {node?.label ?? placed.id}
                      {placed.depth === 0 ? ' (root)' : ` — ${placed.depth} hops from the root`}
                    </button>
                  </li>
                );
              })}
            </ul>
          </canvas>
        )}
      </div>
    </div>
  );
};
