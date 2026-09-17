import { describe, expect, it } from 'vitest';

import {
  bandOfTodo,
  byDate,
  effortSplit,
  groupByBand,
  isOverdue,
  orderByDate,
  sourceTabFor,
  TODO_BANDS,
} from './todoGrouping';
import type { KanbanCard, ParaBand } from '@/types';

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

describe('PARA segmentation', () => {
  it('files a todo under the band it inherited', () => {
    expect(bandOfTodo(card('a', { para: 'projects' }))).toBe('projects');
    expect(bandOfTodo(card('b', { para: 'archive' }))).toBe('archive');
  });

  /**
   * The brief's rule, as a test: an unfiled source means uncategorised, an
   * explicit state, not a band chosen on the todo's behalf.
   */
  it('puts a todo with no inherited band in Uncategorised rather than guessing', () => {
    expect(bandOfTodo(card('a'))).toBe('uncategorised');
    expect(bandOfTodo(card('b', { para: null }))).toBe('uncategorised');
    expect(bandOfTodo(card('c', { para: 'someday' as ParaBand }))).toBe('uncategorised');
  });

  /**
   * Fails if an empty band disappears — "nothing in Projects" and "no
   * Projects band" would then look identical, and the first is the finding.
   */
  it('returns every band, including the empty ones', () => {
    const groups = groupByBand([card('a', { para: 'projects' })]);

    expect(groups.map((g) => g.band)).toEqual(TODO_BANDS);
    expect(groups.find((g) => g.band === 'projects')!.cards).toHaveLength(1);
    expect(groups.find((g) => g.band === 'areas')!.cards).toHaveLength(0);
  });

  /**
   * The question a flat list cannot answer. Fails if done work is counted
   * as effort — finished work is not effort currently going anywhere.
   */
  it('counts open work that finishes against open work that only continues', () => {
    const split = effortSplit([
      card('p1', { para: 'projects' }),
      card('p2', { para: 'projects' }),
      card('p3', { para: 'projects', status: 'done' }),
      card('a1', { para: 'areas' }),
      card('r1', { para: 'resources' }),
      card('u1'),
    ]);

    expect(split).toEqual({ projects: 2, areas: 1 });
  });
});

describe('date ordering', () => {
  it('sorts by due date, then by when it was captured', () => {
    const ordered = byDate([
      card('late', { due_date: '2026-06-01' }),
      card('early', { due_date: '2026-01-01' }),
      card('sameDayLater', { due_date: '2026-01-01', captured_at: '2026-01-01T12:00:00Z' }),
      card('sameDayEarlier', { due_date: '2026-01-01', captured_at: '2026-01-01T08:00:00Z' }),
    ]);

    // All three 2026-01-01 cards come before 'late', and within that day
    // the tiebreak is capture time: 'early' has no captured_at and so
    // falls back to created_at at 00:00Z, ahead of 08:00 and 12:00.
    expect(ordered.map((c) => c.id)).toEqual([
      'early',
      'sameDayEarlier',
      'sameDayLater',
      'late',
    ]);
  });

  /** Fails if undated work floats to the top and buries the dated work. */
  it('sorts undated work last rather than first', () => {
    const ordered = byDate([card('undated'), card('dated', { due_date: '2030-01-01' })]);
    expect(ordered.map((c) => c.id)).toEqual(['dated', 'undated']);
  });

  it('falls back to created_at when a card predates captured_at', () => {
    const ordered = byDate([
      card('newer', { created_at: '2026-05-01T00:00:00Z' }),
      card('older', { created_at: '2026-01-01T00:00:00Z' }),
    ]);
    expect(ordered.map((c) => c.id)).toEqual(['older', 'newer']);
  });

  const now = new Date('2026-06-15T12:00:00Z');

  it('treats work due today as not yet overdue', () => {
    expect(isOverdue(card('a', { due_date: '2026-06-15' }), now)).toBe(false);
    expect(isOverdue(card('b', { due_date: '2026-06-14' }), now)).toBe(true);
    expect(isOverdue(card('c', { due_date: '2026-06-16' }), now)).toBe(false);
  });

  /** Fails if finished work keeps nagging — a done todo is not overdue. */
  it('never calls done work overdue', () => {
    expect(isOverdue(card('a', { due_date: '2020-01-01', status: 'done' }), now)).toBe(false);
  });

  it('ignores a due date it cannot read rather than calling it overdue', () => {
    expect(isOverdue(card('a', { due_date: 'next tuesday' }), now)).toBe(false);
    expect(isOverdue(card('b'), now)).toBe(false);
  });

  it('separates overdue work and puts it first', () => {
    const { overdue, rest } = orderByDate(
      [
        card('soon', { due_date: '2026-07-01' }),
        card('late', { due_date: '2026-01-01' }),
        card('undated'),
      ],
      now,
    );

    expect(overdue.map((c) => c.id)).toEqual(['late']);
    expect(rest.map((c) => c.id)).toEqual(['soon', 'undated']);
  });
});

describe('provenance', () => {
  it('routes a todo back to the surface its source lives on', () => {
    expect(sourceTabFor('meeting')).toBe('meetings');
    expect(sourceTabFor('voice_note')).toBe('capture');
    expect(sourceTabFor('scribble')).toBe('scribble');
  });

  /** A typed todo has nowhere to go back to, and neither does one whose
   *  provenance was never recorded. Fails if either invents a destination. */
  it('offers no destination for a todo with nowhere to return to', () => {
    expect(sourceTabFor('manual')).toBeNull();
    expect(sourceTabFor(null)).toBeNull();
    expect(sourceTabFor(undefined)).toBeNull();
  });
});
