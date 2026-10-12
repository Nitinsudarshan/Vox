/**
 * The skin every diagram Vox renders is drawn in.
 *
 * ## Where it comes from
 *
 * This is the `diagram-design` editorial design system
 * (https://github.com/cathrynlavery/diagram-design, MIT), ported from its
 * `references/style-guide.md` into the one form Mermaid can consume: a flat
 * map of theme variables. The source is a prompt library for an agent drawing
 * SVG by hand, so nothing of it is importable — what transfers is the skin:
 * the semantic roles, the contrast contract that keeps them legible, the
 * typography register, and the stroke/radius grammar.
 *
 * ## Why roles instead of hex
 *
 * `rules/design-system.md` says never to write a raw hex value in a component.
 * Mermaid takes hex strings and not Tailwind classes, so the hex has to exist
 * somewhere; this module is that somewhere. A component asks for `accent`, not
 * for `#bf4520`, and the two skins are swapped by picking a different record
 * rather than by threading an `isDark` flag through colour decisions.
 *
 * ## The one rule that is not a colour
 *
 * `accent` is editorial, not a status flag: one or two focal nodes per diagram,
 * never five. That is why the default node is paper-and-ink and the accent is
 * reachable only by marking a node (`A:::focal`). A theme that painted every
 * node the accent — which is what Mermaid's stock themes do with
 * `primaryColor` — erases the signal the accent exists to carry.
 */

/** A colour as the style guide writes it: an opaque hex, or an `rgba()` tint. */
export type DiagramColor = string;

/**
 * The semantic roles. Every one is named for the job it does, so a type that
 * wants "the colour of an arrow" asks for `muted` and inherits any later
 * change to what that means.
 */
export interface DiagramSkin {
  /**
   * Which direction a renderer should derive shades in. The style guide's
   * inversion rule — same opacities, RGB flipped — is a property of the skin,
   * not of the caller, so it travels with the tokens.
   */
  darkMode: boolean;
  /** Page background, default node fill. */
  paper: DiagramColor;
  /** Diagram container background, secondary fill. */
  paper2: DiagramColor;
  /** Primary text, primary stroke. */
  ink: DiagramColor;
  /** Secondary text, default arrow stroke. */
  muted: DiagramColor;
  /** Sublabels, boundary labels. */
  soft: DiagramColor;
  /** Hairline borders. */
  rule: DiagramColor;
  /** Stronger borders, baselines. */
  ruleSolid: DiagramColor;
  /** Focal — one or two per diagram, never more. */
  accent: DiagramColor;
  /** Fill for accent-bordered boxes. */
  accentTint: DiagramColor;
  /** HTTP/API calls, external arrows. */
  link: DiagramColor;
  /**
   * The fill of an ordinary node, which the style guide's node-type table
   * calls `backend`: white on light paper, so a node reads as a clean surface
   * sitting on the container rather than dissolving into it. Dark follows the
   * inversion rule — `paper` against the `paper2` container.
   */
  nodeFill: DiagramColor;
}

export const DIAGRAM_SKIN: {
  readonly light: DiagramSkin;
  readonly dark: DiagramSkin;
} = {
  light: {
    darkMode: false,
    paper: "#f5f5f5",
    paper2: "#ececec",
    ink: "#2d3142",
    muted: "#4f5d75",
    soft: "#5a6580",
    rule: "rgba(45,49,66,0.12)",
    ruleSolid: "#bfc0c0",
    // Not the raw brand tangerine: that reads 2.86:1 on paper, too faint for a
    // label. This is the same hue darkened until it clears 4.5:1 as text.
    accent: "#bf4520",
    accentTint: "rgba(191,69,32,0.08)",
    link: "#2e5aa8",
    nodeFill: "#ffffff",
  },
  dark: {
    darkMode: true,
    paper: "#2d3142",
    paper2: "#393e53",
    ink: "#f5f5f5",
    muted: "#bfc0c0",
    soft: "#949eb2",
    rule: "rgba(245,245,245,0.12)",
    ruleSolid: "rgba(191,192,192,0.25)",
    accent: "#f08a59",
    accentTint: "rgba(240,138,89,0.10)",
    link: "#739fdf",
    nodeFill: "#2d3142",
  },
} as const;

