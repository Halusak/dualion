// Mouse behaviour shared by all file views: click / Ctrl(Cmd)+click /
// Shift+click selection, drag & drop start, double-click to open, and
// right-click selection mode.

import * as A from './actions';
import { backgroundMenu, entryMenu } from './contextMenu';
import { drag, dragOutside, endDrag, moveDrag, startDrag } from './dnd.svelte';
import { isMac } from './platform';
import { app, type SideName } from './state/app.svelte';
import { settings } from './state/settings.svelte';
import type { Tab } from './state/tab.svelte';
import { F, type Entry } from './types';

/** Double clicks are recognized here: the browser's dblclick needs a click
 *  count of exactly 2, so clicking into the next folder at the same spot
 *  (counted as a 3rd click) did nothing until the mouse moved. */
const DOUBLE_MS = 500;
let lastDown: { tab: Tab; key: string; at: number; x: number; y: number } | null = null;

let rightClickHandled = false;
let rightSelectActive = false;

export function canOpenContextMenu(e: MouseEvent, entry: Entry | null): boolean {
  if (!settings.rightClickSelect) return true;
  if (!entry) return true;
  if (rightClickHandled || rightSelectActive) {
    rightClickHandled = false;
    return false;
  }
  return true;
}

export function itemPointerDown(e: PointerEvent, side: SideName, tab: Tab, entry: Entry, index: number) {
  app.activeSide = side;
  if (tab.renaming && tab.renaming !== entry.key) tab.renaming = null;

  if (e.button === 2) {
    if (!settings.rightClickSelect) {
      if (!tab.isMarked(entry.key)) {
        tab.unmarkAll();
        tab.setCursor(index);
      }
      return;
    }

    // Right-click selection mode
    tab.setCursor(index);
    const targetMark = !(entry.flags & F.UP) ? !tab.isMarked(entry.key) : false;
    if (!(entry.flags & F.UP)) {
      tab.setMarks([entry.key], targetMark);
    }

    rightClickHandled = false;
    rightSelectActive = true;
    let moved = false;
    const sx = e.clientX;
    const sy = e.clientY;
    let lastX = sx;
    let lastY = sy;
    const visitedKeys = new Set<string>([entry.key]);

    let scrollInterval: ReturnType<typeof setInterval> | null = null;
    const stopAutoScroll = () => {
      if (scrollInterval) {
        clearInterval(scrollInterval);
        scrollInterval = null;
      }
    };

    const cleanup = () => {
      if (longPressTimer) {
        clearTimeout(longPressTimer);
        longPressTimer = null;
      }
      stopAutoScroll();
      window.removeEventListener('pointermove', onMove);
      window.removeEventListener('pointerup', onUp);
      window.removeEventListener('pointercancel', cleanup);
      window.removeEventListener('blur', cleanup);
      rightSelectActive = false;
    };

    let longPressTimer: ReturnType<typeof setTimeout> | null = setTimeout(() => {
      longPressTimer = null;
      cleanup();
      rightClickHandled = true;
      if (!(entry.flags & F.UP)) void entryMenu(tab);
      else void backgroundMenu(tab);
    }, 500);

    const updateUnderPoint = (x: number, y: number) => {
      const el = document.elementFromPoint(x, y) as HTMLElement | null;
      const panel = el?.closest<HTMLElement>('[data-side]');
      if (!panel || panel.dataset.side !== side) return;
      const row = el?.closest<HTMLElement>('[data-key]');
      if (!row?.dataset.key) return;
      const key = row.dataset.key;
      if (visitedKeys.has(key)) return;
      visitedKeys.add(key);
      const idx = tab.index.get(key);
      if (idx !== undefined) {
        const item = tab.visible[idx];
        if (item && !(item.flags & F.UP)) {
          tab.setMarks([key], targetMark);
          tab.setCursor(idx);
        }
      }
    };

    const checkScroll = (x: number, y: number) => {
      const el = document.elementFromPoint(x, y) as HTMLElement | null;
      const vp = el?.closest<HTMLElement>('.viewport, .grid');
      if (!vp) {
        stopAutoScroll();
        return;
      }
      const r = vp.getBoundingClientRect();
      const MARGIN = 35;
      const SPEED = 14;
      let dx = 0;
      let dy = 0;
      if (tab.view === 'list') {
        if (x < r.left + MARGIN) dx = -SPEED;
        else if (x > r.right - MARGIN) dx = SPEED;
      } else {
        if (y < r.top + MARGIN) dy = -SPEED;
        else if (y > r.bottom - MARGIN) dy = SPEED;
      }
      if (dx === 0 && dy === 0) {
        stopAutoScroll();
        return;
      }
      if (!scrollInterval) {
        scrollInterval = setInterval(() => {
          if (dx !== 0) vp.scrollLeft += dx;
          if (dy !== 0) vp.scrollTop += dy;
          updateUnderPoint(lastX, lastY);
        }, 30);
      }
    };

    const onMove = (ev: PointerEvent) => {
      lastX = ev.clientX;
      lastY = ev.clientY;
      if (!moved && Math.hypot(ev.clientX - sx, ev.clientY - sy) > 5) {
        moved = true;
        if (longPressTimer) {
          clearTimeout(longPressTimer);
          longPressTimer = null;
        }
      }
      if (moved) {
        updateUnderPoint(ev.clientX, ev.clientY);
        checkScroll(ev.clientX, ev.clientY);
      }
    };

    const onUp = (ev: PointerEvent) => {
      if (ev.button === 2) {
        cleanup();
        rightClickHandled = true;
      }
    };

    window.addEventListener('pointermove', onMove);
    window.addEventListener('pointerup', onUp);
    window.addEventListener('pointercancel', cleanup);
    window.addEventListener('blur', cleanup);
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

  const now = performance.now();
  const double = !!lastDown && lastDown.tab === tab && lastDown.key === entry.key && now - lastDown.at < DOUBLE_MS && Math.hypot(e.clientX - lastDown.x, e.clientY - lastDown.y) < 6;
  lastDown = double ? null : { tab, key: entry.key, at: now, x: e.clientX, y: e.clientY };
  if (double) {
    if (!tab.renaming) void A.openEntry(tab, entry, { external: e.altKey });
    return;
  }

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
  const up = () => {
    window.removeEventListener('pointermove', move);
    window.removeEventListener('pointerup', up);
    if (dragging && drag.active) void endDrag();
  };
  window.addEventListener('pointermove', move);
  window.addEventListener('pointerup', up);
}

/** Click on empty space below the items. */
export function backgroundPointerDown(e: PointerEvent, side: SideName, tab: Tab) {
  app.activeSide = side;
  if (e.button === 0 && e.target === e.currentTarget) {
    tab.renaming = null;
  }
}
