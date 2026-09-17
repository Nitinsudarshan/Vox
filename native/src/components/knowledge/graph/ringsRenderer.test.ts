import { describe, expect, it, vi } from 'vitest';

import {
  easeOutCubic,
  ENTRY_ANIMATION_MS,
  hitTestRings,
  prefersReducedMotion,
  renderRings,
  type RingsRenderContext,
} from './ringsRenderer';
import { layoutRings } from './ringsLayout';
import type { KnowledgeEdge, KnowledgeNode, ParaBand } from '@/types';

/**
 * The Rings renderer draws to a canvas, and jsdom has no 2D context — so
 * these drive it with a recording stub and assert on the drawing calls.
 * That verifies the things the visual brief is actually specific about
 * (depth, glow tracking rank, recency draining colour, ring atmosphere,
 * motion that stops) without needing pixels.
 */

interface FillRecord {
  kind: 'fill' | 'stroke';
  style: unknown;
  arc: { x: number; y: number; r: number } | null;
  shadowBlur: number;
  arcsInPath: number;
}

interface Recorder {
  ctx: CanvasRenderingContext2D;
  fills: FillRecord[];
  gradients: Array<{ x0: number; y0: number; r0: number; x1: number; y1: number; r1: number; stops: Array<[number, string]> }>;
  texts: string[];
  dashes: number[][];
}

function recordingContext(): Recorder {
  const fills: FillRecord[] = [];
  const gradients: Recorder['gradients'] = [];
  const texts: string[] = [];
  const dashes: number[][] = [];
  let pathArcs: Array<{ x: number; y: number; r: number }> = [];

  const state = {
    fillStyle: '' as unknown,
    strokeStyle: '' as unknown,
    lineWidth: 1,
    shadowBlur: 0,
    shadowColor: '',
    globalAlpha: 1,
    font: '',
    textAlign: '',
    textBaseline: '',
  };

  const record = (kind: 'fill' | 'stroke') => {
    fills.push({
      kind,
      style: kind === 'fill' ? state.fillStyle : state.strokeStyle,
      arc: pathArcs[0] ?? null,
      shadowBlur: state.shadowBlur,
      arcsInPath: pathArcs.length,
    });
  };

  const ctx = {
    get fillStyle() { return state.fillStyle; },
    set fillStyle(v) { state.fillStyle = v; },
    get strokeStyle() { return state.strokeStyle; },
    set strokeStyle(v) { state.strokeStyle = v; },
    get lineWidth() { return state.lineWidth; },
    set lineWidth(v) { state.lineWidth = v; },
    get shadowBlur() { return state.shadowBlur; },
    set shadowBlur(v) { state.shadowBlur = v; },
    get shadowColor() { return state.shadowColor; },
    set shadowColor(v) { state.shadowColor = v; },
    get globalAlpha() { return state.globalAlpha; },
    set globalAlpha(v) { state.globalAlpha = v; },
    font: '',
    textAlign: '',
    textBaseline: '',

    save: () => {},
    // `restore` resets the shadow the way a real context would, so a node
    // drawn without a glow cannot inherit the previous node's.
    restore: () => { state.shadowBlur = 0; },
    scale: () => {},
    translate: () => {},
    clearRect: () => {},
    beginPath: () => { pathArcs = []; },
    arc: (x: number, y: number, r: number) => { pathArcs.push({ x, y, r }); },
    moveTo: () => {},
    lineTo: () => {},
    setLineDash: (d: number[]) => { dashes.push([...d]); },
    fill: () => record('fill'),
    stroke: () => record('stroke'),
    fillText: (t: string) => { texts.push(t); },
    measureText: (t: string) => ({ width: t.length * 6 }),
    roundRect: () => {},
    createRadialGradient: (x0: number, y0: number, r0: number, x1: number, y1: number, r1: number) => {
      const stops: Array<[number, string]> = [];
      gradients.push({ x0, y0, r0, x1, y1, r1, stops });
      return { addColorStop: (o: number, c: string) => stops.push([o, c]) };
    },
  } as unknown as CanvasRenderingContext2D;

  return { ctx, fills, gradients, texts, dashes };
}

const node = (
  id: string,
  para: ParaBand | null,
  overrides: Partial<KnowledgeNode> = {},
): KnowledgeNode => ({
  id,
  node_type: 'scribble',
  label: id,
  metadata: {},
  degree: 1,
  para,
  pagerank: 0.05,
  updated_at: '2026-01-01T00:00:00Z',
  ...overrides,
});

