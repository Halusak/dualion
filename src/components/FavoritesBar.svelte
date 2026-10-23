<script lang="ts">
  import Folder from '@lucide/svelte/icons/folder';
  import Plus from '@lucide/svelte/icons/plus';
  import Search from '@lucide/svelte/icons/search';
  import Star from '@lucide/svelte/icons/star';
  import { run } from '../lib/commands';
  import { drag } from '../lib/dnd.svelte';
  import { t } from '../lib/i18n/index.svelte';
  import { locKey, locPath, sameLoc } from '../lib/paths';
  import { dpr, pathIconUrl } from '../lib/platform';
  import { app } from '../lib/state/app.svelte';
  import { settings, type HotlistItem } from '../lib/state/settings.svelte';

  let pointer: { x: number; y: number; location: HotlistItem['location']; moved: boolean } | null = null;
  let suppressClick = false;
  let showFavorites = $state(false);

  const iconPx = Math.round(18 * dpr());

  function open(item: HotlistItem) {
    if (suppressClick) return;
    void app.panel.navigate(item.location);
  }

  function moveFavorite(location: HotlistItem['location'], to: number) {
    const list = [...settings.hotlist];
    const from = list.findIndex((item) => sameLoc(item.location, location));
    if (from < 0 || from === to) return;
    const [item] = list.splice(from, 1);
    list.splice(to, 0, item);
    settings.hotlist = list;
  }

  function pointerDown(e: PointerEvent, item: HotlistItem) {
    if (e.button !== 0) return;
    pointer = { x: e.clientX, y: e.clientY, location: item.location, moved: false };
    window.addEventListener('pointermove', pointerMove);
    window.addEventListener('pointerup', pointerUp);
    window.addEventListener('pointercancel', pointerUp);
  }

  function pointerMove(e: PointerEvent) {
    if (!pointer) return;
    if (!pointer.moved && Math.hypot(e.clientX - pointer.x, e.clientY - pointer.y) > 6) {
      pointer.moved = true;
      suppressClick = true;
    }
    if (!pointer.moved) return;
    const el = document.elementFromPoint(e.clientX, e.clientY) as HTMLElement | null;
    const index = Number(el?.closest<HTMLElement>('[data-favorite-index]')?.dataset.favoriteIndex);
    if (Number.isInteger(index)) moveFavorite(pointer.location, index);
  }

  function pointerUp() {
    window.removeEventListener('pointermove', pointerMove);
    window.removeEventListener('pointerup', pointerUp);
    window.removeEventListener('pointercancel', pointerUp);
    pointer = null;
    setTimeout(() => (suppressClick = false), 0);
  }
</script>

<nav class="favorites" data-favorites-drop aria-label={t('cmd.hotlist')} class:drop={drag.active}>
  <button class="toggle icon-btn" class:on={showFavorites} title={t('cmd.hotlist')} aria-label={t('cmd.hotlist')} aria-pressed={showFavorites} onclick={() => (showFavorites = !showFavorites)}>
    <Star size={17} strokeWidth={1.7} />
  </button>
  {#if showFavorites}
    <div class="items">
      {#each settings.hotlist as item, i (locKey(item.location))}
        <button
          class="favorite icon-btn"
          data-favorite-index={i}
          title={`${item.name}\n${locPath(item.location)}`}
          aria-label={item.name}
          onclick={() => open(item)}
          onpointerdown={(e) => pointerDown(e, item)}
        >
          {#if item.location.kind === 'query' || item.location.kind === 'search'}
            <Search size={17} strokeWidth={1.7} />
          {:else if item.location.kind === 'local'}
            <img src={pathIconUrl(item.location.path, iconPx)} alt="" width="18" height="18" draggable="false" />
          {:else}
            <Folder size={17} strokeWidth={1.7} />
          {/if}
        </button>
      {/each}
      <button class="add icon-btn" title={t('cmd.addHotlist')} aria-label={t('cmd.addHotlist')} onclick={() => run('addHotlist')}>
        <Plus size={17} strokeWidth={1.7} />
      </button>
    </div>
  {/if}
</nav>

<style>
  .favorites {
    flex: 1;
    min-width: 0;
    display: flex;
    align-items: center;
    justify-content: center;
    gap: 3px;
    overflow: hidden;
    padding: 0 4px;
  }
  .items {
    min-width: 0;
    display: flex;
    align-items: center;
    gap: 3px;
    overflow-x: auto;
    overflow-y: hidden;
  }
  .favorite {
    flex: none;
    width: 28px;
    height: 28px;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    padding: 0;
    cursor: grab;
  }
  .favorite:active { cursor: grabbing; }
  .favorite img { flex: none; object-fit: contain; }
  .toggle, .add { flex: none; width: 28px; height: 28px; }
  .favorites.drop { background: var(--bg-hover); border-radius: var(--r-s); }
</style>
