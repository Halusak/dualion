<script lang="ts">
  import { drag } from '../lib/dnd.svelte';
  import { t, what } from '../lib/i18n/index.svelte';
</script>

{#if drag.active}
  <div class="ghost" style="left: {drag.x + 14}px; top: {drag.y + 12}px">
    <strong>{what(drag.names)}</strong>
    {#if drag.over}
      <span>{drag.move ? t('panel.dropMove') : t('panel.dropCopy')}</span>
    {/if}
  </div>
{/if}

<style>
  .ghost {
    position: fixed;
    z-index: 200;
    pointer-events: none;
    display: flex;
    flex-direction: column;
    gap: 2px;
    padding: 6px 10px;
    border-radius: var(--r-m);
    background: var(--bg-elevated);
    box-shadow: var(--shadow-pop);
    font-size: 12px;
    max-width: 280px;
  }
  span {
    color: var(--accent-text);
  }
</style>