function scene(
  nodes: KnowledgeNode[],
  edges: KnowledgeEdge[] = [],
  overrides: Partial<RingsRenderContext> = {},
): { rec: Recorder; rc: RingsRenderContext } {
  const rec = recordingContext();
  const layout = layoutRings(nodes, edges);
  const rc: RingsRenderContext = {
    canvas: { width: 800, height: 600 } as HTMLCanvasElement,
    ctx: rec.ctx,
    layout,
    nodesById: new Map(nodes.map((n) => [n.id, n])),
    edges,
    opacity: new Map(nodes.map((n) => [n.id, 1])),
    adjacency: new Map(),
    camera: { x: 0, y: 0, k: 1 },
    hoveredNodeId: null,
    selectedNodeId: null,
    ...overrides,
  };
  return { rec, rc };
}

/** Pulls the saturation out of an `hsla(h, s%, l%, a)` string. */
function saturationOf(color: string): number {
  const match = /hsla\(\s*[\d.]+\s*,\s*([\d.]+)%/.exec(color);
  return match ? Number.parseFloat(match[1]) : Number.NaN;
}

describe('depth', () => {
  /** Fails if a node is filled with a flat colour — a disc reads as a
   *  sticker, and the brief asks for spheres. */
  it('fills every node with a radial gradient offset towards the light', () => {
    const { rec } = ((): { rec: Recorder } => {
      const s = scene([node('a', 'projects'), node('b', 'areas'), node('c', null)]);
      renderRings(s.rc);
      return s;
    })();

    expect(rec.gradients).toHaveLength(3);
    for (const g of rec.gradients) {
      // Inner stop up and to the left of the node's centre.
      expect(g.x0).toBeLessThan(g.x1);
      expect(g.y0).toBeLessThan(g.y1);
      expect(g.r0).toBeLessThan(g.r1);
      expect(g.stops).toHaveLength(2);
    }
  });

  /**
   * Fails if glow is constant across nodes — the brief's "big nodes read as
   * luminous, small ones as dim" is a claim about rank, not decoration.
   */
  it('scales the glow with PageRank', () => {
    const s = scene([
      node('hub', 'projects', { pagerank: 0.9 }),
      node('leaf', 'projects', { pagerank: 0.001 }),
    ]);
    renderRings(s.rc);

    const hub = s.rc.layout.byId.get('hub')!;
    const leaf = s.rc.layout.byId.get('leaf')!;
    const glowFor = (r: number) =>
      s.rec.fills.find((f) => f.kind === 'fill' && f.arc && Math.abs(f.arc.r - r) < 1e-9)!
        .shadowBlur;

    expect(glowFor(hub.size)).toBeGreaterThan(glowFor(leaf.size));
  });

  /** A node the filter has dimmed must not still be glowing, or the filter
   *  is cosmetic. */
  it('does not glow a dimmed node', () => {
    const nodes = [node('lit', 'projects'), node('hidden', 'projects')];
    const s = scene(nodes, [], {
      opacity: new Map([
        ['lit', 1],
        ['hidden', 0.07],
      ]),
      hoveredNodeId: 'lit',
      adjacency: new Map([['lit', new Set<string>()]]),
    });
    renderRings(s.rc);

    const hidden = s.rc.layout.byId.get('hidden')!;
    const record = s.rec.fills.find(
      (f) => f.kind === 'fill' && f.arc && Math.abs(f.arc.r - hidden.size) < 1e-9,
    )!;
    expect(record.shadowBlur).toBe(0);
  });
});

describe('recency as temperature', () => {
  /**
   * Fails if an old note is drawn with the same saturation as a fresh one —
   * the ramp is the whole mechanism by which the view shows what has gone
   * cold.
   */
  it('drains colour from an old note and leaves a fresh one saturated', () => {
    const s = scene([
      node('fresh', 'projects', { updated_at: '2026-09-01T00:00:00Z' }),
      node('stale', 'projects', { updated_at: '2020-01-01T00:00:00Z' }),
    ]);
    renderRings(s.rc);

    // Gradients are emitted in layout order, which is sorted by id:
    // 'fresh' then 'stale'.
    const freshOuter = s.rec.gradients[0].stops[1][1];
    const staleOuter = s.rec.gradients[1].stops[1][1];

    expect(saturationOf(staleOuter)).toBeLessThan(saturationOf(freshOuter));
    expect(saturationOf(freshOuter)).toBeGreaterThan(0);
  });
});

describe('ring atmosphere', () => {
  /**
   * Fails if the bands are drawn as stacked discs (one arc per fill) rather
   * than annuli (two): stacked discs make each inner band muddier as every
   * outer tint accumulates beneath it.
   */
  it('fills each band as an annulus rather than a disc', () => {
    const s = scene([node('a', 'projects')]);
    renderRings(s.rc);

    const annuli = s.rec.fills.filter((f) => f.kind === 'fill' && f.arcsInPath === 2);
    expect(annuli).toHaveLength(s.rc.layout.bands.length);
    // Very low alpha: felt as a region, not read as a colour.
    for (const ring of annuli) {
      const alpha = /,\s*([\d.]+)\)$/.exec(String(ring.style))?.[1];
      expect(Number.parseFloat(alpha!)).toBeLessThan(0.1);
    }
  });

  it('names each band and how much is in it', () => {
    const s = scene([node('a', 'projects'), node('b', 'projects'), node('c', null)]);
    renderRings(s.rc);

    expect(s.rec.texts).toContain('Projects · 2');
    expect(s.rec.texts).toContain('Unfiled · 1');
    expect(s.rec.texts).toContain('Archive · 0');
  });
});

