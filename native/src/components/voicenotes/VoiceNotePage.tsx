import React, { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import {
  HardDrive,
  Mic,
  ShieldCheck,
  Edit3,
  Trash2,
  GitMerge,
  Copy,
  Check,
  X,
  Save,
  Sparkles,
  Undo,
  AlertCircle,
  ChevronDown,
  ChevronRight,
  Plus,
  LayoutGrid,
  List,
} from 'lucide-react';
import { Badge } from '@/components/ui/badge';
import { findSelection, looksLikeVocabulary, type PhraseSelection } from './selection';
import { diffWords } from '@/lib/diffWords';
import { Button } from '@/components/ui/button';
import { PageHeader } from '../common/PageHeader';
import { EmptyState } from '../common/EmptyState';
import { AppSettings, CorrectionRecord, VaultLocationInfo, VaultNote } from '../../types';

type VaultViewState =
  | { status: 'loading' }
  | { status: 'setup' }
  | { status: 'recovery' }
  | { status: 'ready' };

export type ViewMode = 'grid' | 'list';

function formatNoteTimestamp(iso: string): string {
  const date = new Date(iso);
  if (Number.isNaN(date.getTime())) return iso;

  const now = new Date();
  const yesterday = new Date(now);
  yesterday.setDate(now.getDate() - 1);
  const time = date.toLocaleTimeString([], { hour: 'numeric', minute: '2-digit' });

  if (date.toDateString() === now.toDateString()) return `Today · ${time}`;
  if (date.toDateString() === yesterday.toDateString()) return `Yesterday · ${time}`;
  return `${date.toLocaleDateString([], { month: 'short', day: 'numeric' })} · ${time}`;
}

function countWords(str: string): number {
  return str.trim().split(/\s+/).filter(Boolean).length;
}

interface DateGroup {
  key: string;
  label: string;
  dateStr: string;
  notes: VaultNote[];
  totalWords: number;
}

function groupNotesByDay(notes: VaultNote[]): DateGroup[] {
  const groups: Map<string, { label: string; dateStr: string; notes: VaultNote[] }> = new Map();
  const now = new Date();
  const todayKey = now.toDateString();
  const yesterday = new Date(now);
  yesterday.setDate(now.getDate() - 1);
  const yesterdayKey = yesterday.toDateString();

  for (const note of notes) {
    const d = new Date(note.created_at);
    const key = Number.isNaN(d.getTime()) ? 'other' : d.toDateString();
    let label = 'Other';
    if (key === todayKey) {
      label = 'Today';
    } else if (key === yesterdayKey) {
      label = 'Yesterday';
    } else if (!Number.isNaN(d.getTime())) {
      label = d.toLocaleDateString([], {
        month: 'short',
        day: 'numeric',
        year: d.getFullYear() !== now.getFullYear() ? 'numeric' : undefined,
      });
    }

    if (!groups.has(key)) {
      groups.set(key, { label, dateStr: key, notes: [] });
    }
    groups.get(key)!.notes.push(note);
  }

  const result: DateGroup[] = [];
  for (const [key, val] of groups.entries()) {
    const totalWords = val.notes.reduce((sum, n) => sum + countWords(n.content), 0);
    result.push({
      key,
      label: val.label,
      dateStr: val.dateStr,
      notes: val.notes,
      totalWords,
    });
  }

  return result;
}

interface VaultSetupPromptProps {
  recovery?: boolean;
  defaultPath: string;
  busy: boolean;
  error: string;
  onChooseFolder: () => void;
  onUseDefault: () => void;
}

const VaultSetupPrompt: React.FC<VaultSetupPromptProps> = ({
  recovery = false,
  defaultPath,
  busy,
  error,
  onChooseFolder,
  onUseDefault,
}) => (
  <div className="flex-1 flex flex-col items-center justify-center p-6 text-center">
    <div className="w-12 h-12 rounded-2xl bg-primary/10 border border-primary/20 flex items-center justify-center text-primary mb-4 shadow-sm">
      {recovery ? <ShieldCheck className="w-6 h-6" /> : <HardDrive className="w-6 h-6" />}
    </div>
    <h2 className="text-lg font-bold text-foreground mb-1">
      {recovery ? 'Vault Access Required' : 'Choose Your Vox Vault Location'}
    </h2>
    <p className="text-xs text-muted-foreground max-w-md mb-6 leading-relaxed">
      {recovery
        ? 'The configured Vault folder is missing or inaccessible. Re-select the folder or reset to default to view and store your Voice Notes.'
        : 'Select where Vox stores your Voice Notes, scribbles, and transcripts on your computer.'}
    </p>

    {error && (
      <div className="mb-4 p-3 rounded-lg bg-red-500/10 border border-red-500/30 text-xs text-red-600 dark:text-red-400 max-w-md shadow-xs">
        {error}
      </div>
    )}

    <div className="flex items-center gap-3">
      <Button onClick={onChooseFolder} disabled={busy} size="sm">
        Choose Folder
      </Button>
      {!recovery && (
        <Button onClick={onUseDefault} disabled={busy} size="sm" variant="outline">
          Use Default Vox Vault
        </Button>
      )}
    </div>
  </div>
);

export const VoiceNotePage: React.FC = () => {
  const [vaultState, setVaultState] = useState<VaultViewState>({ status: 'loading' });
  const [defaultPath, setDefaultPath] = useState('');
  const [notes, setNotes] = useState<VaultNote[]>([]);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState('');

  // View Mode: grid or list
  const [viewMode, setViewMode] = useState<ViewMode>(() => {
    try {
      const saved = localStorage.getItem('vox_voicenotes_view_mode');
      if (saved === 'grid' || saved === 'list') {
        return saved;
      }
    } catch {}
    return 'grid';
  });

  useEffect(() => {
    try {
      localStorage.setItem('vox_voicenotes_view_mode', viewMode);
    } catch {}
  }, [viewMode]);

  // Interactive Action States
  const [editingNoteId, setEditingNoteId] = useState<string | null>(null);
  const [editingContent, setEditingContent] = useState('');
  const [unmergingNoteId, setUnmergingNoteId] = useState<string | null>(null);
  const [copiedNoteId, setCopiedNoteId] = useState<string | null>(null);
  const [promotedNoteIds, setPromotedNoteIds] = useState<Set<string>>(new Set());
  const [selectedNoteIds, setSelectedNoteIds] = useState<Set<string>>(new Set());
  const [diffNoteIds, setDiffNoteIds] = useState<Set<string>>(new Set());
  const [activeSelectionMode, setActiveSelectionMode] = useState<'merge' | 'delete' | null>(null);

  const toggleDiffNote = (id: string, e?: React.MouseEvent) => {
    e?.stopPropagation();
    setDiffNoteIds((prev) => {
      const next = new Set(prev);
      if (next.has(id)) {
        next.delete(id);
      } else {
        next.add(id);
      }
      return next;
    });
  };
  const [isMergingBatch, setIsMergingBatch] = useState(false);
  const [isBulkDeleting, setIsBulkDeleting] = useState(false);
  const [actionBusy, setActionBusy] = useState(false);
  const [settings, setSettings] = useState<AppSettings | null>(null);

  // Accordion state
  const [expandedSections, setExpandedSections] = useState<Set<string>>(new Set());

  // Group notes chronologically by day
  const dateGroups = useMemo(() => groupNotesByDay(notes), [notes]);

  // Set default accordion expansion: 'Today' open, or first available group if today is empty
  useEffect(() => {
    if (dateGroups.length > 0 && expandedSections.size === 0) {
      const todayGroup = dateGroups.find((g) => g.label === 'Today');
      if (todayGroup) {
        setExpandedSections(new Set([todayGroup.key]));
      } else {
        setExpandedSections(new Set([dateGroups[0].key]));
      }
    }
  }, [dateGroups, expandedSections.size]);

  const toggleSection = (key: string) => {
    setExpandedSections((prev) => {
      const next = new Set(prev);
      if (next.has(key)) {
        next.delete(key);
      } else {
        next.add(key);
      }
      return next;
    });
  };


  const toggleSelectNote = (id: string) => {
    setSelectedNoteIds((prev) => {
      const next = new Set(prev);
      if (next.has(id)) {
        next.delete(id);
      } else {
        next.add(id);
      }
      return next;
    });
  };

  const handleCardClick = (id: string, e: React.MouseEvent) => {
    const sel = window.getSelection()?.toString().trim();
    if (sel && sel.length > 0) {
      return;
    }
    toggleSelectNote(id);
  };

  const handleSelectAll = () => {
    if (selectedNoteIds.size === notes.length) {
      setSelectedNoteIds(new Set());
    } else {
      setSelectedNoteIds(new Set(notes.map((n) => n.id)));
    }
  };

  const handleClearSelection = () => {
    setSelectedNoteIds(new Set());
    setActiveSelectionMode(null);
    setIsMergingBatch(false);
    setIsBulkDeleting(false);
  };

  // Bulk or Single Delete
  const handleBulkDelete = async () => {
    if (selectedNoteIds.size === 0) return;
    setActionBusy(true);
    setError('');
    const idsToDelete = Array.from(selectedNoteIds);
    try {
      if (idsToDelete.length === 1) {
        await invoke('delete_voice_note', { id: idsToDelete[0] });
      } else {
        await invoke('delete_voice_notes', { ids: idsToDelete });
      }
      setNotes((prev) => prev.filter((n) => !selectedNoteIds.has(n.id)));
      setSelectedNoteIds(new Set());
      setIsBulkDeleting(false);
      setActiveSelectionMode(null);
    } catch (err: any) {
      console.error('Failed to delete voice notes', err);
      setError(err?.message || 'Failed to delete selected voice notes.');
    } finally {
      setActionBusy(false);
    }
  };

  // Chronological Batch Merge: oldest at top, newest at bottom
  const handleBatchMerge = async () => {
    if (selectedNoteIds.size < 2) return;
    setActionBusy(true);
    setError('');

    const sortedSelectedNotes = notes
      .filter((n) => selectedNoteIds.has(n.id))
      .sort((a, b) => a.created_at.localeCompare(b.created_at));

    const sortedIds = sortedSelectedNotes.map((n) => n.id);

    try {
      const merged = await invoke<VaultNote>('merge_multiple_voice_notes', { ids: sortedIds });
      const secondaryIds = new Set(sortedIds.filter((id) => id !== merged.id));

      setNotes((prev) =>
        prev
          .filter((n) => !secondaryIds.has(n.id))
          .map((n) => (n.id === merged.id ? merged : n))
          .sort((a, b) => b.created_at.localeCompare(a.created_at))
      );
      setSelectedNoteIds(new Set());
      setIsMergingBatch(false);
      setActiveSelectionMode(null);
    } catch (err: any) {
      console.error('Failed to merge voice notes', err);
      setError(err?.message || 'Failed to merge selected voice notes.');
    } finally {
      setActionBusy(false);
    }
  };

  const handlePromoteToScribble = async (note: VaultNote) => {
    setActionBusy(true);
    try {
      await invoke('promote_voice_note_to_scribble', {
        voiceNoteId: note.id,
      });
      setPromotedNoteIds((prev) => new Set(prev).add(note.id));
    } catch (err) {
      console.error('Failed to promote Voice Note to Scribble:', err);
    } finally {
      setActionBusy(false);
    }
  };

  const refreshLocation = async () => {
    try {
      const [info, appSetts] = await Promise.all([
        invoke<VaultLocationInfo>('get_vault_location'),
        invoke<AppSettings>('get_settings').catch(() => null),
      ]);
      if (appSetts) setSettings(appSetts);
      setDefaultPath(info.default_path);
      if (!info.configured) {
        setVaultState({ status: 'setup' });
      } else if (!info.accessible) {
        setVaultState({ status: 'recovery' });
      } else {
        setVaultState({ status: 'ready' });
      }
    } catch (err) {
      console.error('Failed to read Vault Directory Location', err);
      setError('Could not determine where Voice Notes are stored.');
      setVaultState({ status: 'recovery' });
    }
  };

  useEffect(() => {
    refreshLocation();

    const unlistenSettings = listen<AppSettings>('settings-changed', ({ payload }) => {
      if (payload) setSettings(payload);
    });

    return () => {
      unlistenSettings.then((unlisten) => unlisten());
    };
  }, []);

  const refreshPromotedScribbles = useCallback(() => {
    invoke<{ source_metadata?: { source_voice_note_id?: string; source_voice_note_ids?: string[] } }[]>('get_scribbles')
      .then((scribbles) => {
        const ids = new Set<string>();
        for (const s of scribbles) {
          if (s.source_metadata?.source_voice_note_id) {
            ids.add(s.source_metadata.source_voice_note_id);
          }
          if (Array.isArray(s.source_metadata?.source_voice_note_ids)) {
            for (const id of s.source_metadata.source_voice_note_ids) {
              ids.add(id);
            }
          }
        }
        setPromotedNoteIds(ids);
      })
      .catch((err) => console.error('Failed to load Scribble promotion mapping', err));
  }, []);

  useEffect(() => {
    if (vaultState.status !== 'ready') return;
    invoke<VaultNote[]>('get_voice_notes')
      .then(setNotes)
      .catch((err) => console.error('Failed to load Voice Notes', err));

    refreshPromotedScribbles();
  }, [vaultState.status, refreshPromotedScribbles]);

  useEffect(() => {
    const unlistenVoice = listen<VaultNote>('voice-note-saved', ({ payload }) => {
      setNotes((prev) => [payload, ...prev.filter((n) => n.id !== payload.id)]);
      refreshPromotedScribbles();
    });
    const unlistenScribble = listen('scribble-saved', () => {
      refreshPromotedScribbles();
    });
    return () => {
      unlistenVoice.then((unlisten) => unlisten());
      unlistenScribble.then((unlisten) => unlisten());
    };
  }, [refreshPromotedScribbles]);

  const handleChooseFolder = async () => {
    setBusy(true);
    setError('');
    try {
      const picked = await invoke<string | null>('choose_vault_folder');
      if (!picked) return;
      await invoke('set_vault_location', { path: picked });
      await refreshLocation();
    } catch (err: any) {
      console.error('Failed to set Vault Directory Location', err);
      setError(err?.message || "Couldn't use that folder — choose another.");
    } finally {
      setBusy(false);
    }
  };

  const handleUseDefault = async () => {
    setBusy(true);
    setError('');
    try {
      await invoke('set_vault_location', { path: defaultPath });
      await refreshLocation();
    } catch (err: any) {
      console.error('Failed to set default Vault Directory Location', err);
      setError(err?.message || 'Could not use the default Vox Vault.');
    } finally {
      setBusy(false);
    }
  };

  // Phrase correction state and listeners
  const [selection, setSelection] = useState<PhraseSelection | null>(null);
  const [correctionOpen, setCorrectionOpen] = useState(false);
  const [replacement, setReplacement] = useState('');
  const [teachVox, setTeachVox] = useState(false);
  const [correcting, setCorrecting] = useState(false);
  const [correctionError, setCorrectionError] = useState<string | null>(null);
  const [undoState, setUndoState] = useState<
    { noteId: string; message: string; undoable: boolean } | null
  >(null);
  const noteBodyRefs = useRef<Record<string, HTMLElement | null>>({});
  const notesRef = useRef<VaultNote[]>(notes);
  notesRef.current = notes;
  const selectionRef = useRef<PhraseSelection | null>(null);
  selectionRef.current = selection;
  const correctionOpenRef = useRef(false);
  correctionOpenRef.current = correctionOpen;

  useEffect(() => {
    const onSelectionChange = () => {
      const found = findSelection(
        noteBodyRefs.current,
        (noteId) => notesRef.current.find((n) => n.id === noteId)?.content,
      );
      if (found) {
        const previous = selectionRef.current;
        if (
          previous &&
          previous.noteId === found.noteId &&
          previous.start === found.start &&
          previous.end === found.end
        ) {
          return;
        }
        setSelection(found);
        setCorrectionOpen(false);
        setReplacement('');
        setTeachVox(false);
        setCorrectionError(null);
        return;
      }
      if (!correctionOpenRef.current) {
        setSelection(null);
      }
    };

    document.addEventListener('selectionchange', onSelectionChange);
    return () => document.removeEventListener('selectionchange', onSelectionChange);
  }, []);

  const dismissSelection = () => {
    setSelection(null);
    setCorrectionOpen(false);
    setReplacement('');
    setTeachVox(false);
    setCorrectionError(null);
    window.getSelection()?.removeAllRanges();
  };

  const handleApplyCorrection = async () => {
    if (!selection || !replacement.trim()) return;
    setCorrecting(true);
    setCorrectionError(null);
    try {
      const result = await invoke<{
        note: VaultNote;
        record: CorrectionRecord;
        learned: boolean;
      }>('correct_voice_note_phrase', {
        id: selection.noteId,
        start: selection.start,
        end: selection.end,
        original: selection.text,
        replacement: replacement.trim(),
        learn: teachVox,
      });
      const corrected = result.note;
      setNotes((prev) => prev.map((n) => (n.id === corrected.id ? corrected : n)));
      setUndoState({
        noteId: corrected.id,
        message: `Corrected "${selection.text}" → "${replacement.trim()}"${
          result.learned ? ' · learned' : ''
        }`,
        undoable: true,
      });
      dismissSelection();
    } catch (err: any) {
      console.error('Failed to correct phrase', err);
      setCorrectionError(err?.message || 'Could not apply that correction.');
    } finally {
      setCorrecting(false);
    }
  };

  const handleAddToDictionary = async () => {
    if (!selection) return;
    const word = selection.text.trim();
    setCorrecting(true);
    setCorrectionError(null);
    try {
      await invoke<string[]>('add_dictionary_word', { word });
      setUndoState({
        noteId: selection.noteId,
        message: `Added "${word}" to your dictionary`,
        undoable: false,
      });
      dismissSelection();
    } catch (err: any) {
      console.error('Failed to add to dictionary', err);
      setCorrectionError(err?.message || 'Could not add that to the dictionary.');
    } finally {
      setCorrecting(false);
    }
  };

  const handleUndoCorrection = async () => {
    if (!undoState) return;
    try {
      const restored = await invoke<VaultNote>('undo_voice_note_correction', {
        id: undoState.noteId,
      });
      const restoredId = restored.id;
      setNotes((prev) => prev.map((n) => (n.id === restoredId ? restored : n)));
      setUndoState(null);
    } catch (err: any) {
      console.error('Failed to undo correction', err);
      setUndoState({
        ...undoState,
        message: err?.message || 'That correction can no longer be undone.',
        undoable: false,
      });
    }
  };

  // Note Action Handlers
  const handleStartEdit = (note: VaultNote) => {
    setEditingNoteId(note.id);
    setEditingContent(note.content);
    setUnmergingNoteId(null);
  };

  const handleCancelEdit = () => {
    setEditingNoteId(null);
    setEditingContent('');
  };

  const handleSaveEdit = async (id: string) => {
    if (!editingContent.trim()) return;
    setActionBusy(true);
    try {
      const updated = await invoke<VaultNote>('update_voice_note', {
        id,
        content: editingContent.trim(),
      });
      setNotes((prev) => prev.map((n) => (n.id === id ? updated : n)));
      setEditingNoteId(null);
    } catch (err) {
      console.error('Failed to update voice note', err);
    } finally {
      setActionBusy(false);
    }
  };

  const handleUnmerge = async (id: string) => {
    setActionBusy(true);
    setError('');
    try {
      const res = await invoke<{ primary: VaultNote; secondary: VaultNote }>('unmerge_voice_note', { id });
      setNotes((prev) => {
        const next = prev.map((n) => (n.id === id ? res.primary : n));
        if (!next.some((n) => n.id === res.secondary.id)) {
          next.push(res.secondary);
        } else {
          for (let i = 0; i < next.length; i++) {
            if (next[i].id === res.secondary.id) {
              next[i] = res.secondary;
            }
          }
        }
        return next.sort((a, b) => b.created_at.localeCompare(a.created_at));
      });
      setUnmergingNoteId(null);
    } catch (err: any) {
      console.error('Failed to unmerge voice note', err);
      setError(err?.message || 'Failed to unmerge voice note. The operation was aborted.');
    } finally {
      setActionBusy(false);
    }
  };

  const handleCopy = (note: VaultNote) => {
    navigator.clipboard.writeText(note.content);
    setCopiedNoteId(note.id);
    setTimeout(() => setCopiedNoteId(null), 1800);
  };

  const stats = useMemo(() => {
    const total = notes.length;
    const totalWords = notes.reduce((sum, n) => sum + countWords(n.content), 0);
    const todayKey = new Date().toDateString();
    const notesToday = notes.filter((n) => new Date(n.created_at).toDateString() === todayKey).length;
    return { total, totalWords, notesToday };
  }, [notes]);

  const hasDiff = (note: VaultNote) => {
    return Boolean(
      note.raw_content &&
      note.raw_content.trim() !== note.content.trim() &&
      note.cleanup_style &&
      note.cleanup_style !== 'raw'
    );
  };

  const renderNoteBody = (note: VaultNote, isGrid: boolean) => {
    const isShowingDiff = diffNoteIds.has(note.id) && hasDiff(note);
    if (isShowingDiff && note.raw_content) {
      const spans = diffWords(note.raw_content, note.content);
      return (
        <div
          ref={(el) => {
            noteBodyRefs.current[note.id] = el;
          }}
          className={`text-foreground/90 leading-relaxed font-sans select-text cursor-text break-words whitespace-pre-wrap ${
            isGrid ? 'text-xs md:text-sm' : 'text-xs'
          }`}
        >
          {spans.map((span, idx) => {
            if (span.kind === 'same') {
              return <span key={idx}>{span.text}</span>;
            }
            if (span.kind === 'removed') {
              return (
                <span
                  key={idx}
                  className="line-through text-rose-600 dark:text-rose-400 bg-rose-500/15 rounded-xs px-0.5"
                >
                  {span.text}
                </span>
              );
            }
            return (
              <span
                key={idx}
                className="text-emerald-700 dark:text-emerald-400 bg-emerald-500/15 rounded-xs px-0.5 font-medium"
              >
                {span.text}
              </span>
            );
          })}
        </div>
      );
    }
    return (
      <p
        ref={(el) => {
          noteBodyRefs.current[note.id] = el;
        }}
        className={`text-foreground/90 leading-relaxed font-sans select-text cursor-text break-words whitespace-pre-wrap ${
          isGrid ? 'text-xs md:text-sm' : 'text-xs'
        }`}
      >
        {note.content}
      </p>
    );
  };

  // Reusable Phrase Selection & Correction Popover
  const renderPhraseCorrectionPopover = (note: VaultNote) => {
    if (selection?.noteId !== note.id) return null;
    return (
      <div
        role="group"
        aria-label="Correct selected phrase"
        onClick={(e) => e.stopPropagation()}
        className="mt-2.5 rounded-xl border border-border/80 bg-card/95 dark:bg-card/90 backdrop-blur-md shadow-lg dark:shadow-xl dark:shadow-black/50 p-3 space-y-2.5 animate-in fade-in zoom-in-95 duration-150 ring-1 ring-border/50"
      >
        <div className="flex items-center justify-between gap-2 text-xs">
          <div className="flex items-center gap-1.5 min-w-0">
            <span className="text-[11px] font-medium text-muted-foreground shrink-0 flex items-center gap-1">
              <Sparkles className="w-3 h-3 text-primary" />
              Selected
            </span>
            <code className="px-2 py-0.5 rounded-md bg-primary/10 dark:bg-primary/20 text-primary dark:text-primary font-mono text-xs font-semibold border border-primary/20 break-all shadow-2xs">
              {selection.text}
            </code>
          </div>
          <Button
            size="sm"
            variant="ghost"
            aria-label="Dismiss selection"
            onClick={dismissSelection}
            className="h-6 w-6 p-0 shrink-0 text-muted-foreground hover:text-foreground rounded-md"
          >
            <X className="w-3.5 h-3.5" />
          </Button>
        </div>

        {!correctionOpen ? (
          <div className="flex flex-wrap items-center gap-2 pt-1 border-t border-border/40">
            <Button
              size="sm"
              variant="default"
              onClick={() => setCorrectionOpen(true)}
              className="h-7 text-xs font-semibold gap-1.5 shadow-xs"
            >
              <Edit3 className="w-3 h-3" />
              Correct
            </Button>
            <Button
              size="sm"
              variant="outline"
              disabled={correcting}
              onClick={() => void handleAddToDictionary()}
              className="h-7 text-xs font-medium gap-1.5"
            >
              <Plus className="w-3 h-3" />
              Add to Dictionary
            </Button>
            <span className="text-[10px] text-muted-foreground/80">
              Adding keeps this spelling — correcting changes the note.
            </span>
          </div>
        ) : (
          <div className="space-y-2 pt-1 border-t border-border/40">
            <div className="flex flex-wrap items-center gap-1.5">
              <input
                autoFocus
                value={replacement}
                onChange={(e) => setReplacement(e.target.value)}
                onKeyDown={(e) => {
                  if (e.key === 'Enter') void handleApplyCorrection();
                  if (e.key === 'Escape') dismissSelection();
                }}
                placeholder="Replace with…"
                aria-label="Replacement text"
                className="flex-1 min-w-[150px] text-xs rounded-lg border border-border bg-background px-2.5 py-1.5 text-foreground focus:outline-none focus:ring-2 focus:ring-primary/20 focus:border-primary transition-all"
              />
              <Button
                size="sm"
                variant="default"
                disabled={!replacement.trim() || correcting}
                onClick={() => void handleApplyCorrection()}
                className="h-7 text-xs font-semibold shadow-xs"
              >
                Replace
              </Button>
              <Button
                size="sm"
                variant="ghost"
                onClick={dismissSelection}
                className="h-7 text-xs"
              >
                Cancel
              </Button>
            </div>

            <label className="flex items-center gap-2 text-[11px] text-muted-foreground cursor-pointer pt-0.5">
              <input
                type="checkbox"
                checked={teachVox}
                onChange={(e) => setTeachVox(e.target.checked)}
                className="accent-primary rounded"
              />
              <span>
                Teach Vox this correction
                {replacement.trim() && looksLikeVocabulary(selection.text, replacement) && (
                  <span className="ml-1 text-emerald-600 dark:text-emerald-400 font-medium">
                    · looks like a name Vox misheard
                  </span>
                )}
              </span>
            </label>
            <p className="text-[10px] text-muted-foreground/80 leading-snug">
              Replaces only this occurrence. Teaching also repairs it in future transcripts.
            </p>
          </div>
        )}

        {correctionError && (
          <p role="alert" className="text-[11px] text-destructive leading-snug">
            {correctionError}
          </p>
        )}
      </div>
    );
  };

  // Reusable Inline Edit Mode
  const renderInlineEdit = (note: VaultNote) => (
    <div className="space-y-2 pt-1" onClick={(e) => e.stopPropagation()}>
      <textarea
        value={editingContent}
        onChange={(e) => setEditingContent(e.target.value)}
        onKeyDown={(e) => {
          if (e.key === 'Enter' && (e.ctrlKey || e.metaKey)) {
            handleSaveEdit(note.id);
          }
          if (e.key === 'Escape') {
            handleCancelEdit();
          }
        }}
        disabled={actionBusy}
        className="w-full min-h-[120px] p-2.5 text-xs md:text-sm bg-background border border-border rounded-lg text-foreground focus:outline-none focus:ring-1 focus:ring-ring font-sans leading-relaxed resize-y"
        autoFocus
      />
      <div className="flex items-center justify-between">
        <span className="text-[10px] text-muted-foreground">
          <kbd className="font-mono text-[9px] bg-muted px-1 py-0.5 rounded">Ctrl+Enter</kbd> to save, <kbd className="font-mono text-[9px] bg-muted px-1 py-0.5 rounded">Esc</kbd> to cancel
        </span>
        <div className="flex items-center gap-1.5">
          <Button
            size="sm"
            variant="ghost"
            onClick={handleCancelEdit}
            disabled={actionBusy}
            className="h-6 text-xs px-2"
          >
            Cancel
          </Button>
          <Button
            size="sm"
            variant="default"
            onClick={() => handleSaveEdit(note.id)}
            disabled={actionBusy || !editingContent.trim()}
            className="h-6 text-xs gap-1 px-2.5"
          >
            <Save className="w-3 h-3" />
            <span>Save</span>
          </Button>
        </div>
      </div>
    </div>
  );

  // Reusable Action Toolbar for Notes
  const renderNoteActions = (note: VaultNote) => (
    <div className="flex items-center gap-1 shrink-0" onClick={(e) => e.stopPropagation()}>
      {note.merged_from && note.merged_from.length > 0 && (
        <Button
          size="icon"
          variant="ghost"
          onClick={() => setUnmergingNoteId(unmergingNoteId === note.id ? null : note.id)}
          disabled={actionBusy}
          className={`h-6 w-6 rounded-md transition-colors ${
            unmergingNoteId === note.id
              ? 'bg-primary/10 text-primary'
              : 'text-muted-foreground hover:text-primary hover:bg-primary/10'
          }`}
          title="Unmerge this Voice Note"
          aria-label="Unmerge this Voice Note"
        >
          <Undo className="w-3 h-3" />
        </Button>
      )}

      <Button
        size="icon"
        variant="ghost"
        onClick={() => handlePromoteToScribble(note)}
        disabled={actionBusy || promotedNoteIds.has(note.id)}
        className={`h-6 w-6 rounded-md transition-colors ${
          promotedNoteIds.has(note.id)
            ? 'bg-primary/10 text-primary cursor-default'
            : 'text-muted-foreground hover:text-primary hover:bg-primary/10'
        }`}
        title={promotedNoteIds.has(note.id) ? 'Promoted to Scribble' : 'Save as Scribble'}
        aria-label="Save as Scribble"
      >
        {promotedNoteIds.has(note.id) ? (
          <Check className="w-3 h-3 text-primary" />
        ) : (
          <Sparkles className="w-3 h-3" />
        )}
      </Button>

      <Button
        size="icon"
        variant="ghost"
        onClick={() => handleStartEdit(note)}
        className="h-6 w-6 rounded-md text-muted-foreground hover:text-foreground hover:bg-muted"
        title="Edit transcript"
        aria-label="Edit transcript"
      >
        <Edit3 className="w-3 h-3" />
      </Button>

      <Button
        size="icon"
        variant="ghost"
        onClick={() => handleCopy(note)}
        className="h-6 w-6 rounded-md text-muted-foreground hover:text-foreground hover:bg-muted"
        title="Copy transcript"
        aria-label="Copy transcript"
      >
        {copiedNoteId === note.id ? (
          <Check className="w-3 h-3 text-emerald-500" />
        ) : (
          <Copy className="w-3 h-3" />
        )}
      </Button>
    </div>
  );

  // Reusable Undo Notification
  const renderUndoNotification = (note: VaultNote) => {
    if (undoState?.noteId !== note.id) return null;
    return (
      <div
        onClick={(e) => e.stopPropagation()}
        className="mt-2 flex flex-wrap items-center justify-between gap-2 rounded-xl border border-border bg-muted/60 dark:bg-muted/30 backdrop-blur-xs px-2.5 py-1.5 text-xs animate-in fade-in duration-150 shadow-xs"
      >
        <span className="text-muted-foreground break-all text-[11px]">{undoState.message}</span>
        <div className="flex items-center gap-1">
          {undoState.undoable && (
            <Button
              size="sm"
              variant="ghost"
              onClick={() => void handleUndoCorrection()}
              className="h-6 text-xs gap-1 px-1.5 text-primary hover:text-primary"
            >
              <Undo className="w-3 h-3" />
              Undo
            </Button>
          )}
          <Button
            size="sm"
            variant="ghost"
            onClick={() => setUndoState(null)}
            className="h-6 text-xs px-1.5 text-muted-foreground"
          >
            Dismiss
          </Button>
        </div>
      </div>
    );
  };

  // Reusable Unmerge Confirmation Banner
  const renderUnmergeBanner = (note: VaultNote) => {
    if (unmergingNoteId !== note.id) return null;
    return (
      <div
        onClick={(e) => e.stopPropagation()}
        className="my-2 p-3 bg-accent/40 border border-border rounded-lg text-xs space-y-2 animate-in fade-in duration-150"
      >
        <div className="flex items-center gap-1.5 font-semibold text-foreground">
          <Undo className="w-4 h-4 text-primary shrink-0" />
          <span>Unmerge this Voice Note?</span>
        </div>
        <p className="text-[11px] text-muted-foreground leading-relaxed">
          This will restore the original Voice Notes and remove the merged version.
        </p>
        <div className="flex items-center justify-end gap-2 pt-1">
          <Button
            size="sm"
            variant="ghost"
            disabled={actionBusy}
            onClick={() => setUnmergingNoteId(null)}
            className="h-7 text-xs"
          >
            Cancel
          </Button>
          <Button
            size="sm"
            variant="default"
            disabled={actionBusy}
            onClick={() => handleUnmerge(note.id)}
            className="h-7 text-xs gap-1.5 font-semibold"
          >
            <Undo className="w-3.5 h-3.5" />
            <span>Unmerge</span>
          </Button>
        </div>
      </div>
    );
  };

  if (vaultState.status === 'loading') {
    return (
      <div className="flex-1 flex items-center justify-center text-xs text-muted-foreground">
        Loading Voice Notes…
      </div>
    );
  }

  if (vaultState.status === 'setup' || vaultState.status === 'recovery') {
    return (
      <VaultSetupPrompt
        recovery={vaultState.status === 'recovery'}
        defaultPath={defaultPath}
        busy={busy}
        error={error}
        onChooseFolder={handleChooseFolder}
        onUseDefault={handleUseDefault}
      />
    );
  }

  return (
    <div className="flex-1 flex flex-col min-h-0 overflow-hidden">
      {/* Hero Banner with Compact Stats */}
      <PageHeader
        title="Voice"
        highlightText="Notes"
        description="Everything you dictate, captured in one truthful history."
        glowColor="emerald"
        compact
      >
        <div className="flex items-center divide-x divide-border/60 bg-background/60 backdrop-blur-xs border border-border/80 rounded-lg py-1 px-1 shadow-2xs">
          <div className="px-3 py-0.5 text-center">
            <p className="text-[9px] font-mono uppercase tracking-widest text-muted-foreground">
              Notes
            </p>
            <p className="text-sm font-extrabold text-foreground font-mono">
              {stats.total}
            </p>
          </div>
          <div className="px-3 py-0.5 text-center">
            <p className="text-[9px] font-mono uppercase tracking-widest text-muted-foreground">
              Words
            </p>
            <p className="text-sm font-extrabold text-foreground font-mono">
              {stats.totalWords.toLocaleString()}
            </p>
          </div>
          <div className="px-3 py-0.5 text-center">
            <p className="text-[9px] font-mono uppercase tracking-widest text-muted-foreground">
              Today
            </p>
            <p className="text-sm font-extrabold text-foreground font-mono">
              {stats.notesToday}
            </p>
          </div>
        </div>
      </PageHeader>

      {/* Error Alert Banner */}
      {error && (
        <div className="flex items-center justify-between gap-2 p-3 rounded-lg bg-red-500/10 border border-red-500/30 text-xs text-red-600 dark:text-red-400 shrink-0 mb-3 shadow-xs">
          <div className="flex items-center gap-2">
            <AlertCircle className="w-4 h-4 shrink-0" />
            <span>{error}</span>
          </div>
          <Button
            size="sm"
            variant="ghost"
            onClick={() => setError('')}
            className="h-6 text-[10px] px-2 text-red-600 hover:text-red-700 dark:text-red-400"
          >
            Dismiss
          </Button>
        </div>
      )}

      {/* Main Transcript History Container */}
      <div className="flex-1 flex flex-col min-h-0 rounded-xl border border-border bg-card/85 backdrop-blur-xs p-5 shadow-xs">
        {/* Header Toolbar: Transcript History, View Mode Switcher, & Top Actions */}
        <div className="flex flex-wrap items-center justify-between gap-3 mb-4 shrink-0 pb-3 border-b border-border/60">
          <div className="flex items-center gap-3">
            <h2 className="text-sm font-bold text-foreground">Transcript History</h2>
            <Badge variant="outline" className="text-[10px] font-mono">
              {notes.length} voice note{notes.length === 1 ? '' : 's'}
            </Badge>

            {/* View Mode Switcher */}
            {notes.length > 0 && (
              <div className="flex items-center bg-muted/60 dark:bg-muted/30 p-0.5 rounded-lg border border-border/60 ml-2">
                <button
                  type="button"
                  onClick={() => setViewMode('grid')}
                  className={`p-1.5 rounded-md transition-all ${
                    viewMode === 'grid'
                      ? 'bg-background text-primary shadow-2xs font-semibold'
                      : 'text-muted-foreground hover:text-foreground'
                  }`}
                  title="Masonry Grid View (3 Columns)"
                  aria-label="Masonry Grid View"
                >
                  <LayoutGrid className="w-3.5 h-3.5" />
                </button>
                <button
                  type="button"
                  onClick={() => setViewMode('list')}
                  className={`p-1.5 rounded-md transition-all ${
                    viewMode === 'list'
                      ? 'bg-background text-primary shadow-2xs font-semibold'
                      : 'text-muted-foreground hover:text-foreground'
                  }`}
                  title="Compact List View"
                  aria-label="Compact List View"
                >
                  <List className="w-3.5 h-3.5" />
                </button>
              </div>
            )}
          </div>

          {notes.length > 0 && (
            <div className="flex items-center gap-2">
              {/* Active Selection Info Pill & Controls */}
              {selectedNoteIds.size > 0 && (
                <div className="flex items-center gap-2 animate-in fade-in duration-150">
                  <Badge variant="secondary" className="text-xs font-mono px-2.5 py-0.5 bg-primary/10 text-primary border border-primary/20">
                    {selectedNoteIds.size} selected
                  </Badge>
                  <Button
                    size="sm"
                    variant="ghost"
                    onClick={handleSelectAll}
                    disabled={actionBusy}
                    className="h-7 text-xs px-2 text-muted-foreground hover:text-foreground font-medium"
                  >
                    {selectedNoteIds.size === notes.length ? 'Deselect All' : 'Select All'}
                  </Button>
                  <Button
                    size="sm"
                    variant="ghost"
                    onClick={handleClearSelection}
                    disabled={actionBusy}
                    className="h-7 text-xs px-2 text-muted-foreground hover:text-foreground"
                  >
                    Clear
                  </Button>
                </div>
              )}

              {/* Top Merge Action */}
              <Button
                size="sm"
                variant={selectedNoteIds.size >= 2 ? 'default' : activeSelectionMode === 'merge' ? 'secondary' : 'outline'}
                onClick={() => {
                  if (selectedNoteIds.size >= 2) {
                    setIsMergingBatch(true);
                    setIsBulkDeleting(false);
                  } else {
                    setActiveSelectionMode((prev) => (prev === 'merge' ? null : 'merge'));
                    setIsMergingBatch(false);
                  }
                }}
                disabled={actionBusy}
                className={`h-7 text-xs gap-1.5 transition-all ${
                  selectedNoteIds.size >= 2
                    ? 'bg-primary text-primary-foreground font-semibold shadow-xs hover:bg-primary/90'
                    : activeSelectionMode === 'merge'
                    ? 'bg-primary/15 text-primary border-primary/30'
                    : ''
                }`}
                title={selectedNoteIds.size >= 2 ? `Merge ${selectedNoteIds.size} notes` : 'Select notes to merge'}
              >
                <GitMerge className="w-3.5 h-3.5" />
                <span>{selectedNoteIds.size >= 2 ? `Merge (${selectedNoteIds.size})` : 'Merge'}</span>
              </Button>

              {/* Top Delete Action */}
              <Button
                size="sm"
                variant={selectedNoteIds.size >= 1 ? 'destructive' : activeSelectionMode === 'delete' ? 'secondary' : 'outline'}
                onClick={() => {
                  if (selectedNoteIds.size >= 1) {
                    setIsBulkDeleting(true);
                    setIsMergingBatch(false);
                  } else {
                    setActiveSelectionMode((prev) => (prev === 'delete' ? null : 'delete'));
                    setIsBulkDeleting(false);
                  }
                }}
                disabled={actionBusy}
                className={`h-7 text-xs gap-1.5 transition-all ${
                  selectedNoteIds.size >= 1
                    ? 'font-semibold shadow-xs'
                    : activeSelectionMode === 'delete'
                    ? 'bg-red-500/15 text-red-600 dark:text-red-400 border-red-500/30'
                    : ''
                }`}
                title={selectedNoteIds.size >= 1 ? `Delete ${selectedNoteIds.size} note${selectedNoteIds.size > 1 ? 's' : ''}` : 'Select notes to delete'}
              >
                <Trash2 className="w-3.5 h-3.5" />
                <span>{selectedNoteIds.size >= 1 ? `Delete (${selectedNoteIds.size})` : 'Delete'}</span>
              </Button>
            </div>
          )}
        </div>

        {/* Merge Mode Guidance Banner */}
        {activeSelectionMode === 'merge' && selectedNoteIds.size < 2 && !isMergingBatch && (
          <div className="mb-4 p-3 rounded-xl bg-primary/10 border border-primary/25 flex items-center justify-between gap-2 text-xs text-primary shrink-0 animate-in fade-in duration-150">
            <div className="flex items-center gap-2">
              <GitMerge className="w-4 h-4 shrink-0" />
              <span>
                Click cards to select 2 or more Voice Notes to merge. Notes are ordered chronologically: oldest at the top, newest at the bottom.
              </span>
            </div>
            <Button
              size="sm"
              variant="ghost"
              onClick={() => setActiveSelectionMode(null)}
              className="h-6 text-xs px-2 text-primary hover:bg-primary/15"
            >
              Done
            </Button>
          </div>
        )}

        {/* Delete Mode Guidance Banner */}
        {activeSelectionMode === 'delete' && selectedNoteIds.size === 0 && !isBulkDeleting && (
          <div className="mb-4 p-3 rounded-xl bg-red-500/10 border border-red-500/25 flex items-center justify-between gap-2 text-xs text-red-600 dark:text-red-400 shrink-0 animate-in fade-in duration-150">
            <div className="flex items-center gap-2">
              <Trash2 className="w-4 h-4 shrink-0" />
              <span>Click cards to select one or more Voice Notes to delete.</span>
            </div>
            <Button
              size="sm"
              variant="ghost"
              onClick={() => setActiveSelectionMode(null)}
              className="h-6 text-xs px-2 text-red-600 dark:text-red-400 hover:bg-red-500/15"
            >
              Done
            </Button>
          </div>
        )}

        {/* Chronological Batch Merge Preview Banner */}
        {isMergingBatch && selectedNoteIds.size >= 2 && (
          <div className="mb-4 p-4 rounded-xl bg-gradient-to-br from-card via-card/95 to-card/90 border border-primary/30 shadow-md space-y-3 shrink-0 animate-in fade-in duration-150">
            <div className="flex items-center justify-between gap-2">
              <div className="flex items-center gap-2">
                <GitMerge className="w-4 h-4 text-primary shrink-0" />
                <span className="text-xs font-bold text-foreground">
                  Merge {selectedNoteIds.size} Voice Notes Chronologically
                </span>
              </div>
              <div className="flex items-center gap-2">
                <Button
                  size="sm"
                  variant="ghost"
                  disabled={actionBusy}
                  onClick={() => setIsMergingBatch(false)}
                  className="h-7 text-xs"
                >
                  Cancel
                </Button>
                <Button
                  size="sm"
                  variant="default"
                  disabled={actionBusy}
                  onClick={handleBatchMerge}
                  className="h-7 text-xs gap-1.5 font-semibold bg-primary text-primary-foreground shadow-xs hover:bg-primary/90"
                >
                  <GitMerge className="w-3.5 h-3.5" />
                  <span>Confirm Merge ({selectedNoteIds.size})</span>
                </Button>
              </div>
            </div>
            <p className="text-[11px] text-muted-foreground leading-relaxed">
              The selected notes will be combined in chronological order: the oldest note is first at the top, and the newest note is at the bottom.
            </p>
            <div className="flex flex-col gap-1.5 max-h-36 overflow-y-auto pr-1">
              {notes
                .filter((n) => selectedNoteIds.has(n.id))
                .sort((a, b) => a.created_at.localeCompare(b.created_at))
                .map((n, idx, arr) => (
                  <div
                    key={n.id}
                    className="flex items-center gap-2 text-xs bg-muted/40 dark:bg-muted/15 p-2 rounded-lg border border-border/50"
                  >
                    <span className="font-mono text-[10px] text-muted-foreground shrink-0 w-6">
                      #{idx + 1}
                    </span>
                    <span className="font-mono text-[10px] text-primary shrink-0">
                      {formatNoteTimestamp(n.created_at)}
                    </span>
                    <span className="truncate text-foreground/85 flex-1 font-sans text-xs">
                      {n.content}
                    </span>
                    {idx === 0 && (
                      <Badge variant="outline" className="text-[9px] font-mono py-0 px-1 bg-primary/10 text-primary border-primary/20 shrink-0">
                        Oldest (Top)
                      </Badge>
                    )}
                    {idx === arr.length - 1 && (
                      <Badge variant="outline" className="text-[9px] font-mono py-0 px-1 bg-primary/10 text-primary border-primary/20 shrink-0">
                        Newest (Bottom)
                      </Badge>
                    )}
                  </div>
                ))}
            </div>
          </div>
        )}

        {/* Bulk or Single Delete Banner */}
        {isBulkDeleting && selectedNoteIds.size > 0 && (
          <div className="mb-4 p-4 rounded-xl bg-red-500/10 border border-red-500/30 flex flex-wrap items-center justify-between gap-3 text-xs text-red-600 dark:text-red-400 shrink-0 animate-in fade-in duration-150 shadow-xs">
            <div className="flex items-center gap-2">
              <Trash2 className="w-4 h-4 shrink-0" />
              <span className="font-medium">
                Move {selectedNoteIds.size} selected Voice Note{selectedNoteIds.size === 1 ? '' : 's'} to Trash? (Kept for 30 days before permanent deletion)
              </span>
            </div>
            <div className="flex items-center gap-2">
              <Button
                size="sm"
                variant="ghost"
                disabled={actionBusy}
                onClick={() => setIsBulkDeleting(false)}
                className="h-7 text-xs"
              >
                Cancel
              </Button>
              <Button
                size="sm"
                variant="destructive"
                disabled={actionBusy}
                onClick={handleBulkDelete}
                className="h-7 text-xs gap-1.5 font-semibold shadow-xs"
              >
                <Trash2 className="w-3.5 h-3.5" />
                <span>Move {selectedNoteIds.size} to Trash</span>
              </Button>
            </div>
          </div>
        )}

        {notes.length === 0 ? (
          <EmptyState
            icon={Mic}
            title="No Voice Notes yet"
            description="Everything you dictate with Vox will show up here."
            minHeight="min-h-[220px]"
            className="flex-1"
          />
        ) : (
          /* Date-based Accordions (Masonry Grid & Compact List) */
          <div className="flex-1 overflow-y-auto space-y-6 pr-1">
            {dateGroups.map((group) => {
              const isExpanded = expandedSections.has(group.key);
              const displayNotes = group.notes;

              return (
                <div key={group.key} className="space-y-3">
                  {/* Date Accordion Header */}
                  <button
                    type="button"
                    onClick={() => toggleSection(group.key)}
                    className="w-full flex items-center justify-between py-1.5 px-1 group/header text-left select-none transition-colors"
                    aria-expanded={isExpanded}
                  >
                    <div className="flex items-center gap-2.5">
                      <div className="w-5 h-5 rounded-md flex items-center justify-center text-muted-foreground group-hover/header:text-foreground transition-transform">
                        {isExpanded ? (
                          <ChevronDown className="w-4 h-4 text-primary" />
                        ) : (
                          <ChevronRight className="w-4 h-4" />
                        )}
                      </div>
                      <span className="text-xs md:text-sm font-bold text-foreground font-sans tracking-tight">
                        {group.label}
                      </span>
                      <Badge variant="outline" className="text-[10px] font-mono px-1.5 py-0 bg-muted/30">
                        {group.notes.length} note{group.notes.length === 1 ? '' : 's'}
                      </Badge>
                      <span className="text-[10px] font-mono text-muted-foreground/80">
                        · {group.totalWords.toLocaleString()} words
                      </span>
                    </div>
                    <div className="h-[1px] flex-1 bg-border/40 mx-4" />
                    <span className="text-[10px] font-mono text-muted-foreground opacity-0 group-hover/header:opacity-100 transition-opacity">
                      {isExpanded ? 'Collapse' : 'Expand'}
                    </span>
                  </button>

                  {isExpanded && (
                    <div className="space-y-3 animate-in fade-in duration-150">
                      {/* VIEW 1: MASONRY GRID */}
                      {viewMode === 'grid' && (
                        <div className="columns-1 md:columns-2 lg:columns-3 gap-3.5 [column-fill:_balance]">
                          {displayNotes.map((note) => {
                            const isEditing = editingNoteId === note.id;
                            const isSelected = selectedNoteIds.has(note.id);

                            return (
                              <div
                                key={note.id}
                                onClick={(e) => handleCardClick(note.id, e)}
                                className={`rounded-xl border transition-all duration-200 group flex flex-col justify-between p-4 select-none relative cursor-pointer break-inside-avoid mb-3.5 w-full ${
                                  isSelected
                                    ? 'border-primary/60 bg-primary/10 dark:bg-primary/15 ring-2 ring-primary/30 shadow-sm dark:shadow-primary/10'
                                    : 'border-border/80 bg-card/90 dark:bg-card/50 hover:border-border hover:bg-muted/15 dark:hover:bg-muted/10 hover:shadow-md hover:-translate-y-0.5'
                                }`}
                              >
                                <div className="flex items-center justify-between gap-2 mb-2.5 shrink-0">
                                  <div className="flex items-center gap-1.5 flex-wrap min-w-0">
                                    <button
                                      type="button"
                                      role="checkbox"
                                      aria-checked={isSelected}
                                      aria-label={`Select voice note from ${formatNoteTimestamp(note.created_at)}`}
                                      onClick={(e) => {
                                        e.stopPropagation();
                                        toggleSelectNote(note.id);
                                      }}
                                      className={`w-4 h-4 rounded-md border flex items-center justify-center transition-all ${
                                        isSelected
                                          ? 'bg-primary border-primary text-primary-foreground shadow-2xs'
                                          : 'border-border/80 bg-background/60 hover:border-primary/50 text-transparent'
                                      }`}
                                    >
                                      <Check className="w-2.5 h-2.5 stroke-[3]" />
                                    </button>

                                    <span className="text-[11px] font-semibold text-foreground font-mono truncate">
                                      {formatNoteTimestamp(note.created_at)}
                                    </span>
                                    <Badge variant="outline" className="text-[9px] font-mono px-1 py-0 bg-muted/40 text-muted-foreground shrink-0">
                                      {countWords(note.content)}w
                                    </Badge>
                                  </div>

                                  <div className="flex items-center gap-1 shrink-0">
                                    {hasDiff(note) && (
                                      <button
                                        type="button"
                                        onClick={(e) => toggleDiffNote(note.id, e)}
                                        className={`text-[9px] font-mono px-1.5 py-0.5 rounded border transition-colors ${
                                          diffNoteIds.has(note.id)
                                            ? 'bg-primary text-primary-foreground border-primary font-semibold shadow-2xs'
                                            : 'bg-muted/40 text-muted-foreground border-border/80 hover:bg-muted hover:text-foreground'
                                        }`}
                                        title={diffNoteIds.has(note.id) ? 'Hide diff' : 'Show diff against raw transcript'}
                                      >
                                        Diff
                                      </button>
                                    )}
                                    {note.merged_from && note.merged_from.length > 0 && (
                                      <Badge variant="outline" className="text-[9px] font-mono px-1 py-0 bg-primary/10 text-primary border-primary/25 gap-1">
                                        <GitMerge className="w-2.5 h-2.5" />
                                        <span>Merged · {note.merged_from.length} Voice Notes</span>
                                      </Badge>
                                    )}
                                    {promotedNoteIds.has(note.id) && (
                                      <Badge variant="outline" className="text-[9px] font-mono px-1 py-0 bg-primary/10 text-primary border-primary/25 gap-1">
                                        <Sparkles className="w-2.5 h-2.5" />
                                        <span>SCRIBBLE</span>
                                      </Badge>
                                    )}
                                  </div>
                                </div>

                                <div className="flex-1 min-h-0 relative my-1 select-text">
                                  {isEditing ? (
                                    renderInlineEdit(note)
                                  ) : (
                                    renderNoteBody(note, true)
                                  )}

                                  {renderPhraseCorrectionPopover(note)}
                                  {renderUndoNotification(note)}
                                  {renderUnmergeBanner(note)}
                                </div>

                                {!isEditing && (
                                  <div
                                    className="flex items-center justify-between pt-2 mt-auto border-t border-border/50 shrink-0"
                                    onClick={(e) => e.stopPropagation()}
                                  >
                                    <span className="text-[10px] text-muted-foreground font-mono">
                                      {new Date(note.created_at).toLocaleTimeString([], { hour: 'numeric', minute: '2-digit' })}
                                    </span>
                                    {renderNoteActions(note)}
                                  </div>
                                )}
                              </div>
                            );
                          })}
                        </div>
                      )}

                      {/* VIEW 2: COMPACT LIST */}
                      {viewMode === 'list' && (
                        <div className="space-y-2">
                          {displayNotes.map((note) => {
                            const isEditing = editingNoteId === note.id;
                            const isSelected = selectedNoteIds.has(note.id);

                            return (
                              <div
                                key={note.id}
                                onClick={(e) => handleCardClick(note.id, e)}
                                className={`rounded-xl border transition-all duration-150 p-3 select-none relative cursor-pointer ${
                                  isSelected
                                    ? 'border-primary/60 bg-primary/10 ring-1 ring-primary/30 shadow-2xs'
                                    : 'border-border/70 bg-card/70 hover:border-border hover:bg-muted/15'
                                }`}
                              >
                                <div className="flex flex-col md:flex-row md:items-start justify-between gap-3">
                                  <div className="flex items-start gap-2.5 flex-1 min-w-0">
                                    <button
                                      type="button"
                                      role="checkbox"
                                      aria-checked={isSelected}
                                      aria-label={`Select voice note from ${formatNoteTimestamp(note.created_at)}`}
                                      onClick={(e) => {
                                        e.stopPropagation();
                                        toggleSelectNote(note.id);
                                      }}
                                      className={`w-4 h-4 mt-0.5 rounded-md border flex items-center justify-center transition-all shrink-0 ${
                                        isSelected
                                          ? 'bg-primary border-primary text-primary-foreground'
                                          : 'border-border bg-background/60 hover:border-primary/50 text-transparent'
                                      }`}
                                    >
                                      <Check className="w-2.5 h-2.5 stroke-[3]" />
                                    </button>

                                    <div className="flex-1 min-w-0 select-text">
                                      <div className="flex items-center gap-2 mb-1 flex-wrap">
                                        <span className="text-[11px] font-semibold text-foreground font-mono">
                                          {formatNoteTimestamp(note.created_at)}
                                        </span>
                                        <Badge variant="outline" className="text-[9px] font-mono px-1 py-0">
                                          {countWords(note.content)}w
                                        </Badge>
                                        {hasDiff(note) && (
                                          <button
                                            type="button"
                                            onClick={(e) => toggleDiffNote(note.id, e)}
                                            className={`text-[9px] font-mono px-1.5 py-0.5 rounded border transition-colors ${
                                              diffNoteIds.has(note.id)
                                                ? 'bg-primary text-primary-foreground border-primary font-semibold shadow-2xs'
                                                : 'bg-muted/40 text-muted-foreground border-border/80 hover:bg-muted hover:text-foreground'
                                            }`}
                                            title={diffNoteIds.has(note.id) ? 'Hide diff' : 'Show diff against raw transcript'}
                                          >
                                            Diff
                                          </button>
                                        )}
                                        {note.merged_from && note.merged_from.length > 0 && (
                                          <Badge variant="outline" className="text-[9px] font-mono px-1 py-0 bg-primary/10 text-primary border-primary/25 gap-1">
                                            <GitMerge className="w-2.5 h-2.5" />
                                            <span>Merged · {note.merged_from.length} Voice Notes</span>
                                          </Badge>
                                        )}
                                        {promotedNoteIds.has(note.id) && (
                                          <Badge variant="outline" className="text-[9px] font-mono px-1 py-0 bg-primary/10 text-primary border-primary/25 gap-1">
                                            <Sparkles className="w-2.5 h-2.5" />
                                            <span>SCRIBBLE</span>
                                          </Badge>
                                        )}
                                      </div>

                                      {isEditing ? (
                                        renderInlineEdit(note)
                                      ) : (
                                        renderNoteBody(note, false)
                                      )}

                                      {renderPhraseCorrectionPopover(note)}
                                      {renderUndoNotification(note)}
                                      {renderUnmergeBanner(note)}
                                    </div>
                                  </div>

                                  {!isEditing && renderNoteActions(note)}
                                </div>
                              </div>
                            );
                          })}
                        </div>
                      )}
                    </div>
                  )}
                </div>
              );
            })}
          </div>
        )}
      </div>
    </div>
  );
};
