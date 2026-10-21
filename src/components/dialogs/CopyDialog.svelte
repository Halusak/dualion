<script lang="ts" module>
  import type { JobOptions, Location } from '../../lib/types';

  export interface CopyResult {
    dest: Location;
    newName: string | null;
    options: JobOptions;
    queue: number | null;
  }
</script>

<script lang="ts">
  import ChevronDown from '@lucide/svelte/icons/chevron-down';
  import ChevronRight from '@lucide/svelte/icons/chevron-right';
  import { onMount } from 'svelte';
  import { api } from '../../lib/api';
  import { localeTag, t, what } from '../../lib/i18n/index.svelte';
  import { baseName, dirName, extOf, joinRemote, joinLocal, locPath, stemOf } from '../../lib/paths';
  import { settings } from '../../lib/state/settings.svelte';
  import { formatDate, formatSize } from '../../lib/format';
  import { errorText, isAppError, showError } from '../../lib/state/ui.svelte';
  import type { ConflictPolicy, JobPlan } from '../../lib/types';
  import Modal from './Modal.svelte';

  let {
    op,
    names,
    dest,
    sameDir = false,
    extracting = false,
    upload = false,
    download = false,
    unpack = false,
    sources = [],
    close,
  }: {
    op: 'copy' | 'move';
    names: string[];
    dest: Location;
    sameDir?: boolean;
    extracting?: boolean;
    upload?: boolean;
    download?: boolean;
    unpack?: boolean;
    /** Source items, for the operation preview. */
    sources?: Location[];
    close: (r: CopyResult | null) => void;
  } = $props();

  function suggestion(name: string): string {
    const ext = extOf(name);
    const stem = ext ? stemOf(name) : name;
    return ext ? `${stem} (2).${ext}` : `${stem} (2)`;
  }

  // svelte-ignore state_referenced_locally
  const destPath = dest.kind === 'remote' ? dest.path : locPath(dest);
  // svelte-ignore state_referenced_locally
  const single = names.length === 1;
  // svelte-ignore state_referenced_locally
  const initial =
    sameDir && single ? (dest.kind === 'remote' ? joinRemote(destPath, suggestion(names[0])) : joinLocal(destPath, suggestion(names[0]))) : destPath;

  let text = $state(initial);
  // svelte-ignore state_referenced_locally
  let conflict = $state<ConflictPolicy>(sameDir && !single ? 'rename' : 'ask');
  let verify = $state(settings.verify);
  let preserveTimes = $state(true);
  let followLinks = $state(false);
  let filter = $state('');
  let limit = $state('');
  let queue = $state(1);
  let showOptions = $state(false);
  let busy = $state(false);
  let input: HTMLInputElement;

  const title = $derived(
    unpack || extracting ? t('copy.extract') : upload ? t('copy.upload') : download ? t('copy.download') : op === 'move' ? t('move.title') : t('copy.title'),
  );
  const label = $derived(
    t(unpack || extracting ? 'unpack.to' : upload && op === 'copy' ? 'upload.label' : download && op === 'copy' ? 'download.label' : op === 'move' ? 'move.label' : 'copy.label', {
      what: what(names),
    }),
  );

  onMount(() => {
    input.focus();
    if (sameDir && single) {
      const nameStart = text.length - suggestion(names[0]).length;
      const dot = text.lastIndexOf('.');
      input.setSelectionRange(nameStart, dot > nameStart ? dot : text.length);
    } else input.select();
  });

  async function resolve(quiet = false): Promise<{ dest: Location; newName: string | null } | null> {
    const typed = text.trim();
    if (!typed) return null;
    if (typed === initial && !(sameDir && single)) return { dest, newName: null };
    if (dest.kind === 'remote') {
      if (sameDir && single) return { dest: { ...dest, path: dirName(typed) || '/' }, newName: baseName(typed) };
      return { dest: { ...dest, path: typed }, newName: null };
    }
    try {
      const r = await api.resolvePath(typed, dest.kind === 'local' ? dest.path : null);
      return { dest: r.location, newName: r.focus && single ? r.focus : null };
    } catch (e) {
      if (isAppError(e) && e.code === 'NotFound') {
        if (single && !unpack) return { dest: { kind: 'local', path: dirName(typed) }, newName: baseName(typed) };
        return { dest: { kind: 'local', path: typed }, newName: null };
      }
      if (!quiet) showError(e);
      return null;
    }
  }

  // ---------------------------------------------------------------- preview (dry run)
  let plan = $state<JobPlan | null>(null);
  let planning = $state(false);
  let planError = $state('');
  let showConflicts = $state(false);
  const locale = localeTag();
  $effect(() => {
    void text;
    void filter;
    if (!sources.length || unpack || extracting) return;
    const timer = setTimeout(async () => {
      planning = true;
      try {
        const r = await resolve(true);
        if (!r) return;
        plan = await api.jobPlan({ op, sources, dest: r.dest, newName: r.newName, options: { filter: filter.trim() || null } });
        planError = '';
      } catch (e) {
        if (!(isAppError(e) && e.code === 'Cancelled')) planError = errorText(e);
      } finally {
        planning = false;
      }
    }, 350);
    return () => clearTimeout(timer);
  });
  const lowSpace = $derived(!!plan && plan.free !== null && plan.needed > plan.free);

  async function submit(toQueue: boolean) {
    if (busy) return;
    busy = true;
    try {
      const r = await resolve();
      if (!r) return;
      settings.verify = verify;
      const mbs = parseFloat(limit.replace(',', '.'));
      close({
        dest: r.dest,
        newName: r.newName,
        queue: toQueue ? queue : null,
        options: {
          conflict,
          verify,
          preserveTimes,
          followLinks,
          filter: filter.trim() || null,
          speedLimit: mbs > 0 ? Math.round(mbs * 1024 * 1024) : null,
          toTrash: false,
        },
      });
    } finally {
      busy = false;
    }
  }

  function onKey(e: KeyboardEvent) {
    // F2 = add to queue, like Total Commander.
    if (e.key === 'F2') {
      e.preventDefault();
      void submit(true);
    }
  }
