// Command registry: every user action with its label, shortcuts (per
// platform) and menu placement. Keyboard, native menu, toolbar, function
// key bar and context menus all dispatch through `run()`.

import * as A from './actions';
import { t } from './i18n/index.svelte';
import { isMac, isWindows } from './platform';
import { app } from './state/app.svelte';
import { settings } from './state/settings.svelte';
import { undo } from './state/history.svelte';
import { dialogOpen, openDialog } from './state/ui.svelte';
import PromptDialog from '../components/dialogs/PromptDialog.svelte';
import { F, type MenuNode } from './types';

export interface Command {
  id: string;
  /** i18n key of the label */
  label: string;
  run: () => unknown;
  /** Key combos; the first one is shown in menus. */
  keys?: string[];
  checked?: () => boolean;
  /** Works while a text field has focus (only safe combos). */
  global?: boolean;
}

const both = (k: string[]) => k;
const mac = (m: string[], w: string[]) => (isMac ? m : w);

export const commands: Command[] = [
  // Function keys (Total Commander layout)
  { id: 'help', label: 'cmd.shortcuts', run: A.help, keys: ['F1'], global: true },
  { id: 'rename', label: 'cmd.rename', run: A.startRename, keys: ['F2', 'Shift+F6'] },
  { id: 'view', label: 'cmd.view', run: () => A.viewFile(), keys: ['F3'] },
  { id: 'edit', label: 'cmd.edit', run: A.editFile, keys: ['F4'] },
  { id: 'newFile', label: 'cmd.newFile', run: A.newFile, keys: ['Shift+F4'] },
  { id: 'copy', label: 'cmd.copy', run: () => A.copyMove('copy'), keys: ['F5'] },
  { id: 'duplicate', label: 'cmd.duplicate', run: A.duplicate, keys: ['Shift+F5'] },
  { id: 'move', label: 'cmd.move', run: () => A.copyMove('move'), keys: ['F6'] },
  { id: 'mkdir', label: 'cmd.mkdir', run: A.makeDir, keys: ['F7', 'Mod+Shift+N'] },
  { id: 'delete', label: 'cmd.delete', run: () => A.deleteItems(false), keys: mac(['F8', 'Mod+Backspace'], ['F8', 'Delete']) },
  {
    id: 'deletePermanent',
    label: 'cmd.deletePermanent',
    run: () => A.deleteItems(true),
    keys: mac(['Shift+F8', 'Mod+Alt+Backspace'], ['Shift+F8', 'Shift+Delete']),
  },
  { id: 'terminal', label: 'cmd.terminal', run: A.openTerminal, keys: ['F9'], global: true },
  { id: 'quit', label: 'cmd.quit', run: A.quit, keys: ['F10'], global: true },
  { id: 'fullscreen', label: 'cmd.fullscreen', run: A.toggleFullscreen, keys: mac(['Mod+Ctrl+F'], ['F11']), global: true },
  { id: 'quickView', label: 'cmd.quickView', run: () => (app.quickView = !app.quickView), keys: ['F12', 'Ctrl+Q'], checked: () => app.quickView, global: true },

  // Files
  { id: 'open', label: 'cmd.open', run: () => A.openEntry(app.panel, app.panel.current), keys: mac(['Mod+ArrowDown'], []) },
  { id: 'openWith', label: 'cmd.openWith', run: A.openWith },
  { id: 'properties', label: isMac ? 'cmd.propertiesMac' : 'cmd.propertiesWin', run: A.showProperties, keys: mac(['Mod+I'], ['Alt+Enter']) },
  { id: 'reveal', label: isMac ? 'cmd.revealMac' : 'cmd.revealWin', run: A.reveal, keys: ['Mod+Alt+R'] },
  { id: 'quickLook', label: 'cmd.quickLook', run: A.quickLook, keys: mac(['Mod+Y'], []) },
  { id: 'pack', label: 'cmd.pack', run: A.pack, keys: ['Alt+F5'] },
  { id: 'unpack', label: 'cmd.unpack', run: A.unpack, keys: ['Alt+F9'] },
  { id: 'checksum', label: 'cmd.checksum', run: A.checksums },
  { id: 'symlink', label: 'cmd.symlink', run: A.createLink, keys: ['Mod+Alt+L'] },
  { id: 'showPackage', label: 'cmd.showPackage', run: A.showPackageContents },

  // Edit / clipboard
  { id: 'clipCopy', label: 'cmd.clipCopy', run: () => A.clipboardSet('copy'), keys: ['Mod+C'] },
  { id: 'clipCut', label: 'cmd.clipCut', run: () => A.clipboardSet('cut'), keys: ['Mod+X'] },
  { id: 'clipPaste', label: 'cmd.clipPaste', run: A.pasteFiles, keys: ['Mod+V'] },
  { id: 'copyPath', label: 'cmd.copyPath', run: () => A.copyPath(false), keys: ['Mod+Shift+C'] },
  { id: 'copyName', label: 'cmd.copyName', run: () => A.copyPath(true), keys: ['Mod+Alt+C'] },
  { id: 'undo', label: 'cmd.undo', run: () => undo(), keys: ['Mod+Z'] },
  { id: 'history', label: 'cmd.history', run: A.showHistory, keys: ['Mod+Shift+Z'] },

  // Selection
  { id: 'selectAll', label: 'cmd.selectAll', run: () => app.panel.markAll(), keys: ['Mod+A'] },
  { id: 'deselectAll', label: 'cmd.deselectAll', run: () => app.panel.unmarkAll(), keys: ['Mod+Shift+A'] },
  { id: 'invertSelection', label: 'cmd.invertSelection', run: () => app.panel.invertMarks(), keys: ['NumMul', 'Mod+Shift+I'] },
  { id: 'selectMask', label: 'cmd.selectMask', run: () => maskDialog(true), keys: ['NumAdd', 'Mod+='] },
  { id: 'deselectMask', label: 'cmd.deselectMask', run: () => maskDialog(false), keys: ['NumSub', 'Mod+-'] },
  { id: 'selectSameExt', label: 'cmd.selectSameExt', run: () => app.panel.markSameExt(), keys: ['Alt+NumAdd', 'Mod+Alt+='] },
  { id: 'compareDirs', label: 'cmd.compareDirs', run: A.compareDirs, keys: ['Shift+F2'] },

  // Tools
  { id: 'search', label: 'cmd.search', run: A.search, keys: ['Alt+F7', 'Mod+F'], global: true },
  { id: 'quickFind', label: 'cmd.quickFind', run: () => A.quickFind(), keys: ['Mod+P'], global: true },
  { id: 'palette', label: 'cmd.palette', run: () => A.quickFind('>'), keys: ['Mod+Shift+P'], global: true },
  { id: 'diskUsage', label: 'cmd.diskUsage', run: () => A.diskUsage(), keys: ['Mod+Alt+U'] },
  { id: 'duplicates', label: 'cmd.duplicates', run: () => A.duplicates(), keys: ['Mod+Alt+D'] },
  { id: 'rescanIndex', label: 'cmd.rescanIndex', run: () => A.rescanIndex() },
  { id: 'multiRename', label: 'cmd.multiRename', run: A.multiRename, keys: ['Mod+M'] },
  { id: 'dirSizes', label: 'cmd.dirSizes', run: A.calcAllDirSizes, keys: ['Alt+Shift+Enter'] },
  { id: 'swapPanels', label: 'cmd.swapPanels', run: A.swapPanels, keys: ['Mod+U'] },
  { id: 'equalPanels', label: 'cmd.equalPanels', run: A.equalPanels, keys: ['Mod+E'] },
  { id: 'jobs', label: 'cmd.jobs', run: () => (app.showJobs = !app.showJobs), keys: ['Mod+J'], checked: () => app.showJobs, global: true },
  { id: 'settings', label: 'cmd.settings', run: () => A.settingsDialog(), keys: ['Mod+,'], global: true },
  { id: 'about', label: 'cmd.about', run: A.about },
  { id: 'discord', label: 'cmd.discord', run: A.openDiscord },
  { id: 'donate', label: 'cmd.donate', run: () => A.settingsDialog('aboutSection') },

  // Network
  { id: 'ftpConnect', label: 'cmd.ftpConnect', run: A.ftpConnect, keys: ['Mod+K'], global: true },
  { id: 'ftpDisconnect', label: 'cmd.ftpDisconnect', run: A.ftpDisconnect, keys: ['Mod+Shift+K'] },

  // View
  { id: 'viewDetails', label: 'cmd.viewDetails', run: () => A.setView('details'), keys: ['Mod+1', ...(isWindows ? ['Ctrl+F2'] : [])], checked: () => app.panel.view === 'details' },
  { id: 'viewList', label: 'cmd.viewList', run: () => A.setView('list'), keys: ['Mod+2', ...(isWindows ? ['Ctrl+F1'] : [])], checked: () => app.panel.view === 'list' },
  { id: 'viewIcons', label: 'cmd.viewIcons', run: () => A.setView('icons'), keys: ['Mod+3'], checked: () => app.panel.view === 'icons' },
  { id: 'viewThumbs', label: 'cmd.viewThumbs', run: () => A.setView('thumbs'), keys: ['Mod+4'], checked: () => app.panel.view === 'thumbs' },
  { id: 'sortName', label: 'cmd.sortName', run: () => A.setSort('name'), keys: mac(['Mod+Alt+1'], ['Ctrl+F3']), checked: () => app.panel.sortKey === 'name' },
  { id: 'sortExt', label: 'cmd.sortExt', run: () => A.setSort('ext'), keys: mac(['Mod+Alt+2'], ['Ctrl+F4']), checked: () => app.panel.sortKey === 'ext' },
  { id: 'sortDate', label: 'cmd.sortDate', run: () => A.setSort('mtime'), keys: mac(['Mod+Alt+3'], ['Ctrl+F5']), checked: () => app.panel.sortKey === 'mtime' },
  { id: 'sortSize', label: 'cmd.sortSize', run: () => A.setSort('size'), keys: mac(['Mod+Alt+4'], ['Ctrl+F6']), checked: () => app.panel.sortKey === 'size' },
  { id: 'sortReverse', label: 'cmd.sortReverse', run: () => (app.panel.sortDesc = !app.panel.sortDesc), checked: () => app.panel.sortDesc },
  { id: 'toggleHidden', label: 'cmd.toggleHidden', run: () => (settings.showHidden = !settings.showHidden), keys: mac(['Mod+Shift+.'], ['Mod+H']), checked: () => settings.showHidden },
  { id: 'toggleToolbar', label: 'cmd.toggleToolbar', run: () => (settings.toolbar = !settings.toolbar), checked: () => settings.toolbar },
  { id: 'wrapNames', label: 'cmd.wrapNames', run: () => (settings.nameLines = settings.nameLines > 1 ? 1 : 2), checked: () => settings.nameLines > 1 },
  { id: 'toggleGroups', label: 'cmd.toggleGroups', run: () => (settings.groupFiles = !settings.groupFiles), keys: ['Mod+Alt+G'], checked: () => settings.groupFiles },
  { id: 'toggleDriveBar', label: 'cmd.toggleDriveBar', run: () => (settings.driveBar = !settings.driveBar), checked: () => settings.driveBar },
  { id: 'toggleFunctionBar', label: 'cmd.toggleFunctionBar', run: () => (settings.functionBar = !settings.functionBar), checked: () => settings.functionBar },
  { id: 'refresh', label: 'cmd.refresh', run: () => app.panel.reload(), keys: ['Mod+R'] },
  { id: 'filter', label: 'cmd.filter', run: () => (app.panel.filterOpen = true), keys: ['Mod+S'] },

  // Go
  { id: 'back', label: 'cmd.back', run: () => app.panel.back(), keys: both(isMac ? ['Mod+[', 'Alt+ArrowLeft'] : ['Alt+ArrowLeft']) },
  { id: 'forward', label: 'cmd.forward', run: () => app.panel.forward(), keys: both(isMac ? ['Mod+]', 'Alt+ArrowRight'] : ['Alt+ArrowRight']) },
  { id: 'parent', label: 'cmd.parent', run: () => app.panel.up(), keys: mac(['Mod+ArrowUp', 'Backspace'], ['Backspace', 'Alt+ArrowUp', 'Mod+PageUp']) },
  { id: 'root', label: 'cmd.root', run: () => A.goRoot(), keys: ['Mod+\\'] },
  { id: 'home', label: 'cmd.home', run: () => A.goHome(), keys: ['Mod+Shift+H'] },
  { id: 'goTo', label: 'cmd.goTo', run: () => (app.editPath = app.activeSide), keys: mac(['Mod+L', 'Mod+Shift+G'], ['Mod+L', 'Alt+D']), global: true },
  { id: 'drivesLeft', label: 'cmd.drivesLeft', run: () => (app.placesFor = 'left'), keys: ['Alt+F1'] },
  { id: 'drivesRight', label: 'cmd.drivesRight', run: () => (app.placesFor = 'right'), keys: ['Alt+F2'] },
  { id: 'hotlist', label: 'cmd.hotlist', run: () => (app.placesFor = app.activeSide), keys: ['Mod+D'] },
  { id: 'addHotlist', label: 'cmd.addHotlist', run: A.addToHotlist, keys: ['Mod+Shift+D'] },
  { id: 'manageHotlist', label: 'cmd.manageHotlist', run: A.hotlistManage },
  { id: 'newTab', label: 'cmd.newTab', run: () => A.newTab(), keys: ['Mod+T'] },
  { id: 'closeTab', label: 'cmd.closeTab', run: () => A.closeTab(), keys: ['Mod+W'] },
  { id: 'lockTab', label: 'cmd.lockTab', run: () => A.toggleLockTab(), keys: ['Mod+Shift+L'], checked: () => app.panel.locked },
  { id: 'duplicateTab', label: 'cmd.duplicateTab', run: () => A.duplicateTab() },
  { id: 'closeOtherTabs', label: 'cmd.closeOtherTabs', run: () => app.activeSideObj.closeOthers() },
  { id: 'nextTab', label: 'cmd.nextTab', run: () => A.cycleTab(1), keys: isMac ? ['Ctrl+Tab', 'Mod+Shift+]'] : ['Ctrl+Tab'] },
  { id: 'prevTab', label: 'cmd.prevTab', run: () => A.cycleTab(-1), keys: isMac ? ['Ctrl+Shift+Tab', 'Mod+Shift+['] : ['Ctrl+Shift+Tab'] },
  { id: 'focusOther', label: 'cmd.focusOther', run: () => (app.activeSide = app.otherSide), keys: ['Tab'] },
];

