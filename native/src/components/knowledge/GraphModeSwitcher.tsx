import React from 'react';
import { CircleDot, GitBranch, Share2, Workflow } from 'lucide-react';

import {
  GRAPH_VIEW_MODES,
  GRAPH_VIEW_MODE_LABELS,
  IMPLEMENTED_GRAPH_VIEW_MODES,
  type GraphViewMode,
} from './graph/graphTypes';

const MODE_ICONS: Record<GraphViewMode, React.ComponentType<{ className?: string }>> = {
  rings: CircleDot,
  focus: Workflow,
  decisions: GitBranch,
  force: Share2,
};

const MODE_HINTS: Record<GraphViewMode, string> = {
  rings: 'PARA as concentric bands — where work sits',
  focus: 'One root, work branching outward — how you got here',
  decisions: 'What was chosen, and why',
  force: 'Free-floating force layout — what clusters with what',
};

interface GraphModeSwitcherProps {
  mode: GraphViewMode;
  onChange: (mode: GraphViewMode) => void;
}

/**
 * Picks which view the graph surface renders.
 *
 * A segmented control rather than a dropdown: there are four modes, they are
 * switched often, and the order is fixed and meaningful — Rings is where you
 * land, Force is where the old view went.
 */
export const GraphModeSwitcher: React.FC<GraphModeSwitcherProps> = ({ mode, onChange }) => (
  <div
    role="tablist"
    aria-label="Graph view mode"
    className="flex items-center gap-0.5 bg-background/60 backdrop-blur-xs border border-border/80 rounded-lg p-1 shadow-2xs"
  >
    {GRAPH_VIEW_MODES.filter((m) => IMPLEMENTED_GRAPH_VIEW_MODES.includes(m)).map((candidate) => {
      const Icon = MODE_ICONS[candidate];
      const isActive = candidate === mode;
      return (
        <button
          key={candidate}
          type="button"
          role="tab"
          aria-selected={isActive}
          title={MODE_HINTS[candidate]}
          onClick={() => onChange(candidate)}
          className={`flex items-center gap-1.5 h-7 px-2.5 rounded-md text-xs font-medium transition-colors ${
            isActive
              ? 'bg-accent text-accent-foreground'
              : 'text-muted-foreground hover:bg-accent/50 hover:text-foreground'
          }`}
        >
          <Icon className="w-3.5 h-3.5" />
          <span>{GRAPH_VIEW_MODE_LABELS[candidate]}</span>
        </button>
      );
    })}
  </div>
);
