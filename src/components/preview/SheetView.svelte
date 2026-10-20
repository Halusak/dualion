<script lang="ts">
  import { api } from '../../lib/api';
  import { t } from '../../lib/i18n/index.svelte';
  import { errorText } from '../../lib/state/ui.svelte';
  import type { SheetPreview } from '../../lib/types';

  let { path, compact = false }: { path: string; compact?: boolean } = $props();

  let sheet = $state(0);
  let data = $state.raw<SheetPreview | null>(null);
  let error = $state('');

  $effect(() => {
    const p = path;
    const s = sheet;
    let cancelled = false;
    error = '';
    api
      .previewSheet(p, s, compact ? 300 : 3000)
      .then((d) => {
        if (!cancelled) data = d;
      })
      .catch((e) => {
        if (!cancelled) error = errorText(e);
      });
    return () => {
      cancelled = true;
    };
  });

  function colName(n: number): string {
    let s = '';
    for (n += 1; n > 0; n = Math.floor((n - 1) / 26)) s = String.fromCharCode(65 + ((n - 1) % 26)) + s;
    return s;
  }

  const cols = $derived(data ? Math.max(0, ...data.rows.map((r) => r.length)) : 0);
</script>

<div class="sheet" class:compact>
  {#if error}
    <div class="msg muted">{error}</div>
  {:else if data}
    <div class="scroll">
      <table>
        <thead>
          <tr>
            <th class="corner"></th>
            {#each { length: cols } as _, c (c)}<th>{colName(data.startCol + c)}</th>{/each}
          </tr>
        </thead>
        <tbody>
          {#each data.rows as row, r (r)}
            <tr>
              <th>{data.startRow + r + 1}</th>
              {#each { length: cols } as _, c (c)}
                {@const v = row[c] ?? ''}
                <td class:num={/^-?[\d\s.,]+$/.test(v)} title={v.length > 40 ? v : undefined}>{v}</td>
              {/each}
            </tr>
          {/each}
        </tbody>
      </table>
      {#if data.rows.length === 0}<div class="msg muted">{t('pv.empty')}</div>{/if}
    </div>
    <footer>
      {#if data.sheets.length > 1}
        <div class="tabs">
          {#each data.sheets as name, i (i)}
            <button class:on={i === data.sheet} onclick={() => (sheet = i)}>{name}</button>
          {/each}
        </div>
      {/if}
      <span class="muted info">
        {t('pv.rows', { rows: data.totalRows, cols: data.totalCols })}{data.truncated ? ` · ${t('pv.truncated')}` : ''}
      </span>
    </footer>
  {/if}
</div>

<style>
  .sheet {
    flex: 1;
    min-height: 0;
    display: flex;
    flex-direction: column;
    background: var(--bg-panel);
  }
  .scroll {
    flex: 1;
    min-height: 0;
    overflow: auto;
    -webkit-user-select: text;
    user-select: text;
  }
  table {
    border-collapse: separate;
    border-spacing: 0;
    font-size: 12px;
    font-variant-numeric: tabular-nums;
  }
  .compact table {
    font-size: 11px;
  }
  th,
  td {
    border-right: 1px solid var(--border);
    border-bottom: 1px solid var(--border);
    padding: 3px 8px;
    white-space: nowrap;
    max-width: 320px;
    overflow: hidden;
    text-overflow: ellipsis;
    text-align: left;
  }
  td.num {
    text-align: right;
  }
  thead th {
    position: sticky;
    top: 0;
    z-index: 2;
    background: var(--bg-header);
    color: var(--text-2);
    font-weight: 500;
    text-align: center;
  }
  tbody th {
    position: sticky;
    left: 0;
    z-index: 1;
    background: var(--bg-header);
    color: var(--text-2);
    font-weight: 500;
    text-align: right;
    min-width: 36px;
  }
  thead th.corner {
    left: 0;
    z-index: 3;
  }
  tbody tr:hover td {
    background: var(--bg-hover);
  }
  footer {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 4px 8px;
    border-top: 1px solid var(--border);
    font-size: 11px;
    min-height: 30px;
  }
  .tabs {
    display: flex;
    gap: 2px;
    overflow-x: auto;
    flex: 1;
    min-width: 0;
  }
  .tabs button {
    border: none;
    background: transparent;
    padding: 3px 10px;
    border-radius: var(--r-s);
    color: var(--text-2);
    white-space: nowrap;
    font-size: 12px;
  }
  .tabs button.on {
    background: var(--accent-20);
    color: var(--accent-text);
    font-weight: 600;
  }
  .info {
    margin-left: auto;
    white-space: nowrap;
  }
  .msg {
    padding: 24px;
    text-align: center;
  }
</style>
