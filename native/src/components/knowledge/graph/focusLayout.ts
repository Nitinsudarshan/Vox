import type { KnowledgeEdge, KnowledgeNode } from '@/types';

/**
 * Focus & Flow: one root, work branching outward, direction shown honestly.
 *
 * ## Knowledge is not a tree, and this does not pretend it is
 *
 * A scribble can have two parents — derived from one note while belonging
 * to another project. Two ways of hiding that are both wrong: forcing a
 * tree drops one of the links, and duplicating the node to give each parent
 * its own copy tells the user there are two thoughts when there is one.
 *
 * So the layout picks a **primary** parent to hang the node from, and every
 * other real link the node has becomes a **cross-link** — drawn, dashed,
 * leaving the tree. Nothing is dropped and nothing is duplicated; the
 * structure is a tree and the graph is still a graph.
 *
 * ## Reproducibility
 *
 * Breadth-first from the root, with neighbours visited in id order and ties
 * broken by a fixed preference. No randomness and no iteration, so the same
 * root over the same vault lays out identically every time.
 */

/**
 * Relationship types that mean something directional.
 *
 * `DerivedFrom` and `BelongsTo` point somewhere — this note came *from*
 * that one, this work sits *inside* that project — and drawing them without
 * direction loses the claim. `RELATED_TO` and `SAME_TOPIC` are symmetric
 * and get no arrowhead, because inventing one would assert a direction the
 * data does not have.
 */
export const DIRECTIONAL_RELATIONSHIPS = new Set([
  'derived_from',
  'belongs_to',
  'summarizes',
  'analyses',
  'references',
  'supersedes',
  'extends',
]);

export function isDirectional(relationship: string | null | undefined): boolean {
  return DIRECTIONAL_RELATIONSHIPS.has((relationship ?? '').toLowerCase());
}

/**
 * How strongly a relationship type argues for being a node's primary parent.
 *
 * Containment beats derivation beats everything else: if a thought belongs
 * to a project and also happens to share a topic with some other note, the
 * project is the honest place to hang it from. Higher wins.
 */
const PARENT_PREFERENCE: Record<string, number> = {
  belongs_to: 5,
  derived_from: 4,
  summarizes: 3,
  extends: 3,
  analyses: 2,
  references: 2,
  supersedes: 2,
  related_to: 1,
  mentions: 0,
  same_topic: 0,
};

function parentPreference(relationship: string): number {
  return PARENT_PREFERENCE[relationship.toLowerCase()] ?? 1;
}

export interface FocusNodeLayout {
  id: string;
  /** Distance from the root in tree hops. The root is 0. */
  depth: number;
  x: number;
  y: number;
  size: number;
  /** The node this one hangs from. `null` only for the root. */
  parentId: string | null;
}

export interface FocusEdgeLayout {
  id: string;
  sourceId: string;
  targetId: string;
  relationship: string;
  confidence: number;
  /** True when the stored relationship type points somewhere. */
  directional: boolean;
  /** True when this is not the node's primary parent edge. */
  crossLink: boolean;
  suggested: boolean;
}

export interface FocusLayout {
  rootId: string | null;
  nodes: FocusNodeLayout[];
  byId: Map<string, FocusNodeLayout>;
  edges: FocusEdgeLayout[];
  maxDepth: number;
  width: number;
  height: number;
}

const COLUMN_GAP = 240;
const ROW_GAP = 46;

const EMPTY: FocusLayout = {
  rootId: null,
  nodes: [],
  byId: new Map(),
  edges: [],
  maxDepth: 0,
  width: 0,
  height: 0,
};

/**
 * Lays the reachable graph out left to right from `rootId`.
 *
 * Only what is connected to the root appears: the question this view
 * answers is "how did I get here", and an unreachable node is not part of
 * any answer to it.
 */
