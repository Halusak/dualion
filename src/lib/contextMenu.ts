// Native context menus (real NSMenu / Win32 popup menus via Tauri).

import { CheckMenuItem, Menu, MenuItem, PredefinedMenuItem, Submenu } from '@tauri-apps/api/menu';
import { LogicalPosition } from '@tauri-apps/api/dpi';
import { command, menuAccel, run } from './commands';
import { t } from './i18n/index.svelte';
import { isArchiveName } from './paths';
import { isMac } from './platform';
import { app } from './state/app.svelte';
import type { Tab } from './state/tab.svelte';
import { F } from './types';

type Spec = { cmd: string; label?: string; enabled?: boolean } | 'sep' | { submenu: string; items: Spec[] };

type Item = MenuItem | CheckMenuItem | PredefinedMenuItem | Submenu;

async function build(specs: Spec[]): Promise<Item[]> {
  const out: Item[] = [];
  for (const s of specs) {
    if (s === 'sep') {
      if (out.length) out.push(await PredefinedMenuItem.new({ item: 'Separator' }));
      continue;
    }
    if ('submenu' in s) {
      out.push(await Submenu.new({ text: s.submenu, items: await build(s.items) }));
      continue;
    }
    const c = command(s.cmd);
    const opts = {
      text: s.label ?? t(c?.label ?? s.cmd),
      enabled: s.enabled ?? true,
      accelerator: menuAccel(s.cmd) ?? undefined,
      action: () => run(s.cmd),
    };
    out.push(c?.checked ? await CheckMenuItem.new({ ...opts, checked: c.checked() }) : await MenuItem.new(opts));
  }
  // No trailing separator.
  while (out.length && out[out.length - 1] instanceof PredefinedMenuItem) out.pop();
  return out;
}

export async function popup(specs: Spec[], at?: { x: number; y: number }) {
  const menu = await Menu.new({ items: await build(specs) });
  await menu.popup(at ? new LogicalPosition(at.x, at.y) : undefined);
}

/** Right click on a file. */
export async function entryMenu(tab: Tab) {
  const items = tab.targets();
  const e = tab.current;
  const writable = tab.location.kind === 'local' || tab.location.kind === 'ftp' || tab.location.kind === 'search';
  const local = tab.location.kind === 'local' || tab.location.kind === 'search';
  const single = items.length === 1;
  const isFile = single && !items[0].isDir;
  const archive = items.some((x) => !x.isDir && isArchiveName(x.name));
  const specs: Spec[] = [
    { cmd: 'open' },
    { cmd: 'openWith', enabled: single && !!e && !(e.flags & F.UP) },
    ...(e && e.flags & F.PACKAGE ? [{ cmd: 'showPackage' } as Spec] : []),
    'sep',
    { cmd: 'view', enabled: isFile },
    { cmd: 'edit', enabled: isFile },
    'sep',
    { cmd: 'copy' },
    { cmd: 'move', enabled: writable },
    { cmd: 'duplicate', enabled: writable },
    { cmd: 'rename', enabled: single && writable },
    { cmd: 'delete', enabled: writable },
    'sep',
    { cmd: 'clipCopy' },
    { cmd: 'clipCut', enabled: writable },
    ...(app.clipboard ? [{ cmd: 'clipPaste' } as Spec] : []),
    { cmd: 'copyPath' },
    { cmd: 'copyName' },
    'sep',
    { cmd: 'pack', enabled: local },
    ...(archive ? [{ cmd: 'unpack' } as Spec] : []),
    { cmd: 'checksum', enabled: local && items.some((x) => !x.isDir) },
    'sep',
    ...(isMac ? [{ cmd: 'quickLook', enabled: single } as Spec] : []),
    { cmd: 'reveal', enabled: local },
    { cmd: 'properties', enabled: local },
  ];
  await popup(specs);
}

/** Right click on empty space in a panel. */
export async function backgroundMenu(tab: Tab) {
  const writable = tab.location.kind === 'local' || tab.location.kind === 'ftp';
  await popup([
    { cmd: 'mkdir', enabled: writable },
    { cmd: 'newFile', enabled: writable },
    ...(app.clipboard ? [{ cmd: 'clipPaste', enabled: writable } as Spec] : []),
    'sep',
    { cmd: 'selectAll' },
    { cmd: 'selectMask' },
    'sep',
    { submenu: t('menu.view'), items: [{ cmd: 'viewDetails' }, { cmd: 'viewList' }, { cmd: 'viewIcons' }, { cmd: 'viewThumbs' }] },
    { submenu: t('menu.sort'), items: [{ cmd: 'sortName' }, { cmd: 'sortExt' }, { cmd: 'sortDate' }, { cmd: 'sortSize' }, 'sep', { cmd: 'sortReverse' }] },
    { cmd: 'toggleHidden' },
    'sep',
    { cmd: 'refresh' },
    { cmd: 'terminal', enabled: tab.location.kind === 'local' },
    { cmd: 'reveal', enabled: tab.location.kind === 'local' },
    { cmd: 'addHotlist', enabled: tab.location.kind === 'local' },
  ]);
}

/** Right click on a tab. */
export async function tabMenu(side: import('./state/app.svelte').Side, index: number) {
  const A = await import('./actions');
  const tab = side.tabs[index];
  if (!tab) return;
  const items = [
    await MenuItem.new({ text: t(tab.locked ? 'tabs.unlock' : 'tabs.lock'), action: () => A.toggleLockTab(side, index) }),
    await MenuItem.new({ text: t('cmd.duplicateTab'), action: () => void A.duplicateTab(side, index) }),
    await PredefinedMenuItem.new({ item: 'Separator' }),
    await MenuItem.new({ text: t('cmd.closeTab'), enabled: !tab.locked && side.tabs.length > 1, action: () => A.closeTab(side, index) }),
    await MenuItem.new({ text: t('cmd.closeOtherTabs'), enabled: side.tabs.length > 1, action: () => side.closeOthers(index) }),
  ];
  const menu = await Menu.new({ items });
  await menu.popup();
}

// ---------------------------------------------------------------- menu bar

type MenuNode = import('./types').MenuNode;

async function buildNodes(nodes: MenuNode[]): Promise<Item[]> {
  const out: Item[] = [];
  for (const n of nodes) {
    if (n.type === 'separator') {
      if (out.length && !(out[out.length - 1] instanceof PredefinedMenuItem)) out.push(await PredefinedMenuItem.new({ item: 'Separator' }));
    } else if (n.type === 'submenu') {
      out.push(await Submenu.new({ text: n.label, items: await buildNodes(n.items) }));
    } else if (n.type === 'item' || n.type === 'check') {
      const id = n.id;
      const opts = { text: n.label, enabled: n.enabled ?? true, accelerator: n.accel ?? undefined, action: () => run(id) };
      out.push(n.type === 'check' ? await CheckMenuItem.new({ ...opts, checked: n.checked }) : await MenuItem.new(opts));
    }
    // Predefined (macOS-only) items have no meaning in the in-window menu bar.
  }
  while (out.length && out[out.length - 1] instanceof PredefinedMenuItem) out.pop();
  return out;
}

/** Opens one top-level menu of the in-window menu bar (Windows / Linux) as a native popup. */
export async function popupMenuNodes(nodes: MenuNode[], at: { x: number; y: number }) {
  const menu = await Menu.new({ items: await buildNodes(nodes) });
  await menu.popup(new LogicalPosition(at.x, at.y));
}
