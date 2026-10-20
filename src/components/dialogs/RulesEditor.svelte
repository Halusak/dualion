<script lang="ts">
  // Folder rules: watch a folder, match files with a query, move / copy / rename / trash.
  import FolderOpen from '@lucide/svelte/icons/folder-open';
  import Plus from '@lucide/svelte/icons/plus';
  import Trash2 from '@lucide/svelte/icons/trash-2';
  import { open as openFileDialog } from '@tauri-apps/plugin-dialog';
  import { onMount } from 'svelte';
  import { api } from '../../lib/api';
  import { formatDate } from '../../lib/format';
  import { localeTag, t } from '../../lib/i18n/index.svelte';
  import { app } from '../../lib/state/app.svelte';
  import { pushHistory } from '../../lib/state/history.svelte';
  import { settings } from '../../lib/state/settings.svelte';
  import { errorText, toast } from '../../lib/state/ui.svelte';
  import type { Rule, RuleActivity } from '../../lib/types';

  let selectedId = $state<string | null>(settings.rules.list[0]?.id ?? null);
  let error = $state('');
  let preview = $state.raw<RuleActivity[] | null>(null);
  let log = $state.raw<RuleActivity[]>([]);
  const locale = localeTag();
  const home = app.info.home;
  const short = (p: string | null) => (p && home && p.startsWith(home) ? `~${p.slice(home.length)}` : (p ?? ''));

  const rule = $derived(settings.rules.list.find((r) => r.id === selectedId) ?? null);

  onMount(async () => {
    log = await api.rulesLog().catch(() => []);
  });

  function add() {
    const r: Rule = {
      id: crypto.randomUUID(),
      name: t('rules.newName'),
      enabled: true,
      folder: app.info.places.find((p) => p.key === 'downloads')?.path ?? home,
      recursive: false,
      query: 'ext:pdf',
      action: 'move',
      target: `${app.info.places.find((p) => p.key === 'documents')?.path ?? home}/{type}/{year}`,
      rename: '',
      auto: false,
    };
    settings.rules.list = [...settings.rules.list, r];
    selectedId = r.id;
    preview = null;
  }

  function remove(id: string) {
    settings.rules.list = settings.rules.list.filter((r) => r.id !== id);
    selectedId = settings.rules.list[0]?.id ?? null;
    preview = null;
  }

  // Validate the condition as it is edited.
  $effect(() => {
    const r = rule;
    if (!r) return;
    const snap = JSON.parse(JSON.stringify(r)) as Rule;
    const timer = setTimeout(() => {
      api.rulesCheck(snap).then(
        () => (error = ''),
        (e) => (error = errorText(e)),
      );
    }, 250);
    return () => clearTimeout(timer);
  });

  async function browse(field: 'folder' | 'target') {
    if (!rule) return;
    const dir = await openFileDialog({ directory: true, defaultPath: rule[field].replace(/\{.*$/, '') || home });
    if (typeof dir === 'string') rule[field] = dir;
  }

  async function run(dry: boolean) {
    if (!rule) return;
    try {
      const res = await api.rulesRun(JSON.parse(JSON.stringify(rule)), dry);
      if (dry) {
        preview = res;
        return;
      }
      preview = null;
      const ok = res.filter((a) => !a.error);
      const undo = ok.flatMap((a) => a.undo);
      if (undo.length) pushHistory({ label: t('rules.applied', { rule: rule.name }) + ` (${ok.length})`, actions: undo });
      toast(t('rules.done', { n: ok.length }));
      log = await api.rulesLog().catch(() => log);
      for (const tab of app.visibleTabs()) void tab.reload();
    } catch (e) {
      error = errorText(e);
    }
  }
</script>

<div class="rules">
  <label class="check"><input type="checkbox" bind:checked={settings.rules.enabled} /> {t('rules.enabled')}</label>
  <small class="muted">{t('rules.hint')}</small>
  <div class="cols">
    <div class="list">
      {#each settings.rules.list as r (r.id)}
        <button class="item" class:on={r.id === selectedId} onclick={() => ((selectedId = r.id), (preview = null))}>
          <input type="checkbox" bind:checked={r.enabled} onclick={(e) => e.stopPropagation()} />
          <span class="ellipsis">{r.name}</span>
          {#if r.auto}<span class="badge">auto</span>{/if}
        </button>
      {/each}
      <button class="btn add" onclick={add}><Plus size={14} /> {t('rules.add')}</button>
    </div>
    {#if rule}
      <div class="form">
        <label class="row"><span>{t('rules.name')}</span><input class="input" bind:value={rule.name} /></label>
        <label class="row">
          <span>{t('rules.folder')}</span>
          <span class="with-btn">
            <input class="input" bind:value={rule.folder} spellcheck="false" />
            <button class="icon-btn" onclick={() => browse('folder')}><FolderOpen size={15} /></button>
          </span>
        </label>
        <label class="check sub"><input type="checkbox" bind:checked={rule.recursive} /> {t('rules.recursive')}</label>
        <label class="row">
          <span>{t('rules.condition')}</span>
          <input class="input mono" class:bad={!!error} bind:value={rule.query} placeholder="ext:pdf faktura, type:image camera:iphone, size>1gb" spellcheck="false" title={t('query.help')} />
        </label>
        <label class="row">
          <span>{t('rules.action')}</span>
          <select class="input" bind:value={rule.action}>
            <option value="move">{t('rules.move')}</option>
            <option value="copy">{t('rules.copy')}</option>
            <option value="rename">{t('rules.rename')}</option>
            <option value="trash">{t('rules.trash')}</option>
          </select>
        </label>
        {#if rule.action === 'move' || rule.action === 'copy'}
          <label class="row">
            <span>{t('rules.target')}</span>
            <span class="with-btn">
              <input class="input mono" bind:value={rule.target} spellcheck="false" />
              <button class="icon-btn" onclick={() => browse('target')}><FolderOpen size={15} /></button>
            </span>
          </label>
        {/if}
        {#if rule.action !== 'trash'}
          <label class="row">
            <span>{t('rules.newNameTpl')}</span>
            <input class="input mono" bind:value={rule.rename} placeholder={rule.action === 'rename' ? '{year}-{month}-{day} {name}.{ext}' : t('rules.keepName')} spellcheck="false" />
          </label>
        {/if}
        <small class="muted tokens">{t('rules.tokens')}</small>
        <label class="check"><input type="checkbox" bind:checked={rule.auto} /> {t('rules.auto')}</label>
        {#if error}<div class="err">{error}</div>{/if}
        <div class="buttons">
          <button class="btn" onclick={() => run(true)}>{t('rules.preview')}</button>
          <button class="btn" disabled={!!error} onclick={() => run(false)}>{t('rules.runNow')}</button>
          <span class="grow"></span>
          <button class="icon-btn" title={t('rules.delete')} onclick={() => remove(rule.id)}><Trash2 size={15} /></button>
        </div>
        {#if preview}
          <div class="preview">
            {#each preview.slice(0, 50) as a, i (i)}
              <div class="pv" class:fail={!!a.error}>
                <span class="ellipsis" title={a.from}>{short(a.from)}</span>
                <span class="arrow">→</span>
                <span class="ellipsis" title={a.to ?? ''}>{a.error ?? (a.action === 'trash' ? t('rules.trash') : short(a.to))}</span>
              </div>
            {:else}
              <div class="muted">{t('rules.noMatch')}</div>
            {/each}
            {#if preview.length > 50}<div class="muted">+{preview.length - 50}</div>{/if}
          </div>
        {/if}
      </div>
    {/if}
  </div>
  {#if log.length}
    <details>
      <summary>{t('rules.log')} ({log.length})</summary>
      <div class="log">
        {#each log.slice(0, 100) as a, i (i)}
          <div class="pv" class:fail={!!a.error}>
            <span class="muted">{formatDate(a.time, locale)}</span>
            <span class="ellipsis">{a.rule}: {short(a.from)} → {a.error ?? short(a.to) ?? a.action}</span>
          </div>
        {/each}
      </div>
    </details>
  {/if}
</div>

<style>
  .rules {
    display: flex;
    flex-direction: column;
    gap: 10px;
  }
  .cols {
    display: grid;
    grid-template-columns: 170px 1fr;
    gap: 12px;
    min-height: 260px;
  }
  .list {
    display: flex;
    flex-direction: column;
    gap: 2px;
    border-right: 1px solid var(--border);
    padding-right: 8px;
  }
  .item {
    display: flex;
    align-items: center;
    gap: 6px;
    border: none;
    background: transparent;
    padding: 4px 6px;
    border-radius: var(--r-s);
    text-align: left;
    color: var(--text);
  }
  .item.on {
    background: var(--bg-hover);
  }
  .badge {
    font-size: 9px;
    padding: 0 4px;
    border-radius: 4px;
    background: var(--accent-20);
    color: var(--accent-text);
  }
  .add {
    margin-top: 6px;
    display: inline-flex;
    align-items: center;
    gap: 4px;
    justify-content: center;
  }
  .form {
    display: flex;
    flex-direction: column;
    gap: 7px;
  }
  .row {
    display: grid;
    grid-template-columns: 120px 1fr;
    align-items: center;
    gap: 8px;
  }
  .sub {
    padding-left: 128px;
  }
  .with-btn {
    display: flex;
    gap: 4px;
  }
  .with-btn .input {
    flex: 1;
  }
  .mono {
    font-family: var(--font-mono);
    font-size: 12px;
  }
  .bad {
    border-color: var(--danger);
  }
  .err,
  .fail {
    color: var(--danger);
    font-size: 12px;
  }
  .tokens {
    font-size: 11px;
    padding-left: 128px;
  }
  .buttons {
    display: flex;
    gap: 6px;
    align-items: center;
  }
  .grow {
    flex: 1;
  }
  .preview,
  .log {
    max-height: 160px;
    overflow-y: auto;
    padding: 6px 8px;
    border-radius: var(--r-m);
    background: var(--bg-subtle);
    font-size: 11px;
  }
  .pv {
    display: grid;
    grid-template-columns: 1fr auto 1fr;
    gap: 6px;
  }
  .log .pv {
    grid-template-columns: auto 1fr;
  }
  .arrow {
    color: var(--text-3);
  }
  details summary {
    cursor: pointer;
    font-size: 12px;
    color: var(--text-2);
  }
</style>