/**
 * The categorical ramp for the chart types that must tell several slices,
 * branches or bands apart.
 *
 * It is deliberately *not* the style guide's five-colour series palette. That
 * palette is opt-in for radar — the one type the guide sanctions it for — and
 * the guide says in as many words not to backfill it elsewhere. It is also the
 * one block of the skin upstream's own `verify-contrast.py` does not measure,
 * and measuring it is how this came up: on light paper, light `series-3`
 * (`#b8915a`) reads 2.66:1, under the 3:1 an essential non-text mark needs. So
 * none of the types Mermaid actually offers Vox gets it.
 *
 * What ships instead is five tokens the contrast contract already covers, in
 * the order a reader should meet them — the focal accent first. Because every
 * one of them clears 4.5:1 as *text* on `paper`, and contrast is symmetric,
 * `paper` clears 4.5:1 as text *on* every one of them. That is what makes a
 * single label colour legal on all five slices, which is the only kind of
 * guarantee Mermaid's one-`pieSectionTextColor` API leaves room for.
 */
export const chartRamp = (skin: DiagramSkin): readonly DiagramColor[] => [
  skin.accent,
  skin.muted,
  skin.link,
  skin.soft,
  skin.ink,
];

/**
 * Font stacks, in the style guide's `system` font source.
 *
 * The skin's own faces are Geist, Geist Mono and Instrument Serif, served from
 * Google Fonts. Vox is a local-first desktop app that already pays for one
 * webfont; three more, for diagrams, is a network request per render for a
 * typographic nicety. `system` is the source the style guide provides for
 * exactly this case: keep the stacks, so a reader who has Geist installed
 * still gets it, and let everyone else fall through to Inter and the platform
 * faces. Every stack therefore ends in a generic family, on purpose.
 */
export const DIAGRAM_FONT = {
  sans: "'Geist', 'Inter', ui-sans-serif, system-ui, -apple-system, sans-serif",
  mono: "'Geist Mono', ui-monospace, SFMono-Regular, 'SF Mono', Menlo, monospace",
} as const;

/**
 * The typographic register. Mono is for *technical* content — ports, field
 * types, protocols, arrow annotations — and sans is for names. Mixing the two
 * up is what makes a diagram read as a code listing.
 */
export const DIAGRAM_TYPE = {
  /** Human-readable node labels. */
  nodeName: { family: DIAGRAM_FONT.sans, size: 12, weight: 600 },
  /** Port, protocol, URL, field type. */
  sublabel: { family: DIAGRAM_FONT.mono, size: 9, weight: 400 },
  /** Type tags and axis labels: uppercase and tracked. */
  eyebrow: {
    family: DIAGRAM_FONT.mono,
    size: 8,
    weight: 500,
    tracking: "0.18em",
  },
  /** Arrow annotations. */
  arrowLabel: {
    family: DIAGRAM_FONT.mono,
    size: 8,
    weight: 400,
    tracking: "0.06em",
  },
} as const;

/** Stroke weights. Most strokes are `default`; `strong` marks the focal node. */
export const DIAGRAM_STROKE = { thin: 0.8, default: 1, strong: 1.2 } as const;

/** Corner radii. `md` is a node box, `lg` a container. */
export const DIAGRAM_RADIUS = { sm: 4, md: 6, lg: 8 } as const;

/** Every coordinate, size and gap in a hand-drawn diagram is divisible by this. */
export const DIAGRAM_GRID = 4;

// ---------------------------------------------------------------------------
// Contrast
// ---------------------------------------------------------------------------

/** A parsed colour: straight RGB channels plus the alpha it was written with. */
export interface ParsedColor {
  rgb: readonly [number, number, number];
  alpha: number;
}

