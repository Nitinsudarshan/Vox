import type { KnowledgeEdge, KnowledgeNode } from '@/types';
import { RELAY_COLOR_MAP } from './graphTypes';
import {
  hexToHsl,
  hsla,
  lighten,
  readGraphPalette,
  RING_TINTS,
  withAge,
  type GraphPalette,
} from './graphTokens';
import type { RingsLayout } from './ringsLayout';
import type { CameraState } from './graphTypes';

/**
 * Canvas 2D rendering for the Rings view.
 *
 * A pure function of its inputs: it reads the layout and the camera and
 * draws, holding no state of its own. Everything that changes between
 * frames — hover, selection, the entry animation's progress — arrives as an
 * argument, so a frame can be reproduced exactly by replaying its context.
 */

export interface RingsRenderContext {
  canvas: HTMLCanvasElement;
  ctx: CanvasRenderingContext2D;
  layout: RingsLayout;
  nodesById: Map<string, KnowledgeNode>;
  edges: KnowledgeEdge[];
  /** Node id → opacity. Filtering lives here and nowhere else. */
  opacity: Map<string, number>;
  adjacency: Map<string, Set<string>>;
  camera: CameraState;
  hoveredNodeId: string | null;
  selectedNodeId: string | null;
  /** Ring labels are drawn unless the caller has a reason to suppress them. */
  showBandLabels?: boolean;
  palette?: GraphPalette;
  /**
   * Entry animation progress, 0 to 1.
   *
   * Nodes ease outward from the centre to their final radius and then stop.
   * There is no idle animation and no ambient drift — a universe that keeps
   * twitching hides the fact that the layout is settled, which is the one
   * thing this view is claiming.
   */
  entryProgress?: number;
}

/**
 * Easing for the entry animation: fast out of the gate, still at the end.
 *
 * Exported so the motion can be tested without a canvas, and so nothing
 * else has to re-derive it.
 */
export function easeOutCubic(t: number): number {
  const clamped = Math.max(0, Math.min(1, t));
  return 1 - Math.pow(1 - clamped, 3);
}

/** Duration of the entry animation, in milliseconds. */
export const ENTRY_ANIMATION_MS = 600;

/**
 * Whether the viewer has asked for less motion.
 *
 * Read per animation rather than cached: the OS setting can change while
 * the app is open, and someone turning it on mid-session means it now.
 */
export function prefersReducedMotion(): boolean {
  try {
    return window.matchMedia('(prefers-reduced-motion: reduce)').matches;
  } catch {
    return false;
  }
}

const BAND_LABELS: Record<string, string> = {
  projects: 'Projects',
  areas: 'Areas',
  resources: 'Resources',
  archive: 'Archive',
  uncategorised: 'Unfiled',
};

/** Colour for a node, from the node-type palette the graph already uses. */
function baseColorOf(node: KnowledgeNode | undefined) {
  if (!node) return hexToHsl(RELAY_COLOR_MAP.default);
  const key =
    node.node_type === 'source' && node.source_type ? node.source_type : node.node_type;
  return hexToHsl(RELAY_COLOR_MAP[key] ?? RELAY_COLOR_MAP[node.node_type] ?? RELAY_COLOR_MAP.default);
}

/**
 * Draws one frame.
 *
 * Order is deliberate: the ring atmosphere sits behind everything, then
 * edges, then nodes, then labels. Edges under nodes is what makes the eye
 * land on the nodes — the view is about where things sit, not about the
 * wiring between them.
 */
export function renderRings(rc: RingsRenderContext): void {
  const {
    canvas,
    ctx,
    layout,
    adjacency,
    camera,
    hoveredNodeId,
    selectedNodeId,
    showBandLabels = true,
    entryProgress = 1,
  } = rc;

  const palette = rc.palette ?? readGraphPalette();
  const dpr = window.devicePixelRatio || 1;
  const cssWidth = canvas.width / dpr;
  const cssHeight = canvas.height / dpr;

  ctx.save();
  ctx.scale(dpr, dpr);
  ctx.clearRect(0, 0, cssWidth, cssHeight);
  ctx.translate(cssWidth / 2 + camera.x, cssHeight / 2 + camera.y);
  ctx.scale(camera.k, camera.k);

  // The 1-hop neighbourhood that hover or selection raises.
  const activeId = hoveredNodeId ?? selectedNodeId;
  const raised = new Set<string>();
  if (activeId) {
    raised.add(activeId);
    adjacency.get(activeId)?.forEach((id) => raised.add(id));
  }

  const eased = easeOutCubic(entryProgress);

  drawBands(ctx, layout, palette, showBandLabels, eased);
  drawEdges(ctx, rc, palette, raised, activeId !== null, eased);
  drawNodes(ctx, rc, palette, raised, activeId !== null, eased);
  // Labels only once everything has arrived: text sliding outward under a
  // moving node is noise, and it is over in 600ms either way.
  if (eased >= 1) {
    drawLabels(ctx, rc, palette, raised, activeId !== null);
  }

  ctx.restore();
}

