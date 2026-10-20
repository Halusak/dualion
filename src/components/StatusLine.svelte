<script lang="ts">
  import { formatSize } from '../lib/format';
  import { localeTag, t } from '../lib/i18n/index.svelte';
  import { settings } from '../lib/state/settings.svelte';
  import type { Tab } from '../lib/state/tab.svelte';
  import { F } from '../lib/types';

  let { tab }: { tab: Tab } = $props();

  const locale = $derived(localeTag());
  const counts = $derived.by(() => {
    let dirs = 0;
    let files = 0;
    let bytes = 0;
    for (const e of tab.visible) {
      if (e.flags & F.UP) continue;
      if (e.isDir) dirs++;
      else {
        files++;
        bytes += e.size;
      }
    }
    return { dirs, files, bytes };
  });
  const sel = $derived(tab.marked.size ? tab.markedSummary() : null);
</script>

<footer class="status">
  <span class="ellipsis">
    {#if sel}
      {t('status.selected', {
        sel: sel.count,
        total: counts.dirs + counts.files,
        size: formatSize(sel.bytes, locale, settings.sizeFormat),
      })}
    {:else}
      {t('status.folders', { count: counts.dirs })}, {t('status.files', { count: counts.files })}
      · {formatSize(counts.bytes, locale, settings.sizeFormat)}
    {/if}
  </span>
  {#if tab.space}
    <span class="free">
      <span class="meter"><span style="width: {tab.space.total ? (100 * (tab.space.total - tab.space.free)) / tab.space.total : 0}%"></span></span>
      {t('status.free', {
        free: formatSize(tab.space.free, locale, settings.sizeFormat),
        total: formatSize(tab.space.total, locale, settings.sizeFormat),
      })}
    </span>
  {/if}
</footer>

<style>
  .status {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 12px;
    height: 24px;
    flex: none;
    padding: 0 10px;
    font-size: var(--fs-small);
    color: var(--text-2);
    border-top: 1px solid var(--border);
    background: var(--bg-header);
    font-variant-numeric: tabular-nums;
  }
  .free {
    display: flex;
    align-items: center;
    gap: 6px;
    white-space: nowrap;
  }
  .meter {
    width: 54px;
    height: 5px;
    border-radius: 3px;
    background: var(--border);
    overflow: hidden;
  }
  .meter span {
    display: block;
    height: 100%;
    background: var(--accent);
  }
</style>
