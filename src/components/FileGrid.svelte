<script lang="ts">
  import Cloud from '@lucide/svelte/icons/cloud';
  import { t } from '../lib/i18n/index.svelte';
  import { commitRename } from '../lib/actions';
  import { backgroundMenu, entryMenu } from '../lib/contextMenu';
  import { drag } from '../lib/dnd.svelte';
  import { setMetrics } from '../lib/panelKeys.svelte';
  import { isWindows } from '../lib/platform';
  import { backgroundPointerDown, itemPointerDown } from '../lib/pointer';
  import type { SideName } from '../lib/state/app.svelte';
  import { settings } from '../lib/state/settings.svelte';
  import type { Tab } from '../lib/state/tab.svelte';
  import { NAME_LINE_H, nameLines, rowHeight } from '../lib/theme';
  import { F, type Entry } from '../lib/types';
  import FileIcon from './FileIcon.svelte';
  import RenameInput from './RenameInput.svelte';

  let { tab, side, active }: { tab: Tab; side: SideName; active: boolean } = $props();

  let viewport: HTMLDivElement;
  let width = $state(400);
  let height = $state(300);
  let scrollTop = $state(0);
  let scrollLeft = $state(0);

  const mode = $derived(tab.view);
  const items = $derived(tab.visible);
  const lines = $derived(nameLines());
  const rowH = $derived(rowHeight(settings.density, isWindows) + (lines - 1) * NAME_LINE_H);

  // "list": column flow (top-to-bottom, then next column), horizontal scrolling.
  const listColW = 230;
  const rowsPerCol = $derived(Math.max(1, Math.floor((height - 4) / rowH)));
  const colsTotal = $derived(Math.ceil(items.length / rowsPerCol));

  // "icons" / "thumbs": row flow grid, vertical scrolling.
  const iconSize = $derived(mode === 'thumbs' ? settings.thumbSize : 48);
  const cellW = $derived(mode === 'thumbs' ? settings.thumbSize + 28 : 104);
  // Cells were sized for two-line names; adjust for the chosen line count.
  const cellH = $derived((mode === 'thumbs' ? settings.thumbSize + 46 : 92) + (lines - 2) * NAME_LINE_H);
  const perRow = $derived(Math.max(1, Math.floor((width - 8) / cellW)));
  const rowsTotal = $derived(Math.ceil(items.length / perRow));

  const range = $derived.by(() => {
    if (mode === 'list') {
      const c0 = Math.max(0, Math.floor(scrollLeft / listColW) - 1);
      const c1 = Math.min(colsTotal, Math.ceil((scrollLeft + width) / listColW) + 1);
      return [c0 * rowsPerCol, Math.min(items.length, c1 * rowsPerCol)];
    }
    const r0 = Math.max(0, Math.floor(scrollTop / cellH) - 2);
    const r1 = Math.min(rowsTotal, Math.ceil((scrollTop + height) / cellH) + 2);
    return [r0 * perRow, Math.min(items.length, r1 * perRow)];
  });
  const slice = $derived(items.slice(range[0], range[1]));

  $effect(() => {
    if (mode === 'list') setMetrics(tab, { perPage: rowsPerCol, cols: 1, rowsPerCol });
    else setMetrics(tab, { perPage: Math.max(1, Math.floor(height / cellH)) * perRow, cols: perRow, rowsPerCol: 1 });
  });

  $effect(() => {
    void tab.revealTick;
    const i = tab.cursor;
    if (!viewport) return;
    if (mode === 'list') {
      const left = Math.floor(i / rowsPerCol) * listColW;
      if (left < viewport.scrollLeft) viewport.scrollLeft = left;
      else if (left + listColW > viewport.scrollLeft + width) viewport.scrollLeft = left + listColW - width;
    } else {
      const top = Math.floor(i / perRow) * cellH;
      if (top < viewport.scrollTop) viewport.scrollTop = top;
      else if (top + cellH > viewport.scrollTop + height) viewport.scrollTop = top + cellH - height;
    }
  });

  function pos(index: number): string {
    if (mode === 'list') {
      const c = Math.floor(index / rowsPerCol);
      const r = index % rowsPerCol;
      return `left:${c * listColW + 4}px;top:${r * rowH + 2}px;width:${listColW - 6}px;height:${rowH}px`;
    }
    const r = Math.floor(index / perRow);
    const c = index % perRow;
    return `left:${c * cellW + 4}px;top:${r * cellH + 4}px;width:${cellW - 6}px;height:${cellH - 6}px`;
  }

  function onScroll() {
    scrollTop = viewport.scrollTop;
    scrollLeft = viewport.scrollLeft;
  }

  // Column flow scrolls horizontally; map the vertical wheel to it
  // (non-passive listener, so the default vertical scroll can be prevented).
  $effect(() => {
    const el = viewport;
    const onWheel = (e: WheelEvent) => {
      if (mode === 'list' && Math.abs(e.deltaY) > Math.abs(e.deltaX)) {
        el.scrollLeft += e.deltaY;
        e.preventDefault();
      }
    };
    el.addEventListener('wheel', onWheel, { passive: false });
    return () => el.removeEventListener('wheel', onWheel);
  });

  function onContext(e: MouseEvent, entry: Entry | null) {
    e.preventDefault();
    if (entry && !(entry.flags & F.UP)) void entryMenu(tab);
    else void backgroundMenu(tab);
  }
</script>

<!-- svelte-ignore a11y_no_static_element_interactions -->
<div
  class="grid {mode}"
  class:wrap={lines > 1}
  bind:this={viewport}
  bind:clientWidth={width}
  bind:clientHeight={height}
  onscroll={onScroll}
  onpointerdown={(e) => backgroundPointerDown(e, side, tab)}
  oncontextmenu={(e) => onContext(e, null)}
  style="--row-h: {rowH}px; --lines: {lines}"
