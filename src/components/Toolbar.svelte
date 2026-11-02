<script lang="ts">
  import ArrowUp from '@lucide/svelte/icons/arrow-up';
  import ChevronLeft from '@lucide/svelte/icons/chevron-left';
  import ChevronRight from '@lucide/svelte/icons/chevron-right';
  import Eye from '@lucide/svelte/icons/eye';
  import EyeOff from '@lucide/svelte/icons/eye-off';
  import GitCompare from '@lucide/svelte/icons/git-compare';
  import Image from '@lucide/svelte/icons/image';
  import LayoutGrid from '@lucide/svelte/icons/layout-grid';
  import List from '@lucide/svelte/icons/list';
  import Archive from '@lucide/svelte/icons/archive';
  import PackageOpen from '@lucide/svelte/icons/package-open';
  import PanelRight from '@lucide/svelte/icons/panel-right';
  import PencilLine from '@lucide/svelte/icons/pencil-line';
  import RotateCw from '@lucide/svelte/icons/rotate-cw';
  import Rows3 from '@lucide/svelte/icons/rows-3';
  import Search from '@lucide/svelte/icons/search';
  import Server from '@lucide/svelte/icons/server';
  import Settings from '@lucide/svelte/icons/settings';
  import SquareTerminal from '@lucide/svelte/icons/square-terminal';
  import FavoritesBar from './FavoritesBar.svelte';
  import { run, shortcutOf } from '../lib/commands';
  import { t } from '../lib/i18n/index.svelte';
  import { isMac, shortcutLabel } from '../lib/platform';
  import { app } from '../lib/state/app.svelte';
  import { settings } from '../lib/state/settings.svelte';
  import type { ViewMode } from '../lib/types';
  import JobsIndicator from './JobsIndicator.svelte';

  const sw = isMac ? 1.75 : 1.5;
  const size = isMac ? 17 : 16;

  function tip(label: string, cmd: string) {
    const k = shortcutOf(cmd);
    return k ? `${t(label)} (${shortcutLabel(k)})` : t(label);
  }

  const tab = $derived(app.panel);
  const views: { mode: ViewMode; cmd: string; label: string; icon: typeof List }[] = [
    { mode: 'details', cmd: 'viewDetails', label: 'cmd.viewDetails', icon: Rows3 },
    { mode: 'list', cmd: 'viewList', label: 'cmd.viewList', icon: List },
    { mode: 'icons', cmd: 'viewIcons', label: 'cmd.viewIcons', icon: LayoutGrid },
    { mode: 'thumbs', cmd: 'viewThumbs', label: 'cmd.viewThumbs', icon: Image },
  ];
</script>

