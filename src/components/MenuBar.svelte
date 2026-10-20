<script lang="ts">
  // In-window menu bar for Windows / Linux, drawn in HTML so it shares the
  // window background and behaves like a Win32 menu bar: once a menu is open,
  // hovering another title opens it; arrows, Enter, Esc and Alt+letter work.
  import Check from '@lucide/svelte/icons/check';
  import ChevronRight from '@lucide/svelte/icons/chevron-right';
  import { onMount } from 'svelte';
  import { buildMenu, commandForCombo, run, shortcutOf } from '../lib/commands';
  import { shortcutLabel } from '../lib/platform';
  import { dialogOpen } from '../lib/state/ui.svelte';
  import type { MenuNode } from '../lib/types';

  type Sub = Extract<MenuNode, { type: 'submenu' }>;

  const tops = $derived(buildMenu().filter((n): n is Sub => n.type === 'submenu'));
  let open = $state(-1);
  /** Menu content, rebuilt on open so check marks are current. */
  let items = $state.raw<MenuNode[]>([]);
  let hi = $state(-1);
  let sub = $state<{ index: number; hi: number } | null>(null);
  let altDown = $state(false);
  let bar: HTMLElement;
  let buttons: HTMLButtonElement[] = [];
  let pos = $state({ x: 0, y: 0 });
  let subPos = $state({ x: 0, y: 0 });

  /** Drops system-only items and leading / doubled / trailing separators. */
  function tidy(list: MenuNode[]): MenuNode[] {
    const out: MenuNode[] = [];
    for (const n of list) {
      if (n.type === 'predefined') continue;
      if (n.type === 'separator' && (!out.length || out[out.length - 1].type === 'separator')) continue;
      out.push(n.type === 'submenu' ? { ...n, items: tidy(n.items) } : n);
    }
    while (out.length && out[out.length - 1].type === 'separator') out.pop();
    return out;
  }

  function openMenu(i: number, highlightFirst = false) {
    const spec = buildMenu().filter((n): n is Sub => n.type === 'submenu');
    if (i < 0 || i >= spec.length) return;
    open = i;
    items = tidy(spec[i].items);
    sub = null;
    hi = highlightFirst ? next(items, -1, 1) : -1;
    const r = buttons[i]?.getBoundingClientRect();
    if (r) pos = { x: r.left, y: r.bottom + 2 };
  }

  function close() {
    open = -1;
    sub = null;
    hi = -1;
  }

  const selectable = (n: MenuNode | undefined) => !!n && n.type !== 'separator' && n.type !== 'predefined' && (n.type === 'submenu' || n.enabled !== false);

  function next(list: MenuNode[], from: number, dir: 1 | -1): number {
    for (let k = 1; k <= list.length; k++) {
      const i = (from + dir * k + list.length * 2) % list.length;
      if (selectable(list[i])) return i;
    }
    return -1;
  }

  function activate(n: MenuNode | undefined) {
    if (!n || (n.type !== 'item' && n.type !== 'check') || n.enabled === false) return;
    close();
    // Let the menu disappear before the command opens dialogs etc.
    setTimeout(() => run(n.id), 0);
  }

  function openSub(index: number, el?: HTMLElement | null, highlightFirst = false) {
    const n = items[index];
    if (n?.type !== 'submenu') return;
    const r = (el ?? document.querySelector<HTMLElement>(`.dd [data-i="${index}"]`))?.getBoundingClientRect();
    if (r) subPos = { x: r.right + 2, y: r.top - 4 };
    sub = { index, hi: highlightFirst ? next(n.items, -1, 1) : -1 };
  }

  function label(n: MenuNode): string {
    return n.type === 'separator' || n.type === 'predefined' ? '' : n.label;
  }

  function keyText(n: MenuNode): string {
    if (n.type !== 'item' && n.type !== 'check') return '';
    const k = shortcutOf(n.id);
    return k ? shortcutLabel(k) : '';
  }

  function onKey(e: KeyboardEvent) {
    altDown = e.key === 'Alt' && !e.repeat ? true : e.key === 'Alt' ? altDown : false;
    // Alt+letter opens the menu with that initial (unless a command uses the combo).
    if (open < 0) {
      if (e.altKey && !e.ctrlKey && !e.metaKey && /^Key[A-Z]$/.test(e.code) && !dialogOpen()) {
        const letter = e.code.slice(3);
        if (commandForCombo(`Alt+${letter}`)) return;
        const i = tops.findIndex((m) => m.label.replace('&', '').toUpperCase().startsWith(letter));
        if (i >= 0) {
          e.preventDefault();
          e.stopPropagation();
          openMenu(i, true);
        }
      }
      return;
    }
    const list = sub ? (items[sub.index] as Sub).items : items;
    const cur = sub ? sub.hi : hi;
    const setCur = (v: number) => (sub ? (sub = { ...sub, hi: v }) : (hi = v));
    let handled = true;
    switch (e.key) {
      case 'ArrowDown':
        setCur(next(list, cur, 1));
        break;
      case 'ArrowUp':
        setCur(next(list, cur < 0 ? 0 : cur, -1));
        break;
      case 'ArrowRight':
        if (!sub && items[hi]?.type === 'submenu') openSub(hi, null, true);
        else openMenu((open + 1) % tops.length, true);
        break;
      case 'ArrowLeft':
        if (sub) sub = null;
        else openMenu((open - 1 + tops.length) % tops.length, true);
        break;
      case 'Enter':
      case ' ':
        if (list[cur]?.type === 'submenu') openSub(cur, null, true);
        else activate(list[cur]);
        break;
      case 'Escape':
        if (sub) sub = null;
        else close();
        break;
      case 'Tab':
        break;
      default:
        handled = e.key === 'Alt';
    }
    if (handled || e.key.length === 1) {
      e.preventDefault();
      e.stopPropagation();
    }
  }

  function onKeyUp(e: KeyboardEvent) {
    if (e.key === 'Alt' && altDown && !dialogOpen()) {
      // A lone Alt press toggles the menu, like Windows.
      e.preventDefault();
      if (open >= 0) close();
      else openMenu(0, true);
    }
    altDown = false;
  }

  onMount(() => {
    const down = (e: PointerEvent) => {
      if (open >= 0 && !(e.target as Element).closest('.menubar, .dd')) close();
    };
    window.addEventListener('keydown', onKey, true);
    window.addEventListener('keyup', onKeyUp, true);
    window.addEventListener('pointerdown', down, true);
    window.addEventListener('blur', close);
    return () => {
      window.removeEventListener('keydown', onKey, true);
      window.removeEventListener('keyup', onKeyUp, true);
      window.removeEventListener('pointerdown', down, true);
      window.removeEventListener('blur', close);
    };
  });