const byId = new Map(commands.map((c) => [c.id, c]));
export const command = (id: string) => byId.get(id);

/** Effective shortcuts of a command (user override or default). */
export function keysOf(c: Command | undefined): string[] {
  if (!c) return [];
  return settings.keymap[c.id] ?? c.keys ?? [];
}

/** Label with context for lists outside menus ("Sort By: Name"). */
export function fullLabel(c: Command): string {
  if (c.id.startsWith('sort') && c.id !== 'sortReverse') return `${t('menu.sort')}: ${t(c.label)}`;
  if (['viewDetails', 'viewList', 'viewIcons', 'viewThumbs'].includes(c.id)) return `${t('menu.view')}: ${t(c.label)}`;
  return t(c.label);
}

export function defaultKeysOf(id: string): string[] {
  return byId.get(id)?.keys ?? [];
}

let bindingCache: { sig: string; map: Map<string, Command> } | null = null;

/** Key combo -> command, rebuilt whenever the user keymap changes. */
function bindings(): Map<string, Command> {
  const sig = JSON.stringify(settings.keymap);
  if (!bindingCache || bindingCache.sig !== sig) {
    const map = new Map<string, Command>();
    for (const c of commands) for (const k of keysOf(c)) if (!map.has(k)) map.set(k, c);
    bindingCache = { sig, map };
  }
  return bindingCache.map;
}

