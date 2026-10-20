<script lang="ts">
  // Disk usage map: squarified treemap of a folder with drill-down.
  import ArrowUp from '@lucide/svelte/icons/arrow-up';
  import FolderOpen from '@lucide/svelte/icons/folder-open';
  import RefreshCw from '@lucide/svelte/icons/refresh-cw';
  import Trash2 from '@lucide/svelte/icons/trash-2';
  import X from '@lucide/svelte/icons/x';
  import { onMount } from 'svelte';
  import { api } from '../../lib/api';
  import { kindOf } from '../../lib/columns';
  import { formatSize } from '../../lib/format';
  import { localeTag, t } from '../../lib/i18n/index.svelte';
  import { baseName, dirName, extOf } from '../../lib/paths';
  import { app } from '../../lib/state/app.svelte';
  import { pushHistory } from '../../lib/state/history.svelte';
  import { settings } from '../../lib/state/settings.svelte';
  import { errorText, showError } from '../../lib/state/ui.svelte';
  import { squarify, type Placed } from '../../lib/treemap';
  import type { DuNode } from '../../lib/types';
  import Modal from './Modal.svelte';

  let { path: rootPath, close }: { path: string; close: (r: null) => void } = $props();

  let node = $state.raw<DuNode | null>(null);
  let scanning = $state(false);
  let dirs = $state(0);
  let error = $state('');
  let opId = 0;
  let selected = $state<DuNode | null>(null);
  let hover = $state<DuNode | null>(null);
  let box = $state({ w: 800, h: 500 });
  const locale = localeTag();
  const size = (n: number) => formatSize(n, locale, settings.sizeFormat);

  async function scan(path: string) {
    scanning = true;
    error = '';
    dirs = 0;
    let first = true;
    try {
      node = await api.duScan(path, (n) => {
        if (first) {
          opId = n;
          first = false;
        } else dirs = n;
      });
      selected = null;
    } catch (e) {
      error = errorText(e);
    } finally {
      scanning = false;
    }
  }

  async function open(path: string) {
    try {
      node = await api.duTree(path);
      selected = null;
    } catch {
      await scan(path);
    }
  }

  onMount(() => {
    void scan(rootPath);
    return () => {
      if (scanning && opId) void api.cancelOp(opId);
    };
  });

  const COLORS: Record<string, string> = {
    image: '#3fa66b',
    raw: '#2f8f5b',
    video: '#8a5cd6',
    audio: '#e08a2e',
    doc: '#3d7fd9',
    code: '#2aa7a7',
    other: '#8c8c93',
  };
  const DIR_HUES = [210, 28, 140, 280, 350, 190, 50, 100];

  function color(n: DuNode, i: number, depth: number): string {
    if (n.other) return 'var(--du-other)';
    if (n.isDir) return `hsl(${DIR_HUES[i % DIR_HUES.length]} 45% ${depth ? 52 : 46}%)`;
    const ext = extOf(n.name).toLowerCase();
    if (['zip', '7z', 'rar', 'tar', 'gz', 'dmg', 'iso'].includes(ext)) return '#a07850';
    return COLORS[kindOf(ext)] ?? COLORS.other;
  }

  interface Tile extends Placed<DuNode> {
    depth: number;
    fill: string;
    label: boolean;
  }

  const tiles = $derived.by(() => {
    if (!node?.children) return [];
    const out: Tile[] = [];
    const top = squarify(node.children, (c) => c.size, { x: 0, y: 0, w: box.w, h: box.h });
    top.forEach((p, i) => {
      const fill = color(p.item, i, 0);
      out.push({ ...p, depth: 0, fill, label: p.w > 50 && p.h > 22 });
      // Second level inside folders big enough to show it.
      if (p.item.isDir && p.item.children?.length && p.w > 60 && p.h > 46) {
        const inner = { x: p.x + 3, y: p.y + 20, w: p.w - 6, h: p.h - 23 };
        squarify(p.item.children, (c) => c.size, inner).forEach((q, j) => {
          out.push({ ...q, depth: 1, fill: q.item.isDir ? color(q.item, i, 1) : color(q.item, j, 1), label: q.w > 60 && q.h > 18 });
        });
      }
    });
    return out;
  });

  const crumbs = $derived.by(() => {
    if (!node) return [];
    const out: { label: string; path: string }[] = [];
    let p = node.path;
    const root = rootPath.replace(/[\\/]+$/, '');
    while (p && p.length >= root.length) {
      out.unshift({ label: baseName(p) || p, path: p });
      const d = dirName(p);
      if (d === p) break;
      p = d;
    }
    return out;
  });

  const list = $derived(node?.children?.slice(0, 40) ?? []);
  const canUp = $derived(!!node && node.path.replace(/[\\/]+$/, '') !== rootPath.replace(/[\\/]+$/, '') && node.path.length > rootPath.length);

  function click(n: DuNode) {
    if (n.other) return;
    if (n.isDir) void open(n.path);
    else selected = n;
  }

  function reveal(n: DuNode) {
    close(null);
    if (n.isDir) void app.panel.navigate({ kind: 'local', path: n.path });
    else void app.panel.navigate({ kind: 'local', path: dirName(n.path) }, { focus: n.name });
  }

  async function trash(n: DuNode) {
    try {
      const action = await api.trashItem(n.path);
      pushHistory({ label: `${t('history.trashed')}: ${n.name}`, actions: [action] });
      app.refreshLocation({ kind: 'local', path: dirName(n.path) });
      await scan(node!.path);
    } catch (e) {
      showError(e);
    }
  }

  const info = $derived(hover ?? selected);
