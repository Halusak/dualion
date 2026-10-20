<script lang="ts">
  // Total Commander style drive buttons above each panel.
  import Globe from '@lucide/svelte/icons/globe';
  import Plug from '@lucide/svelte/icons/plug';
  import { LogicalPosition } from '@tauri-apps/api/dpi';
  import { Menu, MenuItem, PredefinedMenuItem } from '@tauri-apps/api/menu';
  import { onMount } from 'svelte';
  import { api } from '../lib/api';
  import { run } from '../lib/commands';
  import { formatSize } from '../lib/format';
  import { localeTag, t } from '../lib/i18n/index.svelte';
  import { dpr, isWindows, pathIconUrl } from '../lib/platform';
  import { app, type SideName } from '../lib/state/app.svelte';
  import { settings } from '../lib/state/settings.svelte';
  import { Tab } from '../lib/state/tab.svelte';
  import { showError } from '../lib/state/ui.svelte';
  import { volumeOf, volumes, watchVolumes } from '../lib/state/volumes.svelte';
  import type { Volume } from '../lib/types';

  let { side }: { side: SideName } = $props();
  const tab = $derived(app.side(side).tab);

  onMount(() => watchVolumes());

  // Last folder visited on each drive, per panel (like TC).
  const lastOnDrive = new Map<string, string>();
  const current = $derived(tab.location.kind === 'local' ? volumeOf(tab.location.path) : undefined);
  $effect(() => {
    if (tab.location.kind === 'local' && current) lastOnDrive.set(current.path, tab.location.path);
  });

  function label(v: Volume): string {
    if (isWindows) return v.path.slice(0, 1).toLowerCase();
    return v.name;
  }

  function title(v: Volume): string {
    const l = localeTag();
    return v.total ? `${v.name} — ${formatSize(v.free, l, settings.sizeFormat)} / ${formatSize(v.total, l, settings.sizeFormat)}` : v.name;
  }

  function go(v: Volume, e: MouseEvent, newTab = false) {
    app.activeSide = side;
    if (newTab) {
      // Middle click: open the drive in a new tab (like links in a browser).
      const s = app.side(side);
      const t = new Tab({ location: { kind: 'local', path: v.path }, view: tab.view, sortKey: tab.sortKey, sortDesc: tab.sortDesc });
      s.addTab(t);
      void t.navigate({ kind: 'local', path: lastOnDrive.get(v.path) ?? v.path }).catch(() => t.navigate({ kind: 'local', path: v.path }));
      return;
    }
    // Clicking the current drive goes to its root; otherwise return to the last folder there.
    const last = current?.path === v.path || e.shiftKey ? undefined : lastOnDrive.get(v.path);
    void tab.navigate({ kind: 'local', path: last ?? v.path }).catch(() => tab.navigate({ kind: 'local', path: v.path }));
  }

  async function network(e: MouseEvent) {
    app.activeSide = side;
    const el = e.currentTarget as HTMLElement;
    const r = el.getBoundingClientRect();
    const items: (MenuItem | PredefinedMenuItem)[] = [];
    const sites = await api.sitesList().catch(() => []);
    for (const s of sites) {
      items.push(
        await MenuItem.new({
          text: s.name || s.host,
          action: async () => {
            try {
              const c = await api.ftpConnect(s.id, null);
              await tab.navigate({ kind: 'ftp', session: c.session, path: c.path || '/' });
            } catch (err) {
              showError(err);
            }
          },
        }),
      );
    }
    const net = volumes.list.filter((v) => v.kind === 'network');
    if (items.length && net.length) items.push(await PredefinedMenuItem.new({ item: 'Separator' }));
    for (const v of net) items.push(await MenuItem.new({ text: v.name, action: () => tab.navigate({ kind: 'local', path: v.path }) }));
    if (items.length) items.push(await PredefinedMenuItem.new({ item: 'Separator' }));
    items.push(await MenuItem.new({ text: t('cmd.ftpConnect'), action: () => run('ftpConnect') }));
    const menu = await Menu.new({ items });
    await menu.popup(new LogicalPosition(r.left, r.bottom));
  }

  const px = $derived(Math.round(16 * dpr()));
</script>

<div class="drives" role="toolbar" aria-label={t(isWindows ? 'place.drivesWin' : 'place.drivesMac')}>
  {#each volumes.list as v (v.path)}
    <button class:on={current?.path === v.path} class:win={isWindows} title={title(v)} tabindex="-1"
      onclick={(e) => go(v, e)}
      onauxclick={(e) => e.button === 1 && go(v, e, true)}
      onmousedown={(e) => e.button === 1 && e.preventDefault()}>
      <img src={pathIconUrl(v.path, px)} alt="" width="16" height="16" draggable="false" />
      <span class="ellipsis">{label(v)}</span>
    </button>
  {/each}
  <span class="sep"></span>
  <button title={t('drives.network')} tabindex="-1" onclick={network}>
    {#if tab.location.kind === 'ftp'}<Plug size={15} />{:else}<Globe size={15} />{/if}
    <span>{t('drives.network')}</span>
  </button>
</div>

<style>
  .drives {
    flex: none;
    display: flex;
    align-items: stretch;
    height: 28px;
    padding: 0;
    overflow-x: auto;
    scrollbar-width: none;
    background: var(--bg-subtle);
    border-bottom: 1px solid var(--border);
  }
  .drives::-webkit-scrollbar {
    display: none;
  }
  /* Same look as the tab bar: flush segments separated by thin lines. */
  button {
    flex: none;
    display: inline-flex;
    align-items: center;
    gap: 5px;
    max-width: 160px;
    padding: 0 10px;
    border: none;
    border-right: 1px solid var(--border);
    border-radius: 0;
    background: transparent;
    color: var(--text-2);
    font-size: 12px;
  }
  button.win span {
    font-weight: 600;
    text-transform: lowercase;
  }
  button:hover {
    background: var(--bg-hover);
    color: var(--text);
  }
  button.on {
    background: var(--bg-panel);
    color: var(--text);
    font-weight: 600;
  }
  .sep {
    flex: 1;
  }
  img {
    flex: none;
  }
</style>