/** Command currently bound to `combo`, if any. */
export function commandForCombo(combo: string): Command | undefined {
  return bindings().get(combo);
}

/** Assigns `combo` to command `id`, taking it away from any other command. */
export function assignShortcut(id: string, combo: string) {
  const km = { ...settings.keymap };
  const other = commandForCombo(combo);
  if (other && other.id !== id) km[other.id] = keysOf(other).filter((k) => k !== combo);
  const mine = keysOf(byId.get(id)).filter((k) => k !== combo);
  km[id] = [combo, ...mine];
  settings.keymap = km;
}

export function removeShortcut(id: string, combo: string) {
  settings.keymap = { ...settings.keymap, [id]: keysOf(byId.get(id)).filter((k) => k !== combo) };
}

export function resetShortcuts(id?: string) {
  if (!id) {
    settings.keymap = {};
    return;
  }
  const km = { ...settings.keymap };
  delete km[id];
  settings.keymap = km;
}

/** Combos that must keep their meaning inside text fields. */
const textEditing = new Set(['Mod+A', 'Mod+C', 'Mod+V', 'Mod+X', 'Mod+Z', 'Mod+Backspace', 'Mod+Alt+Backspace', 'Backspace', 'Delete', 'Shift+Delete', 'Tab', 'Mod+ArrowUp', 'Mod+ArrowDown', 'Alt+ArrowLeft', 'Alt+ArrowRight', 'Mod+[', 'Mod+]', 'Mod+=', 'Mod+-']);

