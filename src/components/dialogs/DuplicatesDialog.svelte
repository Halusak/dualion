<script lang="ts">
  // Duplicate finder: exact copies (content hash) and similar photos.
  import { onDestroy, untrack } from 'svelte';
  import { api } from '../../lib/api';
  import { formatDate, formatSize } from '../../lib/format';
  import { localeTag, t } from '../../lib/i18n/index.svelte';
  import { dirName } from '../../lib/paths';
  import { dpr, thumbUrl } from '../../lib/platform';
  import { kindOf } from '../../lib/columns';
  import { app } from '../../lib/state/app.svelte';
  import { settings } from '../../lib/state/settings.svelte';
  import { showError, toast } from '../../lib/state/ui.svelte';
  import type { DupeGroup } from '../../lib/types';
  import Modal from './Modal.svelte';

  let { roots: initialRoots, close }: { roots: string[]; close: (r: null) => void } = $props();

  let rootsText = $state(untrack(() => initialRoots.join('\n')));
  let minKb = $state('100');
  let hidden = $state(false);
  let similar = $state(false);
  let threshold = $state(5);
  let running = $state(false);
  let phase = $state('');
  let progress = $state({ done: 0, total: 0 });
  let groups = $state.raw<DupeGroup[] | null>(null);
  let elapsed = $state(0);
  let marked = $state.raw<Set<string>>(new Set());
  let opId = 0;
  const locale = localeTag();
  const size = (n: number) => formatSize(n, locale, settings.sizeFormat);
  const home = app.info.home;
  const short = (p: string) => (home && p.startsWith(home) ? `~${p.slice(home.length)}` : p);

  async function start() {
    groups = null;
    marked = new Set();
    running = true;
    phase = 'scan';
    progress = { done: 0, total: 0 };
    try {
      opId = await api.dupesStart(
        {
          roots: rootsText.split('\n').map((s) => s.trim()).filter(Boolean),
          minSize: Math.max(1, Math.round(parseFloat(minKb.replace(',', '.')) * 1024) || 1),
          hidden,
          similar,
          threshold,
        },
        (e) => {
          if (e.type === 'progress') {
            if (e.phase === 'error') {
              running = false;
              toast(t('dupes.failed'), 'error');
              return;
            }
            phase = e.phase;
            progress = { done: e.done, total: e.total };
          } else {
            groups = e.groups;
            elapsed = e.elapsedMs;
            running = false;
          }
        },
      );
    } catch (err) {
      running = false;
      showError(err);
    }
  }

  function stop() {
    if (opId) void api.cancelOp(opId);
    running = false;
  }
  onDestroy(() => running && stop());

  const wasted = $derived((groups ?? []).reduce((a, g) => a + g.files.slice(1).reduce((b, f) => b + f.size, 0), 0));
  const markedBytes = $derived((groups ?? []).flatMap((g) => g.files).filter((f) => marked.has(f.path)).reduce((a, f) => a + f.size, 0));

  type Keep = 'oldest' | 'newest' | 'shortest' | 'largest';
  function autoMark(keep: Keep) {
    const s = new Set<string>();
    for (const g of groups ?? []) {
      const pick = [...g.files].sort((a, b) =>
        keep === 'oldest' ? a.mtime - b.mtime : keep === 'newest' ? b.mtime - a.mtime : keep === 'largest' ? b.size - a.size : a.path.length - b.path.length,
      )[0];
      for (const f of g.files) if (f !== pick) s.add(f.path);
    }
    marked = s;
  }

  function toggle(p: string) {
    const s = new Set(marked);
    if (s.has(p)) s.delete(p);
    else s.add(p);
    marked = s;
  }

  async function trashMarked() {
    if (!marked.size) return;
    // A delete job: journaled, so it can be undone from the history.
    try {
      await api.jobEnqueue({ op: 'delete', sources: [...marked].map((path) => ({ kind: 'local', path })), options: { toTrash: true } });
      groups = (groups ?? []).map((g) => ({ ...g, files: g.files.filter((f) => !marked.has(f.path)) })).filter((g) => g.files.length > 1);
      marked = new Set();
    } catch (e) {
      showError(e);
    }
  }

  function showInPanel() {
    const hits = (groups ?? []).flatMap((g) => g.files.map((f) => ({ path: f.path, name: f.path.slice(dirName(f.path).length + 1), isDir: false, size: f.size, mtime: f.mtime, matches: [], matchCount: 0 })));
    close(null);
    app.panel.showSearch(`dupes-${Date.now()}`, t('dupes.title'), hits);
  }

  const phaseText = $derived(
    phase === 'scan'
      ? t('dupes.phaseScan', { n: progress.done.toLocaleString(locale) })
      : phase === 'quick'
        ? t('dupes.phaseQuick', { done: progress.done.toLocaleString(locale), total: progress.total.toLocaleString(locale) })
        : phase === 'hash'
          ? t('dupes.phaseHash', { done: size(progress.done), total: size(progress.total) })
          : t('dupes.phaseImages', { done: progress.done.toLocaleString(locale), total: progress.total.toLocaleString(locale) }),
  );
  const px = Math.round(44 * dpr());
