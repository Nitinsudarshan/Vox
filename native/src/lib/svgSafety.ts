/**
 * Stripping anything executable out of an SVG before it is injected.
 *
 * ## Why this exists at all
 *
 * Mermaid renders a diagram to SVG markup, and the only way to put that on the
 * page is `dangerouslySetInnerHTML`. Vox renders markdown it did not write —
 * a model's meeting summary, a captured web page (`docs/capture.md`: captured
 * content is never an instruction) — so the diagram source is attacker
 * influenced, and an injected `<script>` runs with the webview's full Tauri
 * IPC access.
 *
 * Mermaid's own `securityLevel: 'strict'` is the first line and the one that
 * belongs to mermaid: it sanitizes labels and disables click bindings. This is
 * the second, at the point of injection, because the first is a configuration
 * flag one careless edit away from being `'loose'` again — which is exactly
 * how it was set before.
 *
 * ## What it does not try to be
 *
 * A general HTML sanitizer. It takes markup a diagram renderer produced and
 * removes the specific things that execute: script elements, event-handler
 * attributes, and URLs that are code. Everything else — including the
 * `foreignObject` that flowchart labels need to wrap text — is left alone, on
 * purpose: a sanitizer that quietly deletes half a diagram is reported as a
 * rendering bug and worked around, which is worse than one that does less.
 */

/** Elements that execute, or that load something that can. */
const FORBIDDEN_ELEMENTS = new Set(['script', 'iframe', 'object', 'embed', 'link', 'meta', 'base']);

/** Attributes carrying a URL, which may carry a scheme that is code. */
const URL_ATTRIBUTES = ['href', 'xlink:href', 'src', 'from', 'to', 'values'];

const EXECUTABLE_URL = /^\s*(javascript|vbscript|data:text\/html)/i;

/**
 * Returns the SVG with everything executable removed, or an empty string when
 * the markup holds no SVG at all.
 *
 * Empty rather than the original: markup that does not parse as a diagram is
 * not a diagram, and passing it through would defeat the point of looking.
 */
export const sanitizeSvgMarkup = (markup: string): string => {
  if (!markup || typeof DOMParser === 'undefined') return '';

  // Parsed as HTML rather than as XML: mermaid's HTML labels contain markup
  // that is valid HTML and not always well-formed XML, and an XML parse that
  // fails on a stray `<br>` would drop a diagram that renders perfectly well.
  const parsed = new DOMParser().parseFromString(markup, 'text/html');
  const svg = parsed.body.querySelector('svg');
  if (!svg) return '';

  for (const element of Array.from(svg.querySelectorAll('*'))) {
    if (FORBIDDEN_ELEMENTS.has(element.tagName.toLowerCase())) {
      element.remove();
      continue;
    }
    stripExecutableAttributes(element);
  }
  stripExecutableAttributes(svg);

  return svg.outerHTML;
};

const stripExecutableAttributes = (element: Element): void => {
  for (const attribute of Array.from(element.attributes)) {
    const name = attribute.name.toLowerCase();
    // `on…` covers onclick, onload, onerror and every handler added since.
    if (name.startsWith('on')) {
      element.removeAttribute(attribute.name);
      continue;
    }
    if (URL_ATTRIBUTES.includes(name) && EXECUTABLE_URL.test(attribute.value)) {
      element.removeAttribute(attribute.name);
    }
  }
};
