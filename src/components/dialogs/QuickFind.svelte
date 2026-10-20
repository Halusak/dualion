<script lang="ts">
  // Instant search over the file index + command palette (">" prefix).
  import CornerDownLeft from '@lucide/svelte/icons/corner-down-left';
  import FolderSearch from '@lucide/svelte/icons/folder-search';
  import Search from '@lucide/svelte/icons/search';
  import SquareTerminal from '@lucide/svelte/icons/square-terminal';
  import Star from '@lucide/svelte/icons/star';
  import { untrack } from 'svelte';
  import { api } from '../../lib/api';
  import { commands, run, shortcutOf, type Command } from '../../lib/commands';
  import { fold, fuzzyScore, highlight } from '../../lib/fold';
  import { formatDate, formatSize } from '../../lib/format';
  import { localeTag, t } from '../../lib/i18n/index.svelte';
  import { dirName, extOf } from '../../lib/paths';
  import { shortcutLabel } from '../../lib/platform';
  import { app } from '../../lib/state/app.svelte';
  import { index } from '../../lib/state/index.svelte';
  import { settings } from '../../lib/state/settings.svelte';
  import { errorText, toast } from '../../lib/state/ui.svelte';
  import { F, type Entry, type IndexHit, type Location } from '../../lib/types';
  import FileIcon from '../FileIcon.svelte';
  import Modal from './Modal.svelte';

  let { initial = '', close }: { initial?: string; close: (r: null) => void } = $props();

  type Item =
    | { kind: 'file'; hit: IndexHit; entry: Entry }
    | { kind: 'cmd'; cmd: Command }
    | { kind: 'place'; label: string; location: Location; smart?: boolean };

  let text = $state(untrack(() => initial));
  let scopeHere = $state(false);
  let hits = $state.raw<IndexHit[]>([]);
  let total = $state(0);
  let elapsed = $state(0);
  let error = $state('');
  let sel = $state(0);
  let input = $state<HTMLInputElement>();
  let list = $state<HTMLDivElement>();

  const tab = app.panel;
  const here = tab.location.kind === 'local' ? tab.location.path : null;
  const isCmd = $derived(text.startsWith('>'));
  const query = $derived(text.trim());
  const words = $derived(
    query
      .split(/\s+/)
      .filter((w) => w && !/[:<>=]/.test(w) && !w.startsWith('-'))
      .map((w) => fold(w.replace(/^"|"$/g, ''))),
  );

  function toEntry(h: IndexHit): Entry {
    return { name: h.name, flags: h.isDir ? F.DIR : 0, size: h.size, mtime: h.mtime, attrs: 0, ext: h.isDir ? '' : extOf(h.name), isDir: h.isDir, key: h.path, path: h.path };
  }

  // Index search as you type.
  let seq = 0;
  $effect(() => {
    const q = query;
    const under = scopeHere ? here : null;
    if (isCmd || !q) {
      hits = [];
      total = 0;
      error = '';
      return;
    }
    const my = ++seq;
    const timer = setTimeout(async () => {
      try {
        const r = await api.indexSearch(q, 200, under);
        if (my !== seq) return;
        hits = r.hits;
        total = r.total;
        elapsed = r.elapsedMs;
        error = '';
        sel = 0;
      } catch (e) {
        if (my !== seq) return;
        const msg = errorText(e);
        if (!/cancel/i.test(msg)) error = msg;
        hits = [];
        total = 0;
      }
    }, 40);
    return () => clearTimeout(timer);
  });

  const cmdItems = $derived.by(() => {
    const q = isCmd ? text.slice(1).trim() : query;
    if (!isCmd && q.length < 2) return [];
    const scored = commands
      .map((c) => ({ c, s: fuzzyScore(q, `${t(c.label)} ${c.id}`) }))
      .filter((x) => x.s >= 0 && (isCmd || x.s > 6))
      .sort((a, b) => b.s - a.s);
    return scored.slice(0, isCmd ? 60 : 3).map((x) => ({ kind: 'cmd', cmd: x.c }) as Item);
  });

  const placeItems = $derived.by(() => {
    if (isCmd || query) return [];
    const out: Item[] = settings.hotlist.map((h) => ({ kind: 'place', label: h.name, location: h.location, smart: h.location.kind === 'query' }));
    for (const p of app.info.places) out.push({ kind: 'place', label: t(`place.${p.key}`), location: { kind: 'local', path: p.path } });
    return out;
  });

  const items = $derived<Item[]>(
    isCmd ? cmdItems : [...placeItems, ...hits.map((h) => ({ kind: 'file', hit: h, entry: toEntry(h) }) as Item), ...cmdItems],
  );

  $effect(() => {
    void items.length;
    if (sel >= items.length) sel = Math.max(0, items.length - 1);
  });

  $effect(() => {
    const el = list?.querySelector<HTMLElement>(`[data-i="${sel}"]`);
    el?.scrollIntoView({ block: 'nearest' });
  });

  function finish(fn: () => unknown) {
    close(null);
    // Let the dialog unmount first (commands refuse to run under a dialog).
    setTimeout(() => void fn(), 0);
  }

  function activate(item: Item | undefined, mod: { open?: boolean } = {}) {
    if (!item) return;
    if (item.kind === 'cmd') return finish(() => run(item.cmd.id));
    if (item.kind === 'place') return finish(() => tab.navigate(item.location));
    const h = item.hit;
    if (mod.open) return finish(() => api.openPath(h.path));
    if (h.isDir) return finish(() => tab.navigate({ kind: 'local', path: h.path }));
    finish(() => tab.navigate({ kind: 'local', path: dirName(h.path) }, { focus: h.name }));
  }

  function showAll() {
    if (!query || isCmd) return;
    finish(() => tab.navigate({ kind: 'query', query: scopeHere && here ? `${query} in:"${here}"` : query, label: query }));
  }

  function saveSmart() {
    if (!query || isCmd) return;
    const location: Location = { kind: 'query', query: scopeHere && here ? `${query} in:"${here}"` : query, label: query };
    settings.hotlist = [...settings.hotlist, { name: query, location }];
    toast(t('find.saved', { name: query }));
  }

  function onKey(e: KeyboardEvent) {
    const mod = e.metaKey || e.ctrlKey;
    if (e.key === 'ArrowDown') {
      e.preventDefault();
      sel = Math.min(items.length - 1, sel + 1);
    } else if (e.key === 'ArrowUp') {
      e.preventDefault();
      sel = Math.max(0, sel - 1);
    } else if (e.key === 'PageDown') {
      e.preventDefault();
      sel = Math.min(items.length - 1, sel + 10);
    } else if (e.key === 'PageUp') {
      e.preventDefault();
      sel = Math.max(0, sel - 10);
    } else if (e.key === 'Enter') {
      e.preventDefault();
      e.stopPropagation();
      if (e.shiftKey) showAll();
      else activate(items[sel], { open: mod });
    } else if (mod && e.key.toLowerCase() === 's') {
      e.preventDefault();
      saveSmart();
    } else if (e.key === 'Tab' && !isCmd && here) {
      e.preventDefault();
      e.stopPropagation();
      scopeHere = !scopeHere;
    }
  }

  const locale = localeTag();
  const home = app.info.home;
  const short = (p: string) => (home && p.startsWith(home) ? `~${p.slice(home.length)}` : p);
  const st = $derived(index.status);