const HEX_PATTERN = /^#([0-9a-f]{3}|[0-9a-f]{6})$/i;
const RGBA_PATTERN =
  /^rgba?\(\s*(\d{1,3})\s*,\s*(\d{1,3})\s*,\s*(\d{1,3})\s*(?:,\s*(\d*\.?\d+)\s*)?\)$/i;

/**
 * Parses a style-guide colour, or throws. Throwing rather than returning a
 * fallback is deliberate: a token the contrast gate cannot read is a token it
 * cannot vouch for, and a silent default would let it ship unmeasured.
 */
export const parseColor = (value: string): ParsedColor => {
  const trimmed = value.trim();

  const hex = HEX_PATTERN.exec(trimmed);
  if (hex) {
    const digits =
      hex[1].length === 3
        ? hex[1]
            .split("")
            .map((character) => character + character)
            .join("")
        : hex[1];
    return {
      rgb: [
        Number.parseInt(digits.slice(0, 2), 16),
        Number.parseInt(digits.slice(2, 4), 16),
        Number.parseInt(digits.slice(4, 6), 16),
      ],
      alpha: 1,
    };
  }

  const rgba = RGBA_PATTERN.exec(trimmed);
  if (rgba) {
    const channels = [1, 2, 3].map((group) => Number.parseInt(rgba[group], 10));
    const alpha = rgba[4] === undefined ? 1 : Number.parseFloat(rgba[4]);
    if (channels.some((channel) => channel > 255) || alpha < 0 || alpha > 1) {
      throw new Error(`out-of-range colour: ${value}`);
    }
    return { rgb: [channels[0], channels[1], channels[2]], alpha };
  }

  throw new Error(`not a hex or rgba() colour: ${value}`);
};

const channelLuminance = (value: number): number => {
  const fraction = value / 255;
  return fraction <= 0.04045
    ? fraction / 12.92
    : ((fraction + 0.055) / 1.055) ** 2.4;
};

/** WCAG 2.x relative luminance. */
export const relativeLuminance = (
  rgb: readonly [number, number, number],
): number =>
  0.2126 * channelLuminance(rgb[0]) +
  0.7152 * channelLuminance(rgb[1]) +
  0.0722 * channelLuminance(rgb[2]);

/**
 * Flattens a translucent colour onto an opaque background. A value that clears
 * the contrast bar at full strength can fail at half opacity, so a tint is
 * always measured after it is composited, never before.
 */
export const compositeOver = (
  colour: ParsedColor,
  background: readonly [number, number, number],
): readonly [number, number, number] => [
  colour.rgb[0] * colour.alpha + background[0] * (1 - colour.alpha),
  colour.rgb[1] * colour.alpha + background[1] * (1 - colour.alpha),
  colour.rgb[2] * colour.alpha + background[2] * (1 - colour.alpha),
];

/** WCAG 2.x contrast ratio between two opaque colours. */
export const contrastRatio = (
  first: readonly [number, number, number],
  second: readonly [number, number, number],
): number => {
  const a = relativeLuminance(first);
  const b = relativeLuminance(second);
  return (Math.max(a, b) + 0.05) / (Math.min(a, b) + 0.05);
};

// ---------------------------------------------------------------------------
// Mermaid
// ---------------------------------------------------------------------------

/**
 * The skin as Mermaid theme variables.
 *
 * Mermaid has no notion of a node taxonomy — every node gets `primaryColor` —
 * so the mapping is: the container is `paper2`, an ordinary node is
 * `nodeFill` outlined in `ink`, an arrow is `muted`, and the accent appears
 * only where the diagram itself asks for it (notes, the active Gantt bar, a
 * node marked `:::focal`). Unknown keys are ignored by Mermaid, so covering
 * the per-diagram families here costs nothing and stops a sequence diagram or
 * a pie chart from falling back to stock colours the rest of the app has
 * never seen.
 */
