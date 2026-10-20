// Squarified treemap layout (Bruls, Huizing, van Wijk).

export interface Rect {
  x: number;
  y: number;
  w: number;
  h: number;
}

export interface Placed<T> extends Rect {
  item: T;
}

function worst(row: number[], side: number, scale: number): number {
  const s = row.reduce((a, b) => a + b, 0) * scale;
  if (s <= 0) return Infinity;
  const max = Math.max(...row) * scale;
  const min = Math.min(...row) * scale;
  const s2 = side * side;
  return Math.max((s2 * max) / (s * s), (s * s) / (s2 * min));
}

/** Lays out `items` (sorted largest first is best) with `size(item)` areas in `rect`. */
export function squarify<T>(items: readonly T[], size: (t: T) => number, rect: Rect): Placed<T>[] {
  const out: Placed<T>[] = [];
  const list = items.filter((i) => size(i) > 0);
  const total = list.reduce((a, i) => a + size(i), 0);
  if (!total || rect.w <= 0 || rect.h <= 0) return out;
  const scale = (rect.w * rect.h) / total;
  let { x, y, w, h } = rect;
  let i = 0;
  while (i < list.length) {
    const side = Math.min(w, h);
    const row: T[] = [list[i]];
    let sizes = [size(list[i])];
    i++;
    while (i < list.length) {
      const next = [...sizes, size(list[i])];
      if (worst(next, side, scale) > worst(sizes, side, scale)) break;
      row.push(list[i]);
      sizes = next;
      i++;
    }
    const rowArea = sizes.reduce((a, b) => a + b, 0) * scale;
    if (w >= h) {
      // Column on the left.
      const cw = rowArea / h;
      let cy = y;
      row.forEach((item, k) => {
        const ch = (sizes[k] * scale) / cw;
        out.push({ item, x, y: cy, w: cw, h: ch });
        cy += ch;
      });
      x += cw;
      w -= cw;
    } else {
      const rh = rowArea / w;
      let cx = x;
      row.forEach((item, k) => {
        const cw = (sizes[k] * scale) / rh;
        out.push({ item, x: cx, y, w: cw, h: rh });
        cx += cw;
      });
      y += rh;
      h -= rh;
    }
  }
  return out;
}
