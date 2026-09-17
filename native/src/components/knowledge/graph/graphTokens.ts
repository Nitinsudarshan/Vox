/**
 * Reads the design system's HSL tokens so canvas code can use them.
 *
 * Tailwind classes cannot reach a `<canvas>`: the 2D context wants a colour
 * string. Rather than hardcode one, this resolves the same
 * `--background` / `--primary` / … custom properties the rest of the app
 * styles from, so the graph follows the theme — including the light/dark
 * switch — instead of drifting into its own palette.
 */

export interface HslToken {
  h: number;
  s: number;
  l: number;
}

/**
 * Values for environments with no computed style — jsdom under vitest, and
 * the first paint before stylesheets resolve.
 *
 * These mirror `:root` in `native/src/index.css`. They are a fallback, not a
 * palette: every one of them is overwritten the moment a real computed style
 * is available, and nothing should be added here that does not already exist
 * as a token there.
 */
const FALLBACKS: Record<string, HslToken> = {
  '--background': { h: 0, s: 0, l: 100 },
  '--foreground': { h: 0, s: 0, l: 9 },
  '--card': { h: 0, s: 0, l: 98 },
  '--muted': { h: 0, s: 0, l: 96 },
  '--muted-foreground': { h: 0, s: 0, l: 32 },
  '--primary': { h: 221, s: 83, l: 53 },
  '--accent-foreground': { h: 224, s: 76, l: 48 },
  '--border': { h: 0, s: 0, l: 90 },
  '--destructive': { h: 0, s: 84.2, l: 60.2 },
  '--success-foreground': { h: 142, s: 71, l: 29 },
  '--warning-foreground': { h: 37, s: 92, l: 37 },
};

const NEUTRAL: HslToken = { h: 0, s: 0, l: 50 };

/** Parses the `"221 83% 53%"` shape shadcn stores tokens in. */
function parseTriplet(raw: string): HslToken | null {
  const parts = raw.trim().split(/\s+/);
  if (parts.length < 3) return null;
  const h = Number.parseFloat(parts[0]);
  const s = Number.parseFloat(parts[1]);
  const l = Number.parseFloat(parts[2]);
  if ([h, s, l].some((n) => Number.isNaN(n))) return null;
  return { h, s, l };
}

/**
 * Resolves one token.
 *
 * Reading computed style is a layout-flushing call, so callers resolve the
 * handful of tokens they need once per frame rather than per node — see
 * `readGraphPalette`.
 */
export function readToken(name: string): HslToken {
  try {
    if (typeof window !== 'undefined' && typeof getComputedStyle === 'function') {
      const raw = getComputedStyle(document.documentElement).getPropertyValue(name);
      const parsed = raw ? parseTriplet(raw) : null;
      if (parsed) return parsed;
    }
  } catch {
    // Falls through to the table below — a canvas that cannot read the
    // theme should still draw something legible.
  }
  return FALLBACKS[name] ?? NEUTRAL;
}

/** A CSS colour string, optionally with alpha. */
export function hsla({ h, s, l }: HslToken, alpha = 1): string {
  const a = Math.max(0, Math.min(1, alpha));
  return `hsla(${h}, ${s}%, ${l}%, ${a})`;
}

/** Shifts a token's lightness, clamped to the legal range. */
export function lighten(token: HslToken, delta: number): HslToken {
  return { ...token, l: Math.max(0, Math.min(100, token.l + delta)) };
}

/**
 * Ages a colour: a note edited today keeps its hue and saturation, one
 * untouched for a year desaturates towards the muted token and darkens.
 *
 * `recency` is the 0..1 ramp the layout computes — 0 is the oldest thing in
 * the vault, 1 the most recently touched. `isDark` decides which direction
 * "faded" points: on a dark canvas an old note sinks towards ash, on a light
 * one it rises towards paper. Fading towards black on a white background
 * would make the oldest notes the *loudest* thing on screen.
 */
export function withAge(token: HslToken, recency: number, isDark: boolean): HslToken {
  const age = 1 - Math.max(0, Math.min(1, recency));
  return {
    h: token.h,
    // Colour drains with age — the strongest single cue, and the one that
    // survives being scaled down to a 4px dot.
    s: token.s * (1 - 0.72 * age),
    l: isDark ? token.l * (1 - 0.55 * age) : token.l + (88 - token.l) * 0.55 * age,
  };
}

/** Whether the app is currently in dark mode. */
export function isDarkTheme(): boolean {
  try {
    return document.documentElement.classList.contains('dark');
  } catch {
    return false;
  }
}

/** Converts a `#rrggbb` palette entry into the HSL shape the rest of this
 *  module works in, so node-type colours and tokens compose. */
export function hexToHsl(hex: string): HslToken {
  const clean = hex.replace('#', '');
  if (clean.length !== 6) return NEUTRAL;
  const r = Number.parseInt(clean.slice(0, 2), 16) / 255;
  const g = Number.parseInt(clean.slice(2, 4), 16) / 255;
  const b = Number.parseInt(clean.slice(4, 6), 16) / 255;
  if ([r, g, b].some((n) => Number.isNaN(n))) return NEUTRAL;

  const max = Math.max(r, g, b);
  const min = Math.min(r, g, b);
  const l = (max + min) / 2;
  const d = max - min;
  if (d === 0) return { h: 0, s: 0, l: l * 100 };

  const s = d / (1 - Math.abs(2 * l - 1));
  let h: number;
  if (max === r) h = ((g - b) / d) % 6;
  else if (max === g) h = (b - r) / d + 2;
  else h = (r - g) / d + 4;
  h *= 60;
  if (h < 0) h += 360;

  return { h, s: s * 100, l: l * 100 };
}

export interface GraphPalette {
  isDark: boolean;
  background: HslToken;
  foreground: HslToken;
  muted: HslToken;
  mutedForeground: HslToken;
  border: HslToken;
  primary: HslToken;
}

/** Every token the graph renderer needs, resolved once per frame. */
export function readGraphPalette(): GraphPalette {
  return {
    isDark: isDarkTheme(),
    background: readToken('--background'),
    foreground: readToken('--foreground'),
    muted: readToken('--muted'),
    mutedForeground: readToken('--muted-foreground'),
    border: readToken('--border'),
    primary: readToken('--primary'),
  };
}

/**
 * Ring tints, one per PARA band.
 *
 * Drawn as a very low-alpha annulus, so these carry region identity rather
 * than being read as a colour in their own right. Projects and Areas take
 * the theme's own accent hues so the active rings feel like the app;
 * Resources and Archive step towards neutral as the work gets colder.
 */
export const RING_TINTS: Record<string, HslToken> = {
  projects: { h: 221, s: 83, l: 53 },
  areas: { h: 174, s: 72, l: 45 },
  resources: { h: 37, s: 92, l: 50 },
  archive: { h: 0, s: 0, l: 50 },
  uncategorised: { h: 0, s: 0, l: 40 },
};