export function layoutFocus(
  rootId: string | null,
  nodes: KnowledgeNode[],
  edges: KnowledgeEdge[],
  maxDepth = 4,
): FocusLayout {
  if (!rootId || !nodes.some((n) => n.id === rootId)) return EMPTY;

  const present = new Set(nodes.map((n) => n.id));
  const rankById = new Map(nodes.map((n) => [n.id, n.pagerank ?? 0]));
  const maxRank = Math.max(...nodes.map((n) => n.pagerank ?? 0), 0);

  // Adjacency in both directions, so a parent can be reached from a child
  // whichever way the stored edge happens to point.
  const links = new Map<string, KnowledgeEdge[]>();
  for (const edge of edges) {
    if (!present.has(edge.source_id) || !present.has(edge.target_id)) continue;
    if (edge.source_id === edge.target_id) continue;
    if (!links.has(edge.source_id)) links.set(edge.source_id, []);
    if (!links.has(edge.target_id)) links.set(edge.target_id, []);
    links.get(edge.source_id)!.push(edge);
    links.get(edge.target_id)!.push(edge);
  }

  // --- Breadth-first, choosing one primary parent per node ---------------
  const depth = new Map<string, number>([[rootId, 0]]);
  const parent = new Map<string, string | null>([[rootId, null]]);
  const parentEdgeId = new Map<string, string>();
  const order: string[] = [rootId];

  for (let cursor = 0; cursor < order.length; cursor++) {
    const current = order[cursor];
    const currentDepth = depth.get(current)!;
    if (currentDepth >= maxDepth) continue;

    // Neighbours in a fixed order: id, then the strongest parent claim
    // first, so which parent wins is a property of the data rather than of
    // the order edges happened to arrive in.
    const neighbours = [...(links.get(current) ?? [])]
      .map((edge) => ({
        edge,
        other: edge.source_id === current ? edge.target_id : edge.source_id,
      }))
      .sort((a, b) => {
        const byPreference =
          parentPreference(b.edge.relationship) - parentPreference(a.edge.relationship);
        if (byPreference !== 0) return byPreference;
        return a.other < b.other ? -1 : a.other > b.other ? 1 : 0;
      });

    for (const { edge, other } of neighbours) {
      if (depth.has(other)) continue;
      depth.set(other, currentDepth + 1);
      parent.set(other, current);
      parentEdgeId.set(other, edge.id);
      order.push(other);
    }
  }

  // --- Vertical placement: depth-first over the tree ---------------------
  // Walking the tree rather than bucketing by depth is what keeps a
  // subtree together vertically; bucketing would interleave cousins and
  // make the branches unreadable.
  const children = new Map<string, string[]>();
  for (const id of order) {
    const p = parent.get(id);
    if (!p) continue;
    if (!children.has(p)) children.set(p, []);
    children.get(p)!.push(id);
  }
  for (const list of children.values()) {
    list.sort((a, b) => {
      const byRank = (rankById.get(b) ?? 0) - (rankById.get(a) ?? 0);
      return byRank !== 0 ? byRank : a < b ? -1 : 1;
    });
  }

  const laid: FocusNodeLayout[] = [];
  let row = 0;

  const place = (id: string) => {
    const d = depth.get(id)!;
    const rank = maxRank > 0 ? (rankById.get(id) ?? 0) / maxRank : 0;
    const kids = children.get(id) ?? [];

    if (kids.length === 0) {
      laid.push({
        id,
        depth: d,
        x: d * COLUMN_GAP,
        y: row * ROW_GAP,
        size: 5 + Math.sqrt(rank) * 12,
        parentId: parent.get(id) ?? null,
      });
      row += 1;
      return;
    }

    const firstRow = row;
    for (const kid of kids) place(kid);
    const lastRow = row - 1;

    // A parent sits level with the middle of its children, which is what
    // makes a branch read as one thing rather than a column of dots.
    laid.push({
      id,
      depth: d,
      x: d * COLUMN_GAP,
      y: ((firstRow + lastRow) / 2) * ROW_GAP,
      size: 5 + Math.sqrt(rank) * 12,
      parentId: parent.get(id) ?? null,
    });
  };

  place(rootId);
  laid.sort((a, b) => (a.id < b.id ? -1 : a.id > b.id ? 1 : 0));
  const byId = new Map(laid.map((n) => [n.id, n]));

  // --- Edges: one primary per node, everything else a cross-link ---------
  const primaryEdgeIds = new Set(parentEdgeId.values());
  const seenEdges = new Set<string>();
  const laidEdges: FocusEdgeLayout[] = [];

  for (const edge of edges) {
    if (!byId.has(edge.source_id) || !byId.has(edge.target_id)) continue;
    if (edge.source_id === edge.target_id) continue;
    if (seenEdges.has(edge.id)) continue;
    seenEdges.add(edge.id);

    laidEdges.push({
      id: edge.id,
      sourceId: edge.source_id,
      targetId: edge.target_id,
      relationship: edge.relationship,
      confidence: edge.confidence ?? 1,
      directional: isDirectional(edge.relationship),
      // Every real link is drawn. The ones that are not holding a node in
      // the tree leave it as cross-links rather than being discarded.
      crossLink: !primaryEdgeIds.has(edge.id),
      suggested: (edge.source ?? '').toLowerCase() === 'ai',
    });
  }
  laidEdges.sort((a, b) => (a.id < b.id ? -1 : a.id > b.id ? 1 : 0));

  const depths = laid.map((n) => n.depth);
  return {
    rootId,
    nodes: laid,
    byId,
    edges: laidEdges,
    maxDepth: depths.length ? Math.max(...depths) : 0,
    width: (depths.length ? Math.max(...depths) : 0) * COLUMN_GAP,
    height: Math.max(0, row - 1) * ROW_GAP,
  };
}

/**
 * The chain of node ids from the root down to `nodeId`, root first.
 *
 * This is the feature — clicking a node answers "how did I get here" by
 * lighting the route and dimming everything else. Returns an empty array
 * for a node outside the tree, which the caller renders as "no path"
 * rather than as an empty highlight.
 */
export function pathToRoot(layout: FocusLayout, nodeId: string | null): string[] {
  if (!nodeId || !layout.byId.has(nodeId)) return [];

  const path: string[] = [];
  const guard = new Set<string>();
  let cursor: string | null = nodeId;

  while (cursor && layout.byId.has(cursor) && !guard.has(cursor)) {
    guard.add(cursor);
    path.push(cursor);
    cursor = layout.byId.get(cursor)!.parentId;
  }

  return path.reverse();
}

/**
 * The edges along a path, as `sourceId|targetId` keys in both directions.
 *
 * Both directions because the renderer looks an edge up by the way it is
 * stored, which is not necessarily the way the path walks it.
 */
export function pathEdgeKeys(path: string[]): Set<string> {
  const keys = new Set<string>();
  for (let i = 1; i < path.length; i++) {
    keys.add(`${path[i - 1]}|${path[i]}`);
    keys.add(`${path[i]}|${path[i - 1]}`);
  }
  return keys;
}
