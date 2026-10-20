<script lang="ts">
  import ChevronDown from '@lucide/svelte/icons/chevron-down';
  import ChevronUp from '@lucide/svelte/icons/chevron-up';
  import { commitRename } from '../lib/actions';
  import { isResults } from '../lib/paths';
  import { columnById, type MetaColumn } from '../lib/columns';
  import { backgroundMenu, columnsMenu, entryMenu } from '../lib/contextMenu';
  import { drag } from '../lib/dnd.svelte';
  import { formatAttrs, formatDate, formatSize } from '../lib/format';
  import { localeTag, t } from '../lib/i18n/index.svelte';
  import { setMetrics } from '../lib/panelKeys.svelte';
  import { isWindows } from '../lib/platform';
  import { backgroundPointerDown, itemDoubleClick, itemPointerDown } from '../lib/pointer';
  import type { SideName } from '../lib/state/app.svelte';
  import { settings } from '../lib/state/settings.svelte';
  import type { Tab } from '../lib/state/tab.svelte';
  import { rowHeight } from '../lib/theme';
  import { F, type Entry, type SortKey } from '../lib/types';
  import FileIcon from './FileIcon.svelte';
  import RenameInput from './RenameInput.svelte';

  let { tab, side, active }: { tab: Tab; side: SideName; active: boolean } = $props();

  let viewport: HTMLDivElement;
  let scrollTop = $state(0);
  let height = $state(300);

  const rowH = $derived(rowHeight(settings.density, isWindows));
  const items = $derived(tab.visible);
  const total = $derived(items.length);
  const start = $derived(Math.max(0, Math.floor(scrollTop / rowH) - 10));
  const end = $derived(Math.min(total, Math.ceil((scrollTop + height) / rowH) + 10));
  const slice = $derived(items.slice(start, end));
  const locale = $derived(localeTag());
  const isSearch = $derived(isResults(tab.location));
  const isDrives = $derived(tab.location.kind === 'drives');
  const w = $derived(settings.colWidths);
  const metaCols = $derived(tab.columns.map((id) => columnById.get(id)!).filter(Boolean));
  const metaW = (c: MetaColumn) => settings.metaWidths[c.id] ?? c.width;
  const cols = $derived(
    `minmax(120px, 1fr) ${isSearch ? 'minmax(100px, 0.8fr)' : `${w.ext}px`} ${w.size}px ${w.date}px${settings.showAttrs ? ` ${w.attrs}px` : ''}${metaCols.map((c) => ` ${metaW(c)}px`).join('')}`,
  );
  // Narrow panels scroll horizontally instead of squeezing the name column.
  const minWidth = $derived(
    180 + (isSearch ? 100 : w.ext) + w.size + w.date + (settings.showAttrs ? w.attrs : 0) + metaCols.reduce((a, c) => a + metaW(c), 0) + 16,
  );
  let headerEl = $state<HTMLDivElement>();

  $effect(() => {
    setMetrics(tab, { perPage: Math.max(1, Math.floor(height / rowH)), cols: 1, rowsPerCol: Math.max(1, Math.floor(height / rowH)) });
  });

  // Keep the cursor visible when it moves via keyboard.
  $effect(() => {
    void tab.revealTick;
    const i = tab.cursor;
    if (!viewport) return;
    const top = i * rowH;
    if (top < viewport.scrollTop) viewport.scrollTop = top;
    else if (top + rowH > viewport.scrollTop + height) viewport.scrollTop = top + rowH - height;
  });

  function sizeText(e: Entry): string {
    if (e.flags & F.UP) return '';
    if (e.flags & F.VOLUME) return formatSize(e.size, locale, settings.sizeFormat);
    if (e.isDir) {
      const s = tab.dirSizes.get(e.key);
      if (s === undefined) return t('panel.dir');
      if (s < 0) return '…';
      return formatSize(s, locale, settings.sizeFormat);
    }
    return formatSize(e.groupBytes !== undefined && !tab.expandedGroups.has(e.key) ? e.groupBytes : e.size, locale, settings.sizeFormat);
  }

  function volume(e: Entry) {
    return tab.volumes.find((v) => v.path === e.name);
  }

  function header(key: SortKey) {
    tab.setSort(key);
  }

  function startResize(e: PointerEvent, col: string) {
    e.stopPropagation();
    e.preventDefault();
    const startX = e.clientX;
    const meta = columnById.get(col);
    const startW = meta ? metaW(meta) : settings.colWidths[col as 'ext'];
    const target = e.currentTarget as HTMLElement;
    target.setPointerCapture(e.pointerId);
    const move = (ev: PointerEvent) => {
      // Handles sit on the left edge of their column: dragging left widens it.
      const width = Math.max(40, Math.min(400, startW - (ev.clientX - startX)));
      if (meta) settings.metaWidths = { ...settings.metaWidths, [col]: width };
      else settings.colWidths = { ...settings.colWidths, [col]: width };
    };
    const up = () => {
      target.removeEventListener('pointermove', move);
      target.removeEventListener('pointerup', up);
    };
    target.addEventListener('pointermove', move);
    target.addEventListener('pointerup', up);
  }

  function onContext(e: MouseEvent, entry: Entry | null) {
    e.preventDefault();
    if (entry && !(entry.flags & F.UP)) void entryMenu(tab);
    else void backgroundMenu(tab);
  }
