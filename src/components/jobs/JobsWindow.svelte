<script lang="ts">
  import ArrowDown from '@lucide/svelte/icons/arrow-down';
  import ArrowUp from '@lucide/svelte/icons/arrow-up';
  import Check from '@lucide/svelte/icons/check';
  import CircleAlert from '@lucide/svelte/icons/circle-alert';
  import Pause from '@lucide/svelte/icons/pause';
  import Play from '@lucide/svelte/icons/play';
  import X from '@lucide/svelte/icons/x';
  import { listen } from '@tauri-apps/api/event';
  import { getCurrentWindow, Window } from '@tauri-apps/api/window';
  import { onMount } from 'svelte';
  import { api } from '../../lib/api';
  import { useAppearance } from '../../lib/appearance.svelte';
  import { formatDate, formatDuration, formatSize, formatSpeed } from '../../lib/format';
  import { localeTag, t } from '../../lib/i18n/index.svelte';
  import { isFinished, jobAvgSpeed, jobElapsed, jobPct, jobStatus, jobTitle } from '../../lib/jobs';
  import { baseName } from '../../lib/paths';
  import { app } from '../../lib/state/app.svelte';
  import { applySettings, settings, type Settings } from '../../lib/state/settings.svelte';
  import { errorText } from '../../lib/state/ui.svelte';
  import type { JobSnapshot } from '../../lib/types';
  import ProgressBar from './ProgressBar.svelte';

  useAppearance();

  let jobs = $state.raw<JobSnapshot[]>([]);
  let selected = $state<number | null>(null);
  let now = $state(Date.now());

  const locale = $derived(localeTag());
  const size = (n: number) => formatSize(n, locale, settings.sizeFormat);
  const speed = (n: number) => formatSpeed(n, locale, settings.sizeFormat);
  const date = (ms: number | null) => (ms ? formatDate(ms, locale, true) : '–');

  // Running and queued jobs in queue order, then the finished ones, newest first.
  const active = $derived(jobs.filter((j) => !isFinished(j)));
  const done = $derived(jobs.filter(isFinished).sort((a, b) => (b.finished ?? 0) - (a.finished ?? 0)));
  const ordered = $derived([...active, ...done]);
  const job = $derived(ordered.find((j) => j.id === selected) ?? null);

  // Keep a selection: the first job, or the next one when the selected disappears.
  $effect(() => {
    if (selected === null || !ordered.some((j) => j.id === selected)) selected = ordered[0]?.id ?? null;
  });

  $effect(() => {
    document.title = active.length ? `${t('jobs.title')} – ${t('jobs.running', { count: active.length })}` : t('jobs.title');
  });

  async function loadSettings() {
    applySettings(await api.storeLoad<Partial<Settings>>('settings').catch(() => null));
  }

  function select(delta: number) {
    const i = ordered.findIndex((j) => j.id === selected);
    const next = ordered[Math.max(0, Math.min(ordered.length - 1, i + delta))];
    if (next) {
      selected = next.id;
      document.querySelector(`[data-job="${next.id}"]`)?.scrollIntoView({ block: 'nearest' });
    }
  }

  function onKey(e: KeyboardEvent) {
    if ((e.target as HTMLElement).closest('select, input')) return;
    if (e.key === 'ArrowDown') select(1);
    else if (e.key === 'ArrowUp') select(-1);
    else if (e.key === 'Home') select(-ordered.length);
    else if (e.key === 'End') select(ordered.length);
    else if (e.key === 'Escape') void getCurrentWindow().close();
    else return;
    e.preventDefault();
  }

  async function focusMain() {
    const main = await Window.getByLabel('main');
    await main?.setFocus();
  }

  function setQueue(j: JobSnapshot, v: string) {
    void api.jobSetQueue(j.id, v === '' ? null : Number(v));
  }

  const paused = (j: JobSnapshot) => j.status === 'paused' && !j.held;
  const timeCol = (j: JobSnapshot) => {
    if (j.status === 'running' && j.eta !== null) return t('jobs.eta', { time: formatDuration(j.eta, locale) });
    const s = jobElapsed(j, now);
    return s === null ? '' : formatDuration(s, locale);
  };

  onMount(() => {
    const unlisteners: Promise<() => void>[] = [];
    unlisteners.push(listen<JobSnapshot[]>('jobs', (e) => (jobs = e.payload)));
    // Settings are edited in the main window: pick up changes when coming back.
    unlisteners.push(getCurrentWindow().onFocusChanged((e) => e.payload && void loadSettings()));
    const timer = setInterval(() => (now = Date.now()), 1000);
    const noMenu = (e: MouseEvent) => {
      if (!(e.target as HTMLElement).closest('.selectable')) e.preventDefault();
    };
    window.addEventListener('contextmenu', noMenu);
    (async () => {
      try {
        app.info = await api.appInfo();
        await loadSettings();
        jobs = await api.jobsList();
      } finally {
        const w = getCurrentWindow();
        await w.show();
        await w.setFocus();
      }
    })();
    return () => {
      clearInterval(timer);
      window.removeEventListener('contextmenu', noMenu);
      for (const u of unlisteners) void u.then((f) => f());
    };
  });
