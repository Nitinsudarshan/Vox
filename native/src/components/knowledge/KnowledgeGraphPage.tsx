import React, { useCallback, useEffect, useMemo, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { Network, RefreshCw } from 'lucide-react';

import { Badge } from '@/components/ui/badge';
import { Button } from '@/components/ui/button';
import { EmptyState } from '@/components/common/EmptyState';
import { PageHeader } from '@/components/common/PageHeader';

import { KnowledgeGraphView } from './KnowledgeGraphView';
import { GraphModeSwitcher } from './GraphModeSwitcher';
import { RingsView } from './RingsView';
import { loadGraphViewMode, saveGraphViewMode } from './graph/graphStorage';
import type { GraphViewMode } from './graph/graphTypes';
import type { RingBand } from './graph/ringsLayout';

import type { KnowledgeGraphData, KnowledgeTelemetrySnapshot, Scribble } from '@/types';

interface KnowledgeGraphPageProps {
  /**
   * Reveals a scribble in the Scribbles workspace. The graph itself owns
   * connecting, merging and trashing; editing a thought's prose belongs to the
   * surface built for it, so opening the editor is a navigation, not a mode.
   */
  onOpenScribble?: (id: string) => void;
}

/**
 * The Knowledge Graph as its own surface.
 *
 * It reads the same two commands the Scribbles workspace does — `get_scribbles`
 * for the thoughts a node can act on, `get_knowledge_graph` for the topology —
 * because the graph spans more than scribbles (topics, entities, sources,
 * meetings, documents) and is no longer a sub-tab of one of its inputs.
 */
export const KnowledgeGraphPage: React.FC<KnowledgeGraphPageProps> = ({ onOpenScribble }) => {
  const [graphData, setGraphData] = useState<KnowledgeGraphData>({ nodes: [], edges: [] });
  const [scribbles, setScribbles] = useState<Scribble[]>([]);
  const [telemetry, setTelemetry] = useState<KnowledgeTelemetrySnapshot | null>(null);
  const [loading, setLoading] = useState(true);
  const [refreshing, setRefreshing] = useState(false);
  const [viewMode, setViewMode] = useState<GraphViewMode>(loadGraphViewMode);

  const refreshData = useCallback(async () => {
    try {
      const [loadedGraph, loadedScribbles, loadedTelemetry] = await Promise.all([
        invoke<KnowledgeGraphData>('get_knowledge_graph', { filter: null }),
        invoke<Scribble[]>('get_scribbles'),
        invoke<KnowledgeTelemetrySnapshot>('get_knowledge_telemetry').catch(() => null),
      ]);
      setGraphData(loadedGraph ?? { nodes: [], edges: [] });
      setScribbles(loadedScribbles ?? []);
      setTelemetry(loadedTelemetry);
    } catch (err) {
      console.error('Failed to load the knowledge graph:', err);
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    refreshData();
  }, [refreshData]);

  // Anything that changes a scribble changes the graph it sits in.
  useEffect(() => {
    const unlistenSaved = listen<Scribble>('scribble-saved', () => refreshData());
    const unlistenEnriched = listen<Scribble>('scribble-enriched', () => refreshData());

    return () => {
      unlistenSaved.then((u) => u());
      unlistenEnriched.then((u) => u());
    };
  }, [refreshData]);

  /**
   * Switching modes changes which view renders the graph already in state.
   * It deliberately does not refetch: the data is the same data, and a
   * refetch would make a view switch cost a vault read.
   */
  const handleModeChange = (mode: GraphViewMode) => {
    setViewMode(mode);
    saveGraphViewMode(mode);
  };

  /** Files a scribble under a PARA band — what Rings reads position from. */
  const handleSetPara = useCallback(
    async (scribbleId: string, band: RingBand | null) => {
      try {
        await invoke('set_scribble_para', {
          id: scribbleId,
          para: band === null || band === 'uncategorised' ? null : band,
        });
        await refreshData();
      } catch (err) {
        console.error('Failed to file the scribble under a PARA band:', err);
      }
    },
    [refreshData],
  );

  const handleManualRefresh = async () => {
    setRefreshing(true);
    await refreshData();
    setRefreshing(false);
  };

  const handleScribbleUpdated = (updated: Scribble) => {
    setScribbles((prev) => prev.map((s) => (s.id === updated.id ? updated : s)));
    refreshData();
  };

  const handleScribbleCreated = (created: Scribble) => {
    setScribbles((prev) => [created, ...prev.filter((s) => s.id !== created.id)]);
    refreshData();
  };

  const handleScribbleDeleted = async (id: string) => {
    try {
      await invoke('delete_scribble', { id });
      setScribbles((prev) => prev.filter((s) => s.id !== id));
      refreshData();
    } catch (err) {
      console.error('Failed to move scribble to trash:', err);
    }
  };

  const counts = useMemo(
    () => ({
      nodes: graphData.nodes.length,
      edges: graphData.edges.length,
      orphans: graphData.nodes.filter((n) => n.degree === 0).length,
    }),
    [graphData],
  );

  return (
    <div className="flex-1 flex flex-col min-h-0 min-w-0 overflow-hidden">
      <PageHeader
        title="How everything"
        highlightText="connects."
        description="Scribbles, topics, entities and sources as an interactive knowledge graph."
        glowColor="primary"
        compact
      >
        <div className="flex items-center gap-3 flex-wrap sm:flex-nowrap shrink-0">
          <GraphModeSwitcher mode={viewMode} onChange={handleModeChange} />

          <div className="flex items-center divide-x divide-border/60 bg-background/60 backdrop-blur-xs border border-border/80 rounded-lg py-1 px-1 shadow-2xs">
            <span className="sr-only">{counts.edges} link{counts.edges === 1 ? '' : 's'}</span>
            <span className="sr-only">{counts.orphans} unconnected</span>
            {telemetry && (
              <span className="sr-only">
                {telemetry.total_entities} entit{telemetry.total_entities === 1 ? 'y' : 'ies'} ·{' '}
                {telemetry.total_relationships} relationship
                {telemetry.total_relationships === 1 ? '' : 's'} · {telemetry.active_memories} memor
                {telemetry.active_memories === 1 ? 'y' : 'ies'}
              </span>
            )}
            <div className="px-2.5 py-0.5 text-center" aria-hidden="true">
              <p className="text-[9px] font-mono uppercase tracking-widest text-muted-foreground">
                Links
              </p>
              <p className="text-sm font-extrabold text-foreground font-mono">
                {counts.edges}
              </p>
            </div>
            <div className="px-2.5 py-0.5 text-center" aria-hidden="true">
              <p className="text-[9px] font-mono uppercase tracking-widest text-muted-foreground">
                Unconnected
              </p>
              <p className="text-sm font-extrabold text-foreground font-mono">
                {counts.orphans}
              </p>
            </div>
            {telemetry && (
              <>
                <div className="px-2.5 py-0.5 text-center" aria-hidden="true">
                  <p className="text-[9px] font-mono uppercase tracking-widest text-muted-foreground">
                    Entities
                  </p>
                  <p className="text-sm font-extrabold text-foreground font-mono">
                    {telemetry.total_entities}
                  </p>
                </div>
                <div className="px-2.5 py-0.5 text-center" aria-hidden="true">
                  <p className="text-[9px] font-mono uppercase tracking-widest text-muted-foreground">
                    Relations
                  </p>
                  <p className="text-sm font-extrabold text-foreground font-mono">
                    {telemetry.total_relationships}
                  </p>
                </div>
                <div className="px-2.5 py-0.5 text-center" aria-hidden="true">
                  <p className="text-[9px] font-mono uppercase tracking-widest text-muted-foreground">
                    Memory
                  </p>
                  <p className="text-sm font-extrabold text-foreground font-mono">
                    {telemetry.active_memories}
                  </p>
                </div>
              </>
            )}
          </div>

          <Button
            size="sm"
            variant="outline"
            onClick={handleManualRefresh}
            disabled={refreshing}
            className="h-8 px-3 text-xs gap-1.5 shrink-0 border-border/80 bg-background/60 backdrop-blur-xs hover:bg-accent hover:text-accent-foreground text-foreground shadow-2xs transition-all active:scale-[0.98]"
          >
            <RefreshCw className={`w-3.5 h-3.5 ${refreshing ? 'animate-spin' : ''}`} />
            <span>Rebuild graph</span>
          </Button>
        </div>
      </PageHeader>

      {!loading && counts.nodes === 0 ? (
        <EmptyState
          icon={Network}
          title="Nothing to connect yet"
          description="The graph is built from Scribbles, their topics and the entities Vox resolves out of them. Capture a thought, promote a Voice Note or import a document and it will appear here."
          minHeight="min-h-[260px]"
        />
      ) : (
        <div className="flex-1 flex min-h-0">
          {viewMode === 'rings' ? (
            <RingsView
              graphData={graphData}
              onOpenScribbleEditor={onOpenScribble}
              onSetPara={handleSetPara}
            />
          ) : (
            <KnowledgeGraphView
              graphData={graphData}
              allScribbles={scribbles}
              isLoading={loading}
              onOpenScribbleEditor={onOpenScribble}
              onScribbleUpdated={handleScribbleUpdated}
              onScribbleCreated={handleScribbleCreated}
              onScribbleDeleted={handleScribbleDeleted}
            />
          )}
        </div>
      )}
    </div>
  );
};
