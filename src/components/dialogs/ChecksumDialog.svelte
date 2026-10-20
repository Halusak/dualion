<script lang="ts">
  import { onDestroy } from 'svelte';
  import { api } from '../../lib/api';
  import { copyText } from '../../lib/clipboard';
  import { formatSize } from '../../lib/format';
  import { localeTag, t } from '../../lib/i18n/index.svelte';
  import { baseName, dirName, joinLocal } from '../../lib/paths';
  import { app } from '../../lib/state/app.svelte';
  import { settings } from '../../lib/state/settings.svelte';
  import { errorText, showError, toast } from '../../lib/state/ui.svelte';
  import Modal from './Modal.svelte';

  let { paths, close }: { paths: string[]; close: (r: null) => void } = $props();

  const algos = ['md5', 'sha1', 'sha256', 'sha512', 'crc32', 'xxh3'] as const;
  let algo = $state<(typeof algos)[number]>('sha256');
  let results = $state<{ path: string; hash: string | null; error: string | null }[]>([]);
  let running = $state(false);
  let done = $state(0);
  let total = $state(0);
  let compare = $state('');
  let op: number | null = null;
  const locale = localeTag();

  async function compute() {
    results = [];
    running = true;
    done = 0;
    try {
      op = await api.checksumStart(paths, algo, (e) => {
        if (e.type === 'progress') {
          done = e.doneBytes;
          total = e.totalBytes;
        } else if (e.type === 'result') {
          results = [...results, { path: e.path, hash: e.hash, error: e.error ? errorText(e.error) : null }];
        } else {
          running = false;
          op = null;
        }
      });
    } catch (e) {
      running = false;
      showError(e);
    }
  }

  onDestroy(() => {
    if (op !== null) void api.cancelOp(op);
  });

  const text = $derived(results.filter((r) => r.hash).map((r) => `${r.hash}  ${baseName(r.path)}`).join('\n'));

  async function saveFile() {
    const dir = dirName(paths[0]);
    const name = paths.length === 1 ? `${baseName(paths[0])}.${algo}` : `checksums.${algo}`;
    try {
      await api.writeTextFile(joinLocal(dir, name), text + '\n');
      toast(`${t('save')}: ${name}`);
      app.refreshLocation({ kind: 'local', path: dir });
    } catch (e) {
      showError(e);
    }
  }

  function matches(hash: string | null) {
    const c = compare.trim().toLowerCase();
    return c && hash ? c === hash.toLowerCase() : null;
  }
</script>

<Modal title={t('sum.title')} width={760} onclose={() => close(null)} onsubmit={compute}>
  <div class="top">
    <label>{t('sum.algo')}
      <select class="input" bind:value={algo}>
        {#each algos as a (a)}<option value={a}>{a.toUpperCase()}</option>{/each}
      </select>
    </label>
    <label class="grow">{t('sum.compare')}
      <input class="input mono" bind:value={compare} placeholder="…" spellcheck="false" />
    </label>
  </div>
  {#if running}
    <div class="progress"><span style="width: {total ? (100 * done) / total : 0}%"></span></div>
    <div class="muted small">{t('sum.working')} {formatSize(done, locale, settings.sizeFormat)} / {formatSize(total, locale, settings.sizeFormat)}</div>
  {/if}
  <div class="results">
    {#each results as r (r.path)}
      {@const m = matches(r.hash)}
      <div class="res">
        <span class="name ellipsis" title={r.path}>{baseName(r.path)}</span>
        {#if r.hash}
          <code class:ok={m === true} class:bad={m === false}>{r.hash}</code>
          {#if m !== null}<span class:ok={m} class:bad={!m}>{m ? t('sum.match') : t('sum.mismatch')}</span>{/if}
        {:else}
          <span class="bad">{r.error}</span>
        {/if}
      </div>
    {/each}
  </div>
  {#snippet footer()}
    <button class="btn b-extra" disabled={!results.length} onclick={() => copyText(text).then(() => toast(t('copied')))}>{t('sum.copyAll')}</button>
    <button class="btn b-extra" disabled={!results.length} onclick={saveFile}>{t('sum.save')}</button>
    <button class="btn b-cancel" onclick={() => close(null)}>{t('close')}</button>
    <button class="btn primary b-primary" disabled={running} onclick={compute}>{t('sum.compute')}</button>
  {/snippet}
</Modal>

<style>
  .top {
    display: flex;
    gap: 12px;
  }
  .top label {
    display: flex;
    flex-direction: column;
    gap: 4px;
    font-size: 12px;
    color: var(--text-2);
  }
  .grow {
    flex: 1;
  }
  .progress {
    height: 5px;
    border-radius: 3px;
    background: var(--border);
    overflow: hidden;
    margin-top: 12px;
  }
  .progress span {
    display: block;
    height: 100%;
    background: var(--accent);
  }
  .small {
    font-size: 11px;
    margin-top: 4px;
  }
  .results {
    margin-top: 12px;
    max-height: 340px;
    overflow: auto;
    display: grid;
    gap: 4px;
  }
  .res {
    display: grid;
    grid-template-columns: 180px 1fr auto;
    gap: 10px;
    align-items: center;
    font-size: 12px;
    padding: 4px 6px;
    border-radius: var(--r-s);
    background: var(--bg-subtle);
  }
  code {
    font-family: var(--font-mono);
    font-size: 11px;
    word-break: break-all;
    -webkit-user-select: text;
    user-select: text;
  }
  .ok {
    color: var(--ok);
  }
  .bad {
    color: var(--danger);
  }
</style>