<header class="toolbar" class:mac={isMac} data-tauri-drag-region>
  <div class="lead" data-tauri-drag-region></div>
  <div class="actions" data-tauri-drag-region>
  <div class="group">
    <button class="icon-btn" title={tip('cmd.back', 'back')} disabled={!tab.canGoBack} onclick={() => run('back')}>
      <ChevronLeft {size} strokeWidth={sw} />
    </button>
    <button class="icon-btn" title={tip('cmd.forward', 'forward')} disabled={!tab.canGoForward} onclick={() => run('forward')}>
      <ChevronRight {size} strokeWidth={sw} />
    </button>
    <button class="icon-btn" title={tip('cmd.parent', 'parent')} disabled={!tab.listing?.parent} onclick={() => run('parent')}>
      <ArrowUp {size} strokeWidth={sw} />
    </button>
    <button class="icon-btn" title={tip('cmd.refresh', 'refresh')} onclick={() => run('refresh')}>
      <RotateCw size={size - 1} strokeWidth={sw} />
    </button>
  </div>

  <div class="segmented" role="radiogroup">
    {#each views as v (v.mode)}
      {@const Icon = v.icon}
      <button
        class:on={tab.view === v.mode}
        role="radio"
        aria-checked={tab.view === v.mode}
        title={tip(v.label, v.cmd)}
        onclick={() => run(v.cmd)}
      >
        <Icon size={size - 1} strokeWidth={sw} />
      </button>
    {/each}
  </div>

  <div class="group">
    <button class="icon-btn" class:on={settings.showHidden} title={tip('cmd.toggleHidden', 'toggleHidden')} onclick={() => run('toggleHidden')}>
      {#if settings.showHidden}<Eye {size} strokeWidth={sw} />{:else}<EyeOff {size} strokeWidth={sw} />{/if}
    </button>
    <button class="icon-btn" class:on={app.quickView} title={tip('cmd.quickView', 'quickView')} onclick={() => run('quickView')}>
      <PanelRight {size} strokeWidth={sw} />
    </button>
  </div>

  <div class="group">
    <button class="icon-btn" title={tip('cmd.search', 'search')} onclick={() => run('search')}>
      <Search {size} strokeWidth={sw} />
    </button>
    <button class="icon-btn" title={tip('cmd.multiRename', 'multiRename')} onclick={() => run('multiRename')}>
      <PencilLine {size} strokeWidth={sw} />
    </button>
    <button class="icon-btn" title={tip('cmd.compareDirs', 'compareDirs')} onclick={() => run('compareDirs')}>
      <GitCompare {size} strokeWidth={sw} />
    </button>
    <button class="icon-btn" title={tip('cmd.pack', 'pack')} onclick={() => run('pack')}>
      <Archive {size} strokeWidth={sw} />
    </button>
    <button class="icon-btn" title={tip('cmd.unpack', 'unpack')} onclick={() => run('unpack')}>
      <PackageOpen {size} strokeWidth={sw} />
    </button>
    <button class="icon-btn" title={tip('cmd.terminal', 'terminal')} onclick={() => run('terminal')}>
      <SquareTerminal {size} strokeWidth={sw} />
    </button>
    <button class="icon-btn" title={tip('cmd.ftpConnect', 'ftpConnect')} onclick={() => run('ftpConnect')}>
      <Server {size} strokeWidth={sw} />
    </button>
  </div>
  <span class="divider" aria-hidden="true"></span>
  <FavoritesBar />
  </div>

  <div class="trail" data-tauri-drag-region>
    <JobsIndicator />
    <button class="icon-btn" title={tip('cmd.settings', 'settings')} onclick={() => run('settings')}>
      <Settings {size} strokeWidth={sw} />
    </button>
  </div>
</header>

<style>
  .toolbar {
    height: var(--toolbar-h);
    flex: none;
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 0 10px 0 var(--chrome-left);
    background: var(--bg-chrome);
  }
  .actions {
    display: flex;
    align-items: center;
    gap: 10px;
    min-width: 0;
    flex: 1;
  }
  .lead {
    display: none;
  }
  .trail {
    margin-left: auto;
    display: flex;
    align-items: center;
    gap: 2px;
    align-self: stretch;
  }
  /* macOS: actions centred in the unified title bar, like Finder/Safari. */
  .toolbar.mac {
    display: flex;
    padding: 2px 10px 0 0;
  }
  .toolbar.mac .lead {
    display: block;
    align-self: stretch;
    min-width: var(--chrome-left);
    flex: none;
  }
  .toolbar.mac .trail {
    margin-left: 0;
  }
  .group {
    display: flex;
    align-items: center;
    gap: 2px;
  }
  .divider {
    width: 1px;
    height: 18px;
    flex: none;
    background: var(--border-strong);
  }
  .segmented {
    display: flex;
    align-items: center;
    padding: 2px;
    gap: 2px;
    border-radius: calc(var(--r-s) + 2px);
    background: var(--bg-subtle);
  }
  .segmented button {
    width: 30px;
    height: 24px;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    border: none;
    border-radius: var(--r-s);
    background: transparent;
    color: var(--text-2);
    padding: 0;
  }
  .segmented button:hover {
    color: var(--text);
  }
  .segmented button.on {
    background: var(--bg-panel);
    color: var(--text);
    box-shadow: 0 0.5px 2px rgba(0, 0, 0, 0.18);
  }
  :global(:root[data-theme='dark']) .segmented button.on {
    background: rgba(255, 255, 255, 0.16);
  }
</style>
