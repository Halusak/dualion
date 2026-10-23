// Drag & drop between panels (pointer based, so it coexists with Tauri's
// native file-drop handling) and drops of files from Finder / Explorer.

import { api } from './api';
import { childLoc, sameLoc } from './paths';
import { isMac, platform } from './platform';
import { app, type SideName } from './state/app.svelte';
import { settings } from './state/settings.svelte';
import { showError } from './state/ui.svelte';
import { F, type Entry, type Location } from './types';

export interface DropTarget {
  side: SideName;
  /** Folder entry under the pointer (drop into it), otherwise the panel folder. */
  entryKey: string | null;
}

export const drag = $state<{
  active: boolean;
  from: SideName | null;
  items: Location[];
  names: string[];
  x: number;
  y: number;
  over: DropTarget | null;
  move: boolean;
}>({ active: false, from: null, items: [], names: [], x: 0, y: 0, over: null, move: false });

/** Finds the panel/row under a viewport point using data attributes on the DOM. */
export function targetAt(x: number, y: number): DropTarget | null {
  const el = document.elementFromPoint(x, y) as HTMLElement | null;
  const panel = el?.closest<HTMLElement>('[data-side]');
  if (!panel) return null;
  const side = panel.dataset.side as SideName;
  const row = el?.closest<HTMLElement>('[data-key]');
  const key = row?.dataset.key ?? null;
  const tab = app.side(side).tab;
  const entry = key ? tab.all.find((e) => e.key === key) : undefined;
  return { side, entryKey: entry && entry.isDir && !(entry.flags & F.UP) ? entry.key : null };
}

function destOf(t: DropTarget): Location | null {
  const tab = app.side(t.side).tab;
  if (t.entryKey) {
    const e = tab.all.find((x) => x.key === t.entryKey);
    if (e) return childLoc(tab.location, e);
  }
  const loc = tab.location;
  return loc.kind === 'local' || loc.kind === 'remote' ? loc : null;
}

export function startDrag(from: SideName, entries: Entry[], x: number, y: number) {
  const tab = app.side(from).tab;
  drag.active = true;
  drag.from = from;
  drag.items = entries.map((e) => childLoc(tab.location, e));
  drag.names = entries.map((e) => e.name);
  drag.x = x;
  drag.y = y;
  drag.over = null;
}

/** Shift moves instead of copying (on macOS also ⌘, like Finder). */
export function wantsMove(e: { shiftKey: boolean; metaKey: boolean }): boolean {
  return e.shiftKey || (isMac && e.metaKey);
}

export function moveDrag(e: PointerEvent) {
  drag.x = e.clientX;
  drag.y = e.clientY;
  drag.move = wantsMove(e);
  drag.over = targetAt(e.clientX, e.clientY);
}

/** Modifier keys pressed / released while dragging (without moving the mouse). */
export function dragKey(e: KeyboardEvent) {
  if (!drag.active) return;
  if (e.key === 'Escape') {
    e.preventDefault();
    e.stopPropagation();
    cancelDrag();
    return;
  }
  drag.move = wantsMove(e);
}

export function cancelDrag() {
  drag.active = false;
  drag.over = null;
  drag.items = [];
}

/**
 * The pointer left the window during a drag: hand local files over to a native
 * drag so they can be dropped into Finder / Explorer / other apps.
 */
export function dragOutside(): boolean {
  // Not available on Linux yet: the drag simply stays inside the window.
  if (platform === 'linux') return false;
  const paths = drag.items.flatMap((i) => (i.kind === 'local' ? [i.path] : []));
  if (!paths.length || paths.length !== drag.items.length) return false;
  const from = drag.from;
  drag.active = false;
  drag.over = null;
  drag.items = [];
  void api
    .dragOut(paths)
    .then(() => {
      if (from && settings.unmarkAfterDrop) app.side(from).tab.unmarkAll();
    })
    .catch(showError);
  return true;
}

export async function endDrag() {
  const over = drag.over;
  const items = drag.items;
  const move = drag.move;
  const from = drag.from;
  drag.active = false;
  drag.over = null;
  drag.items = [];
  if (!over || !items.length) return;
  const dest = destOf(over);
  if (!dest) return;
  // Dropping onto the folder the items already live in does nothing.
  if (!over.entryKey && from === over.side) return;
  if (items.some((i) => sameLoc(i, dest))) return;
  const extracting = items[0].kind === 'archive';
  if (items[0].kind === 'archive') {
    const { ensureArchiveAccess } = await import('./archives');
    if (!(await ensureArchiveAccess(items[0].archive))) return;
  }
  try {
    await api.jobEnqueue({
      op: extracting ? 'extract' : move ? 'move' : 'copy',
      sources: items,
      dest,
      options: { verify: settings.verify },
    });
    if (from && settings.unmarkAfterDrop) app.side(from).tab.unmarkAll();
  } catch (e) {
    showError(e);
  }
}

/** Files dropped from the OS (Finder / Explorer). Position is in physical pixels. */
export function dropPaths(paths: string[], pos: { x: number; y: number }) {
  if (!paths.length) return;
  const scale = window.devicePixelRatio || 1;
  const target = targetAt(pos.x / scale, pos.y / scale) ?? { side: app.activeSide, entryKey: null };
  const dest = destOf(target);
  if (!dest) return;
  void api
    .jobEnqueue({
      op: 'copy',
      sources: paths.map((p) => ({ kind: 'local', path: p })),
      dest,
      options: { verify: settings.verify },
    })
    .catch(showError);
}
