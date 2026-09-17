import type { DecisionRecord, KnowledgeEdge, KnowledgeNode } from '@/types';

/**
 * Turns decisions into the layered graph the Decision Tree draws.
 *
 * The layout itself is Focus & Flow's — same breadth-first layering, same
 * reproducibility. What differs is what an edge means: here the edge
 * carries the decision, so its weight and its dashes say how much the
 * record is worth believing, not what kind of link it is.
 *
 * ## What is and is not shown
 *
 * Inferred decisions are excluded unless the caller asks for them. A faint
 * wrong line is still a line, and a guess on a tree that otherwise shows
 * what the user actually said is a guess that will be read as fact.
 *
 * Superseded decisions are always included. A tree that hides what was
 * reversed is a tree that cannot answer why the current answer is the
 * current answer — which is most of what it is for.
 */

/** Edge relationship for a subject's decision. */
export const REL_DECIDED = 'decided';
/** Edge relationship for a decision that replaced an earlier one. */
export const REL_SUPERSEDES = 'supersedes';

export interface DecisionGraph {
  nodes: KnowledgeNode[];
  edges: KnowledgeEdge[];
  /** Decision id → the record it came from, for the renderer and inspector. */
  byDecisionId: Map<string, DecisionRecord>;
  /** Node ids that are subjects rather than decisions. */
  subjectIds: string[];
}

/** A stable node id for a subject. */
export function subjectNodeId(subject: string): string {
  return `subject:${subject.trim().toLowerCase()}`;
}

/**
 * Builds the graph.
 *
 * `showGuesses` defaults to false, matching the toggle's default state —
 * the safe value is the one you get by forgetting to pass it.
 */
export function buildDecisionGraph(
  decisions: DecisionRecord[],
  showGuesses = false,
): DecisionGraph {
  const visible = decisions.filter(
    (d) => showGuesses || d.provenance !== 'inferred',
  );

  const nodes: KnowledgeNode[] = [];
  const edges: KnowledgeEdge[] = [];
  const byDecisionId = new Map<string, DecisionRecord>();
  const subjectIds: string[] = [];
  const seenSubjects = new Set<string>();

  // Sorted by id so the graph, and therefore the layout seeded from it, is
  // the same on every build.
  const ordered = [...visible].sort((a, b) => (a.id < b.id ? -1 : a.id > b.id ? 1 : 0));

  for (const decision of ordered) {
    const subjectId = subjectNodeId(decision.subject);
    if (!seenSubjects.has(subjectId)) {
      seenSubjects.add(subjectId);
      subjectIds.push(subjectId);
      nodes.push({
        id: subjectId,
        node_type: 'topic',
        label: decision.subject,
        metadata: { subject: decision.subject },
        degree: 0,
        pagerank: 0.1,
      });
    }

    byDecisionId.set(decision.id, decision);
    nodes.push({
      id: decision.id,
      node_type: 'decision',
      label: decision.choice,
      summary: decision.rationale ?? null,
      metadata: {
        provenance: decision.provenance,
        superseded: decision.superseded,
      },
      degree: 0,
      // Size by how much the record is worth believing, so a guess is
      // small wherever it appears, not only where it is drawn.
      pagerank: decision.confidence * 0.1,
      updated_at: decision.updated_at,
    });

    edges.push({
      id: `decided:${decision.id}`,
      source_id: subjectId,
      target_id: decision.id,
      relationship: REL_DECIDED,
      confidence: decision.confidence,
      source: decision.provenance === 'inferred' ? 'ai' : 'user',
    });
  }

  // Supersession, drawn only when both ends are on screen — a chain that
  // runs into a hidden guess must not render as an edge to nowhere.
  const present = new Set(nodes.map((n) => n.id));
  for (const decision of ordered) {
    if (!decision.supersedes_id || !present.has(decision.supersedes_id)) continue;
    edges.push({
      id: `supersedes:${decision.id}`,
      source_id: decision.supersedes_id,
      target_id: decision.id,
      relationship: REL_SUPERSEDES,
      confidence: decision.confidence,
      source: 'user',
    });
  }

  const degrees = new Map<string, number>();
  for (const edge of edges) {
    degrees.set(edge.source_id, (degrees.get(edge.source_id) ?? 0) + 1);
    degrees.set(edge.target_id, (degrees.get(edge.target_id) ?? 0) + 1);
  }
  for (const node of nodes) {
    node.degree = degrees.get(node.id) ?? 0;
  }

  return { nodes, edges, byDecisionId, subjectIds };
}

/** How each rung is described to the user. */
export const PROVENANCE_LABELS: Record<DecisionRecord['provenance'], string> = {
  confirmed: 'Confirmed twice',
  captured: 'You said this',
  extracted: 'Read from a transcript',
  inferred: 'Guess',
};

/**
 * How a decision's edge is drawn.
 *
 * Weight and opacity climb with standing so a guess can never look like
 * something the user stated, and `dashed` marks both guesses and reversed
 * decisions — the two kinds of line that should not be read as current
 * fact.
 */
export function decisionEdgeStyle(decision: DecisionRecord): {
  width: number;
  opacity: number;
  dashed: boolean;
  struckThrough: boolean;
} {
  const width =
    decision.provenance === 'confirmed'
      ? 3.4
      : decision.provenance === 'captured'
        ? 2.8
        : decision.provenance === 'extracted'
          ? 1.6
          : 0.7;

  const opacity =
    decision.provenance === 'confirmed' || decision.provenance === 'captured'
      ? 1
      : decision.provenance === 'extracted'
        ? 0.7
        : 0.3;

  return {
    width,
    // A reversed decision is kept but must not read as current.
    opacity: decision.superseded ? opacity * 0.45 : opacity,
    dashed: decision.superseded || decision.provenance === 'inferred',
    struckThrough: decision.superseded,
  };
}
