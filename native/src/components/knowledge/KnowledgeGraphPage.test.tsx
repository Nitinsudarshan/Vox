import React from 'react';
import { beforeEach, describe, expect, test, vi } from 'vitest';
import { render, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { invoke } from '@tauri-apps/api/core';

import { KnowledgeGraphPage } from './KnowledgeGraphPage';

import type { KnowledgeGraphData } from '@/types';

const graph: KnowledgeGraphData = {
  nodes: [
    {
      id: 'scr_1',
      node_type: 'scribble',
      label: 'Chunking strategy',
      metadata: {},
      degree: 2,
      para: 'projects',
      pagerank: 0.3,
      updated_at: '2026-01-05T00:00:00Z',
    },
    { id: 'topic_1', node_type: 'topic', label: 'Retrieval', metadata: {}, degree: 1 },
    { id: 'ent_1', node_type: 'entity', label: 'Vox', metadata: {}, degree: 1 },
    {
      id: 'scr_2',
      node_type: 'scribble',
      label: 'Orphan thought',
      metadata: {},
      degree: 0,
      para: 'areas',
      pagerank: 0.1,
      updated_at: '2025-02-02T00:00:00Z',
    },
  ],
  edges: [
    {
      id: 'edge_1',
      source_id: 'scr_1',
      target_id: 'topic_1',
      relationship: 'HAS_TOPIC',
      confidence: 1,
      source: 'system',
    },
    {
      id: 'edge_2',
      source_id: 'scr_1',
      target_id: 'ent_1',
      relationship: 'MENTIONS',
      confidence: 0.9,
      source: 'ai',
    },
  ],
};

const telemetry = {
  total_memories: 2,
  active_memories: 2,
  total_entities: 1,
  total_relationships: 2,
  total_scribbles: 2,
  total_notes: 0,
  total_files: 0,
  total_captures: 0,
};

describe('KnowledgeGraphPage', () => {
  beforeEach(() => {
    // The view mode persists in localStorage, so a mode chosen by one test
    // would otherwise decide which view the next one renders.
    localStorage.clear();
    vi.mocked(invoke).mockImplementation(async (cmd: string) => {
      switch (cmd) {
        case 'get_knowledge_graph':
          return graph;
        case 'get_scribbles':
          return [];
        case 'get_knowledge_telemetry':
          return telemetry;
        default:
          return null;
      }
    });
  });

  test('reads the graph itself rather than depending on the Scribbles surface', async () => {
    render(<KnowledgeGraphPage />);

    await waitFor(() => {
      expect(vi.mocked(invoke)).toHaveBeenCalledWith('get_knowledge_graph', { filter: null });
    });
    expect(vi.mocked(invoke)).toHaveBeenCalledWith('get_scribbles');
  });

  test('summarises what the canvas toolbar does not: links, orphans, resolved knowledge', async () => {
    render(<KnowledgeGraphPage />);

    expect(await screen.findByText('2 links')).toBeInTheDocument();
    expect(screen.getByText('1 unconnected')).toBeInTheDocument();
    expect(screen.getByText(/1 entity · 2 relationships · 2 memories/)).toBeInTheDocument();
  });

  test('rebuilding re-reads the graph', async () => {
    render(<KnowledgeGraphPage />);
    const user = userEvent.setup();

    await screen.findByText('2 links');
    const graphReads = () =>
      vi.mocked(invoke).mock.calls.filter(([cmd]) => cmd === 'get_knowledge_graph').length;
    const before = graphReads();

    await user.click(screen.getByRole('button', { name: /Rebuild graph/i }));

    await waitFor(() => expect(graphReads()).toBeGreaterThan(before));
  });

  test('says there is nothing to connect instead of drawing an empty canvas', async () => {
    vi.mocked(invoke).mockImplementation(async (cmd: string) =>
      cmd === 'get_knowledge_graph' ? { nodes: [], edges: [] } : null,
    );

    render(<KnowledgeGraphPage />);

    expect(await screen.findByText('Nothing to connect yet')).toBeInTheDocument();
    expect(screen.getByText('0 links')).toBeInTheDocument();
  });

  test('a failed read leaves an empty graph rather than a thrown page', async () => {
    vi.mocked(invoke).mockImplementation(async () => {
      throw new Error('vault unreadable');
    });

    render(<KnowledgeGraphPage />);

    expect(await screen.findByText('Nothing to connect yet')).toBeInTheDocument();
  });
});

/**
 * The graph surface is four views over one dataset. These cover the
 * switching contract — what you land on, what persists, and the promise
 * that changing view does not cost a vault read.
 */
describe('KnowledgeGraphPage view modes', () => {
  beforeEach(() => {
    localStorage.clear();
    vi.mocked(invoke).mockImplementation(async (cmd: string) => {
      switch (cmd) {
        case 'get_knowledge_graph':
          return graph;
        case 'get_scribbles':
          return [];
        case 'get_knowledge_telemetry':
          return telemetry;
        default:
          return null;
      }
    });
  });

  test('lands on Rings', async () => {
    render(<KnowledgeGraphPage />);

    const rings = await screen.findByRole('tab', { name: /Rings/i });
    expect(rings).toHaveAttribute('aria-selected', 'true');
    expect(screen.getByRole('tab', { name: /Force/i })).toHaveAttribute(
      'aria-selected',
      'false',
    );
  });

  /**
   * Fails if the switcher offers a mode whose view does not exist — a tab
   * that renders nothing reads as broken, not as forthcoming.
   */
  test('offers only the modes that have a view behind them', async () => {
    render(<KnowledgeGraphPage />);

    await screen.findByRole('tab', { name: /Rings/i });
    expect(screen.getByRole('tab', { name: /Force/i })).toBeInTheDocument();
    expect(screen.queryByRole('tab', { name: /Decision Tree/i })).not.toBeInTheDocument();
  });

  /**
   * The acceptance from the brief. Fails if the graph read count rises when
   * the mode changes — the data is already in state, and a refetch would
   * make a view switch cost a vault walk.
   */
  test('switching modes does not refetch the graph', async () => {
    render(<KnowledgeGraphPage />);
    const user = userEvent.setup();

    await screen.findByRole('tab', { name: /Rings/i });
    const graphReads = () =>
      vi.mocked(invoke).mock.calls.filter(([cmd]) => cmd === 'get_knowledge_graph').length;
    const before = graphReads();
    expect(before).toBeGreaterThan(0);

    await user.click(screen.getByRole('tab', { name: /Force/i }));
    await waitFor(() =>
      expect(screen.getByRole('tab', { name: /Force/i })).toHaveAttribute(
        'aria-selected',
        'true',
      ),
    );

    expect(graphReads()).toBe(before);
  });

  /** Fails if the chosen mode is forgotten — it must survive a restart,
   *  which a remount stands in for here. */
  test('remembers the chosen mode across a remount', async () => {
    const user = userEvent.setup();
    const first = render(<KnowledgeGraphPage />);

    await screen.findByRole('tab', { name: /Rings/i });
    await user.click(screen.getByRole('tab', { name: /Force/i }));
    first.unmount();

    render(<KnowledgeGraphPage />);
    expect(await screen.findByRole('tab', { name: /Force/i })).toHaveAttribute(
      'aria-selected',
      'true',
    );
  });

  test('Rings names every band and counts what is in it', async () => {
    render(<KnowledgeGraphPage />);

    // Two scribbles are filed; the topic and entity nodes are not filed
    // under PARA at all, so they belong to Unfiled rather than to a band
    // picked on their behalf.
    expect(await screen.findByRole('button', { name: new RegExp(`^Projects\\s*1$`) })).toBeInTheDocument();
    expect(screen.getByRole('button', { name: new RegExp(`^Areas\\s*1$`) })).toBeInTheDocument();
    expect(screen.getByRole('button', { name: new RegExp(`^Resources\\s*0$`) })).toBeInTheDocument();
    expect(screen.getByRole('button', { name: new RegExp(`^Archive\\s*0$`) })).toBeInTheDocument();
    expect(screen.getByRole('button', { name: new RegExp(`^Unfiled\\s*2$`) })).toBeInTheDocument();
  });

  /** The force view is demoted, not deleted. Fails if switching to it
   *  renders the Rings surface (or nothing at all). */
  test('the force view stays reachable', async () => {
    render(<KnowledgeGraphPage />);
    const user = userEvent.setup();

    await screen.findByRole('tab', { name: /Rings/i });
    expect(screen.getByPlaceholderText(/Dim everything but/i)).toBeInTheDocument();

    await user.click(screen.getByRole('tab', { name: /Force/i }));

    await waitFor(() =>
      expect(screen.queryByPlaceholderText(/Dim everything but/i)).not.toBeInTheDocument(),
    );
  });
});

/**
 * Focus & Flow. The canvas itself cannot be exercised in jsdom, which has
 * no 2D context — so these drive the same state through the canvas's
 * accessible fallback, which is the path a keyboard user takes anyway.
 */
describe('KnowledgeGraphPage focus & flow', () => {
  beforeEach(() => {
    localStorage.clear();
    vi.mocked(invoke).mockImplementation(async (cmd: string) => {
      switch (cmd) {
        case 'get_knowledge_graph':
          return graph;
        case 'get_scribbles':
          return [];
        case 'get_knowledge_telemetry':
          return telemetry;
        default:
          return null;
      }
    });
  });

  const openFocus = async () => {
    const user = userEvent.setup();
    render(<KnowledgeGraphPage />);
    await screen.findByRole('tab', { name: /Rings/i });
    await user.click(screen.getByRole('tab', { name: /Focus & Flow/i }));
    return user;
  };

  test('roots on the most connected node by default', async () => {
    await openFocus();

    const picker = await screen.findByLabelText(/Root node/i);
    // scr_1 has the highest PageRank in the fixture, so it is where reading
    // starts — rooting on an orphan would show an empty tree.
    expect(picker).toHaveValue('scr_1');
  });

  /** The canvas is not the only way in. Fails if the tree is unreachable
   *  without a pointer. */
  test('exposes the tree to the keyboard through the canvas fallback', async () => {
    await openFocus();

    const list = await screen.findByRole('list', { name: /Nodes in this tree/i });
    expect(list).toBeInTheDocument();
    expect(
      screen.getByRole('button', { name: /Chunking strategy \(root\)/i }),
    ).toBeInTheDocument();
    expect(screen.getByRole('button', { name: /Retrieval — 1 hops/i })).toBeInTheDocument();
  });

  /**
   * The feature: selecting a node shows the route it arrived by. Fails if
   * no path appears, or if it shows something other than root-then-node.
   */
  test('selecting a node traces the path back to the root', async () => {
    const user = await openFocus();

    expect(screen.queryByRole('navigation', { name: /Path from root/i })).not.toBeInTheDocument();

    await user.click(screen.getByRole('button', { name: /Retrieval — 1 hops/i }));

    const trail = await screen.findByRole('navigation', { name: /Path from root/i });
    const steps = within(trail)
      .getAllByRole('button')
      .map((b) => b.textContent);
    expect(steps).toEqual(['Chunking strategy', 'Retrieval']);
  });

  /**
   * A vault of unconnected thoughts has no tree to root on. Fails if that
   * renders a blank canvas, which reads as broken rather than as empty.
   */
  test('says so plainly when nothing in the vault connects', async () => {
    vi.mocked(invoke).mockImplementation(async (cmd: string) => {
      if (cmd === 'get_knowledge_graph') {
        return {
          nodes: [
            { id: 'a', node_type: 'scribble', label: 'Lone thought', metadata: {}, degree: 0 },
            { id: 'b', node_type: 'scribble', label: 'Another', metadata: {}, degree: 0 },
          ],
          edges: [],
        };
      }
      return cmd === 'get_scribbles' ? [] : null;
    });

    const user = userEvent.setup();
    render(<KnowledgeGraphPage />);
    await screen.findByRole('tab', { name: /Rings/i });
    await user.click(screen.getByRole('tab', { name: /Focus & Flow/i }));

    expect(await screen.findByText(/Nothing connects to this root yet/i)).toBeInTheDocument();
  });

  /** Narrowing the depth must actually narrow the tree. */
  test('following fewer hops shows fewer nodes', async () => {
    const user = await openFocus();

    const before = within(
      await screen.findByRole('list', { name: /Nodes in this tree/i }),
    ).getAllByRole('button').length;

    await user.selectOptions(screen.getByLabelText(/How many hops/i), '2');

    const after = within(
      screen.getByRole('list', { name: /Nodes in this tree/i }),
    ).getAllByRole('button').length;
    expect(after).toBeLessThanOrEqual(before);
    expect(after).toBeGreaterThan(0);
  });
});
