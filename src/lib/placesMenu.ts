// Native popup menus of the path bar: drives & places, subfolders of a crumb.

import { LogicalPosition } from '@tauri-apps/api/dpi';
import { IconMenuItem, Menu, MenuItem, PredefinedMenuItem } from '@tauri-apps/api/menu';
import { run } from './commands';
import { api, type BackendLocation } from './api';
import { formatSize } from './format';
import { localeTag, t } from './i18n/index.svelte';
import { childLoc } from './paths';
import { dpr, isWindows } from './platform';
import { collator } from './sort';
import { app } from './state/app.svelte';
import { settings } from './state/settings.svelte';
import type { Tab } from './state/tab.svelte';
import { toEntry } from './state/tab.svelte';
import { showError } from './state/ui.svelte';
import { F, type Location } from './types';

type IconRef = { kind: 'dir' | 'path'; value: string } | null;
const iconCache = new Map<string, Promise<Uint8Array | null>>();

/** PNG bytes of a native icon, fetched over IPC (custom-scheme fetches are blocked in dev). */
function iconBytes(ref: NonNullable<IconRef>, px: number): Promise<Uint8Array | null> {
  const key = `${ref.kind}|${ref.value}|${px}`;
  let p = iconCache.get(key);
  if (!p) {
    p = api
      .iconPng(ref.kind, ref.value, px)
      .then((b) => new Uint8Array(b))
      .catch(() => null);
    iconCache.set(key, p);
  }
  return p;
}

const px = () => 16 * dpr();
const pathIcon = (path: string): IconRef => ({ kind: 'path', value: path });
const dirIcon: IconRef = { kind: 'dir', value: '' };

async function entry(text: string, icon: IconRef, action: () => void, enabled = true) {
  const bytes = icon ? await iconBytes(icon, px()) : null;
  if (bytes) return IconMenuItem.new({ text, icon: bytes, action, enabled });
  return MenuItem.new({ text, action, enabled });
}

const sep = () => PredefinedMenuItem.new({ item: 'Separator' });
const header = (text: string) => MenuItem.new({ text, enabled: false });

export async function placesMenu(tab: Tab, at: { x: number; y: number }) {
  const items: (MenuItem | IconMenuItem | PredefinedMenuItem)[] = [];
  const go = (loc: Location) => () => void tab.navigate(loc);

  items.push(await header(t('place.places')));
  for (const p of app.info.places) {
    items.push(await entry(t(`place.${p.key}`), pathIcon(p.path), go({ kind: 'local', path: p.path })));
  }

  items.push(await sep());
  items.push(await header(t(isWindows ? 'place.drivesWin' : 'place.drivesMac')));
  const vols = await api.volumes().catch(() => []);
  const locale = localeTag();
  for (const v of vols) {
    const free = v.total ? ` — ${formatSize(v.free, locale, settings.sizeFormat)} / ${formatSize(v.total, locale, settings.sizeFormat)}` : '';
    items.push(await entry(`${v.name}${free}`, pathIcon(v.path), go({ kind: 'local', path: v.path })));
  }
  if (isWindows) items.push(await entry(t('place.drivesWin'), null, go({ kind: 'drives' })));

  items.push(await sep());
  items.push(await header(t('place.favorites')));
  if (settings.hotlist.length === 0) items.push(await MenuItem.new({ text: t('place.noFavorites'), enabled: false }));
  for (const h of settings.hotlist) {
    items.push(await entry(h.name, h.location.kind === 'local' ? pathIcon(h.location.path) : dirIcon, go(h.location)));
  }
  items.push(await MenuItem.new({ text: t('cmd.addHotlist'), action: () => run('addHotlist') }));
  items.push(await MenuItem.new({ text: t('cmd.manageHotlist'), action: () => run('manageHotlist') }));

  items.push(await sep());
  items.push(await MenuItem.new({ text: t('cmd.ftpConnect'), action: () => run('ftpConnect') }));

  const menu = await Menu.new({ items });
  await menu.popup(new LogicalPosition(at.x, at.y));
}

/** Subfolders of `loc` (Explorer-style chevron in the breadcrumb). */
export async function subfoldersMenu(tab: Tab, loc: Location, at: { x: number; y: number }) {
  if (loc.kind === 'search') return;
  let listing;
  try {
    listing = await api.listDir(loc as BackendLocation);
  } catch (e) {
    showError(e);
    return;
  }
  const c = collator(localeTag());
  const dirs = listing.entries
    .map(toEntry)
    .filter((e) => e.isDir && (settings.showHidden || !(e.flags & F.HIDDEN)))
    .sort((a, b) => c.compare(a.name, b.name))
    .slice(0, 80);
  const items = [];
  for (const d of dirs) {
    const target = childLoc(listing.location, d);
    items.push(await entry(d.name, loc.kind === 'drives' ? pathIcon(d.name) : dirIcon, () => void tab.navigate(target)));
  }
  if (!items.length) items.push(await MenuItem.new({ text: t('panel.empty'), enabled: false }));
  const menu = await Menu.new({ items });
  await menu.popup(new LogicalPosition(at.x, at.y));
}

/** Popup listing arbitrary locations (breadcrumb overflow). */
export async function popupLocations(list: { label: string; location: Location }[], tab: Tab, at: { x: number; y: number }) {
  const items = [];
  for (const l of list) {
    const icon: IconRef = l.location.kind === 'local' ? pathIcon(l.location.path) : dirIcon;
    items.push(await entry(l.label, icon, () => void tab.navigate(l.location)));
  }
  if (!items.length) return;
  const menu = await Menu.new({ items });
  await menu.popup(new LogicalPosition(at.x, at.y));
}
