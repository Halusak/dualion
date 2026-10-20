<script lang="ts">
  import Plus from '@lucide/svelte/icons/plus';
  import RotateCcw from '@lucide/svelte/icons/rotate-ccw';
  import X from '@lucide/svelte/icons/x';
  import { onDestroy } from 'svelte';
  import {
    assignShortcut,
    commandForCombo,
    commands,
    fullLabel,
    comboOf,
    keysOf,
    removeShortcut,
    resetShortcuts,
  } from '../../lib/commands';
  import { t } from '../../lib/i18n/index.svelte';
  import { shortcutLabel } from '../../lib/platform';
  import { settings } from '../../lib/state/settings.svelte';

  let q = $state('');
  let recording = $state<string | null>(null);
  let pending = $state<{ id: string; combo: string; other: string } | null>(null);

  const rows = $derived(
    commands
      .map((c) => ({ id: c.id, label: fullLabel(c), keys: keysOf(c), changed: c.id in settings.keymap }))
      .filter((r) => !q || (r.label + ' ' + r.keys.map(shortcutLabel).join(' ')).toLowerCase().includes(q.toLowerCase()))
      .sort((a, b) => a.label.localeCompare(b.label)),
  );

  const MODIFIERS = new Set(['Shift', 'Control', 'Alt', 'Meta', 'CapsLock', 'Fn', 'AltGraph']);

  function onKey(e: KeyboardEvent) {
    if (!recording) return;
    e.preventDefault();
    e.stopPropagation();
    if (MODIFIERS.has(e.key)) return;
    const combo = comboOf(e);
    const id = recording;
    recording = null;
    if (combo === 'Escape') return;
    const other = commandForCombo(combo);
    if (other && other.id !== id) {
      pending = { id, combo, other: fullLabel(other) };
      return;
    }
    assignShortcut(id, combo);
  }

  // Capture phase: the recorder must see keys before the dialog handles Esc/Enter.
  window.addEventListener('keydown', onKey, true);
  onDestroy(() => window.removeEventListener('keydown', onKey, true));

  function confirmPending() {
    if (pending) assignShortcut(pending.id, pending.combo);
    pending = null;
  }
</script>

<div class="keys">
  <div class="top">
    <input class="input" bind:value={q} placeholder={t('help.filter')} spellcheck="false" />
    <button class="btn" disabled={Object.keys(settings.keymap).length === 0} onclick={() => resetShortcuts()}>
      {t('keys.resetAll')}
    </button>
  </div>
  <p class="hint muted">{t('keys.hint')}</p>
  {#if pending}
    <div class="conflict">
      <span>{t('keys.conflict', { combo: shortcutLabel(pending.combo), command: pending.other })}</span>
      <button class="btn small primary" onclick={confirmPending}>{t('keys.reassign')}</button>
      <button class="btn small" onclick={() => (pending = null)}>{t('cancel')}</button>
    </div>
  {/if}
  <div class="list">
    {#each rows as r (r.id)}
      <div class="row" class:changed={r.changed}>
        <span class="label ellipsis" title={r.label}>{r.label}</span>
        <span class="chips">
          {#each r.keys as k (k)}
            <span class="chip">
              {shortcutLabel(k)}
              <button class="x" title={t('remove')} aria-label={t('remove')} onclick={() => removeShortcut(r.id, k)}><X size={10} /></button>
            </span>
          {/each}
          {#if recording === r.id}
            <span class="chip rec">{t('keys.press')}</span>
          {/if}
        </span>
        <span class="actions">
          <button class="icon-btn" title={t('keys.add')} onclick={() => (recording = recording === r.id ? null : r.id)}><Plus size={14} /></button>
          <button
            class="icon-btn"
            title={t('keys.reset')}
            disabled={!r.changed}
            onclick={() => resetShortcuts(r.id)}
          >
            <RotateCcw size={13} />
          </button>
        </span>
      </div>
    {/each}
  </div>
</div>

<style>
  .keys {
    display: flex;
    flex-direction: column;
    gap: 8px;
    min-height: 0;
  }
  .top {
    display: flex;
    gap: 8px;
  }
  .top .input {
    flex: 1;
  }
  .hint {
    margin: 0;
    font-size: 11px;
  }
  .conflict {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 8px 10px;
    border-radius: var(--r-m);
    background: var(--accent-12);
    font-size: 12px;
  }
  .conflict span {
    flex: 1;
  }
  .list {
    max-height: 360px;
    overflow: auto;
    border: 1px solid var(--border);
    border-radius: var(--r-m);
  }
  .row {
    display: grid;
    grid-template-columns: minmax(140px, 1fr) minmax(120px, 1.2fr) auto;
    align-items: center;
    gap: 8px;
    padding: 3px 6px 3px 10px;
    min-height: 32px;
  }
  .row:nth-child(even) {
    background: var(--bg-subtle);
  }
  .row.changed .label {
    font-weight: 600;
  }
  .chips {
    display: flex;
    flex-wrap: wrap;
    gap: 4px;
  }
  .chip {
    display: inline-flex;
    align-items: center;
    gap: 2px;
    padding: 1px 2px 1px 7px;
    border-radius: 5px;
    background: var(--bg-panel);
    border: 1px solid var(--border);
    font-size: 12px;
    font-weight: 600;
    color: var(--accent-text);
    white-space: nowrap;
  }
  .chip.rec {
    padding-right: 7px;
    border-color: var(--accent);
    animation: blink 1s ease-in-out infinite;
  }
  @keyframes blink {
    50% {
      opacity: 0.45;
    }
  }
  .x {
    border: none;
    background: transparent;
    color: var(--text-3);
    display: inline-flex;
    padding: 2px;
    border-radius: 3px;
  }
  .x:hover {
    color: var(--danger);
    background: var(--bg-hover);
  }
  .actions {
    display: flex;
    gap: 2px;
  }
  .actions .icon-btn {
    width: 24px;
    height: 24px;
  }
</style>
