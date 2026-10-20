import { readdirSync, readFileSync, statSync } from 'node:fs';
import { join } from 'node:path';
import { describe, expect, it } from 'vitest';
import { cs } from './cs';
import { en } from './en';

function sources(dir: string): string[] {
  return readdirSync(dir).flatMap((f) => {
    const p = join(dir, f);
    if (statSync(p).isDirectory()) return sources(p);
    return /\.(ts|svelte)$/.test(f) && !f.endsWith('.test.ts') ? [p] : [];
  });
}

const has = (dict: Record<string, string>, key: string) =>
  key in dict || Object.keys(dict).some((k) => k.startsWith(`${key}_`));

describe('translations', () => {
  it('Czech covers every English key', () => {
    const missing = Object.keys(en).filter((k) => !(k in cs));
    expect(missing).toEqual([]);
  });

  it('every literal key used in the UI exists in both languages', () => {
    const root = join(__dirname, '..', '..');
    const used = new Set<string>();
    for (const file of sources(root)) {
      const text = readFileSync(file, 'utf8');
      for (const m of text.matchAll(/\bt\(\s*'([a-zA-Z0-9_.]+)'/g)) used.add(m[1]);
      for (const m of text.matchAll(/label:\s*'((?:cmd|fkey)\.[a-zA-Z0-9_.]+)'/g)) used.add(m[1]);
    }
    const missing = [...used].filter((k) => !has(en, k) || !has(cs, k));
    expect(missing).toEqual([]);
  });

  it('placeholders match between languages', () => {
    const params = (s: string) => [...s.matchAll(/\{(\w+)\}/g)].map((m) => m[1]).sort().join(',');
    const mismatched = Object.keys(en).filter((k) => params(en[k as keyof typeof en]) !== params(cs[k]));
    expect(mismatched).toEqual([]);
  });
});
