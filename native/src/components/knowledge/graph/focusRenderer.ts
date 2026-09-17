import type { KnowledgeNode } from '@/types';

import { edgeColorFor, GHOST_EDGE_COLOR, RELAY_COLOR_MAP, type CameraState } from './graphTypes';
import { hexToHsl, hsla, lighten, readGraphPalette, type GraphPalette } from './graphTokens';
import { pathEdgeKeys, type FocusLayout } from './focusLayout';

/**
 * Canvas 2D rendering for Focus & Flow.
 *
 * The view's one job is to answer "how did I get here", so everything here
 * is in service of the highlighted path: when a node is selected, the route
 * from the root is drawn at full strength and everything else drops to a
 * background wash. Nothing moves when that happens — the layout is fixed
 * and selection only changes what is bright.
 */

export interface FocusRenderContext {
  canvas: HTMLCanvasElement;
  ctx: CanvasRenderingContext2D;
  layout: FocusLayout;
  nodesById: Map<string, KnowledgeNode>;
  camera: CameraState;
  hoveredNodeId: string | null;
  selectedNodeId: string | null;
  /** Root-to-selection chain. Empty means nothing is selected. */
  path: string[];
  palette?: GraphPalette;
}

const ARROW_SIZE = 7;

function nodeColor(node: KnowledgeNode | undefined) {
  if (!node) return hexToHsl(RELAY_COLOR_MAP.default);
  const key = node.node_type === 'source' && node.source_type ? node.source_type : node.node_type;
  return hexToHsl(
    RELAY_COLOR_MAP[key] ?? RELAY_COLOR_MAP[node.node_type] ?? RELAY_COLOR_MAP.default,
  );
}

/** An arrowhead at the target end, inset so it meets the node's edge. */
function drawArrowhead(
  ctx: CanvasRenderingContext2D,
  fromX: number,
  fromY: number,
  toX: number,
  toY: number,
  targetRadius: number,
): void {
  const angle = Math.atan2(toY - fromY, toX - fromX);
  const tipX = toX - Math.cos(angle) * (targetRadius + 2);
  const tipY = toY - Math.sin(angle) * (targetRadius + 2);

  ctx.beginPath();
  ctx.moveTo(tipX, tipY);
  ctx.lineTo(
    tipX - ARROW_SIZE * Math.cos(angle - Math.PI / 6),
    tipY - ARROW_SIZE * Math.sin(angle - Math.PI / 6),
  );
  ctx.lineTo(
    tipX - ARROW_SIZE * Math.cos(angle + Math.PI / 6),
    tipY - ARROW_SIZE * Math.sin(angle + Math.PI / 6),
  );
  ctx.closePath();
  ctx.fill();
}

