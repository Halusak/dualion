import { api, type BackendLocation } from '../api';
import { localeTag } from '../i18n/index.svelte';
import { autoColumns, columnById } from '../columns';
import { groupEntries, type Grouped } from '../groups';
import { baseName, dirName, extOf, joinLocal, maskMatcher, parentLoc, sameLoc } from '../paths';
import { sortEntries } from '../sort';
import {
  F,
  type AppError,
  type Entry,
  type Listing,
  type Location,
  type RawEntry,
  type SearchHit,
  type SortKey,
  type Space,
  type ViewMode,
  type Volume,
} from '../types';
import type { Side } from './app.svelte';
import { metaFor, metaPeek } from './meta.svelte';
import { settings } from './settings.svelte';
import { showError } from './ui.svelte';

let nextId = 1;

export const UP_ENTRY: Entry = {
  name: '..',
  flags: F.DIR | F.UP,
  size: 0,
  mtime: 0,
  attrs: 0,
  ext: '',
  isDir: true,
  key: '\u0000..',
};

export function toEntry(r: RawEntry): Entry {
  const [name, flags, size, mtime, attrs] = r;
  const isDir = (flags & F.DIR) !== 0 && (flags & F.PACKAGE) === 0;
  return { name, flags, size, mtime, attrs, ext: isDir ? '' : extOf(name), isDir, key: name };
}

function lastName(loc: Location): string | null {
  switch (loc.kind) {
    case 'local':
    case 'ftp':
      return baseName(loc.path) || null;
    case 'archive':
      return loc.inner ? baseName(loc.inner) : baseName(loc.archive);
    default:
      return null;
  }
}

/** Search results kept per id so Back/Forward can return to them. */
const searchResults = new Map<string, { label: string; hits: Entry[] }>();

export interface TabSnapshot {
  location: Location;
  view: ViewMode;
  sortKey: SortKey;
  sortDesc: boolean;
  locked?: boolean;
}

export class Tab {
  readonly id = nextId++;
  location = $state.raw<Location>({ kind: 'drives' });
  listing = $state.raw<Listing | null>(null);
  all = $state.raw<Entry[]>([]);
  view = $state<ViewMode>('details');
  sortKey = $state<SortKey>('name');
  sortDesc = $state(false);
  filter = $state('');
  filterOpen = $state(false);
  cursorKey = $state<string | null>(null);
  marked = $state.raw<Set<string>>(new Set());
  loading = $state(false);
  error = $state.raw<AppError | null>(null);
  dirSizes = $state.raw<Map<string, number>>(new Map());
  space = $state.raw<Space | null>(null);
  volumes = $state.raw<Volume[]>([]);
  /** Keys highlighted by "Compare folders". */
  diff = $state.raw<Set<string>>(new Set());
  /** Bumped to ask the list to scroll the cursor into view. */
  revealTick = $state(0);
  /** Key of the entry being renamed inline. */
  renaming = $state<string | null>(null);
  /** Locked tabs keep their folder: going elsewhere opens a new tab. */
  locked = $state(false);
  /** Owning panel side (set by Side). */
  side: Side | null = null;
  /** Group leaders whose members are listed. */
  expandedGroups = $state.raw<Set<string>>(new Set());
  /** Result of a query-language filter (`type:image width>2000`). */
  filterQuery = $state.raw<{ text: string; keys: Set<string> | null; error: string | null } | null>(null);
  private filterToken = 0;

  private history: Location[] = [];
  private histPos = -1;
  private navToken = 0;
  private anchor: string | null = null;
  private baseMarks: Set<string> | null = null;

  private grouped = $derived.by(() => this.computeGrouped());
  visible = $derived.by(() => {
    const list = this.grouped.list;
    if (settings.showUp && this.listing?.parent && !this.filter) return [UP_ENTRY, ...list];
    return list;
  });
  index = $derived.by(() => {
    const m = new Map<string, number>();
    const v = this.visible;
    for (let i = 0; i < v.length; i++) m.set(v[i].key, i);
    return m;
  });
  cursor = $derived(this.cursorKey !== null ? (this.index.get(this.cursorKey) ?? 0) : 0);
  current = $derived<Entry | undefined>(this.visible[this.cursor]);

