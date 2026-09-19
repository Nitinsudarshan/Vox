import { describe, expect, test } from 'vitest';

import {
  MAX_SCALE,
  MIN_LEGIBLE_SCALE,
  MIN_SCALE,
  clampPan,
  clampScale,
  fitScale,
  formatScale,
  initialScale,
  isOverflowing,
  parseSvgSize,
  PANEL_MAX_HEIGHT,
  PANEL_MIN_HEIGHT,
  panStep,
  panelHeight,
  scaledSize,
  zoomAbout,
  zoomBy,
} from './diagramViewport';

const svg = (attributes: string) => `<svg ${attributes}><g/></svg>`;

describe('parseSvgSize', () => {
  test('prefers the viewBox, which is the drawing’s own coordinates', () => {
    expect(parseSvgSize(svg('viewBox="0 0 1840 620" width="100%"'))).toEqual({
      width: 1840,
      height: 620,
    });
  });

  test('falls back to width and height, with or without a unit', () => {
    expect(parseSvgSize(svg('width="820px" height="410px"'))).toEqual({
      width: 820,
      height: 410,
    });
  });

  test('rejects a size it cannot use rather than guessing one', () => {
    // A percentage width is not a size, and a zero one is not a diagram.
    expect(parseSvgSize(svg('width="100%" height="100%"'))).toBeNull();
    expect(parseSvgSize(svg('viewBox="0 0 0 0"'))).toBeNull();
    expect(parseSvgSize('not markup')).toBeNull();
  });
});

describe('fitScale', () => {
  test('fits the constraining axis', () => {
    expect(fitScale({ width: 1000, height: 400 }, { width: 500, height: 400 })).toBeCloseTo(0.5);
    expect(fitScale({ width: 400, height: 1000 }, { width: 400, height: 250 })).toBeCloseTo(0.25);
  });

  test('never enlarges a small diagram to fill the panel', () => {
    expect(fitScale({ width: 200, height: 100 }, { width: 900, height: 600 })).toBe(1);
  });

  test('gives up on a viewport that has not been measured yet', () => {
    // A ResizeObserver has not fired on the first paint, and scaling by zero
    // would make the diagram vanish on the frame the user is watching.
    expect(fitScale({ width: 800, height: 400 }, { width: 0, height: 0 })).toBe(1);
  });
});

describe('initialScale', () => {
  test('fits when fitting is still readable', () => {
    expect(initialScale({ width: 1000, height: 400 }, { width: 800, height: 500 })).toBeCloseTo(0.8);
  });

  test('stops shrinking a very wide diagram and lets it overflow instead', () => {
    // The complaint this module answers: a flowchart three times the panel's
    // width, fitted, is a picture of a diagram rather than a diagram.
    const diagram = { width: 2400, height: 500 };
    const viewport = { width: 600, height: 400 };
    expect(fitScale(diagram, viewport)).toBeCloseTo(0.25);
    expect(initialScale(diagram, viewport)).toBe(MIN_LEGIBLE_SCALE);
    expect(isOverflowing(diagram, viewport, initialScale(diagram, viewport))).toBe(true);
  });
});

describe('clampScale', () => {
  test('holds both ends', () => {
    expect(clampScale(0.001)).toBe(MIN_SCALE);
    expect(clampScale(99)).toBe(MAX_SCALE);
    expect(clampScale(1.5)).toBe(1.5);
  });

  test('zooming cannot walk past the limits by repeating', () => {
    let scale = 1;
    for (let step = 0; step < 50; step += 1) scale = zoomBy(scale, 1.25);
    expect(scale).toBe(MAX_SCALE);
    for (let step = 0; step < 50; step += 1) scale = zoomBy(scale, 1 / 1.25);
    expect(scale).toBe(MIN_SCALE);
  });
});

describe('clampPan', () => {
  const diagram = { width: 1000, height: 800 };
  const viewport = { width: 400, height: 300 };

  test('pins an axis with room to spare, so layout can centre it', () => {
    const pan = clampPan({ x: -50, y: -50 }, { width: 200, height: 100 }, viewport, 1);
    expect(pan).toEqual({ x: 0, y: 0 });
  });

  test('stops the diagram being dragged off its own viewport', () => {
    expect(clampPan({ x: 500, y: 500 }, diagram, viewport, 1)).toEqual({ x: 0, y: 0 });
    expect(clampPan({ x: -9999, y: -9999 }, diagram, viewport, 1)).toEqual({
      x: viewport.width - diagram.width,
      y: viewport.height - diagram.height,
    });
  });

  test('the reachable range shrinks as the diagram is zoomed out', () => {
    const zoomedOut = clampPan({ x: -9999, y: 0 }, diagram, viewport, 0.5);
    expect(zoomedOut.x).toBe(viewport.width - scaledSize(diagram, 0.5).width);
  });
});

describe('zoomAbout', () => {
  test('keeps the point under the cursor where it is', () => {
    const focus = { x: 200, y: 150 };
    const before = { x: -100, y: -80 };
    const after = zoomAbout(before, focus, 1, 2);

    // The diagram coordinate under the focus is unchanged by the zoom.
    const coordinateBefore = (focus.x - before.x) / 1;
    const coordinateAfter = (focus.x - after.x) / 2;
    expect(coordinateAfter).toBeCloseTo(coordinateBefore);
  });

  test('a zoom that does not change the scale does not move anything', () => {
    const pan = { x: -30, y: -40 };
    expect(zoomAbout(pan, { x: 10, y: 10 }, 1, 1)).toEqual(pan);
  });
});

describe('presentation helpers', () => {
  test('shift pans further than a nudge', () => {
    expect(panStep(true)).toBeGreaterThan(panStep(false));
  });

  test('a zoom reads as a whole percentage', () => {
    expect(formatScale(MIN_LEGIBLE_SCALE)).toBe('55%');
    expect(formatScale(1)).toBe('100%');
    expect(formatScale(0.5500000001)).toBe('55%');
  });
});

describe('panelHeight', () => {
  test('follows a small diagram rather than boxing it in empty space', () => {
    expect(panelHeight({ width: 400, height: 240 }, 1)).toBe(240);
  });

  test('stops growing before a diagram takes over the page', () => {
    expect(panelHeight({ width: 900, height: 3000 }, 1)).toBe(PANEL_MAX_HEIGHT);
  });

  test('keeps a floor, so a single node still looks deliberate', () => {
    expect(panelHeight({ width: 120, height: 40 }, 1)).toBe(PANEL_MIN_HEIGHT);
    expect(panelHeight(null, 1)).toBe(PANEL_MIN_HEIGHT);
  });
});
