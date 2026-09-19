import { describe, expect, test } from 'vitest';

import { sanitizeSvgMarkup } from './svgSafety';

const wrap = (inner: string) =>
  `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 100">${inner}</svg>`;

describe('sanitizeSvgMarkup', () => {
  test('keeps an ordinary diagram intact', () => {
    const svg = wrap('<g class="node"><rect width="40" height="20"/><text>Ship Thursday</text></g>');
    const clean = sanitizeSvgMarkup(svg);
    expect(clean).toContain('Ship Thursday');
    expect(clean).toContain('<rect');
    expect(clean).toContain('viewBox="0 0 100 100"');
  });

  test('keeps the foreignObject that wraps flowchart labels', () => {
    // The whole reason this is hand-written rather than a generic profile: an
    // SVG allowlist that drops foreignObject deletes every label in a
    // flowchart, which reads as a rendering bug rather than as a policy.
    const svg = wrap(
      '<foreignObject width="80" height="30"><div xmlns="http://www.w3.org/1999/xhtml">Send the deck<br>first</div></foreignObject>',
    );
    const clean = sanitizeSvgMarkup(svg);
    expect(clean).toContain('foreignObject');
    expect(clean).toContain('Send the deck');
  });

  test('removes a script smuggled through a diagram label', () => {
    const svg = wrap('<text>ok</text><script>window.__owned = true;</script>');
    const clean = sanitizeSvgMarkup(svg);
    expect(clean).not.toContain('__owned');
    expect(clean).not.toContain('<script');
    expect(clean).toContain('ok');
  });

  test('removes event-handler attributes whatever their casing', () => {
    const svg = wrap('<rect width="10" height="10" onload="steal()" OnClick="steal()"/>');
    const clean = sanitizeSvgMarkup(svg);
    expect(clean.toLowerCase()).not.toContain('onload');
    expect(clean.toLowerCase()).not.toContain('onclick');
    expect(clean).toContain('<rect');
  });

  test('removes links whose scheme is code, and keeps ordinary ones', () => {
    const dangerous = sanitizeSvgMarkup(wrap('<a href="javascript:steal()"><text>x</text></a>'));
    expect(dangerous.toLowerCase()).not.toContain('javascript:');

    const ordinary = sanitizeSvgMarkup(wrap('<a href="https://example.com"><text>x</text></a>'));
    expect(ordinary).toContain('https://example.com');
  });

  test('removes an iframe hidden inside a label', () => {
    const clean = sanitizeSvgMarkup(
      wrap('<foreignObject><iframe src="https://example.com"></iframe></foreignObject>'),
    );
    expect(clean).not.toContain('<iframe');
  });

  test('returns nothing for markup that is not a diagram', () => {
    // Not the original: markup with no SVG in it is not a diagram, and
    // passing it through would defeat the point of looking.
    expect(sanitizeSvgMarkup('<div onclick="steal()">not a diagram</div>')).toBe('');
    expect(sanitizeSvgMarkup('')).toBe('');
  });
});
