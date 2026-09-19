import React from 'react';
import { describe, expect, test } from 'vitest';
import { render, screen } from '@testing-library/react';
import { MarkdownView, mermaidConfig } from './MarkdownView';

describe('MarkdownView layout and containment', () => {
  test('renders markdown tables into structured HTML table with horizontal scroll container', () => {
    const markdown = `
| Header A | Header B | Header C |
| --- | --- | --- |
| Row 1 A | Row 1 B | Row 1 C |
| Row 2 A | Row 2 B | Row 2 C |
    `.trim();

    const { container } = render(<MarkdownView content={markdown} />);

    // Table elements exist
    expect(screen.getByText('Header A')).toBeDefined();
    expect(screen.getByText('Row 1 B')).toBeDefined();
    expect(screen.getByText('Row 2 C')).toBeDefined();

    // Contains table element
    const table = container.querySelector('table');
    expect(table).not.toBeNull();

    // Contains scrollable container preventing viewport overflow
    const tableWrapper = container.querySelector('.overflow-x-auto.max-w-full');
    expect(tableWrapper).not.toBeNull();
  });

  test('renders markdown images with responsive containment', () => {
    const markdown = '![Diagram Preview](https://example.com/preview.png)';

    const { container } = render(<MarkdownView content={markdown} />);
    const img = container.querySelector('img');
    expect(img).not.toBeNull();
    expect(img?.getAttribute('src')).toBe('https://example.com/preview.png');
    expect(img?.getAttribute('alt')).toBe('Diagram Preview');
    expect(img?.className).toContain('max-w-full');
  });

  test('renders paragraphs and code blocks with word break containment', () => {
    const markdown = `
A very long unbroken URL like https://github.com/stablyai/orca/blob/main/deep/nested/path/to/some/source/file/with/extreme/length.ts

\`\`\`typescript
const veryLongSymbol = "unbreakable-string-value-that-should-never-blow-out-the-editor-viewport";
\`\`\`
    `.trim();

    const { container } = render(<MarkdownView content={markdown} />);
    const paragraph = container.querySelector('p');
    expect(paragraph?.className).toContain('break-words');

    const codeWrapper = container.querySelector('.overflow-x-auto.max-w-full');
    expect(codeWrapper).not.toBeNull();
  });
});

describe('mermaid configuration', () => {
  test('never renders diagram labels as live HTML', () => {
    // Vox renders markdown it did not write — a model's meeting summary, a
    // captured web page — and 'loose' passes HTML in a diagram label straight
    // into the SVG the component injects, inside a webview holding the whole
    // Tauri command surface. This was 'loose'; it must not go back.
    expect(mermaidConfig(false).securityLevel).toBe('strict');
    expect(mermaidConfig(true).securityLevel).toBe('strict');
  });

  test('lets a diagram keep its own size so the viewport can decide what is shown', () => {
    // Fitting to the container is what made a wide flowchart render as an
    // unreadable strip with no way to get closer.
    const config = mermaidConfig(false);
    expect(config.flowchart?.useMaxWidth).toBe(false);
    expect(config.sequence?.useMaxWidth).toBe(false);
  });

  test('follows the active theme', () => {
    expect(mermaidConfig(true).theme).toBe('dark');
    expect(mermaidConfig(false).theme).toBe('default');
  });
});
