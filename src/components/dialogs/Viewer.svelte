<script lang="ts">
  import ExternalLink from '@lucide/svelte/icons/external-link';
  import X from '@lucide/svelte/icons/x';
  import { onMount } from 'svelte';
  import { api } from '../../lib/api';
  import { formatSize } from '../../lib/format';
  import { localeTag, t } from '../../lib/i18n/index.svelte';
  import { settings } from '../../lib/state/settings.svelte';
  import { showError } from '../../lib/state/ui.svelte';
  import type { ViewerProbe } from '../../lib/types';
  import Preview, { hasPreview } from '../preview/Preview.svelte';
  import Modal from './Modal.svelte';

  let { path, name, close }: { path: string; name: string; close: (r: null) => void } = $props();

  const CHUNK = 4 * 1024 * 1024;
  let probe = $state<ViewerProbe | null>(null);
  // Unknown until probed: no mode-specific controls may grab focus and vanish.
  let mode = $state<'text' | 'hex' | 'preview' | null>(null);
  let encoding = $state('utf-8');
  let text = $state('');
  let offset = $state(0);
  let eof = $state(true);
  let wrap = $state(true);
  let find = $state('');
  let textarea = $state<HTMLTextAreaElement>();
  let findInput = $state<HTMLInputElement>();
  let notFound = $state(false);

  onMount(async () => {
    try {
      probe = await api.viewerProbe(path);
      encoding = probe.encoding || 'utf-8';
      if (hasPreview(probe.kind)) mode = 'preview';
      else if (probe.kind === 'binary') mode = 'hex';
      else {
        mode = 'text';
        await loadText(true);
      }
    } catch (e) {
      showError(e);
      close(null);
    }
  });

  async function loadText(reset: boolean) {
    const start = reset ? 0 : offset;
    const chunk = await api.viewerReadText(path, start, CHUNK, encoding);
    text = reset ? chunk.text : text + chunk.text;
    offset = chunk.end;
    eof = chunk.eof;
  }

  async function changeEncoding() {
    await loadText(true);
  }

  function findNext(back = false) {
    if (!textarea || !find) return;
    const hay = text.toLocaleLowerCase();
    const needle = find.toLocaleLowerCase();
    const from = back ? Math.max(0, textarea.selectionStart - 1) : textarea.selectionEnd;
    let i = back ? hay.lastIndexOf(needle, from - 1) : hay.indexOf(needle, from);
    if (i < 0) i = back ? hay.lastIndexOf(needle) : hay.indexOf(needle);
    notFound = i < 0;
    if (i < 0) return;
    textarea.focus();
    textarea.setSelectionRange(i, i + needle.length);
    // Scroll the match into view (approximate by line).
    const line = text.slice(0, i).split('\n').length - 1;
    const lh = parseFloat(getComputedStyle(textarea).lineHeight) || 16;
    textarea.scrollTop = Math.max(0, line * lh - textarea.clientHeight / 3);
    findInput?.focus();
  }

  // ---------------------------------------------------------------- hex
  const BYTES = 16;
  const PAGE = 64 * 1024;
  const pages = new Map<number, Uint8Array>();
  let pageTick = $state(0);
  let hexEl = $state<HTMLDivElement>();
  let hexTop = $state(0);
  let hexH = $state(400);
  const hexRowH = 18;
  const hexRows = $derived(Math.ceil((probe?.size ?? 0) / BYTES));
  const hexStart = $derived(Math.max(0, Math.floor(hexTop / hexRowH) - 10));
  const hexEnd = $derived(Math.min(hexRows, Math.ceil((hexTop + hexH) / hexRowH) + 10));
  const loading = new Set<number>();

  function byteAt(pos: number): number | undefined {
    const p = Math.floor(pos / PAGE);
    const page = pages.get(p);
    if (!page) {
      if (!loading.has(p)) {
        loading.add(p);
        void api.viewerReadBytes(path, p * PAGE, PAGE).then((buf) => {
          pages.set(p, new Uint8Array(buf));
          loading.delete(p);
          pageTick++;
        });
      }
      return undefined;
    }
    return page[pos - p * PAGE];
  }

  function hexRow(row: number): { addr: string; hex: string; ascii: string } {
    void pageTick;
    const base = row * BYTES;
    let hex = '';
    let ascii = '';
    for (let i = 0; i < BYTES; i++) {
      const pos = base + i;
      if (pos >= (probe?.size ?? 0)) {
        hex += '   ';
        continue;
      }
      const b = byteAt(pos);
      hex += b === undefined ? '·· ' : b.toString(16).padStart(2, '0') + ' ';
      if (i === 7) hex += ' ';
      ascii += b === undefined ? ' ' : b >= 32 && b < 127 ? String.fromCharCode(b) : '.';
    }
    return { addr: base.toString(16).padStart(8, '0'), hex, ascii };
  }

  function onKey(e: KeyboardEvent) {
    if (e.key === 'F3') {
      e.preventDefault();
      close(null);
    } else if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === 'f') {
      e.preventDefault();
      findInput?.focus();
    }
  }

  const lines = $derived.by(() => {
    if (!text) return 0;
    let n = 0;
    for (let i = text.indexOf('\n'); i >= 0; i = text.indexOf('\n', i + 1)) n++;
    return text.endsWith('\n') ? n : n + 1;
  });
</script>

