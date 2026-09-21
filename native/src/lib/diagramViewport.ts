/**
 * The arithmetic behind panning and zooming a diagram.
 *
 * Kept apart from the component because every rule here is a calculation over
 * two rectangles, and jsdom has no layout — a test that rendered the component
 * would measure zeroes and prove nothing. These functions take sizes as
 * numbers and are tested directly.
 *
 * The problem they exist to solve: a summary's flowchart can be three times
 * wider than the panel it lands in. Scaling it to fit produces a picture of a
 * diagram rather than a diagram — technically complete, practically
 * unreadable — so past a point it is better to show part of it at a legible
 * size and let the reader move around.
 */

export interface Size {
  width: number;
  height: number;
}

export interface Pan {
  x: number;
  y: number;
}

/** Smallest zoom offered. Below this even a large diagram is a grey smudge. */
export const MIN_SCALE = 0.2;

/** Largest zoom offered. Past this the SVG is bigger than any useful detail. */
export const MAX_SCALE = 4;

/**
 * The point below which fitting stops being worth it.
 *
 * A diagram shrunk past roughly half size has labels smaller than body text,
 * which is the complaint this module answers. Below this, the viewport keeps
 * the diagram readable and becomes scrollable instead of shrinking further.
 */
export const MIN_LEGIBLE_SCALE = 0.55;

/** One press of a zoom button, or one notch of a zooming wheel. */
export const ZOOM_STEP = 1.25;

/** One arrow-key press, in viewport pixels. */
export const PAN_STEP = 64;

/** One arrow-key press with shift held — a screenful rather than a nudge. */
export const PAN_STEP_LARGE = 256;

export const NO_PAN: Pan = { x: 0, y: 0 };

/**
 * The diagram's own size, from the markup rather than from the DOM.
 *
 * `viewBox` first because mermaid always emits one and it is the authority on
 * the drawing's coordinates; `width`/`height` are a fallback, and may carry a
 * unit or a percentage, which is why anything non-finite or non-positive is
 * rejected rather than coerced.
 */
