import { describe, expect, it } from 'vitest';

import { isDirectional, layoutFocus, pathEdgeKeys, pathToRoot } from './focusLayout';
import type { KnowledgeEdge, KnowledgeNode } from '@/types';

const node = (id: string, pagerank = 0.1): KnowledgeNode => ({
  id,
  node_type: 'scribble',
  label: id,
  metadata: {},
  degree: 1,
  pagerank,
});

const edge = (
  sourceId: string,
  targetId: string,
  relationship = 'related_to',
  overrides: Partial<KnowledgeEdge> = {},
): KnowledgeEdge => ({
  id: `${sourceId}->${targetId}:${relationship}`,
  source_id: sourceId,
  target_id: targetId,
  relationship,
  confidence: 1,
  source: 'user',
  ...overrides,
});

describe('layoutFocus', () => {
  it('puts the root at depth zero and layers outward left to right', () => {
    const nodes = [node('root'), node('a'), node('b'), node('deep')];
    const edges = [edge('root', 'a'), edge('root', 'b'), edge('a', 'deep')];

    const layout = layoutFocus('root', nodes, edges);

    expect(layout.byId.get('root')!.depth).toBe(0);
    expect(layout.byId.get('a')!.depth).toBe(1);
    expect(layout.byId.get('deep')!.depth).toBe(2);
    expect(layout.byId.get('deep')!.x).toBeGreaterThan(layout.byId.get('a')!.x);
    expect(layout.byId.get('a')!.x).toBeGreaterThan(layout.byId.get('root')!.x);
  });

  /**
   * The case the brief calls out. Fails if the node appears twice (a tree
   * faked by duplication) or if either link is missing (a tree faked by
   * dropping one).
   */
  it('renders a two-parent node once, with one primary edge and one dashed cross-link', () => {
    const nodes = [node('root'), node('projectA'), node('projectB'), node('child')];
    const edges = [
      edge('root', 'projectA'),
      edge('root', 'projectB'),
      // The child belongs to one project and derives from the other.
      edge('child', 'projectA', 'belongs_to'),
      edge('child', 'projectB', 'derived_from'),
    ];

    const layout = layoutFocus('root', nodes, edges);

    const appearances = layout.nodes.filter((n) => n.id === 'child');
    expect(appearances).toHaveLength(1);

    const childEdges = layout.edges.filter(
      (e) => e.sourceId === 'child' || e.targetId === 'child',
    );
    expect(childEdges).toHaveLength(2);

    const primary = childEdges.filter((e) => !e.crossLink);
    const cross = childEdges.filter((e) => e.crossLink);
    expect(primary).toHaveLength(1);
    expect(cross).toHaveLength(1);

    // Containment wins the parent slot over derivation, so the node hangs
    // from the project it belongs to.
    expect(layout.byId.get('child')!.parentId).toBe('projectA');
    expect(primary[0].relationship).toBe('belongs_to');
    expect(cross[0].relationship).toBe('derived_from');
  });

  /** Fails if the same root and vault lay out differently between runs —
   *  e.g. because edge arrival order decided which parent won. */
  it('is reproducible, whatever order the edges arrive in', () => {
    const nodes = [node('root'), node('x'), node('y'), node('z')];
    const edges = [edge('root', 'x'), edge('root', 'y'), edge('x', 'z'), edge('y', 'z')];

    const first = layoutFocus('root', nodes, edges);
    const second = layoutFocus('root', [...nodes].reverse(), [...edges].reverse());

    for (const n of first.nodes) {
      const other = second.byId.get(n.id)!;
      expect(other.x).toBe(n.x);
      expect(other.y).toBe(n.y);
      expect(other.parentId).toBe(n.parentId);
    }
  });

  it('keeps direction on the relationships that have one, and invents none', () => {
    const nodes = [node('root'), node('a'), node('b')];
    const edges = [edge('a', 'root', 'derived_from'), edge('b', 'root', 'same_topic')];

    const layout = layoutFocus('root', nodes, edges);

    expect(layout.edges.find((e) => e.relationship === 'derived_from')!.directional).toBe(true);
    expect(layout.edges.find((e) => e.relationship === 'same_topic')!.directional).toBe(false);
    expect(isDirectional('BELONGS_TO')).toBe(true);
    expect(isDirectional('related_to')).toBe(false);
    expect(isDirectional(null)).toBe(false);
  });

  /** The view answers "how did I get here", so something unreachable from
   *  the root is not part of any answer. */
  it('leaves out what the root cannot reach', () => {
    const nodes = [node('root'), node('near'), node('island')];
    const layout = layoutFocus('root', nodes, [edge('root', 'near')]);

    expect(layout.byId.has('near')).toBe(true);
    expect(layout.byId.has('island')).toBe(false);
  });

  it('stops at the depth limit rather than walking the whole vault', () => {
    const nodes = [node('n0'), node('n1'), node('n2'), node('n3'), node('n4')];
    const edges = [edge('n0', 'n1'), edge('n1', 'n2'), edge('n2', 'n3'), edge('n3', 'n4')];

    const layout = layoutFocus('n0', nodes, edges, 2);

    expect(layout.byId.has('n2')).toBe(true);
    expect(layout.byId.has('n3')).toBe(false);
    expect(layout.maxDepth).toBe(2);
  });

  it('is empty for a root that is not in the graph', () => {
    expect(layoutFocus('ghost', [node('root')], []).nodes).toHaveLength(0);
    expect(layoutFocus(null, [node('root')], []).nodes).toHaveLength(0);
  });

  it('sits a parent level with the middle of its children', () => {
    const nodes = [node('root'), node('a'), node('b'), node('c')];
    const edges = [edge('root', 'a'), edge('root', 'b'), edge('root', 'c')];

    const layout = layoutFocus('root', nodes, edges);
    const kids = ['a', 'b', 'c'].map((id) => layout.byId.get(id)!.y).sort((x, y) => x - y);

    expect(layout.byId.get('root')!.y).toBeCloseTo((kids[0] + kids[2]) / 2, 6);
  });
});

