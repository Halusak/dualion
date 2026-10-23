// Implementations of user commands. Kept free of UI layout concerns: they
// read and change app state, call the backend and open dialogs.

import { getCurrentWindow } from '@tauri-apps/api/window';
import { ask, open as openFileDialog } from '@tauri-apps/plugin-dialog';
import { api, type BackendLocation } from './api';
import { withHostKey } from './remote';
import { copyText } from './clipboard';
import { formatSize } from './format';
import { localeTag, t, what } from './i18n/index.svelte';
import { baseName, childLoc, dirName, isArchiveName, isDiskImage, isResults, joinLocal, locPath, sameLoc, stemOf, extOf } from './paths';
import { isMac, isWindows, shortcutLabel } from './platform';
import { links } from './links';
import { app, Side, type FileClipboard, type SideName } from './state/app.svelte';
import { settings } from './state/settings.svelte';
import { Tab, UP_ENTRY } from './state/tab.svelte';
import { openDialog, showError, toast } from './state/ui.svelte';
import QuickFind from '../components/dialogs/QuickFind.svelte';
import HistoryDialog from '../components/dialogs/HistoryDialog.svelte';
import DiskUsageDialog from '../components/dialogs/DiskUsageDialog.svelte';
import DuplicatesDialog from '../components/dialogs/DuplicatesDialog.svelte';
import { ensureArchiveAccess, mount } from './archives';
import { renameGroup } from './groups';
import { pushHistory } from './state/history.svelte';
import { F, type AppError, type Entry, type RemoteSource, type JobRequest, type Location, type SortKey, type ViewMode } from './types';

import AboutDialog from '../components/dialogs/AboutDialog.svelte';
import ChecksumDialog from '../components/dialogs/ChecksumDialog.svelte';
import CopyDialog, { type CopyResult } from '../components/dialogs/CopyDialog.svelte';
import FtpDialog from '../components/dialogs/FtpDialog.svelte';
import HelpDialog from '../components/dialogs/HelpDialog.svelte';
import HotlistDialog from '../components/dialogs/HotlistDialog.svelte';
import MultiRenameDialog from '../components/dialogs/MultiRenameDialog.svelte';
import PackDialog, { type PackResult } from '../components/dialogs/PackDialog.svelte';
import PromptDialog, { type PromptResult } from '../components/dialogs/PromptDialog.svelte';
import SearchDialog from '../components/dialogs/SearchDialog.svelte';
import SettingsDialog from '../components/dialogs/SettingsDialog.svelte';
import Viewer from '../components/dialogs/Viewer.svelte';
import type { Connected } from './types';

const nothing = () => toast(t('misc.noSelection'));

function backend(loc: Location): BackendLocation | null {
  return isResults(loc) ? null : (loc as BackendLocation);
}

function writable(loc: Location): boolean {
  return loc.kind === 'local' || loc.kind === 'remote';
}

/** Full location of an entry in a tab (handles search results). */
export function entryLoc(tab: Tab, e: Entry): Location {
  return childLoc(tab.location, e);
}

/** Local file system path of an entry, materializing remote/archived files. */
async function localPathOf(tab: Tab, e: Entry): Promise<string> {
  const loc = entryLoc(tab, e);
  if (loc.kind === 'local') return loc.path;
  const b = backend(loc);
  if (!b) throw new Error('unsupported');
  return api.materialize(b);
}

// ---------------------------------------------------------------- navigation

export async function openEntry(tab: Tab, e: Entry | undefined, opts: { external?: boolean; inside?: boolean } = {}) {
  if (!e) return;
  if (e.flags & F.UP) return tab.up();
  const loc = tab.location;
  if (loc.kind === 'drives') return void tab.navigate({ kind: 'local', path: e.name });
  if (loc.kind === 'network') return void openNetwork(tab, loc.host, e.name);
  // Several marked files: open them all (after a confirmation).
  if (!e.isDir && tab.marked.has(e.key)) {
    const files = tab.targets().filter((x) => !x.isDir && !(x.flags & F.UP));
    if (files.length > 1) return void openMany(tab, files);
  }
  const child = entryLoc(tab, e);
  if (e.isDir || (opts.inside && e.flags & F.PACKAGE)) return void tab.navigate(child);
  if (!opts.external && child.kind === 'local' && isArchiveName(e.name)) {
    return void tab.navigate({ kind: 'archive', archive: child.path, inner: '' });
  }
  try {
    const path = child.kind === 'local' ? child.path : await api.materialize(child as BackendLocation);
    await api.openPath(path);
  } catch (err) {
    showError(err);
  }
}