// ---------------------------------------------------------------- keyboard

const codeKeys: Record<string, string> = {
  Space: 'Space',
  NumpadAdd: 'NumAdd',
  NumpadSubtract: 'NumSub',
  NumpadMultiply: 'NumMul',
  NumpadEnter: 'Enter',
  Period: '.',
  Comma: ',',
  Slash: '/',
  Backslash: '\\',
  BracketLeft: '[',
  BracketRight: ']',
  Minus: '-',
  Equal: '=',
  Backquote: '`',
  Semicolon: ';',
  Quote: "'",
};

export function keyOf(e: KeyboardEvent): string {
  if (codeKeys[e.code]) return codeKeys[e.code];
  if (/^Digit\d$/.test(e.code)) return e.code.slice(5);
  if (/^F\d{1,2}$/.test(e.code)) return e.code;
  if (e.key.length === 1 && /[a-z]/i.test(e.key)) return e.key.toUpperCase();
  if (/^Key[A-Z]$/.test(e.code)) return e.code.slice(3);
  return e.key;
}

export function comboOf(e: KeyboardEvent): string {
  const parts: string[] = [];
  if (isMac) {
    if (e.metaKey) parts.push('Mod');
    if (e.ctrlKey) parts.push('Ctrl');
  } else {
    if (e.ctrlKey) parts.push('Mod');
    if (e.metaKey) parts.push('Meta');
  }
  if (e.altKey) parts.push('Alt');
  if (e.shiftKey) parts.push('Shift');
  parts.push(keyOf(e));
  return parts.join('+');
}

