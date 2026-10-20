<script lang="ts">
  import Undo2 from '@lucide/svelte/icons/undo-2';
  import { formatDate } from '../../lib/format';
  import { localeTag, t } from '../../lib/i18n/index.svelte';
  import { history, undo } from '../../lib/state/history.svelte';
  import Modal from './Modal.svelte';

  let { close }: { close: (r: null) => void } = $props();
  const locale = localeTag();

  function detail(e: (typeof history.entries)[number]): string {
    if (e.rename) return e.rename.pairs.map((p) => `${p.to} → ${p.from}`).join('\n');
    return (e.actions ?? []).map((a) => (a.type === 'moved' ? `${a.from} → ${a.to}` : a.path)).join('\n');
  }
</script>

<Modal title={t('history.title')} width={620} onclose={() => close(null)}>
  <p class="muted hint">{t('history.hint')}</p>
  <div class="list">
    {#each history.entries as e (e.id)}
      <div class="row" class:undone={e.undone}>
        <div class="main" title={detail(e)}>
          <div class="ellipsis">{e.label}</div>
          <div class="muted small">{formatDate(e.time, locale, true)}{e.undone ? ` · ${t('history.wasUndone')}` : ''}</div>
        </div>
        <button class="btn" disabled={e.undone} onclick={() => undo(e.id)}><Undo2 size={14} /> {t('history.undo')}</button>
      </div>
    {:else}
      <div class="empty muted">{t('history.empty')}</div>
    {/each}
  </div>
  {#snippet footer()}
    <button class="btn primary" onclick={() => close(null)}>{t('close')}</button>
  {/snippet}
</Modal>

<style>
  .hint {
    margin: 0 0 10px;
    font-size: 12px;
  }
  .list {
    max-height: 420px;
    overflow-y: auto;
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  .row {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 6px 8px;
    border-radius: var(--r-m);
  }
  .row:hover {
    background: var(--bg-hover);
  }
  .row.undone .main {
    opacity: 0.5;
    text-decoration: line-through;
  }
  .main {
    flex: 1;
    min-width: 0;
  }
  .small {
    font-size: 11px;
  }
  .btn {
    display: inline-flex;
    align-items: center;
    gap: 5px;
  }
  .empty {
    padding: 30px;
    text-align: center;
  }
</style>
