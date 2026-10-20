import { describe, expect, it } from 'vitest';
import { squarify } from './treemap';

describe('squarify', () => {
  it('fills the rectangle proportionally', () => {
    const items = [6, 6, 4, 3, 2, 2, 1];
    const r = squarify(items, (n) => n, { x: 0, y: 0, w: 600, h: 400 });
    expect(r).toHaveLength(7);
    const area = r.reduce((a, p) => a + p.w * p.h, 0);
    expect(area).toBeCloseTo(240000, 3);
    for (const p of r) {
      expect((p.w * p.h) / 240000).toBeCloseTo(p.item / 24, 5);
      expect(p.x + p.w).toBeLessThanOrEqual(600.0001);
      expect(p.y + p.h).toBeLessThanOrEqual(400.0001);
    }
  });
});