/** Ctrl+Tab on Windows is "Mod+Tab" after normalization; accept both spellings. */
function lookup(combo: string): Command | undefined {
  const b = bindings();
  return b.get(combo) ?? (isWindows ? b.get(combo.replace(/^Mod\+/, 'Ctrl+')) : undefined);
}

export function isTextTarget(el: EventTarget | null): boolean {
  if (!(el instanceof HTMLElement)) return false;
  if (el.isContentEditable) return true;
  if (el instanceof HTMLTextAreaElement) return true;
  if (el instanceof HTMLInputElement) return !['checkbox', 'radio', 'button', 'submit', 'range'].includes(el.type);
  if (el instanceof HTMLSelectElement) return true;
  return false;
}

let lastRun = { id: '', at: 0 };

/** Runs a command unless the same one ran a moment ago (keyboard + native accelerator). */
export function run(id: string) {
  const now = performance.now();
  if (lastRun.id === id && now - lastRun.at < 250) return;
  lastRun = { id, at: now };
  const c = byId.get(id);
  if (!c) return;
  try {
    const r = c.run();
    if (r instanceof Promise) r.catch((e) => console.error(id, e));
  } catch (e) {
    console.error(id, e);
  }
}

/** Global key handler. Returns true when the event was consumed. */
export function handleKey(e: KeyboardEvent): boolean {
  if (dialogOpen()) return false;
  const combo = comboOf(e);
  const c = lookup(combo);
  if (!c) return false;
  if (isTextTarget(e.target)) {
    const fkey = /(^|\+)F\d{1,2}$/.test(combo);
    if (!(c.global || fkey) || textEditing.has(combo)) return false;
  }
  e.preventDefault();
  run(c.id);
  return true;
}