/** Opens marked files one after another in their default applications. */
async function openMany(tab: Tab, files: Entry[]) {
  const ok = await ask(t('open.manyAsk', { n: files.length }), { title: t('open.manyTitle'), kind: 'info', okLabel: t('open.manyOk'), cancelLabel: t('cancel') });
  if (!ok) return;
  for (const f of files) {
    try {
      const loc = entryLoc(tab, f);
      await api.openPath(loc.kind === 'local' ? loc.path : await api.materialize(loc as BackendLocation));
    } catch (err) {
      showError(err);
    }
    // A short pause keeps applications from choking on a burst of requests.
    await new Promise((r) => setTimeout(r, 150));
  }
}

/**
 * Network place: a computer shows its shares (signing in first when the
 * computer asks for it), a share is connected and opened as a folder.
 */
async function openNetwork(tab: Tab, host: string | undefined, name: string) {
  try {
    // Network shortcuts and connected shares are plain paths ("\\\\server\\share").
    if (!host && name.startsWith('\\\\')) return void tab.navigate({ kind: 'local', path: name });
    if (!host) {
      const target: Location = { kind: 'network', host: name };
      try {
        await api.listDir(target);
      } catch (err) {
        if ((err as AppError)?.code !== 'PermissionDenied') throw err;
        // Windows signs in to the computer; macOS lets the user pick shares and mounts them.
        const path = await api.networkConnect(name, null);
        if (!isWindows) return void tab.navigate({ kind: 'local', path });
      }
      return void tab.navigate(target);
    }
    toast(t('net.connecting', { name: `${host}/${name}` }));
    const path = await api.networkConnect(host, name);
    await tab.navigate({ kind: 'local', path });
  } catch (err) {
    if ((err as AppError)?.code !== 'Cancelled') showError(err);
  }
}

export async function goTo(tab: Tab, input: string): Promise<boolean> {
  const text = input.trim();
  if (!text) return false;
  if (tab.location.kind === 'remote' && !/^[A-Za-z]:|^~|^\\\\/.test(text) && (text.startsWith('/') || !text.includes(':'))) {
    const path = text.startsWith('/') ? text : `${tab.location.path.replace(/\/$/, '')}/${text}`;
    return tab.navigate({ kind: 'remote', session: tab.location.session, path });
  }
  try {
    const base = tab.location.kind === 'local' ? tab.location.path : null;
    const r = await api.resolvePath(text, base);
    return tab.navigate(r.location, { focus: r.focus });
  } catch (err) {
    showError(err);
    return false;
  }
}

