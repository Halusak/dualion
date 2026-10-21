<script lang="ts">
  import TriangleAlert from '@lucide/svelte/icons/triangle-alert';
  import { t } from '../../lib/i18n/index.svelte';
  import { errorText } from '../../lib/state/ui.svelte';
  import type { Answer, JobSnapshot, Prompt } from '../../lib/types';
  import Modal from './Modal.svelte';

  let { prompt, close }: { job: JobSnapshot; prompt: Extract<Prompt, { kind: 'error' }>; close: (a: Answer | null) => void } = $props();
</script>

<Modal title={t('jobError.title')} width={680} onclose={() => close({ action: 'cancel' })} onsubmit={() => close({ action: prompt.canRetry ? 'retry' : 'skip', all: false } as Answer)}>
  <div class="body">
    <span class="icon"><TriangleAlert size={32} strokeWidth={1.5} /></span>
    <p>{errorText(prompt.error)}</p>
  </div>
  {#snippet footer()}
    <button class="btn b-extra" onclick={() => close({ action: 'cancel' })}>{t('jobError.cancel')}</button>
    <button class="btn b-cancel" onclick={() => close({ action: 'skip', all: true })}>{t('jobError.skipAll')}</button>
    <button class="btn b-cancel" onclick={() => close({ action: 'skip', all: false })}>{t('jobError.skip')}</button>
    {#if prompt.canRetry}
      <button class="btn primary b-primary" onclick={() => close({ action: 'retry' })}>{t('jobError.retry')}</button>
    {/if}
  {/snippet}
</Modal>

<style>
  .body {
    display: flex;
    gap: 14px;
    align-items: flex-start;
  }
  .icon {
    color: var(--warn);
    flex: none;
  }
  p {
    margin: 4px 0 0;
    white-space: pre-line;
    word-break: break-word;
  }
</style>