describe('motion that decays to nothing', () => {
  it('eases out and is exactly finished at 1', () => {
    expect(easeOutCubic(0)).toBe(0);
    expect(easeOutCubic(1)).toBe(1);
    expect(easeOutCubic(0.5)).toBeGreaterThan(0.5); // fast out of the gate
    // Clamped, so a late frame cannot overshoot the final layout.
    expect(easeOutCubic(1.4)).toBe(1);
    expect(easeOutCubic(-2)).toBe(0);
    expect(ENTRY_ANIMATION_MS).toBe(600);
  });

  /**
   * Fails if a mid-animation frame draws nodes at their final coordinates
   * (no animation) or if the final frame does not (the animation converges
   * somewhere other than the deterministic layout, which would make the
   * layout's reproducibility meaningless on screen).
   */
  it('arrives exactly at the laid-out coordinates and not before', () => {
    const nodes = [node('a', 'projects'), node('b', 'archive')];

    const midway = scene(nodes, [], { entryProgress: 0.5 });
    renderRings(midway.rc);
    const settled = scene(nodes, [], { entryProgress: 1 });
    renderRings(settled.rc);

    const target = settled.rc.layout.byId.get('b')!;
    const finalArc = settled.rec.gradients.find(
      (g) => Math.abs(g.x1 - target.x) < 1e-9 && Math.abs(g.y1 - target.y) < 1e-9,
    );
    expect(finalArc).toBeDefined();

    const midArc = midway.rec.gradients.find(
      (g) => Math.abs(g.x1 - target.x) < 1e-9 && Math.abs(g.y1 - target.y) < 1e-9,
    );
    expect(midArc).toBeUndefined();
  });

  /** Labels while everything is still moving is noise; fails if text is
   *  drawn for a node mid-flight. */
  it('holds node labels back until the motion has stopped', () => {
    const nodes = [node('a', 'projects')];

    const midway = scene(nodes, [], { entryProgress: 0.4, camera: { x: 0, y: 0, k: 1 } });
    renderRings(midway.rc);
    expect(midway.rec.texts).not.toContain('a');

    const settled = scene(nodes, [], { entryProgress: 1, camera: { x: 0, y: 0, k: 1 } });
    renderRings(settled.rc);
    expect(settled.rec.texts).toContain('a');
  });

  it('reports the viewer preference for reduced motion', () => {
    const original = window.matchMedia;

    window.matchMedia = vi.fn().mockReturnValue({ matches: true }) as never;
    expect(prefersReducedMotion()).toBe(true);

    window.matchMedia = vi.fn().mockReturnValue({ matches: false }) as never;
    expect(prefersReducedMotion()).toBe(false);

    // A host with no media-query support must not throw on a render path.
    window.matchMedia = undefined as never;
    expect(prefersReducedMotion()).toBe(false);

    window.matchMedia = original;
  });
});

describe('hit testing', () => {
  it('finds the node under the pointer and nothing where there is none', () => {
    const nodes = [node('a', 'projects'), node('b', 'archive')];
    const layout = layoutRings(nodes, []);
    const target = layout.byId.get('b')!;
    const camera = { x: 0, y: 0, k: 1 };

    const screenX = target.x * camera.k + 800 / 2 + camera.x;
    const screenY = target.y * camera.k + 600 / 2 + camera.y;

    expect(hitTestRings(layout, screenX, screenY, 800, 600, camera)).toBe('b');
    expect(hitTestRings(layout, 0, 0, 800, 600, camera)).toBeNull();
  });

  /** Clicking through a filter should not select what the filter says you
   *  are not looking at. */
  it('ignores a node the filter has dimmed', () => {
    const nodes = [node('a', 'projects')];
    const layout = layoutRings(nodes, []);
    const target = layout.byId.get('a')!;
    const camera = { x: 0, y: 0, k: 1 };
    const screenX = target.x + 400;
    const screenY = target.y + 300;

    expect(hitTestRings(layout, screenX, screenY, 800, 600, camera)).toBe('a');
    expect(
      hitTestRings(layout, screenX, screenY, 800, 600, camera, new Map([['a', 0.07]])),
    ).toBeNull();
  });
});