  constructor(init?: Partial<TabSnapshot>) {
    this.view = init?.view ?? settings.defaultView;
    this.sortKey = init?.sortKey ?? settings.defaultSort;
    this.sortDesc = init?.sortDesc ?? false;
    this.locked = init?.locked ?? false;
    if (init?.location) this.location = init.location;
  }

  snapshot(): TabSnapshot {
    let loc: Location = this.location;
    for (let i = this.history.length - 1; loc.kind === 'search' && i >= 0; i--) loc = this.history[i];
    if (loc.kind === 'search' || loc.kind === 'ftp') loc = { kind: 'drives' };
    return { location: loc, view: this.view, sortKey: this.sortKey, sortDesc: this.sortDesc, locked: this.locked };
  }

  /** Folder key for per-folder column choices. */
  get columnsKey(): string | null {
    const l = this.location;
    return l.kind === 'local' ? l.path : l.kind === 'search' || l.kind === 'query' ? 'search' : null;
  }

  /** Metadata column ids shown in the details view. */
  columns = $derived.by(() => {
    if (!settings.metaColumns) return [];
    const key = this.columnsKey;
    if (key === null) return [];
    const chosen = settings.folderColumns[key];
    return (chosen ?? autoColumns(this.all)).filter((id) => columnById.has(id));
  });

  /** Local path of an entry when it has one (local folders and search results). */
  pathOf(e: Entry): string | null {
    if (e.path) return e.path;
    return this.location.kind === 'local' && !(e.flags & F.UP) ? joinLocal(this.location.path, e.name) : null;
  }

  /** Metadata of an entry (loads it in the background when missing). */
  meta(e: Entry, load = true) {
    // Reading an online-only cloud file would download it.
    if (e.isDir || e.flags & F.CLOUD) return null;
    const p = this.pathOf(e);
    if (!p) return null;
    return (load ? metaFor : metaPeek)(p, e.size, e.mtime) ?? null;
  }

  /** Applies the filter text: masks locally, query language via the backend. */
  async updateFilter() {
    const text = this.filter.trim();
    const token = ++this.filterToken;
    if (!isQueryText(text)) {
      this.filterQuery = null;
      return;
    }
    const items = this.all
      .map((e) => [this.pathOf(e), e] as const)
      .filter((x): x is readonly [string, Entry] => x[0] !== null);
    try {
      const res = await api.queryFilter(
        text,
        items.map(([p, e]) => [p, e.isDir, e.size, e.mtime]),
      );
      if (token !== this.filterToken) return;
      this.filterQuery = { text, keys: new Set(items.filter((_, i) => res[i]).map(([, e]) => e.key)), error: null };
    } catch (err) {
      if (token !== this.filterToken) return;
      this.filterQuery = { text, keys: null, error: (err as AppError).message ?? String(err) };
    }
  }

  /** Whether related files are grouped in this tab. */
  get grouping(): boolean {
    const k = this.location.kind;
    return settings.groupFiles && (k === 'local' || k === 'ftp' || k === 'archive');
  }

  /** All files of a group led by `key` (undefined if not a group). */
  groupMembers(key: string): Entry[] | undefined {
    return this.grouped.members.get(key);
  }

  toggleGroup(key: string, open?: boolean) {
    const s = new Set(this.expandedGroups);
    if (open ?? !s.has(key)) s.add(key);
    else s.delete(key);
    this.expandedGroups = s;
  }

  /** Collapsed groups stand for all their files in operations. */
  private expand(e: Entry): Entry[] {
    const ms = this.grouped.members.get(e.key);
    return ms && !this.expandedGroups.has(e.key) ? ms : [e];
  }

  private computeGrouped(): Grouped {
    const list = this.computeSorted();
    if (!this.grouping) return { list, members: new Map() };
    return groupEntries(list, this.expandedGroups);
  }

  private computeSorted(): Entry[] {
    let list = this.all;
    if (!settings.showHidden) list = list.filter((e) => (e.flags & F.HIDDEN) === 0);
    if (this.filter) {
      const q = this.filterQuery;
      if (q && q.text === this.filter.trim()) {
        if (q.keys) list = list.filter((e) => q.keys!.has(e.key));
      } else if (!isQueryText(this.filter.trim())) {
        const m = maskMatcher(this.filter);
        list = list.filter((e) => m(e.name));
      }
    }
    const key = this.sortKey;
    const col = key.startsWith('m:') ? columnById.get(key.slice(2)) : undefined;
    const metaValue = col
      ? (e: Entry) => {
          const m = this.meta(e);
          return m ? col.value(m) : undefined;
        }
      : undefined;
    return this.location.kind === 'drives' ? list : sortEntries(list, key, this.sortDesc, localeTag(), this.dirSizes, metaValue);
  }

