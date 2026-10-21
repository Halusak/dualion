<script lang="ts">
  import { isResults } from '../lib/paths';
  import ChevronRight from '@lucide/svelte/icons/chevron-right';
  import HardDrive from '@lucide/svelte/icons/hard-drive';
  import House from '@lucide/svelte/icons/house';
  import Globe from '@lucide/svelte/icons/globe';
  import Monitor from '@lucide/svelte/icons/monitor';
  import Server from '@lucide/svelte/icons/server';
  import Archive from '@lucide/svelte/icons/archive';
  import Search from '@lucide/svelte/icons/search';
  import { tick } from 'svelte';
  import { goTo } from '../lib/actions';
  import { api } from '../lib/api';
  import { t } from '../lib/i18n/index.svelte';
  import { locPath } from '../lib/paths';
  import { isWindows } from '../lib/platform';
  import { placesMenu, popupLocations, subfoldersMenu } from '../lib/placesMenu';
  import { app, type SideName } from '../lib/state/app.svelte';
  import { settings } from '../lib/state/settings.svelte';
  import type { Tab } from '../lib/state/tab.svelte';
  import type { Crumb } from '../lib/types';

  let { tab, side, active }: { tab: Tab; side: SideName; active: boolean } = $props();

  let editing = $state(false);
  let text = $state('');
  let input = $state<HTMLInputElement>();
  let suggestions = $state<string[]>([]);
  let selected = $state(-1);
  let placesBtn: HTMLButtonElement;
  let completeTimer: ReturnType<typeof setTimeout>;

  const crumbs = $derived<Crumb[]>(tab.listing?.crumbs ?? []);

  // Leading crumbs that do not fit go into an overflow menu (like Explorer's «).
  let navWidth = $state(0);
  let fontSpec = $state({ size: '13px', family: 'system-ui' });
  const measure = document.createElement('canvas').getContext('2d');
  function crumbWidth(c: Crumb, last: boolean): number {
    if (!measure) return 80;
    measure.font = `${last ? 600 : 400} ${fontSpec.size} ${fontSpec.family}`;
    const icon = c.icon === 'home' || c.icon === 'server' || c.icon === 'archive' ? 17 : 0;
    return Math.ceil(measure.measureText(crumbLabel(c)).width) + 12 + icon + 16;
  }
  const firstVisible = $derived.by(() => {
    if (!navWidth) return 0;
    const avail = navWidth - 56;
    let sum = 0;
    for (let i = crumbs.length - 1; i >= 0; i--) {
      sum += crumbWidth(crumbs[i], i === crumbs.length - 1);
      if (sum > avail && i < crumbs.length - 1) return i + 1;
    }
    return 0;
  });

  function overflowMenu(e: MouseEvent) {
    e.stopPropagation();
    app.activeSide = side;
    const r = (e.currentTarget as HTMLElement).getBoundingClientRect();
    const hidden = crumbs.slice(0, firstVisible).reverse();
    void popupLocations(
      hidden.map((c) => ({ label: crumbLabel(c), location: c.location })),
      tab,
      { x: r.left, y: r.bottom + 2 },
    );
  }

  function crumbLabel(c: Crumb): string {
    if (c.icon === 'drives') return t(isWindows ? 'place.drivesWin' : 'place.drivesMac');
    if (c.icon === 'network') return t('drives.network');
    return c.label || '/';
  }

  async function startEdit() {
    app.activeSide = side;
    text = tab.location.kind === 'remote' ? tab.location.path : locPath(tab.location);
    editing = true;
    suggestions = [];
    selected = -1;
    await tick();
    input?.focus();
    input?.select();
  }

  function cancel() {
    editing = false;
    suggestions = [];
  }

  async function submit(value = text) {
    const ok = await goTo(tab, value);
    if (ok) cancel();
  }

  function scheduleComplete() {
    clearTimeout(completeTimer);
    if (tab.location.kind === 'remote') return;
    completeTimer = setTimeout(async () => {
      const base = tab.location.kind === 'local' ? tab.location.path : null;
      suggestions = await api.completePath(text, base, settings.showHidden).catch(() => []);
      selected = -1;
    }, 120);
  }

  function onKey(e: KeyboardEvent) {
    e.stopPropagation();
    if (e.key === 'Escape') {
      e.preventDefault();
      cancel();
      app.activeSide = side;
    } else if (e.key === 'Enter') {
      e.preventDefault();
      void submit(selected >= 0 ? suggestions[selected] : text);
    } else if (e.key === 'ArrowDown' && suggestions.length) {
      e.preventDefault();
      selected = (selected + 1) % suggestions.length;
    } else if (e.key === 'ArrowUp' && suggestions.length) {
      e.preventDefault();
      selected = selected <= 0 ? suggestions.length - 1 : selected - 1;
    } else if (e.key === 'Tab' && suggestions.length) {
      e.preventDefault();
      const s = suggestions[selected >= 0 ? selected : 0];
      text = s + (isWindows ? '\\' : '/');
      scheduleComplete();
    }
  }

  // Ctrl+L / Cmd+L and Alt+F1 / Alt+F2 requests from the command layer.
  $effect(() => {
    if (app.editPath === side) {
      app.editPath = null;
      void startEdit();
    }
  });
  $effect(() => {
    if (app.placesFor === side) {
      app.placesFor = null;
      const r = placesBtn.getBoundingClientRect();
      void placesMenu(tab, { x: r.left, y: r.bottom + 2 });
    }
  });

  function openPlaces() {
    app.activeSide = side;
    const r = placesBtn.getBoundingClientRect();
    void placesMenu(tab, { x: r.left, y: r.bottom + 2 });
  }

  function chevron(e: MouseEvent, c: Crumb) {
    e.stopPropagation();
    app.activeSide = side;
    const r = (e.currentTarget as HTMLElement).getBoundingClientRect();
    void subfoldersMenu(tab, c.location, { x: r.left, y: r.bottom + 2 });
  }