export function shortcutOf(id: string): string | undefined {
  return keysOf(byId.get(id))[0];
}

// ---------------------------------------------------------------- native menu

/** Converts our combo to a Tauri accelerator; null for combos menus must not grab. */
function accel(combo: string | undefined): string | null {
  if (!combo) return null;
  const parts = combo.split('+');
  const key = parts.pop()!;
  if (combo.endsWith('++')) return null;
  const mods = parts;
  const keyMap: Record<string, string> = { ',': 'Comma', '.': 'Period', '[': 'BracketLeft', ']': 'BracketRight', '\\': 'Backslash', '=': 'Equal', '-': 'Minus' };
  const isF = /^F\d{1,2}$/.test(key);
  const hasMod = mods.some((m) => m === 'Mod' || m === 'Ctrl' || m === 'Alt');
  // Plain keys (Enter, Backspace, Tab, letters…) would steal typing from text fields.
  if (!isF && !hasMod) return null;
  if (textEditing.has(combo)) return null;
  if (key.startsWith('Num') || key.startsWith('Arrow') || key === 'Enter' || key === 'Tab') return null;
  const k = keyMap[key] ?? key;
  const m = mods.map((x) => (x === 'Mod' ? 'CmdOrCtrl' : x === 'Ctrl' ? (isMac ? 'Ctrl' : 'CmdOrCtrl') : x));
  return [...m, k].join('+');
}

/** Accelerator text for menus built in the frontend (context menus). */
export function menuAccel(id: string): string | null {
  return accel(keysOf(byId.get(id))[0]);
}

function item(id: string, opts: { noAccel?: boolean } = {}): MenuNode {
  const c = byId.get(id)!;
  const acc = opts.noAccel ? null : accel(keysOf(c)[0]);
  if (c.checked) return { type: 'check', id, label: t(c.label), accel: acc, checked: c.checked() };
  return { type: 'item', id, label: t(c.label), accel: acc };
}

const sep: MenuNode = { type: 'separator' };
const pre = (name: string, label?: string): MenuNode => ({ type: 'predefined', name, label: label ? t(label) : null });

