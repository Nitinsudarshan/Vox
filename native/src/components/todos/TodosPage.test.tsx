import React from 'react';
import { beforeEach, describe, expect, test, vi } from 'vitest';
import { render, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { invoke } from '@tauri-apps/api/core';

import { TodosPage } from './TodosPage';

import type { KanbanCard } from '@/types';

const card = (id: string, overrides: Partial<KanbanCard> = {}): KanbanCard => ({
  id,
  title: id,
  assignee: '',
  status: 'todo',
  priority: 'medium',
  created_at: '2026-01-01T00:00:00Z',
  description: '',
  ...overrides,
});

const cards: KanbanCard[] = [
  card('meeting-todo', {
    title: 'Send the pricing deck',
    para: 'projects',
    source_kind: 'meeting',
    source_ref: { id: 'meeting_7', turn_ordinal: 42, label: 'Pricing sync' },
    due_date: '2020-01-01',
  }),
  card('area-todo', {
    title: 'Review the on-call rota',
    para: 'areas',
    source_kind: 'scribble',
    source_ref: { id: 'scribble_3', label: 'Ops notes' },
  }),
  card('unfiled-todo', {
    title: 'Look into the flaky test',
    source_kind: 'voice_note',
    source_ref: { id: 'note_9' },
  }),
  card('legacy-todo', {
    title: 'Written before provenance existed',
    status: 'done',
  }),
];

const mockBackend = (list: KanbanCard[] = cards) => {
  vi.mocked(invoke).mockImplementation(async (cmd: string, args?: Record<string, unknown>) => {
    switch (cmd) {
      case 'get_kanban_cards':
        return list;
      case 'create_manual_todo':
        return card('new-todo', {
          title: String(args?.title ?? ''),
          source_kind: 'manual',
        });
      case 'set_todo_status':
        return { ...list[0], status: args?.status };
      default:
        return null;
    }
  });
};

describe('TodosPage', () => {
  beforeEach(() => mockBackend());

  test('lands on the PARA view', async () => {
    render(<TodosPage />);
    expect(await screen.findByRole('tab', { name: /By PARA/i })).toHaveAttribute(
      'aria-selected',
      'true',
    );
  });

  /**
   * The question the PARA view exists to answer and a flat list cannot.
   * Fails if the projects-vs-areas count is absent or wrong.
   */
  test('says how much open effort goes into work that finishes', async () => {
    render(<TodosPage />);
    const summary = await screen.findByText(/work that finishes/i);

    expect(summary).toHaveTextContent('1 open in Projects');
    expect(summary).toHaveTextContent('1 in Areas');
  });

  /** Fails if an empty band vanishes — that makes "nothing in Resources"
   *  indistinguishable from "no Resources band". */
  test('shows every band with its count, empty ones included', async () => {
    render(<TodosPage />);

    expect(await screen.findByRole('heading', { name: /Projects 1/i })).toBeInTheDocument();
    expect(screen.getByRole('heading', { name: /Areas 1/i })).toBeInTheDocument();
    expect(screen.getByRole('heading', { name: /Resources 0/i })).toBeInTheDocument();
    expect(screen.getByRole('heading', { name: /Archive 0/i })).toBeInTheDocument();
    // Two: the voice note whose source is unfiled, and the legacy card.
    expect(screen.getByRole('heading', { name: /Uncategorised 2/i })).toBeInTheDocument();
  });

  /**
   * Every todo shows where it came from. Fails if a card renders with no
   * provenance, or if a card with none pretends to have some.
   */
  test('shows each todo its origin, and admits when there is none', async () => {
    render(<TodosPage />);

    expect(await screen.findByText(/Meeting · Pricing sync \(turn 42\)/)).toBeInTheDocument();
    expect(screen.getByText(/Scribble · Ops notes/)).toBeInTheDocument();
    expect(screen.getByText(/Voice note · note_9/)).toBeInTheDocument();
    // Never recorded is not the same as typed by hand.
    expect(screen.getByText('Source unknown')).toBeInTheDocument();
  });

  test('links a todo back to the surface its source lives on', async () => {
    const onNavigateTab = vi.fn();
    const user = userEvent.setup();
    render(<TodosPage onNavigateTab={onNavigateTab} />);

    await user.click(await screen.findByText(/Meeting · Pricing sync/));
    expect(onNavigateTab).toHaveBeenCalledWith('meetings');
  });

  test('separates overdue work and puts it first in the date view', async () => {
    const user = userEvent.setup();
    render(<TodosPage />);

    await user.click(await screen.findByRole('tab', { name: /By date/i }));

    const overdue = await screen.findByRole('heading', { name: /Overdue 1/i });
    expect(overdue).toBeInTheDocument();
    expect(screen.getByRole('heading', { name: /Everything else 3/i })).toBeInTheDocument();
  });

  test('groups the board into the three stored statuses', async () => {
    const user = userEvent.setup();
    render(<TodosPage />);

    await user.click(await screen.findByRole('tab', { name: /Board/i }));

    expect(await screen.findByRole('heading', { name: /To do 3/i })).toBeInTheDocument();
    expect(screen.getByRole('heading', { name: /In progress 0/i })).toBeInTheDocument();
    expect(screen.getByRole('heading', { name: /Done 1/i })).toBeInTheDocument();
  });

  /** Drag needs a pointer; the board must still be operable without one. */
  test('moves a card between columns from the keyboard, and persists it', async () => {
    const user = userEvent.setup();
    render(<TodosPage />);

    await user.click(await screen.findByRole('tab', { name: /Board/i }));
    const todoColumn = screen.getByRole('region', { name: 'To do' });
    const [firstMove] = within(todoColumn).getAllByRole('button', { name: /In progress/i });
    await user.click(firstMove);

    await waitFor(() =>
      expect(vi.mocked(invoke)).toHaveBeenCalledWith('set_todo_status', {
        id: 'meeting-todo',
        status: 'in_progress',
      }),
    );
  });

  /** Typed capture: Enter creates it, no modal. */
  test('creates a todo from the typed input on Enter', async () => {
    const user = userEvent.setup();
    render(<TodosPage />);

    const input = await screen.findByLabelText(/Add a todo/i);
    await user.type(input, 'Call the bank{Enter}');

    await waitFor(() =>
      expect(vi.mocked(invoke)).toHaveBeenCalledWith('create_manual_todo', {
        title: 'Call the bank',
      }),
    );
    expect(input).toHaveValue('');
  });

  /** Fails if a blank Enter creates an empty todo. */
  test('does not create anything from an empty input', async () => {
    const user = userEvent.setup();
    render(<TodosPage />);

    const input = await screen.findByLabelText(/Add a todo/i);
    await user.type(input, '   {Enter}');

    expect(vi.mocked(invoke)).not.toHaveBeenCalledWith('create_manual_todo', expect.anything());
  });

  /**
   * A failed write must not leave the UI claiming the change landed.
   * Fails if the optimistic status sticks after the backend refuses.
   */
  test('rolls a status change back and says so when the write fails', async () => {
    mockBackend();
    vi.mocked(invoke).mockImplementation(async (cmd: string) => {
      if (cmd === 'get_kanban_cards') return cards;
      if (cmd === 'set_todo_status') throw new Error('vault is read-only');
      return null;
    });

    const user = userEvent.setup();
    render(<TodosPage />);

    const checkbox = await screen.findByLabelText(/Mark "Send the pricing deck" done/i);
    expect(checkbox).not.toBeChecked();
    await user.click(checkbox);

    expect(await screen.findByRole('alert')).toHaveTextContent(/could not be saved/i);
    await waitFor(() => expect(checkbox).not.toBeChecked());
  });

  test('says the list is empty rather than rendering nothing', async () => {
    mockBackend([]);
    render(<TodosPage />);
    expect(await screen.findByText('No todos yet')).toBeInTheDocument();
  });

  test('a failed read reports itself instead of showing a silent empty list', async () => {
    vi.mocked(invoke).mockImplementation(async () => {
      throw new Error('vault unreadable');
    });

    render(<TodosPage />);
    expect(await screen.findByRole('alert')).toHaveTextContent(/Could not read your todos/i);
  });
});