describe('pathToRoot', () => {
  /** The feature: clicking a node shows the route it arrived by. */
  it('returns the chain from the root down to the node', () => {
    const nodes = [node('root'), node('mid'), node('leaf')];
    const edges = [edge('root', 'mid'), edge('mid', 'leaf')];

    const layout = layoutFocus('root', nodes, edges);

    expect(pathToRoot(layout, 'leaf')).toEqual(['root', 'mid', 'leaf']);
    expect(pathToRoot(layout, 'root')).toEqual(['root']);
  });

  it('has no path for a node outside the tree, rather than a wrong one', () => {
    const layout = layoutFocus('root', [node('root')], []);
    expect(pathToRoot(layout, 'elsewhere')).toEqual([]);
    expect(pathToRoot(layout, null)).toEqual([]);
  });

  /**
   * The path follows primary edges only. A node reached through a
   * cross-link still shows the route it actually hangs from — claiming the
   * cross-link as the path would answer the question wrongly.
   */
  it('walks primary edges, not cross-links', () => {
    const nodes = [node('root'), node('projectA'), node('projectB'), node('child')];
    const edges = [
      edge('root', 'projectA'),
      edge('root', 'projectB'),
      edge('child', 'projectA', 'belongs_to'),
      edge('child', 'projectB', 'derived_from'),
    ];

    const layout = layoutFocus('root', nodes, edges);
    expect(pathToRoot(layout, 'child')).toEqual(['root', 'projectA', 'child']);
  });

  it('keys path edges in both directions, since storage order is arbitrary', () => {
    const keys = pathEdgeKeys(['root', 'mid', 'leaf']);
    expect(keys.has('root|mid')).toBe(true);
    expect(keys.has('mid|root')).toBe(true);
    expect(keys.has('leaf|mid')).toBe(true);
    expect(keys.has('root|leaf')).toBe(false);
    expect(pathEdgeKeys([]).size).toBe(0);
  });
});
