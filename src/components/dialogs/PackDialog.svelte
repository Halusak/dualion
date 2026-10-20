<script lang="ts" module>
  export interface PackResult {
    path: string;
    level: number;
    queue: number | null;
    format: string;
    password: string | null;
    encryptNames: boolean;
  }

  export const PACK_FORMATS = ['zip', '7z', 'tar', 'tar.gz', 'tar.bz2', 'tar.xz', 'tar.zst'] as const;
  const ENCRYPTABLE = new Set(['zip', '7z']);

  /** Name without a known archive extension ("a.tar.gz" -> "a"). */
  function stripFormat(path: string): string {
    for (const f of [...PACK_FORMATS].sort((a, b) => b.length - a.length)) {
      if (path.toLowerCase().endsWith(`.${f}`)) return path.slice(0, -(f.length + 1));
    }
    return path;
  }
</script>

<script lang="ts">
  import { onMount } from 'svelte';
  import { t, what } from '../../lib/i18n/index.svelte';
  import Modal from './Modal.svelte';

  let { names, path, close }: { names: string[]; path: string; close: (r: PackResult | null) => void } = $props();

  // svelte-ignore state_referenced_locally
  let target = $state(path);
  let format = $state<string>('zip');
  let level = $state(6);
  let password = $state('');
  let confirm = $state('');
  let encryptNames = $state(true);
  let usePassword = $state(false);
  let input: HTMLInputElement;

  const canEncrypt = $derived(ENCRYPTABLE.has(format));
  const mismatch = $derived(usePassword && canEncrypt && password !== confirm);
  const empty = $derived(usePassword && canEncrypt && !password);

  onMount(() => {
    input.focus();
    const slash = Math.max(target.lastIndexOf('/'), target.lastIndexOf('\\'));
    const stem = stripFormat(target);
    input.setSelectionRange(slash + 1, stem.length > slash ? stem.length : target.length);
  });

  // Keep the extension in sync with the chosen format.
  function setFormat(f: string) {
    format = f;
    target = `${stripFormat(target.trim())}.${f}`;
  }

  function submit(queue: number | null) {
    let p = target.trim();
    if (!p || mismatch || empty) return;
    if (!p.toLowerCase().endsWith(`.${format}`)) p = `${stripFormat(p)}.${format}`;
    close({ path: p, level, queue, format, password: usePassword && canEncrypt ? password : null, encryptNames: format === '7z' && encryptNames });
  }
</script>

<Modal title={t('pack.title')} width={560} onclose={() => close(null)} onsubmit={() => submit(null)}>
  <p class="muted what">{what(names)}</p>
  <label class="field">
    <span>{t('pack.archive')}</span>
    <input class="input" bind:this={input} bind:value={target} spellcheck="false" />
  </label>
  <div class="row2">
    <label class="field">
      <span>{t('pack.format')}</span>
      <select class="input" value={format} onchange={(e) => setFormat(e.currentTarget.value)}>
        {#each PACK_FORMATS as f (f)}<option value={f}>{f.toUpperCase()}</option>{/each}
      </select>
    </label>
    <label class="field">
      <span>{t('pack.level')}</span>
      <select class="input" bind:value={level}>
        <option value={0}>{t('level.store')}</option>
        <option value={1}>{t('level.fast')}</option>
        <option value={6}>{t('level.normal')}</option>
        <option value={9}>{t('level.best')}</option>
      </select>
    </label>
  </div>
  <label class="check pw" class:off={!canEncrypt} title={canEncrypt ? '' : t('pack.noPassword')}>
    <input type="checkbox" bind:checked={usePassword} disabled={!canEncrypt} /> {t('pack.usePassword')}
    {#if !canEncrypt}<span class="muted small">({t('pack.noPassword')})</span>{/if}
  </label>
  {#if usePassword && canEncrypt}
    <div class="row2">
      <label class="field"><span>{t('pack.password')}</span><input class="input" type="password" bind:value={password} autocomplete="new-password" data-no-submit /></label>
      <label class="field"><span>{t('pack.confirm')}</span><input class="input" type="password" bind:value={confirm} autocomplete="new-password" class:bad={mismatch && confirm.length > 0} /></label>
    </div>
    {#if format === '7z'}
      <label class="check"><input type="checkbox" bind:checked={encryptNames} /> {t('pack.encryptNames')}</label>
    {/if}
    <p class="muted small">{t(format === 'zip' ? 'pack.aesZip' : 'pack.aes7z')}</p>
  {/if}
  {#snippet footer()}
    <button class="btn b-extra" onclick={() => submit(1)}>{t('copy.queue')}</button>
    <button class="btn b-cancel" onclick={() => close(null)}>{t('cancel')}</button>
    <button class="btn primary b-primary" disabled={mismatch || empty} onclick={() => submit(null)}>{t('cmd.pack').replace('…', '')}</button>
  {/snippet}
</Modal>

<style>
  .what {
    margin: 0 0 10px;
  }
  .field {
    display: flex;
    flex-direction: column;
    gap: 5px;
    margin-top: 8px;
  }
  .row2 {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 12px;
  }
  .pw {
    margin-top: 14px;
  }
  .off {
    opacity: 0.6;
  }
  .small {
    font-size: 11px;
    margin: 6px 0 0;
  }
  .bad {
    border-color: var(--danger);
  }
</style>
