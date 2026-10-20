<script lang="ts">
  import { onMount } from 'svelte';
  import { t, what } from '../../lib/i18n/index.svelte';
  import Modal from './Modal.svelte';

  let { names, path, close }: { names: string[]; path: string; close: (r: { path: string; level: number; queue: number | null } | null) => void } = $props();

  // svelte-ignore state_referenced_locally
  let target = $state(path);
  let level = $state(6);
  let input: HTMLInputElement;

  onMount(() => {
    input.focus();
    const slash = Math.max(target.lastIndexOf('/'), target.lastIndexOf('\\'));
    const dot = target.lastIndexOf('.');
    input.setSelectionRange(slash + 1, dot > slash ? dot : target.length);
  });

  function submit(queue: number | null) {
    let p = target.trim();
    if (!p) return;
    if (!/\.zip$/i.test(p)) p += '.zip';
    close({ path: p, level, queue });
  }
</script>

<Modal title={t('pack.title')} width={540} onclose={() => close(null)} onsubmit={() => submit(null)}>
  <p class="muted what">{what(names)}</p>
  <label class="field">
    <span>{t('pack.archive')}</span>
    <input class="input" bind:this={input} bind:value={target} spellcheck="false" />
  </label>
  <label class="field">
    <span>{t('pack.level')}</span>
    <select class="input" bind:value={level}>
      <option value={0}>{t('level.store')}</option>
      <option value={1}>{t('level.fast')}</option>
      <option value={6}>{t('level.normal')}</option>
      <option value={9}>{t('level.best')}</option>
    </select>
  </label>
  {#snippet footer()}
    <button class="btn b-extra" onclick={() => submit(1)}>{t('copy.queue')}</button>
    <button class="btn b-cancel" onclick={() => close(null)}>{t('cancel')}</button>
    <button class="btn primary b-primary" onclick={() => submit(null)}>{t('cmd.pack').replace('…', '')}</button>
  {/snippet}
</Modal>

<style>
  .what {
    margin: 0 0 10px;
  }
  .field {
    display: flex;
    flex-direction: column;
    gap: 5px;
    margin-top: 8px;
  }
</style>
