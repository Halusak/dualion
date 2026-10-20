<script lang="ts" module>
  import type { SearchHit, SearchParams } from '../../lib/types';

  // Survives closing the dialog, like Total Commander keeps the last search.
  const last = {
    params: null as SearchParams | null,
    hits: [] as SearchHit[],
    summary: '',
  };
</script>

<script lang="ts">
  import ChevronDown from '@lucide/svelte/icons/chevron-down';
  import ChevronRight from '@lucide/svelte/icons/chevron-right';
  import FolderOpen from '@lucide/svelte/icons/folder-open';
  import { open as openDialogNative } from '@tauri-apps/plugin-dialog';
  import { onDestroy } from 'svelte';
  import { api } from '../../lib/api';
  import { kindOf } from '../../lib/columns';
  import { formatDate, formatDuration, formatSize } from '../../lib/format';
  import { metaFor } from '../../lib/state/meta.svelte';
  import { localeTag, t } from '../../lib/i18n/index.svelte';
  import { baseName, dirName, extOf } from '../../lib/paths';
  import { app } from '../../lib/state/app.svelte';
  import { settings } from '../../lib/state/settings.svelte';
  import { showError } from '../../lib/state/ui.svelte';
  import { F, type Entry } from '../../lib/types';
  import FileIcon from '../FileIcon.svelte';
  import Modal from './Modal.svelte';

  let { root, close }: { root: string; close: (r: null) => void } = $props();

  const defaults: SearchParams = {
    // svelte-ignore state_referenced_locally
    roots: [root],
    name: '',
    nameRegex: false,
    nameCase: false,
    text: '',
    textRegex: false,
    textCase: false,
    textWord: false,
    textNot: false,
    encoding: 'auto',
    binary: false,
    minSize: null,
    maxSize: null,
    newerThan: null,
    olderThan: null,
    maxDepth: null,
    hidden: settings.showHidden,
    followLinks: false,
    gitignore: false,
    excludeDirs: '',
    kind: 'all',
    maxResults: 50000,
    query: '',
  };
  // svelte-ignore state_referenced_locally
  const p = $state<SearchParams>({ ...(last.params ?? defaults), roots: [root] });
  // svelte-ignore state_referenced_locally
  let rootsText = $state(root);
  let minKb = $state(p.minSize ? String(p.minSize / 1024) : '');
  let maxKb = $state(p.maxSize ? String(p.maxSize / 1024) : '');
  let newer = $state(p.newerThan ? new Date(p.newerThan).toISOString().slice(0, 10) : '');
  let older = $state(p.olderThan ? new Date(p.olderThan).toISOString().slice(0, 10) : '');
  let depth = $state(p.maxDepth !== null ? String(p.maxDepth) : '');
  let advanced = $state(false);

  let hits = $state.raw<SearchHit[]>(last.hits);
  let running = $state(false);
  let opId: number | null = null;
  let scanned = $state(0);
  let current = $state('');
  let summary = $state(last.summary);
  let selected = $state(0);
  const locale = localeTag();

  let listEl = $state<HTMLDivElement>();
  let scrollTop = $state(0);
  let viewH = $state(300);
  // ---------------------------------------------------------------- result table
  const COLS = 'minmax(180px, 2fr) minmax(120px, 1.4fr) 64px 120px 84px 128px 56px';
  type Col = 'name' | 'folder' | 'kind' | 'size' | 'mtime' | 'matches';
  let sortCol = $state<Col | null>(null);
  let sortDesc = $state(false);
  let kindFilter = $state<string | null>(null);
  const KIND_LABEL: Record<string, string> = { folder: 'kind.folder', image: 'kind.image', raw: 'kind.image', video: 'kind.video', audio: 'kind.audio', doc: 'kind.doc', code: 'kind.code', other: 'kind.other' };
  const kindOfHit = (h: SearchHit) => (h.isDir ? 'folder' : kindOf(extOf(h.name)) === 'raw' ? 'image' : kindOf(extOf(h.name)));
  const roots = $derived(rootsText.split(';').map((r) => r.trim().replace(/[\\/]+$/, '')).filter(Boolean));
  function relFolder(path: string): string {
    const d = dirName(path);
    for (const r of roots) {
      if (d === r) return '.';
      if (d.startsWith(r) && /[\\/]/.test(d[r.length])) return d.slice(r.length + 1);
    }
    return d;
  }
  const kinds = $derived.by(() => {
    const m = new Map<string, { n: number; bytes: number }>();
    for (const h of hits) {
      const k = kindOfHit(h);
      const v = m.get(k) ?? { n: 0, bytes: 0 };
      v.n++;
      v.bytes += h.isDir ? 0 : h.size;
      m.set(k, v);
    }
    return [...m].sort((a, b) => b[1].n - a[1].n);
  });
  const totalBytes = $derived(hits.reduce((a, h) => a + (h.isDir ? 0 : h.size), 0));
  const view = $derived.by(() => {
    let list = kindFilter ? hits.filter((h) => kindOfHit(h) === kindFilter) : hits;
    if (sortCol) {
      const c = new Intl.Collator(locale, { numeric: true, sensitivity: 'base' });
      const key: Record<Col, (a: SearchHit, b: SearchHit) => number> = {
        name: (a, b) => c.compare(a.name, b.name),
        folder: (a, b) => c.compare(relFolder(a.path), relFolder(b.path)),
        kind: (a, b) => c.compare(extOf(a.name), extOf(b.name)),
        size: (a, b) => a.size - b.size,
        mtime: (a, b) => a.mtime - b.mtime,
        matches: (a, b) => a.matchCount - b.matchCount,
      };
      const f = key[sortCol];
      list = [...list].sort((a, b) => (sortDesc ? -f(a, b) : f(a, b)));
    }
    return list;
  });
  $effect(() => {
    void kindFilter;
    void sortCol;
    void sortDesc;
    selected = 0;
  });
  function sortBy(col: Col) {
    if (sortCol === col) sortDesc = !sortDesc;
    else {
      sortCol = col;
      sortDesc = col === 'size' || col === 'mtime' || col === 'matches';
    }
  }
  const textMode = $derived(hits.some((h) => h.matches.length > 0));
  const rowH = $derived(textMode ? 40 : 26);
  const start = $derived(Math.max(0, Math.floor(scrollTop / rowH) - 5));
  const end = $derived(Math.min(view.length, Math.ceil((scrollTop + viewH) / rowH) + 5));
  /** Short metadata summary: dimensions or duration. */
  function metaText(h: SearchHit): string {
    if (h.isDir || h.cloud) return '';
    const m = metaFor(h.path, h.size, h.mtime);
    if (!m) return '';
    if (m.duration) return formatDuration(m.duration, locale) + (m.width && m.height ? ` · ${m.width}×${m.height}` : '');
    if (m.width && m.height) return `${m.width} × ${m.height}`;
    if (m.pages) return t('search.pages', { count: m.pages });
    return '';
  }

  async function start_() {
    if (running) return stop();
    const params: SearchParams = {
      ...p,
      roots: rootsText.split(';').map((s) => s.trim()).filter(Boolean),
      minSize: minKb ? Math.round(parseFloat(minKb.replace(',', '.')) * 1024) : null,
      maxSize: maxKb ? Math.round(parseFloat(maxKb.replace(',', '.')) * 1024) : null,
      newerThan: newer ? new Date(newer).getTime() : null,
      olderThan: older ? new Date(older).getTime() + 86_400_000 : null,
      maxDepth: depth ? parseInt(depth, 10) : null,
      encoding: p.encoding === 'auto' ? '' : p.encoding,
    };
    last.params = { ...params, encoding: p.encoding };
    hits = [];
    scanned = 0;
    summary = '';
    selected = 0;
    running = true;
    let buffer: SearchHit[] = [];
    let flush: ReturnType<typeof setTimeout> | null = null;
    try {
      opId = await api.searchStart(params, (ev) => {
        if (ev.type === 'hits') {
          buffer.push(...ev.items);
          flush ??= setTimeout(() => {
            hits = hits.concat(buffer);
            buffer = [];
            flush = null;
          }, 100);
        } else if (ev.type === 'progress') {
          scanned = ev.scanned;
          current = ev.current;
        } else if (ev.type === 'done') {
          if (flush) clearTimeout(flush);
          hits = hits.concat(buffer);
          buffer = [];
          running = false;
          opId = null;
          scanned = ev.scanned;
          current = '';
          summary = [
            t('search.found', { count: ev.found }),
            t('search.scanned', { count: ev.scanned }),
            ev.cancelled ? t('search.cancelled') : t('search.done', { time: (ev.elapsedMs / 1000).toFixed(1) }),
            ev.truncated ? t('search.truncated') : '',
          ]
            .filter(Boolean)
            .join(' · ');
          last.hits = hits;
          last.summary = summary;
        }
      });
    } catch (e) {
      running = false;
      showError(e);
    }
  }

  function stop() {
    if (opId !== null) void api.cancelOp(opId);
  }

  onDestroy(stop);

  async function browse() {
    const dir = await openDialogNative({ directory: true, defaultPath: rootsText.split(';')[0] });
    if (typeof dir === 'string') rootsText = dir;
  }

  async function goToHit(h: SearchHit | undefined) {
    if (!h) return;
    stop();
    close(null);
    const tab = app.panel;
    if (h.isDir) await tab.navigate({ kind: 'local', path: h.path });
    else await tab.navigate({ kind: 'local', path: dirName(h.path) }, { focus: baseName(h.path) });
  }

  function feed() {
    stop();
    const id = `${Date.now()}`;
    const label = `${t('panel.searchResults')}: ${p.name || p.text || '*'}`;
    app.panel.showSearch(id, label, view, true, rootsText.split(';').map((r) => r.trim()).filter(Boolean));
    close(null);
  }

  function entryOf(h: SearchHit): Entry {
    return { name: h.name, flags: (h.isDir ? F.DIR : 0) | (h.cloud ? F.CLOUD : 0), size: h.size, mtime: h.mtime, attrs: 0, ext: h.isDir ? '' : extOf(h.name), isDir: h.isDir, key: h.path, path: h.path };
  }

  function listKey(e: KeyboardEvent) {
    if (e.key === 'ArrowDown' || e.key === 'ArrowUp') {
      e.preventDefault();
      selected = Math.max(0, Math.min(view.length - 1, selected + (e.key === 'ArrowDown' ? 1 : -1)));
      if (listEl) {
        const top = selected * rowH;
        if (top < listEl.scrollTop) listEl.scrollTop = top;
        else if (top + rowH > listEl.scrollTop + viewH) listEl.scrollTop = top + rowH - viewH;
      }
    } else if (e.key === 'Enter') {
      e.preventDefault();
      e.stopPropagation();
      void goToHit(view[selected]);
    }
  }