</script>

<div class="details" style="--cols: {cols}; --row-h: {rowH}px; --min-w: {minWidth}px">
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <div
    class="header"
    bind:this={headerEl}
    oncontextmenu={(e) => {
      e.preventDefault();
      e.stopPropagation();
      void columnsMenu(tab);
    }}
  >
   <div class="hgrid">
    {#snippet col(key: SortKey, label: string, cls: string, resize: string | null)}
      <button class="h {cls}" class:sorted={tab.sortKey === key} onclick={() => header(key)}>
        {#if resize}
          <!-- svelte-ignore a11y_no_static_element_interactions -->
          <span class="grip" onpointerdown={(e) => startResize(e, resize)}></span>
        {/if}
        <span class="ellipsis">{label}</span>
        {#if tab.sortKey === key}
          {#if tab.sortDesc}<ChevronDown size={12} />{:else}<ChevronUp size={12} />{/if}
        {/if}
      </button>
    {/snippet}
    {@render col('name', t('col.name'), 'name', null)}
    {#if isSearch}
      <div class="h">{t('col.folder')}</div>
    {:else}
      {@render col('ext', t('col.ext'), '', 'ext')}
    {/if}
    {@render col('size', t('col.size'), 'right', 'size')}
    {@render col('mtime', t('col.date'), '', 'date')}
    {#if settings.showAttrs}<div class="h">{t('col.attrs')}</div>{/if}
    {#each metaCols as c (c.id)}
      {@render col(`m:${c.id}`, t(c.label), c.right ? 'right' : '', c.id)}
    {/each}
   </div>
  </div>

  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <div
    class="viewport"
    bind:this={viewport}
    bind:clientHeight={height}
    onscroll={() => {
      scrollTop = viewport.scrollTop;
      if (headerEl) headerEl.scrollLeft = viewport.scrollLeft;
    }}
    onpointerdown={(e) => backgroundPointerDown(e, side, tab)}
    oncontextmenu={(e) => onContext(e, null)}
  >
    <div class="spacer" style="height: {total * rowH}px">
      {#each slice as e, i (e.key)}
        {@const index = start + i}
        {@const cursor = index === tab.cursor}
        {@const marked = tab.marked.has(e.key)}
        {@const vol = isDrives ? volume(e) : undefined}
        <!-- svelte-ignore a11y_no_static_element_interactions -->
        <div
          class="row"
          class:alt={index % 2 === 1}
          class:cursor
          class:active
          class:marked
          class:hidden-file={(e.flags & F.HIDDEN) !== 0}
          class:diff={tab.diff.has(e.key)}
          class:drop={drag.active && drag.over?.side === side && drag.over.entryKey === e.key}
          data-key={e.key}
          style="top: {index * rowH}px"
          onpointerdown={(ev) => itemPointerDown(ev, side, tab, e, index)}
          ondblclick={(ev) => itemDoubleClick(tab, e, ev)}
          oncontextmenu={(ev) => {
            ev.stopPropagation();
            onContext(ev, e);
          }}
        >
          <div class="c name">
            <FileIcon entry={e} location={tab.location} size={16} />
            {#if e.groupOf}<span class="branch">↳</span>{/if}
            {#if tab.renaming === e.key}
              <RenameInput value={e.name} isDir={e.isDir} onDone={(v) => (v === null ? (tab.renaming = null) : commitRename(tab, e, v))} />
            {:else}
              <span class="ellipsis label">{vol ? vol.name : e.name}</span>
              {#if e.groupSize}
                <button
                  class="group"
                  class:open={tab.expandedGroups.has(e.key)}
                  tabindex="-1"
                  title={t('groups.tip', { names: (tab.groupMembers(e.key) ?? []).map((m) => m.ext).join(', ') })}
                  onpointerdown={(ev) => ev.stopPropagation()}
                  onclick={() => tab.toggleGroup(e.key)}>+{e.groupSize - 1}</button
                >
              {/if}
            {/if}
          </div>
          {#if isSearch}
            <div class="c muted ellipsis" title={e.note}>{e.note}</div>
          {:else}
            <div class="c muted ellipsis">{e.ext}</div>
          {/if}
          <div class="c right size">{sizeText(e)}</div>
          <div class="c muted date">
            {#if vol}
              <span class="bar"><span style="width: {vol.total ? (100 * (vol.total - vol.free)) / vol.total : 0}%"></span></span>
              {formatSize(vol.free, locale, settings.sizeFormat)}
            {:else}
              {formatDate(e.mtime, locale)}
            {/if}
          </div>
          {#if settings.showAttrs}
            <div class="c muted mono attrs">{e.flags & F.UP ? '' : formatAttrs(e.attrs, isWindows)}</div>
          {/if}
          {#if metaCols.length}
            {@const m = tab.meta(e)}
            {#each metaCols as c (c.id)}
              {@const v = m ? c.text(m, locale) : ''}
              <div class="c muted ellipsis meta" class:right={c.right} title={v.length > 14 ? v : undefined}>{v}</div>
            {/each}
          {/if}
        </div>
      {/each}
    </div>
  </div>
</div>

<style>
  .details {
    flex: 1;
    min-height: 0;
    display: flex;
    flex-direction: column;
  }
  .hgrid {
    display: grid;
    grid-template-columns: var(--cols);
    min-width: var(--min-w);
    height: 100%;
  }
  .header {
    overflow: hidden;
    flex: none;
    height: 24px;
    border-bottom: 1px solid var(--border);
    background: var(--bg-header);
    padding: 0 8px 0 0;
  }
  .h {
    position: relative;
    display: flex;
    align-items: center;
    gap: 4px;
    min-width: 0;
    padding: 0 8px;
    border: none;
    background: transparent;
    color: var(--text-2);
    font-size: var(--fs-small);
    text-align: left;
    height: 100%;
  }
  :global(:root[data-platform='windows']) .h {
    font-size: 12px;
  }
  .h.name {
    padding-left: 32px;
  }
  .h:hover {
    background: var(--bg-hover);
  }
  .h.sorted {
    color: var(--text);
    font-weight: 600;
  }
  .h.right {
    justify-content: flex-end;
  }
  .grip {
    position: absolute;
    left: -3px;
    top: 0;
    bottom: 0;
    width: 7px;
    cursor: col-resize;
    z-index: 1;
  }
  .grip::after {
    content: '';
    position: absolute;
    left: 3px;
    top: 5px;
    bottom: 5px;
    width: 1px;
    background: var(--border);
  }
  .viewport {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    overflow-x: auto;
    position: relative;
    outline: none;
  }
  .spacer {
    position: relative;
    min-height: 100%;
    min-width: var(--min-w);
  }
  .meta {
    font-variant-numeric: tabular-nums;
  }
  .row {
    position: absolute;
    left: 4px;
    right: 4px;
    height: var(--row-h);
    display: grid;
    grid-template-columns: var(--cols);
    align-items: center;
    border-radius: var(--r-s);
    contain: strict;
  }
  .row.alt {
    background: var(--bg-row-alt);
  }
  .row.marked {
    background: var(--mark-bg);
    color: var(--mark-fg);
  }
  .row.marked .label {
    font-weight: 600;
  }
  .row.cursor {
    background: var(--cursor-bg-inactive);
    color: var(--cursor-fg-inactive);
  }
  .row.cursor.active {
    background: var(--cursor-bg);
    color: var(--cursor-fg);
  }
  .row.cursor.active .muted {
    color: inherit;
    opacity: 0.85;
  }
  :global(:root[data-platform='windows']) .row.cursor.active::before {
    content: '';
    position: absolute;
    left: 0;
    top: 25%;
    bottom: 25%;
    width: 3px;
    border-radius: 2px;
    background: var(--accent);
  }
  :global(:root[data-platform='windows']) .row:not(.cursor):hover {
    background: var(--bg-hover);
  }
  .row.hidden-file .label,
  .row.hidden-file :global(img) {
    opacity: 0.55;
  }
  .row.diff .label {
    color: var(--diff-fg);
  }
  .row.drop {
    box-shadow: inset 0 0 0 2px var(--accent);
  }
  .c {
    min-width: 0;
    padding: 0 8px;
    white-space: nowrap;
  }
  .c.name {
    display: flex;
    align-items: center;
    gap: 7px;
    padding-left: 6px;
  }
  .c.right {
    text-align: right;
  }
  .size,
  .date {
    font-variant-numeric: tabular-nums;
  }
  .date {
    display: flex;
    align-items: center;
    gap: 6px;
  }
  .bar {
    display: inline-block;
    width: 48px;
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
  .attrs {
    font-size: 11px;
  }
  .branch {
    color: var(--text-3);
    margin-left: 6px;
    margin-right: -3px;
  }
  .group {
    flex: none;
    border: none;
    padding: 0 5px;
    height: 16px;
    line-height: 16px;
    border-radius: 8px;
    font-size: 10px;
    font-weight: 700;
    background: var(--accent-20);
    color: var(--accent-text);
    font-variant-numeric: tabular-nums;
  }
  .group.open {
    background: var(--accent);
    color: white;
  }
  .row.cursor.active .group {
    background: rgba(255, 255, 255, 0.25);
    color: inherit;
  }
</style>
