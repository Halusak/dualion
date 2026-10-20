// Text folding (lower case, no diacritics) and fuzzy matching for search UIs.

const cache = new Map<string, string>();

/** Folds one character, keeping string length ("Ž" -> "z"). */
export function foldChar(c: string): string {
  let f = cache.get(c);
  if (f === undefined) {
    f = c.normalize('NFD')[0].toLowerCase();
    if (f.length !== 1) f = c.toLowerCase().slice(0, 1) || c;
    cache.set(c, f);
  }
  return f;
}

/** "Žluťoučký Kůň" -> "zlutoucky kun" (same length as the input). */
export function fold(s: string): string {
  let out = '';
  for (const c of s) out += c.charCodeAt(0) < 128 ? c.toLowerCase() : foldChar(c);
  return out;
}

/** Ranges [start, end) of `words` found in `text` (folded matching). */
export function matchRanges(text: string, words: string[]): [number, number][] {
  text = text.normalize('NFC');
  const f = fold(text);
  if (f.length !== text.length) return [];
  const out: [number, number][] = [];
  for (const w of words) {
    if (!w) continue;
    const i = f.indexOf(w);
    if (i >= 0) out.push([i, i + w.length]);
  }
  out.sort((a, b) => a[0] - b[0]);
  // Merge overlaps.
  const merged: [number, number][] = [];
  for (const r of out) {
    const last = merged[merged.length - 1];
    if (last && r[0] <= last[1]) last[1] = Math.max(last[1], r[1]);
    else merged.push([...r]);
  }
  return merged;
}

/** Splits text into plain / highlighted parts. */
export function highlight(text: string, words: string[]): { text: string; hit: boolean }[] {
  text = text.normalize('NFC');
  const ranges = matchRanges(text, words);
  const parts: { text: string; hit: boolean }[] = [];
  let pos = 0;
  for (const [a, b] of ranges) {
    if (a > pos) parts.push({ text: text.slice(pos, a), hit: false });
    parts.push({ text: text.slice(a, b), hit: true });
    pos = b;
  }
  if (pos < text.length) parts.push({ text: text.slice(pos), hit: false });
  return parts;
}

/**
 * Fuzzy score of `query` against `text` (higher is better, -1 = no match):
 * every query character must appear in order; consecutive runs, word starts
 * and early matches score higher.
 */
export function fuzzyScore(query: string, text: string): number {
  const q = fold(query.normalize('NFC')).replace(/\s+/g, '');
  if (!q) return 0;
  const t = fold(text.normalize('NFC'));
  let score = 0;
  let ti = 0;
  let run = 0;
  for (const qc of q) {
    const i = t.indexOf(qc, ti);
    if (i < 0) return -1;
    const wordStart = i === 0 || /[\s\-_./]/.test(t[i - 1]);
    run = i === ti ? run + 1 : 0;
    score += 1 + run * 2 + (wordStart ? 3 : 0) - Math.min(3, (i - ti) * 0.1);
    ti = i + 1;
  }
  if (t.startsWith(q)) score += 6;
  else if (t.includes(q)) score += 4;
  return score - t.length * 0.01;
}
