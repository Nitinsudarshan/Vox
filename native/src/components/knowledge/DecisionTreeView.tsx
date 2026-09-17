import React, { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { AlertTriangle, BadgeCheck, Crosshair, Eye, EyeOff, Plus } from 'lucide-react';

import { Button } from '@/components/ui/button';
import type { DecisionRecord } from '@/types';

import {
  buildDecisionGraph,
  PROVENANCE_LABELS,
  subjectNodeId,
} from './graph/decisionTree';
import { layoutFocus } from './graph/focusLayout';
import { renderDecisions } from './graph/decisionRenderer';
import type { CameraState } from './graph/graphTypes';

/**
 * The Decision Tree: what was chosen, and why.
 *
 * Every decision here came from somewhere the user can go and check. The
 * one rule the whole surface is built around is that a guess must be
 * distinguishable from something the user said — in the tree, in the list,
 * and in the inspector — because an invented "because" corrupts the user's
 * own record of their project and is worse than an empty view.
 *
 * Guesses are therefore off by default, and turning them on says how many
 * you are about to see rather than silently mixing them in.
 */
export const DecisionTreeView: React.FC = () => {
  const containerRef = useRef<HTMLDivElement | null>(null);
  const canvasRef = useRef<HTMLCanvasElement | null>(null);

  const [decisions, setDecisions] = useState<DecisionRecord[]>([]);
  const [showGuesses, setShowGuesses] = useState(false);
  const [subject, setSubject] = useState<string | null>(null);
  const [selectedNodeId, setSelectedNodeId] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [camera] = useState<CameraState>({ x: 0, y: 0, k: 0.85 });

  const [draftSubject, setDraftSubject] = useState('');
  const [draftChoice, setDraftChoice] = useState('');
  const [draftWhy, setDraftWhy] = useState('');

  const refresh = useCallback(async () => {
    try {
      const loaded = await invoke<DecisionRecord[]>('list_decisions');
      setDecisions(loaded ?? []);
      setError(null);
    } catch (err) {
      console.error('Failed to read decisions:', err);
      setError('Could not read your decisions.');
    }
  }, []);

  useEffect(() => {
    refresh();
  }, [refresh]);

  const graph = useMemo(
    () => buildDecisionGraph(decisions, showGuesses),
    [decisions, showGuesses],
  );

  const subjects = useMemo(
    () => [...new Set(graph.nodes.filter((n) => n.node_type === 'topic').map((n) => n.label))],
    [graph],
  );

  const activeSubject = subject ?? subjects[0] ?? null;

  const layout = useMemo(
    () =>
      layoutFocus(
        activeSubject ? subjectNodeId(activeSubject) : null,
        graph.nodes,
        graph.edges,
        4,
      ),
    [activeSubject, graph],
  );

  const nodesById = useMemo(
    () => new Map(graph.nodes.map((n) => [n.id, n])),
    [graph.nodes],
  );

  const hiddenGuesses = useMemo(
    () => decisions.filter((d) => d.provenance === 'inferred').length,
    [decisions],
  );

  const shown = useMemo(
    () =>
      decisions.filter(
        (d) =>
          (showGuesses || d.provenance !== 'inferred') && d.subject === activeSubject,
      ),
    [decisions, showGuesses, activeSubject],
  );

  const draw = useCallback(() => {
    const canvas = canvasRef.current;
    const ctx = canvas?.getContext('2d');
    if (!canvas || !ctx) return;
    renderDecisions({
      canvas,
      ctx,
      layout,
      nodesById,
      decisions: graph.byDecisionId,
      camera,
      hoveredNodeId: null,
      selectedNodeId,
    });
  }, [layout, nodesById, graph.byDecisionId, camera, selectedNodeId]);

  useEffect(() => {
    const canvas = canvasRef.current;
    const container = containerRef.current;
    if (!canvas || !container) return;

    const resize = () => {
      const dpr = window.devicePixelRatio || 1;
      canvas.width = container.clientWidth * dpr;
      canvas.height = container.clientHeight * dpr;
      canvas.style.width = `${container.clientWidth}px`;
      canvas.style.height = `${container.clientHeight}px`;
      draw();
    };

    resize();
    const observer = new ResizeObserver(resize);
    observer.observe(container);
    return () => observer.disconnect();
  }, [draw]);

  useEffect(() => {
    draw();
  }, [draw]);

  const stateDecision = async () => {
    const s = draftSubject.trim();
    const c = draftChoice.trim();
    if (!s || !c) return;
    try {
      await invoke<DecisionRecord>('record_decision', {
        decision: {
          subject: s,
          choice: c,
          rationale: draftWhy.trim() || null,
          provenance: 'captured',
          source_id: 'decision_tree',
          source_type: 'stated',
          // The user's own words are the evidence. A captured decision
          // with nothing behind it is refused by the backend, so this is
          // not optional.
          evidence: draftWhy.trim() || c,
        },
      });
      setDraftSubject('');
      setDraftChoice('');
      setDraftWhy('');
      setSubject(s);
      await refresh();
    } catch (err) {
      console.error('Failed to record the decision:', err);
      setError('That decision could not be recorded.');
    }
  };

  const confirm = async (decision: DecisionRecord) => {
    try {
      await invoke<DecisionRecord>('confirm_decision', {
        id: decision.id,
        sourceId: 'decision_tree',
        evidence: `Confirmed: ${decision.choice}`,
      });
      await refresh();
    } catch (err) {
      console.error('Failed to confirm the decision:', err);
      setError('That confirmation could not be saved.');
    }
  };

  return (
    <div className="flex-1 flex flex-col min-h-0 min-w-0">
      <div className="flex items-center gap-2 px-3 py-2 border-b border-border/60 flex-wrap">
        <label className="flex items-center gap-1.5 text-xs text-muted-foreground">
          <Crosshair className="w-3.5 h-3.5" />
          <span>About</span>
          <select
            value={activeSubject ?? ''}
            onChange={(e) => setSubject(e.target.value || null)}
            aria-label="Decision subject"
            className="h-7 max-w-56 px-2 text-xs bg-background border border-border rounded-md text-foreground focus:outline-hidden focus:ring-1 focus:ring-ring"
          >
            {subjects.length === 0 && <option value="">Nothing decided yet</option>}
            {subjects.map((s) => (
              <option key={s} value={s}>
                {s}
              </option>
            ))}
          </select>
        </label>

        <Button
          size="sm"
          variant="outline"
          onClick={() => setShowGuesses((v) => !v)}
          aria-pressed={showGuesses}
          className="h-7"
        >
          {showGuesses ? (
            <EyeOff className="w-3.5 h-3.5" />
          ) : (
            <Eye className="w-3.5 h-3.5" />
          )}
          <span className="ml-1">
            {showGuesses ? 'Hide guesses' : `Show guesses (${hiddenGuesses})`}
          </span>
        </Button>

        <p className="text-[11px] text-muted-foreground ml-auto">
          Nothing here was invented. Every decision links to what it came from.
        </p>
      </div>

      <div className="px-3 py-2 border-b border-border/60 flex items-center gap-2 flex-wrap">
        <input
          value={draftSubject}
          onChange={(e) => setDraftSubject(e.target.value)}
          placeholder="About…"
          aria-label="What the decision is about"
          className="h-7 w-36 px-2 text-xs bg-background border border-border rounded-md text-foreground placeholder:text-muted-foreground focus:outline-hidden focus:ring-1 focus:ring-ring"
        />
        <input
          value={draftChoice}
          onChange={(e) => setDraftChoice(e.target.value)}
          placeholder="We decided to…"
          aria-label="What was chosen"
          className="h-7 flex-1 min-w-40 px-2 text-xs bg-background border border-border rounded-md text-foreground placeholder:text-muted-foreground focus:outline-hidden focus:ring-1 focus:ring-ring"
        />
        <input
          value={draftWhy}
          onChange={(e) => setDraftWhy(e.target.value)}
          placeholder="Because…"
          aria-label="Why"
          className="h-7 flex-1 min-w-40 px-2 text-xs bg-background border border-border rounded-md text-foreground placeholder:text-muted-foreground focus:outline-hidden focus:ring-1 focus:ring-ring"
        />
        <Button
          size="sm"
          variant="outline"
          onClick={stateDecision}
          disabled={!draftSubject.trim() || !draftChoice.trim()}
          className="h-7"
        >
          <Plus className="w-3.5 h-3.5" />
          <span className="ml-1">State it</span>
        </Button>
      </div>

      {error && (
        <div
          role="alert"
          className="mx-3 mt-2 flex items-center gap-2 px-3 py-2 text-xs text-destructive border border-destructive/40 bg-destructive/10 rounded-md"
        >
          <AlertTriangle className="w-3.5 h-3.5 shrink-0" />
          <span>{error}</span>
        </div>
      )}

      <div className="flex-1 min-h-0 flex">
        <div ref={containerRef} className="relative flex-1 min-h-0 overflow-hidden">
          {layout.nodes.length === 0 ? (
            <p className="absolute inset-0 flex items-center justify-center text-xs text-muted-foreground px-6 text-center">
              No decisions recorded yet. State one above, and it will appear here with what
              it rests on.
            </p>
          ) : (
            <canvas ref={canvasRef} className="absolute inset-0" />
          )}
        </div>

        {/*
          The list is not a duplicate of the canvas: it is where a decision
          can be read, checked against its evidence and confirmed. It is
          also the only part of this view a keyboard can reach.
        */}
        <aside
          aria-label="Decisions"
          className="w-80 shrink-0 border-l border-border/60 overflow-y-auto p-3 space-y-2"
        >
          {shown.length === 0 && (
            <p className="text-xs text-muted-foreground">Nothing decided about this yet.</p>
          )}
          {shown.map((decision) => (
            <article
              key={decision.id}
              onClick={() => setSelectedNodeId(decision.id)}
              className={`rounded-lg border p-2.5 space-y-1.5 ${
                decision.superseded
                  ? 'border-border/40 bg-muted/20'
                  : 'border-border/70 bg-card/60'
              }`}
            >
              <div className="flex items-start justify-between gap-2">
                <p
                  className={`text-xs ${
                    decision.superseded
                      ? 'text-muted-foreground line-through'
                      : 'text-foreground'
                  }`}
                >
                  {decision.choice}
                </p>
                <span
                  className={`text-[9px] font-mono uppercase px-1.5 py-0.5 rounded border shrink-0 ${
                    decision.provenance === 'inferred'
                      ? 'border-destructive/50 text-destructive'
                      : decision.provenance === 'extracted'
                        ? 'border-border text-muted-foreground'
                        : 'border-border text-foreground'
                  }`}
                >
                  {PROVENANCE_LABELS[decision.provenance]}
                </span>
              </div>

              {decision.rationale ? (
                <p className="text-[11px] text-muted-foreground">{decision.rationale}</p>
              ) : (
                // Never fabricate a reason. Saying none was given is the
                // honest rendering of not knowing why.
                <p className="text-[11px] text-muted-foreground/60 italic">
                  No reason was given.
                </p>
              )}

              {decision.superseded && (
                <p className="text-[10px] text-muted-foreground">
                  Replaced by a later decision. Kept for the record.
                </p>
              )}

              <ul className="space-y-0.5">
                {decision.evidence.map((item, index) => (
                  <li key={`${item.source_id}-${index}`} className="text-[10px] text-muted-foreground">
                    <span className="font-mono">{item.source_type || 'source'}</span>
                    {item.source_id ? ` · ${item.source_id}` : ''}
                    {item.evidence ? ` — “${item.evidence}”` : ''}
                  </li>
                ))}
                {decision.evidence.length === 0 && (
                  <li className="text-[10px] text-destructive">
                    No traceable source. Shown as a guess.
                  </li>
                )}
              </ul>

              {!decision.superseded && decision.provenance !== 'confirmed' && (
                <Button
                  size="sm"
                  variant="outline"
                  className="h-6 text-[10px]"
                  onClick={() => confirm(decision)}
                >
                  <BadgeCheck className="w-3 h-3" />
                  <span className="ml-1">
                    {decision.provenance === 'captured' ? 'Confirm again' : 'Confirm'}
                  </span>
                </Button>
              )}
            </article>
          ))}
        </aside>
      </div>
    </div>
  );
};
