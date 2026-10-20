<script lang="ts">
  import { command, functionKeys, keysOf, run } from '../lib/commands';
  import { t } from '../lib/i18n/index.svelte';
  import { shortcutLabel } from '../lib/platform';

  /** Prefer an F-key binding, else the first shortcut (keymap can change them). */
  function keyLabel(cmd: string): string {
    const keys = keysOf(command(cmd));
    const k = keys.find((x) => /^F\d{1,2}$/.test(x)) ?? keys[0];
    return k ? shortcutLabel(k) : '';
  }
</script>

<nav class="fbar">
  {#each functionKeys as f (f.key)}
    <button tabindex="-1" onclick={() => run(f.cmd)}>
      <span class="k">{keyLabel(f.cmd)}</span>
      <span class="ellipsis">{t(f.label)}</span>
    </button>
  {/each}
</nav>

<style>
  .fbar {
    display: grid;
    grid-template-columns: repeat(10, 1fr);
    gap: 4px;
    padding: 0 8px 6px;
    flex: none;
  }
  button {
    height: 24px;
    display: flex;
    align-items: center;
    justify-content: center;
    gap: 6px;
    min-width: 0;
    border: none;
    border-radius: var(--r-s);
    background: var(--bg-subtle);
    color: var(--text);
    font-size: 12px;
    padding: 0 6px;
  }
  :global(:root[data-effects='mica']) button,
  :global(:root[data-effects='vibrancy']) button {
    background: rgba(127, 127, 127, 0.12);
  }
  button:hover {
    background: var(--bg-hover);
  }
  button:active {
    background: var(--bg-pressed);
  }
  .k {
    font-weight: 700;
    color: var(--accent-text);
    font-size: 11px;
  }
</style>