export const mermaidThemeVariables = (
  skin: DiagramSkin,
): Record<string, string | boolean> => {
  const ramp = chartRamp(skin);

  return {
    darkMode: skin.darkMode,
    background: skin.paper2,
    fontFamily: DIAGRAM_FONT.sans,
    fontSize: `${DIAGRAM_TYPE.nodeName.size}px`,

    // Base
    primaryColor: skin.nodeFill,
    primaryTextColor: skin.ink,
    primaryBorderColor: skin.ink,
    secondaryColor: skin.paper2,
    secondaryTextColor: skin.ink,
    secondaryBorderColor: skin.ruleSolid,
    tertiaryColor: skin.paper,
    tertiaryTextColor: skin.muted,
    tertiaryBorderColor: skin.rule,
    lineColor: skin.muted,
    textColor: skin.ink,
    mainBkg: skin.nodeFill,
    errorBkgColor: skin.accentTint,
    errorTextColor: skin.accent,

    // Flowchart
    nodeBorder: skin.ink,
    nodeTextColor: skin.ink,
    clusterBkg: skin.paper,
    clusterBorder: skin.ruleSolid,
    defaultLinkColor: skin.muted,
    titleColor: skin.ink,
    edgeLabelBackground: skin.paper2,
    labelBackground: skin.paper2,

    // Notes — the one place the accent appears without being asked for, because
    // a note is an editorial aside and that is exactly what the accent is for.
    noteBkgColor: skin.accentTint,
    noteTextColor: skin.ink,
    noteBorderColor: skin.accent,

    // Sequence
    actorBkg: skin.nodeFill,
    actorBorder: skin.ink,
    actorTextColor: skin.ink,
    actorLineColor: skin.ruleSolid,
    signalColor: skin.muted,
    signalTextColor: skin.muted,
    labelBoxBkgColor: skin.paper2,
    labelBoxBorderColor: skin.ruleSolid,
    labelTextColor: skin.ink,
    // `muted`, not `soft`: a loop label sits on the diagram background, where
    // `soft` falls to 3.92:1 in the dark skin.
    loopTextColor: skin.muted,
    activationBkgColor: skin.accentTint,
    activationBorderColor: skin.accent,
    // `paper` on a token fill, which the ramp's symmetry argument below makes
    // safe in both skins.
    sequenceNumberColor: skin.paper,

    // State
    labelColor: skin.ink,
    stateBkg: skin.nodeFill,
    stateLabelColor: skin.ink,
    transitionColor: skin.muted,
    transitionLabelColor: skin.muted,
    specialStateColor: skin.ink,
    innerEndBackground: skin.ink,
    altBackground: skin.paper2,
    compositeBackground: skin.paper,
    compositeTitleBackground: skin.paper2,
    compositeBorder: skin.ruleSolid,

    // Class and ER
    classText: skin.ink,
    attributeBackgroundColorOdd: skin.paper,
    attributeBackgroundColorEven: skin.paper2,

    // Gantt — `accent` marks the two things a reader looks for first: what is
    // active, and where today is.
    sectionBkgColor: skin.paper,
    sectionBkgColor2: skin.paper2,
    altSectionBkgColor: skin.paper2,
    gridColor: skin.ruleSolid,
    taskBkgColor: skin.nodeFill,
    taskBorderColor: skin.ink,
    taskTextColor: skin.ink,
    taskTextDarkColor: skin.ink,
    taskTextLightColor: skin.paper,
    taskTextOutsideColor: skin.muted,
    taskTextClickableColor: skin.link,
    activeTaskBkgColor: skin.accentTint,
    activeTaskBorderColor: skin.accent,
    doneTaskBkgColor: skin.paper2,
    doneTaskBorderColor: skin.ruleSolid,
    critBkgColor: skin.accentTint,
    critBorderColor: skin.accent,
    todayLineColor: skin.accent,

    // Journey, pie and git graph all need categorical fills, and all three take
    // `paper` as the label colour drawn on them — see `chartRamp`.
    fillType0: ramp[0],
    fillType1: ramp[1],
    fillType2: ramp[2],
    fillType3: ramp[3],
    fillType4: ramp[4],
    fillType5: ramp[0],
    fillType6: ramp[1],
    fillType7: ramp[2],

    pie1: ramp[0],
    pie2: ramp[1],
    pie3: ramp[2],
    pie4: ramp[3],
    pie5: ramp[4],
    pie6: ramp[0],
    pie7: ramp[1],
    pie8: ramp[2],
    pie9: ramp[3],
    pie10: ramp[4],
    pie11: ramp[0],
    pie12: ramp[1],
    pieTitleTextColor: skin.ink,
    pieSectionTextColor: skin.paper,
    pieLegendTextColor: skin.ink,
    pieStrokeColor: skin.paper2,
    pieOuterStrokeColor: skin.ruleSolid,

    git0: ramp[0],
    git1: ramp[1],
    git2: ramp[2],
    git3: ramp[3],
    git4: ramp[4],
    git5: ramp[0],
    git6: ramp[1],
    git7: ramp[2],
    commitLabelColor: skin.ink,
    commitLabelBackground: skin.paper2,
    tagLabelColor: skin.ink,
    tagLabelBackground: skin.accentTint,
    tagLabelBorder: skin.accent,

    // Quadrant
    quadrant1Fill: skin.paper,
    quadrant2Fill: skin.paper2,
    quadrant3Fill: skin.paper,
    quadrant4Fill: skin.paper2,
    quadrant1TextFill: skin.ink,
    quadrant2TextFill: skin.ink,
    quadrant3TextFill: skin.ink,
    quadrant4TextFill: skin.ink,
    quadrantPointFill: skin.accent,
    quadrantPointTextFill: skin.ink,
    quadrantXAxisTextFill: skin.muted,
    quadrantYAxisTextFill: skin.muted,
    quadrantInternalBorderStrokeFill: skin.rule,
    quadrantExternalBorderStrokeFill: skin.ruleSolid,
    quadrantTitleFill: skin.ink,
  };
};