/** Opens the folder (or selects the file) whose path is in the clipboard. */
export async function goToClipboard(tab = app.panel) {
  let text = '';
  try {
    text = await api.clipboardText();
  } catch {
    /* empty or not text */
  }
  // First line, without quotes and file:// prefix.
  text = (text.split(/\r?\n/).find((l) => l.trim()) ?? '').trim().replace(/^["']|["']$/g, '');
  if (text.startsWith('file://')) text = decodeURIComponent(text.slice(7).replace(/^localhost/, ''));
  if (isWindows && /^\/[A-Za-z]:/.test(text)) text = text.slice(1);
  if (!text) return toast(t('nav.clipboardEmpty'));
  await goTo(tab, text);
}

export function goHome(tab = app.panel) {
  return tab.navigate({ kind: 'local', path: app.info.home });
}

export function goRoot(tab = app.panel) {
  const loc = tab.location;
  if (loc.kind === 'local') {
    const m = /^([A-Za-z]:\\|\\\\[^\\]+\\[^\\]+\\?)/.exec(loc.path);
    const root = m ? m[1] : loc.path.startsWith('/Volumes/') ? loc.path.split('/').slice(0, 3).join('/') : '/';
    return tab.navigate({ kind: 'local', path: root });
  }
  if (loc.kind === 'remote') return tab.navigate({ ...loc, path: '/' });
  if (loc.kind === 'archive') return tab.navigate({ ...loc, inner: '' });
}

// ---------------------------------------------------------------- file ops

export async function copyMove(op: 'copy' | 'move', sameDir = false) {
  const src = app.panel;
  const dst = app.other;
  const items = src.targets();
  if (!items.length) return nothing();
  if (src.location.kind === 'drives' || src.location.kind === 'network') return;
  const extracting = src.location.kind === 'archive';
  if (extracting && op === 'move') op = 'copy';
  const dest = sameDir ? src.location : dst.location;
  if (!writable(dest)) return toast(t('err.Unsupported'), 'error');

  const sources = items.map((e) => entryLoc(src, e));
  if (extracting && src.location.kind === 'archive' && !(await ensureArchiveAccess(src.location.archive))) return;
  let result: CopyResult | null = {
    dest,
    newName: null,
    options: { verify: settings.verify, toTrash: false },
    queue: null,
  };
  if (settings.confirmCopy || sameDir) {
    result = await openDialog<CopyResult>(CopyDialog, {
      op,
      names: items.map((e) => e.name),
      sources: extracting ? [] : sources,
      dest,
      sameDir,
      extracting,
      upload: src.location.kind === 'local' && dest.kind === 'remote',
      download: src.location.kind === 'remote' && dest.kind === 'local',
    });
    if (!result) return;
  }
  const req: JobRequest = {
    op: extracting ? 'extract' : op,
    sources,
    dest: result.dest,
    newName: result.newName,
    options: result.options,
    queue: result.queue,
  };
  try {
    await api.jobEnqueue(req);
    src.unmarkAll();
  } catch (err) {
    showError(err);
  }
}

/** Ctrl+V: paste files from the internal clipboard into the active panel. */
export async function pasteFiles() {
  const tab = app.panel;
  // Files copied in Finder / Explorer since our own Copy / Cut win.
  const os = await api.clipboardFiles().catch(() => [] as string[]);
  const ours = app.clipboard?.items.flatMap((l) => (l.kind === 'local' ? [l.path] : [])) ?? [];
  const same = os.length === ours.length && os.every((p) => ours.includes(p));
  const cb: FileClipboard | null =
    os.length && !same ? { op: 'copy', items: os.map((p) => ({ kind: 'local', path: p })), names: os.map(baseName) } : app.clipboard;
  if (!cb || !cb.items.length) return;
  if (!writable(tab.location)) return toast(t('err.Unsupported'), 'error');
  const extracting = cb.items[0].kind === 'archive';
  const first = cb.items[0];
  if (first.kind === 'archive' && !(await ensureArchiveAccess(first.archive))) return;
  try {
    await api.jobEnqueue({
      op: extracting ? 'extract' : cb.op === 'cut' ? 'move' : 'copy',
      sources: cb.items,
      dest: tab.location,
      options: { verify: settings.verify },
    });
    if (cb.op === 'cut') app.clipboard = null;
  } catch (err) {
    showError(err);
  }
}

export function clipboardSet(op: 'copy' | 'cut') {
  const tab = app.panel;
  const items = tab.targets();
  if (!items.length) return nothing();
  const locs = items.map((e) => entryLoc(tab, e));
  app.clipboard = { op, items: locs, names: items.map((e) => e.name) };
  // Local files also go to the system clipboard (paste in Finder / Explorer);
  // otherwise their paths as text.
  const local = locs.flatMap((l) => (l.kind === 'local' ? [l.path] : []));
  if (local.length === locs.length) void api.clipboardSetFiles(local).catch(() => copyText(local.join('\n')));
  else void copyText(locs.map(locPath).join('\n'));
  toast(`${op === 'cut' ? t('cmd.clipCut') : t('cmd.clipCopy')}: ${what(items.map((e) => e.name))}`);
}

export async function deleteItems(permanent: boolean) {
  const tab = app.panel;
  const items = tab.targets();
  if (!items.length) return nothing();
  if (!writable(tab.location) && !isResults(tab.location)) return toast(t('err.Unsupported'), 'error');
  const remote = tab.location.kind === 'remote';
  const toTrash = !permanent && !remote && settings.useTrash;
  const w = what(items.map((e) => e.name));
  if (settings.confirmDelete || permanent || remote) {
    // Dry run (bounded in time) so the user sees what is at stake.
    const sources = items.map((e) => entryLoc(tab, e));
    const plan = await Promise.race([
      api.jobPlan({ op: 'delete', sources, options: {} }).catch(() => null),
      new Promise<null>((r) => setTimeout(() => r(null), 900)),
    ]);
    const locale = localeTag();
    const summary = plan
      ? `\n\n${t('plan.files', { count: plan.files })}${plan.dirs ? `, ${t('plan.dirs', { count: plan.dirs })}` : ''} · ${formatSize(plan.bytes, locale, settings.sizeFormat)}`
      : '';
    const msg = (remote
      ? t('confirm.deleteRemote', { what: w })
      : toTrash
        ? t(isWindows ? 'confirm.recycle' : 'confirm.trash', { what: w })
        : `${t('confirm.delete', { what: w })}\n${t('confirm.deleteDetail')}`) +
      summary +
      (toTrash ? `\n${t('confirm.undoable', { key: shortcutLabel('Mod+Z') })}` : '');
    const ok = await ask(msg, { title: 'Dualion', kind: 'warning', okLabel: t('delete'), cancelLabel: t('cancel') });
    if (!ok) return;
  }
  // Keep the cursor on a neighbour of the deleted block.
  const v = tab.visible;
  const gone = new Set(items.map((e) => e.key));
  let i = tab.cursor;
  while (i < v.length && gone.has(v[i].key)) i++;
  if (i >= v.length) {
    i = tab.cursor;
    while (i > 0 && gone.has(v[i].key)) i--;
  }
  if (v[i]) tab.cursorKey = v[i].key;
  try {
    await api.jobEnqueue({ op: 'delete', sources: items.map((e) => entryLoc(tab, e)), options: { toTrash } });
    tab.unmarkAll();
  } catch (err) {
    showError(err);
  }
}

export async function makeDir() {
  const tab = app.panel;
  const b = backend(tab.location);
  if (!b || !writable(tab.location)) return toast(t('err.Unsupported'), 'error');
  const r = await openDialog<PromptResult>(PromptDialog, { title: t('prompt.mkdir'), label: t('prompt.mkdirLabel'), value: '' });
  if (!r?.value.trim()) return;
  try {
    const existed = tab.all.some((x) => x.name.toLowerCase() === r.value.trim().split(/[\\/]/)[0].toLowerCase());
    const first = await api.makeDir(b, r.value.trim());
    if (b.kind === 'local' && !existed) pushHistory({ label: `${t('history.mkdir')}: ${first}`, actions: [{ type: 'created', path: joinLocal(b.path, first) }] });
    await tab.reload();
    tab.setCursorKey(first);
  } catch (err) {
    showError(err);
  }
}

export async function newFile() {
  const tab = app.panel;
  const b = backend(tab.location);
  if (!b || !writable(tab.location)) return toast(t('err.Unsupported'), 'error');
  const r = await openDialog<PromptResult>(PromptDialog, { title: t('prompt.newFile'), label: t('prompt.newFileLabel'), value: '', selectStem: true });
  const name = r?.value.trim();
  if (!name) return;
  try {
    const path = await api.createFile(b, name);
    if (b.kind === 'local') pushHistory({ label: `${t('history.newFile')}: ${name}`, actions: [{ type: 'created', path }] });
    await tab.reload();
    tab.setCursorKey(name);
    if (b.kind === 'local') await api.editFile(path, settings.editor);
  } catch (err) {
    showError(err);
  }
}

/** Starts inline renaming of the cursor item (F2 / Shift+F6). */
export function startRename() {
  const tab = app.panel;
  const e = tab.current;
  if (!e || e.flags & F.UP || !writable(tab.location)) return;
  tab.renaming = e.key;
}

export async function commitRename(tab: Tab, e: Entry, newName: string) {
  tab.renaming = null;
  const name = newName.trim();
  if (!name || name === e.name) return;
  const b = backend(tab.location);
  if (!b) return;
  // A collapsed group is renamed as a whole: every file keeps its own suffix.
  const members = tab.groupMembers(e.key);
  if (members && !tab.expandedGroups.has(e.key) && b.kind === 'local') {
    try {
      const pairs = renameGroup(members, e, name);
      const res = await api.batchRename(b.path, pairs);
      const ok = res.filter((r) => !r.error);
      pushHistory({ label: `${t('history.rename')}: ${e.name}`, rename: { dir: b.path, pairs: pairs.filter((_, i) => !res[i]?.error).map((p) => ({ from: p.to, to: p.from })) } });
      if (ok.length < pairs.length) toast(t('mr.failed', { count: pairs.length - ok.length }), 'error');
      await tab.reload();
      const lead = pairs.find((p) => p.from === e.name);
      if (lead) tab.setCursorKey(lead.to);
    } catch (err) {
      showError(err);
    }
    return;
  }
  try {
    await api.renameItem(b, e.name, name);
    if (b.kind === 'local') pushHistory({ label: `${t('history.rename')}: ${e.name} → ${name}`, rename: { dir: b.path, pairs: [{ from: name, to: e.name }] } });
    await tab.reload();
    tab.setCursorKey(name);
  } catch (err) {
    showError(err);
  }
}

export async function duplicate() {
  return copyMove('copy', true);
}

export async function viewFile(tab = app.panel) {
  const e = tab.current;
  if (!e || e.flags & F.UP) return;
  if (e.isDir) return calcDirSize(tab, e);
  try {
    const path = await localPathOf(tab, e);
    await openDialog(Viewer, { path, name: e.name });
  } catch (err) {
    showError(err);
  }
}

export async function editFile() {
  const tab = app.panel;
  const e = tab.current;
  if (!e || e.isDir) return;
  try {
    const path = await localPathOf(tab, e);
    await api.editFile(path, settings.editor);
  } catch (err) {
    showError(err);
  }
}

export async function openWith() {
  const tab = app.panel;
  const e = tab.current;
  if (!e || e.flags & F.UP) return;
  const picked = await openFileDialog({
    title: t('prompt.openWith'),
    defaultPath: isMac ? '/Applications' : 'C:\\Program Files',
    multiple: false,
    directory: false,
    filters: isWindows ? [{ name: 'Programs', extensions: ['exe', 'bat', 'cmd'] }] : undefined,
  });
  if (!picked || Array.isArray(picked)) return;
  try {
    await api.openWith(await localPathOf(tab, e), picked);
  } catch (err) {
    showError(err);
  }
}

export async function openTerminal() {
  const loc = app.panel.location;
  const dir = loc.kind === 'local' ? loc.path : app.info.home;
  try {
    await api.openTerminal(dir, settings.terminal);
  } catch (err) {
    showError(err);
  }
}

export async function calcDirSize(tab: Tab, e: Entry) {
  const loc = entryLoc(tab, e);
  if (loc.kind !== 'local' || !e.isDir) return;
  tab.setDirSize(e.key, -1);
  try {
    const r = await api.dirSize(loc.path);
    tab.setDirSize(e.key, r.bytes);
  } catch (err) {
    tab.setDirSize(e.key, 0);
    showError(err);
  }
}

export async function calcAllDirSizes() {
  const tab = app.panel;
  const marked = tab.targets().filter((e) => e.isDir);
  const dirs = marked.length > 1 ? marked : tab.visible.filter((e) => e.isDir && !(e.flags & F.UP));
  let next = 0;
  const worker = async () => {
    while (next < dirs.length) await calcDirSize(tab, dirs[next++]);
  };
  await Promise.all([worker(), worker(), worker()]);
}

export async function showProperties() {
  const tab = app.panel;
  const e = tab.current;
  const loc = e && !(e.flags & F.UP) ? entryLoc(tab, e) : tab.location;
  if (loc.kind !== 'local') return toast(t('misc.notLocal'));
  try {
    await api.showProperties(loc.path);
  } catch (err) {
    showError(err);
  }
}

export async function reveal() {
  const tab = app.panel;
  const e = tab.current;
  const loc = e && !(e.flags & F.UP) ? entryLoc(tab, e) : tab.location;
  if (loc.kind !== 'local') return toast(t('misc.notLocal'));
  try {
    await api.revealPath(loc.path);
  } catch (err) {
    showError(err);
  }
}

export async function quickLook() {
  const tab = app.panel;
  const e = tab.current;
  if (!e || e.flags & F.UP) return;
  try {
    await api.quickLook(await localPathOf(tab, e));
  } catch (err) {
    showError(err);
  }
}

export async function copyPath(nameOnly: boolean) {
  const tab = app.panel;
  const items = tab.targets();
  const text = items.length
    ? items.map((e) => (nameOnly ? e.name : locPath(entryLoc(tab, e)))).join('\n')
    : locPath(tab.location);
  if (await copyText(text)) toast(t('copied'));
}

/** Copies the path of the folder shown in the active panel. */
export async function copyDirPath() {
  const p = locPath(app.panel.location);
  if (p && (await copyText(p))) toast(t('copied'));
}

export async function createLink() {
  const tab = app.panel;
  const e = tab.current;
  const dest = app.other.location;
  if (!e || e.flags & F.UP || tab.location.kind !== 'local' || dest.kind !== 'local') return toast(t('misc.notLocal'));
  const r = await openDialog<PromptResult>(PromptDialog, {
    title: t('prompt.link'),
    label: t('prompt.linkLabel'),
    value: e.name,
    checkbox: t('prompt.linkHard'),
  });
  if (!r?.value.trim()) return;
  try {
    await api.createLink(joinLocal(tab.location.path, e.name), dest.path, r.value.trim(), !!r.checked);
    void app.other.reload();
  } catch (err) {
    showError(err);
  }
}

// ---------------------------------------------------------------- archives

export async function pack() {
  const tab = app.panel;
  const items = tab.targets();
  if (!items.length) return nothing();
  if (tab.location.kind !== 'local' && !isResults(tab.location)) return toast(t('misc.notLocal'));
  const destDir = app.other.location.kind === 'local' ? app.other.location.path : tab.location.kind === 'local' ? tab.location.path : app.info.home;
  const base = items.length === 1 ? stemOf(items[0].name) || items[0].name : tab.title || 'archive';
  const r = await openDialog<PackResult>(PackDialog, {
    names: items.map((e) => e.name),
    path: joinLocal(destDir, `${base}.zip`),
  });
  if (!r) return;
  try {
    await api.jobEnqueue({
      op: 'pack',
      sources: items.map((e) => entryLoc(tab, e)),
      dest: { kind: 'local', path: r.path },
      options: { level: r.level, format: r.format, password: r.password, encryptNames: r.encryptNames },
      queue: r.queue,
    });
    tab.unmarkAll();
  } catch (err) {
    showError(err);
  }
}

export async function unpack() {
  const tab = app.panel;
  const archives = tab.targets().filter((e) => !e.isDir && isArchiveName(e.name));
  if (!archives.length || tab.location.kind !== 'local' && !isResults(tab.location)) return nothing();
  const dest = app.other.location.kind === 'local' ? app.other.location : tab.location;
  const r = await openDialog<CopyResult>(CopyDialog, {
    op: 'copy',
    names: archives.map((e) => e.name),
    dest,
    extracting: true,
    unpack: true,
  });
  if (!r) return;
  for (const a of archives) {
    const loc = entryLoc(tab, a);
    if (loc.kind !== 'local') continue;
    if (!(await ensureArchiveAccess(loc.path))) continue;
    try {
      await api.jobEnqueue({
        op: 'extract',
        sources: [{ kind: 'archive', archive: loc.path, inner: '' }],
        dest: r.dest,
        options: r.options,
        queue: r.queue,
      });
    } catch (err) {
      showError(err);
    }
  }
}

// ---------------------------------------------------------------- tools

/** Mounts the disk image under the cursor (ISO, DMG, VHD …). */
export async function mountImage() {
  const tab = app.panel;
  const e = tab.current;
  if (!e || e.isDir) return nothing();
  const loc = entryLoc(tab, e);
  if (loc.kind !== 'local' || !isDiskImage(e.name)) return toast(t('arc.notImage'));
  await mount(loc.path);
}

/** Disk usage map of the active panel's folder. */
export async function diskUsage() {
  const loc = app.panel.location;
  if (loc.kind !== 'local') return toast(t('misc.notLocal'));
  await openDialog(DiskUsageDialog, { path: loc.path });
}

/** Duplicate finder over both panels' folders. */
export async function duplicates() {
  const roots = [app.panel.location, app.other.location]
    .filter((l): l is { kind: 'local'; path: string } => l.kind === 'local')
    .map((l) => l.path)
    .filter((p, i, a) => a.indexOf(p) === i && !a.some((o, j) => j !== i && p.startsWith(o.replace(/[\\/]+$/, '') + (isWindows ? '\\' : '/'))));
  await openDialog(DuplicatesDialog, { roots: roots.length ? roots : [app.info.home] });
}

export async function showHistory() {
  await openDialog(HistoryDialog, {});
}

/** Instant search over the file index (Spotlight / Everything style). */
export async function quickFind(initial = '') {
  await openDialog(QuickFind, { initial });
}

export function rescanIndex() {
  if (!settings.index.enabled) settings.index.enabled = true;
  void api.indexRescan();
  toast(t('find.rescanning'));
}

export async function search() {
  const tab = app.panel;
  const root = tab.location.kind === 'local' ? tab.location.path : app.info.home;
  await openDialog(SearchDialog, { root });
}

export async function multiRename() {
  const tab = app.panel;
  if (tab.location.kind !== 'local') return toast(t('misc.notLocal'));
  let items = tab.targets();
  if (items.length <= 1) items = tab.visible.filter((e) => !(e.flags & F.UP) && !e.isDir);
  if (!items.length) return nothing();
  const dir = tab.location.path;
  const done = await openDialog<{ dir: string; pairs: { from: string; to: string }[] }>(MultiRenameDialog, { dir, entries: items });
  if (done) {
    pushHistory({ label: `${t('history.rename')}: ${done.pairs.length}×`, rename: { dir, pairs: done.pairs.map((p) => ({ from: p.to, to: p.from })) } });
    await tab.reload();
    tab.unmarkAll();
  }
}

export async function checksums() {
  const tab = app.panel;
  const files = tab.targets().filter((e) => !e.isDir);
  if (!files.length) return nothing();
  const paths: string[] = [];
  for (const f of files) {
    const loc = entryLoc(tab, f);
    if (loc.kind === 'local') paths.push(loc.path);
  }
  if (!paths.length) return toast(t('misc.notLocal'));
  await openDialog(ChecksumDialog, { paths });
}

export async function ftpConnect() {
  // Remember the panels now: closing the dialog moves focus around.
  const tab = app.panel;
  const other = app.other;
  const c = await openDialog<Connected & { localPath?: string }>(FtpDialog, {});
  if (!c) return;
  await tab.navigate({ kind: 'remote', session: c.session, path: c.path || '/' });
  if (c.localPath) await other.navigate({ kind: 'local', path: c.localPath });
  toast(t('ftp.connected', { label: c.label }));
}

/** Opens a saved site in `tab` (Network menu). */
export async function openSite(tab: Tab, siteId: string) {
  try {
    const c = await withHostKey((trust) => api.remoteConnect(siteId, null, trust));
    if (c) await tab.navigate({ kind: 'remote', session: c.session, path: c.path || '/' });
  } catch (err) {
    showError(err);
  }
}

/** Opens an Android device, container or rclone remote in `tab`. */
export async function openSource(tab: Tab, source: RemoteSource) {
  try {
    toast(t('net.connecting', { name: source.name }));
    const c = await api.remoteOpenSource(source);
    await tab.navigate({ kind: 'remote', session: c.session, path: c.path || '/' });
  } catch (err) {
    showError(err);
  }
}

export async function ftpDisconnect() {
  const tab = app.panel;
  const loc = tab.location;
  if (loc.kind !== 'remote') return;
  await api.remoteDisconnect(loc.session);
  for (const tb of app.allTabs()) {
    if (tb.location.kind === 'remote' && tb.location.session === loc.session) await goHome(tb);
  }
}

/** Marks files that differ between the two panels (missing or newer). */
export function compareDirs() {
  const l = app.left.tab;
  const r = app.right.tab;
  const files = (tab: Tab) => new Map(tab.all.filter((e) => !e.isDir).map((e) => [e.name.toLowerCase(), e]));
  const lf = files(l);
  const rf = files(r);
  const lDiff: string[] = [];
  const rDiff: string[] = [];
  for (const [k, e] of lf) {
    const o = rf.get(k);
    if (!o || e.mtime > o.mtime + 2000 || (Math.abs(e.mtime - o.mtime) <= 2000 && e.size !== o.size)) lDiff.push(e.key);
  }
  for (const [k, e] of rf) {
    const o = lf.get(k);
    if (!o || e.mtime > o.mtime + 2000 || (Math.abs(e.mtime - o.mtime) <= 2000 && e.size !== o.size)) rDiff.push(e.key);
  }
  l.unmarkAll();
  r.unmarkAll();
  l.setMarks(lDiff, true);
  r.setMarks(rDiff, true);
  toast(lDiff.length + rDiff.length === 0 ? t('compare.same') : t('compare.result', { left: lDiff.length, right: rDiff.length }));
}

export async function settingsDialog(section?: string) {
  await openDialog(SettingsDialog, section ? { section } : {});
}

export function openDiscord() {
  api.openUrl(links.discord).catch(showError);
}

export async function help() {
  await openDialog(HelpDialog, {});
}

export async function about() {
  await openDialog(AboutDialog, {});
}

export async function hotlistManage() {
  await openDialog(HotlistDialog, {});
}

export async function addToHotlist() {
  const tab = app.panel; // captured before the dialog changes focus
  const loc = tab.location;
  if (loc.kind !== 'local') return toast(t('misc.notLocal'));
  const r = await openDialog<PromptResult>(PromptDialog, {
    title: t('cmd.addHotlist'),
    label: t('prompt.hotlistName'),
    value: tab.title || loc.path,
  });
  if (!r?.value.trim()) return;
  settings.hotlist = [...settings.hotlist.filter((h) => !sameLoc(h.location, loc)), { name: r.value.trim(), location: loc }];
}

// ---------------------------------------------------------------- panels & tabs

export function setView(view: ViewMode) {
  app.panel.view = view;
}

export function setSort(key: SortKey) {
  app.panel.setSort(key);
}

export function swapPanels() {
  const l = app.left;
  app.left = app.right;
  app.right = l;
}

export async function equalPanels() {
  await app.other.navigate(app.panel.location);
}

export async function newTab(sideName: SideName = app.activeSide) {
  const side = app.side(sideName);
  await duplicateTab(side, side.active);
}

export function closeTab(side: Side = app.activeSideObj, index = side.active) {
  if (!side.closeTab(index)) toast(t('tabs.lockedHint'));
}

export function toggleLockTab(side: Side = app.activeSideObj, index = side.active) {
  const tab = side.tabs[index];
  if (tab) side.setLocked(index, !tab.locked);
}

/** Locks the tab to its current folder (switching back returns there); again = unlock. */
export function toggleLockTabFolder(side: Side = app.activeSideObj, index = side.active) {
  const tab = side.tabs[index];
  if (!tab) return;
  if (tab.home) return side.setLocked(index, false);
  const loc = tab.location;
  if (loc.kind !== 'local' && loc.kind !== 'remote' && loc.kind !== 'archive' && loc.kind !== 'drives') return;
  if (!tab.locked) side.setLocked(index, true);
  tab.home = loc;
}

/** Gives a tab its own name; an empty name returns to the folder name. */
export async function renameTab(side: Side = app.activeSideObj, index = side.active) {
  const tab = side.tabs[index];
  if (!tab) return;
  const r = await openDialog<PromptResult>(PromptDialog, { title: t('tabs.rename'), label: t('tabs.renameLabel'), value: tab.customTitle || tab.title });
  if (!r) return;
  const name = r.value.trim();
  tab.customTitle = name === tab.title ? '' : name;
}

export async function duplicateTab(side: Side = app.activeSideObj, index = side.active) {
  const cur = side.tabs[index];
  if (!cur) return;
  const tab = new Tab({ location: cur.location, view: cur.view, sortKey: cur.sortKey, sortDesc: cur.sortDesc });
  side.addTab(tab);
  await tab.navigate(cur.location.kind === 'search' ? { kind: 'local', path: app.info.home } : cur.location, { focus: cur.current?.key });
}

export function cycleTab(delta: number) {
  const s: Side = app.activeSideObj;
  s.active = (s.active + delta + s.tabs.length) % s.tabs.length;
}

export async function toggleFullscreen() {
  const w = getCurrentWindow();
  await w.setFullscreen(!(await w.isFullscreen()));
}

export async function quit() {
  if (app.activeJobs.length > 0) {
    const ok = await ask(t('close.message'), { title: t('close.title'), kind: 'warning', okLabel: t('close.quit'), cancelLabel: t('cancel') });
    if (!ok) return;
  }
  await api.exit();
}

export function showPackageContents() {
  const tab = app.panel;
  const e = tab.current;
  if (e && e.flags & F.PACKAGE) void tab.navigate(entryLoc(tab, e));
}

export function markCurrentAndMove(tab: Tab, delta: number) {
  const e = tab.current;
  if (e) tab.toggleMark(e.key);
  if (delta) tab.setCursor(tab.cursor + delta);
}

export function isUp(e: Entry | undefined) {
  return !e || e.key === UP_ENTRY.key;
}

export { extOf, dirName };
