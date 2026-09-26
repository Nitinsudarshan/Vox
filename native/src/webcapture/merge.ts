/**
 * Putting samples back together.
 *
 * A virtualizing page hands over its content in overlapping, partly-repeating
 * slices, and every slice arrives out of the context that would have ordered
 * it. This module is what turns that back into a document: recognise what has
 * already been seen, keep the better version of it, and restore the order the
 * page intended.
 *
 * Three ordering keys, strongest first, because no single one is available
 * everywhere:
 *
 * 1. **The page's own ordinal.** ChatGPT numbers its turns
 *    (`conversation-turn-N`). When every item has one, that *is* the order,
 *    and gaps in it are directly measurable incompleteness.
 * 2. **Offset within the scrolling surface.** Stable across samples on a
 *    virtualized list, because the list keeps its geometry whether or not an
 *    item is mounted.
 * 3. **First-seen order.** Correct by construction when traversal runs top to
 *    bottom, and the only option on a page that exposes neither of the above.
 */

import type { CaptureMessage, ContentBlock, HarvestedItem } from './types';

/**
 * A stable-enough fingerprint for a piece of content.
 *
 * Hashes the *whole* text, never a prefix. Prefix keying is the trap: two
 * distinct turns that open with the same sentence — an assistant reply that
 * was redrafted, a template, a repeated question — collapse into one and the
 * capture silently loses a message.
 */
export function fingerprint(role: string | undefined, blocks: ContentBlock[]): string {
  const text = blocks.map(blockText).join('');
  return `${role ?? ''}${hash(text)}${text.length}`;
}

/** Everything about a block that distinguishes it from another one. */
function blockText(block: ContentBlock): string {
  switch (block.type) {
    case 'heading':
    case 'paragraph':
    case 'quote':
    case 'code':
      return block.text;
    case 'list':
      return block.items.join('');
    case 'table':
      return [block.headers.join(''), ...block.rows.map((r) => r.join(''))].join('');
    case 'image':
      return `img:${block.src ?? block.reference ?? ''}:${block.alt ?? ''}`;
    case 'attachment':
      return `file:${block.name ?? ''}:${block.href ?? block.reference ?? ''}`;
    default:
      return '';
  }
}

/**
 * FNV-1a, 32-bit.
 *
 * A hash, not a cryptographic one: the requirement is that two different
 * strings collide rarely enough that a conversation does not lose a turn, and
 * the text length is carried alongside it in the key, which leaves a collision
 * between two same-length different strings as the only failure mode.
 */
function hash(text: string): string {
  let value = 0x811c9dc5;
  for (let i = 0; i < text.length; i += 1) {
    value ^= text.charCodeAt(i);
    value = Math.imul(value, 0x01000193);
  }
  return (value >>> 0).toString(36);
}

/**
 * How much content an item carries, used to prefer the richer of two versions.
 *
 * The actual text length, not the fingerprint's — a hash is a fixed-width
 * value, so weighing it would make a truncated message and its expanded form
 * compare equal, which is exactly the comparison this exists to decide.
 */
export function weigh(blocks: ContentBlock[]): number {
  return blocks.reduce((total, block) => total + blockText(block).length, 0) + blocks.length;
}

interface Accumulated {
  item: HarvestedItem;
  firstSeen: number;
}

export interface MergeResult {
  messages: CaptureMessage[];
  /** Repeat sightings that were recognised and not stored twice. */
  duplicatesDropped: number;
  /** Distinct items kept. */
  discovered: number;
  /** Gaps in the page's own ordinal sequence. Absent when there is no ordinal. */
  missing?: number;
  /** How the final order was decided, for the diagnostics record. */
  ordering: 'ordinal' | 'offset' | 'first_seen';
}

/**
 * Accumulates harvested items across samples and reconstructs the sequence.
 *
 * Identity, when the page provides one, beats the fingerprint: a message whose
 * text changed because it was expanded is still the same message, and keying
 * on text alone would store it twice.
 */
export class SampleMerger {
  private readonly byKey = new Map<string, Accumulated>();
  /** Storage keys already holding items with a given text fingerprint. */
  private readonly byFingerprint = new Map<string, string[]>();
  private order = 0;
  private duplicates = 0;