/**
 * The four PARA bands plus Unfiled, as filled annuli.
 *
 * Each band is a very low-alpha fill in its own tint, so the regions are
 * felt as areas rather than read off a label. The fill is an annulus — an
 * outer arc clockwise and an inner arc counter-clockwise, which the
 * non-zero winding rule renders as a ring with a hole — rather than
 * stacked discs, which would make the inner bands progressively muddier as
 * every outer band's tint accumulated underneath them.
 */
function drawBands(
  ctx: CanvasRenderingContext2D,
  layout: RingsLayout,
  palette: GraphPalette,
  showLabels: boolean,
  eased: number,
): void {
  for (const band of layout.bands) {
    const tint = RING_TINTS[band.band] ?? palette.muted;

    ctx.beginPath();
    ctx.arc(0, 0, band.outerRadius, 0, Math.PI * 2, false);
    ctx.arc(0, 0, band.innerRadius, 0, Math.PI * 2, true);
    ctx.fillStyle = hsla(tint, 0.05 * eased);
    ctx.fill();

    ctx.beginPath();
    ctx.arc(0, 0, band.outerRadius, 0, Math.PI * 2);
    ctx.strokeStyle = hsla(tint, 0.22 * eased);
    ctx.lineWidth = 1;
    ctx.stroke();

    if (showLabels) {
      ctx.save();
      ctx.font = '500 13px system-ui, -apple-system, BlinkMacSystemFont, sans-serif';
      ctx.textAlign = 'center';
      ctx.textBaseline = 'middle';
      ctx.fillStyle = hsla(palette.mutedForeground, 0.55 * eased);
      const labelRadius = (band.innerRadius + band.outerRadius) / 2;
      ctx.fillText(
        `${BAND_LABELS[band.band] ?? band.band} · ${band.count}`,
        0,
        -labelRadius,
      );
      ctx.restore();
    }
  }
}

function drawEdges(
  ctx: CanvasRenderingContext2D,
  rc: RingsRenderContext,
  palette: GraphPalette,
  raised: Set<string>,
  quieting: boolean,
  eased: number,
): void {
  const { layout, edges, opacity } = rc;

  for (const edge of edges) {
    const a = layout.byId.get(edge.source_id);
    const b = layout.byId.get(edge.target_id);
    if (!a || !b) continue;

    const visible = Math.min(opacity.get(a.id) ?? 1, opacity.get(b.id) ?? 1);
    const isRaised = quieting && raised.has(a.id) && raised.has(b.id);
    const alpha = isRaised ? visible * 0.85 : visible * (quieting ? 0.05 : 0.16);
    if (alpha < 0.01) continue;

    const from = entryPosition(a, eased);
    const to = entryPosition(b, eased);

    ctx.beginPath();
    ctx.moveTo(from.x, from.y);
    ctx.lineTo(to.x, to.y);
    ctx.strokeStyle = hsla(palette.mutedForeground, alpha * eased);
    ctx.lineWidth = isRaised ? 1.4 : 0.7;
    ctx.stroke();
  }
}

/**
 * A node's position partway through the entry animation.
 *
 * Nodes ease outward along their own final angle, so the rings assemble
 * from the centre rather than every node taking an arbitrary path. At
 * `eased === 1` this is exactly the laid-out coordinate — the animation
 * converges on the deterministic layout rather than near it.
 */
function entryPosition(
  node: { x: number; y: number; angle: number; radius: number },
  eased: number,
): { x: number; y: number } {
  if (eased >= 1) return { x: node.x, y: node.y };
  const r = node.radius * eased;
  return { x: Math.cos(node.angle) * r, y: Math.sin(node.angle) * r };
}

/**
 * Nodes, with depth and temperature.
 *
 * Three things are being said at once, and each has one visual channel so
 * they stay separable:
 *
 * - **Size and glow** carry PageRank. A hub reads as luminous; a leaf as a
 *   dim speck. The glow is `shadowBlur`, scaled from the node's own radius,
 *   which the layout already derived from rank.
 * - **Saturation and lightness** carry recency. A note edited today keeps
 *   its hue; one untouched for a year drains towards ash (or towards paper
 *   in light mode — see `withAge`).
 * - **Hue** carries node type, from the palette the graph already uses, so
 *   a scribble is the same blue here as everywhere else.
 *
 * The fill is a radial gradient offset up and left, which is what stops a
 * flat disc reading as a sticker and makes a large node read as a sphere.
 */
