// Native context menus (real NSMenu / Win32 popup menus via Tauri).

import { CheckMenuItem, IconMenuItem, Menu, MenuItem, PredefinedMenuItem, Submenu } from '@tauri-apps/api/menu';
import { LogicalPosition } from '@tauri-apps/api/dpi';
import { META_COLUMNS } from './columns';
import { api } from './api';
import { command, menuAccel, run, shortcutOf } from './commands';
import { t } from './i18n/index.svelte';
import { childLoc, dirName, isArchiveName, isDiskImage, isResults } from './paths';
import { isMac, isWindows, shortcutLabel } from './platform';
import { app } from './state/app.svelte';
import { settings } from './state/settings.svelte';
import { showError } from './state/ui.svelte';
import type { Tab } from './state/tab.svelte';
import { F } from './types';

type Item = MenuItem | CheckMenuItem | IconMenuItem | PredefinedMenuItem | Submenu;

type Spec = { cmd: string; label?: string; enabled?: boolean } | 'sep' | { submenu: string; items: Spec[] } | { item: Item };

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
    if ('item' in s) {
      out.push(s.item);
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

/** Flattens specs into the items of the "Dualion" submenu of Explorer's menu. */
function shellItems(specs: Spec[]): { id: string; label?: string; enabled?: boolean }[] {
  const out: { id: string; label?: string; enabled?: boolean }[] = [];
  for (const s of specs) {
    if (s === 'sep') {
      if (out.length && out[out.length - 1].id) out.push({ id: '' });
    } else if ('cmd' in s) {
      const k = shortcutOf(s.cmd);
      const label = s.label ?? t(command(s.cmd)?.label ?? s.cmd);
      out.push({ id: s.cmd, label: k ? `${label}\t${shortcutLabel(k)}` : label, enabled: s.enabled ?? true });
    }
  }
  while (out.length && !out[out.length - 1].id) out.pop();
  return out;
}

/**
 * Windows: Explorer's own context menu (shell extensions, Send to, Open with…)
 * with Dualion's commands in a submenu. False when it cannot be shown.
 */
async function explorerMenu(tab: Tab, specs: Spec[]): Promise<boolean> {
  const entries = tab.targets().filter((x) => !(x.flags & F.UP));
  const paths = entries.map((x) => childLoc(tab.location, x)).flatMap((l) => (l.kind === 'local' ? [l.path] : []));
  if (!paths.length || paths.length !== entries.length || new Set(paths.map(dirName)).size !== 1) return false;
  try {
    const id = await api.shellMenu(paths, entries.every((x) => x.isDir), 'Dualion', shellItems(specs));
    if (id) run(id);
    return true;
  } catch (err) {
    console.warn('shell menu', err);
    return false;
  }
}

/** Right click on a file. */
export async function entryMenu(tab: Tab) {
  const items = tab.targets();
  const e = tab.current;
  const writable = tab.location.kind === 'local' || tab.location.kind === 'remote' || isResults(tab.location);
  const local = tab.location.kind === 'local' || isResults(tab.location);
  const single = items.length === 1;
  const isFile = single && !items[0].isDir;
  const archive = items.some((x) => !x.isDir && isArchiveName(x.name));
  if (isWindows && local) {
    const dualion: Spec[] = [
      { cmd: 'view', enabled: isFile },
      { cmd: 'edit', enabled: isFile },
      'sep',
      { cmd: 'copy' },
      { cmd: 'move' },
      { cmd: 'duplicate' },
      'sep',
      { cmd: 'pack' },
      ...(archive ? [{ cmd: 'unpack' } as Spec] : []),
      ...(single && isDiskImage(items[0].name) ? [{ cmd: 'mountImage' } as Spec] : []),
      { cmd: 'checksum', enabled: items.some((x) => !x.isDir) },
      'sep',
      { cmd: 'copyPath' },
      { cmd: 'copyName' },
      ...(app.clipboard ? [{ cmd: 'clipPaste' } as Spec] : []),
    ];
    if (await explorerMenu(tab, dualion)) return;
  }
  if (isMac) return popup(await finderSpecs(tab, { writable, local, single, isFile, archive }));
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
    ...(single && isDiskImage(items[0].name) ? [{ cmd: 'mountImage' } as Spec] : []),
    { cmd: 'checksum', enabled: local && items.some((x) => !x.isDir) },
    'sep',
    ...(isMac ? [{ cmd: 'quickLook', enabled: single } as Spec] : []),
    { cmd: 'reveal', enabled: local },
    { cmd: 'properties', enabled: local },
  ];
  await popup(specs);
}

/** "Open With" submenu: the applications registered for the file, default first. */
async function openWithMenu(path: string): Promise<Submenu> {
  const apps = await api.openWithApps(path).catch(() => []);
  const items: Item[] = [];
  for (const a of apps) {
    const text = a.default ? `${a.name} (${t('ctx.default')})` : a.name;
    const action = () => void api.openWith(path, a.path).catch(showError);
    items.push(a.icon.length ? await IconMenuItem.new({ text, icon: new Uint8Array(a.icon), action }) : await MenuItem.new({ text, action }));
    if (a.default && apps.length > 1) items.push(await PredefinedMenuItem.new({ item: 'Separator' }));
  }
  if (items.length) items.push(await PredefinedMenuItem.new({ item: 'Separator' }));
  items.push(await MenuItem.new({ text: t('ctx.otherApp'), action: () => run('openWith') }));
  return Submenu.new({ text: t('ctx.openWith'), items });
}

