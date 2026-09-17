import {
  GraphFiltersSettings,
  GraphGroup,
  GraphDisplaySettings,
  GraphForcesSettings,
  GraphPositionMap,
} from '../../../types';
import {
  DEFAULT_FILTERS,
  DEFAULT_DISPLAY,
  DEFAULT_FORCES,
  DEFAULT_GRAPH_VIEW_MODE,
  IMPLEMENTED_GRAPH_VIEW_MODES,
  GraphViewMode,
} from './graphTypes';

const SETTINGS_STORAGE_KEY = 'relay_knowledge_graph_settings_v2';
const POSITIONS_STORAGE_KEY = 'relay_knowledge_graph_positions_v1';
const VIEW_MODE_STORAGE_KEY = 'relay_knowledge_graph_view_mode_v1';

export interface StoredGraphSettings {
  filters: GraphFiltersSettings;
  groups: GraphGroup[];
  display: GraphDisplaySettings;
  forces: GraphForcesSettings;
}

/**
 * Load persisted graph settings from LocalStorage
 */
export function loadGraphSettings(): StoredGraphSettings {
  try {
    const raw = localStorage.getItem(SETTINGS_STORAGE_KEY);
    if (raw) {
      const parsed = JSON.parse(raw);
      return {
        filters: { ...DEFAULT_FILTERS, ...(parsed.filters || {}) },
        groups: Array.isArray(parsed.groups) ? parsed.groups : [],
        display: { ...DEFAULT_DISPLAY, ...(parsed.display || {}) },
        forces: { ...DEFAULT_FORCES, ...(parsed.forces || {}) },
      };
    }
  } catch (err) {
    console.warn('Failed to parse saved graph settings:', err);
  }

  return {
    filters: DEFAULT_FILTERS,
    groups: [],
    display: DEFAULT_DISPLAY,
    forces: DEFAULT_FORCES,
  };
}

/**
 * Save graph settings to LocalStorage
 */
export function saveGraphSettings(settings: StoredGraphSettings): boolean {
  try {
    localStorage.setItem(SETTINGS_STORAGE_KEY, JSON.stringify(settings));
    return true;
  } catch (err) {
    console.error('Failed to save graph settings:', err);
    return false;
  }
}

/**
 * Load persisted node coordinates
 */
export function loadNodePositions(): GraphPositionMap {
  try {
    const raw = localStorage.getItem(POSITIONS_STORAGE_KEY);
    if (raw) {
      const parsed = JSON.parse(raw);
      if (parsed && typeof parsed === 'object') {
        return parsed;
      }
    }
  } catch (err) {
    console.warn('Failed to parse saved node positions:', err);
  }
  return {};
}

/**
 * Save node coordinates to LocalStorage
 */
export function saveNodePositions(positions: GraphPositionMap): boolean {
  try {
    localStorage.setItem(POSITIONS_STORAGE_KEY, JSON.stringify(positions));
    return true;
  } catch (err) {
    console.error('Failed to save node positions:', err);
    return false;
  }
}

/**
 * Reset node positions (clears saved coordinates)
 */
export function clearNodePositions(): void {
  try {
    localStorage.removeItem(POSITIONS_STORAGE_KEY);
  } catch (err) {
    console.error('Failed to clear node positions:', err);
  }
}

/**
 * The view mode to open the graph in.
 *
 * Stored separately from the force view's settings blob so that choosing a
 * mode survives independently of the physics knobs — and so an unreadable
 * settings blob cannot strand the user in a mode they did not pick. An
 * unrecognised stored value falls back to Rings rather than rendering
 * nothing.
 */
export function loadGraphViewMode(): GraphViewMode {
  try {
    const raw = localStorage.getItem(VIEW_MODE_STORAGE_KEY);
    // A mode that is no longer offered (or was never implemented) falls
    // back rather than stranding the user on a blank surface.
    if (raw && (IMPLEMENTED_GRAPH_VIEW_MODES as readonly string[]).includes(raw)) {
      return raw as GraphViewMode;
    }
  } catch (err) {
    console.warn('Failed to read the saved graph view mode:', err);
  }
  return DEFAULT_GRAPH_VIEW_MODE;
}

export function saveGraphViewMode(mode: GraphViewMode): boolean {
  try {
    localStorage.setItem(VIEW_MODE_STORAGE_KEY, mode);
    return true;
  } catch (err) {
    console.error('Failed to save the graph view mode:', err);
    return false;
  }
}
