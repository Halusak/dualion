<script lang="ts">
  import ArrowDown from '@lucide/svelte/icons/arrow-down';
  import ArrowUp from '@lucide/svelte/icons/arrow-up';
  import CircleAlert from '@lucide/svelte/icons/circle-alert';
  import Check from '@lucide/svelte/icons/check';
  import Pause from '@lucide/svelte/icons/pause';
  import Play from '@lucide/svelte/icons/play';
  import X from '@lucide/svelte/icons/x';
  import { api } from '../lib/api';
  import { formatDuration, formatSize, formatSpeed } from '../lib/format';
  import { localeTag, t } from '../lib/i18n/index.svelte';
  import { app } from '../lib/state/app.svelte';
  import { settings } from '../lib/state/settings.svelte';
  import { errorText } from '../lib/state/ui.svelte';
  import type { JobSnapshot } from '../lib/types';

  const locale = $derived(localeTag());
  const jobs = $derived([...app.jobs].reverse());

  function title(j: JobSnapshot): string {
    const op = j.op === 'delete' && j.toTrash ? t('jobs.op.trash') : t(`jobs.op.${j.op}`);
    const what = j.sourceCount === 1 ? t('what.one', { name: j.firstSource }) : t('what.items', { count: j.sourceCount });
    return `${op} ${what}`;
  }

  function pct(j: JobSnapshot): number {
    if (j.status === 'done') return 100;
    if (j.totalBytes > 0) return Math.min(100, (100 * j.doneBytes) / j.totalBytes);
    if (j.totalFiles > 0) return Math.min(100, (100 * j.doneFiles) / j.totalFiles);
    return 0;
  }

  const finished = (j: JobSnapshot) => j.status === 'done' || j.status === 'failed' || j.status === 'cancelled';
</script>

<!-- svelte-ignore a11y_no_static_element_interactions -->
<!-- svelte-ignore a11y_click_events_have_key_events -->
<div class="backdrop" onclick={() => (app.showJobs = false)}></div>
<aside class="jobs" aria-label={t('jobs.title')}>
  <header>
    <strong>{t('jobs.title')}</strong>
    <button class="btn small" disabled={!app.jobs.some(finished)} onclick={() => api.jobsClearFinished()}>{t('jobs.clear')}</button>
  </header>
  <div class="list">
    {#if jobs.length === 0}
      <div class="empty muted">{t('jobs.empty')}</div>
    {/if}
    {#each jobs as j (j.id)}
      <div class="job" class:done={j.status === 'done'} class:failed={j.status === 'failed'}>
        <div class="top">
          <div class="title ellipsis" title={title(j)}>{title(j)}</div>
          <div class="actions">
            {#if j.status === 'queued'}
              <button class="icon-btn" title={t('jobs.up')} onclick={() => api.jobMove(j.id, -1)}><ArrowUp size={14} /></button>
              <button class="icon-btn" title={t('jobs.down')} onclick={() => api.jobMove(j.id, 1)}><ArrowDown size={14} /></button>
              <button class="icon-btn" title={t('jobs.startNow')} onclick={() => api.jobStartNow(j.id)}><Play size={14} /></button>
            {:else if j.status === 'running' || j.status === 'scanning'}
              <button class="icon-btn" title={t('jobs.pause')} onclick={() => api.jobPause(j.id, true)}><Pause size={14} /></button>
            {:else if j.status === 'paused'}
              <button class="icon-btn" title={t('jobs.resume')} onclick={() => api.jobPause(j.id, false)}><Play size={14} /></button>
            {/if}
            {#if !finished(j)}
              <button class="icon-btn" title={t('jobs.cancel')} onclick={() => api.jobCancel(j.id)}><X size={14} /></button>
            {:else if j.status === 'done'}
              <span class="ok"><Check size={14} /></span>
            {:else if j.status === 'failed'}
              <span class="bad"><CircleAlert size={14} /></span>
            {/if}
          </div>
        </div>
        {#if j.dest && j.op !== 'delete'}
          <div class="dest muted ellipsis" title={j.dest}>{t('jobs.to', { dest: j.dest })}</div>
        {/if}
        <div class="bar" class:indeterminate={j.status === 'scanning'}><span style="width: {pct(j)}%"></span></div>
        <div class="meta muted">
          <span>
            {t(`jobs.st.${j.status}`)}
            {#if j.queue !== null && j.status === 'queued'}· {t('jobs.queue', { n: j.queue })}{/if}
          </span>
          {#if j.totalBytes > 0}
            <span>{formatSize(j.doneBytes, locale, settings.sizeFormat)} / {formatSize(j.totalBytes, locale, settings.sizeFormat)}</span>
          {:else if j.totalFiles > 0}
            <span>{t('jobs.files', { done: j.doneFiles, total: j.totalFiles })}</span>
          {/if}
          {#if j.status === 'running' && j.speed > 0}
            <span>{formatSpeed(j.speed, locale, settings.sizeFormat)}</span>
          {/if}
          {#if j.status === 'running' && j.eta !== null}
            <span>{t('jobs.eta', { time: formatDuration(j.eta, locale) })}</span>
          {/if}
          {#if j.skipped}<span>{t('jobs.skipped', { count: j.skipped })}</span>{/if}
          {#if j.errors}<span class="bad">{t('jobs.errors', { count: j.errors })}</span>{/if}
        </div>
        {#if j.current && !finished(j)}
          <div class="current muted ellipsis" title={j.current}>{j.current}</div>
        {/if}
        {#if j.error}
          <div class="err">{errorText(j.error)}</div>
        {/if}
      </div>
    {/each}
  </div>
</aside>

<style>
  .backdrop {
    position: fixed;
    inset: 0;
    z-index: 40;
  }
  .jobs {
    position: fixed;
    top: calc(var(--toolbar-h) + 4px);
    right: 10px;
    width: 420px;
    max-height: min(70vh, 640px);
    z-index: 41;
    display: flex;
    flex-direction: column;
    border-radius: var(--r-l);
    background: var(--bg-elevated);
    box-shadow: var(--shadow-pop);
    overflow: hidden;
  }
  header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 10px 12px;
    border-bottom: 1px solid var(--border);
  }
  .list {
    overflow: auto;
    padding: 6px;
  }
  .empty {
    padding: 24px;
    text-align: center;
  }
  .job {
    padding: 8px 8px 10px;
    border-radius: var(--r-m);
  }
  .job + .job {
    border-top: 1px solid var(--border);
  }
  .top {
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .title {
    flex: 1;
    font-weight: 600;
  }
  .actions {
    display: flex;
    gap: 2px;
  }
  .actions .icon-btn {
    width: 24px;
    height: 24px;
  }
  .dest,
  .current {
    font-size: 11px;
    margin-top: 2px;
  }
  .bar {
    height: 5px;
    border-radius: 3px;
    background: var(--border);
    overflow: hidden;
    margin: 7px 0 5px;
  }
  .bar span {
    display: block;
    height: 100%;
    background: var(--accent);
    transition: width 0.2s linear;
  }
  .done .bar span {
    background: var(--ok);
  }
  .failed .bar span {
    background: var(--danger);
  }
  .bar.indeterminate span {
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
  .meta {
    display: flex;
    flex-wrap: wrap;
    gap: 4px 12px;
    font-size: 11px;
    font-variant-numeric: tabular-nums;
  }
  .ok {
    color: var(--ok);
    display: inline-flex;
    padding: 4px;
  }
  .bad {
    color: var(--danger);
    display: inline-flex;
  }
  .err {
    margin-top: 4px;
    font-size: 11px;
    color: var(--danger);
    white-space: pre-line;
  }
</style>