function drawNodes(
  ctx: CanvasRenderingContext2D,
  rc: RingsRenderContext,
  palette: GraphPalette,
  raised: Set<string>,
  quieting: boolean,
  eased: number,
): void {
  const { layout, nodesById, opacity, hoveredNodeId, selectedNodeId } = rc;

  const maxSize = layout.nodes.reduce((m, n) => Math.max(m, n.size), 1);

  for (const node of layout.nodes) {
    const alpha = opacity.get(node.id) ?? 1;
    if (alpha < 0.01) continue;

    const isActive = node.id === selectedNodeId || node.id === hoveredNodeId;
    const isRaised = quieting && raised.has(node.id);
    const dimmed = quieting && !raised.has(node.id);
    const effective = (dimmed ? alpha * 0.18 : alpha) * eased;

    const base = baseColorOf(nodesById.get(node.id));
    const aged = withAge(base, node.recency, palette.isDark);
    const { x, y } = entryPosition(node, eased);
    const r = Math.max(1, node.size);

    ctx.save();

    // Glow tracks rank, and lifts further when the node or a neighbour is
    // under the pointer. Suppressed while dimmed: a filtered-out node that
    // still glows defeats the filter.
    if (!dimmed) {
      const rankShare = r / maxSize;
      ctx.shadowColor = hsla(aged, 0.55 * effective);
      ctx.shadowBlur = (4 + rankShare * 14) * (isActive ? 2.1 : isRaised ? 1.5 : 1);
    }

    const gradient = ctx.createRadialGradient(
      x - r * 0.32,
      y - r * 0.32,
      r * 0.12,
      x,
      y,
      r,
    );
    gradient.addColorStop(0, hsla(lighten(aged, palette.isDark ? 18 : 12), effective));
    gradient.addColorStop(1, hsla(aged, effective * 0.88));

    ctx.beginPath();
    ctx.arc(x, y, r, 0, Math.PI * 2);
    ctx.fillStyle = gradient;
    ctx.fill();
    ctx.restore();

    if (isActive) {
      ctx.beginPath();
      ctx.arc(x, y, r, 0, Math.PI * 2);
      ctx.lineWidth = node.id === selectedNodeId ? 2.5 : 1.5;
      ctx.strokeStyle = hsla(palette.foreground, effective);
      ctx.stroke();
    }
  }
}

function drawLabels(
  ctx: CanvasRenderingContext2D,
  rc: RingsRenderContext,
  palette: GraphPalette,
  raised: Set<string>,
  quieting: boolean,
): void {
  const { layout, nodesById, opacity, camera, hoveredNodeId, selectedNodeId } = rc;

  // Labelling every node at every zoom is unreadable. Below this zoom only
  // the nodes the user is actually pointing at are named.
  const labelEverything = camera.k > 0.75;
  const fontSize = Math.max(9, Math.min(13, 12 / Math.sqrt(camera.k)));

  ctx.font = `500 ${fontSize}px system-ui, -apple-system, BlinkMacSystemFont, sans-serif`;
  ctx.textAlign = 'center';
  ctx.textBaseline = 'top';

  for (const node of layout.nodes) {
    const isActive = node.id === hoveredNodeId || node.id === selectedNodeId;
    if (!isActive && !labelEverything) continue;
    if (quieting && !raised.has(node.id)) continue;

    const alpha = opacity.get(node.id) ?? 1;
    if (alpha < 0.3) continue;

    const label = nodesById.get(node.id)?.label ?? '';
    if (!label) continue;

    const text = label.length > 24 ? `${label.slice(0, 23)}…` : label;
    ctx.fillStyle = hsla(palette.foreground, isActive ? alpha : alpha * 0.7);
    ctx.fillText(text, node.x, node.y + node.size + 3);
  }
}

/**
 * The node under a screen-space point, or `null`.
 *
 * Walks nodes in reverse draw order so the topmost wins, and uses a minimum
 * hit radius so the smallest nodes stay clickable at any zoom — a 4px dot
 * is a legitimate thing to want to open.
 */
export function hitTestRings(
  layout: RingsLayout,
  screenX: number,
  screenY: number,
  canvasWidth: number,
  canvasHeight: number,
  camera: CameraState,
  opacity?: Map<string, number>,
): string | null {
  const worldX = (screenX - canvasWidth / 2 - camera.x) / camera.k;
  const worldY = (screenY - canvasHeight / 2 - camera.y) / camera.k;
  const slack = 4 / camera.k;

  for (let i = layout.nodes.length - 1; i >= 0; i--) {
    const node = layout.nodes[i];
    // A dimmed node is not a target: clicking through a filter should not
    // select something the filter says you are not looking at.
    if (opacity && (opacity.get(node.id) ?? 1) < 0.3) continue;
    if (Math.hypot(worldX - node.x, worldY - node.y) <= node.size + slack) {
      return node.id;
    }
  }
  return null;
}
