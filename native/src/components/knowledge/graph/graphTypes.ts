import {
  KnowledgeNode,
  KnowledgeEdge,
  GraphFiltersSettings,
  GraphGroup,
  GraphDisplaySettings,
  GraphForcesSettings,
  LocalGraphSettings,
} from '../../../types';

export interface SimNode extends KnowledgeNode {
  x: number;
  y: number;
  vx: number;
  vy: number;
  radius: number;
  color: string;
  isPinned?: boolean;
  opacity?: number;
  createdAtTimestamp?: number;
}

export interface SimEdge extends KnowledgeEdge {
  isExplicit?: boolean;
}

export interface CameraState {
  x: number;
  y: number;
  k: number;
}

/**
 * The graph surface's view modes, in the order the switcher shows them.
 *
 * `force` is the original force-directed view, kept reachable rather than
 * deleted: it is the only mode that answers "what is near what" without
 * first asking how the vault is filed.
 */
export const GRAPH_VIEW_MODES = ['rings', 'focus', 'decisions', 'force'] as const;

export type GraphViewMode = (typeof GRAPH_VIEW_MODES)[number];

export const DEFAULT_GRAPH_VIEW_MODE: GraphViewMode = 'rings';

/**
 * The modes the switcher actually offers.
 *
 * Separate from `GRAPH_VIEW_MODES`, which is the fixed vocabulary and the
 * order they appear in. A mode joins this list when its view exists —
 * showing a tab that renders nothing is the ghost UI `rules/ui-components.md`
 * forbids, and it reads to the user as a feature that is broken rather than
 * one that is coming.
 */
export const IMPLEMENTED_GRAPH_VIEW_MODES: readonly GraphViewMode[] = ['rings', 'force'];

export const GRAPH_VIEW_MODE_LABELS: Record<GraphViewMode, string> = {
  rings: 'Rings',
  focus: 'Focus & Flow',
  decisions: 'Decision Tree',
  force: 'Force',
};

export const RELAY_COLOR_MAP: Record<string, string> = {
  scribble: '#3b82f6',     // Electric blue (Scribble)
  voice_note: '#ec4899',   // Vibrant pink (Voice Note)
  topic: '#f59e0b',        // Warm amber (Topic cluster)
  entity: '#10b981',       // Emerald green (Named Entity)
  person: '#8b5cf6',       // Purple (Person)
  organization: '#14b8a6', // Teal (Organization)
  place: '#f97316',        // Orange (Place)
  project: '#06b6d4',      // Cyan (Project)
  source: '#64748b',       // Slate (Source/Attachment)
  file: '#64748b',         // Slate (File)
  document: '#0284c7',     // Sky (Document)
  task: '#84cc16',         // Lime (Task)
  meeting: '#a855f7',      // Fuchsia (Meeting)
  unresolved: '#6b7280',   // Gray (Unresolved)
  default: '#94a3b8',      // Slate default
};

/**
 * Edge colour by relationship type.
 *
 * Vox stores a typed relationship on every link and the old renderer threw
 * it away, colouring by a single "is this derived" boolean. These keys cover
 * both vocabularies that reach the graph: `relationships/model.rs`'s
 * snake_case types and the SCREAMING_CASE constants scribble frontmatter
 * carries. Lookup is case-insensitive via `edgeColorFor`, so both arrive at
 * the same colour.
 */
export const RELATIONSHIP_COLOR_MAP: Record<string, string> = {
  derived_from: '#c084fc',  // Purple — provenance
  summarizes: '#38bdf8',    // Sky — condensation
  analyses: '#2dd4bf',      // Teal — examination
  references: '#94a3b8',    // Slate — a citation, deliberately quiet
  belongs_to: '#22c55e',    // Green — containment
  supersedes: '#f97316',    // Orange — replacement
  related_to: '#64748b',    // Slate — the unspecific default
  mentions: '#10b981',      // Emerald — matches the entity node colour
  same_topic: '#f59e0b',    // Amber — matches the topic node colour
  same_project: '#06b6d4',  // Cyan — matches the project node colour
  contradicts: '#ef4444',   // Red — disagreement
  extends: '#8b5cf6',       // Violet — elaboration
};

/**
 * The colour for a relationship, whichever spelling it arrives in.
 *
 * An unknown type falls back to the neutral `related_to` slate rather than
 * being dropped or given an invented hue — an edge whose meaning we cannot
 * read should look unremarkable, not distinctive.
 */
export function edgeColorFor(relationship: string | undefined | null): string {
  if (!relationship) return RELATIONSHIP_COLOR_MAP.related_to;
  return (
    RELATIONSHIP_COLOR_MAP[relationship.toLowerCase()] ?? RELATIONSHIP_COLOR_MAP.related_to
  );
}

/**
 * The hue reserved for suggested links.
 *
 * Deliberately outside `RELATIONSHIP_COLOR_MAP`: a guess must not be
 * mistakable for any real relationship type, so it gets a hue no real edge
 * can take, plus a dash pattern.
 */
export const GHOST_EDGE_COLOR = '#a78bfa'; // Soft violet

/**
 * Whether an edge is a guess rather than a link that exists.
 *
 * Vox already records this and the old renderer discarded it. `source` is
 * `user` for a link the person drew, `system` for one the graph derives
 * deterministically from stored fields (a scribble to its topic, to its
 * entity, to the recording it came from), and `ai` for one enrichment
 * proposed from shared topics — a similarity guess nobody confirmed. Only
 * the last of those is a suggestion.
 */
export function isSuggestedEdge(edge: { source?: string | null }): boolean {
  return (edge.source ?? '').toLowerCase() === 'ai';
}

export const PRESET_GROUP_COLORS = [
  '#ef4444', // Red
  '#f97316', // Orange
  '#eab308', // Yellow
  '#22c55e', // Green
  '#06b6d4', // Cyan
  '#3b82f6', // Blue
  '#8b5cf6', // Purple
  '#ec4899', // Pink
  '#14b8a6', // Teal
  '#f43f5e', // Rose
];

export const DEFAULT_FILTERS: GraphFiltersSettings = {
  searchQuery: '',
  showScribbles: true,
  showVoiceNotes: true,
  showTags: true, // Topics
  showEntities: true,
  showAttachments: true,
  existingFilesOnly: false,
  showOrphans: true,
  showUnresolved: true,
};

export const DEFAULT_DISPLAY: GraphDisplaySettings = {
  showArrows: false,
  textFadeThreshold: 0.6,
  nodeSizeMultiplier: 1.0,
  linkThickness: 1.0,
};

export const DEFAULT_FORCES: GraphForcesSettings = {
  centerForce: 0.40,
  repelForce: 10.0,
  linkForce: 0.60,
  linkDistance: 130,
};

export const DEFAULT_LOCAL_GRAPH: LocalGraphSettings = {
  enabled: false,
  rootNodeId: null,
  depth: 1,
};
