import type { KnowledgeEdge, KnowledgeNode, ParaBand } from '@/types';
import { PARA_BANDS } from '@/types';

/**
 * The Rings layout: PARA as concentric bands, position carrying meaning.
 *
 * Two phases, in this order and no other:
 *
 * 1. **Band assignment** is deterministic and total. A node's ring is a
 *    lookup on its PARA band, not an outcome of the simulation, so a node
 *    can never drift into a ring that misdescribes it.
 * 2. **Relaxation** runs *inside* the band. Linked nodes pull together in
 *    angle so related work lines up radially across rings; nodes in the same
 *    band push apart so they do not stack. Radius is clamped to the band on
 *    every iteration, so the constraint holds throughout rather than being
 *    applied once at the end.
 *
 * ## Why this is reproducible
 *
 * A fixed iteration count with no convergence test, and a seeded RNG rather
 * than `Math.random`. An early exit on "close enough" makes the number of
 * iterations depend on the input, which makes the output depend on the input
 * in a way nobody can predict — the same vault would settle differently
 * after an unrelated edit. Here the same input does the same arithmetic in
 * the same order and produces bit-identical coordinates.
 *
 * ## Why filtering is not an input
 *
 * `layoutRings` takes every node, never a filtered subset. Filtering changes
 * opacity (`applyVisibility`) and nothing else. If a filter could reach the
 * layout, hiding topics would rearrange the scribbles, and the position a
 * user had learned to read would stop meaning what it meant.
 */

/** A ring, including the one for nodes that were never filed. */
export type RingBand = ParaBand | 'uncategorised';

/**
 * Every ring, innermost first.
 *
 * `uncategorised` is a real ring rather than a hidden state: PARA is stored
 * explicitly and most of a vault starts unfiled, so those nodes have to land
 * somewhere the user can see them. Putting them outside Archive says "not
 * filed" without claiming they were archived.
 */
export const RING_BANDS: RingBand[] = [...PARA_BANDS, 'uncategorised'];

/** Band extents as a fraction of the layout's outer radius. */
const BAND_EXTENTS: Record<RingBand, { inner: number; outer: number }> = {
  projects: { inner: 0.07, outer: 0.2 },
  areas: { inner: 0.26, outer: 0.41 },
  resources: { inner: 0.47, outer: 0.62 },
  archive: { inner: 0.68, outer: 0.81 },
  uncategorised: { inner: 0.87, outer: 1.0 },
};

/** World-space radius of the outermost ring. Viewport-independent by design:
 *  a layout that depended on window size would not survive a resize. */
export const RINGS_OUTER_RADIUS = 1000;

/** Fixed, deliberately not a convergence threshold. See the module comment. */
const RELAXATION_ITERATIONS = 160;

/** How hard a link pulls its endpoints together in angle. */
const LINK_ANGULAR_PULL = 0.055;

/** How hard two same-band neighbours push apart in angle. */
const SAME_BAND_PUSH = 0.35;

/** Angular separation, in radians, below which same-band nodes repel. */
const MIN_ANGULAR_GAP = 0.045;

export interface RingNodeLayout {
  id: string;
  band: RingBand;
  /** World coordinates. Origin is the centre of the rings. */
  x: number;
  y: number;
  angle: number;
  radius: number;
  /** Drawing radius in world units, scaled from PageRank. */
  size: number;
  /** 0 = oldest thing in the vault, 1 = the most recently touched. */
  recency: number;
}

export interface RingBandLayout {
  band: RingBand;
  innerRadius: number;
  outerRadius: number;
  count: number;
}

export interface RingsLayout {
  nodes: RingNodeLayout[];
  byId: Map<string, RingNodeLayout>;
  bands: RingBandLayout[];
  outerRadius: number;
}

/**
 * Deterministic 32-bit PRNG (mulberry32).
 *
 * Seeded from a constant, so the sequence is the same on every run and on
 * every machine. `Math.random` would make the layout unreproducible, which
 * is the one property this view is built on.
 */
