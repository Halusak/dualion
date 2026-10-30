<script lang="ts">
  import { onMount } from 'svelte';
  import { api } from '../../lib/api';
  import { formatDuration, formatSize, formatSpeed } from '../../lib/format';
  import { localeTag, t } from '../../lib/i18n/index.svelte';
  import { isFinished, jobElapsed, jobPct, jobStatus, jobTitle } from '../../lib/jobs';
  import { baseName } from '../../lib/paths';
  import { app } from '../../lib/state/app.svelte';
  import { settings } from '../../lib/state/settings.svelte';
  import { errorText, toast } from '../../lib/state/ui.svelte';
  import Modal from '../dialogs/Modal.svelte';
  import ProgressBar from './ProgressBar.svelte';

  let { jobId, close }: { jobId: number; close: (r: null) => void } = $props();

  const locale = $derived(localeTag());
  const job = $derived(app.jobs.find((j) => j.id === jobId) ?? null);
  const size = (n: number) => formatSize(n, locale, settings.sizeFormat);

  let now = $state(Date.now());
  onMount(() => {
    const timer = setInterval(() => (now = Date.now()), 1000);
    return () => clearInterval(timer);
  });

  // Done or cancelled: the window has nothing more to say. A failure stays
  // on screen so the error can be read.
  $effect(() => {
    if (job && (job.status === 'done' || job.status === 'cancelled')) close(null);
  });

  const running = $derived(!!job && !isFinished(job));
  const paused = $derived(job?.status === 'paused' && !job.held);
  const filePct = $derived(job && job.currentSize > 0 ? Math.min(100, (100 * job.currentDone) / job.currentSize) : 0);
  const elapsed = $derived(job ? jobElapsed(job, now) : null);

  async function toQueue() {
    if (!job) return;
    const n = 1;
    await api.jobSetQueue(job.id, n).catch(() => {});
    toast(t('jobs.addedToQueue', { n }));
    close(null);
  }
</script>

<Modal title={job ? jobTitle(job) : t('jobs.st.scanning')} width={560} onclose={() => close(null)}>
  {#if job}
    <dl class="where">
      {#if job.sourceDir}
        <dt>{t('jobs.d.from')}</dt>
        <dd class="path" title={job.sourceDir}>{job.sourceDir}</dd>
      {/if}
      {#if job.dest && job.op !== 'delete'}
        <dt>{t('jobs.d.to')}</dt>
        <dd class="path" title={job.dest}>{job.dest}</dd>
      {/if}
    </dl>

    <div class="block">
      <div class="line">
        <span class="name ellipsis" title={job.current}>{job.current ? baseName(job.current) : jobStatus(job)}</span>
        {#if job.currentSize > 0}
          <span class="num muted">{size(job.currentDone)} / {size(job.currentSize)}</span>
        {/if}
      </div>
      <ProgressBar pct={filePct} indeterminate={job.status === 'scanning'} />
    </div>

    <div class="block">
      <div class="line">
        <span>{t('progress.total')} <span class="muted">· {jobStatus(job)}</span></span>
        <span class="num muted">
          {#if job.totalBytes > 0}{size(job.doneBytes)} / {size(job.totalBytes)} ·{/if}
          {t('jobs.files', { done: job.doneFiles, total: job.totalFiles })} · {Math.floor(jobPct(job))} %
        </span>
      </div>
      <ProgressBar pct={jobPct(job)} indeterminate={job.status === 'scanning'} failed={job.status === 'failed'} />
    </div>

    <div class="stats">
      <div><span class="muted">{t('jobs.d.speed')}</span>{job.status === 'running' && job.speed > 0 ? formatSpeed(job.speed, locale, settings.sizeFormat) : '–'}</div>
      <div><span class="muted">{t('jobs.d.elapsed')}</span>{elapsed !== null ? formatDuration(elapsed, locale) : '–'}</div>
      <div><span class="muted">{t('jobs.d.remaining')}</span>{job.status === 'running' && job.eta !== null ? formatDuration(job.eta, locale) : '–'}</div>
    </div>

    {#if job.skipped || job.errors}
      <div class="problems muted">
        {#if job.skipped}<span>{t('jobs.skipped', { count: job.skipped })}</span>{/if}
        {#if job.errors}<span class="bad">{t('jobs.errors', { count: job.errors })}</span>{/if}
      </div>
    {/if}
    {#if job.error}
      <div class="err">{errorText(job.error)}</div>
    {/if}
  {:else}
    <ProgressBar pct={0} indeterminate />
  {/if}

  {#snippet footer()}
    {#if running && job}
      {#if job.queue === null}
        <button class="btn b-extra" title={t('jobs.queue', { n: 1 })} onclick={toQueue}>{t('progress.toQueue')}</button>
      {/if}
      <button class="btn b-extra" onclick={() => api.jobPause(job.id, !paused)}>{paused ? t('jobs.resume') : t('jobs.pause')}</button>
      <button class="btn b-cancel" onclick={() => api.jobCancel(job.id)}>{t('jobs.cancel')}</button>
      <button class="btn primary b-primary" data-autofocus onclick={() => close(null)}>{t('progress.background')}</button>
    {:else}
      <button class="btn primary b-primary" data-autofocus onclick={() => close(null)}>{t('progress.close')}</button>
    {/if}
  {/snippet}
</Modal>

<style>
  .where {
    display: grid;
    grid-template-columns: auto 1fr;
    gap: 3px 10px;
    margin: 0 0 12px;
    font-size: 12px;
  }
  dt {
    color: var(--text-2);
  }
  dd {
    margin: 0;
    min-width: 0;
  }
  .path {
    overflow-wrap: anywhere;
    user-select: text;
    -webkit-user-select: text;
  }
  .block {
    margin-top: 10px;
  }
  .line {
    display: flex;
    align-items: baseline;
    justify-content: space-between;
    gap: 12px;
    font-size: 12px;
  }
  .name {
    min-width: 0;
    font-weight: 500;
  }
  .num {
    flex: none;
    font-variant-numeric: tabular-nums;
  }
  .stats {
    display: grid;
    grid-template-columns: repeat(3, 1fr);
    gap: 8px;
    margin-top: 14px;
    padding: 8px 10px;
    border-radius: var(--r-m);
    background: var(--bg-subtle);
    font-size: 13px;
    font-variant-numeric: tabular-nums;
  }
  .stats div {
    display: flex;
    flex-direction: column;
    gap: 1px;
  }
  .stats .muted {
    font-size: 11px;
  }
  .problems {
    display: flex;
    gap: 12px;
    margin-top: 8px;
    font-size: 12px;
  }
  .bad {
    color: var(--danger);
  }
  .err {
    margin-top: 8px;
    font-size: 12px;
    color: var(--danger);
    white-space: pre-line;
    user-select: text;
    -webkit-user-select: text;
  }
</style>
