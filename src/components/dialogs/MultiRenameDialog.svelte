<script lang="ts" module>
  import { defaultRule, type RenameRule } from '../../lib/multirename';
  let lastRule: RenameRule | null = null;
</script>

<script lang="ts">
  import { api } from '../../lib/api';
  import { localeTag, t } from '../../lib/i18n/index.svelte';
  import { previewRename } from '../../lib/multirename';
  import { baseName } from '../../lib/paths';
  import { isWindows } from '../../lib/platform';
  import { showError, toast } from '../../lib/state/ui.svelte';
  import type { Entry } from '../../lib/types';
  import Modal from './Modal.svelte';

  let { dir, entries, close }: { dir: string; entries: Entry[]; close: (r: { dir: string; pairs: { from: string; to: string }[] } | null) => void } = $props();

  const rule = $state<RenameRule>(lastRule ? { ...lastRule } : defaultRule());
  let busy = $state(false);

  const preview = $derived(
    previewRename(
      entries.map((e) => ({ name: e.name, isDir: e.isDir, mtime: e.mtime })),
      rule,
      { parent: baseName(dir), locale: localeTag(), windows: isWindows, caseInsensitive: true },
    ),
  );
  const changes = $derived(preview.filter((p) => p.status === 'ok'));
  const blocked = $derived(preview.some((p) => p.status === 'duplicate' || p.status === 'invalid'));

  function insert(token: string) {
    rule.nameMask += token;
  }

  async function apply() {
    if (busy || blocked || !changes.length) return;
    busy = true;
    lastRule = { ...rule };
    try {
      const res = await api.batchRename(
        dir,
        changes.map((p) => ({ from: p.from, to: p.to })),
      );
      const ok = res.filter((r) => !r.error);
      const failed = res.length - ok.length;
      toast(t('mr.done', { count: ok.length }) + (failed ? ` · ${t('mr.failed', { count: failed })}` : ''), failed ? 'error' : 'info');
      close({ dir, pairs: ok.map((r) => ({ from: r.from, to: r.to })) });
    } catch (e) {
      showError(e);
    } finally {
      busy = false;
    }
  }

  const presets: { label: string; apply: () => void }[] = [
    { label: '[N].[E]', apply: () => Object.assign(rule, defaultRule()) },
    { label: '[Y]-[M]-[D] [N]', apply: () => (rule.nameMask = '[Y]-[M]-[D] [N]') },
    { label: '[P] [C]', apply: () => Object.assign(rule, { nameMask: '[P] [C]', counterDigits: 3 }) },
    { label: 'lowercase', apply: () => (rule.caseMode = 'lower') },
  ];
</script>

