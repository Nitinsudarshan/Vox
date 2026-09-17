import { describe, expect, it } from 'vitest';

import {
  applyVisibility,
  bandOf,
  layoutRings,
  RING_BANDS,
  RINGS_OUTER_RADIUS,
  type RingBand,
} from './ringsLayout';
import type { KnowledgeEdge, KnowledgeNode, ParaBand } from '@/types';

/**
 * The Rings layout is the one view whose whole claim is that position means
 * something and keeps meaning it. These assert the two properties that claim
 * rests on — the same vault lays out identically, and filtering never moves
 * a node — plus the constraint that makes the rings readable at all.
 */

const node = (
  id: string,
  para: ParaBand | null = null,
  overrides: Partial<KnowledgeNode> = {},
): KnowledgeNode => ({
  id,
  node_type: 'scribble',
  label: id,
  metadata: {},
  degree: 0,
  para,
  pagerank: 0.01,
  updated_at: '2026-01-01T00:00:00Z',
  ...overrides,
});

const edge = (source_id: string, target_id: string, confidence = 1): KnowledgeEdge => ({
  id: `${source_id}->${target_id}`,
  source_id,
  target_id,
  relationship: 'RELATED_TO',
  confidence,
  source: 'user',
});

/** A vault with every band populated and enough links to make relaxation work. */
function sampleVault(): { nodes: KnowledgeNode[]; edges: KnowledgeEdge[] } {
  const bands: Array<ParaBand | null> = ['projects', 'areas', 'resources', 'archive', null];
  const nodes: KnowledgeNode[] = [];
  for (let i = 0; i < 60; i++) {
    nodes.push(
      node(`n${i}`, bands[i % bands.length], {
        pagerank: 0.001 * ((i % 9) + 1),
        updated_at: new Date(Date.UTC(2025, i % 12, (i % 27) + 1)).toISOString(),
      }),
    );
  }
  const edges: KnowledgeEdge[] = [];
  for (let i = 0; i < 60; i++) {
    edges.push(edge(`n${i}`, `n${(i * 7 + 3) % 60}`, 0.4 + (i % 6) / 10));
  }
  return { nodes, edges };
}

describe('layoutRings determinism', () => {
  /**
   * Fails if any coordinate differs by any amount. `toBe` is `Object.is`, so
   * a difference of one ULP fails this — which is the point: "nearly the
   * same layout" is a layout the user cannot learn to read.
   */
  it('produces bit-identical coordinates for the same input, twice over', () => {
    const { nodes, edges } = sampleVault();

    const first = layoutRings(nodes, edges);
    const second = layoutRings(nodes, edges);

    expect(second.nodes).toHaveLength(first.nodes.length);
    expect(first.nodes.length).toBeGreaterThan(0);
    for (let i = 0; i < first.nodes.length; i++) {
      expect(second.nodes[i].id).toBe(first.nodes[i].id);
      expect(second.nodes[i].x).toBe(first.nodes[i].x);
      expect(second.nodes[i].y).toBe(first.nodes[i].y);
      expect(second.nodes[i].angle).toBe(first.nodes[i].angle);
      expect(second.nodes[i].radius).toBe(first.nodes[i].radius);
    }
  });

  /**
   * Node arrival order comes from a Rust `HashMap` and from whatever the
   * caller did to the array on the way in. If order leaked into the layout,
   * the graph would rearrange itself between two reads of an unchanged
   * vault. Fails if any coordinate moves when the input is shuffled.
   */
  it('does not depend on the order nodes arrive in', () => {
    const { nodes, edges } = sampleVault();
    const shuffled = [...nodes].reverse();

    const inOrder = layoutRings(nodes, edges);
    const outOfOrder = layoutRings(shuffled, edges);

    for (const node of inOrder.nodes) {
      const other = outOfOrder.byId.get(node.id);
      expect(other).toBeDefined();
      expect(other!.x).toBe(node.x);
      expect(other!.y).toBe(node.y);
    }
  });

  /** Two independent runs must agree on ring membership too, not just on
   *  coordinates that happen to round the same way. */
  it('assigns each node the ring its PARA band names', () => {
    const nodes = [
      node('p', 'projects'),
      node('a', 'areas'),
      node('r', 'resources'),
      node('x', 'archive'),
      node('u', null),
    ];

    const layout = layoutRings(nodes, []);

    expect(layout.byId.get('p')!.band).toBe('projects');
    expect(layout.byId.get('a')!.band).toBe('areas');
    expect(layout.byId.get('r')!.band).toBe('resources');
    expect(layout.byId.get('x')!.band).toBe('archive');
    expect(layout.byId.get('u')!.band).toBe('uncategorised');
  });
});

