import React, { useCallback, useEffect, useMemo, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import {
  AlertTriangle,
  CalendarClock,
  Columns3,
  Layers,
  ListChecks,
  Loader2,
  Mic,
  Plus,
  Square,
} from 'lucide-react';

import { Button } from '@/components/ui/button';
import { EmptyState } from '@/components/common/EmptyState';
import { PageHeader } from '@/components/common/PageHeader';
import {
  TODO_STATUSES,
  TODO_STATUS_LABELS,
  type KanbanCard,
  type MainTabType,
  type ProcessedPipelineResult,
  type TodoStatus,
} from '@/types';

import {
  bandOfTodo,
  capturedAt,
  effortSplit,
  groupByBand,
  isOverdue,
  orderByDate,
  sourceTabFor,
  SOURCE_LABELS,
  TODO_BAND_LABELS,
} from './todoGrouping';

type TodoView = 'para' | 'date' | 'board';

/**
 * The capture mode the backend routes to a todo.
 *
 * The same recorder, STT, normalisation and dictionary every other spoken
 * capture uses — this mode differs only in what gets written at the end.
 * There is no second recording implementation here.
 */
const TODO_CAPTURE_MODE = 'todo';

type VoiceState = 'idle' | 'recording' | 'transcribing';

const VIEW_LABELS: Record<TodoView, string> = {
  para: 'By PARA',
  date: 'By date',
  board: 'Board',
};

const VIEW_ICONS: Record<TodoView, React.ComponentType<{ className?: string }>> = {
  para: Layers,
  date: CalendarClock,
  board: Columns3,
};

interface TodosPageProps {
  onNavigateTab?: (tab: MainTabType) => void;
}

function formatDate(raw: string | null | undefined): string | null {
  if (!raw) return null;
  const parsed = Date.parse(raw);
  if (Number.isNaN(parsed)) return null;
  return new Date(parsed).toLocaleDateString(undefined, {
    month: 'short',
    day: 'numeric',
    year: 'numeric',
  });
}

/**
 * Where a todo came from, and a way back to it.
 *
 * Shown on every card in every view. A todo whose provenance was never
 * recorded says so rather than claiming to have been typed — the two are
 * different, and only one of them is a thing the user did.
 */
const SourceTrail: React.FC<{
  card: KanbanCard;
  onNavigateTab?: (tab: MainTabType) => void;
}> = ({ card, onNavigateTab }) => {
  const kind = card.source_kind;
  const label = card.source_ref?.label ?? card.source_ref?.id ?? card.source_note_id;
  const tab = sourceTabFor(kind);

  if (!kind) {
    return <span className="text-[10px] text-muted-foreground">Source unknown</span>;
  }

  const text = label ? `${SOURCE_LABELS[kind]} · ${label}` : SOURCE_LABELS[kind];
  const turn = card.source_ref?.turn_ordinal;
  const full = turn ? `${text} (turn ${turn})` : text;

  if (!tab || !onNavigateTab) {
    return <span className="text-[10px] text-muted-foreground">{full}</span>;
  }

  return (
    <button
      type="button"
      onClick={() => onNavigateTab(tab as MainTabType)}
      className="text-[10px] text-muted-foreground hover:text-foreground hover:underline text-left"
    >
      {full}
    </button>
  );
};

const TodoRow: React.FC<{
  card: KanbanCard;
  onSetStatus: (id: string, status: TodoStatus) => void;
  onNavigateTab?: (tab: MainTabType) => void;
}> = ({ card, onSetStatus, onNavigateTab }) => {
  const due = formatDate(card.due_date);
  const overdue = isOverdue(card);

  return (
    <li className="flex items-start gap-2 px-3 py-2 border border-border/60 rounded-lg bg-card/60">
      <input
        type="checkbox"
        checked={card.status === 'done'}
        onChange={(e) => onSetStatus(card.id, e.target.checked ? 'done' : 'todo')}
        aria-label={`Mark "${card.title}" done`}
        className="mt-0.5 shrink-0 accent-primary"
      />
      <div className="min-w-0 flex-1">
        <p
          className={`text-xs ${
            card.status === 'done' ? 'text-muted-foreground line-through' : 'text-foreground'
          }`}
        >
          {card.title}
        </p>
        <div className="flex items-center gap-2 flex-wrap mt-0.5">
          <SourceTrail card={card} onNavigateTab={onNavigateTab} />
          {due && (
            <span
              className={`text-[10px] ${overdue ? 'text-destructive font-medium' : 'text-muted-foreground'}`}
            >
              {overdue ? 'Overdue ' : 'Due '}
              {due}
            </span>
          )}
        </div>
      </div>
      <span className="text-[10px] font-mono text-muted-foreground shrink-0 capitalize">
        {TODO_STATUS_LABELS[card.status as TodoStatus] ?? card.status}
      </span>
    </li>
  );
};

/**
 * Every action item Vox has captured, in one place.
 *
 * Three views over one list. PARA segmentation answers the question a flat
 * list cannot — whether effort is going into work that finishes or work
 * that only continues — so that count is surfaced rather than left to be
 * counted by eye.
 */
export const TodosPage: React.FC<TodosPageProps> = ({ onNavigateTab }) => {
  const [cards, setCards] = useState<KanbanCard[]>([]);
  const [view, setView] = useState<TodoView>('para');
  const [draft, setDraft] = useState('');
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [dragging, setDragging] = useState<string | null>(null);
  const [voice, setVoice] = useState<VoiceState>('idle');

  const refresh = useCallback(async () => {
    try {
      const loaded = await invoke<KanbanCard[]>('get_kanban_cards');
      setCards(loaded ?? []);
      setError(null);
    } catch (err) {
      console.error('Failed to read the todo list:', err);
      setError('Could not read your todos from the vault.');
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    refresh();
  }, [refresh]);

  const addTyped = async () => {
    const title = draft.trim();
    if (!title) return;
    try {
      const created = await invoke<KanbanCard>('create_manual_todo', { title });
      setCards((prev) => [...prev, created]);
      setDraft('');
      setError(null);
    } catch (err) {
      console.error('Failed to create a todo:', err);
      setError('That todo could not be saved.');
    }
  };

  /**
   * Click to start, click again to stop.
   *
   * Every failure the capture path can produce is reported and produces no
   * todo: a mic that heard nothing, a transcription that failed, and a
   * transcript that came back empty are all distinct messages, because
   * silently creating a blank todo — or silently creating none — is worse
   * than any of them.
   */
  const toggleVoice = async () => {
    if (voice === 'recording') {
      setVoice('transcribing');
      try {
        const result = await invoke<ProcessedPipelineResult | null>('stop_capture');
        if (!result) {
          setError('Nothing was heard, so no todo was created.');
        } else if (!result.kanban_cards_created) {
          setError('That recording produced no usable text, so no todo was created.');
        } else {
          setError(null);
          await refresh();
        }
      } catch (err) {
        console.error('Voice todo capture failed:', err);
        setError(
          'That recording could not be transcribed, so no todo was created.',
        );
      } finally {
        setVoice('idle');
      }
      return;
    }

    try {
      await invoke('start_capture', { mode: TODO_CAPTURE_MODE });
      setVoice('recording');
      setError(null);
    } catch (err) {
      console.error('Could not start recording:', err);
      setError('Could not start recording. Another capture may be in progress.');
      setVoice('idle');
    }
  };

  const setStatus = async (id: string, status: TodoStatus) => {
    // Optimistic, then reconciled: the checkbox has to feel instant, but a
    // failed write must not leave the UI claiming the change landed.
    const previous = cards;
    setCards((prev) => prev.map((c) => (c.id === id ? { ...c, status } : c)));
    try {
      await invoke<KanbanCard>('set_todo_status', { id, status });
      setError(null);
    } catch (err) {
      console.error('Failed to move the todo:', err);
      setCards(previous);
      setError('That change could not be saved.');
    }
  };

  const bands = useMemo(() => groupByBand(cards), [cards]);
  const split = useMemo(() => effortSplit(cards), [cards]);
  const dated = useMemo(() => orderByDate(cards), [cards]);

  const renderRows = (list: KanbanCard[]) => (
    <ul className="space-y-1.5">
      {list.map((card) => (
        <TodoRow
          key={card.id}
          card={card}
          onSetStatus={setStatus}
          onNavigateTab={onNavigateTab}
        />
      ))}
    </ul>
  );

  return (
    <div className="flex-1 flex flex-col min-h-0 min-w-0 overflow-hidden">
      <PageHeader
        title="Everything you said"
        highlightText="you'd do."
        description="Action items from meetings, voice notes and scribbles, plus whatever you add here."
        glowColor="primary"
        compact
      >
        <div
          role="tablist"
          aria-label="Todo view"
          className="flex items-center gap-0.5 bg-background/60 backdrop-blur-xs border border-border/80 rounded-lg p-1 shadow-2xs"
        >
          {(Object.keys(VIEW_LABELS) as TodoView[]).map((candidate) => {
            const Icon = VIEW_ICONS[candidate];
            const isActive = candidate === view;
            return (
              <button
                key={candidate}
                type="button"
                role="tab"
                aria-selected={isActive}
                onClick={() => setView(candidate)}
                className={`flex items-center gap-1.5 h-7 px-2.5 rounded-md text-xs font-medium transition-colors ${
                  isActive
                    ? 'bg-accent text-accent-foreground'
                    : 'text-muted-foreground hover:bg-accent/50 hover:text-foreground'
                }`}
              >
                <Icon className="w-3.5 h-3.5" />
                <span>{VIEW_LABELS[candidate]}</span>
              </button>
            );
          })}
        </div>
      </PageHeader>

      <div className="px-4 pb-2 flex items-center gap-2">
        <input
          value={draft}
          onChange={(e) => setDraft(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === 'Enter') {
              e.preventDefault();
              addTyped();
            }
          }}
          placeholder="Add a todo and press Enter"
          aria-label="Add a todo"
          className="h-9 flex-1 px-3 text-sm bg-background border border-border rounded-md text-foreground placeholder:text-muted-foreground focus:outline-hidden focus:ring-1 focus:ring-ring"
        />
        <Button size="sm" variant="outline" onClick={addTyped} disabled={!draft.trim()}>
          <Plus className="w-3.5 h-3.5" />
          <span className="ml-1">Add</span>
        </Button>
        <Button
          size="sm"
          variant={voice === 'recording' ? 'destructive' : 'outline'}
          onClick={toggleVoice}
          disabled={voice === 'transcribing'}
          aria-pressed={voice === 'recording'}
          aria-label={voice === 'recording' ? 'Stop recording' : 'Record a todo'}
        >
          {voice === 'transcribing' ? (
            <Loader2 className="w-3.5 h-3.5 animate-spin" />
          ) : voice === 'recording' ? (
            <Square className="w-3.5 h-3.5" />
          ) : (
            <Mic className="w-3.5 h-3.5" />
          )}
          <span className="ml-1">
            {voice === 'transcribing'
              ? 'Transcribing…'
              : voice === 'recording'
                ? 'Stop'
                : 'Speak'}
          </span>
        </Button>
      </div>

      {error && (
        <div
          role="alert"
          className="mx-4 mb-2 flex items-center gap-2 px-3 py-2 text-xs text-destructive border border-destructive/40 bg-destructive/10 rounded-md"
        >
          <AlertTriangle className="w-3.5 h-3.5 shrink-0" />
          <span>{error}</span>
        </div>
      )}

      {!loading && cards.length === 0 ? (
        <EmptyState
          icon={ListChecks}
          title="No todos yet"
          description="Action items found in meetings, voice notes and scribbles land here automatically. You can also type one above."
          minHeight="min-h-[260px]"
        />
      ) : (
        <div className="flex-1 min-h-0 overflow-y-auto px-4 pb-4">
          {view === 'para' && (
            <div className="space-y-4">
              <p className="text-xs text-muted-foreground">
                <span className="font-medium text-foreground">{split.projects}</span> open in
                Projects — work that finishes — against{' '}
                <span className="font-medium text-foreground">{split.areas}</span> in Areas,
                which only continues.
              </p>
              {bands.map(({ band, cards: bandCards }) => (
                <section key={band} className="space-y-1.5">
                  <h3 className="text-[10px] font-bold font-mono uppercase tracking-widest text-muted-foreground">
                    {TODO_BAND_LABELS[band]}{' '}
                    <span className="text-foreground">{bandCards.length}</span>
                  </h3>
                  {bandCards.length === 0 ? (
                    <p className="text-[11px] text-muted-foreground/70 pl-1">Nothing here.</p>
                  ) : (
                    renderRows(bandCards)
                  )}
                </section>
              ))}
            </div>
          )}

          {view === 'date' && (
            <div className="space-y-4">
              {dated.overdue.length > 0 && (
                <section className="space-y-1.5">
                  <h3 className="text-[10px] font-bold font-mono uppercase tracking-widest text-destructive">
                    Overdue <span>{dated.overdue.length}</span>
                  </h3>
                  {renderRows(dated.overdue)}
                </section>
              )}
              <section className="space-y-1.5">
                <h3 className="text-[10px] font-bold font-mono uppercase tracking-widest text-muted-foreground">
                  Everything else <span className="text-foreground">{dated.rest.length}</span>
                </h3>
                {renderRows(dated.rest)}
              </section>
            </div>
          )}

          {view === 'board' && (
            <div className="grid grid-cols-1 md:grid-cols-3 gap-3">
              {TODO_STATUSES.map((status) => {
                const column = cards.filter((c) => c.status === status);
                return (
                  <section
                    key={status}
                    aria-label={TODO_STATUS_LABELS[status]}
                    onDragOver={(e) => e.preventDefault()}
                    onDrop={(e) => {
                      e.preventDefault();
                      const id = dragging ?? e.dataTransfer.getData('text/plain');
                      if (id) setStatus(id, status);
                      setDragging(null);
                    }}
                    className="rounded-lg border border-border/60 bg-muted/20 p-2 space-y-1.5 min-h-[120px]"
                  >
                    <h3 className="text-[10px] font-bold font-mono uppercase tracking-widest text-muted-foreground">
                      {TODO_STATUS_LABELS[status]}{' '}
                      <span className="text-foreground">{column.length}</span>
                    </h3>
                    <ul className="space-y-1.5">
                      {column.map((card) => (
                        <li
                          key={card.id}
                          draggable
                          onDragStart={(e) => {
                            setDragging(card.id);
                            e.dataTransfer.setData('text/plain', card.id);
                          }}
                          onDragEnd={() => setDragging(null)}
                          className="px-2.5 py-2 rounded-md border border-border/60 bg-card cursor-grab active:cursor-grabbing"
                        >
                          <p className="text-xs text-foreground">{card.title}</p>
                          <div className="mt-0.5 flex items-center gap-2 flex-wrap">
                            <SourceTrail card={card} onNavigateTab={onNavigateTab} />
                            <span className="text-[10px] text-muted-foreground">
                              {TODO_BAND_LABELS[bandOfTodo(card)]}
                            </span>
                          </div>
                          {/*
                            Drag needs a pointer. These move the same card
                            between the same columns from the keyboard, so
                            the board is not the one view a keyboard user
                            cannot operate.
                          */}
                          <div className="mt-1 flex gap-1">
                            {TODO_STATUSES.filter((s) => s !== status).map((target) => (
                              <button
                                key={target}
                                type="button"
                                onClick={() => setStatus(card.id, target)}
                                className="text-[9px] px-1.5 py-0.5 rounded border border-border/50 text-muted-foreground hover:text-foreground hover:bg-accent"
                              >
                                → {TODO_STATUS_LABELS[target]}
                              </button>
                            ))}
                          </div>
                          <span className="sr-only">
                            Captured {formatDate(capturedAt(card))}
                          </span>
                        </li>
                      ))}
                    </ul>
                  </section>
                );
              })}
            </div>
          )}
        </div>
      )}
    </div>
  );
};
