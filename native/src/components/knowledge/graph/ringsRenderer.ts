import type { KnowledgeEdge, KnowledgeNode } from '@/types';
import { RELAY_COLOR_MAP } from './graphTypes';
import { hexToHsl, hsla, readGraphPalette, RING_TINTS, type GraphPalette } from './graphTokens';
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

  drawBands(ctx, layout, palette, showBandLabels);
  drawEdges(ctx, rc, palette, raised, activeId !== null);
  drawNodes(ctx, rc, palette, raised, activeId !== null);
  drawLabels(ctx, rc, palette, raised, activeId !== null);

  ctx.restore();
}

/** The four PARA bands plus Unfiled, as tinted annuli. */
function drawBands(
  ctx: CanvasRenderingContext2D,
  layout: RingsLayout,
  palette: GraphPalette,
  showLabels: boolean,
): void {
  for (const band of layout.bands) {
    const tint = RING_TINTS[band.band] ?? palette.muted;

    ctx.beginPath();
    ctx.arc(0, 0, band.outerRadius, 0, Math.PI * 2);
    ctx.strokeStyle = hsla(tint, 0.22);
    ctx.lineWidth = 1;
    ctx.stroke();

    if (showLabels) {
      ctx.save();
      ctx.font = '500 13px system-ui, -apple-system, BlinkMacSystemFont, sans-serif';
      ctx.textAlign = 'center';
      ctx.textBaseline = 'middle';
      ctx.fillStyle = hsla(palette.mutedForeground, 0.55);
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

    ctx.beginPath();
    ctx.moveTo(a.x, a.y);
    ctx.lineTo(b.x, b.y);
    ctx.strokeStyle = hsla(palette.mutedForeground, alpha);
    ctx.lineWidth = isRaised ? 1.4 : 0.7;
    ctx.stroke();
  }
}

function drawNodes(
  ctx: CanvasRenderingContext2D,
  rc: RingsRenderContext,
  palette: GraphPalette,
  raised: Set<string>,
  quieting: boolean,
): void {
  const { layout, nodesById, opacity, hoveredNodeId, selectedNodeId } = rc;

  for (const node of layout.nodes) {
    const alpha = opacity.get(node.id) ?? 1;
    if (alpha < 0.01) continue;

    const dimmed = quieting && !raised.has(node.id);
    const effective = dimmed ? alpha * 0.18 : alpha;
    const color = baseColorOf(nodesById.get(node.id));

    ctx.beginPath();
    ctx.arc(node.x, node.y, node.size, 0, Math.PI * 2);
    ctx.fillStyle = hsla(color, effective * 0.9);
    ctx.fill();

    if (node.id === selectedNodeId || node.id === hoveredNodeId) {
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