</script>

<Modal fill onclose={() => close(null)}>
  <div class="dupes">
    <div class="form">
      <label class="col roots">
        <span>{t('dupes.roots')}</span>
        <textarea class="input" rows="2" bind:value={rootsText} spellcheck="false"></textarea>
      </label>
      <div class="opts">
        <label>{t('dupes.minSize')} <input class="input small" bind:value={minKb} inputmode="decimal" /> kB</label>
        <label class="check"><input type="checkbox" bind:checked={hidden} /> {t('search.hidden')}</label>
        <label class="check"><input type="checkbox" bind:checked={similar} /> {t('dupes.similar')}</label>
        {#if similar}
          <label title={t('dupes.thresholdHint')}>{t('dupes.threshold')} <input type="range" min="0" max="7" bind:value={threshold} /> {threshold}</label>
        {/if}
      </div>
      <div class="go">
        {#if running}
          <button class="btn" onclick={stop}>{t('cancel')}</button>
        {:else}
          <button class="btn primary" onclick={start}>{t('dupes.start')}</button>
        {/if}
      </div>
    </div>

    {#if running}
      <div class="status">{phaseText}</div>
    {:else if groups}
      <div class="status">
        {t('dupes.found', { groups: groups.length.toLocaleString(locale), size: size(wasted), s: (elapsed / 1000).toFixed(1) })}
        <span class="grow"></span>
        {#if groups.length}
          <span class="muted">{t('dupes.markAll')}:</span>
          <button class="link" onclick={() => autoMark('oldest')}>{t('dupes.keepOldest')}</button>
          <button class="link" onclick={() => autoMark('newest')}>{t('dupes.keepNewest')}</button>
          <button class="link" onclick={() => autoMark('shortest')}>{t('dupes.keepShortest')}</button>
          {#if similar}<button class="link" onclick={() => autoMark('largest')}>{t('dupes.keepLargest')}</button>{/if}
          <button class="link" onclick={() => (marked = new Set())}>{t('dupes.unmark')}</button>
        {/if}
      </div>
    {/if}

    <div class="results">
      {#each groups ?? [] as g, gi (gi)}
        <section>
          <header>
            <span class="badge" class:sim={g.kind === 'similar'}>{t(g.kind === 'exact' ? 'dupes.exact' : 'dupes.similarBadge')}</span>
            {g.files.length} × {size(g.files[0].size)}
            <span class="muted"> · {t('dupes.saves', { size: size(g.files.slice(1).reduce((a, f) => a + f.size, 0)) })}</span>
          </header>
          {#each g.files as f (f.path)}
            <label class="file" class:marked={marked.has(f.path)}>
              <input type="checkbox" checked={marked.has(f.path)} onchange={() => toggle(f.path)} />
              {#if ['image', 'raw'].includes(kindOf(f.path.split('.').pop() ?? ''))}
                <img src={thumbUrl(f.path, px, f.mtime)} alt="" loading="lazy" />
              {/if}
              <span class="ellipsis path" title={f.path}>{short(f.path)}</span>
              <span class="muted nowrap">{size(f.size)}</span>
              <span class="muted nowrap">{formatDate(f.mtime, locale)}</span>
            </label>
          {/each}
        </section>
      {:else}
        {#if groups && !running}<div class="empty muted">{t('dupes.none')}</div>{/if}
      {/each}
    </div>

    <footer>
      <span class="muted">{marked.size ? t('dupes.marked', { n: marked.size, size: size(markedBytes) }) : ''}</span>
      <span class="grow"></span>
      <button class="btn" disabled={!groups?.length} onclick={showInPanel}>{t('dupes.toPanel')}</button>
      <button class="btn danger" disabled={!marked.size} onclick={trashMarked}>{t('dupes.trash')}</button>
      <button class="btn" onclick={() => close(null)}>{t('close')}</button>
    </footer>
  </div>
</Modal>

<style>
  .dupes {
    flex: 1;
    min-height: 0;
    display: flex;
    flex-direction: column;
  }
  .form {
    display: grid;
    grid-template-columns: 1fr auto auto;
    gap: 14px;
    align-items: end;
    padding: 12px 14px;
    border-bottom: 1px solid var(--border);
  }
  .col {
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  textarea {
    font-family: var(--font-mono);
    font-size: 12px;
    resize: vertical;
  }
  .opts {
    display: grid;
    gap: 6px;
    font-size: 12px;
  }
  .opts label {
    display: flex;
    align-items: center;
    gap: 6px;
  }
  .small {
    width: 70px;
  }
  .status {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 8px 14px;
    font-size: 12px;
    border-bottom: 1px solid var(--border);
    flex-wrap: wrap;
  }
  .grow {
    flex: 1;
  }
  .link {
    border: none;
    background: none;
    color: var(--accent-text);
    padding: 0;
    cursor: pointer;
    font-size: 12px;
  }
  .results {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    padding: 8px 14px;
  }
  section {
    margin-bottom: 10px;
    border-radius: var(--r-m);
    background: var(--bg-subtle);
    padding: 6px 8px;
  }
  section header {
    font-size: 12px;
    font-weight: 600;
    padding: 2px 4px 6px;
    display: flex;
    align-items: center;
    gap: 6px;
  }
  .badge {
    font-size: 10px;
    padding: 1px 6px;
    border-radius: 8px;
    background: var(--accent-20);
    color: var(--accent-text);
  }
  .badge.sim {
    background: rgba(160, 100, 220, 0.2);
    color: #a064dc;
  }
  .file {
    display: grid;
    grid-template-columns: auto auto 1fr auto auto;
    gap: 10px;
    align-items: center;
    padding: 3px 4px;
    border-radius: var(--r-s);
    font-size: 12px;
  }
  .file:hover {
    background: var(--bg-hover);
  }
  .file.marked .path {
    text-decoration: line-through;
    color: var(--danger);
  }
  .file img {
    width: 44px;
    height: 44px;
    object-fit: cover;
    border-radius: 4px;
  }
  .file:not(:has(img)) {
    grid-template-columns: auto 1fr auto auto;
  }
  .nowrap {
    white-space: nowrap;
    font-variant-numeric: tabular-nums;
  }
  .empty {
    padding: 40px;
    text-align: center;
  }
  footer {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 10px 14px;
    border-top: 1px solid var(--border);
  }
  .btn.danger:not(:disabled) {
    background: var(--danger);
    color: white;
    border-color: transparent;
  }
</style>
