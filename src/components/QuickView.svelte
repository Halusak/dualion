<script lang="ts">
  import { entryLoc } from '../lib/actions';
  import { api } from '../lib/api';
  import { formatDate, formatSize } from '../lib/format';
  import { localeTag, t } from '../lib/i18n/index.svelte';
  import { app } from '../lib/state/app.svelte';
  import { settings } from '../lib/state/settings.svelte';
  import { F, type DirSize, type Entry, type ViewerProbe } from '../lib/types';
  import Cloud from '@lucide/svelte/icons/cloud';
  import FileIcon from './FileIcon.svelte';
  import Preview, { hasPreview } from './preview/Preview.svelte';

  const source = $derived(app.panel);
  const entry = $derived<Entry | undefined>(source.current);

  let probe = $state<ViewerProbe | null>(null);
  let text = $state('');
  let dir = $state<DirSize | null>(null);
  let path = $state<string | null>(null);
  let failed = $state<string | null>(null);
  /** Online-only cloud file the user chose to download for previewing. */
  let allowCloud = $state<string | null>(null);
  const cloudOnly = $derived(!!entry && (entry.flags & F.CLOUD) !== 0 && allowCloud !== entry.key);
  const locale = $derived(localeTag());

  $effect(() => {
    const e = entry;
    const tab = source;
    probe = null;
    text = '';
    dir = null;
    path = null;
    if (!e || e.flags & F.UP) return;
    // Do not touch online-only cloud files until asked (reading downloads them).
    if (e.flags & F.CLOUD && allowCloud !== e.key) return;
    const loc = entryLoc(tab, e);
    if (loc.kind !== 'local') return;
    let cancelled = false;
    const timer = setTimeout(async () => {
      path = loc.path;
      if (e.isDir) {
        const r = await api.dirSize(loc.path).catch(() => null);
        if (!cancelled) dir = r;
        return;
      }
      const p = await api.viewerProbe(loc.path).catch(() => null);
      if (cancelled || !p) return;
      probe = p;
      if (p.kind === 'text') {
        const chunk = await api.viewerReadText(loc.path, 0, 64 * 1024, p.encoding).catch(() => null);
        if (!cancelled && chunk) text = chunk.text;
      }
    }, 120);
    return () => {
      cancelled = true;
      clearTimeout(timer);
    };
  });
</script>

<div class="qv">
  {#if !entry || entry.flags & F.UP}
    <div class="center muted">{t('qv.select')}</div>
  {:else if probe && path && hasPreview(probe.kind) && failed !== path}
    {@const p = path}
    <Preview path={p} name={entry.name} {probe} compact onfail={() => (failed = p)} />
  {:else if probe?.kind === 'text'}
    <pre class="text">{text}</pre>
  {:else}
    <div class="center info">
      <FileIcon {entry} location={source.location} size={96} />
      <div class="name">{entry.name}</div>
      {#if entry.isDir && dir}
        <div class="muted">{formatSize(dir.bytes, locale, settings.sizeFormat)}</div>
        <div class="muted">{t('qv.folder', { files: dir.files, dirs: dir.dirs })}</div>
      {:else if !entry.isDir}
        <div class="muted">{formatSize(entry.size, locale, settings.sizeFormat)}</div>
      {/if}
      {#if cloudOnly}
        <div class="cloud muted"><Cloud size={14} /> {t('cloud.onlineOnly')}</div>
        <button class="btn" onclick={() => entry && (allowCloud = entry.key)}>{t('cloud.download')}</button>
      {/if}
      <div class="muted">{formatDate(entry.mtime, locale, true)}</div>
    </div>
  {/if}
</div>

<style>
  .qv {
    flex: 1;
    min-height: 0;
    display: flex;
    flex-direction: column;
    background: var(--bg-panel);
  }
  .center {
    flex: 1;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 6px;
    padding: 20px;
    text-align: center;
  }
  .cloud {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    margin-top: 8px;
  }
  .info .name {
    font-weight: 600;
    font-size: 14px;
    word-break: break-all;
    margin-top: 6px;
  }
  .text {
    flex: 1;
    margin: 0;
    padding: 10px 12px;
    overflow: auto;
    font-family: var(--font-mono);
    font-size: 12px;
    line-height: 1.45;
    white-space: pre-wrap;
    word-break: break-word;
    -webkit-user-select: text;
    user-select: text;
  }
</style>
