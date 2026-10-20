<script lang="ts">
  // In-window menu bar for Windows / Linux. The native Win32 menu bar cannot
  // share the window's Mica background and flickers whenever it is rebuilt,
  // so the bar is drawn here and each menu opens as a native popup.
  import { buildMenu } from '../lib/commands';
  import { popupMenuNodes } from '../lib/contextMenu';
  import type { MenuNode } from '../lib/types';

  // Built lazily on open so check marks always reflect the current state.
  const top = $derived(buildMenu().filter((n): n is Extract<MenuNode, { type: 'submenu' }> => n.type === 'submenu'));
  let open = $state(-1);

  async function show(i: number, el: HTMLElement) {
    if (open >= 0) return;
    const r = el.getBoundingClientRect();
    open = i;
    try {
      await popupMenuNodes(buildMenu().filter((n) => n.type === 'submenu')[i].items, { x: r.left, y: r.bottom });
    } finally {
      open = -1;
    }
  }
</script>

<nav class="menubar">
  {#each top as m, i (m.label)}
    <button
      class:open={open === i}
      tabindex="-1"
      onpointerdown={(e) => {
        if (e.button !== 0) return;
        e.preventDefault();
        void show(i, e.currentTarget);
      }}>{m.label}</button
    >
  {/each}
</nav>

<style>
  .menubar {
    flex: none;
    display: flex;
    align-items: center;
    gap: 1px;
    height: 28px;
    padding: 0 6px;
    background: var(--bg-chrome);
    border-bottom: 1px solid var(--border);
  }
  button {
    border: none;
    background: transparent;
    color: var(--text);
    font: inherit;
    font-size: 12px;
    height: 22px;
    padding: 0 9px;
    border-radius: var(--r-s);
  }
  button:hover,
  button.open {
    background: var(--bg-hover);
  }
</style>