/**
 * The parts of the skin that are CSS rather than a colour: the typographic
 * register for the label slots Mermaid renders as HTML, the stroke grammar,
 * and the one affordance an author can reach for — `:::focal`.
 *
 * Mermaid attaches any class named with `A:::focal` or `class A focal` to the
 * node's group whether or not a `classDef` declares it, which is what makes
 * this work without the diagram having to spell out hex values it should not
 * know. Two focal nodes is the ceiling; the rule lives in prose because CSS
 * cannot count.
 */
export const mermaidThemeCss = (skin: DiagramSkin): string => `
  .node rect, .node circle, .node ellipse, .node polygon, .node path {
    stroke-width: ${DIAGRAM_STROKE.default}px;
  }
  .nodeLabel, .nodeLabel p, .label text, .cluster-label .nodeLabel {
    font-family: ${DIAGRAM_TYPE.nodeName.family};
    font-weight: ${DIAGRAM_TYPE.nodeName.weight};
  }
  .edgeLabel, .edgeLabel p, .edgeLabel span {
    font-family: ${DIAGRAM_TYPE.arrowLabel.family};
    font-size: ${DIAGRAM_TYPE.arrowLabel.size}px;
    font-weight: ${DIAGRAM_TYPE.arrowLabel.weight};
    letter-spacing: ${DIAGRAM_TYPE.arrowLabel.tracking};
    color: ${skin.muted};
  }
  .cluster-label, .cluster-label p, .cluster-label span {
    font-family: ${DIAGRAM_TYPE.eyebrow.family};
    font-size: ${DIAGRAM_TYPE.eyebrow.size}px;
    font-weight: ${DIAGRAM_TYPE.eyebrow.weight};
    letter-spacing: ${DIAGRAM_TYPE.eyebrow.tracking};
    text-transform: uppercase;
    color: ${skin.soft};
  }
  .cluster rect {
    stroke-width: ${DIAGRAM_STROKE.thin}px;
  }
  .node.focal rect, .node.focal circle, .node.focal ellipse,
  .node.focal polygon, .node.focal path {
    fill: ${skin.accentTint};
    stroke: ${skin.accent};
    stroke-width: ${DIAGRAM_STROKE.strong}px;
  }
  .node.focal .nodeLabel, .node.focal .nodeLabel p {
    color: ${skin.ink};
  }
`;
