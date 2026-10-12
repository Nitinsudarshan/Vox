import { describe, expect, test } from 'vitest';

import {
  DIAGRAM_SKIN,
  DiagramSkin,
  chartRamp,
  compositeOver,
  contrastRatio,
  mermaidThemeCss,
  mermaidThemeVariables,
  parseColor,
  relativeLuminance,
} from './diagramTheme';

/**
 * The contrast contract, as a gate rather than a paragraph.
 *
 * `diagram-design` ships this as `scripts/verify-contrast.py` and runs it in
 * its own CI; a skin copied out of a style guide and never measured again is a
 * skin that drifts. This is the same contract against Vox's copy, with one
 * difference that matters: upstream measures every token against `paper`,
 * because that is the surface its hand-drawn diagrams put almost everything
 * on. Mermaid has three surfaces — the container, a node, and a subgraph — so
 * each role is measured against the ones it actually lands on here.
 */

type Surface = 'paper' | 'paper2' | 'nodeFill';
type Use = 'text' | 'non-text';

const THRESHOLD: Record<Use, number> = { text: 4.5, 'non-text': 3 };

/**
 * Every role the skin ships, the surfaces Vox puts it on, and what it does
 * there. A role missing from this table fails the completeness check below, so
 * a token cannot be added without someone deciding what it has to clear.
 *
 * `soft` is on `paper` alone on purpose: it is the subgraph boundary label,
 * and on the diagram background it drops to 3.92:1 in the dark skin. That is
 * why `loopTextColor` and the quadrant axis labels use `muted` instead.
 */
const CONTRACT: Record<string, { surfaces: Surface[]; uses: Use[] } | 'surface' | 'decorative'> = {
  paper: 'surface',
  paper2: 'surface',
  nodeFill: 'surface',
  ink: { surfaces: ['paper', 'paper2', 'nodeFill'], uses: ['text', 'non-text'] },
  muted: { surfaces: ['paper', 'paper2', 'nodeFill'], uses: ['text', 'non-text'] },
  soft: { surfaces: ['paper'], uses: ['text'] },
  accent: { surfaces: ['paper'], uses: ['text', 'non-text'] },
  link: { surfaces: ['paper', 'nodeFill'], uses: ['text', 'non-text'] },
  // No minimum, but never the only carrier of meaning.
  rule: 'decorative',
  ruleSolid: 'decorative',
  accentTint: 'decorative',
  darkMode: 'decorative',
};

const opaque = (skin: DiagramSkin, role: Surface): readonly [number, number, number] => {
  const parsed = parseColor(skin[role]);
  expect(parsed.alpha, `${role} must be opaque to anchor a ratio`).toBe(1);
  return parsed.rgb;
};

const ratioOn = (skin: DiagramSkin, colour: string, surface: Surface): number => {
  const background = opaque(skin, surface);
  // A tint is composited before it is measured: a value that clears the bar at
  // full strength can fail at 8% opacity.
  return contrastRatio(compositeOver(parseColor(colour), background), background);
};

const skins: [name: string, skin: DiagramSkin][] = [
  ['light', DIAGRAM_SKIN.light],
  ['dark', DIAGRAM_SKIN.dark],
];

