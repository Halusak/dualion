<script lang="ts">
  import Lock from '@lucide/svelte/icons/lock';
  import Plus from '@lucide/svelte/icons/plus';
  import X from '@lucide/svelte/icons/x';
  import { closeTab, newTab, renameTab } from '../lib/actions';
  import { tabMenu } from '../lib/contextMenu';
  import { t } from '../lib/i18n/index.svelte';
  import { isWindows } from '../lib/platform';
  import { app, type SideName } from '../lib/state/app.svelte';
  import type { Tab } from '../lib/state/tab.svelte';

  let { side, active = false }: { side: SideName; active?: boolean } = $props();
  const s = $derived(app.side(side));

  function label(tab: Tab): string {
    if (tab.customTitle) return tab.customTitle;
    if (tab.location.kind === 'drives') return t(isWindows ? 'place.drivesWin' : 'place.drivesMac');
    return tab.title || '…';
  }

  let dragFrom = -1;
</script>

<!-- svelte-ignore a11y_no_static_element_interactions -->
<div class="tabs" ondblclick={(e) => e.target === e.currentTarget && newTab(side)}>
  {#each s.tabs as tab, i (tab.id)}
    <!-- svelte-ignore a11y_no_static_element_interactions -->
    <div
      class="tab"
      class:on={i === s.active}
      class:focus={active && i === s.active}
      class:locked={tab.locked}
      class:last-locked={tab.locked && !s.tabs[i + 1]?.locked}
      title={tab.locked ? `${label(tab)} – ${t('tabs.locked')}` : label(tab)}
      draggable="true"
      ondragstart={() => (dragFrom = i)}
      ondragover={(e) => e.preventDefault()}
      ondblclick={() => void renameTab(s, i)}
      ondrop={() => {
        if (dragFrom >= 0 && dragFrom !== i) s.moveTab(dragFrom, i);
        dragFrom = -1;
      }}
      oncontextmenu={(e) => {
        e.preventDefault();
        e.stopPropagation();
        void tabMenu(s, i);
      }}
      onpointerdown={(e) => {
        app.activeSide = side;
        if (e.button === 1) {
          e.preventDefault();
          closeTab(s, i);
        } else if (e.button === 0) s.active = i;
      }}
    >
      {#if tab.locked}
        <span class="lock"><Lock size={11} strokeWidth={2.2} /></span>
      {/if}
      <span class="ellipsis label">{label(tab)}</span>
      {#if !tab.locked && s.tabs.length > 1}
        <button
          class="close"
          tabindex="-1"
          aria-label={t('cmd.closeTab')}
          onpointerdown={(e) => e.stopPropagation()}
          onclick={() => closeTab(s, i)}
        >
          <X size={11} strokeWidth={2} />
        </button>
      {/if}
    </div>
  {/each}
  <button class="add" tabindex="-1" title={t('cmd.newTab')} onclick={() => newTab(side)}><Plus size={13} /></button>
</div>

<style>
  .tabs {
    display: flex;
    align-items: stretch;
    height: 30px;
    padding: 0 4px 0 0;
    flex: none;
    background: var(--bg-subtle);
    border-bottom: 1px solid var(--border);
    overflow: hidden;
  }
  .tab {
    flex: 0 1 180px;
    min-width: 54px;
    height: 100%;
    display: flex;
    align-items: center;
    gap: 5px;
    padding: 0 4px 0 10px;
    border-right: 1px solid var(--border);
    color: var(--text-2);
    font-size: 12px;
    position: relative;
  }
  .tab.locked {
    flex: 0 1 140px;
    padding-right: 10px;
  }
  .tab.last-locked {
    border-right-color: var(--border-strong);
  }
  .tab:hover {
    background: var(--bg-hover);
  }
  .tab.on {
    background: var(--bg-panel);
    color: var(--text);
    box-shadow: 0 1px 0 var(--bg-panel);
    font-weight: 600;
    z-index: 1;
  }
  /* Only the active tab of the active panel carries the accent line. */
  .tab.focus::before {
    content: '';
    position: absolute;
    left: 0;
    right: 0;
    top: 0;
    height: 2px;
    background: var(--accent);
    pointer-events: none;
  }
  .label {
    flex: 1;
    min-width: 0;
  }
  .lock {
    display: inline-flex;
    color: var(--accent-text);
    flex: none;
  }
  .close {
    width: 18px;
    height: 18px;
    flex: none;
    border: none;
    border-radius: 4px;
    background: transparent;
    color: var(--text-3);
    display: inline-flex;
    align-items: center;
    justify-content: center;
    padding: 0;
    opacity: 0;
  }
  .tab:hover .close,
  .tab.on .close {
    opacity: 1;
  }
  .close:hover {
    background: var(--bg-pressed);
    color: var(--text);
  }
  .add {
    width: 24px;
    height: 22px;
    align-self: center;
    margin-left: 2px;
    border: none;
    border-radius: var(--r-s);
    background: transparent;
    color: var(--text-2);
    display: inline-flex;
    align-items: center;
    justify-content: center;
  }
  .add:hover {
    background: var(--bg-hover);
    color: var(--text);
  }
</style>
