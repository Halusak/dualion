// File index status (pushed by the backend) and its configuration.

import { listen } from '@tauri-apps/api/event';
import { api } from '../api';
import type { IndexConfig, IndexStatus, SemanticStatus } from '../types';
import type { SemanticConfig } from '../api';
import { app } from './app.svelte';
import { settings } from './settings.svelte';

export const index = $state<{ status: IndexStatus; defaultExcludes: string; cloudRoots: string[]; semantic: SemanticStatus }>({
  cloudRoots: [],
  semantic: { state: 'off', files: 0, done: 0, total: 0, error: null, models: [] },
  status: { state: 'off', ready: false, files: 0, dirs: 0, scannedDirs: 0, metaDone: 0, metaTotal: 0, lastScan: 0 },
  defaultExcludes: '',
});

let started = false;

export async function startIndex() {
  if (started) return;
  started = true;
  void listen<IndexStatus>('index-status', (e) => {
    index.status = e.payload;
  });
  void listen<SemanticStatus>('semantic-status', (e) => {
    index.semantic = e.payload;
  });
  const defs = await api.indexDefaults().catch(() => ({ excludes: '', cloud: [] as string[] }));
  index.defaultExcludes = defs.excludes;
  index.cloudRoots = defs.cloud ?? [];
  index.status = await api.indexStatus().catch(() => index.status);
}

export function indexConfig(): IndexConfig {
  const s = settings.index;
  return {
    enabled: s.enabled,
    roots: s.roots.length ? s.roots : [app.info.home],
    // Cloud sync folders can be left out entirely (their names are indexed otherwise).
    excludes: [s.excludes ?? index.defaultExcludes, ...(s.skipCloud ? index.cloudRoots : [])].filter(Boolean).join(';'),
    hidden: s.hidden,
    meta: s.meta,
  };
}

export function semanticConfig(): SemanticConfig {
  const s = settings.semantic;
  const places = (keys: string[]) => app.info.places.filter((p) => keys.includes(p.key)).map((p) => p.path);
  return { ...s, roots: s.roots.length ? s.roots : places(['documents', 'desktop', 'downloads', ...(s.images ? ['pictures'] : [])]) };
}