describe('the diagram skin honours its contrast contract', () => {
  test.each(skins)('%s: every role is classified', (_name, skin) => {
    expect(Object.keys(skin).sort()).toEqual(Object.keys(CONTRACT).sort());
  });

  test.each(skins)('%s: every role clears the bar for every job it does', (_name, skin) => {
    const failures: string[] = [];

    for (const [role, contract] of Object.entries(CONTRACT)) {
      if (contract === 'surface' || contract === 'decorative') continue;
      for (const surface of contract.surfaces) {
        const ratio = ratioOn(skin, skin[role as keyof DiagramSkin] as string, surface);
        for (const use of contract.uses) {
          if (ratio < THRESHOLD[use]) {
            failures.push(
              `${role} on ${surface} is ${ratio.toFixed(2)}:1, under ${THRESHOLD[use]} for ${use}`,
            );
          }
        }
      }
    }

    expect(failures).toEqual([]);
  });

  test.each(skins)('%s: the shipped numbers match the style guide', (name, skin) => {
    // The four the guide states outright. If a token is ever edited, this is
    // the test that says the port and its source have parted company.
    const onPaper = (colour: string) => Number(ratioOn(skin, colour, 'paper').toFixed(2));
    const expected = name === 'light'
      ? { accent: 4.71, soft: 5.34, link: 6.13, muted: 6.11 }
      : { accent: 5.21, soft: 4.78, link: 4.76, muted: 7.07 };

    expect({
      accent: onPaper(skin.accent),
      soft: onPaper(skin.soft),
      link: onPaper(skin.link),
      muted: onPaper(skin.muted),
    }).toEqual(expected);
  });
});

describe('the chart ramp', () => {
  test.each(skins)('%s: carries `paper` as a label on every step', (_name, skin) => {
    // The whole argument for a single `pieSectionTextColor`: contrast is
    // symmetric, so a fill that passes as text on paper takes paper as text.
    const paper = opaque(skin, 'paper');
    for (const step of chartRamp(skin)) {
      expect(contrastRatio(parseColor(step).rgb, paper)).toBeGreaterThanOrEqual(4.5);
    }
  });

  test.each(skins)('%s: opens on the accent and holds five distinct steps', (_name, skin) => {
    const ramp = chartRamp(skin);
    expect(ramp[0]).toBe(skin.accent);
    expect(new Set(ramp).size).toBe(ramp.length);
  });
});

describe('colour parsing', () => {
  test('reads both shapes the style guide writes', () => {
    expect(parseColor('#f5f5f5')).toEqual({ rgb: [245, 245, 245], alpha: 1 });
    expect(parseColor('#fff')).toEqual({ rgb: [255, 255, 255], alpha: 1 });
    expect(parseColor('rgba(45,49,66,0.12)')).toEqual({ rgb: [45, 49, 66], alpha: 0.12 });
  });

  test('refuses a value it cannot measure rather than guessing one', () => {
    // A fallback here would let an unmeasurable token ship unmeasured, which
    // is the one thing this module exists to prevent.
    expect(() => parseColor('hsl(210 40% 50%)')).toThrow();
    expect(() => parseColor('rgba(300,0,0,0.5)')).toThrow();
  });

  test('matches the WCAG reference points', () => {
    expect(relativeLuminance([255, 255, 255])).toBeCloseTo(1, 5);
    expect(relativeLuminance([0, 0, 0])).toBeCloseTo(0, 5);
    expect(contrastRatio([255, 255, 255], [0, 0, 0])).toBeCloseTo(21, 5);
  });
});

describe('the Mermaid mapping', () => {
  test.each(skins)('%s: never paints an ordinary node the accent', (_name, skin) => {
    // One or two focal nodes per diagram. Mermaid gives every node
    // `primaryColor`, so the accent reaching it would put it on all of them
    // and erase the signal the accent exists to carry.
    const variables = mermaidThemeVariables(skin);
    expect(variables.primaryColor).toBe(skin.nodeFill);
    expect(variables.mainBkg).toBe(skin.nodeFill);
    expect(variables.nodeBorder).toBe(skin.ink);
  });

  test.each(skins)('%s: tells Mermaid which way to derive shades', (_name, skin) => {
    expect(mermaidThemeVariables(skin).darkMode).toBe(skin.darkMode);
  });

  test.each(skins)('%s: reserves the accent for focal nodes in CSS', (_name, skin) => {
    const css = mermaidThemeCss(skin);
    expect(css).toContain('.node.focal rect');
    expect(css).toContain(skin.accentTint);
    // The accent appears only inside the focal rules, never on `.node rect`.
    const beforeFocal = css.slice(0, css.indexOf('.node.focal'));
    expect(beforeFocal).not.toContain(skin.accent);
  });
});
