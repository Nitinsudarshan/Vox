import React from 'react';
import { afterEach, describe, expect, test, vi } from 'vitest';
import { act, render, screen, waitFor } from '@testing-library/react';
import mermaid from 'mermaid';
import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';

import { DIAGRAM_SKIN } from '@/lib/diagramTheme';

import { MarkdownView, mermaidConfig } from './MarkdownView';

// jsdom has no layout, so the real renderer cannot measure a label. The tests
// below are about which configuration reaches it, not about the SVG it draws.
vi.mock('mermaid', () => ({
  default: {
    initialize: vi.fn(),
    render: vi.fn(async (id: string) => ({ svg: `<svg id="${id}"><g /></svg>` })),
  },
}));

const lastInitialize = () => {
  const calls = vi.mocked(mermaid.initialize).mock.calls;
  return calls[calls.length - 1][0] as { themeVariables?: Record<string, unknown> };
};

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

  test('drives the whole palette from the skin rather than a stock theme', () => {
    // 'default' and 'dark' compute much of their palette before themeVariables
    // are applied, so a skin laid over one of them only half-takes. 'base' is
    // the theme that exists to be driven entirely by the variables.
    expect(mermaidConfig(true).theme).toBe('base');
    expect(mermaidConfig(false).theme).toBe('base');
  });

  test('follows the active theme', () => {
    expect(mermaidConfig(true).themeVariables?.darkMode).toBe(true);
    expect(mermaidConfig(false).themeVariables?.darkMode).toBe(false);
    expect(mermaidConfig(true).themeVariables?.background).toBe(DIAGRAM_SKIN.dark.paper2);
    expect(mermaidConfig(false).themeVariables?.background).toBe(DIAGRAM_SKIN.light.paper2);
  });

  test('carries no colour of its own', () => {
    // `rules/design-system.md`: no raw hex in a component. Mermaid wants hex
    // rather than Tailwind classes, so the hex lives in the token module and
    // this file names roles.
    const source = readFileSync(
      resolve(process.cwd(), 'src/components/common/MarkdownView.tsx'),
      'utf8',
    );
    expect(source.match(/#[0-9a-fA-F]{6}\b/g)).toBeNull();
  });
});

describe('a diagram already on screen', () => {
  afterEach(() => {
    document.documentElement.classList.remove('dark');
    vi.mocked(mermaid.initialize).mockClear();
  });

  test('is redrawn when the theme flips under it', async () => {
    // A rendered diagram is a baked SVG — its colours are attributes on the
    // elements — so flipping the theme does nothing to one already mounted.
    // Switching to light mode used to leave a column of dark diagrams running
    // down the middle of a light page of meeting summaries.
    document.documentElement.classList.remove('dark');
    render(<MarkdownView content={'```mermaid\ngraph TD\nA-->B\n```'} />);

    await waitFor(() => expect(mermaid.initialize).toHaveBeenCalled());
    expect(lastInitialize().themeVariables?.darkMode).toBe(false);

    act(() => {
      document.documentElement.classList.add('dark');
    });

    await waitFor(() => expect(lastInitialize().themeVariables?.darkMode).toBe(true));
  });
});

describe('MarkdownView with untrusted content', () => {
  test('does not fetch remote images until asked', async () => {
    const { container } = render(
      <MarkdownView content="![pixel](https://tracker.example/p.gif)" untrusted />,
    );
    expect(container.querySelector('img')).toBeNull();
    const button = screen.getByRole('button', { name: /tracker\.example/ });
    button.click();
    expect(await screen.findByAltText('pixel')).toBeDefined();
  });

  test('never renders a diagram, fenced or bare', () => {
    const { container } = render(
      <MarkdownView content={'```text\ngraph TD\nA-->B\n```\n\npie title x'} untrusted />,
    );
    expect(container.querySelector('pre')?.textContent).toContain('graph TD');
    expect(container.textContent).toContain('pie title x');
    expect(container.querySelector('[data-mermaid], svg')).toBeNull();
  });

  test('a longer fence is not closed by a shorter one inside it', () => {
    const { container } = render(
      <MarkdownView content={'````\n```\n![x](https://e.com/i.png)\n````'} untrusted />,
    );
    const pre = container.querySelector('pre');
    expect(pre?.textContent).toContain('![x](https://e.com/i.png)');
    expect(container.querySelector('img')).toBeNull();
    expect(screen.queryByRole('button')).toBeNull();
  });
});
