<script lang="ts">
  import { api } from '../../lib/api';
  import { t } from '../../lib/i18n/index.svelte';
  import { dpr, thumbUrl } from '../../lib/platform';
  import { errorText } from '../../lib/state/ui.svelte';
  import type { SlidePreview } from '../../lib/types';

  let { path, mtime, compact = false }: { path: string; mtime: number; compact?: boolean } = $props();

  let slides = $state.raw<SlidePreview[] | null>(null);
  let error = $state('');
  let coverFailed = $state(false);

  $effect(() => {
    const p = path;
    let cancelled = false;
    slides = null;
    error = '';
    coverFailed = false;
    api
      .previewSlides(p)
      .then((s) => !cancelled && (slides = s))
      .catch((e) => !cancelled && (error = errorText(e)));
    return () => {
      cancelled = true;
    };
  });
</script>

<div class="wrap" class:compact>
  {#if !coverFailed}
    <img class="cover" src={thumbUrl(path, Math.round((compact ? 480 : 1100) * dpr()), mtime, true)} alt="" onerror={() => (coverFailed = true)} />
  {/if}
  {#if error}
    <div class="msg muted">{error}</div>
  {:else if slides}
    {#each slides as s, i (i)}
      <section class="slide">
        <span class="n">{t('pv.slide', { n: i + 1 })}</span>
        {#if s.title}<h3>{s.title}</h3>{/if}
        {#each s.lines as line, j (j)}<p>{line}</p>{/each}
      </section>
    {/each}
    {#if slides.length === 0}<div class="msg muted">{t('pv.empty')}</div>{/if}
  {/if}
</div>

<style>
  .wrap {
    flex: 1;
    min-height: 0;
    overflow: auto;
    padding: 20px;
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 12px;
    background: var(--bg-subtle);
  }
  .compact {
    padding: 10px;
    gap: 8px;
  }
  .cover {
    max-width: min(100%, 900px);
    border-radius: 6px;
    box-shadow: 0 2px 10px rgba(0, 0, 0, 0.18);
  }
  .slide {
    width: min(100%, 900px);
    padding: 16px 22px;
    border-radius: 8px;
    background: var(--bg-panel);
    box-shadow: 0 0 0 1px var(--border);
    -webkit-user-select: text;
    user-select: text;
  }
  .n {
    font-size: 11px;
    color: var(--text-2);
  }
  h3 {
    margin: 4px 0 8px;
    font-size: 17px;
  }
  .compact h3 {
    font-size: 14px;
  }
  p {
    margin: 0 0 4px;
    white-space: pre-wrap;
  }
  .msg {
    padding: 24px;
    text-align: center;
  }
</style>