<!-- svelte-ignore a11y_no_static_element_interactions -->
<div onkeydown={onKey}>
  <Modal fill onclose={() => close(null)}>
    <div class="viewer">
      <header>
        <strong class="ellipsis" title={path}>{name}</strong>
        {#if probe}<span class="muted">{formatSize(probe.size, localeTag(), settings.sizeFormat)}</span>{/if}
        <div class="seg">
          {#if probe && hasPreview(probe.kind)}
            <button class:on={mode === 'preview'} onclick={() => (mode = 'preview')}>{t('pv.preview')}</button>
          {/if}
          {#if probe && (!hasPreview(probe.kind) || probe.textual)}
            <button class:on={mode === 'text'} onclick={() => { mode = 'text'; if (!text) void loadText(true); }}>{t('viewer.text')}</button>
          {/if}
          <button class:on={mode === 'hex'} onclick={() => (mode = 'hex')}>{t('viewer.hex')}</button>
        </div>
        {#if mode === 'text'}
          <select class="input enc" bind:value={encoding} onchange={changeEncoding} title={t('viewer.encoding')}>
            {#each ['utf-8', 'utf-16le', 'utf-16be', 'windows-1250', 'iso-8859-2', 'windows-1252', 'ibm852', 'koi8-r', 'shift_jis', 'gbk'] as enc (enc)}
              <option value={enc}>{enc}</option>
            {/each}
          </select>
          <label class="check"><input type="checkbox" bind:checked={wrap} /> {t('viewer.wrap')}</label>
          <input
            class="input find"
            class:nf={notFound}
            bind:this={findInput}
            bind:value={find}
            placeholder={t('viewer.find')}
            onkeydown={(e) => {
              if (e.key === 'Enter') {
                e.preventDefault();
                e.stopPropagation();
                findNext(e.shiftKey);
              }
            }}
          />
        {/if}
        <span class="grow"></span>
        <button class="icon-btn" title={t('viewer.openExternal')} onclick={() => api.openPath(path)}><ExternalLink size={16} /></button>
        <button class="icon-btn" title={t('close')} onclick={() => close(null)}><X size={16} /></button>
      </header>

      <div class="body">
        {#if mode === 'text'}
          <textarea bind:this={textarea} class="text" class:nowrap={!wrap} readonly spellcheck="false" value={text}></textarea>
          {#if !eof}
            <div class="more"><button class="btn" onclick={() => loadText(false)}>{t('viewer.more')}</button></div>
          {/if}
        {:else if mode === 'hex'}
          <div class="hex" bind:this={hexEl} bind:clientHeight={hexH} onscroll={() => (hexTop = hexEl!.scrollTop)}>
            <div style="height: {hexRows * hexRowH}px; position: relative">
              {#each { length: hexEnd - hexStart } as _, k (hexStart + k)}
                {@const r = hexRow(hexStart + k)}
                <div class="hr" style="top: {(hexStart + k) * hexRowH}px"><span class="addr">{r.addr}</span><span>{r.hex}</span><span class="ascii">{r.ascii}</span></div>
              {/each}
            </div>
          </div>
        {:else if probe}
          <Preview {path} {name} {probe} />
        {/if}
      </div>
      {#if mode === 'text'}
        <footer class="muted">{encoding} · {t('viewer.lines', { count: lines })}{notFound ? ` · ${t('viewer.notFound')}` : ''}</footer>
      {/if}
    </div>
  </Modal>
</div>

<style>
  .viewer {
    flex: 1;
    min-height: 0;
    display: flex;
    flex-direction: column;
  }
  header {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 10px 12px;
    border-bottom: 1px solid var(--border);
  }
  header strong {
    max-width: 30%;
  }
  .grow {
    flex: 1;
  }
  .seg {
    display: flex;
    gap: 2px;
    padding: 2px;
    border-radius: calc(var(--r-s) + 2px);
    background: var(--bg-subtle);
  }
  .seg button {
    border: none;
    background: transparent;
    padding: 2px 10px;
    border-radius: var(--r-s);
    color: var(--text-2);
  }
  .seg button.on {
    background: var(--bg-panel);
    color: var(--text);
    box-shadow: 0 0.5px 2px rgba(0, 0, 0, 0.15);
  }
  .enc {
    width: 130px;
  }
  .find {
    width: 180px;
  }
  .find.nf {
    border-color: var(--danger);
  }
  .body {
    flex: 1;
    min-height: 0;
    display: flex;
    flex-direction: column;
    position: relative;
    background: var(--bg-panel);
  }
  .text {
    flex: 1;
    width: 100%;
    border: none;
    outline: none;
    resize: none;
    padding: 12px 16px;
    font-family: var(--font-mono);
    font-size: 12.5px;
    line-height: 1.5;
    background: transparent;
    color: var(--text);
    tab-size: 4;
  }
  .text.nowrap {
    white-space: pre;
    overflow-x: auto;
  }
  .more {
    position: absolute;
    right: 20px;
    bottom: 16px;
  }
  .hex {
    flex: 1;
    overflow: auto;
    font-family: var(--font-mono);
    font-size: 12px;
    -webkit-user-select: text;
    user-select: text;
  }
  .hr {
    position: absolute;
    left: 0;
    height: 18px;
    line-height: 18px;
    white-space: pre;
    padding: 0 16px;
    display: flex;
    gap: 18px;
  }
  .addr {
    color: var(--text-3);
  }
  .ascii {
    color: var(--text-2);
  }
  footer {
    padding: 6px 12px;
    font-size: 11px;
    border-top: 1px solid var(--border);
  }
</style>