export function buildMenu(): MenuNode[] {
  const menu: MenuNode[] = [];
  if (isMac) {
    menu.push({
      type: 'submenu',
      label: 'Dualion',
      items: [pre('about', 'sys.about'), sep, item('settings'), sep, pre('services', 'sys.services'), sep, pre('hide', 'sys.hide'), pre('hideOthers', 'sys.hideOthers'), pre('showAll', 'sys.showAll'), sep, pre('quit', 'sys.quit')],
    });
  }
  menu.push({
    type: 'submenu',
    label: t('menu.file'),
    items: [
      item('open'),
      item('openWith'),
      item('view'),
      item('edit'),
      sep,
      item('newFile'),
      item('mkdir'),
      sep,
      item('copy'),
      item('move'),
      item('duplicate'),
      item('rename'),
      item('delete'),
      item('deletePermanent'),
      sep,
      item('pack'),
      item('unpack'),
      item('checksum'),
      item('symlink'),
      sep,
      item('properties'),
      item('reveal'),
      ...(isMac ? [item('quickLook')] : [sep, item('settings'), sep, item('quit')]),
    ],
  });
  menu.push({
    type: 'submenu',
    label: t('menu.edit'),
    items: isMac
      ? [pre('undo', 'sys.undo'), pre('redo', 'sys.redo'), sep, pre('cut', 'sys.cut'), pre('copy', 'sys.copy'), pre('paste', 'sys.paste'), pre('selectAll', 'sys.selectAll'), sep, item('undo'), item('history'), item('copyPath'), item('copyName')]
      : [item('undo', { noAccel: true }), item('history'), sep, item('clipCut', { noAccel: true }), item('clipCopy', { noAccel: true }), item('clipPaste', { noAccel: true }), sep, item('copyPath'), item('copyName')],
  });
  menu.push({
    type: 'submenu',
    label: t('menu.mark'),
    items: [item('selectAll', { noAccel: true }), item('deselectAll'), item('invertSelection'), sep, item('selectMask'), item('deselectMask'), item('selectSameExt'), sep, item('compareDirs')],
  });
  menu.push({
    type: 'submenu',
    label: t('menu.commands'),
    items: [item('quickFind'), item('palette'), item('search'), item('multiRename'), item('dirSizes'), item('diskUsage'), item('duplicates'), sep, item('terminal'), sep, item('swapPanels'), item('equalPanels'), sep, item('jobs')],
  });
  menu.push({ type: 'submenu', label: t('menu.net'), items: [item('ftpConnect'), item('ftpDisconnect')] });
  menu.push({
    type: 'submenu',
    label: t('menu.view'),
    items: [
      item('viewDetails'),
      item('viewList'),
      item('viewIcons'),
      item('viewThumbs'),
      sep,
      { type: 'submenu', label: t('menu.sort'), items: [item('sortName'), item('sortExt'), item('sortDate'), item('sortSize'), sep, item('sortReverse')] },
      sep,
      item('toggleHidden'),
      item('toggleGroups'),
      item('wrapNames'),
      item('quickView'),
      item('filter'),
      sep,
      item('toggleToolbar'),
      item('toggleDriveBar'),
      item('toggleFunctionBar'),
      sep,
      item('refresh'),
      ...(isMac ? [] : [item('fullscreen')]),
    ],
  });
  menu.push({
    type: 'submenu',
    label: t('menu.go'),
    items: [
      item('back'),
      item('forward'),
      item('parent', { noAccel: true }),
      item('root'),
      item('home'),
      item('goTo'),
      sep,
      item('drivesLeft'),
      item('drivesRight'),
      sep,
      item('hotlist'),
      item('addHotlist'),
      item('manageHotlist'),
      sep,
      item('newTab'),
      item('duplicateTab'),
      item('lockTab'),
      item('closeTab'),
      item('closeOtherTabs'),
      item('nextTab', { noAccel: true }),
      item('prevTab', { noAccel: true }),
    ],
  });
  if (isMac) {
    menu.push({
      type: 'submenu',
      label: t('menu.window'),
      role: 'window',
      items: [pre('minimize', 'sys.minimize'), pre('maximize', 'sys.zoom'), pre('fullscreen', 'sys.fullscreen'), sep, pre('bringAllToFront', 'sys.bringAll')],
    });
  }
  menu.push({
    type: 'submenu',
    label: t('menu.help'),
    role: 'help',
    items: [item('help'), sep, item('discord'), item('donate'), ...(isMac ? [] : [sep, item('about')])],
  });
  return menu;
}

// ---------------------------------------------------------------- helpers

async function maskDialog(select: boolean) {
  const r = await openDialog<{ value: string; checked?: boolean }>(PromptDialog, {
    title: t(select ? 'prompt.mask' : 'prompt.unmask'),
    label: t('prompt.maskLabel'),
    value: '*.*',
    checkbox: t('prompt.maskDirs'),
  });
  if (r) app.panel.markByMask(r.value, select, !!r.checked);
}

/** Function key bar entries (TC style). */
export const functionKeys: { key: string; label: string; cmd: string }[] = [
  { key: 'F1', label: 'fkey.help', cmd: 'help' },
  { key: 'F2', label: 'fkey.rename', cmd: 'rename' },
  { key: 'F3', label: 'fkey.view', cmd: 'view' },
  { key: 'F4', label: 'fkey.edit', cmd: 'edit' },
  { key: 'F5', label: 'fkey.copy', cmd: 'copy' },
  { key: 'F6', label: 'fkey.move', cmd: 'move' },
  { key: 'F7', label: 'fkey.mkdir', cmd: 'mkdir' },
  { key: 'F8', label: 'fkey.delete', cmd: 'delete' },
  { key: 'F9', label: 'fkey.terminal', cmd: 'terminal' },
  { key: 'F10', label: 'fkey.quit', cmd: 'quit' },
];

export { F };