</script>

<Modal width={720} onclose={() => close(null)}>
  <div class="qf">
    <div class="bar">
      {#if isCmd}<SquareTerminal size={18} />{:else}<Search size={18} />{/if}
      <input
        bind:this={input}
        bind:value={text}
        data-autofocus
        data-no-submit
        spellcheck="false"
        autocomplete="off"
        placeholder={isCmd ? t('find.cmdHint') : t('find.hint')}
        onkeydown={onKey}
        title={t('query.help')}
      />
      {#if !isCmd && here}
        <div class="seg" title="Tab">
          <button class:on={!scopeHere} tabindex="-1" onclick={() => (scopeHere = false)}>{t('find.everywhere')}</button>
          <button class:on={scopeHere} tabindex="-1" onclick={() => (scopeHere = true)}>{t('find.here')}</button>
        </div>
      {/if}
    </div>

    {#if !isCmd && !st.ready}
      <div class="note">
        {#if st.state === 'off'}
          {t('find.off')}
          <button class="btn" onclick={() => (settings.index.enabled = true)}>{t('find.enable')}</button>
        {:else}
          {t('find.building', { dirs: st.scannedDirs.toLocaleString(locale) })}
        {/if}
      </div>
    {:else if error}
      <div class="note err">{error}</div>
    {/if}

    <div class="list" bind:this={list}>
      {#each items as item, i (item.kind === 'file' ? item.hit.path : item.kind === 'cmd' ? `c:${item.cmd.id}` : `p:${item.label}:${i}`)}
        <!-- svelte-ignore a11y_click_events_have_key_events -->
        <!-- svelte-ignore a11y_no_static_element_interactions -->
        <div class="row" class:sel={i === sel} data-i={i} onpointermove={() => (sel = i)} onclick={(e) => activate(item, { open: e.metaKey || e.ctrlKey })}>
          {#if item.kind === 'file'}
            <FileIcon entry={item.entry} location={{ kind: 'local', path: dirName(item.hit.path) }} size={20} />
            <div class="main">
              <div class="name ellipsis">
                {#each highlight(item.hit.name, words) as part, j (j)}{#if part.hit}<mark>{part.text}</mark>{:else}{part.text}{/if}{/each}
              </div>
              <div class="sub ellipsis">{short(dirName(item.hit.path))}</div>
            </div>
            <div class="meta">
              {#if !item.hit.isDir}<span>{formatSize(item.hit.size, locale, settings.sizeFormat)}</span>{/if}
              <span>{formatDate(item.hit.mtime, locale)}</span>
            </div>
          {:else if item.kind === 'cmd'}
            <span class="ico"><SquareTerminal size={16} /></span>
            <div class="main"><div class="name ellipsis">{t(item.cmd.label)}</div></div>
            {#if shortcutOf(item.cmd.id)}<kbd>{shortcutLabel(shortcutOf(item.cmd.id)!)}</kbd>{/if}
          {:else}
            <span class="ico">{#if item.smart}<FolderSearch size={16} />{:else}<Star size={16} />{/if}</span>
            <div class="main"><div class="name ellipsis">{item.label}</div></div>
          {/if}
          {#if i === sel}<span class="enter"><CornerDownLeft size={13} /></span>{/if}
        </div>
      {/each}
      {#if query && !isCmd && st.ready && !error && hits.length === 0 && cmdItems.length === 0}
        <div class="empty muted">{t('find.none')}</div>
      {/if}
    </div>

    <footer class="muted">
      {#if isCmd}
        <span>↵ {t('find.run')}</span>
      {:else}
        <span>↵ {t('find.go')}</span>
        <span>{shortcutLabel('Mod+Enter')} {t('find.open')}</span>
        <span>⇧↵ {t('find.all')}</span>
        <span>{shortcutLabel('Mod+S')} {t('find.save')}</span>
        <span class="grow"></span>
        {#if query && st.ready}<span>{t('find.count', { n: total.toLocaleString(locale), ms: elapsed })}</span>{/if}
        {#if st.ready && !query}<span>{t('find.indexed', { n: st.files.toLocaleString(locale) })}{st.state === 'scanning' ? ' · ' + t('find.updating') : ''}</span>{/if}
      {/if}
    </footer>
  </div>
</Modal>

<style>
  .qf {
    display: flex;
    flex-direction: column;
    min-height: 0;
    max-height: min(620px, calc(100vh - 100px));
  }
  .bar {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 12px 14px;
    border-bottom: 1px solid var(--border);
    color: var(--text-2);
  }
  .bar input {
    flex: 1;
    min-width: 0;
    border: none;
    outline: none;
    background: transparent;
    color: var(--text);
    font-size: 17px;
  }
  .seg {
    display: flex;
    gap: 2px;
    padding: 2px;
    border-radius: calc(var(--r-s) + 2px);
    background: var(--bg-subtle);
    flex: none;
  }
  .seg button {
    border: none;
    background: transparent;
    padding: 2px 9px;
    border-radius: var(--r-s);
    color: var(--text-2);
    font-size: 12px;
  }
  .seg button.on {
    background: var(--bg-panel);
    color: var(--text);
    box-shadow: 0 0.5px 2px rgba(0, 0, 0, 0.15);
  }
  .note {
    padding: 8px 14px;
    font-size: 12px;
    color: var(--text-2);
    display: flex;
    align-items: center;
    gap: 10px;
    border-bottom: 1px solid var(--border);
  }
  .note.err {
    color: var(--danger);
  }
  .list {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    padding: 6px;
  }
  .row {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 5px 8px;
    border-radius: var(--r-m);
    cursor: default;
    min-height: 40px;
  }
  .row.sel {
    background: var(--accent);
    color: white;
  }
  .row.sel .sub,
  .row.sel .meta,
  .row.sel kbd {
    color: rgba(255, 255, 255, 0.82);
  }
  .main {
    flex: 1;
    min-width: 0;
  }
  .name {
    font-size: 13px;
  }
  .sub {
    font-size: 11px;
    color: var(--text-2);
  }
  mark {
    background: transparent;
    color: inherit;
    font-weight: 700;
    text-decoration: underline;
    text-decoration-thickness: 1px;
    text-underline-offset: 2px;
  }
  .meta {
    display: flex;
    flex-direction: column;
    align-items: flex-end;
    font-size: 11px;
    color: var(--text-2);
    font-variant-numeric: tabular-nums;
    flex: none;
  }
  .ico {
    width: 20px;
    display: inline-flex;
    justify-content: center;
    color: var(--text-2);
  }
  .row.sel .ico {
    color: white;
  }
  kbd {
    font-family: inherit;
    font-size: 11px;
    color: var(--text-2);
  }
  .enter {
    display: inline-flex;
    opacity: 0.8;
  }
  .empty {
    padding: 24px;
    text-align: center;
  }
  footer {
    display: flex;
    gap: 14px;
    padding: 7px 14px;
    border-top: 1px solid var(--border);
    font-size: 11px;
    flex-wrap: wrap;
  }
  .grow {
    flex: 1;
  }
</style>