</script>

<Modal onkey={onKey} {title} width={560} onclose={() => close(null)} onsubmit={() => submit(false)}>
  <label class="field">
    <span>{label}</span>
    <input class="input" bind:this={input} bind:value={text} spellcheck="false" autocomplete="off" />
  </label>

  {#if sources.length && !unpack && !extracting}
    <div class="plan" class:warn={lowSpace}>
      {#if plan}
        <div>
          {t('plan.files', { count: plan.files })}{plan.dirs ? `, ${t('plan.dirs', { count: plan.dirs })}` : ''} · {formatSize(plan.bytes, locale, settings.sizeFormat)}{plan.truncated ? '+' : ''}
          {#if plan.renameOnly}<span class="ok"> · {t('plan.renameOnly')}</span>{/if}
        </div>
        {#if plan.conflictsTotal}
          <button class="link" onclick={() => (showConflicts = !showConflicts)}>
            {t('plan.conflicts', { count: plan.conflictsTotal })}{plan.targetNewer ? ` · ${t('plan.newer', { count: plan.targetNewer })}` : ''}{plan.identical ? ` · ${t('plan.identical', { count: plan.identical })}` : ''}
          </button>
        {/if}
        {#if plan.free !== null && !plan.renameOnly}
          <div class:bad={lowSpace}>
            {lowSpace
              ? t('plan.noSpace', { need: formatSize(plan.needed, locale, settings.sizeFormat), free: formatSize(plan.free, locale, settings.sizeFormat) })
              : t('plan.space', { free: formatSize(plan.free, locale, settings.sizeFormat) })}
          </div>
        {/if}
        {#if showConflicts}
          <div class="conflicts">
            {#each plan.conflicts as c (c.path)}
              <div class="c">
                <span class="ellipsis" title={c.path}>{c.path}</span>
                <span class:newer={c.dstMtime > c.srcMtime + 2000}>{formatSize(c.srcSize, locale, settings.sizeFormat)} → {formatSize(c.dstSize, locale, settings.sizeFormat)}, {formatDate(c.dstMtime, locale)}</span>
              </div>
            {/each}
          </div>
        {/if}
      {:else if planError}
        <div class="muted">{planError}</div>
      {:else}
        <div class="muted">{t('plan.computing')}</div>
      {/if}
      {#if planning && plan}<span class="spin muted">…</span>{/if}
    </div>
  {/if}

  <button class="disclosure" onclick={() => (showOptions = !showOptions)}>
    {#if showOptions}<ChevronDown size={14} />{:else}<ChevronRight size={14} />{/if}
    {t('copy.options')}
  </button>

  {#if showOptions}
    <div class="options">
      <label class="row">
        <span>{t('copy.whenExists')}</span>
        <select class="input" bind:value={conflict}>
          <option value="ask">{t('conflict.ask')}</option>
          <option value="overwrite">{t('conflict.overwrite')}</option>
          <option value="skip">{t('conflict.skip')}</option>
          <option value="overwriteOlder">{t('conflict.overwriteOlder')}</option>
          <option value="rename">{t('conflict.rename')}</option>
        </select>
      </label>
      {#if !extracting && !unpack}
        <label class="row">
          <span>{t('copy.filter')}</span>
          <input class="input" bind:value={filter} placeholder="*.jpg;*.png" spellcheck="false" data-no-submit />
        </label>
        <label class="row">
          <span>{t('copy.speedLimit')}</span>
          <input class="input" bind:value={limit} inputmode="decimal" placeholder="∞" />
        </label>
        <div class="checks">
          <label class="check"><input type="checkbox" bind:checked={verify} /> {t('copy.verify')}</label>
          <label class="check"><input type="checkbox" bind:checked={preserveTimes} /> {t('copy.preserveTimes')}</label>
          <label class="check"><input type="checkbox" bind:checked={followLinks} /> {t('copy.followLinks')}</label>
        </div>
      {/if}
      <label class="row">
        <span>{t('copy.queueLabel')}</span>
        <select class="input" bind:value={queue}>
          {#each [1, 2, 3] as n (n)}<option value={n}>{t('copy.queueN', { n })}</option>{/each}
        </select>
      </label>
    </div>
  {/if}

  {#snippet footer()}
    <button class="btn b-extra" title="F2" onclick={() => submit(true)}>{t('copy.queue')}</button>
    <button class="btn b-cancel" onclick={() => close(null)}>{t('cancel')}</button>
    <button class="btn primary b-primary" disabled={busy} onclick={() => submit(false)}>{title}</button>
  {/snippet}
</Modal>

<style>
  .field {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  .field input {
    width: 100%;
  }
  .plan {
    position: relative;
    margin-top: 10px;
    padding: 8px 10px;
    border-radius: var(--r-m);
    background: var(--bg-subtle);
    font-size: 12px;
    display: grid;
    gap: 3px;
  }
  .plan.warn {
    box-shadow: inset 0 0 0 1px var(--danger);
  }
  .plan .ok {
    color: var(--success, #2e9d4f);
  }
  .plan .bad {
    color: var(--danger);
    font-weight: 600;
  }
  .plan .spin {
    position: absolute;
    right: 10px;
    top: 6px;
  }
  .link {
    border: none;
    background: none;
    padding: 0;
    text-align: left;
    color: var(--accent-text);
    cursor: pointer;
  }
  .conflicts {
    max-height: 140px;
    overflow-y: auto;
    margin-top: 4px;
    font-size: 11px;
  }
  .conflicts .c {
    display: grid;
    grid-template-columns: 1fr auto;
    gap: 10px;
  }
  .newer {
    color: var(--danger);
  }
  .disclosure {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    margin-top: 12px;
    padding: 2px 4px 2px 0;
    border: none;
    background: transparent;
    color: var(--text-2);
  }
  .disclosure:hover {
    color: var(--text);
  }
  .options {
    display: grid;
    gap: 8px;
    margin-top: 8px;
    padding: 12px;
    border-radius: var(--r-m);
    background: var(--bg-subtle);
  }
  .row {
    display: grid;
    grid-template-columns: 190px 1fr;
    align-items: center;
    gap: 10px;
  }
  .checks {
    display: grid;
    gap: 6px;
    padding-left: 200px;
  }
</style>