  get title(): string {
    const crumbs = this.listing?.crumbs;
    if (this.location.kind === 'search' || this.location.kind === 'query') return this.location.label;
    if (crumbs && crumbs.length > 0) return crumbs[crumbs.length - 1].label;
    // Not loaded yet (lazily restored tabs): derive a name from the location.
    const loc = this.location;
    if (loc.kind === 'local' || loc.kind === 'ftp') return baseName(loc.path) || loc.path;
    if (loc.kind === 'archive') return baseName(loc.inner) || baseName(loc.archive);
    return '';
  }

  get canGoBack() {
    return this.histPos > 0;
  }
  get canGoForward() {
    return this.histPos < this.history.length - 1;
  }

  private apply(listing: Listing) {
    this.listing = listing;
    this.location = listing.location;
    this.all = listing.entries.map(toEntry);
    this.marked = new Set();
    this.filter = '';
    this.filterOpen = false;
    this.dirSizes = new Map();
    this.diff = new Set();
    this.error = null;
    this.anchor = null;
  }

  private pushHistory(loc: Location) {
    this.history = this.history.slice(0, this.histPos + 1);
    if (!sameLoc(this.history[this.history.length - 1], loc)) this.history.push(loc);
    if (this.history.length > 200) this.history.shift();
    this.histPos = this.history.length - 1;
  }

  /** Loads `loc`. Keeps the old listing on failure (and reports the error). */
  async navigate(loc: Location, opts: { focus?: string | null; push?: boolean } = {}): Promise<boolean> {
    if (this.locked && this.side && this.listing && !sameLoc(loc, this.location)) {
      // Like Total Commander: a locked tab stays put, the new folder opens in a new tab.
      const tab = new Tab({ location: loc, view: this.view, sortKey: this.sortKey, sortDesc: this.sortDesc });
      this.side.addTab(tab);
      return tab.navigate(loc, { focus: opts.focus });
    }
    const push = opts.push ?? true;
    const token = ++this.navToken;
    if (loc.kind === 'search') {
      const res = searchResults.get(loc.id);
      if (!res) return false;
      this.showSearch(loc.id, res.label, null, push);
      return true;
    }
    if (loc.kind === 'query') {
      // Smart folder: re-run the saved index query.
      this.loading = true;
      try {
        const r = await api.indexSearch(loc.query, 10_000);
        if (token !== this.navToken) return false;
        this.showSearch(`q:${loc.query}`, loc.label, r.hits.map((h) => ({ ...h, matches: [], matchCount: 0 })), push, [], loc);
        return true;
      } catch (e) {
        if (token !== this.navToken) return false;
        if (!this.listing) this.error = e as AppError;
        else showError(e);
        return false;
      } finally {
        if (token === this.navToken) this.loading = false;
      }
    }
    this.loading = true;
    try {
      const listing = await api.listDir(loc as BackendLocation);
      if (token !== this.navToken) return false;
      const prev = this.location;
      this.apply(listing);
      let focus = opts.focus ?? null;
      if (!focus && sameLoc(parentLoc(prev), listing.location)) focus = lastName(prev);
      const hasFocus = focus !== null && this.all.some((e) => e.key === focus);
      this.cursorKey = hasFocus ? focus : (this.visible[0]?.key ?? null);
      this.revealTick++;
      if (push) this.pushHistory(listing.location);
      void this.refreshInfo();
      return true;
    } catch (e) {
      if (token !== this.navToken) return false;
      // Protected archive: ask for the password (or offer to mount an image) and retry.
      if (loc.kind === 'archive') {
        this.loading = false;
        const { recoverArchive } = await import('../archives');
        if (await recoverArchive(loc.archive, e)) return this.navigate(loc, opts);
        if ((e as AppError).code === 'PasswordRequired' || (e as AppError).code === 'Unsupported') return false;
      }
      if (!this.listing) this.error = e as AppError;
      else showError(e);
      return false;
    } finally {
      if (token === this.navToken) this.loading = false;
    }
  }

