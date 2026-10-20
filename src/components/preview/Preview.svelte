<script lang="ts" module>
  import type { PreviewKind } from '../../lib/types';

  /** Kinds with a native (non-text) preview. */
  export function hasPreview(kind: PreviewKind): boolean {
    return kind !== 'text' && kind !== 'binary';
  }
</script>

<script lang="ts">
  import { t } from '../../lib/i18n/index.svelte';
  import { assetUrl, dpr, thumbUrl } from '../../lib/platform';
  import type { ViewerProbe } from '../../lib/types';
  import DocView from './DocView.svelte';
  import FontView from './FontView.svelte';
  import SheetView from './SheetView.svelte';
  import SlidesView from './SlidesView.svelte';

  let {
    path,
    name,
    probe,
    compact = false,
    onfail,
  }: { path: string; name: string; probe: ViewerProbe; compact?: boolean; onfail?: () => void } = $props();

  let fit = $state(true);
  let failed = $state<string | null>(null);
  let coverFailed = $state<string | null>(null);

  // Rendered previews (PSD, RAW, Affinity…) come from the thumbnail pipeline.
  const renderPx = $derived(Math.min(4096, Math.round((compact ? 900 : Math.max(window.screen.width, window.screen.height)) * dpr())));
  const renderSrc = $derived(thumbUrl(path, renderPx, probe.mtime, true));
</script>

{#if probe.kind === 'sheet'}
  <SheetView {path} {compact} />
{:else if probe.kind === 'document'}
  <DocView {path} {compact} />
{:else if probe.kind === 'slides'}
  <SlidesView {path} mtime={probe.mtime} {compact} />
{:else if probe.kind === 'font'}
  <FontView {path} {name} {compact} />
{:else if probe.kind === 'pdf'}
  <iframe class="pdf" src={assetUrl(path)} title={name}></iframe>
{:else if probe.kind === 'video'}
  <div class="media fit">
    <!-- svelte-ignore a11y_media_has_caption -->
    <video src={assetUrl(path)} controls autoplay={!compact} preload="metadata"></video>
  </div>
{:else if probe.kind === 'audio'}
  <div class="media fit audio">
    {#if coverFailed !== path}
      <img class="cover" src={thumbUrl(path, Math.round(320 * dpr()), probe.mtime, true)} alt="" onerror={() => (coverFailed = path)} />
    {/if}
    <audio src={assetUrl(path)} controls autoplay={!compact}></audio>
  </div>
{:else if probe.kind === 'image' || probe.kind === 'render'}
  {@const src = probe.kind === 'image' ? assetUrl(path) : renderSrc}
  {#if failed === src}
    <div class="media muted">{t('pv.failed')}</div>
  {:else}
    <!-- svelte-ignore a11y_click_events_have_key_events -->
    <!-- svelte-ignore a11y_no_static_element_interactions -->
    <div class="media checker" class:fit onclick={() => !compact && (fit = !fit)}>
      <img
        {src}
        alt={name}
        onerror={() => {
          failed = src;
          onfail?.();
        }}
      />
    </div>
  {/if}
{/if}

<style>
  .media {
    flex: 1;
    min-height: 0;
    overflow: auto;
    display: flex;
    align-items: center;
    justify-content: center;
    padding: 12px;
  }
  .checker {
    background: repeating-conic-gradient(var(--bg-subtle) 0% 25%, transparent 0% 50%) 50% / 20px 20px;
  }
  .media.fit img,
  .media.fit video {
    max-width: 100%;
    max-height: 100%;
    object-fit: contain;
  }
  .media:not(.fit) {
    align-items: flex-start;
    justify-content: flex-start;
  }
  .media img {
    border-radius: 2px;
  }
  .audio {
    flex-direction: column;
    gap: 16px;
  }
  .audio .cover {
    width: 220px;
    height: 220px;
    object-fit: cover;
    border-radius: 8px;
    box-shadow: 0 4px 18px rgba(0, 0, 0, 0.25);
  }
  .pdf {
    flex: 1;
    border: none;
    width: 100%;
    min-height: 0;
  }
</style>
