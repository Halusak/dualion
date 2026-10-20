<script lang="ts">
  import LoaderCircle from '@lucide/svelte/icons/loader-circle';
  import TriangleAlert from '@lucide/svelte/icons/triangle-alert';
  import { goHome } from '../lib/actions';
  import { drag } from '../lib/dnd.svelte';
  import { t } from '../lib/i18n/index.svelte';
  import { quick } from '../lib/panelKeys.svelte';
  import { app, type SideName } from '../lib/state/app.svelte';
  import { settings } from '../lib/state/settings.svelte';
  import { dialogOpen, errorText, ui } from '../lib/state/ui.svelte';
  import FileGrid from './FileGrid.svelte';
  import FileList from './FileList.svelte';
  import FilterBar from './FilterBar.svelte';
  import PathBar from './PathBar.svelte';
  import QuickView from './QuickView.svelte';
  import StatusLine from './StatusLine.svelte';
  import TabBar from './TabBar.svelte';

  let { side }: { side: SideName } = $props();

  let root: HTMLElement;
  const sideObj = $derived(app.side(side));
  const tab = $derived(sideObj.tab);
  const active = $derived(app.activeSide === side);
  const showQuickView = $derived(app.quickView && !active);
  const dropHere = $derived(drag.active && drag.over?.side === side && !drag.over.entryKey && drag.from !== side);

  // Show the spinner only for slow loads to avoid flicker.
  let slow = $state(false);
  $effect(() => {
    if (!tab.loading) {
      slow = false;
      return;
    }
    const timer = setTimeout(() => (slow = true), 180);
    return () => clearTimeout(timer);
  });

  // Tabs restored from the session load lazily when first shown.
  $effect(() => {
    if (!tab.listing && !tab.loading && !tab.error) void tab.navigate(tab.location);
  });

  // Keyboard focus follows the active panel.
  $effect(() => {
    void ui.dialogs.length;
    if (active && !dialogOpen() && root && !root.contains(document.activeElement)) {
      const el = document.activeElement;
      if (!el || el === document.body || el.closest('[data-side]')) root.focus({ preventScroll: true });
    }
  });
</script>

<!-- svelte-ignore a11y_no_noninteractive_tabindex -->
<section
  role="group"
  aria-label={side}
  class="panel"
  class:active
  class:drop={dropHere}
  data-side={side}
  bind:this={root}
  tabindex="0"
  onpointerdown={() => (app.activeSide = side)}
  onfocusin={() => (app.activeSide = side)}
>
  {#if settings.alwaysShowTabs || sideObj.tabs.length > 1}
    <TabBar {side} />
  {/if}
  <PathBar {tab} {side} {active} />

  <div class="body">
    {#if showQuickView}
      <QuickView />
    {:else if tab.error && !tab.listing}
      <div class="state">
        <TriangleAlert size={28} strokeWidth={1.5} />
        <p>{errorText(tab.error)}</p>
        <div class="row">
          <button class="btn" onclick={() => tab.navigate(tab.location)}>{t('panel.retry')}</button>
          <button class="btn" onclick={() => goHome(tab)}>{t('panel.goHome')}</button>
        </div>
      </div>
    {:else}
      {#if tab.view === 'details'}
        <FileList {tab} {side} {active} />
      {:else}
        <FileGrid {tab} {side} {active} />
      {/if}
      {#if tab.listing && tab.visible.length === 0 && !tab.loading}
        <div class="empty">{tab.filter ? t('panel.noMatch') : t('panel.empty')}</div>
      {/if}
      {#if slow}
        <div class="loading"><LoaderCircle size={20} class="spin" /></div>
      {/if}
    {/if}
  </div>

  {#if tab.filterOpen || tab.filter}
    <FilterBar {tab} />
  {/if}
  {#if quick.tab === tab}
    <div class="quick">
      <span class="muted">{t('panel.quickSearch')}:</span>
      <strong>{quick.text}</strong>
    </div>
  {/if}
  <StatusLine {tab} />
</section>

<style>
  .panel {
    min-width: 0;
    min-height: 0;
    display: flex;
    flex-direction: column;
    background: var(--bg-panel);
    border-radius: var(--r-l);
    border: 1px solid var(--border);
    overflow: hidden;
    position: relative;
    outline: none;
    margin-bottom: 6px;
  }
  :global(:root[data-platform='windows']) .panel {
    border-radius: var(--r-l);
  }
  .panel.active {
    border-color: var(--border-strong);
  }
  .panel.active::after {
    content: '';
    position: absolute;
    left: 0;
    right: 0;
    top: 0;
    height: 2px;
    background: var(--accent);
    opacity: 0.85;
    pointer-events: none;
  }
  .panel.drop {
    box-shadow: inset 0 0 0 2px var(--accent);
  }
  .body {
    flex: 1;
    min-height: 0;
    display: flex;
    flex-direction: column;
    position: relative;
  }
  .state {
    flex: 1;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 10px;
    color: var(--text-2);
    padding: 24px;
    text-align: center;
    white-space: pre-line;
  }
  .state .row {
    display: flex;
    gap: 8px;
  }
  .empty {
    position: absolute;
    top: 40%;
    left: 0;
    right: 0;
    text-align: center;
    color: var(--text-3);
    pointer-events: none;
  }
  .loading {
    position: absolute;
    top: 12px;
    right: 14px;
    color: var(--text-2);
  }
  .loading :global(.spin) {
    animation: spin 0.9s linear infinite;
  }
  @keyframes spin {
    to {
      transform: rotate(360deg);
    }
  }
  .quick {
    position: absolute;
    right: 10px;
    bottom: 34px;
    z-index: 5;
    padding: 5px 10px;
    border-radius: var(--r-m);
    background: var(--bg-elevated);
    box-shadow: var(--shadow-pop);
    display: flex;
    gap: 6px;
  }
</style>