  /** Re-reads the current folder keeping cursor, marks and calculated sizes. */
  async reload(): Promise<void> {
    if (this.location.kind === 'search') return;
    if (this.location.kind === 'query') {
      const cur = this.cursorKey;
      await this.navigate(this.location, { push: false });
      if (cur) this.setCursorKey(cur);
      return;
    }
    const token = ++this.navToken;
    const oldCursor = this.cursor;
    try {
      const listing = await api.listDir(this.location as BackendLocation);
      if (token !== this.navToken) return;
      const keep = this.marked;
      const sizes = this.dirSizes;
      const cur = this.cursorKey;
      this.listing = listing;
      this.all = listing.entries.map(toEntry);
      const keys = new Set(this.all.map((e) => e.key));
      this.marked = new Set([...keep].filter((k) => keys.has(k)));
      this.dirSizes = new Map([...sizes].filter(([k]) => keys.has(k)));
      if (this.filterQuery) void this.updateFilter();
      if (cur === null || (!keys.has(cur) && cur !== UP_ENTRY.key)) {
        const v = this.visible;
        this.cursorKey = v[Math.min(oldCursor, v.length - 1)]?.key ?? null;
      }
      this.error = null;
      void this.refreshInfo();
    } catch (e) {
      if (token === this.navToken && !this.listing) this.error = e as AppError;
    }
  }

  private async refreshInfo() {
    const loc = this.location;
    if (loc.kind === 'local') {
      this.space = await api.space(loc.path).catch(() => null);
    } else {
      this.space = null;
    }
    if (loc.kind === 'drives') {
      this.volumes = await api.volumes().catch(() => []);
    }
  }

  showSearch(id: string, label: string, hits: SearchHit[] | null, push = true, roots: string[] = [], at?: Location) {
    if (this.locked && this.side && hits) {
      // A locked tab keeps its folder: results go to a new tab.
      const tab = new Tab({ view: this.view, sortKey: this.sortKey, sortDesc: this.sortDesc });
      this.side.addTab(tab);
      tab.showSearch(id, label, hits, push, roots, at);
      return;
    }
    // Folder column: path relative to the searched folder ("Photos/2024").
    const rel = (dir: string) => {
      for (const r of roots) {
        const root = r.replace(/[\\/]+$/, '');
        if (dir === root) return '.';
        if (dir.startsWith(root) && /[\\/]/.test(dir[root.length])) return dir.slice(root.length + 1);
      }
      return dir;
    };
    if (hits) {
      searchResults.set(id, {
        label,
        hits: hits.map((h) => ({
          name: h.name,
          flags: (h.isDir ? F.DIR : 0) | (h.cloud ? F.CLOUD : 0),
          size: h.size,
          mtime: h.mtime,
          attrs: 0,
          ext: h.isDir ? '' : extOf(h.name),
          isDir: h.isDir,
          key: h.path,
          path: h.path,
          note: rel(dirName(h.path)),
        })),
      });
      if (searchResults.size > 10) searchResults.delete(searchResults.keys().next().value!);
    }
    const res = searchResults.get(id);
    if (!res) return;
    const location: Location = at ?? { kind: 'search', id, label };
    this.navToken++;
    this.listing = {
      location,
      displayPath: label,
      parent: null,
      crumbs: [{ label, icon: 'folder', location }],
      entries: [],
    };
    this.location = location;
    this.all = res.hits;
    this.marked = new Set();
    this.filter = '';
    this.dirSizes = new Map();
    this.error = null;
    this.loading = false;
    this.space = null;
    this.cursorKey = this.visible[0]?.key ?? null;
    this.revealTick++;
    if (push) this.pushHistory(location);
  }

  async back() {
    if (!this.canGoBack) return;
    this.histPos--;
    if (!(await this.navigate(this.history[this.histPos], { push: false }))) this.histPos++;
  }

  async forward() {
    if (!this.canGoForward) return;
    this.histPos++;
    if (!(await this.navigate(this.history[this.histPos], { push: false }))) this.histPos--;
  }

  async up() {
    const p = this.listing?.parent;
    if (p) await this.navigate(p);
  }

  // ---------------------------------------------------------------- cursor

  setCursor(index: number, opts: { keepAnchor?: boolean } = {}) {
    const v = this.visible;
    if (v.length === 0) return;
    const i = Math.max(0, Math.min(v.length - 1, index));
    this.cursorKey = v[i].key;
    if (!opts.keepAnchor) this.anchor = null;
    this.revealTick++;
  }

