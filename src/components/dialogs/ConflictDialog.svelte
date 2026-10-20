<script lang="ts">
  import { formatDate, formatSize } from '../../lib/format';
  import { localeTag, t } from '../../lib/i18n/index.svelte';
  import { baseName, extOf } from '../../lib/paths';
  import { settings } from '../../lib/state/settings.svelte';
  import { F, type Answer, type Entry, type JobSnapshot, type Prompt } from '../../lib/types';
  import FileIcon from '../FileIcon.svelte';
  import Modal from './Modal.svelte';

  let { prompt, close }: { job: JobSnapshot; prompt: Extract<Prompt, { kind: 'conflict' }>; close: (a: Answer | null) => void } = $props();

  const locale = localeTag();
  let all = $state(false);
  // svelte-ignore state_referenced_locally
  let newName = $state(prompt.suggested);
  // svelte-ignore state_referenced_locally
  const name = baseName(prompt.target.path);

  // svelte-ignore state_referenced_locally
  const cmp = prompt.source.mtime - prompt.target.mtime;
  const srcAge = Math.abs(cmp) < 2000 ? 'same' : cmp > 0 ? 'newer' : 'older';
  const dstAge = srcAge === 'same' ? 'same' : srcAge === 'newer' ? 'older' : 'newer';

  function fake(path: string, isDir: boolean): Entry {
    const n = baseName(path);
    return { name: n, flags: isDir ? F.DIR : 0, size: 0, mtime: 0, attrs: 0, ext: isDir ? '' : extOf(n), isDir, key: n, path };
  }
</script>

<Modal title={t('conflict.title')} width={640} onclose={() => close({ action: 'cancel' })} onsubmit={() => close({ action: 'overwrite', all })}>
  <p class="msg">{t('conflict.message', { name })}</p>
  <div class="cmp">
    {#each [{ f: prompt.source, label: t('conflict.source'), age: srcAge }, { f: prompt.target, label: t('conflict.target'), age: dstAge }] as side (side.label)}
      <div class="card">
        <FileIcon entry={fake(side.f.path, side.f.isDir)} location={{ kind: 'drives' }} size={40} />
        <div class="info">
          <strong>{side.label}</strong>
          <span class="muted ellipsis" title={side.f.path}>{side.f.path}</span>
          <span>{formatSize(side.f.size, locale, settings.sizeFormat)} · {formatDate(side.f.mtime, locale, true)}</span>
          <span class="age {side.age}">{t(`conflict.${side.age}`)}</span>
        </div>
      </div>
    {/each}
  </div>
  <label class="rename">
    <span class="muted">{t('conflict.renameTo')}</span>
    <input
      class="input"
      bind:value={newName}
      spellcheck="false"
      data-no-submit
      onkeydown={(e) => {
        if (e.key === 'Enter') {
          e.preventDefault();
          close({ action: 'rename', name: newName, all });
        }
      }}
    />
  </label>
  <label class="check all"><input type="checkbox" bind:checked={all} /> {t('conflict.applyAll')}</label>

  {#snippet footer()}
    <button class="btn b-extra" onclick={() => close({ action: 'overwriteOlder' })}>{t('conflict.btnOlder')}</button>
    <button class="btn b-extra" onclick={() => close({ action: 'cancel' })}>{t('conflict.cancel')}</button>
    <button class="btn b-cancel" onclick={() => close({ action: 'skip', all })}>{t('conflict.btnSkip')}</button>
    <button class="btn b-cancel" onclick={() => close({ action: 'rename', name: newName, all })}>{t('conflict.btnRename')}</button>
    <button class="btn primary b-primary" data-autofocus onclick={() => close({ action: 'overwrite', all })}>{t('conflict.btnOverwrite')}</button>
  {/snippet}
</Modal>

<style>
  .msg {
    margin: 0 0 12px;
  }
  .cmp {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 10px;
  }
  .card {
    display: flex;
    gap: 10px;
    padding: 10px;
    border-radius: var(--r-m);
    background: var(--bg-subtle);
    min-width: 0;
  }
  .info {
    display: flex;
    flex-direction: column;
    gap: 2px;
    min-width: 0;
    font-size: 12px;
  }
  .age {
    font-weight: 600;
  }
  .age.newer {
    color: var(--ok);
  }
  .age.older {
    color: var(--warn);
  }
  .rename {
    display: flex;
    flex-direction: column;
    gap: 4px;
    margin-top: 12px;
  }
  .all {
    margin-top: 10px;
  }
</style>
