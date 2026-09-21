import { describe, it, expect } from 'vitest';
import { diffWords } from './diffWords';

describe('diffWords', () => {
  it('detects removed filler and added punctuation/capitalization', () => {
    const original = 'um so I think we should ship it';
    const rewritten = 'I think we should ship it.';
    const spans = diffWords(original, rewritten);

    const removed = spans.filter((s) => s.kind === 'removed');
    const added = spans.filter((s) => s.kind === 'added');

    expect(removed.some((s) => s.text.includes('um'))).toBe(true);
    expect(added.some((s) => s.text.includes('.'))).toBe(true);
  });

  it('handles identical strings as a single unchanged span', () => {
    const spans = diffWords('already clean text', 'already clean text');
    expect(spans).toHaveLength(1);
    expect(spans[0].kind).toBe('same');
    expect(spans[0].text).toBe('already clean text');
  });
});