</script>

<Modal fill onclose={() => close(null)}>
  <div class="du">
    <header>
      <button class="icon-btn" disabled={!canUp || scanning} title={t('cmd.parent')} onclick={() => node && open(dirName(node.path))}><ArrowUp size={16} /></button>
      <nav class="crumbs">
        {#each crumbs as c, i (c.path)}
          {#if i}<span class="sep">›</span>{/if}
          <button class:last={i === crumbs.length - 1} onclick={() => open(c.path)}>{c.label}</button>
        {/each}
      </nav>
      {#if node}<span class="muted">{size(node.size)} · {t('plan.files', { count: node.files })}</span>{/if}
      <span class="grow"></span>
      <button class="icon-btn" title={t('du.rescan')} disabled={scanning} onclick={() => node && scan(node.path)}><RefreshCw size={15} /></button>
      <button class="icon-btn" title={t('close')} onclick={() => close(null)}><X size={16} /></button>
    </header>
    <div class="body">
      <div class="map" bind:clientWidth={box.w} bind:clientHeight={box.h}>
        {#if scanning && !node}
          <div class="center muted">{t('du.scanning', { dirs: dirs.toLocaleString(locale) })}</div>
        {:else if error}
          <div class="center muted">{error}</div>
        {/if}
        {#each tiles as tl, i (i)}
          <!-- svelte-ignore a11y_click_events_have_key_events -->
          <!-- svelte-ignore a11y_no_static_element_interactions -->
          <div
            class="tile"
            class:dir={tl.item.isDir}
            class:inner={tl.depth > 0}
            class:sel={selected === tl.item}
            style="left:{tl.x}px;top:{tl.y}px;width:{tl.w}px;height:{tl.h}px;background:{tl.fill}"
            onpointerenter={() => (hover = tl.item)}
            onpointerleave={() => hover === tl.item && (hover = null)}
            onclick={(e) => {
              e.stopPropagation();
              click(tl.item);
            }}
            ondblclick={() => !tl.item.other && reveal(tl.item)}
          >
            {#if tl.label}
              <span class="tl">{tl.item.other ? t('du.other', { n: tl.item.name }) : tl.item.name}</span>
              {#if tl.h > 34}<span class="ts">{size(tl.item.size)}</span>{/if}
            {/if}
          </div>
        {/each}
        {#if scanning && node}<div class="busy">{t('du.scanning', { dirs: dirs.toLocaleString(locale) })}</div>{/if}
      </div>
      <aside>
        {#if info}
          <div class="info">
            <strong class="ellipsis" title={info.path}>{info.other ? t('du.other', { n: info.name }) : info.name}</strong>
            <div class="muted">{size(info.size)}{node?.size ? ` · ${((info.size / node.size) * 100).toFixed(1)} %` : ''}{info.isDir ? ` · ${t('plan.files', { count: info.files })}` : ''}</div>
            {#if !info.other}
              <div class="actions">
                <button class="btn" onclick={() => reveal(info!)}><FolderOpen size={14} /> {t('du.show')}</button>
                <button class="btn" onclick={() => trash(info!)}><Trash2 size={14} /> {t('du.trash')}</button>
              </div>
            {/if}
          </div>
        {/if}
        <div class="list">
          {#each list as c (c.path || c.name)}
            <!-- svelte-ignore a11y_click_events_have_key_events -->
            <!-- svelte-ignore a11y_no_static_element_interactions -->
            <div class="item" class:sel={selected === c} onclick={() => (c.isDir ? open(c.path) : (selected = c))} onpointerenter={() => (hover = c)} onpointerleave={() => hover === c && (hover = null)}>
              <span class="ellipsis">{c.other ? t('du.other', { n: c.name }) : c.name}{c.isDir ? '/' : ''}</span>
              <span class="bar"><span style="width:{node?.size ? (c.size / node.size) * 100 : 0}%"></span></span>
              <span class="sz">{size(c.size)}</span>
            </div>
          {/each}
        </div>
      </aside>
    </div>
  </div>
</Modal>

<style>
  .du {
    flex: 1;
    min-height: 0;
    display: flex;
    flex-direction: column;
    --du-other: repeating-linear-gradient(45deg, #777 0 4px, #6a6a6a 4px 8px);
  }
  header {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 8px 12px;
    border-bottom: 1px solid var(--border);
  }
  .crumbs {
    display: flex;
    align-items: center;
    gap: 2px;
    min-width: 0;
    overflow: hidden;
  }
  .crumbs button {
    border: none;
    background: transparent;
    color: var(--text-2);
    padding: 2px 4px;
    border-radius: var(--r-s);
    white-space: nowrap;
  }
  .crumbs button:hover {
    background: var(--bg-hover);
  }
  .crumbs button.last {
    color: var(--text);
    font-weight: 600;
  }
  .sep {
    color: var(--text-3);
  }
  .grow {
    flex: 1;
  }
  .body {
    flex: 1;
    min-height: 0;
    display: grid;
    grid-template-columns: 1fr 300px;
  }
  .map {
    position: relative;
    margin: 10px;
    overflow: hidden;
    border-radius: 6px;
    background: var(--bg-subtle);
  }
  .tile {
    position: absolute;
    box-sizing: border-box;
    border: 1px solid rgba(0, 0, 0, 0.28);
    overflow: hidden;
    color: white;
    font-size: 11px;
    padding: 3px 5px;
    cursor: default;
    text-shadow: 0 1px 1px rgba(0, 0, 0, 0.35);
    display: flex;
    flex-direction: column;
  }
  .tile.dir:not(.inner) {
    border-color: rgba(0, 0, 0, 0.45);
  }
  .tile.inner {
    border-color: rgba(0, 0, 0, 0.18);
    filter: brightness(1.08);
  }
  .tile:hover {
    filter: brightness(1.2);
  }
  .tile.sel {
    outline: 2px solid white;
    outline-offset: -2px;
    z-index: 1;
  }
  .tl {
    font-weight: 600;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .ts {
    opacity: 0.85;
    font-size: 10px;
  }
  .center {
    position: absolute;
    inset: 0;
    display: flex;
    align-items: center;
    justify-content: center;
  }
  .busy {
    position: absolute;
    right: 8px;
    bottom: 8px;
    padding: 4px 8px;
    border-radius: 6px;
    background: var(--bg-elevated);
    font-size: 11px;
  }
  aside {
    border-left: 1px solid var(--border);
    display: flex;
    flex-direction: column;
    min-height: 0;
  }
  .info {
    padding: 10px 12px;
    border-bottom: 1px solid var(--border);
    display: grid;
    gap: 4px;
  }
  .actions {
    display: flex;
    gap: 6px;
    margin-top: 4px;
  }
  .actions .btn {
    display: inline-flex;
    align-items: center;
    gap: 5px;
  }
  .list {
    flex: 1;
    overflow-y: auto;
    padding: 6px;
  }
  .item {
    display: grid;
    grid-template-columns: 1fr 60px 64px;
    gap: 8px;
    align-items: center;
    padding: 3px 6px;
    border-radius: var(--r-s);
    font-size: 12px;
  }
  .item:hover,
  .item.sel {
    background: var(--bg-hover);
  }
  .bar {
    height: 5px;
    border-radius: 3px;
    background: var(--border);
    overflow: hidden;
  }
  .bar span {
    display: block;
    height: 100%;
    background: var(--accent);
  }
  .sz {
    text-align: right;
    font-variant-numeric: tabular-nums;
    color: var(--text-2);
  }
</style>
