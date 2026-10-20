<script lang="ts">
  import Funnel from '@lucide/svelte/icons/funnel';
  import X from '@lucide/svelte/icons/x';
  import { onMount } from 'svelte';
  import { t } from '../lib/i18n/index.svelte';
  import type { Tab } from '../lib/state/tab.svelte';

  let { tab }: { tab: Tab } = $props();
  let input: HTMLInputElement;

  onMount(() => {
    if (tab.filterOpen) input.focus();
  });

  function close() {
    tab.filter = '';
    tab.filterOpen = false;
  }

  function onKey(e: KeyboardEvent) {
    if (e.key === 'Escape') {
      e.preventDefault();
      e.stopPropagation();
      close();
    } else if (e.key === 'Enter' || e.key === 'ArrowDown') {
      // Hand the keyboard back to the list, keeping the filter applied.
      e.preventDefault();
      e.stopPropagation();
      tab.filterOpen = false;
      (input.closest('[data-side]') as HTMLElement | null)?.focus();
    }
  }
</script>

<div class="filter">
  <Funnel size={13} strokeWidth={1.75} />
  <input bind:this={input} bind:value={tab.filter} placeholder={t('panel.filterHint')} spellcheck="false" onkeydown={onKey} />
  <button class="x" aria-label={t('close')} onclick={close}><X size={12} /></button>
</div>

<style>
  .filter {
    display: flex;
    align-items: center;
    gap: 6px;
    height: 30px;
    padding: 0 8px;
    flex: none;
    border-top: 1px solid var(--border);
    background: var(--bg-header);
    color: var(--text-2);
  }
  input {
    flex: 1;
    min-width: 0;
    border: none;
    outline: none;
    background: transparent;
    color: var(--text);
  }
  .x {
    border: none;
    background: transparent;
    color: var(--text-2);
    display: inline-flex;
    padding: 2px;
    border-radius: 4px;
  }
  .x:hover {
    background: var(--bg-hover);
  }
</style>