  /**
   * Where an item is stored.
   *
   * The page's identity when it has one. Without it, two items with the same
   * text in one sample are two turns — "continue" said twice — and keying on
   * text alone merged them into one while the payload still claimed the
   * whole conversation. Such an item is matched to the stored copy nearest
   * its offset that this sample has not already claimed. (Ordinals are not
   * used as identity: some pages repeat them.)
   */
  private keyFor(item: HarvestedItem, claimed: Set<string>): string {
    if (item.identity) return `id:${item.identity}`;

    const stored = this.byFingerprint.get(item.fingerprint) ?? [];
    let nearest: string | undefined;
    let distance = Infinity;
    for (const key of stored) {
      if (claimed.has(key)) continue;
      const offset = this.byKey.get(key)?.item.offset ?? 0;
      const gap = Math.abs(offset - item.offset);
      if (gap < distance) {
        distance = gap;
        nearest = key;
      }
    }
    if (nearest) return nearest;

    const fresh = `text:${item.fingerprint}#${stored.length}`;
    this.byFingerprint.set(item.fingerprint, [...stored, fresh]);
    return fresh;
  }

  /** Adds one sample's worth of items. Returns how many were new. */
  add(items: HarvestedItem[]): number {
    let added = 0;
    const claimed = new Set<string>();
    for (const item of items) {
      const key = this.keyFor(item, claimed);
      claimed.add(key);
      const existing = this.byKey.get(key);

      if (!existing) {
        this.byKey.set(key, { item, firstSeen: this.order });
        this.order += 1;
        added += 1;
        continue;
      }

      this.duplicates += 1;
      // A later sighting that carries more content is the expanded version of
      // the same item; keep it, but keep its original position.
      if (item.weight > existing.item.weight) {
        this.byKey.set(key, { item, firstSeen: existing.firstSeen });
      }
    }
    return added;
  }

  get size(): number {
    return this.byKey.size;
  }

  result(): MergeResult {
    const entries = Array.from(this.byKey.values());

    const ordinals = entries.map((e) => e.item.ordinal);
    const everyOrdinal = ordinals.length > 0 && ordinals.every((o) => typeof o === 'number');
    const distinctOrdinals = new Set(ordinals.filter((o): o is number => typeof o === 'number'));
    const usableOrdinals = everyOrdinal && distinctOrdinals.size === entries.length;

    let ordering: MergeResult['ordering'] = 'first_seen';
    if (usableOrdinals) {
      entries.sort((a, b) => (a.item.ordinal ?? 0) - (b.item.ordinal ?? 0));
      ordering = 'ordinal';
    } else if (entries.length > 1 && entries.some((e) => e.item.offset > 0)) {
      entries.sort((a, b) => a.item.offset - b.item.offset || a.firstSeen - b.firstSeen);
      ordering = 'offset';
    } else {
      entries.sort((a, b) => a.firstSeen - b.firstSeen);
    }

    let missing: number | undefined;
    if (usableOrdinals) {
      const values = Array.from(distinctOrdinals).sort((a, b) => a - b);
      const span = values[values.length - 1] - values[0] + 1;
      // The page numbered its turns and some numbers are absent: that is
      // measured incompleteness rather than a guess about it.
      missing = Math.max(0, span - values.length);
    }

    return {
      messages: entries.map((entry) => ({
        role: entry.item.role ?? 'participant',
        blocks: entry.item.blocks,
        timestamp: entry.item.timestamp,
        ordinal: entry.item.ordinal,
      })),
      duplicatesDropped: this.duplicates,
      discovered: this.byKey.size,
      missing,
      ordering,
    };
  }
}

/**
 * The same merge, for document-shaped pages.
 *
 * Blocks have no identity of their own, so the key is the whole block's text
 * and the order is first-seen — which is reading order, given that traversal
 * runs top to bottom.
 */
export class BlockMerger {
  /** How many copies of each block are stored. */
  private readonly stored = new Map<string, number>();
  private readonly blocks: ContentBlock[] = [];
  private duplicates = 0;

  /**
   * A block repeated within one sample is kept as often as it appears
   * there: a heading or code block the author wrote twice is on the page
   * twice, and collapsing every repeat across the document lost it. Blocks
   * carry no position, so a repeat seen only in different samples still
   * collapses — TODO(capture): record each block's offset so those can be
   * told apart too.
   */
  add(blocks: ContentBlock[]): number {
    let added = 0;
    const inSample = new Map<string, number>();
    for (const block of blocks) {
      const key = fingerprint(block.type, [block]);
      const occurrence = (inSample.get(key) ?? 0) + 1;
      inSample.set(key, occurrence);
      if (occurrence <= (this.stored.get(key) ?? 0)) {
        this.duplicates += 1;
        continue;
      }
      this.stored.set(key, occurrence);
      this.blocks.push(block);
      added += 1;
    }
    return added;
  }

  result(): { blocks: ContentBlock[]; duplicatesDropped: number } {
    return { blocks: this.blocks, duplicatesDropped: this.duplicates };
  }
}
