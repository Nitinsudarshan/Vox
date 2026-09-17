import React from 'react';
import { beforeEach, describe, expect, test, vi } from 'vitest';
import { render, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { invoke } from '@tauri-apps/api/core';

import { DecisionTreeView } from './DecisionTreeView';

import type { DecisionRecord } from '@/types';

const decision = (
  id: string,
  provenance: DecisionRecord['provenance'],
  overrides: Partial<DecisionRecord> = {},
): DecisionRecord => ({
  id,
  subject: 'Retrieval',
  choice: `Choice ${id}`,
  rationale: 'Because it runs embedded.',
  provenance,
  confidence: 0.9,
  evidence: [
    {
      source_id: 'note_1',
      source_type: 'scribble',
      evidence: 'We are going with LanceDB.',
      extracted_by: 'user',
    },
  ],
  superseded: false,
  created_at: '2026-01-01T00:00:00Z',
  updated_at: '2026-01-01T00:00:00Z',
  ...overrides,
});

const records: DecisionRecord[] = [
  decision('captured-1', 'captured', { choice: 'Use LanceDB' }),
  decision('extracted-1', 'extracted', { choice: 'Chunk at 512 tokens' }),
  decision('inferred-1', 'inferred', {
    choice: 'Maybe switch to SQLite',
    evidence: [],
    rationale: null,
  }),
];

const mockBackend = (list: DecisionRecord[] = records) => {
  vi.mocked(invoke).mockImplementation(async (cmd: string) => {
    if (cmd === 'list_decisions') return list;
    if (cmd === 'record_decision') return decision('new', 'captured');
    if (cmd === 'confirm_decision') return decision('captured-1', 'confirmed');
    return null;
  });
};

describe('DecisionTreeView', () => {
  beforeEach(() => mockBackend());

  /**
   * The brief's default, at the surface. Fails if a guess is on screen
   * without being asked for.
   */
  test('hides guesses by default and says how many are hidden', async () => {
    render(<DecisionTreeView />);

    expect(await screen.findByText('Use LanceDB')).toBeInTheDocument();
    expect(screen.getByText('Chunk at 512 tokens')).toBeInTheDocument();
    expect(screen.queryByText('Maybe switch to SQLite')).not.toBeInTheDocument();

    const toggle = screen.getByRole('button', { name: /Show guesses \(1\)/i });
    expect(toggle).toHaveAttribute('aria-pressed', 'false');
  });

  /**
   * And when they are shown, they must be labelled as guesses everywhere.
   * Fails if a guess renders indistinguishably from something stated.
   */
  test('marks a guess as a guess once it is shown', async () => {
    const user = userEvent.setup();
    render(<DecisionTreeView />);

    await user.click(await screen.findByRole('button', { name: /Show guesses/i }));

    const guess = (await screen.findByText('Maybe switch to SQLite')).closest('article')!;
    expect(within(guess).getByText('Guess')).toBeInTheDocument();
    // And it says outright that there is nothing behind it.
    expect(within(guess).getByText(/No traceable source/i)).toBeInTheDocument();
  });

  /** Every decision links back to what it came from. */
  test('shows the evidence a decision rests on', async () => {
    render(<DecisionTreeView />);

    const captured = (await screen.findByText('Use LanceDB')).closest('article')!;
    // The source type sits in its own <span>, so match on the whole line.
    const [evidenceLine] = within(captured).getAllByRole('listitem');
    expect(evidenceLine).toHaveTextContent('scribble · note_1');
    expect(evidenceLine).toHaveTextContent('We are going with LanceDB');
    expect(within(captured).getByText('You said this')).toBeInTheDocument();
  });

  /**
   * Never fabricate a reason. Fails if a decision with no rationale gets
   * a plausible-sounding one rather than an admission.
   */
  test('says no reason was given rather than inventing one', async () => {
    mockBackend([decision('d', 'captured', { rationale: null })]);
    render(<DecisionTreeView />);

    expect(await screen.findByText(/No reason was given/i)).toBeInTheDocument();
  });

  /** Promotion is a real write, not a UI state. */
  test('confirming an extracted decision writes the confirmation', async () => {
    const user = userEvent.setup();
    render(<DecisionTreeView />);

    const extracted = (await screen.findByText('Chunk at 512 tokens')).closest('article')!;
    await user.click(within(extracted).getByRole('button', { name: /^Confirm$/i }));

    await waitFor(() =>
      expect(vi.mocked(invoke)).toHaveBeenCalledWith('confirm_decision', {
        id: 'extracted-1',
        sourceId: 'decision_tree',
        evidence: 'Confirmed: Chunk at 512 tokens',
      }),
    );
  });

  /** A decision already at the top rung has nothing left to confirm. */
  test('offers no confirmation on an already-confirmed decision', async () => {
    mockBackend([decision('d', 'confirmed')]);
    render(<DecisionTreeView />);

    await screen.findByText('Confirmed twice');
    expect(screen.queryByRole('button', { name: /Confirm/i })).not.toBeInTheDocument();
  });

  /**
   * A reversed decision is kept, struck through, and said to have been
   * replaced. Fails if it vanishes.
   */
  test('keeps a superseded decision and marks it replaced', async () => {
    mockBackend([
      decision('old', 'captured', {
        choice: 'Use LanceDB',
        superseded: true,
        superseded_by: 'new',
      }),
      decision('new', 'captured', { choice: 'Use SQLite FTS', supersedes_id: 'old' }),
    ]);
    render(<DecisionTreeView />);

    const old = (await screen.findByText('Use LanceDB')).closest('article')!;
    expect(within(old).getByText(/Replaced by a later decision/i)).toBeInTheDocument();
    expect(screen.getByText('Use SQLite FTS')).toBeInTheDocument();
  });

  /** Explicit capture is the path the project chose; it must reach the
   *  backend with a source, since a captured decision needs one. */
  test('states a decision with the user own words as its evidence', async () => {
    const user = userEvent.setup();
    render(<DecisionTreeView />);

    await user.type(await screen.findByLabelText(/What the decision is about/i), 'Storage');
    await user.type(screen.getByLabelText(/What was chosen/i), 'Keep everything local');
    await user.type(screen.getByLabelText(/^Why$/i), 'No cloud dependency');
    await user.click(screen.getByRole('button', { name: /State it/i }));

    await waitFor(() =>
      expect(vi.mocked(invoke)).toHaveBeenCalledWith('record_decision', {
        decision: {
          subject: 'Storage',
          choice: 'Keep everything local',
          rationale: 'No cloud dependency',
          provenance: 'captured',
          source_id: 'decision_tree',
          source_type: 'stated',
          evidence: 'No cloud dependency',
        },
      }),
    );
  });

  test('will not state a decision with no choice in it', async () => {
    const user = userEvent.setup();
    render(<DecisionTreeView />);

    await user.type(await screen.findByLabelText(/What the decision is about/i), 'Storage');
    expect(screen.getByRole('button', { name: /State it/i })).toBeDisabled();
  });

  test('says so plainly when nothing has been decided', async () => {
    mockBackend([]);
    render(<DecisionTreeView />);

    expect(await screen.findByText(/No decisions recorded yet/i)).toBeInTheDocument();
  });

  test('a failed read reports itself', async () => {
    vi.mocked(invoke).mockImplementation(async () => {
      throw new Error('memory index unreadable');
    });
    render(<DecisionTreeView />);

    expect(await screen.findByRole('alert')).toHaveTextContent(/Could not read your decisions/i);
  });
});
