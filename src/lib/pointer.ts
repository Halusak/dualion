// Mouse behaviour shared by all file views: click / Ctrl(Cmd)+click /
// Shift+click selection, drag & drop start and double-click to open.

import * as A from './actions';
import { drag, endDrag, moveDrag, startDrag } from './dnd.svelte';
import { isMac } from './platform';
import { app, type SideName } from './state/app.svelte';
import type { Tab } from './state/tab.svelte';
import { F, type Entry } from './types';

export function itemPointerDown(e: PointerEvent, side: SideName, tab: Tab, entry: Entry, index: number) {
  app.activeSide = side;
  if (tab.renaming && tab.renaming !== entry.key) tab.renaming = null;
  if (e.button === 2) {
    if (!tab.isMarked(entry.key)) {
      tab.unmarkAll();
      tab.setCursor(index);
    }
    return;
  }
  if (e.button !== 0) return;
  const mod = isMac ? e.metaKey : e.ctrlKey;
  if (e.shiftKey) {
    tab.selectTo(index);
    return;
  }
  if (mod) {
    // Like Explorer/Finder: the item that had the focus joins the selection.
    const cur = tab.current;
    if (tab.marked.size === 0 && cur && cur.key !== entry.key && !(cur.flags & F.UP)) tab.toggleMark(cur.key);
    tab.toggleMark(entry.key);
    tab.setCursor(index);
    return;
  }

  // Like Total Commander: a plain click only moves the cursor, marks stay
  // (they are cleared explicitly: Deselect all, Num -, Esc …).
  tab.setCursor(index);

  const sx = e.clientX;
  const sy = e.clientY;
  let dragging = false;
  const move = (ev: PointerEvent) => {
    if (!dragging && Math.hypot(ev.clientX - sx, ev.clientY - sy) > 6 && !(entry.flags & F.UP)) {
      dragging = true;
      const items = tab.isMarked(entry.key) ? tab.targets() : [entry];
      startDrag(side, items, ev.clientX, ev.clientY);
    }
    if (dragging) moveDrag(ev);
  };
  const up = () => {
    window.removeEventListener('pointermove', move);
    window.removeEventListener('pointerup', up);
    if (dragging && drag.active) void endDrag();
  };
  window.addEventListener('pointermove', move);
  window.addEventListener('pointerup', up);
}

export function itemDoubleClick(tab: Tab, entry: Entry, e: MouseEvent) {
  if (tab.renaming) return;
  void A.openEntry(tab, entry, { external: e.altKey });
}

/** Click on empty space below the items. */
export function backgroundPointerDown(e: PointerEvent, side: SideName, tab: Tab) {
  app.activeSide = side;
  if (e.button === 0 && e.target === e.currentTarget) {
    tab.renaming = null;
  }
}