</script>

<div class="pathbar" class:active class:editing>
  <button class="places" bind:this={placesBtn} title={t('cmd.places')} onclick={openPlaces}>
    {#if tab.location.kind === 'remote'}
      <Server size={15} strokeWidth={1.75} />
    {:else if tab.location.kind === 'drives'}
      <Monitor size={15} strokeWidth={1.75} />
    {:else if tab.location.kind === 'network'}
      <Globe size={15} strokeWidth={1.75} />
    {:else}
      <HardDrive size={15} strokeWidth={1.75} />
    {/if}
  </button>

  {#if editing}
    <div class="edit">
      <input
        bind:this={input}
        bind:value={text}
        class="path-input"
        spellcheck="false"
        autocomplete="off"
        oninput={scheduleComplete}
        onkeydown={onKey}
        onblur={() => setTimeout(cancel, 120)}
      />
      {#if suggestions.length}
        <ul class="suggest" role="listbox">
          {#each suggestions as s, i (s)}
            <li role="option" aria-selected={i === selected} class:sel={i === selected}>
              <button onmousedown={(e) => e.preventDefault()} onclick={() => submit(s)}>{s}</button>
            </li>
          {/each}
        </ul>
      {/if}
    </div>
  {:else}
    <nav
      class="crumbs"
      bind:clientWidth={navWidth}
      {@attach (el) => {
        const cs = getComputedStyle(el);
        fontSpec = { size: cs.fontSize, family: cs.fontFamily };
      }}
    >
      {#if firstVisible > 0}
        <button class="crumb more" tabindex="-1" title="…" onclick={overflowMenu}>…</button>
      {/if}
      {#each crumbs as c, i (i)}
        {#if i < firstVisible}
          <!-- hidden in the overflow menu -->
        {:else}
        {#if i > 0}
          <button class="chev" tabindex="-1" onclick={(e) => chevron(e, crumbs[i - 1])} aria-label="›">
            <ChevronRight size={12} strokeWidth={2} />
          </button>
        {/if}
        <button
          class="crumb"
          class:last={i === crumbs.length - 1}
          tabindex="-1"
          title={crumbLabel(c)}
          onclick={() => {
            app.activeSide = side;
            void tab.navigate(c.location);
          }}
        >
          {#if c.icon === 'home'}<House size={13} strokeWidth={1.75} />{/if}
          {#if c.icon === 'server'}<Server size={13} strokeWidth={1.75} />{/if}
          {#if c.icon === 'archive'}<Archive size={13} strokeWidth={1.75} />{/if}
          {#if isResults(tab.location)}<Search size={13} strokeWidth={1.75} />{/if}
          <span class="ellipsis">{crumbLabel(c)}</span>
        </button>
        {/if}
      {/each}
      {#if crumbs.length}
        <button class="chev tail" tabindex="-1" onclick={(e) => chevron(e, crumbs[crumbs.length - 1])} aria-label="›">
          <ChevronRight size={12} strokeWidth={2} />
        </button>
      {/if}
      <button class="rest" tabindex="-1" title={t('panel.editPath')} onclick={startEdit} aria-label={t('panel.editPath')}></button>
    </nav>
  {/if}
</div>

<style>
  .pathbar {
    display: flex;
    align-items: center;
    gap: 2px;
    height: 30px;
    padding: 0 4px;
    flex: none;
    border-bottom: 1px solid var(--border);
    background: var(--bg-header);
  }
  .places {
    width: 26px;
    height: 24px;
    flex: none;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    border: none;
    border-radius: var(--r-s);
    background: transparent;
    color: var(--text-2);
  }
  .places:hover {
    background: var(--bg-hover);
    color: var(--text);
  }
  .crumbs {
    flex: 1;
    min-width: 0;
    display: flex;
    align-items: center;
    height: 100%;
    overflow: hidden;
  }
  .crumb {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    min-width: 28px;
    flex: 0 1 auto;
    height: 22px;
    padding: 0 5px;
    border: none;
    border-radius: var(--r-s);
    background: transparent;
    color: var(--text-2);
    white-space: nowrap;
  }
  .crumb.more {
    min-width: 0;
    flex: none;
    padding: 0 6px;
  }
  .crumb.last {
    flex-shrink: 0;
    max-width: 60%;
    color: var(--text);
    font-weight: 600;
  }
  .active .crumb.last {
    color: var(--text);
  }
  .crumb:hover,
  .chev:hover {
    background: var(--bg-hover);
    color: var(--text);
  }
  .chev {
    flex: none;
    width: 16px;
    height: 22px;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    padding: 0;
    border: none;
    border-radius: var(--r-s);
    background: transparent;
    color: var(--text-3);
  }
  .rest {
    flex: 1;
    min-width: 24px;
    height: 100%;
    border: none;
    background: transparent;
    cursor: text;
  }
  .edit {
    flex: 1;
    position: relative;
    min-width: 0;
  }
  .path-input {
    width: 100%;
    height: 24px;
    padding: 0 7px;
    border-radius: var(--r-s);
    border: 1px solid var(--accent);
    background: var(--bg-input);
    outline: none;
    box-shadow: var(--focus-ring);
    font-family: var(--font-ui);
  }
  .suggest {
    position: absolute;
    left: 0;
    right: 0;
    top: 28px;
    z-index: 30;
    margin: 0;
    padding: 4px;
    list-style: none;
    max-height: 260px;
    overflow: auto;
    border-radius: var(--r-m);
    background: var(--bg-elevated);
    box-shadow: var(--shadow-pop);
  }
  .suggest button {
    width: 100%;
    text-align: left;
    padding: 4px 8px;
    border: none;
    border-radius: var(--r-s);
    background: transparent;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .suggest li.sel button,
  .suggest button:hover {
    background: var(--accent);
    color: var(--accent-fg);
  }
</style>