>
  <div
    class="spacer"
    style={mode === 'list' ? `width:${colsTotal * listColW + 8}px;height:100%` : `height:${rowsTotal * cellH + 8}px`}
  >
    {#each slice as e, i (e.key)}
      {@const index = range[0] + i}
      <!-- svelte-ignore a11y_no_static_element_interactions -->
      <div
        class="item"
        class:cursor={index === tab.cursor}
        class:active
        class:marked={tab.marked.has(e.key)}
        class:hidden-file={(e.flags & F.HIDDEN) !== 0}
        class:diff={tab.diff.has(e.key)}
        class:drop={drag.active && drag.over?.side === side && drag.over.entryKey === e.key}
        data-key={e.key}
        style={pos(index)}
        title={e.name}
        onpointerdown={(ev) => itemPointerDown(ev, side, tab, e, index)}
        oncontextmenu={(ev) => {
          ev.stopPropagation();
          onContext(ev, e);
        }}
      >
        <div class="icon">
          <FileIcon entry={e} location={tab.location} size={mode === 'list' ? 16 : iconSize} thumb={mode === 'thumbs'} />
          {#if e.flags & F.CLOUD}<span class="cloudmark" title={t('cloud.onlineOnly')}><Cloud size={12} /></span>{/if}
          {#if e.groupSize}
            <button
              class="group"
              class:open={tab.expandedGroups.has(e.key)}
              tabindex="-1"
              title={(tab.groupMembers(e.key) ?? []).map((m) => m.name).join('\n')}
              onpointerdown={(ev) => ev.stopPropagation()}
              ondblclick={(ev) => ev.stopPropagation()}
              onclick={() => tab.toggleGroup(e.key)}>+{e.groupSize - 1}</button
            >
          {/if}
        </div>
        {#if tab.renaming === e.key}
          <RenameInput value={e.name} isDir={e.isDir} onDone={(v) => (v === null ? (tab.renaming = null) : commitRename(tab, e, v))} />
        {:else}
          <span class="name">{e.name}</span>
        {/if}
      </div>
    {/each}
  </div>
</div>

<style>
  .grid {
    flex: 1;
    min-height: 0;
    position: relative;
    overflow: auto;
    outline: none;
  }
  .grid.list {
    overflow-y: hidden;
  }
  .grid:not(.list) {
    overflow-x: hidden;
  }
  .spacer {
    position: relative;
    min-height: 100%;
  }
  .item {
    position: absolute;
    border-radius: var(--r-m);
    display: flex;
    min-width: 0;
  }
  .list .item {
    align-items: center;
    gap: 7px;
    padding: 0 6px;
    border-radius: var(--r-s);
  }
  .list .name {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .list.wrap .name {
    white-space: normal;
    display: -webkit-box;
    -webkit-line-clamp: var(--lines);
    line-clamp: var(--lines);
    -webkit-box-orient: vertical;
    word-break: break-word;
    line-height: 15px;
  }
  .icons .item,
  .thumbs .item {
    flex-direction: column;
    align-items: center;
    padding: 6px 4px 4px;
    gap: 4px;
  }
  .icons .icon,
  .thumbs .icon {
    flex: 1;
    display: flex;
    align-items: center;
    justify-content: center;
    min-height: 0;
  }
  .icons .name,
  .thumbs .name {
    max-width: 100%;
    text-align: center;
    line-height: 1.25;
    display: -webkit-box;
    -webkit-line-clamp: var(--lines);
    line-clamp: var(--lines);
    -webkit-box-orient: vertical;
    overflow: hidden;
    word-break: break-word;
    padding: 0 3px;
    border-radius: 4px;
    font-size: 12px;
  }
  .item.marked {
    background: var(--mark-bg);
    color: var(--mark-fg);
  }
  .item.marked .name {
    font-weight: 600;
  }
  .list .item.cursor {
    background: var(--cursor-bg-inactive);
  }
  .list .item.cursor.active {
    background: var(--cursor-bg);
    color: var(--cursor-fg);
  }
  .icons .item.cursor,
  .thumbs .item.cursor {
    background: var(--cursor-bg-inactive);
  }
  .icons .item.cursor.active,
  .thumbs .item.cursor.active {
    background: var(--accent-20);
  }
  :global(:root[data-platform='macos']) .icons .item.cursor.active .name,
  :global(:root[data-platform='macos']) .thumbs .item.cursor.active .name {
    background: var(--accent);
    color: var(--accent-fg);
  }
  .item:not(.cursor):hover {
    background: var(--bg-hover);
  }
  .item.hidden-file {
    opacity: 0.6;
  }
  .item.diff .name {
    color: var(--diff-fg);
  }
  .item.drop {
    box-shadow: inset 0 0 0 2px var(--accent);
  }
  .icon {
    position: relative;
  }
  .group {
    position: absolute;
    right: -4px;
    bottom: -2px;
    border: none;
    padding: 0 5px;
    height: 16px;
    line-height: 16px;
    border-radius: 8px;
    font-size: 10px;
    font-weight: 700;
    background: var(--accent);
    color: white;
    box-shadow: 0 0 0 2px var(--bg-panel);
  }
  .group.open {
    background: var(--text-2);
  }
  .cloudmark {
    position: absolute;
    left: -3px;
    bottom: -2px;
    display: inline-flex;
    padding: 1px;
    border-radius: 50%;
    background: var(--bg-panel);
    color: var(--text-2);
  }
</style>