</script>

<nav class="menubar" bind:this={bar} class:alt={altDown}>
  {#each tops as m, i (m.label)}
    <button
      bind:this={buttons[i]}
      class:open={open === i}
      tabindex="-1"
      onpointerdown={(e) => {
        if (e.button !== 0) return;
        e.preventDefault();
        if (open === i) close();
        else openMenu(i);
      }}
      onpointerenter={() => open >= 0 && open !== i && openMenu(i)}
    ><span class="mn">{m.label.slice(0, 1)}</span>{m.label.slice(1)}</button>
  {/each}
</nav>

{#snippet list(nodes: MenuNode[], current: number, onHover: (i: number, el: HTMLElement) => void, nested: boolean)}
  {#each nodes as n, i (i)}
    {#if n.type === 'separator'}
      <div class="sep"></div>
    {:else if n.type !== 'predefined'}
      <!-- svelte-ignore a11y_click_events_have_key_events -->
      <div
        class="mi"
        class:hi={i === current}
        class:disabled={n.type !== 'submenu' && n.enabled === false}
        role="menuitem"
        tabindex="-1"
        data-i={i}
        onpointerenter={(e) => onHover(i, e.currentTarget)}
        onclick={() => (n.type === 'submenu' ? !nested && openSub(i) : activate(n))}
      >
        <span class="ck">{#if n.type === 'check' && n.checked}<Check size={13} />{/if}</span>
        <span class="lb">{label(n)}</span>
        {#if n.type === 'submenu'}
          <span class="kb"><ChevronRight size={13} /></span>
        {:else}
          <span class="kb">{keyText(n)}</span>
        {/if}
      </div>
    {/if}
  {/each}
{/snippet}

{#if open >= 0}
  <div class="dd" role="menu" tabindex="-1" style="left:{pos.x}px; top:{pos.y}px">
    {@render list(items, hi, (i, el) => {
      hi = i;
      if (items[i]?.type === 'submenu') openSub(i, el);
      else sub = null;
    }, false)}
  </div>
  {#if sub && items[sub.index]?.type === 'submenu'}
    <div class="dd" role="menu" tabindex="-1" style="left:{subPos.x}px; top:{subPos.y}px">
      {@render list((items[sub.index] as Sub).items, sub.hi, (i) => sub && (sub = { ...sub, hi: i }), true)}
    </div>
  {/if}
{/if}

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
  .menubar button {
    border: none;
    background: transparent;
    color: var(--text);
    font: inherit;
    font-size: 12px;
    height: 22px;
    padding: 0 9px;
    border-radius: var(--r-s);
  }
  .menubar button:hover,
  .menubar button.open {
    background: var(--bg-hover);
  }
  .menubar.alt .mn {
    text-decoration: underline;
  }
  .dd {
    position: fixed;
    z-index: 70;
    min-width: 220px;
    max-width: 420px;
    max-height: calc(100vh - 40px);
    overflow-y: auto;
    padding: 4px;
    border-radius: 8px;
    background: var(--bg-elevated);
    box-shadow: var(--shadow-pop);
    border: 1px solid var(--border);
    font-size: 12px;
    outline: none;
  }
  .mi {
    display: grid;
    grid-template-columns: 22px 1fr auto;
    align-items: center;
    gap: 6px;
    height: 28px;
    padding: 0 10px 0 4px;
    border-radius: 4px;
    color: var(--text);
    cursor: default;
    white-space: nowrap;
  }
  .mi.hi:not(.disabled) {
    background: var(--bg-hover);
  }
  .mi.disabled {
    color: var(--text-3);
  }
  .ck {
    display: inline-flex;
    justify-content: center;
    color: var(--accent-text);
  }
  .lb {
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .kb {
    color: var(--text-2);
    font-size: 11px;
    padding-left: 24px;
    display: inline-flex;
  }
  .sep {
    height: 1px;
    margin: 4px 8px;
    background: var(--border);
  }
</style>
