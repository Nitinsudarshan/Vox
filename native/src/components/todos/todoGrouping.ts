import { PARA_BANDS, type KanbanCard, type ParaBand, type TodoSourceKind } from '@/types';

/**
 * Grouping and ordering for the TODOs surface.
 *
 * Pure functions over a card list, kept out of the components so the
 * questions each view answers can be tested without rendering anything.
 */

/** A PARA band, or the explicit absence of one. */
export type TodoBand = ParaBand | 'uncategorised';

/** Every band a todo can land in, in order, Uncategorised last. */
export const TODO_BANDS: TodoBand[] = [...PARA_BANDS, 'uncategorised'];

export const TODO_BAND_LABELS: Record<TodoBand, string> = {
  projects: 'Projects',
  areas: 'Areas',
  resources: 'Resources',
  archive: 'Archive',
  uncategorised: 'Uncategorised',
};

export const SOURCE_LABELS: Record<TodoSourceKind, string> = {
  meeting: 'Meeting',
  voice_note: 'Voice note',
  scribble: 'Scribble',
  manual: 'Typed here',
  talkback: 'Talkback',
};

/** The band a todo inherited, or the explicit uncategorised bucket. */
export function bandOfTodo(card: KanbanCard): TodoBand {
  const para = card.para;
  return para && (PARA_BANDS as readonly string[]).includes(para)
    ? (para as ParaBand)
    : 'uncategorised';
}

export interface TodoBandGroup {
  band: TodoBand;
  cards: KanbanCard[];
}

/**
 * Todos grouped into their inherited bands.
 *
 * Every band is returned, including empty ones: a band that disappears
 * when it empties makes "nothing in Projects" look the same as "no
 * Projects band", and the first of those is the finding.
 */
export function groupByBand(cards: KanbanCard[]): TodoBandGroup[] {
  const groups = new Map<TodoBand, KanbanCard[]>(TODO_BANDS.map((b) => [b, []]));
  for (const card of cards) {
    groups.get(bandOfTodo(card))!.push(card);
  }
  return TODO_BANDS.map((band) => ({ band, cards: byDate(groups.get(band)!) }));
}

/**
 * Projects against Areas: effort on things that finish, against things
 * that only continue.
 *
 * This is the question the PARA view exists to answer and a flat list
 * cannot. Open cards only — a done project todo is work that finished, not
 * effort currently going somewhere.
 */
export interface EffortSplit {
  projects: number;
  areas: number;
}

export function effortSplit(cards: KanbanCard[]): EffortSplit {
  const open = cards.filter((c) => c.status !== 'done');
  return {
    projects: open.filter((c) => bandOfTodo(c) === 'projects').length,
    areas: open.filter((c) => bandOfTodo(c) === 'areas').length,
  };
}

/** When a todo arrived, falling back to when its record was written. */
export function capturedAt(card: KanbanCard): string {
  return card.captured_at ?? card.created_at;
}

/** Sorts by due date, then by capture date. Undated work sorts last. */
export function byDate(cards: KanbanCard[]): KanbanCard[] {
  return [...cards].sort((a, b) => {
    const dueA = a.due_date ?? '';
    const dueB = b.due_date ?? '';
    if (dueA !== dueB) {
      if (!dueA) return 1;
      if (!dueB) return -1;
      return dueA < dueB ? -1 : 1;
    }
    const capA = capturedAt(a);
    const capB = capturedAt(b);
    return capA < capB ? -1 : capA > capB ? 1 : a.id < b.id ? -1 : 1;
  });
}

/** Whether a todo's due date has passed. Done work is never overdue. */
export function isOverdue(card: KanbanCard, now: Date = new Date()): boolean {
  if (!card.due_date || card.status === 'done') return false;
  const due = Date.parse(card.due_date);
  if (Number.isNaN(due)) return false;
  // Compared by calendar day: something due today is not overdue until
  // today is over.
  const endOfDueDay = new Date(due);
  endOfDueDay.setHours(23, 59, 59, 999);
  return endOfDueDay.getTime() < now.getTime();
}

export interface DateOrdered {
  overdue: KanbanCard[];
  rest: KanbanCard[];
}

/** Overdue work separated and first, everything else by date. */
export function orderByDate(cards: KanbanCard[], now: Date = new Date()): DateOrdered {
  const overdue: KanbanCard[] = [];
  const rest: KanbanCard[] = [];
  for (const card of cards) {
    (isOverdue(card, now) ? overdue : rest).push(card);
  }
  return { overdue: byDate(overdue), rest: byDate(rest) };
}

/** Which tab in the app a todo's source lives on, if any. */
export function sourceTabFor(kind: TodoSourceKind | null | undefined): string | null {
  switch (kind) {
    case 'meeting':
      return 'meetings';
    case 'voice_note':
      return 'capture';
    case 'scribble':
      return 'scribble';
    default:
      return null;
  }
}