/** macOS: Finder's layout first, Dualion's own commands below. */
async function finderSpecs(
  tab: Tab,
  f: { writable: boolean; local: boolean; single: boolean; isFile: boolean; archive: boolean },
): Promise<Spec[]> {
  const items = tab.targets();
  const e = items[0];
  const loc = f.single && e && !(e.flags & F.UP) ? childLoc(tab.location, e) : null;
  return [
    { cmd: 'open' },
    ...(loc?.kind === 'local' ? [{ item: await openWithMenu(loc.path) } as Spec] : []),
    ...(e && e.flags & F.PACKAGE ? [{ cmd: 'showPackage' } as Spec] : []),
    'sep',
    { cmd: 'delete', label: settings.useTrash ? t('ctx.trash') : undefined, enabled: f.writable },
    'sep',
    { cmd: 'properties', enabled: f.local },
    { cmd: 'rename', enabled: f.single && f.writable },
    { cmd: 'pack', enabled: f.local },
    { cmd: 'duplicate', enabled: f.writable },
    { cmd: 'quickLook', enabled: f.single },
    'sep',
    { cmd: 'clipCopy' },
    { cmd: 'clipCut', enabled: f.writable },
    ...(app.clipboard ? [{ cmd: 'clipPaste' } as Spec] : []),
    { cmd: 'copyPath' },
    { cmd: 'copyName' },
    'sep',
    { cmd: 'view', enabled: f.isFile },
    { cmd: 'edit', enabled: f.isFile },
    { cmd: 'copy' },
    { cmd: 'move', enabled: f.writable },
    ...(f.archive ? [{ cmd: 'unpack' } as Spec] : []),
    ...(f.single && isDiskImage(e.name) ? [{ cmd: 'mountImage' } as Spec] : []),
    { cmd: 'checksum', enabled: f.local && items.some((x) => !x.isDir) },
    'sep',
    { cmd: 'reveal', enabled: f.local },
  ];
}

/** Right click on empty space in a panel. */
export async function backgroundMenu(tab: Tab) {
  const writable = tab.location.kind === 'local' || tab.location.kind === 'remote';
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
    ...(tab.home ? [] : [await MenuItem.new({ text: t('tabs.lockFolder'), action: () => A.toggleLockTabFolder(side, index) })]),
    await MenuItem.new({ text: t('tabs.rename') + '…', action: () => void A.renameTab(side, index) }),
    await MenuItem.new({ text: t('cmd.duplicateTab'), action: () => void A.duplicateTab(side, index) }),
    await MenuItem.new({ text: t('cmd.moveTabToOtherSide'), action: () => void A.moveTabToOtherSide(side, index) }),
    await PredefinedMenuItem.new({ item: 'Separator' }),
    await MenuItem.new({ text: t('cmd.closeTab'), enabled: !tab.locked && side.tabs.length > 1, action: () => A.closeTab(side, index) }),
    await MenuItem.new({ text: t('cmd.closeOtherTabs'), enabled: side.tabs.length > 1, action: () => side.closeOthers(index) }),
  ];
  const menu = await Menu.new({ items });
  await menu.popup();
}

// ---------------------------------------------------------------- columns

/** Right click on the list header: choose metadata columns for this folder. */
export async function columnsMenu(tab: Tab) {
  const key = tab.columnsKey;
  const current = tab.columns;
  const set = (ids: string[] | null) => {
    if (key === null) return;
    const next = { ...settings.folderColumns };
    if (ids === null) delete next[key];
    else next[key] = ids;
    // Keep the map bounded.
    const keys = Object.keys(next);
    if (keys.length > 500) delete next[keys[0]];
    settings.folderColumns = next;
    if (!settings.metaColumns) settings.metaColumns = true;
  };
  const items: Item[] = [];
  items.push(
    await CheckMenuItem.new({
      text: t('columns.auto'),
      checked: key !== null && !settings.folderColumns[key],
      enabled: key !== null,
      action: () => set(null),
    }),
  );
  items.push(await PredefinedMenuItem.new({ item: 'Separator' }));
  for (const group of ['photo', 'video', 'audio', 'doc', 'other'] as const) {
    const sub: Item[] = [];
    for (const c of META_COLUMNS.filter((c) => c.group === group)) {
      const on = current.includes(c.id);
      sub.push(
        await CheckMenuItem.new({
          text: t(c.label),
          checked: on,
          enabled: key !== null,
          action: () => set(on ? current.filter((x) => x !== c.id) : [...current, c.id]),
        }),
      );
    }
    items.push(await Submenu.new({ text: t(`columns.${group}`), items: sub }));
  }
  items.push(await PredefinedMenuItem.new({ item: 'Separator' }));
  items.push(await MenuItem.new({ text: t('columns.none'), enabled: key !== null && current.length > 0, action: () => set([]) }));
  items.push(
    await CheckMenuItem.new({ text: t('col.attrs'), checked: settings.showAttrs, action: () => (settings.showAttrs = !settings.showAttrs) }),
  );
  items.push(
    await CheckMenuItem.new({
      text: t('settings.metaColumns'),
      checked: settings.metaColumns,
      action: () => (settings.metaColumns = !settings.metaColumns),
    }),
  );
  const menu = await Menu.new({ items });
  await menu.popup();
}
