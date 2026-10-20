<script lang="ts">
  import { onMount } from 'svelte';

  let { value, isDir, onDone }: { value: string; isDir: boolean; onDone: (v: string | null) => void } = $props();

  let input: HTMLInputElement;
  // svelte-ignore state_referenced_locally
  let text = $state(value);
  let done = false;

  onMount(() => {
    input.focus();
    // Select the name without the extension, like Finder and Explorer.
    const dot = isDir ? -1 : text.lastIndexOf('.');
    input.setSelectionRange(0, dot > 0 ? dot : text.length);
  });

  function finish(v: string | null) {
    if (done) return;
    done = true;
    onDone(v);
  }

  function onKey(e: KeyboardEvent) {
    e.stopPropagation();
    if (e.key === 'Enter') {
      e.preventDefault();
      finish(text);
    } else if (e.key === 'Escape') {
      e.preventDefault();
      finish(null);
    }
  }
</script>

<input
  bind:this={input}
  bind:value={text}
  class="rename"
  spellcheck="false"
  autocomplete="off"
  onkeydown={onKey}
  onblur={() => finish(text)}
  onpointerdown={(e) => e.stopPropagation()}
  ondblclick={(e) => e.stopPropagation()}
/>

<style>
  .rename {
    flex: 1;
    min-width: 0;
    height: calc(var(--row-h) - 2px);
    padding: 0 4px;
    margin-left: -4px;
    border: 1px solid var(--accent);
    border-radius: 3px;
    background: var(--bg-panel);
    color: var(--text);
    outline: none;
    box-shadow: var(--focus-ring);
    font: inherit;
  }
</style>