<Modal title={t('mr.title')} width={900} onclose={() => close(null)} onsubmit={apply}>
  <div class="mr">
    <div class="form">
      <label>{t('mr.mask')}<input class="input mono" bind:value={rule.nameMask} spellcheck="false" data-autofocus /></label>
      <label>{t('mr.extMask')}<input class="input mono" bind:value={rule.extMask} spellcheck="false" /></label>
      <div class="tokens">
        {#each ['[N]', '[N1-8]', '[E]', '[C]', '[Y]', '[M]', '[D]', '[h]', '[m]', '[s]', '[P]'] as tk (tk)}
          <button class="btn small" onclick={() => insert(tk)}>{tk}</button>
        {/each}
      </div>
      <label>{t('mr.search')}<input class="input" bind:value={rule.search} spellcheck="false" /></label>
      <label>{t('mr.replace')}<input class="input" bind:value={rule.replace} spellcheck="false" /></label>
      <div class="checks">
        <label class="check"><input type="checkbox" bind:checked={rule.regex} /> {t('mr.regex')}</label>
        <label class="check"><input type="checkbox" bind:checked={rule.caseSensitive} /> {t('mr.caseSensitive')}</label>
      </div>
      <label>{t('mr.case')}
        <select class="input" bind:value={rule.caseMode}>
          <option value="keep">{t('mr.caseKeep')}</option>
          <option value="lower">{t('mr.caseLower')}</option>
          <option value="upper">{t('mr.caseUpper')}</option>
          <option value="title">{t('mr.caseTitle')}</option>
          <option value="first">{t('mr.caseFirst')}</option>
        </select>
      </label>
      <fieldset>
        <legend>{t('mr.counter')}</legend>
        <label>{t('mr.start')}<input class="input" type="number" bind:value={rule.counterStart} /></label>
        <label>{t('mr.step')}<input class="input" type="number" bind:value={rule.counterStep} /></label>
        <label>{t('mr.digits')}<input class="input" type="number" min="1" max="9" bind:value={rule.counterDigits} /></label>
      </fieldset>
      <div class="presets">
        <span class="muted">{t('mr.presets')}:</span>
        {#each presets as p (p.label)}
          <button class="btn small" onclick={p.apply}>{p.label}</button>
        {/each}
      </div>
      <p class="help muted">{t('mr.help')}</p>
    </div>
    <div class="table">
      <div class="thead">
        <span>{t('mr.old')}</span>
        <span>{t('mr.new')}</span>
      </div>
      <div class="tbody">
        {#each preview.slice(0, 3000) as p, i (i)}
          <div class="tr {p.status}">
            <span class="ellipsis" title={p.from}>{p.from}</span>
            <span class="ellipsis" title={p.to}>
              {p.to}
              {#if p.status === 'duplicate'}<em>({t('mr.duplicate')})</em>{/if}
              {#if p.status === 'invalid'}<em>({t('mr.invalid')})</em>{/if}
            </span>
          </div>
        {/each}
      </div>
    </div>
  </div>
  {#snippet footer()}
    <span class="b-extra muted count">{changes.length} / {preview.length}</span>
    <button class="btn b-cancel" onclick={() => close(null)}>{t('cancel')}</button>
    <button class="btn primary b-primary" disabled={busy || blocked || !changes.length} onclick={apply}>{t('mr.rename')}</button>
  {/snippet}
</Modal>

<style>
  .mr {
    display: grid;
    grid-template-columns: 330px 1fr;
    gap: 16px;
    height: min(560px, calc(100vh - 220px));
  }
  .form {
    display: flex;
    flex-direction: column;
    gap: 8px;
    overflow: auto;
    padding-right: 4px;
  }
  .form label:not(.check) {
    display: flex;
    flex-direction: column;
    gap: 4px;
    font-size: 12px;
    color: var(--text-2);
  }
  .form .input {
    color: var(--text);
  }
  .tokens,
  .presets {
    display: flex;
    flex-wrap: wrap;
    gap: 4px;
    align-items: center;
  }
  .checks {
    display: flex;
    gap: 14px;
  }
  fieldset {
    border: 1px solid var(--border);
    border-radius: var(--r-m);
    display: grid;
    grid-template-columns: repeat(3, 1fr);
    gap: 8px;
    padding: 6px 10px 10px;
    margin: 0;
  }
  legend {
    font-size: 12px;
    color: var(--text-2);
    padding: 0 4px;
  }
  .help {
    font-size: 11px;
    margin: 4px 0 0;
    line-height: 1.5;
  }
  .table {
    display: flex;
    flex-direction: column;
    min-height: 0;
    border: 1px solid var(--border);
    border-radius: var(--r-m);
    overflow: hidden;
    background: var(--bg-panel);
  }
  .thead,
  .tr {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 12px;
    padding: 0 10px;
  }
  .thead {
    height: 26px;
    align-items: center;
    font-size: 11px;
    color: var(--text-2);
    border-bottom: 1px solid var(--border);
  }
  .tbody {
    overflow: auto;
    flex: 1;
  }
  .tr {
    height: 24px;
    align-items: center;
    font-size: 12px;
  }
  .tr:nth-child(even) {
    background: var(--bg-row-alt);
  }
  .tr.ok span:last-child {
    color: var(--accent-text);
    font-weight: 600;
  }
  .tr.unchanged span:last-child {
    color: var(--text-3);
  }
  .tr.duplicate span:last-child,
  .tr.invalid span:last-child {
    color: var(--danger);
  }
  em {
    font-style: normal;
    font-size: 11px;
    margin-left: 4px;
  }
  .count {
    font-size: 12px;
    font-variant-numeric: tabular-nums;
  }
</style>
