<script lang="ts" module>
  export interface PromptResult {
    value: string;
    checked?: boolean;
  }
</script>

<script lang="ts">
  import { onMount } from 'svelte';
  import { t } from '../../lib/i18n/index.svelte';
  import Modal from './Modal.svelte';

  let {
    title,
    label,
    value = '',
    selectStem = false,
    checkbox,
    password = false,
    hint = '',
    close,
  }: {
    title: string;
    label: string;
    value?: string;
    selectStem?: boolean;
    checkbox?: string;
    /** Hidden input (passwords). */
    password?: boolean;
    /** Extra line under the field (e.g. "wrong password"). */
    hint?: string;
    close: (r: PromptResult | null) => void;
  } = $props();

  // svelte-ignore state_referenced_locally
  let text = $state(value);
  let checked = $state(false);
  let input: HTMLInputElement;

  onMount(() => {
    input.focus();
    const dot = text.lastIndexOf('.');
    input.setSelectionRange(0, selectStem && dot > 0 ? dot : text.length);
  });

  const submit = () => close({ value: text, checked });
</script>

<Modal {title} width={440} onclose={() => close(null)} onsubmit={submit}>
  <label class="field">
    <span>{label}</span>
    <input class="input" bind:this={input} bind:value={text} spellcheck="false" autocomplete="off" type={password ? 'password' : 'text'} />
  </label>
  {#if hint}<p class="hint">{hint}</p>{/if}
  {#if checkbox}
    <label class="check"><input type="checkbox" bind:checked /> {checkbox}</label>
  {/if}
  {#snippet footer()}
    <button class="btn b-cancel" onclick={() => close(null)}>{t('cancel')}</button>
    <button class="btn primary b-primary" onclick={submit}>{t('ok')}</button>
  {/snippet}
</Modal>

<style>
  .field {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  .field input {
    width: 100%;
  }
  .check {
    margin-top: 10px;
  }
  .hint {
    margin: 6px 0 0;
    font-size: 12px;
    color: var(--danger);
  }
</style>
