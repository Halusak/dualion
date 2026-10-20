<script lang="ts">
  import { commands, fullLabel, keysOf } from '../../lib/commands';
  import { t } from '../../lib/i18n/index.svelte';
  import { shortcutLabel } from '../../lib/platform';
  import Modal from './Modal.svelte';

  let { close }: { close: (r: null) => void } = $props();
  let q = $state('');

  const nav = [
    ['↑ ↓ PgUp PgDn Home End', 'help.nav.move'],
    ['Shift + ↑ ↓', 'help.nav.range'],
    ['Insert', 'help.nav.insert'],
    ['Space', 'help.nav.space'],
    ['Enter', 'help.nav.enter'],
    ['a–z', 'help.nav.type'],
    ['Esc', 'help.nav.esc'],
  ] as const;

  const rows = $derived(
    commands
      .filter((c) => keysOf(c).length)
      .map((c) => ({ label: fullLabel(c), keys: keysOf(c).map(shortcutLabel).join('  ·  ') }))
      .filter((r) => !q || (r.label + r.keys).toLowerCase().includes(q.toLowerCase())),
  );
</script>

<Modal title={t('help.title')} width={720} onclose={() => close(null)}>
  <input class="input search" bind:value={q} placeholder={t('help.filter')} data-autofocus />
  <div class="cols">
    <div class="table">
      {#each rows as r (r.label)}
        <div class="r"><span>{r.label}</span><kbd>{r.keys}</kbd></div>
      {/each}
    </div>
    {#if !q}
      <div class="nav">
        {#each nav as [k, label] (k)}
          <div class="r"><kbd>{k}</kbd><span>{t(label)}</span></div>
        {/each}
        <p class="muted tip">{t('help.tip')}</p>
      </div>
    {/if}
  </div>
  {#snippet footer()}
    <button class="btn primary b-primary" onclick={() => close(null)}>{t('close')}</button>
  {/snippet}
</Modal>

<style>
  .search {
    width: 100%;
    margin-bottom: 10px;
  }
  .cols {
    display: grid;
    grid-template-columns: 1fr 240px;
    gap: 16px;
    max-height: min(520px, calc(100vh - 260px));
  }
  .table {
    overflow: auto;
  }
  .r {
    display: flex;
    justify-content: space-between;
    gap: 12px;
    padding: 4px 6px;
    border-radius: var(--r-s);
    font-size: 12px;
  }
  .r:nth-child(odd) {
    background: var(--bg-subtle);
  }
  kbd {
    font-family: var(--font-ui);
    color: var(--accent-text);
    font-weight: 600;
    white-space: nowrap;
  }
  .nav .r {
    justify-content: flex-start;
  }
  .tip {
    font-size: 11px;
    line-height: 1.5;
  }
</style>
