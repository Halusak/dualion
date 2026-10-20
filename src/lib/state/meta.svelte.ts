// Metadata cache for the file lists: requested in batches, keyed by
// path + size + mtime so changed files are re-read.

import { SvelteMap } from 'svelte/reactivity';
import { api } from '../api';
import type { FileMeta } from '../types';

const cache = new SvelteMap<string, FileMeta | null>();
const queued = new Map<string, string>();
const inflight = new Set<string>();
let timer: ReturnType<typeof setTimeout> | undefined;

export const metaKey = (path: string, size: number, mtime: number) => `${path}\u0000${size}\u0000${mtime}`;

/** Cached metadata, or undefined while unknown (and queued for loading). */
export function metaFor(path: string, size: number, mtime: number): FileMeta | null | undefined {
  const key = metaKey(path, size, mtime);
  const m = cache.get(key);
  if (m !== undefined || inflight.has(key)) return m;
  queued.set(key, path);
  timer ??= setTimeout(flush, 16);
  return undefined;
}

/** Peeks without queueing (for sorting). */
export function metaPeek(path: string, size: number, mtime: number): FileMeta | null | undefined {
  return cache.get(metaKey(path, size, mtime));
}

async function flush() {
  timer = undefined;
  const batch = [...queued].slice(0, 400);
  for (const [k] of batch) {
    queued.delete(k);
    inflight.add(k);
  }
  if (queued.size) timer = setTimeout(flush, 0);
  try {
    const res = await api.fileMeta(batch.map(([, p]) => p));
    batch.forEach(([k], i) => cache.set(k, res[i] ?? null));
  } catch {
    for (const [k] of batch) cache.set(k, null);
  } finally {
    for (const [k] of batch) inflight.delete(k);
  }
  if (cache.size > 60_000) {
    let drop = cache.size - 50_000;
    for (const k of cache.keys()) {
      if (drop-- <= 0) break;
      cache.delete(k);
    }
  }
}