  setCursorKey(key: string) {
    if (this.index.has(key)) {
      this.cursorKey = key;
      this.revealTick++;
    }
  }

  /** Shift+click / Shift+arrows: marks the range from the anchor. */
  selectTo(index: number) {
    const v = this.visible;
    if (v.length === 0) return;
    index = Math.max(0, Math.min(v.length - 1, index));
    if (this.anchor === null || !this.index.has(this.anchor)) {
      this.anchor = this.current?.key ?? v[0].key;
      this.baseMarks = new Set(this.marked);
    }
    const a = this.index.get(this.anchor) ?? 0;
    const [from, to] = a < index ? [a, index] : [index, a];
    const s = new Set(this.baseMarks ?? []);
    for (let i = from; i <= to; i++) if ((v[i].flags & F.UP) === 0) s.add(v[i].key);
    this.marked = s;
    this.cursorKey = v[index].key;
    this.revealTick++;
  }

  // ---------------------------------------------------------------- marks

  isMarked(key: string) {
    return this.marked.has(key);
  }

  toggleMark(key: string) {
    if (key === UP_ENTRY.key) return;
    const s = new Set(this.marked);
    if (s.has(key)) s.delete(key);
    else s.add(key);
    this.marked = s;
    this.anchor = null;
  }

  setMarks(keys: Iterable<string>, value: boolean) {
    const s = new Set(this.marked);
    for (const k of keys) {
      if (k === UP_ENTRY.key) continue;
      if (value) s.add(k);
      else s.delete(k);
    }
    this.marked = s;
  }

  markAll() {
    this.marked = new Set(this.visible.filter((e) => (e.flags & F.UP) === 0).map((e) => e.key));
  }

  unmarkAll() {
    this.marked = new Set();
    this.anchor = null;
  }

  invertMarks() {
    const s = new Set<string>();
    for (const e of this.visible) if ((e.flags & F.UP) === 0 && !this.marked.has(e.key)) s.add(e.key);
    this.marked = s;
  }

  markByMask(mask: string, value: boolean, includeDirs = false) {
    const m = maskMatcher(mask);
    this.setMarks(
      this.visible.filter((e) => (includeDirs || !e.isDir) && (e.flags & F.UP) === 0 && m(e.name)).map((e) => e.key),
      value,
    );
  }

  markSameExt() {
    const c = this.current;
    if (!c || c.isDir) return;
    const ext = c.ext.toLowerCase();
    this.setMarks(
      this.visible.filter((e) => !e.isDir && e.ext.toLowerCase() === ext).map((e) => e.key),
      true,
    );
  }

  /** Items an operation applies to: marked ones, otherwise the cursor item. */
  targets(): Entry[] {
    if (this.marked.size > 0) {
      const seen = new Set<string>();
      return this.all.filter((e) => this.marked.has(e.key)).flatMap((e) => this.expand(e)).filter((e) => !seen.has(e.key) && !!seen.add(e.key));
    }
    const c = this.current;
    return c && (c.flags & F.UP) === 0 ? this.expand(c) : [];
  }

  markedSummary(): { count: number; bytes: number } {
    let bytes = 0;
    let count = 0;
    for (const e of this.all) {
      if (!this.marked.has(e.key)) continue;
      count++;
      bytes += e.isDir ? (this.dirSizes.get(e.key) ?? 0) : this.expand(e).reduce((a, m) => a + m.size, 0);
    }
    return { count, bytes };
  }

  setDirSize(key: string, bytes: number) {
    const m = new Map(this.dirSizes);
    m.set(key, bytes);
    this.dirSizes = m;
  }

  setSort(key: SortKey) {
    if (this.sortKey === key) this.sortDesc = !this.sortDesc;
    else {
      this.sortKey = key;
      this.sortDesc = key === 'mtime' || key === 'size' || (key.startsWith('m:') && !!columnById.get(key.slice(2))?.right);
    }
    this.revealTick++;
  }
}

/** Filter text in the query language rather than a name mask ("width>2000", "type:video"). */
export function isQueryText(text: string): boolean {
  return /(^|[\s(|!-])[\p{L}_]+(:|>|<|=)/u.test(text);
}
