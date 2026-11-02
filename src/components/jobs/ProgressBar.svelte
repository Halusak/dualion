<script lang="ts">
  let { pct, indeterminate = false, failed = false, done = false }: { pct: number; indeterminate?: boolean; failed?: boolean; done?: boolean } = $props();
</script>

<div class="bar" class:indeterminate class:failed class:done role="progressbar" aria-valuemin={0} aria-valuemax={100} aria-valuenow={Math.round(pct)}>
  <span style="width: {pct}%"></span>
</div>

<style>
  .bar {
    height: 6px;
    border-radius: 3px;
    background: var(--border);
    overflow: hidden;
    margin-top: 5px;
  }
  .bar span {
    display: block;
    height: 100%;
    border-radius: 3px;
    background: var(--accent);
    transition: width 0.2s linear;
  }
  .done span {
    background: var(--ok);
  }
  .failed span {
    background: var(--danger);
  }
  .indeterminate span {
    width: 30% !important;
    animation: slide 1.1s ease-in-out infinite;
  }
  @keyframes slide {
    from {
      transform: translateX(-100%);
    }
    to {
      transform: translateX(340%);
    }
  }
</style>
