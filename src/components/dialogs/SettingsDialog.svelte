<script lang="ts">
  import { t } from '../../lib/i18n/index.svelte';
  import { isMac } from '../../lib/platform';
  import { app } from '../../lib/state/app.svelte';
  import { settings } from '../../lib/state/settings.svelte';
  import AboutContent from '../AboutContent.svelte';
  import Modal from './Modal.svelte';
  import ShortcutsEditor from './ShortcutsEditor.svelte';

  type Section = 'general' | 'appearance' | 'panels' | 'operations' | 'tools' | 'shortcuts' | 'aboutSection';
  let { close, section: initial = 'general' }: { close: (r: null) => void; section?: Section } = $props();

  // svelte-ignore state_referenced_locally
  let section = $state<Section>(initial);
  const sections: Section[] = ['general', 'appearance', 'panels', 'operations', 'shortcuts', 'tools', 'aboutSection'];
</script>

<Modal title={t('settings.title')} width={760} onclose={() => close(null)}>
  <div class="settings">
    <nav>
      {#each sections as s (s)}
        <button class:on={section === s} onclick={() => (section = s)}>{t(`settings.${s}`)}</button>
      {/each}
      <div class="version muted">Dualion · {t('settings.about', { version: app.info.version })}</div>
    </nav>
    <div class="pane">
      {#if section === 'general'}
        <label class="row">
          <span>{t('settings.language')}</span>
          <select class="input" bind:value={settings.lang}>
            <option value="auto">{t('settings.languageAuto')}</option>
            <option value="cs">Čeština</option>
            <option value="en">English</option>
          </select>
        </label>
        <label class="check"><input type="checkbox" bind:checked={settings.restoreSession} /> {t('settings.restoreSession')}</label>
        <label class="row">
          <span>{t('settings.typeAhead')}</span>
          <select class="input" bind:value={settings.typeAhead}>
            <option value="jump">{t('typeAhead.jump')}</option>
            <option value="filter">{t('typeAhead.filter')}</option>
          </select>
        </label>
      {:else if section === 'appearance'}
        <label class="row">
          <span>{t('settings.theme')}</span>
          <select class="input" bind:value={settings.theme}>
            <option value="system">{t('settings.themeSystem')}</option>
            <option value="light">{t('settings.themeLight')}</option>
            <option value="dark">{t('settings.themeDark')}</option>
          </select>
        </label>
        <label class="row">
          <span>{t('settings.density')}</span>
          <select class="input" bind:value={settings.density}>
            <option value="compact">{t('density.compact')}</option>
            <option value="normal">{t('density.normal')}</option>
            <option value="comfortable">{t('density.comfortable')}</option>
          </select>
        </label>
        <label class="row">
          <span>{t('settings.fontSize')}</span>
          <select class="input" bind:value={settings.fontSize}>
            <option value={0}>{t('auto')}</option>
            {#each [11, 12, 13, 14, 15] as s (s)}<option value={s}>{s} px</option>{/each}
          </select>
        </label>
        <label class="row">
          <span>{t('settings.sizeFormat')}</span>
          <select class="input" bind:value={settings.sizeFormat}>
            <option value="auto">{t('size.auto')}</option>
            <option value="binary">{t('size.binary')}</option>
            <option value="decimal">{t('size.decimal')}</option>
            <option value="bytes">{t('size.bytes')}</option>
          </select>
        </label>
        <label class="check"><input type="checkbox" bind:checked={settings.toolbar} /> {t('settings.toolbar')}</label>
        <label class="check"><input type="checkbox" bind:checked={settings.functionBar} /> {t('settings.functionBar')}</label>
      {:else if section === 'panels'}
        <label class="check"><input type="checkbox" bind:checked={settings.showHidden} /> {t('settings.showHidden')}</label>
        <label class="check"><input type="checkbox" bind:checked={settings.showUp} /> {t('settings.showUp')}</label>
        <label class="check"><input type="checkbox" bind:checked={settings.alwaysShowTabs} /> {t('settings.alwaysShowTabs')}</label>
        <label class="check"><input type="checkbox" bind:checked={settings.driveBar} /> {t('settings.driveBar')}</label>
        <label class="check"><input type="checkbox" bind:checked={settings.showAttrs} /> {t('col.attrs')}</label>
        <label class="row">
          <span>{t('settings.thumbSize')}</span>
          <select class="input" bind:value={settings.thumbSize}>
            {#each [96, 128, 160, 200, 256] as s (s)}<option value={s}>{s} px</option>{/each}
          </select>
        </label>
      {:else if section === 'operations'}
        <label class="check"><input type="checkbox" bind:checked={settings.confirmCopy} /> {t('settings.confirmCopy')}</label>
        <label class="check"><input type="checkbox" bind:checked={settings.confirmDelete} /> {t('settings.confirmDelete')}</label>
        <label class="check"><input type="checkbox" bind:checked={settings.useTrash} /> {t('settings.useTrash')}</label>
        <label class="check"><input type="checkbox" bind:checked={settings.verify} /> {t('settings.verify')}</label>
      {:else if section === 'shortcuts'}
        <ShortcutsEditor />
      {:else if section === 'aboutSection'}
        <AboutContent />
      {:else if section === 'tools'}
        <label class="col">
          <span>{t('settings.editor')}</span>
          <input class="input" bind:value={settings.editor} placeholder={isMac ? 'TextEdit, Visual Studio Code…' : 'notepad.exe, code…'} spellcheck="false" />
          <small class="muted">{t('settings.editorHint')}</small>
        </label>
        <label class="col">
          <span>{t('settings.terminal')}</span>
          <input class="input" bind:value={settings.terminal} placeholder={isMac ? 'Terminal' : 'wt.exe'} spellcheck="false" />
          <small class="muted">{t(isMac ? 'settings.terminalHintMac' : 'settings.terminalHintWin')}</small>
        </label>
      {/if}
    </div>
  </div>
  {#snippet footer()}
    <button class="btn primary b-primary" onclick={() => close(null)}>{t('close')}</button>
  {/snippet}
</Modal>

<style>
  .settings {
    display: grid;
    grid-template-columns: 170px 1fr;
    gap: 18px;
    min-height: 440px;
  }
  nav {
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  nav button {
    text-align: left;
    padding: 6px 10px;
    border: none;
    border-radius: var(--r-s);
    background: transparent;
    color: var(--text);
  }
  nav button:hover {
    background: var(--bg-hover);
  }
  nav button.on {
    background: var(--cursor-bg);
    color: var(--cursor-fg);
  }
  .version {
    margin-top: auto;
    font-size: 11px;
    padding: 6px 10px;
  }
  .pane {
    display: flex;
    flex-direction: column;
    gap: 12px;
  }
  .row {
    display: grid;
    grid-template-columns: 180px 1fr;
    align-items: center;
    gap: 10px;
  }
  .col {
    display: flex;
    flex-direction: column;
    gap: 5px;
  }
  small {
    font-size: 11px;
  }
</style>
