// Keyboard navigation inside the active panel (cursor movement, marking,
// type-ahead find). Commands bound to keys live in commands.ts.

import * as A from './actions';
import { isMac } from './platform';
import { settings } from './state/settings.svelte';
import type { Tab } from './state/tab.svelte';
import { F } from './types';

/** Layout metrics published by the active view (rows per page, grid columns…). */
export interface ViewMetrics {
  perPage: number;
  /** Items per row in the icon/thumbnail grids. */
  cols: number;
  /** Items per column in the column-flow "list" view. */
  rowsPerCol: number;
}

const metrics = new WeakMap<Tab, ViewMetrics>();

export function setMetrics(tab: Tab, m: ViewMetrics) {
  metrics.set(tab, m);
}

function metricsOf(tab: Tab): ViewMetrics {
  return metrics.get(tab) ?? { perPage: 20, cols: 1, rowsPerCol: 20 };
}

// ---------------------------------------------------------------- type-ahead

export const quick = $state<{ tab: Tab | null; text: string }>({ tab: null, text: '' });
let quickTimer: ReturnType<typeof setTimeout> | undefined;

function bumpQuickTimer() {
  clearTimeout(quickTimer);
  quickTimer = setTimeout(endQuickSearch, 2500);
}

export function endQuickSearch() {
  quick.tab = null;
  quick.text = '';
  clearTimeout(quickTimer);
}

function findMatch(tab: Tab, text: string, from: number, dir: 1 | -1): number {
  const v = tab.visible;
  const needle = text.toLocaleLowerCase();
  const n = v.length;
  // Prefix matches first, then anywhere in the name.
  for (const mode of ['prefix', 'contains'] as const) {
    for (let k = 0; k < n; k++) {
      const i = (from + dir * k + n * 2) % n;
      const name = v[i].name.toLocaleLowerCase();
      if (mode === 'prefix' ? name.startsWith(needle) : name.includes(needle)) return i;
    }
  }
  return -1;
}

function typeAhead(tab: Tab, ch: string): boolean {
  if (settings.typeAhead === 'filter') {
    tab.filterOpen = true;
    tab.filter += ch;
    return true;
  }
  const text = quick.tab === tab ? quick.text + ch : ch;
  const i = findMatch(tab, text, tab.cursor, 1);
  quick.tab = tab;
  quick.text = text;
  if (i >= 0) tab.setCursor(i);
  bumpQuickTimer();
  return true;
}

// ---------------------------------------------------------------- keys

/** Handles navigation keys for `tab`. Returns true when consumed. */
export function panelKey(e: KeyboardEvent, tab: Tab): boolean {
  const m = metricsOf(tab);
  const v = tab.visible;
  const mod = isMac ? e.metaKey : e.ctrlKey;
  const grid = tab.view === 'icons' || tab.view === 'thumbs';
  const list = tab.view === 'list';

  // Quick search keeps Up/Down for "previous / next match".
  if (quick.tab === tab) {
    if (e.key === 'Escape') {
      endQuickSearch();
      return true;
    }
    if (e.key === 'Backspace') {
      quick.text = quick.text.slice(0, -1);
      if (!quick.text) endQuickSearch();
      else bumpQuickTimer();
      return true;
    }
    if ((e.key === 'ArrowDown' || e.key === 'ArrowUp') && !e.shiftKey) {
      const dir = e.key === 'ArrowDown' ? 1 : -1;
      const i = findMatch(tab, quick.text, tab.cursor + dir, dir);
      if (i >= 0) tab.setCursor(i);
      bumpQuickTimer();
      return true;
    }
    // While searching, Space is part of the name ("my file"), not a mark.
    if (e.key === ' ' && !e.shiftKey && !e.metaKey && !e.ctrlKey && !e.altKey) return typeAhead(tab, ' ');
    if (e.key === 'Enter') endQuickSearch();
    else if (e.key.length !== 1) endQuickSearch();
  }

  const move = (to: number) => {
    if (e.shiftKey) tab.selectTo(to);
    else tab.setCursor(to);
    return true;
  };

  if (!mod && !e.altKey) {
    switch (e.key) {
      case 'ArrowDown':
        return move(tab.cursor + (grid ? m.cols : 1));
      case 'ArrowUp':
        return move(tab.cursor - (grid ? m.cols : 1));
      case 'ArrowRight':
        if (grid) return move(tab.cursor + 1);
        if (list) return move(tab.cursor + m.rowsPerCol);
        return false;
      case 'ArrowLeft':
        if (grid) return move(tab.cursor - 1);
        if (list) return move(tab.cursor - m.rowsPerCol);
        return false;
      case 'PageDown':
        return move(tab.cursor + Math.max(1, m.perPage - 1));
      case 'PageUp':
        return move(tab.cursor - Math.max(1, m.perPage - 1));
      case 'Home':
        return move(0);
      case 'End':
        return move(v.length - 1);
      case 'Enter':
        if (tab.renaming) return false;
        void A.openEntry(tab, tab.current, { external: e.shiftKey });
        return true;
      case 'Insert':
        A.markCurrentAndMove(tab, 1);
        return true;
      case 'Escape':
        if (tab.filter || tab.filterOpen) {
          tab.filter = '';
          tab.filterOpen = false;
          return true;
        }
        if (tab.marked.size) {
          tab.unmarkAll();
          return true;
        }
        return false;
    }
    if (e.key === ' ' || e.code === 'Space') {
      if (e.shiftKey) return false;
      const c = tab.current;
      if (!c || c.flags & F.UP) return true;
      tab.toggleMark(c.key);
      if (c.isDir && tab.isMarked(c.key) && !tab.dirSizes.has(c.key)) void A.calcDirSize(tab, c);
      return true;
    }
  }

  // Type-ahead find on printable characters.
  if (!mod && !e.ctrlKey && e.key.length === 1 && e.key !== ' ' && !(isMac && e.metaKey)) {
    if (e.altKey && !isMac) return false;
    return typeAhead(tab, e.key);
  }
  return false;
}
