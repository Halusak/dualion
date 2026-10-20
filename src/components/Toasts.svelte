<script lang="ts">
  import { ui } from '../lib/state/ui.svelte';
</script>

<div class="toasts" aria-live="polite">
  {#each ui.toasts as toast (toast.id)}
    <div class="toast" class:error={toast.kind === 'error'}>{toast.text}</div>
  {/each}
</div>

<style>
  .toasts {
    position: fixed;
    left: 50%;
    bottom: 48px;
    transform: translateX(-50%);
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 6px;
    z-index: 100;
    pointer-events: none;
  }
  .toast {
    max-width: 560px;
    padding: 8px 14px;
    border-radius: var(--r-m);
    background: var(--bg-elevated);
    color: var(--text);
    box-shadow: var(--shadow-pop);
    white-space: pre-line;
    text-align: center;
    animation: in 0.18s ease-out;
  }
  .toast.error {
    border-left: 3px solid var(--danger);
  }
  @keyframes in {
    from {
      opacity: 0;
      transform: translateY(6px);
    }
  }
</style>
