<script lang="ts">
  import ArrowDown from '@lucide/svelte/icons/arrow-down';
  import ArrowUp from '@lucide/svelte/icons/arrow-up';
  import Trash2 from '@lucide/svelte/icons/trash-2';
  import { t } from '../../lib/i18n/index.svelte';
  import { locPath } from '../../lib/paths';
  import { app } from '../../lib/state/app.svelte';
  import { settings } from '../../lib/state/settings.svelte';
  import Modal from './Modal.svelte';

  let { close }: { close: (r: null) => void } = $props();

  function move(i: number, d: number) {
    const list = [...settings.hotlist];
    const j = i + d;
    if (j < 0 || j >= list.length) return;
    [list[i], list[j]] = [list[j], list[i]];
    settings.hotlist = list;
  }

  function remove(i: number) {
    settings.hotlist = settings.hotlist.filter((_, k) => k !== i);
  }

  function go(i: number) {
    void app.panel.navigate(settings.hotlist[i].location);
    close(null);
  }
</script>

<Modal title={t('cmd.hotlist')} width={560} onclose={() => close(null)}>
  {#if !settings.hotlist.length}
    <p class="muted">{t('place.noFavorites')}</p>
  {/if}
  <div class="list">
    {#each settings.hotlist as h, i (i)}
      <div class="item">
        <input class="input name" bind:value={settings.hotlist[i].name} />
        <button class="path ellipsis" title={locPath(h.location)} onclick={() => go(i)}>{locPath(h.location)}</button>
        <button class="icon-btn" title={t('jobs.up')} onclick={() => move(i, -1)}><ArrowUp size={14} /></button>
        <button class="icon-btn" title={t('jobs.down')} onclick={() => move(i, 1)}><ArrowDown size={14} /></button>
        <button class="icon-btn" title={t('remove')} onclick={() => remove(i)}><Trash2 size={14} /></button>
      </div>
    {/each}
  </div>
  {#snippet footer()}
    <button class="btn primary b-primary" onclick={() => close(null)}>{t('close')}</button>
  {/snippet}
</Modal>

<style>
  .list {
    display: grid;
    gap: 6px;
  }
  .item {
    display: grid;
    grid-template-columns: 150px 1fr auto auto auto;
    gap: 6px;
    align-items: center;
  }
  .path {
    border: none;
    background: transparent;
    text-align: left;
    color: var(--accent-text);
    min-width: 0;
  }
</style>
