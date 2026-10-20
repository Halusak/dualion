// Operation history for undo: job journals (copy, move, trash, pack,
// extract), new folders / files and renames.

import { api } from '../api';
import { t } from '../i18n/index.svelte';
import { dirName } from '../paths';
import type { UndoAction } from '../types';
import { app } from './app.svelte';
import { showError, toast } from './ui.svelte';

export interface HistoryEntry {
  id: number;
  time: number;
  label: string;
  actions?: UndoAction[];
  rename?: { dir: string; pairs: { from: string; to: string }[] };
  undone?: boolean;
}

export const history = $state<{ entries: HistoryEntry[] }>({ entries: [] });
let seq = 0;

export function pushHistory(e: Omit<HistoryEntry, 'id' | 'time'>) {
  if (!e.actions?.length && !e.rename?.pairs.length) return;
  history.entries = [{ ...e, id: ++seq, time: Date.now() }, ...history.entries].slice(0, 100);
}

/** Reverses one entry (the latest one when no id is given). */
export async function undo(id?: number) {
  const entry = id === undefined ? history.entries.find((e) => !e.undone) : history.entries.find((e) => e.id === id);
  if (!entry || entry.undone) return toast(t('misc.undoNothing'));
  try {
    const dirs = new Set<string>();
    let ok = true;
    if (entry.rename) {
      const res = await api.batchRename(entry.rename.dir, entry.rename.pairs);
      const failed = res.filter((r) => r.error);
      if (failed.length) toast(t('mr.failed', { count: failed.length }), 'error');
      ok = failed.length < res.length;
      dirs.add(entry.rename.dir);
    }
    if (entry.actions) {
      const r = await api.undoApply(entry.actions);
      for (const d of r.touched) dirs.add(d);
      ok = r.done > 0;
      if (r.failed.length) {
        showError(r.failed[0]);
        if (r.failed.length > 1) toast(t('history.failed', { count: r.failed.length }), 'error');
      }
      if (r.moves.length) toast(t('history.queued', { count: r.moves.length }));
    }
    for (const d of dirs) app.refreshLocation({ kind: 'local', path: d });
    if (!ok) return;
    history.entries = history.entries.map((e) => (e.id === entry.id ? { ...e, undone: true } : e));
    toast(t('history.undone', { label: entry.label }));
  } catch (err) {
    showError(err);
  }
}

/** Label for a journal: "Zkopírováno: foto.jpg (+3)". */
export function journalLabel(kind: string, actions: UndoAction[]): string {
  const first = actions[0];
  const path = first.type === 'moved' ? first.to : first.path;
  const name = path.slice(dirName(path).length).replace(/^[\\/]/, '');
  return `${t(`history.${kind}`)}: ${name}${actions.length > 1 ? ` (+${actions.length - 1})` : ''}`;
}
