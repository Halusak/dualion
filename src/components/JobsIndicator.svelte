<script lang="ts">
  import Layers from '@lucide/svelte/icons/layers';
  import { run } from '../lib/commands';
  import { t } from '../lib/i18n/index.svelte';
  import { isMac } from '../lib/platform';
  import { app } from '../lib/state/app.svelte';

  const active = $derived(app.activeJobs);
  const progress = $derived.by(() => {
    let total = 0;
    let done = 0;
    for (const j of active) {
      total += j.totalBytes || j.totalFiles;
      done += j.totalBytes ? j.doneBytes : j.doneFiles;
    }
    return total > 0 ? Math.min(1, done / total) : 0;
  });
  const waiting = $derived(active.some((j) => j.status === 'waiting'));
  const r = 9;
  const c = 2 * Math.PI * r;
</script>

<button
  class="icon-btn jobs"
  class:on={app.showJobs}
  class:busy={active.length > 0}
  class:waiting
  title={active.length ? t('jobs.running', { count: active.length }) : t('cmd.jobs')}
  onclick={() => run('jobs')}
>
  {#if active.length}
    <svg width="22" height="22" viewBox="0 0 22 22" aria-hidden="true">
      <circle cx="11" cy="11" {r} class="track" />
      <circle cx="11" cy="11" {r} class="bar" stroke-dasharray={c} stroke-dashoffset={c * (1 - progress)} />
    </svg>
    <span class="count">{active.length}</span>
  {:else}
    <Layers size={isMac ? 17 : 16} strokeWidth={isMac ? 1.75 : 1.5} />
  {/if}
</button>

<style>
  .jobs {
    position: relative;
  }
  svg {
    transform: rotate(-90deg);
  }
  circle {
    fill: none;
    stroke-width: 2.5;
  }
  .track {
    stroke: var(--border-strong);
  }
  .bar {
    stroke: var(--accent);
    stroke-linecap: round;
    transition: stroke-dashoffset 0.25s linear;
  }
  .waiting .bar {
    stroke: var(--warn);
  }
  .count {
    position: absolute;
    font-size: 9px;
    font-weight: 700;
    color: var(--text);
  }
</style>
