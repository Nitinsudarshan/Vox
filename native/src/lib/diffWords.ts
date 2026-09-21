export interface DiffSpan {
  kind: 'same' | 'removed' | 'added';
  text: string;
}

/**
 * Word-level diff, longest-common-subsequence.
 * Matches the Rust backend implementation in capture/rewrite.rs.
 */
export function diffWords(original: string, rewritten: string): DiffSpan[] {
  const split = (text: string): string[] => {
    const words: string[] = [];
    let i = 0;
    while (i < text.length) {
      const start = i;
      while (i < text.length && !/\s/.test(text[i])) i++;
      while (i < text.length && /\s/.test(text[i])) i++;
      words.push(text.slice(start, i));
    }
    return words;
  };

  const left = split(original);
  const right = split(rewritten);
  const rows = left.length + 1;
  const cols = right.length + 1;
  const lcs = new Int32Array(rows * cols);

  for (let i = left.length - 1; i >= 0; i--) {
    for (let j = right.length - 1; j >= 0; j--) {
      if (left[i].trim() === right[j].trim()) {
        lcs[i * cols + j] = lcs[(i + 1) * cols + (j + 1)] + 1;
      } else {
        lcs[i * cols + j] = Math.max(lcs[(i + 1) * cols + j], lcs[i * cols + (j + 1)]);
      }
    }
  }

  const spans: DiffSpan[] = [];
  const pushSpan = (span: DiffSpan) => {
    const last = spans[spans.length - 1];
    if (last && last.kind === span.kind) {
      last.text += span.text;
    } else {
      spans.push(span);
    }
  };

  let i = 0;
  let j = 0;
  while (i < left.length && j < right.length) {
    if (left[i].trim() === right[j].trim()) {
      if (left[i] === right[j]) {
        pushSpan({ kind: 'same', text: left[i] });
      } else {
        pushSpan({ kind: 'removed', text: left[i] });
        pushSpan({ kind: 'added', text: right[j] });
      }
      i++;
      j++;
    } else if (lcs[(i + 1) * cols + j] >= lcs[i * cols + (j + 1)]) {
      pushSpan({ kind: 'removed', text: left[i] });
      i++;
    } else {
      pushSpan({ kind: 'added', text: right[j] });
      j++;
    }
  }
  while (i < left.length) {
    pushSpan({ kind: 'removed', text: left[i] });
    i++;
  }
  while (j < right.length) {
    pushSpan({ kind: 'added', text: right[j] });
    j++;
  }
  return spans;
}