function mulberry32(seed: number): () => number {
  let a = seed >>> 0;
  return () => {
    a = (a + 0x6d2b79f5) >>> 0;
    let t = a;
    t = Math.imul(t ^ (t >>> 15), t | 1);
    t ^= t + Math.imul(t ^ (t >>> 7), t | 61);
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}

const LAYOUT_SEED = 0x5ca1ab1e;

/** The ring a node belongs to. Only scribbles carry a PARA band. */
export function bandOf(node: KnowledgeNode): RingBand {
  const para = node.para;
  return para && (PARA_BANDS as readonly string[]).includes(para)
    ? (para as ParaBand)
    : 'uncategorised';
}

/** Normalises an angle into (-π, π]. */
function wrapAngle(a: number): number {
  const twoPi = Math.PI * 2;
  let r = a % twoPi;
  if (r > Math.PI) r -= twoPi;
  if (r <= -Math.PI) r += twoPi;
  return r;
}

/**
 * Milliseconds for a node's `updated_at`, or `null` when it has none.
 *
 * Topics and entities inherit a timestamp from the scribbles that produced
 * them, so in practice only a malformed node lands here without one.
 */
function timestampOf(node: KnowledgeNode): number | null {
  const raw = node.updated_at ?? (node.metadata?.updated_at as string | undefined);
  if (!raw) return null;
  const parsed = Date.parse(raw);
  return Number.isNaN(parsed) ? null : parsed;
}

/**
 * Computes the whole layout.
 *
 * Pass every node in the graph, filtered or not — see the module comment for
 * why a filtered subset is the wrong input.
 */
export function layoutRings(
  nodes: KnowledgeNode[],
  edges: KnowledgeEdge[],
  outerRadius: number = RINGS_OUTER_RADIUS,
): RingsLayout {
  // Sort by id up front. Node arrival order comes from a HashMap on the Rust
  // side and from filter operations on this one; neither is a stable basis
  // for a layout that promises the same picture twice.
  const ordered = [...nodes].sort((a, b) => (a.id < b.id ? -1 : a.id > b.id ? 1 : 0));

  const rng = mulberry32(LAYOUT_SEED);

  const maxPagerank = ordered.reduce((max, n) => Math.max(max, n.pagerank ?? 0), 0);
  const timestamps = ordered.map(timestampOf).filter((t): t is number => t !== null);
  const oldest = timestamps.length ? Math.min(...timestamps) : 0;
  const newest = timestamps.length ? Math.max(...timestamps) : 0;
  const span = newest - oldest;

  // --- Phase 1: band assignment, then a seeded starting position ---------
  const byBand = new Map<RingBand, KnowledgeNode[]>();
  for (const band of RING_BANDS) byBand.set(band, []);
  for (const node of ordered) {
    byBand.get(bandOf(node))!.push(node);
  }

  const laid: RingNodeLayout[] = [];
  const indexById = new Map<string, number>();

  for (const band of RING_BANDS) {
    const members = byBand.get(band)!;
    if (members.length === 0) continue;

    const extent = BAND_EXTENTS[band];
    const inner = extent.inner * outerRadius;
    const outer = extent.outer * outerRadius;

    // Seat the band's most structurally important nodes first, spread
    // evenly around the circle, so a hub is never buried behind a leaf.
    const seated = [...members].sort((a, b) => {
      const byRank = (b.pagerank ?? 0) - (a.pagerank ?? 0);
      return byRank !== 0 ? byRank : a.id < b.id ? -1 : 1;
    });

    for (let i = 0; i < seated.length; i++) {
      const node = seated[i];
      const rank = maxPagerank > 0 ? (node.pagerank ?? 0) / maxPagerank : 0;
      const stamp = timestampOf(node);

      laid.push({
        id: node.id,
        band,
        angle: wrapAngle((i / seated.length) * Math.PI * 2 + rng() * 0.12),
        // Jitter the radius inside the band so the ring reads as a region
        // rather than a wire.
        radius: inner + rng() * (outer - inner),
        x: 0,
        y: 0,
        // sqrt keeps the biggest hub from swallowing the view: PageRank is
        // long-tailed, so a linear map would make one node enormous and the
        // rest indistinguishable dots.
        size: 4 + Math.sqrt(rank) * 16,
        recency: span > 0 && stamp !== null ? (stamp - oldest) / span : 1,
      });
      indexById.set(node.id, laid.length - 1);
    }
  }

  // --- Phase 2: relaxation, constrained to the band ----------------------
  // Edges resolved to indices once; both endpoints must be in the layout.
  const links: Array<{ a: number; b: number; weight: number }> = [];
  for (const edge of edges) {
    const a = indexById.get(edge.source_id);
    const b = indexById.get(edge.target_id);
    if (a === undefined || b === undefined || a === b) continue;
    links.push({ a, b, weight: Math.max(0.1, Math.min(1, edge.confidence ?? 1)) });
  }

  // Same-band neighbour lists, in angular order, so repulsion is O(n) per
  // band rather than O(n²) over the whole graph.
  const bandMembers = new Map<RingBand, number[]>();
  for (let i = 0; i < laid.length; i++) {
    const list = bandMembers.get(laid[i].band);
    if (list) list.push(i);
    else bandMembers.set(laid[i].band, [i]);
  }

  const angularDelta = new Float64Array(laid.length);

  for (let iter = 0; iter < RELAXATION_ITERATIONS; iter++) {
    angularDelta.fill(0);

    // Links pull endpoints together in angle. Radius is untouched — pulling
    // radially is what would drag a node out of its ring.
    for (let i = 0; i < links.length; i++) {
      const { a, b, weight } = links[i];
      const diff = wrapAngle(laid[b].angle - laid[a].angle);
      const pull = diff * LINK_ANGULAR_PULL * weight;
      angularDelta[a] += pull;
      angularDelta[b] -= pull;
    }

    // Same-band crowding. Sorting by angle each iteration keeps the
    // comparison to actual neighbours; the sort is stable on id, so it does
    // not introduce an order that varies between runs.
    for (const members of bandMembers.values()) {
      if (members.length < 2) continue;
      const sorted = [...members].sort((x, y) => {
        const d = laid[x].angle - laid[y].angle;
        return d !== 0 ? d : laid[x].id < laid[y].id ? -1 : 1;
      });
      for (let k = 0; k < sorted.length; k++) {
        const cur = sorted[k];
        const next = sorted[(k + 1) % sorted.length];
        if (cur === next) continue;
        const gap = Math.abs(wrapAngle(laid[next].angle - laid[cur].angle));
        if (gap < MIN_ANGULAR_GAP) {
          const push = (MIN_ANGULAR_GAP - gap) * SAME_BAND_PUSH;
          angularDelta[cur] -= push;
          angularDelta[next] += push;
        }
      }
    }

    // Cooling schedule is a function of the iteration index alone, so it is
    // identical on every run regardless of how the graph is shaped.
    const cooling = 1 - iter / RELAXATION_ITERATIONS;
    for (let i = 0; i < laid.length; i++) {
      laid[i].angle = wrapAngle(laid[i].angle + angularDelta[i] * cooling);

      // The constraint, re-applied every iteration rather than once at the
      // end: a node is inside its band at all times, never on its way back.
      const extent = BAND_EXTENTS[laid[i].band];
      const inner = extent.inner * outerRadius;
      const outer = extent.outer * outerRadius;
      laid[i].radius = Math.min(outer, Math.max(inner, laid[i].radius));
    }
  }

  for (const node of laid) {
    node.x = Math.cos(node.angle) * node.radius;
    node.y = Math.sin(node.angle) * node.radius;
  }

  // Emit in id order so consumers see a stable array, whatever order the
  // bands happened to seat their members in.
  laid.sort((a, b) => (a.id < b.id ? -1 : a.id > b.id ? 1 : 0));

  const bands: RingBandLayout[] = RING_BANDS.map((band) => ({
    band,
    innerRadius: BAND_EXTENTS[band].inner * outerRadius,
    outerRadius: BAND_EXTENTS[band].outer * outerRadius,
    count: byBand.get(band)?.length ?? 0,
  }));

  return {
    nodes: laid,
    byId: new Map(laid.map((n) => [n.id, n])),
    bands,
    outerRadius,
  };
}

/**
 * Opacity per node for a filter predicate.
 *
 * Deliberately separate from `layoutRings` and deliberately returns opacity
 * rather than a node list: this is the whole mechanism by which filtering
 * dims instead of moving. A caller cannot accidentally re-layout a filtered
 * set, because the filtered set never reaches the layout.
 */
export function applyVisibility(
  layout: RingsLayout,
  isVisible: (nodeId: string) => boolean,
  dimmedOpacity = 0.07,
): Map<string, number> {
  const opacity = new Map<string, number>();
  for (const node of layout.nodes) {
    opacity.set(node.id, isVisible(node.id) ? 1 : dimmedOpacity);
  }
  return opacity;
}
