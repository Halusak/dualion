<script lang="ts">
  import Lock from '@lucide/svelte/icons/lock';
  import LockKeyhole from '@lucide/svelte/icons/lock-keyhole';
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
    if (tab.location.kind === 'network') return tab.location.host ?? t('drives.network');
    return tab.title || '…';
  }

  /** Tab being dragged to a new position (pointer based: HTML5 drag & drop is taken by file drops). */
  let dragging = $state<number | null>(null);

  function homeName(tab: Tab): string {
    const h = tab.home;
    if (!h) return '';
    const p = h.kind === 'local' || h.kind === 'remote' ? h.path : h.kind === 'archive' ? h.archive : '';
    return p.split(/[\\/]/).filter(Boolean).pop() ?? p;
  }

  function startDrag(e: PointerEvent) {
    const bar = (e.currentTarget as HTMLElement).parentElement!;
    const id = s.tabs[s.active]?.id;
    const sx = e.clientX;
    let moved = false;
    const move = (ev: PointerEvent) => {
      if (!moved && Math.abs(ev.clientX - sx) < 5) return;
      moved = true;
      dragging = id;
      const from = s.tabs.findIndex((t) => t.id === id);
      // New index = number of other tabs whose middle lies left of the pointer.
      let to = 0;
      bar.querySelectorAll<HTMLElement>('.tab').forEach((el, k) => {
        if (k === from) return;
        const r = el.getBoundingClientRect();
        if (ev.clientX > r.left + r.width / 2) to++;
      });
      if (from >= 0 && to !== from) s.moveTab(from, to);
    };
    const up = () => {
      window.removeEventListener('pointermove', move);
      window.removeEventListener('pointerup', up);
      dragging = null;
    };
    window.addEventListener('pointermove', move);
    window.addEventListener('pointerup', up);
  }
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
      class:dragging={dragging === tab.id}
      title={tab.home
        ? `${label(tab)} – ${t('tabs.lockedFolder', { name: homeName(tab) })}`
        : tab.locked
          ? `${label(tab)} – ${t('tabs.locked')}`
          : label(tab)}
      ondblclick={() => void renameTab(s, i)}
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
        } else if (e.button === 0) {
          s.active = i;
          startDrag(e);
        }
      }}
    >
      {#if tab.locked}
        <span class="lock">
          {#if tab.home}<LockKeyhole size={11} strokeWidth={2.2} />{:else}<Lock size={11} strokeWidth={2.2} />{/if}
        </span>
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
  .tab.dragging {
    opacity: 0.75;
    box-shadow: 0 0 0 1px var(--accent) inset;
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
