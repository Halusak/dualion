<script lang="ts">
  import { api } from '../../lib/api';
  import { t } from '../../lib/i18n/index.svelte';
  import { errorText } from '../../lib/state/ui.svelte';
  import type { DocumentPreview } from '../../lib/types';

  let { path, compact = false }: { path: string; compact?: boolean } = $props();

  let doc = $state.raw<DocumentPreview | null>(null);
  let error = $state('');

  $effect(() => {
    const p = path;
    let cancelled = false;
    doc = null;
    error = '';
    api
      .previewDocument(p)
      .then((d) => !cancelled && (doc = d))
      .catch((e) => !cancelled && (error = errorText(e)));
    return () => {
      cancelled = true;
    };
  });
</script>

<div class="wrap" class:compact>
  {#if error}
    <div class="msg muted">{error}</div>
  {:else if doc}
    <article class="page">
      {#each doc.blocks as b, i (i)}
        {#if b.kind === 'h1'}<h1>{b.text}</h1>
        {:else if b.kind === 'h2'}<h2>{b.text}</h2>
        {:else if b.kind === 'h3'}<h3>{b.text}</h3>
        {:else if b.kind === 'li'}<p class="li">{b.text}</p>
        {:else if b.kind === 'table' && b.rows}
          <table>
            <tbody>
              {#each b.rows as row, r (r)}
                <tr>{#each row as cell, c (c)}<td>{cell}</td>{/each}</tr>
              {/each}
            </tbody>
          </table>
        {:else}<p>{b.text}</p>{/if}
      {/each}
      {#if doc.blocks.length === 0}<p class="muted">{t('pv.empty')}</p>{/if}
      {#if doc.truncated}<p class="muted note">{t('pv.truncated')}</p>{/if}
    </article>
  {/if}
</div>

<style>
  .wrap {
    flex: 1;
    min-height: 0;
    overflow: auto;
    padding: 24px 16px;
    background: var(--bg-subtle);
  }
  .compact {
    padding: 10px;
  }
  .page {
    max-width: 820px;
    margin: 0 auto;
    padding: 48px 56px;
    background: var(--bg-panel);
    border-radius: 4px;
    box-shadow:
      0 1px 3px rgba(0, 0, 0, 0.12),
      0 0 0 1px var(--border);
    font-size: 14px;
    line-height: 1.55;
    -webkit-user-select: text;
    user-select: text;
    white-space: pre-wrap;
    word-break: break-word;
  }
  .compact .page {
    padding: 18px 20px;
    font-size: 12px;
  }
  h1 {
    font-size: 1.7em;
    margin: 0.4em 0 0.5em;
  }
  h2 {
    font-size: 1.35em;
    margin: 0.9em 0 0.4em;
  }
  h3 {
    font-size: 1.12em;
    margin: 0.8em 0 0.3em;
  }
  p {
    margin: 0 0 0.6em;
  }
  .li {
    padding-left: 1.4em;
    position: relative;
  }
  .li::before {
    content: '•';
    position: absolute;
    left: 0.4em;
  }
  table {
    border-collapse: collapse;
    margin: 0.6em 0 1em;
    font-size: 0.92em;
  }
  td {
    border: 1px solid var(--border-strong);
    padding: 4px 8px;
    vertical-align: top;
  }
  .note {
    margin-top: 1.5em;
    font-style: italic;
  }
  .msg {
    padding: 24px;
    text-align: center;
  }
</style>
