// Mouse behaviour shared by all file views: click / Ctrl(Cmd)+click /
// Shift+click selection, drag & drop start and double-click to open.

import * as A from './actions';
import { cancelDrag, drag, dragKey, dragOutside, endDrag, moveDrag, startDrag, wantsMove } from './dnd.svelte';
import { isMac } from './platform';
import { app, type SideName } from './state/app.svelte';
import type { Tab } from './state/tab.svelte';
import { F, type Entry } from './types';

/** Double clicks are recognized here: the browser's dblclick needs a click
 *  count of exactly 2, so clicking into the next folder at the same spot
 *  (counted as a 3rd click) did nothing until the mouse moved. */
const DOUBLE_MS = 500;
let lastDown: { tab: Tab; key: string; at: number; x: number; y: number } | null = null;

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

  // A plain click only moves the cursor, marks stay
  // (they are cleared explicitly: Deselect all, Num -, Esc …).
  tab.setCursor(index);

  // A second press on the same item opens it on release, unless the mouse
  // moves first: a quick "click, then grab and drag" is a drag.
  const now = performance.now();
  const double = !!lastDown && lastDown.tab === tab && lastDown.key === entry.key && now - lastDown.at < DOUBLE_MS && Math.hypot(e.clientX - lastDown.x, e.clientY - lastDown.y) < 6;
  lastDown = double ? null : { tab, key: entry.key, at: now, x: e.clientX, y: e.clientY };

  const sx = e.clientX;
  const sy = e.clientY;
  let dragging = false;
  const move = (ev: PointerEvent) => {
    if (!dragging && Math.hypot(ev.clientX - sx, ev.clientY - sy) > 6 && !(entry.flags & F.UP)) {
      dragging = true;
      const items = tab.isMarked(entry.key) ? tab.targets() : [entry];
      startDrag(side, items, ev.clientX, ev.clientY);
    }
    if (!dragging) return;
    const outside = ev.clientX < 0 || ev.clientY < 0 || ev.clientX >= window.innerWidth || ev.clientY >= window.innerHeight;
    if (outside && drag.active && dragOutside()) {
      up();
      return;
    }
    moveDrag(ev);
  };
  const keys = (ev: KeyboardEvent) => dragKey(ev);
  const up = (ev?: PointerEvent) => {
    window.removeEventListener('pointermove', move);
    window.removeEventListener('pointerup', up);
    window.removeEventListener('pointercancel', cancel);
    window.removeEventListener('keydown', keys, true);
    window.removeEventListener('keyup', keys, true);
    if (dragging && drag.active) {
      if (ev) drag.move = wantsMove(ev);
      void endDrag();
    } else if (!dragging && double && ev && !tab.renaming) {
      void A.openEntry(tab, entry, { external: ev.altKey || e.altKey });
    }
  };
  const cancel = () => {
    up();
    if (drag.active) cancelDrag();
  };
  window.addEventListener('pointermove', move);
  window.addEventListener('pointerup', up);
  window.addEventListener('pointercancel', cancel);
  window.addEventListener('keydown', keys, true);
  window.addEventListener('keyup', keys, true);
}

/** Click on empty space below the items. */
export function backgroundPointerDown(e: PointerEvent, side: SideName, tab: Tab) {
  app.activeSide = side;
  if (e.button === 0 && e.target === e.currentTarget) {
    tab.renaming = null;
  }
}