</script>

<svelte:window onkeydown={onKey} />

{#snippet row(j: JobSnapshot)}
  <!-- svelte-ignore a11y_click_events_have_key_events -->
  <div
    class="row"
    class:sel={j.id === selected}
    class:failed={j.status === 'failed'}
    role="row"
    tabindex="-1"
    aria-selected={j.id === selected}
    data-job={j.id}
    onclick={() => (selected = j.id)}
  >
    <span class="c-op">
      <span class="icon">
        {#if j.status === 'done'}<span class="ok"><Check size={13} /></span>
        {:else if j.status === 'failed'}<span class="bad"><CircleAlert size={13} /></span>
        {:else if j.status === 'cancelled'}<X size={13} />
        {:else if paused(j)}<Pause size={13} />
        {/if}
      </span>
      <span class="ellipsis" title={jobTitle(j)}>{jobTitle(j)}</span>
    </span>
    <span class="c-status ellipsis" class:warn={j.status === 'waiting'}>
      {jobStatus(j)}{#if j.queue !== null && !isFinished(j) && !j.held}{` · ${t('jobs.queue', { n: j.queue })}`}{/if}
    </span>
    <span class="c-progress">
      <ProgressBar pct={jobPct(j)} indeterminate={j.status === 'scanning'} done={j.status === 'done'} failed={j.status === 'failed'} />
      <span class="pct">{Math.floor(jobPct(j))} %</span>
    </span>
    <span class="c-speed num">{j.status === 'running' && j.speed > 0 ? speed(j.speed) : isFinished(j) && jobAvgSpeed(j) > 0 ? speed(jobAvgSpeed(j)) : ''}</span>
    <span class="c-time num">{timeCol(j)}</span>
  </div>
{/snippet}

<div class="win">
  <div class="table" role="grid" aria-label={t('jobs.title')}>
    <div class="head" role="row">
      <span class="c-op">{t('jobs.col.op')}</span>
      <span class="c-status">{t('jobs.col.status')}</span>
      <span class="c-progress">{t('jobs.col.progress')}</span>
      <span class="c-speed">{t('jobs.col.speed')}</span>
      <span class="c-time">{t('jobs.col.time')}</span>
    </div>
    <div class="rows">
      {#if !jobs.length}
        <div class="empty muted">{t('jobs.empty')}</div>
      {/if}
      {#if active.length}
        <div class="group">{t('jobs.active')}</div>
        {#each active as j (j.id)}{@render row(j)}{/each}
      {/if}
      {#if done.length}
        <div class="group">
          <span>{t('jobs.finished')}</span>
          <button class="btn small" onclick={() => api.jobsClearFinished()}>{t('jobs.clear')}</button>
        </div>
        {#each done as j (j.id)}{@render row(j)}{/each}
      {/if}
    </div>
  </div>

  <section class="detail">
    {#if job}
      <div class="d-head">
        <h2 class="selectable">{jobTitle(job)}</h2>
        <div class="actions">
          {#if job.status === 'waiting'}
            <button class="btn small primary" onclick={focusMain}>{t('jobs.respond')}</button>
          {/if}
          {#if job.status === 'queued'}
            <button class="icon-btn" title={t('jobs.up')} onclick={() => api.jobMove(job.id, -1)}><ArrowUp size={15} /></button>
            <button class="icon-btn" title={t('jobs.down')} onclick={() => api.jobMove(job.id, 1)}><ArrowDown size={15} /></button>
          {/if}
          {#if job.status === 'queued' || job.held}
            <button class="btn small" onclick={() => api.jobStartNow(job.id)}><Play size={13} /> {t('jobs.startNow')}</button>
          {:else if job.status === 'running' || job.status === 'scanning'}
            <button class="btn small" onclick={() => api.jobPause(job.id, true)}><Pause size={13} /> {t('jobs.pause')}</button>
          {:else if paused(job)}
            <button class="btn small" onclick={() => api.jobPause(job.id, false)}><Play size={13} /> {t('jobs.resume')}</button>
          {/if}
          {#if !isFinished(job)}
            <button class="btn small" onclick={() => api.jobCancel(job.id)}><X size={13} /> {t('jobs.cancel')}</button>
          {/if}
        </div>
      </div>

      {#if !isFinished(job)}
        <div class="bars">
          {#if job.currentSize > 0}
            <div class="line">
              <span class="ellipsis">{baseName(job.current)}</span>
              <span class="num muted">{size(job.currentDone)} / {size(job.currentSize)}</span>
            </div>
            <ProgressBar pct={Math.min(100, (100 * job.currentDone) / job.currentSize)} />
          {/if}
          <div class="line">
            <span>{t('progress.total')}</span>
            <span class="num muted">{Math.floor(jobPct(job))} %</span>
          </div>
          <ProgressBar pct={jobPct(job)} indeterminate={job.status === 'scanning'} />
        </div>
      {/if}

      <dl class="selectable">
        {#if job.sourceDir}
          <dt>{t('jobs.d.from')}</dt>
          <dd class="path">{job.sourceDir}</dd>
        {/if}
        {#if job.dest && job.op !== 'delete'}
          <dt>{t('jobs.d.to')}</dt>
          <dd class="path">{job.dest}</dd>
        {/if}
        <dt>{t('jobs.d.sources')}</dt>
        <dd>
          <div class="sources">
            {#each job.sources as s (s)}<div class="path">{s}</div>{/each}
            {#if job.sourceCount > job.sources.length}
              <div class="muted">{t('jobs.d.more', { count: job.sourceCount - job.sources.length })}</div>
            {/if}
          </div>
        </dd>
        {#if job.current && !isFinished(job)}
          <dt>{t('jobs.d.current')}</dt>
          <dd class="path">{job.current}</dd>
        {/if}
        <dt>{t('jobs.col.status')}</dt>
        <dd>{jobStatus(job)}</dd>
        {#if job.totalBytes > 0}
          <dt>{t('jobs.d.size')}</dt>
          <dd class="num">{size(job.doneBytes)} / {size(job.totalBytes)}</dd>
        {/if}
        {#if job.totalFiles > 0}
          <dt>{t('jobs.d.files')}</dt>
          <dd class="num">{t('jobs.files', { done: job.doneFiles, total: job.totalFiles })}</dd>
        {/if}
        <dt>{t('jobs.d.created')}</dt>
        <dd class="num">{date(job.created)}</dd>
        <dt>{t('jobs.d.started')}</dt>
        <dd class="num">{date(job.started)}</dd>
        {#if job.finished}
          <dt>{t('jobs.d.finished')}</dt>
          <dd class="num">{date(job.finished)}</dd>
        {/if}
        {#if jobElapsed(job, now) !== null}
          <dt>{isFinished(job) ? t('jobs.d.duration') : t('jobs.d.elapsed')}</dt>
          <dd class="num">{formatDuration(jobElapsed(job, now)!, locale)}</dd>
        {/if}
        {#if job.status === 'running' && job.eta !== null}
          <dt>{t('jobs.d.remaining')}</dt>
          <dd class="num">{formatDuration(job.eta, locale)}</dd>
        {/if}
        {#if job.status === 'running' && job.speed > 0}
          <dt>{t('jobs.d.speed')}</dt>
          <dd class="num">{speed(job.speed)}</dd>
        {/if}
        {#if jobAvgSpeed(job, now) > 0}
          <dt>{t('jobs.d.avgSpeed')}</dt>
          <dd class="num">{speed(jobAvgSpeed(job, now))}</dd>
        {/if}
        {#if !isFinished(job)}
          <dt>{t('jobs.d.queue')}</dt>
          <dd>
            <select class="input queue" value={job.queue === null ? '' : String(job.queue)} onchange={(e) => setQueue(job, e.currentTarget.value)}>
              <option value="">{t('jobs.queueNone')}</option>
              {#each [1, 2, 3] as n (n)}<option value={String(n)}>{t('copy.queueN', { n })}</option>{/each}
            </select>
          </dd>
        {/if}
        {#if job.skipped || job.errors}
          <dt>{t('jobs.d.problems')}</dt>
          <dd>
            {#if job.skipped}{t('jobs.skipped', { count: job.skipped })}{/if}
            {#if job.skipped && job.errors}·{/if}
            {#if job.errors}<span class="bad">{t('jobs.errors', { count: job.errors })}</span>{/if}
          </dd>
        {/if}
        {#if job.error}
          <dt>{t('jobs.d.error')}</dt>
          <dd class="bad pre">{errorText(job.error)}</dd>
        {/if}
      </dl>
    {:else}
      <div class="empty muted">{t('jobs.select')}</div>
    {/if}
  </section>
</div>

<style>
  .win {
    height: 100%;
    display: grid;
    grid-template-rows: minmax(140px, 1fr) minmax(180px, auto);
    background: var(--bg-panel);
    color: var(--text);
  }
  .table {
    display: flex;
    flex-direction: column;
    min-height: 0;
    border-bottom: 1px solid var(--border);
  }
  .head,
  .row {
    display: grid;
    grid-template-columns: minmax(180px, 2.4fr) minmax(110px, 1.2fr) minmax(120px, 1.2fr) 90px 80px;
    align-items: center;
    gap: 10px;
    padding: 0 12px;
  }
  .head {
    height: 28px;
    flex: none;
    font-size: 11px;
    font-weight: 600;
    color: var(--text-2);
    border-bottom: 1px solid var(--border);
  }
  .rows {
    flex: 1;
    overflow: auto;
    padding-bottom: 4px;
  }
  .group {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 8px 12px 3px;
    font-size: 11px;
    font-weight: 600;
    color: var(--text-2);
  }
  .row {
    height: 30px;
    cursor: default;
    outline: none;
  }
  .row.sel {
    background: var(--cursor-bg);
    color: var(--cursor-fg);
  }
  .row.sel .ok,
  .row.sel .bad,
  .row.sel .warn {
    color: inherit;
  }
  .c-op {
    display: flex;
    align-items: center;
    gap: 6px;
    min-width: 0;
  }
  .icon {
    display: inline-flex;
    width: 14px;
    flex: none;
  }
  .c-progress {
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .c-progress :global(.bar) {
    flex: 1;
    margin: 0;
  }
  .row.sel .c-progress :global(.bar) {
    background: color-mix(in srgb, currentColor 30%, transparent);
  }
  .row.sel .c-progress :global(.bar span) {
    background: currentColor;
  }
  .pct {
    width: 36px;
    text-align: right;
    font-size: 11px;
    font-variant-numeric: tabular-nums;
  }
  .c-speed,
  .c-time {
    text-align: right;
  }
  .num {
    font-variant-numeric: tabular-nums;
  }
  .ok {
    color: var(--ok);
  }
  .bad {
    color: var(--danger);
  }
  .warn {
    color: var(--warn);
  }
  .empty {
    padding: 28px;
    text-align: center;
  }
  .detail {
    overflow: auto;
    max-height: 60vh;
    padding: 12px 16px 16px;
    background: var(--bg-window);
  }
  .d-head {
    display: flex;
    align-items: flex-start;
    gap: 12px;
  }
  h2 {
    flex: 1;
    margin: 2px 0 0;
    font-size: 14px;
    font-weight: 600;
    overflow-wrap: anywhere;
  }
  .actions {
    display: flex;
    align-items: center;
    gap: 4px;
    flex: none;
  }
  .actions .btn {
    display: inline-flex;
    align-items: center;
    gap: 4px;
  }
  .bars {
    margin-top: 10px;
  }
  .line {
    display: flex;
    justify-content: space-between;
    gap: 12px;
    margin-top: 6px;
    font-size: 12px;
  }
  dl {
    display: grid;
    grid-template-columns: max-content 1fr;
    gap: 5px 14px;
    margin: 14px 0 0;
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
  }
  .pre {
    white-space: pre-line;
  }
  .sources {
    max-height: 120px;
    overflow: auto;
    display: grid;
    gap: 2px;
  }
  .selectable {
    user-select: text;
    -webkit-user-select: text;
    cursor: text;
  }
  .queue {
    height: 24px;
  }
  .selectable select {
    cursor: default;
  }
</style>