export function renderFocus(rc: FocusRenderContext): void {
  const { canvas, ctx, layout, nodesById, camera, hoveredNodeId, selectedNodeId, path } = rc;
  const palette = rc.palette ?? readGraphPalette();

  const dpr = window.devicePixelRatio || 1;
  const cssWidth = canvas.width / dpr;
  const cssHeight = canvas.height / dpr;

  ctx.save();
  ctx.scale(dpr, dpr);
  ctx.clearRect(0, 0, cssWidth, cssHeight);
  // The tree grows rightward from the root, so the origin sits at the left
  // edge rather than in the middle of the viewport.
  ctx.translate(60 + camera.x, cssHeight / 2 + camera.y);
  ctx.scale(camera.k, camera.k);

  const onPath = new Set(path);
  const pathEdges = pathEdgeKeys(path);
  const tracing = path.length > 0;

  for (const edge of layout.edges) {
    const from = layout.byId.get(edge.sourceId);
    const to = layout.byId.get(edge.targetId);
    if (!from || !to) continue;

    const isOnPath = pathEdges.has(`${edge.sourceId}|${edge.targetId}`);
    // Dimming is the whole mechanism: off-path is pushed to a wash so the
    // route reads at a glance.
    const alpha = tracing ? (isOnPath ? 0.95 : 0.07) : edge.crossLink ? 0.22 : 0.42;

    const color = hexToHsl(
      edge.suggested ? GHOST_EDGE_COLOR : edgeColorFor(edge.relationship),
    );
    const confidence = Math.max(0, Math.min(1, edge.confidence));

    ctx.save();
    // Cross-links and suggestions both dash, for different reasons: one
    // leaves the tree, the other was never asserted. They are told apart by
    // hue, not by pattern.
    ctx.setLineDash(edge.crossLink || edge.suggested ? [5, 4] : []);
    ctx.beginPath();
    ctx.moveTo(from.x, from.y);
    ctx.lineTo(to.x, to.y);
    ctx.strokeStyle = hsla(color, alpha);
    ctx.lineWidth = (0.5 + confidence * 1.2) * (isOnPath ? 2 : 1);
    ctx.stroke();
    ctx.setLineDash([]);

    // Direction is drawn only where the relationship type has one.
    if (edge.directional && alpha > 0.1) {
      ctx.fillStyle = hsla(color, alpha);
      drawArrowhead(ctx, from.x, from.y, to.x, to.y, to.size);
    }
    ctx.restore();
  }

  for (const placed of layout.nodes) {
    const node = nodesById.get(placed.id);
    const isRoot = placed.id === layout.rootId;
    const isSelected = placed.id === selectedNodeId;
    const isHovered = placed.id === hoveredNodeId;
    const lit = !tracing || onPath.has(placed.id);
    const alpha = lit ? 1 : 0.12;

    const base = nodeColor(node);
    const r = Math.max(3, placed.size) * (isRoot ? 1.5 : 1);

    ctx.save();
    if (lit) {
      ctx.shadowColor = hsla(base, 0.5);
      ctx.shadowBlur = isSelected || isHovered ? 16 : isRoot ? 12 : 5;
    }
    const gradient = ctx.createRadialGradient(x0(placed.x, r), x0(placed.y, r), r * 0.12, placed.x, placed.y, r);
    gradient.addColorStop(0, hsla(lighten(base, palette.isDark ? 18 : 12), alpha));
    gradient.addColorStop(1, hsla(base, alpha * 0.9));

    ctx.beginPath();
    ctx.arc(placed.x, placed.y, r, 0, Math.PI * 2);
    ctx.fillStyle = gradient;
    ctx.fill();
    ctx.restore();

    if (isSelected || isHovered || isRoot) {
      ctx.beginPath();
      ctx.arc(placed.x, placed.y, r, 0, Math.PI * 2);
      ctx.lineWidth = isSelected ? 2.5 : 1.5;
      ctx.strokeStyle = hsla(palette.foreground, alpha);
      ctx.stroke();
    }

    const label = node?.label ?? '';
    if (label && (lit || !tracing)) {
      const text = label.length > 26 ? `${label.slice(0, 25)}…` : label;
      ctx.font = `${isRoot ? '600' : '400'} 12px system-ui, -apple-system, BlinkMacSystemFont, sans-serif`;
      ctx.textAlign = 'left';
      ctx.textBaseline = 'middle';
      ctx.fillStyle = hsla(palette.foreground, alpha);
      ctx.fillText(text, placed.x + r + 6, placed.y);
    }
  }

  ctx.restore();
}

/** Offsets a gradient's inner focus up and to the left of the node centre. */
function x0(coordinate: number, radius: number): number {
  return coordinate - radius * 0.32;
}

/** The node under a screen-space point, or `null`. */
export function hitTestFocus(
  layout: FocusLayout,
  screenX: number,
  screenY: number,
  canvasHeight: number,
  camera: CameraState,
): string | null {
  const worldX = (screenX - 60 - camera.x) / camera.k;
  const worldY = (screenY - canvasHeight / 2 - camera.y) / camera.k;
  const slack = 5 / camera.k;

  for (let i = layout.nodes.length - 1; i >= 0; i--) {
    const node = layout.nodes[i];
    const r = node.size * (node.id === layout.rootId ? 1.5 : 1);
    if (Math.hypot(worldX - node.x, worldY - node.y) <= r + slack) return node.id;
  }
  return null;
}
