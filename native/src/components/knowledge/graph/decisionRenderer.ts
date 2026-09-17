import type { DecisionRecord, KnowledgeNode } from '@/types';

import { hexToHsl, hsla, lighten, readGraphPalette, type GraphPalette } from './graphTokens';
import { RELAY_COLOR_MAP, type CameraState } from './graphTypes';
import { decisionEdgeStyle } from './decisionTree';
import type { FocusLayout } from './focusLayout';

/**
 * Canvas 2D rendering for the Decision Tree.
 *
 * Same layered layout as Focus & Flow — this reuses `FocusLayout` rather
 * than laying anything out itself. What differs is the edge: here it
 * carries the decision, so its weight, opacity and dashes are the
 * confidence ladder made visible.
 *
 * The one thing this file must never do is let a guess look like something
 * the user said. Every visual channel it has — weight, opacity, dash,
 * strike-through — moves in the same direction for that reason.
 */

export interface DecisionRenderContext {
  canvas: HTMLCanvasElement;
  ctx: CanvasRenderingContext2D;
  layout: FocusLayout;
  nodesById: Map<string, KnowledgeNode>;
  /** Decision id → record. Nodes not in here are subjects. */
  decisions: Map<string, DecisionRecord>;
  camera: CameraState;
  hoveredNodeId: string | null;
  selectedNodeId: string | null;
  palette?: GraphPalette;
}

/** A line through the middle of a reversed decision's label. */
function strikeThrough(
  ctx: CanvasRenderingContext2D,
  text: string,
  x: number,
  y: number,
  color: string,
): void {
  const width = ctx.measureText(text).width;
  ctx.beginPath();
  ctx.moveTo(x, y);
  ctx.lineTo(x + width, y);
  ctx.strokeStyle = color;
  ctx.lineWidth = 1;
  ctx.stroke();
}

export function renderDecisions(rc: DecisionRenderContext): void {
  const { canvas, ctx, layout, nodesById, decisions, camera, hoveredNodeId, selectedNodeId } =
    rc;
  const palette = rc.palette ?? readGraphPalette();

  const dpr = window.devicePixelRatio || 1;
  const cssWidth = canvas.width / dpr;
  const cssHeight = canvas.height / dpr;

  ctx.save();
  ctx.scale(dpr, dpr);
  ctx.clearRect(0, 0, cssWidth, cssHeight);
  ctx.translate(60 + camera.x, cssHeight / 2 + camera.y);
  ctx.scale(camera.k, camera.k);

  for (const edge of layout.edges) {
    const from = layout.byId.get(edge.sourceId);
    const to = layout.byId.get(edge.targetId);
    if (!from || !to) continue;

    // The decision an edge carries is the one at its target end: a
    // subject decides a decision, and a decision supersedes a decision.
    const decision = decisions.get(edge.targetId);
    const style = decision
      ? decisionEdgeStyle(decision)
      : { width: 1, opacity: 0.4, dashed: true, struckThrough: false };

    const color = hexToHsl(RELAY_COLOR_MAP.default);

    ctx.save();
    ctx.setLineDash(style.dashed ? [5, 4] : []);
    ctx.beginPath();
    ctx.moveTo(from.x, from.y);
    ctx.lineTo(to.x, to.y);
    ctx.strokeStyle = hsla(color, style.opacity);
    ctx.lineWidth = style.width;
    ctx.stroke();
    ctx.restore();
  }

  for (const placed of layout.nodes) {
    const node = nodesById.get(placed.id);
    const decision = decisions.get(placed.id);
    const isSubject = !decision;
    const isActive = placed.id === selectedNodeId || placed.id === hoveredNodeId;

    const style = decision
      ? decisionEdgeStyle(decision)
      : { width: 2, opacity: 1, dashed: false, struckThrough: false };

    const base = hexToHsl(
      isSubject ? RELAY_COLOR_MAP.topic : RELAY_COLOR_MAP.task ?? RELAY_COLOR_MAP.default,
    );
    const r = Math.max(4, placed.size) * (isSubject ? 1.4 : 1);

    ctx.save();
    if (style.opacity > 0.5) {
      ctx.shadowColor = hsla(base, 0.45 * style.opacity);
      ctx.shadowBlur = isActive ? 14 : 6;
    }
    const gradient = ctx.createRadialGradient(
      placed.x - r * 0.32,
      placed.y - r * 0.32,
      r * 0.12,
      placed.x,
      placed.y,
      r,
    );
    gradient.addColorStop(0, hsla(lighten(base, palette.isDark ? 18 : 12), style.opacity));
    gradient.addColorStop(1, hsla(base, style.opacity * 0.9));

    ctx.beginPath();
    ctx.arc(placed.x, placed.y, r, 0, Math.PI * 2);
    ctx.fillStyle = gradient;
    ctx.fill();
    ctx.restore();

    if (isActive) {
      ctx.beginPath();
      ctx.arc(placed.x, placed.y, r, 0, Math.PI * 2);
      ctx.lineWidth = placed.id === selectedNodeId ? 2.5 : 1.5;
      ctx.strokeStyle = hsla(palette.foreground, style.opacity);
      ctx.stroke();
    }

    const label = node?.label ?? '';
    if (!label) continue;
    const text = label.length > 30 ? `${label.slice(0, 29)}…` : label;

    ctx.font = `${isSubject ? '600' : '400'} 12px system-ui, -apple-system, BlinkMacSystemFont, sans-serif`;
    ctx.textAlign = 'left';
    ctx.textBaseline = 'middle';
    const textColor = hsla(palette.foreground, style.opacity);
    ctx.fillStyle = textColor;
    const textX = placed.x + r + 6;
    ctx.fillText(text, textX, placed.y);

    // A reversed decision is kept, and struck through so it cannot be
    // read as the current answer.
    if (style.struckThrough) {
      strikeThrough(ctx, text, textX, placed.y, textColor);
    }
  }

  ctx.restore();
}