</script>

<Modal title={t('search.title')} fill onclose={() => close(null)} onsubmit={start_}>
  <div class="search">
    <div class="form">
      <label class="row">
        <span>{t('search.name')}</span>
        <input class="input" bind:value={p.name} placeholder={t('search.nameHint')} spellcheck="false" data-autofocus />
        <label class="check"><input type="checkbox" bind:checked={p.nameRegex} /> {t('search.regex')}</label>
      </label>
      <label class="row">
        <span>{t('search.text')}</span>
        <input class="input" bind:value={p.text} placeholder={t('search.textHint')} spellcheck="false" />
        <span class="inline">
          <label class="check"><input type="checkbox" bind:checked={p.textCase} /> {t('search.case')}</label>
          <label class="check"><input type="checkbox" bind:checked={p.textWord} /> {t('search.word')}</label>
          <label class="check"><input type="checkbox" bind:checked={p.textRegex} /> {t('search.regex')}</label>
          <label class="check"><input type="checkbox" bind:checked={p.textNot} /> {t('search.not')}</label>
        </span>
      </label>
      <label class="row">
        <span>{t('search.query')}</span>
        <input class="input" bind:value={p.query} placeholder="type:image width>3000 taken:2024 camera:sony" title={t('query.help')} spellcheck="false" />
      </label>
      <label class="row">
        <span>{t('search.in')}</span>
        <span class="with-btn">
          <input class="input" bind:value={rootsText} spellcheck="false" />
          <button class="icon-btn" title={t('browse')} onclick={browse}><FolderOpen size={16} /></button>
        </span>
        <span class="inline">
          <label class="check"><input type="checkbox" bind:checked={p.hidden} /> {t('search.hidden')}</label>
        </span>
      </label>

      <button class="disclosure" onclick={() => (advanced = !advanced)}>
        {#if advanced}<ChevronDown size={14} />{:else}<ChevronRight size={14} />{/if}
        {t('search.advanced')}
      </button>
      {#if advanced}
        <div class="adv">
          <label>{t('search.kind')}
            <select class="input" bind:value={p.kind}>
              <option value="all">{t('search.kindAll')}</option>
              <option value="files">{t('search.kindFiles')}</option>
              <option value="dirs">{t('search.kindDirs')}</option>
            </select>
          </label>
          <label>{t('search.sizeMin')}<input class="input" bind:value={minKb} inputmode="decimal" /></label>
          <label>{t('search.sizeMax')}<input class="input" bind:value={maxKb} inputmode="decimal" /></label>
          <label>{t('search.newer')}<input class="input" type="date" bind:value={newer} /></label>
          <label>{t('search.older')}<input class="input" type="date" bind:value={older} /></label>
          <label>{t('search.depth')}<input class="input" bind:value={depth} inputmode="numeric" placeholder={t('search.depthAny')} /></label>
          <label>{t('search.encoding')}
            <select class="input" bind:value={p.encoding}>
              <option value="auto">{t('auto')} (UTF-8 / BOM)</option>
              <option value="windows-1250">Windows-1250 (CZ)</option>
              <option value="iso-8859-2">ISO-8859-2</option>
              <option value="utf-16le">UTF-16 LE</option>
              <option value="windows-1252">Windows-1252</option>
              <option value="ibm852">CP852 (DOS)</option>
            </select>
          </label>
          <label>{t('search.exclude')}<input class="input" bind:value={p.excludeDirs} placeholder="node_modules;.git" spellcheck="false" /></label>
          <div class="checks">
            <label class="check"><input type="checkbox" bind:checked={p.nameCase} /> {t('search.case')} ({t('search.name')})</label>
            <label class="check"><input type="checkbox" bind:checked={p.followLinks} /> {t('search.links')}</label>
            <label class="check"><input type="checkbox" bind:checked={p.gitignore} /> {t('search.gitignore')}</label>
            <label class="check"><input type="checkbox" bind:checked={p.binary} /> {t('search.binary')}</label>
          </div>
        </div>
      {/if}
    </div>

    <!-- svelte-ignore a11y_no_noninteractive_tabindex -->
    <div
      class="results"
      bind:this={listEl}
      bind:clientHeight={viewH}
      tabindex="0"
      role="listbox"
      onscroll={() => (scrollTop = listEl!.scrollTop)}
      onkeydown={listKey}
    >
      {#if hits.length}
        <div class="thead" style="--cols: {COLS}">
          {#snippet th(col: Col, label: string, right = false)}
            <button class="th" class:right class:on={sortCol === col} onclick={() => sortBy(col)}>
              {label}{#if sortCol === col}<span class="arrow">{sortDesc ? '▾' : '▴'}</span>{/if}
            </button>
          {/snippet}
          {@render th('name', t('col.name'))}
          {@render th('folder', t('col.folder'))}
          {@render th('kind', t('col.ext'))}
          <span class="th">{t('search.colInfo')}</span>
          {@render th('size', t('col.size'), true)}
          {@render th('mtime', t('col.date'))}
          {@render th('matches', t('search.colMatches'), true)}
        </div>
      {:else if !running}
        <div class="placeholder muted">{summary || t('search.ready')}</div>
      {/if}
      <div style="height: {view.length * rowH}px; position: relative">
        {#each view.slice(start, end) as h, i (h.path)}
          {@const idx = start + i}
          <!-- svelte-ignore a11y_click_events_have_key_events -->
          <div
            class="hit"
            class:sel={idx === selected}
            class:alt={idx % 2 === 1}
            role="option"
            aria-selected={idx === selected}
            tabindex="-1"
            style="top: {idx * rowH}px; height: {rowH}px; --cols: {COLS}"
            onclick={() => (selected = idx)}
            ondblclick={() => goToHit(h)}
            title={h.path}
          >
            <div class="cell name">
              <FileIcon entry={entryOf(h)} location={{ kind: 'local', path: dirName(h.path) }} size={16} />
              <div class="nmwrap">
                <span class="nm ellipsis">{h.name}</span>
                {#if h.matches[0]}
                  <span class="l2 ellipsis"><span class="line">{h.matches[0].line}:</span> {h.matches[0].text.trim()}</span>
                {/if}
              </div>
            </div>
            <div class="cell muted ellipsis">{relFolder(h.path)}</div>
            <div class="cell muted ellipsis">{h.isDir ? t('panel.dir') : extOf(h.name)}</div>
            <div class="cell muted ellipsis num">{metaText(h)}</div>
            <div class="cell num right">{h.isDir ? '' : formatSize(h.size, locale, settings.sizeFormat)}</div>
            <div class="cell muted num">{formatDate(h.mtime, locale)}</div>
            <div class="cell num right">{#if h.matchCount}<span class="badge">{h.matchCount}</span>{/if}</div>
          </div>
        {/each}
      </div>
    </div>

    {#if hits.length}
      <div class="kinds">
        <button class="chip" class:on={!kindFilter} onclick={() => (kindFilter = null)}>{t('search.all')} <b>{hits.length.toLocaleString(locale)}</b> · {formatSize(totalBytes, locale, settings.sizeFormat)}</button>
        {#each kinds as [k, v] (k)}
          <button class="chip" class:on={kindFilter === k} onclick={() => (kindFilter = kindFilter === k ? null : k)}>
            {t(KIND_LABEL[k] ?? 'kind.other')} <b>{v.n.toLocaleString(locale)}</b>{v.bytes ? ` · ${formatSize(v.bytes, locale, settings.sizeFormat)}` : ''}
          </button>
        {/each}
      </div>
    {/if}

    <div class="statusbar">
      {#if running}
        <span class="spinner"></span>
        <span class="ellipsis">{t('search.found', { count: hits.length })} · {t('search.scanned', { count: scanned })} · {current}</span>
      {:else}
        <span class="ellipsis">{summary}</span>
      {/if}
      <span class="grow"></span>
      <button class="btn" disabled={!view.length} onclick={() => goToHit(view[selected])}>{t('search.goTo')}</button>
      <button class="btn" disabled={!hits.length} onclick={feed}>{t('search.feed')}</button>
      <button class="btn" onclick={() => close(null)}>{t('close')}</button>
      <button class="btn primary" onclick={start_}>{running ? t('stop') : t('search.start')}</button>
    </div>
  </div>
</Modal>

<style>
  .search {
    flex: 1;
    min-height: 0;
    display: flex;
    flex-direction: column;
  }
  .form {
    padding: 14px 18px 8px;
    display: grid;
    gap: 8px;
    border-bottom: 1px solid var(--border);
  }
  .row {
    display: grid;
    grid-template-columns: 130px minmax(200px, 1fr) auto;
    align-items: center;
    gap: 10px;
  }
  .inline {
    display: flex;
    flex-wrap: wrap;
    gap: 4px 12px;
  }
  .with-btn {
    display: flex;
    gap: 4px;
  }
  .with-btn input {
    flex: 1;
  }
  .disclosure {
    justify-self: start;
    display: inline-flex;
    align-items: center;
    gap: 4px;
    border: none;
    background: transparent;
    color: var(--text-2);
    padding: 2px 0;
  }
  .adv {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(200px, 1fr));
    gap: 8px 14px;
    padding: 10px;
    border-radius: var(--r-m);
    background: var(--bg-subtle);
  }
  .adv label {
    display: flex;
    flex-direction: column;
    gap: 4px;
    font-size: 12px;
    color: var(--text-2);
  }
  .adv .checks {
    grid-column: 1 / -1;
    display: flex;
    flex-wrap: wrap;
    gap: 6px 16px;
  }
  .adv .checks label {
    flex-direction: row;
    color: var(--text);
  }
  .results {
    flex: 1;
    min-height: 0;
    overflow: auto;
    position: relative;
    outline: none;
    background: var(--bg-panel);
  }
  .placeholder {
    position: absolute;
    inset: 0;
    display: flex;
    align-items: center;
    justify-content: center;
  }
  .thead {
    position: sticky;
    top: 0;
    z-index: 2;
    display: grid;
    grid-template-columns: var(--cols);
    padding: 0 16px;
    height: 26px;
    background: var(--bg-header);
    border-bottom: 1px solid var(--border);
  }
  .th {
    border: none;
    background: transparent;
    text-align: left;
    font-size: 11px;
    color: var(--text-2);
    padding: 0 8px;
    display: flex;
    align-items: center;
    gap: 3px;
    white-space: nowrap;
  }
  .th.right {
    justify-content: flex-end;
  }
  .th.on {
    color: var(--text);
    font-weight: 600;
  }
  .arrow {
    font-size: 9px;
  }
  .hit {
    position: absolute;
    left: 6px;
    right: 6px;
    display: grid;
    grid-template-columns: var(--cols);
    align-items: center;
    padding: 0 10px;
    border-radius: var(--r-s);
    font-size: 12px;
  }
  .hit.alt {
    background: var(--bg-row-alt);
  }
  .hit:hover {
    background: var(--bg-hover);
  }
  .hit.sel {
    background: var(--cursor-bg);
    color: var(--cursor-fg);
  }
  .hit.sel .muted {
    color: inherit;
    opacity: 0.85;
  }
  .cell {
    min-width: 0;
    padding: 0 8px;
  }
  .cell.name {
    display: flex;
    align-items: center;
    gap: 7px;
  }
  .nmwrap {
    min-width: 0;
    display: flex;
    flex-direction: column;
  }
  .right {
    text-align: right;
  }
  .num {
    font-variant-numeric: tabular-nums;
    white-space: nowrap;
  }
  .badge {
    font-size: 10px;
    padding: 0 6px;
    border-radius: 8px;
    background: var(--accent-20);
    color: var(--accent-text);
  }
  .l2 {
    font-size: 11px;
    opacity: 0.72;
    font-family: var(--font-mono);
  }
  .line {
    font-family: var(--font-ui);
  }
  .kinds {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
    padding: 8px 16px 0;
  }
  .chip {
    border: 1px solid var(--border);
    background: var(--bg-subtle);
    color: var(--text-2);
    border-radius: 12px;
    padding: 2px 10px;
    font-size: 11px;
  }
  .chip b {
    color: var(--text);
  }
  .chip.on {
    background: var(--accent-20);
    border-color: transparent;
    color: var(--accent-text);
  }
  .statusbar {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 10px 16px;
    border-top: 1px solid var(--border);
    font-size: 12px;
    color: var(--text-2);
  }
  .grow {
    flex: 1;
  }
  .spinner {
    width: 12px;
    height: 12px;
    border-radius: 50%;
    border: 2px solid var(--border-strong);
    border-top-color: var(--accent);
    animation: spin 0.8s linear infinite;
    flex: none;
  }
  @keyframes spin {
    to {
      transform: rotate(360deg);
    }
  }
</style>
