import React from 'react';
import {
  Sparkles,
  RefreshCw,
  X,
  Pencil,
  Save,
  AlertTriangle,
  Network,
} from 'lucide-react';

import { Button } from '@/components/ui/button';
import { Badge } from '@/components/ui/badge';
import { MarkdownView } from '@/components/common/MarkdownView';
import type {
  MeetingSummary,
  MeetingTemplate,
  SummaryProgress,
} from '@/types/meetings';
import { summaryIsRunning } from '@/lib/meetings';

interface MeetingSummaryPanelProps {
  summary: MeetingSummary | null;
  templates: MeetingTemplate[];
  templateId: string;
  onTemplateChange: (id: string) => void;
  progress: SummaryProgress | null;
  hasTranscript: boolean;
  onGenerate: (force: boolean) => void;
  onCancel: () => void;
  onSave: (markdown: string) => void;
  onPromote: () => void;
  /** Opens Settings › AI Models & STT, when the failure is a setting. */
  onOpenProviderSettings?: () => void;
}

/**
 * Whether a failure is "nothing is configured" rather than "the model failed".
 *
 * The backend distinguishes these with its own error code; the report record
 * carries only the message, so the marker is the sentence itself. Both of the
 * app's routes to a fix are named in every `ProviderUnavailable` message, and
 * a test on the Rust side keeps that true.
 */
function isConfigurationProblem(error: string): boolean {
  return error.includes('Settings') || error.includes('ollama.com');
}

/**
 * The report side of a meeting: choose a template, generate, read, edit.
 *
 * A run in flight shows its stage rather than a spinner, because a local model
 * summarising a long meeting takes minutes and "Generating…" for four minutes
 * is indistinguishable from a hang.
 */
export const MeetingSummaryPanel: React.FC<MeetingSummaryPanelProps> = ({
  summary,
  templates,
  templateId,
  onTemplateChange,
  progress,
  hasTranscript,
  onGenerate,
  onCancel,
  onSave,
  onPromote,
  onOpenProviderSettings,
}) => {
  const [editing, setEditing] = React.useState(false);
  const [draft, setDraft] = React.useState('');

  const running = summaryIsRunning(summary) || summaryIsRunning(
    progress ? ({ status: progress.status } as MeetingSummary) : null,
  );
  const markdown = summary?.markdown?.trim() ?? '';

  React.useEffect(() => {
    // A run that finishes while the user is editing must not overwrite what
    // they have typed.
    if (!editing) setDraft(markdown);
  }, [markdown, editing]);

  const startEditing = () => {
    setDraft(markdown);
    setEditing(true);
  };

  const save = () => {
    onSave(draft);
    setEditing(false);
  };

  return (
    <div className="flex flex-col min-h-0 h-full">
      <div className="flex flex-wrap items-center gap-2 mb-3 shrink-0">
        <select
          value={templateId}
          onChange={(event) => onTemplateChange(event.target.value)}
          disabled={running}
          aria-label="Summary template"
          className="h-8 rounded-lg border border-border bg-card px-2 text-xs text-foreground disabled:opacity-50"
        >
          {templates.map((template) => (
            <option key={template.id} value={template.id}>
              {template.name}
              {template.custom ? ' (yours)' : ''}
            </option>
          ))}
        </select>

        {running ? (
          <Button variant="outline" size="sm" onClick={onCancel} className="gap-1.5">
            <X className="w-3.5 h-3.5" />
            Cancel
          </Button>
        ) : (
          <Button
            size="sm"
            onClick={() => onGenerate(markdown.length > 0)}
            disabled={!hasTranscript}
            className="gap-1.5"
          >
            {markdown ? <RefreshCw className="w-3.5 h-3.5" /> : <Sparkles className="w-3.5 h-3.5" />}
            {markdown ? 'Regenerate' : 'Generate'}
          </Button>
        )}

        {markdown && !running && !editing && (
          <>
            <Button variant="outline" size="sm" onClick={startEditing} className="gap-1.5">
              <Pencil className="w-3.5 h-3.5" />
              Edit
            </Button>
            <Button variant="outline" size="sm" onClick={onPromote} className="gap-1.5">
              <Network className="w-3.5 h-3.5" />
              Add to graph
            </Button>
          </>
        )}
        {editing && (
          <Button size="sm" onClick={save} className="gap-1.5">
            <Save className="w-3.5 h-3.5" />
            Save
          </Button>
        )}
      </div>

      {running && (
        <div className="mb-3 shrink-0">
          <p className="text-xs text-muted-foreground mb-1.5">
            {progress?.stage ?? 'Working…'}
          </p>
          <div className="h-1 rounded-full bg-muted overflow-hidden">
            <div
              className="h-full rounded-full bg-primary transition-[width] duration-500"
              style={{
                width:
                  progress?.fraction != null ? `${Math.round(progress.fraction * 100)}%` : '20%',
              }}
            />
          </div>
        </div>
      )}

      {summary?.status === 'failed' && summary.error && (
        <div
          className={`flex items-start gap-2 text-xs mb-3 shrink-0 ${
            isConfigurationProblem(summary.error) ? 'text-foreground' : 'text-destructive'
          }`}
        >
          <AlertTriangle
            className={`w-3.5 h-3.5 shrink-0 mt-0.5 ${
              isConfigurationProblem(summary.error) ? 'text-amber-500' : ''
            }`}
          />
          <span>
            {summary.error}
            {markdown && ' The previous report is still shown below.'}
            {isConfigurationProblem(summary.error) && onOpenProviderSettings && (
              <>
                {' '}
                <button
                  type="button"
                  onClick={onOpenProviderSettings}
                  className="underline underline-offset-2 hover:text-primary"
                >
                  Open settings
                </button>
              </>
            )}
          </span>
        </div>
      )}
      {summary?.status === 'cancelled' && (
        <p className="text-xs text-muted-foreground mb-3 shrink-0">
          Generation was cancelled.{markdown && ' The previous report is unchanged.'}
        </p>
      )}

      <div className="flex-1 min-h-0 overflow-y-auto pr-1">
        {editing ? (
          <textarea
            value={draft}
            onChange={(event) => setDraft(event.target.value)}
            aria-label="Meeting report"
            className="w-full h-full min-h-[300px] rounded-lg border border-border bg-card p-3 text-sm font-mono text-foreground resize-none focus:outline-hidden focus:ring-2 focus:ring-primary/40"
          />
        ) : markdown ? (
          <MarkdownView content={markdown} />
        ) : (
          <div className="py-8 text-center">
            <p className="text-xs text-muted-foreground">
              {hasTranscript
                ? 'No report yet. Pick a template and generate one.'
                : 'A report needs a transcript first.'}
            </p>
          </div>
        )}
      </div>

      {summary?.model && !editing && (
        <div className="flex flex-wrap items-center gap-2 mt-3 shrink-0">
          <Badge variant="outline" className="text-[10px] font-mono">
            {summary.model}
          </Badge>
          {summary.chunk_count > 1 && (
            <Badge variant="outline" className="text-[10px] font-mono">
              {summary.chunk_count} parts
            </Badge>
          )}
          {summary.processing_ms > 0 && (
            <span className="text-[10px] text-muted-foreground font-mono">
              {(summary.processing_ms / 1000).toFixed(1)}s
            </span>
          )}
        </div>
      )}
    </div>
  );
};