export const parseSvgSize = (markup: string): Size | null => {
  const viewBox = markup.match(/viewBox\s*=\s*["']\s*([-\d.]+)[\s,]+([-\d.]+)[\s,]+([\d.]+)[\s,]+([\d.]+)\s*["']/i);
  if (viewBox) {
    const width = Number(viewBox[3]);
    const height = Number(viewBox[4]);
    if (isUsable(width) && isUsable(height)) {
      return { width, height };
    }
  }

  const width = Number(markup.match(/\swidth\s*=\s*["']([\d.]+)(?:px)?["']/i)?.[1]);
  const height = Number(markup.match(/\sheight\s*=\s*["']([\d.]+)(?:px)?["']/i)?.[1]);
  return isUsable(width) && isUsable(height) ? { width, height } : null;
};

const isUsable = (value: number): boolean => Number.isFinite(value) && value > 0;

/**
 * The scale at which the whole diagram is visible.
 *
 * Never above 1: a four-node flowchart blown up to fill a wide panel looks
 * like a mistake, and nothing is gained by it.
 */
export const fitScale = (diagram: Size, viewport: Size): number => {
  if (!isUsable(diagram.width) || !isUsable(diagram.height)) return 1;
  if (!isUsable(viewport.width) || !isUsable(viewport.height)) return 1;
  return Math.min(1, viewport.width / diagram.width, viewport.height / diagram.height);
};

/**
 * Where a diagram opens.
 *
 * In stage/fullscreen mode, fits the diagram to the viewport.
 * In panel mode, fitted when fitting leaves it readable, and floored at
 * [`MIN_LEGIBLE_SCALE`] when it does not — at which point the diagram
 * overflows on purpose and the viewport says so.
 */
export const initialScale = (diagram: Size, viewport: Size, isStage = false): number => {
  const fitted = fitScale(diagram, viewport);
  return isStage ? clampScale(fitted) : Math.max(fitted, MIN_LEGIBLE_SCALE);
};

export const clampScale = (scale: number): number =>
  Math.min(MAX_SCALE, Math.max(MIN_SCALE, scale));

export const zoomBy = (scale: number, factor: number): number => clampScale(scale * factor);

/** The diagram's size on screen at a given zoom. */
export const scaledSize = (diagram: Size, scale: number): Size => ({
  width: diagram.width * scale,
  height: diagram.height * scale,
});

/**
 * Calculates initial pan:
 * - Centers any axis that fits within the viewport.
 * - Starts at 0 for any axis that overflows, ensuring the beginning of the diagram is visible.
 */
export const centerPan = (diagram: Size, viewport: Size, scale: number): Pan => {
  const scaled = scaledSize(diagram, scale);
  return {
    x: Math.max(0, (viewport.width - scaled.width) / 2),
    y: Math.max(0, (viewport.height - scaled.height) / 2),
  };
};

export const initialPan = (diagram: Size, viewport: Size, scale: number): Pan =>
  centerPan(diagram, viewport, scale);

/**
 * Keeps the diagram from being dragged off its own viewport.
 *
 * When content exceeds viewport, pan is bounded to [viewport - content, 0].
 * When content fits within viewport, pan is bounded to [0, viewport - content].
 */
export const clampPan = (pan: Pan, diagram: Size, viewport: Size, scale: number): Pan => {
  const scaled = scaledSize(diagram, scale);
  return {
    x: clampAxis(pan.x, scaled.width, viewport.width),
    y: clampAxis(pan.y, scaled.height, viewport.height),
  };
};

const clampAxis = (offset: number, content: number, available: number): number => {
  if (!isUsable(content) || !isUsable(available)) return 0;
  const minBound = Math.min(0, available - content);
  const maxBound = Math.max(0, available - content);
  return Math.min(maxBound, Math.max(minBound, offset));
};

/** Whether the diagram is larger than its viewport at this zoom, on either axis. */
export const isOverflowing = (diagram: Size, viewport: Size, scale: number): boolean => {
  const scaled = scaledSize(diagram, scale);
  return scaled.width > viewport.width + 1 || scaled.height > viewport.height + 1;
};

/**
 * Zooms about a point rather than about the corner.
 *
 * Zooming about the origin walks whatever you were looking at off the screen,
 * which on a wide flowchart means losing your place on every click. The point
 * under the cursor — or the middle of the viewport, for a keyboard press —
 * stays where it is.
 */
export const zoomAbout = (
  pan: Pan,
  focus: Pan,
  previousScale: number,
  nextScale: number,
): Pan => {
  if (previousScale <= 0) return pan;
  const ratio = nextScale / previousScale;
  return {
    x: focus.x - (focus.x - pan.x) * ratio,
    y: focus.y - (focus.y - pan.y) * ratio,
  };
};

/** How far one key press moves the diagram. */
export const panStep = (large: boolean): number => (large ? PAN_STEP_LARGE : PAN_STEP);

/** Shortest a diagram panel gets, so a one-node diagram still looks deliberate. */
export const PANEL_MIN_HEIGHT = 160;

/**
 * Tallest a diagram panel gets before it becomes scrollable.
 *
 * A summary is read by scrolling through it, and a diagram that takes the
 * whole window pushes the text that explains it off the screen. Past this the
 * panel keeps its height and the diagram moves inside it — or the reader opens
 * it full screen, which is what that button is for.
 */
export const PANEL_MAX_HEIGHT = 420;

/**
 * How tall the panel should be for a diagram at a given zoom.
 *
 * Measured from the diagram rather than fixed, so a small flowchart does not
 * sit in a half-empty box and a large one does not take over the page.
 */
export const panelHeight = (diagram: Size | null, scale: number): number => {
  if (!diagram) return PANEL_MIN_HEIGHT;
  const wanted = Math.round(diagram.height * scale);
  return Math.min(PANEL_MAX_HEIGHT, Math.max(PANEL_MIN_HEIGHT, wanted));
};

/** A zoom rounded for display. 0.55 is "55%", not "55.00000000000001%". */
export const formatScale = (scale: number): string => `${Math.round(scale * 100)}%`;
