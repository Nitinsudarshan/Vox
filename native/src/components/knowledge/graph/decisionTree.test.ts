import { describe, expect, it } from 'vitest';

import {
  buildDecisionGraph,
  decisionEdgeStyle,
  REL_DECIDED,
  REL_SUPERSEDES,
  subjectNodeId,
} from './decisionTree';
import { DECISION_CONFIDENCE, type DecisionProvenance, type DecisionRecord } from '@/types';

const decision = (
  id: string,
  provenance: DecisionProvenance,
  overrides: Partial<DecisionRecord> = {},
): DecisionRecord => ({
  id,
  subject: 'Retrieval',
  choice: `Choice ${id}`,
  rationale: null,
  provenance,
  confidence: DECISION_CONFIDENCE[provenance],
  evidence: [
    {
      source_id: 'note_1',
      source_type: 'scribble',
      evidence: 'We said so.',
      extracted_by: provenance === 'inferred' ? 'inference' : 'user',
    },
  ],
  superseded: false,
  created_at: '2026-01-01T00:00:00Z',
  updated_at: '2026-01-01T00:00:00Z',
  ...overrides,
});

describe('buildDecisionGraph', () => {
  it('hangs each decision off the subject it is about', () => {
    const graph = buildDecisionGraph([
      decision('d1', 'captured'),
      decision('d2', 'captured', { subject: 'Storage' }),
    ]);

    expect(graph.subjectIds).toEqual([subjectNodeId('Retrieval'), subjectNodeId('Storage')]);
    const edge = graph.edges.find((e) => e.target_id === 'd1')!;
    expect(edge.relationship).toBe(REL_DECIDED);
    expect(edge.source_id).toBe(subjectNodeId('Retrieval'));
  });

  /**
   * The brief's default, as a test. Fails if a guess appears without
   * being asked for — a faint wrong line is still a line, and on a tree
   * of things the user actually said it will be read as one of them.
   */
  it('hides inferred decisions unless guesses are asked for', () => {
    const decisions = [decision('real', 'captured'), decision('guess', 'inferred')];

    const hidden = buildDecisionGraph(decisions);
    expect(hidden.nodes.map((n) => n.id)).not.toContain('guess');
    expect(hidden.nodes.map((n) => n.id)).toContain('real');

    const shown = buildDecisionGraph(decisions, true);
    expect(shown.nodes.map((n) => n.id)).toContain('guess');
  });

  /** Forgetting the argument must give you the safe behaviour. */
  it('defaults to not showing guesses', () => {
    expect(buildDecisionGraph([decision('g', 'inferred')]).nodes).toHaveLength(0);
  });

  /**
   * A tree that hides what was reversed cannot say why the current answer
   * is the current answer. Fails if a superseded decision is dropped.
   */
  it('keeps superseded decisions and links them to what replaced them', () => {
    const graph = buildDecisionGraph([
      decision('old', 'captured', { superseded: true, superseded_by: 'new' }),
      decision('new', 'captured', { supersedes_id: 'old' }),
    ]);

    expect(graph.nodes.map((n) => n.id)).toContain('old');
    const link = graph.edges.find((e) => e.relationship === REL_SUPERSEDES)!;
    expect(link.source_id).toBe('old');
    expect(link.target_id).toBe('new');
  });

  /** Fails if a chain into a hidden guess leaves a dangling edge. */
  it('does not draw a supersession edge to a decision it is hiding', () => {
    const graph = buildDecisionGraph([
      decision('guess', 'inferred'),
      decision('real', 'captured', { supersedes_id: 'guess' }),
    ]);

    expect(graph.edges.filter((e) => e.relationship === REL_SUPERSEDES)).toHaveLength(0);
    expect(graph.edges.every((e) => graph.nodes.some((n) => n.id === e.source_id))).toBe(true);
  });

  it('is reproducible whatever order the decisions arrive in', () => {
    const decisions = [
      decision('d1', 'captured'),
      decision('d2', 'extracted'),
      decision('d3', 'captured', { subject: 'Storage' }),
    ];

    const a = buildDecisionGraph(decisions);
    const b = buildDecisionGraph([...decisions].reverse());

    expect(b.nodes.map((n) => n.id)).toEqual(a.nodes.map((n) => n.id));
    expect(b.edges.map((e) => e.id)).toEqual(a.edges.map((e) => e.id));
  });

  /** Node size must not make a guess look substantial. */
  it('sizes a decision by how much it is worth believing', () => {
    const graph = buildDecisionGraph(
      [decision('sure', 'confirmed'), decision('guess', 'inferred')],
      true,
    );

    const sure = graph.nodes.find((n) => n.id === 'sure')!;
    const guess = graph.nodes.find((n) => n.id === 'guess')!;
    expect(sure.pagerank!).toBeGreaterThan(guess.pagerank!);
  });
});

describe('decisionEdgeStyle', () => {
  /**
   * The ladder, as drawn. Fails if any two rungs become
   * indistinguishable — the whole point is that a guess cannot look like
   * something the user stated.
   */
  it('draws each rung more strongly than the one below it', () => {
    const confirmed = decisionEdgeStyle(decision('a', 'confirmed'));
    const captured = decisionEdgeStyle(decision('b', 'captured'));
    const extracted = decisionEdgeStyle(decision('c', 'extracted'));
    const inferred = decisionEdgeStyle(decision('d', 'inferred'));

    expect(confirmed.width).toBeGreaterThan(captured.width);
    expect(captured.width).toBeGreaterThan(extracted.width);
    expect(extracted.width).toBeGreaterThan(inferred.width);

    expect(captured.opacity).toBe(1);
    expect(extracted.opacity).toBeLessThan(1);
    expect(inferred.opacity).toBeLessThan(extracted.opacity);
  });

  it('draws a captured decision solid and a guess dashed', () => {
    expect(decisionEdgeStyle(decision('a', 'captured')).dashed).toBe(false);
    expect(decisionEdgeStyle(decision('b', 'inferred')).dashed).toBe(true);
  });

  /** A reversed decision is kept, but must not read as current. */
  it('dashes and strikes a superseded decision, and quiets it', () => {
    const current = decisionEdgeStyle(decision('a', 'captured'));
    const reversed = decisionEdgeStyle(decision('b', 'captured', { superseded: true }));

    expect(reversed.dashed).toBe(true);
    expect(reversed.struckThrough).toBe(true);
    expect(current.struckThrough).toBe(false);
    expect(reversed.opacity).toBeLessThan(current.opacity);
  });
});

describe('the confidence ladder', () => {
  /** Fails if a rung moves away from the brief's numbers. */
  it('is the one the brief specifies, and leaves the top for two confirmations', () => {
    expect(DECISION_CONFIDENCE.captured).toBe(0.9);
    expect(DECISION_CONFIDENCE.extracted).toBe(0.55);
    expect(DECISION_CONFIDENCE.inferred).toBe(0.15);
    expect(DECISION_CONFIDENCE.captured).toBeLessThan(1);
    expect(DECISION_CONFIDENCE.confirmed).toBeGreaterThan(DECISION_CONFIDENCE.captured);
  });
});