describe('layoutRings band constraint', () => {
  /**
   * The constraint that makes the view legible. Fails if any node's radius
   * falls outside the extents of the band it was assigned — which is what
   * an unconstrained relaxation would eventually produce.
   */
  it('never lets a node leave its ring', () => {
    const { nodes, edges } = sampleVault();
    const layout = layoutRings(nodes, edges);

    const extents = new Map<RingBand, { inner: number; outer: number }>(
      layout.bands.map((b) => [b.band, { inner: b.innerRadius, outer: b.outerRadius }]),
    );

    for (const n of layout.nodes) {
      const extent = extents.get(n.band)!;
      const distance = Math.hypot(n.x, n.y);
      // Float slack only: the constraint is a clamp, not an approximation.
      expect(distance).toBeGreaterThanOrEqual(extent.inner - 1e-9);
      expect(distance).toBeLessThanOrEqual(extent.outer + 1e-9);
    }
  });

  it('orders the rings Projects inward to Uncategorised outward', () => {
    const layout = layoutRings([], []);

    expect(layout.bands.map((b) => b.band)).toEqual(RING_BANDS);
    for (let i = 1; i < layout.bands.length; i++) {
      expect(layout.bands[i].innerRadius).toBeGreaterThan(layout.bands[i - 1].outerRadius);
    }
    expect(layout.bands[layout.bands.length - 1].outerRadius).toBe(RINGS_OUTER_RADIUS);
  });

  it('counts what landed in each ring', () => {
    const layout = layoutRings(
      [node('a', 'projects'), node('b', 'projects'), node('c', null)],
      [],
    );

    const counts = new Map(layout.bands.map((b) => [b.band, b.count]));
    expect(counts.get('projects')).toBe(2);
    expect(counts.get('areas')).toBe(0);
    expect(counts.get('uncategorised')).toBe(1);
  });
});

describe('filtering', () => {
  /**
   * The rule from the brief, as a test: a filter changes opacity and
   * nothing else. Fails if any node moves when a filter is applied — which
   * is what laying out a filtered subset would do.
   */
  it('dims without moving anything', () => {
    const { nodes, edges } = sampleVault();
    const layout = layoutRings(nodes, edges);
    const before = layout.nodes.map((n) => ({ id: n.id, x: n.x, y: n.y }));

    const opacity = applyVisibility(layout, (id) => id.endsWith('0'));

    for (const snapshot of before) {
      const after = layout.byId.get(snapshot.id)!;
      expect(after.x).toBe(snapshot.x);
      expect(after.y).toBe(snapshot.y);
    }
    expect(opacity.get('n10')).toBe(1);
    expect(opacity.get('n11')).toBeLessThan(1);
    expect(opacity.size).toBe(layout.nodes.length);
  });

  /**
   * The failure this guards against is subtler: laying out only what
   * survives a filter would leave the survivors in different places, so the
   * view would appear to rearrange as the user typed in a search box.
   */
  it('leaves survivors where they were when the hidden nodes are removed from the input', () => {
    const { nodes, edges } = sampleVault();
    const full = layoutRings(nodes, edges);

    const subset = nodes.filter((n) => n.para === 'projects');
    const laidOutSubset = layoutRings(subset, edges);

    const moved = subset.filter((n) => {
      const a = full.byId.get(n.id)!;
      const b = laidOutSubset.byId.get(n.id)!;
      return a.x !== b.x || a.y !== b.y;
    });

    expect(moved.length).toBeGreaterThan(0);
    // ...which is exactly why the view filters with `applyVisibility` over
    // the full layout instead of re-laying-out a subset.
  });
});

describe('node appearance inputs', () => {
  it('sizes nodes by PageRank rather than degree', () => {
    const layout = layoutRings(
      [
        node('hub', 'projects', { pagerank: 0.5, degree: 1 }),
        node('leaf', 'projects', { pagerank: 0.001, degree: 40 }),
      ],
      [],
    );

    expect(layout.byId.get('hub')!.size).toBeGreaterThan(layout.byId.get('leaf')!.size);
  });

  it('reads recency as a 0..1 ramp between the oldest and newest node', () => {
    const layout = layoutRings(
      [
        node('old', 'areas', { updated_at: '2020-01-01T00:00:00Z' }),
        node('mid', 'areas', { updated_at: '2023-01-01T00:00:00Z' }),
        node('new', 'areas', { updated_at: '2026-01-01T00:00:00Z' }),
      ],
      [],
    );

    expect(layout.byId.get('old')!.recency).toBe(0);
    expect(layout.byId.get('new')!.recency).toBe(1);
    const mid = layout.byId.get('mid')!.recency;
    expect(mid).toBeGreaterThan(0);
    expect(mid).toBeLessThan(1);
  });

  it('treats a node with no timestamp as current rather than ancient', () => {
    const layout = layoutRings([node('n', 'areas', { updated_at: null })], []);
    expect(layout.byId.get('n')!.recency).toBe(1);
  });

  it('files a node whose band is absent or unrecognised as uncategorised', () => {
    expect(bandOf(node('a', null))).toBe('uncategorised');
    expect(bandOf({ ...node('b'), para: 'someday' as ParaBand })).toBe('uncategorised');
    expect(bandOf(node('c', 'areas'))).toBe('areas');
  });
});
