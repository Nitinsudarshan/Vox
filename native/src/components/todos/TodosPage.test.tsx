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
  vi.mocked(invoke).mockImplementation(async (cmd: string, rawArgs?: unknown) => {
    const args = (rawArgs ?? {}) as Record<string, unknown>;
    switch (cmd) {
      case 'get_kanban_cards':
        return list;
      case 'create_manual_todo':
        return card('new-todo', {
          title: String(args.title ?? ''),
          source_kind: 'manual',
        });
      case 'set_todo_status':
        return { ...list[0], status: args.status };
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

/**
 * Voice capture. It reuses the existing recorder and STT path — the page
 * only starts and stops a capture in the `todo` mode — so what is worth
 * testing here is the part that is this page's own: that every way the
 * capture can fail is reported, and that none of them creates a todo.
 */
describe('TodosPage voice capture', () => {
  beforeEach(() => mockBackend());

  test('records in the todo capture mode rather than a second implementation', async () => {
    const user = userEvent.setup();
    render(<TodosPage />);

    await user.click(await screen.findByRole('button', { name: /Record a todo/i }));

    expect(vi.mocked(invoke)).toHaveBeenCalledWith('start_capture', { mode: 'todo' });
    expect(await screen.findByRole('button', { name: /Stop recording/i })).toBeInTheDocument();
  });

  test('stopping a recording that produced a todo refreshes the list', async () => {
    vi.mocked(invoke).mockImplementation(async (cmd: string) => {
      if (cmd === 'get_kanban_cards') return cards;
      if (cmd === 'start_capture') return 'ok';
      if (cmd === 'stop_capture') {
        return { mode: 'todo', transcript: 'Call the bank', kanban_cards_created: 1 };
      }
      return null;
    });

    const user = userEvent.setup();
    render(<TodosPage />);

    await user.click(await screen.findByRole('button', { name: /Record a todo/i }));
    await user.click(await screen.findByRole('button', { name: /Stop recording/i }));

    await waitFor(() => expect(vi.mocked(invoke)).toHaveBeenCalledWith('stop_capture'));
    expect(screen.queryByRole('alert')).not.toBeInTheDocument();
  });

  /**
   * The brief's hard requirement. Fails if a failed transcription passes
   * silently — which would leave the user believing a todo exists.
   */
  test('a failed transcription says so and creates nothing', async () => {
    vi.mocked(invoke).mockImplementation(async (cmd: string) => {
      if (cmd === 'get_kanban_cards') return cards;
      if (cmd === 'start_capture') return 'ok';
      if (cmd === 'stop_capture') throw new Error('STT_FAILED');
      return null;
    });

    const user = userEvent.setup();
    render(<TodosPage />);

    await user.click(await screen.findByRole('button', { name: /Record a todo/i }));
    await user.click(await screen.findByRole('button', { name: /Stop recording/i }));

    expect(await screen.findByRole('alert')).toHaveTextContent(
      /could not be transcribed, so no todo was created/i,
    );
    expect(vi.mocked(invoke)).not.toHaveBeenCalledWith(
      'create_manual_todo',
      expect.anything(),
    );
  });

  /** Silence is distinct from a transcription failure, and says so. */
  test('a recording that heard nothing says so and creates nothing', async () => {
    vi.mocked(invoke).mockImplementation(async (cmd: string) => {
      if (cmd === 'get_kanban_cards') return cards;
      if (cmd === 'start_capture') return 'ok';
      if (cmd === 'stop_capture') return null;
      return null;
    });

    const user = userEvent.setup();
    render(<TodosPage />);

    await user.click(await screen.findByRole('button', { name: /Record a todo/i }));
    await user.click(await screen.findByRole('button', { name: /Stop recording/i }));

    expect(await screen.findByRole('alert')).toHaveTextContent(/Nothing was heard/i);
  });

  /** Audio that yielded no usable words is a third, distinct outcome. */
  test('a recording that yielded no usable text says so and creates nothing', async () => {
    vi.mocked(invoke).mockImplementation(async (cmd: string) => {
      if (cmd === 'get_kanban_cards') return cards;
      if (cmd === 'start_capture') return 'ok';
      if (cmd === 'stop_capture') {
        return { mode: 'todo', transcript: '', kanban_cards_created: 0 };
      }
      return null;
    });

    const user = userEvent.setup();
    render(<TodosPage />);

    await user.click(await screen.findByRole('button', { name: /Record a todo/i }));
    await user.click(await screen.findByRole('button', { name: /Stop recording/i }));

    expect(await screen.findByRole('alert')).toHaveTextContent(/no usable text/i);
  });

  /** The recorder is shared with dictation and meetings; a refusal to
   *  start must be visible rather than looking like a dead button. */
  test('reports a recorder that will not start', async () => {
    vi.mocked(invoke).mockImplementation(async (cmd: string) => {
      if (cmd === 'get_kanban_cards') return cards;
      if (cmd === 'start_capture') throw new Error('DICTATION_HOTKEY_ACTIVE');
      return null;
    });

    const user = userEvent.setup();
    render(<TodosPage />);

    await user.click(await screen.findByRole('button', { name: /Record a todo/i }));

    expect(await screen.findByRole('alert')).toHaveTextContent(/Could not start recording/i);
    expect(screen.getByRole('button', { name: /Record a todo/i })).toBeInTheDocument();
  });
});
