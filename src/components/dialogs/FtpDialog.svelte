<script lang="ts">
  import FolderOpen from '@lucide/svelte/icons/folder-open';
  import Lock from '@lucide/svelte/icons/lock';
  import Plus from '@lucide/svelte/icons/plus';
  import Server from '@lucide/svelte/icons/server';
  import ShieldCheck from '@lucide/svelte/icons/shield-check';
  import { ask, open as openNative } from '@tauri-apps/plugin-dialog';
  import { onMount } from 'svelte';
  import { api } from '../../lib/api';
  import { formatDate } from '../../lib/format';
  import { localeTag, t } from '../../lib/i18n/index.svelte';
  import { isMac } from '../../lib/platform';
  import { errorText, isAppError, openDialog, toast } from '../../lib/state/ui.svelte';
  import type { Connected, FtpProtocol, SiteInput, SiteSummary, VaultStatus } from '../../lib/types';
  import Modal from './Modal.svelte';
  import VaultDialog from './VaultDialog.svelte';

  let { close }: { close: (r: (Connected & { localPath?: string }) | null) => void } = $props();

  let status = $state<VaultStatus | null>(null);
  let sites = $state<SiteSummary[]>([]);
  let selectedId = $state<string | null>(null);
  let locked = $state(false);
  let busy = $state(false);
  let message = $state<{ text: string; error: boolean } | null>(null);

  const blank = (): SiteInput & { anonymous: boolean } => ({
    id: null,
    name: '',
    group: '',
    protocol: 'ftp',
    host: '',
    port: 0,
    username: '',
    password: '',
    passive: true,
    encoding: '',
    acceptInvalidCerts: false,
    remotePath: '',
    localPath: '',
    notes: '',
    anonymous: false,
  });
  let form = $state(blank());
  let hasStoredPassword = $state(false);

  async function refresh() {
    status = await api.vaultStatus();
    try {
      sites = await api.sitesList();
      locked = false;
    } catch (e) {
      if (isAppError(e) && e.code === 'VaultLocked') locked = true;
      else message = { text: errorText(e), error: true };
    }
  }

  onMount(async () => {
    await refresh();
    if (locked && status?.protection === 'master') await unlock();
    if (sites.length) select(sites[0]);
  });

  async function unlock() {
    const ok = await openDialog<boolean>(VaultDialog, { mode: 'unlock' });
    if (ok) await refresh();
  }

  function select(s: SiteSummary) {
    selectedId = s.id;
    form = {
      id: s.id,
      name: s.name,
      group: s.group,
      protocol: s.protocol,
      host: s.host,
      port: s.port,
      username: s.username,
      password: '',
      passive: s.passive,
      encoding: s.encoding,
      acceptInvalidCerts: s.acceptInvalidCerts,
      remotePath: s.remotePath,
      localPath: s.localPath,
      notes: s.notes,
      anonymous: !s.username,
    };
    hasStoredPassword = s.hasPassword;
    message = null;
  }

  function newSite() {
    selectedId = null;
    form = blank();
    hasStoredPassword = false;
    message = null;
  }

  function input(): SiteInput {
    const { anonymous, ...rest } = form;
    return {
      ...rest,
      username: anonymous ? '' : rest.username.trim(),
      // Empty password field keeps the stored one.
      password: anonymous ? '' : rest.password ? rest.password : hasStoredPassword ? null : '',
      port: Number(rest.port) || 0,
    };
  }

  async function save(): Promise<SiteSummary | null> {
    if (!form.host.trim()) return null;
    try {
      const saved = await api.siteSave(input());
      await refresh();
      const s = sites.find((x) => x.id === saved.id);
      if (s) select(s);
      return saved;
    } catch (e) {
      message = { text: errorText(e), error: true };
      return null;
    }
  }

  async function test() {
    busy = true;
    message = { text: t('ftp.connecting'), error: false };
    try {
      const welcome = await api.ftpTest(input());
      message = { text: `${t('ftp.testOk')} ${welcome}`.trim(), error: false };
    } catch (e) {
      message = { text: errorText(e), error: true };
    } finally {
      busy = false;
    }
  }

  async function connect(saveFirst: boolean) {
    if (!form.host.trim() || busy) return;
    busy = true;
    message = { text: t('ftp.connecting'), error: false };
    try {
      let c: Connected;
      if (saveFirst && !locked) {
        const saved = await save();
        if (!saved) return;
        c = await api.ftpConnect(saved.id, null);
      } else {
        c = await api.ftpConnect(null, input());
      }
      close({ ...c, localPath: form.localPath.trim() || undefined });
    } catch (e) {
      message = { text: errorText(e), error: true };
    } finally {
      busy = false;
    }
  }

  async function remove() {
    if (!selectedId) return;
    const ok = await ask(t('ftp.deleteConfirm', { name: form.name || form.host }), { kind: 'warning', okLabel: t('delete'), cancelLabel: t('cancel') });
    if (!ok) return;
    await api.siteDelete(selectedId);
    await refresh();
    newSite();
  }

  async function setMaster() {
    const ok = await openDialog<boolean>(VaultDialog, { mode: 'set' });
    if (ok) {
      await refresh();
      toast(t('ftp.secMaster'));
    }
  }

  async function useKeychain() {
    try {
      await api.vaultSetMaster(null);
      await refresh();
    } catch (e) {
      message = { text: errorText(e), error: true };
    }
  }

  async function browseLocal() {
    const dir = await openNative({ directory: true, defaultPath: form.localPath || undefined });
    if (typeof dir === 'string') form.localPath = dir;
  }

  const defaultPort = $derived(form.protocol === 'ftpsImplicit' ? 990 : 21);
  const locale = localeTag();
  const groups = $derived.by(() => {
    const m = new Map<string, SiteSummary[]>();
    for (const s of sites) {
      const g = s.group || '';
      if (!m.has(g)) m.set(g, []);
      m.get(g)!.push(s);
    }
    return [...m.entries()];
  });
</script>

<Modal title={t('ftp.title')} width={820} onclose={() => close(null)} onsubmit={() => connect(!!selectedId)}>
  <div class="ftp">
    <aside class="sites">
      <div class="sites-head">
        <strong>{t('ftp.sites')}</strong>
        <button class="icon-btn" title={t('ftp.new')} onclick={newSite}><Plus size={15} /></button>
      </div>
      <div class="list">
        {#if locked}
          <button class="btn unlock" onclick={unlock}><Lock size={14} /> {t('vault.unlockTitle')}</button>
        {:else if !sites.length}
          <p class="muted empty">{t('ftp.empty')}</p>
        {/if}
        {#each groups as [g, list] (g)}
          {#if g}<div class="group">{g}</div>{/if}
          {#each list as s (s.id)}
            <button class="site" class:sel={s.id === selectedId} onclick={() => select(s)} ondblclick={() => connect(true)}>
              <Server size={15} strokeWidth={1.6} />
              <span class="st">
                <span class="ellipsis">{s.name}</span>
                <span class="muted ellipsis small">{s.username ? `${s.username}@` : ''}{s.host}</span>
              </span>
            </button>
          {/each}
        {/each}
      </div>
      <button class="site quick" class:sel={selectedId === null} onclick={newSite}>
        <Plus size={15} />
        <span>{t('ftp.quick')}</span>
      </button>
    </aside>

    <section class="form">
      <div class="grid">
        <label>{t('ftp.name')}<input class="input" bind:value={form.name} placeholder={form.host} /></label>
        <label>{t('ftp.group')}<input class="input" bind:value={form.group} /></label>
        <label>{t('ftp.host')}<input class="input" bind:value={form.host} placeholder="ftp.example.com" spellcheck="false" data-autofocus /></label>
        <label>{t('ftp.port')}
          <input
            class="input"
            value={form.port || ''}
            oninput={(e) => (form.port = parseInt(e.currentTarget.value, 10) || 0)}
            inputmode="numeric"
            placeholder={String(defaultPort)}
          />
        </label>
        <label>{t('ftp.protocol')}
          <select class="input" bind:value={form.protocol}>
            {#each ['ftp', 'ftpsExplicit', 'ftpsImplicit'] as const as p (p)}
              <option value={p as FtpProtocol}>{t(`ftp.proto.${p}`)}</option>
            {/each}
          </select>
        </label>
        <label>{t('ftp.encoding')}
          <select class="input" bind:value={form.encoding}>
            <option value="">UTF-8</option>
            <option value="windows-1250">Windows-1250 (CZ)</option>
            <option value="iso-8859-2">ISO-8859-2</option>
            <option value="windows-1252">Windows-1252</option>
            <option value="ibm852">CP852</option>
          </select>
        </label>
        <label>{t('ftp.user')}<input class="input" bind:value={form.username} disabled={form.anonymous} spellcheck="false" autocomplete="off" /></label>
        <label>{t('ftp.password')}
          <input class="input" type="password" bind:value={form.password} disabled={form.anonymous} placeholder={hasStoredPassword ? t('ftp.passwordStored') : ''} autocomplete="off" />
        </label>
        <div class="checks">
          <label class="check"><input type="checkbox" bind:checked={form.anonymous} /> {t('ftp.anonymous')}</label>
          <label class="check"><input type="checkbox" bind:checked={form.passive} /> {t('ftp.passive')}</label>
          {#if form.protocol !== 'ftp'}
            <label class="check"><input type="checkbox" bind:checked={form.acceptInvalidCerts} /> {t('ftp.invalidCerts')}</label>
          {/if}
        </div>
        <label>{t('ftp.remotePath')}<input class="input" bind:value={form.remotePath} placeholder="/" spellcheck="false" /></label>
        <label>{t('ftp.localPath')}
          <span class="with-btn">
            <input class="input" bind:value={form.localPath} spellcheck="false" />
            <button class="icon-btn" title={t('browse')} onclick={browseLocal}><FolderOpen size={15} /></button>
          </span>
        </label>
        <label class="full">{t('ftp.notes')}<textarea class="input notes" bind:value={form.notes} rows="2"></textarea></label>
      </div>
      {#if selectedId}
        {@const s = sites.find((x) => x.id === selectedId)}
        {#if s?.lastUsed}<div class="muted small">{t('ftp.lastUsed', { date: formatDate(s.lastUsed, locale) })}</div>{/if}
      {/if}
      {#if message}
        <div class="msg" class:error={message.error}>{message.text}</div>
      {/if}
      <div class="security">
        <ShieldCheck size={15} />
        <span>
          {#if status?.protection === 'master'}{t('ftp.secMaster')}{:else}{t(isMac ? 'ftp.secKeychainMac' : 'ftp.secKeychainWin')}{/if}
        </span>
        {#if status?.protection === 'master'}
          <button class="link" onclick={useKeychain}>{t('ftp.removeMaster')}</button>
          <button class="link" onclick={() => api.vaultLock().then(refresh)}>{t('ftp.lock')}</button>
        {:else}
          <button class="link" onclick={setMaster}>{t('ftp.setMaster')}</button>
        {/if}
      </div>
    </section>
  </div>

  {#snippet footer()}
    {#if selectedId}
      <button class="btn danger b-extra" onclick={remove}>{t('delete')}</button>
    {/if}
    <button class="btn b-extra" disabled={busy || !form.host} onclick={test}>{t('ftp.test')}</button>
    <button class="btn b-cancel" onclick={() => close(null)}>{t('cancel')}</button>
    <button class="btn b-cancel" disabled={busy || !form.host || locked} onclick={save}>{t('save')}</button>
    <button class="btn primary b-primary" disabled={busy || !form.host} onclick={() => connect(!!selectedId)}>{t('connect')}</button>
  {/snippet}
</Modal>

<style>
  .ftp {
    display: grid;
    grid-template-columns: 230px 1fr;
    gap: 16px;
    min-height: 420px;
  }
  .sites {
    display: flex;
    flex-direction: column;
    border-radius: var(--r-m);
    background: var(--bg-subtle);
    padding: 6px;
    min-height: 0;
  }
  .sites-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 2px 4px 6px 6px;
  }
  .list {
    flex: 1;
    overflow: auto;
    min-height: 0;
  }
  .group {
    font-size: 11px;
    font-weight: 700;
    color: var(--text-2);
    padding: 8px 8px 2px;
    text-transform: uppercase;
  }
  .site {
    width: 100%;
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 6px 8px;
    border: none;
    border-radius: var(--r-s);
    background: transparent;
    text-align: left;
    color: var(--text);
  }
  .site:hover {
    background: var(--bg-hover);
  }
  .site.sel {
    background: var(--cursor-bg);
    color: var(--cursor-fg);
  }
  .st {
    display: flex;
    flex-direction: column;
    min-width: 0;
  }
  .small {
    font-size: 11px;
  }
  .site.sel .muted {
    color: inherit;
    opacity: 0.8;
  }
  .quick {
    margin-top: 4px;
    border-top: 1px solid var(--border);
    border-radius: 0 0 var(--r-s) var(--r-s);
  }
  .empty {
    padding: 8px;
    font-size: 12px;
  }
  .unlock {
    width: 100%;
  }
  .form {
    display: flex;
    flex-direction: column;
    gap: 10px;
    min-width: 0;
  }
  .grid {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 8px 12px;
  }
  .grid label:not(.check) {
    display: flex;
    flex-direction: column;
    gap: 4px;
    font-size: 12px;
    color: var(--text-2);
  }
  .grid .input {
    color: var(--text);
  }
  .full,
  .checks {
    grid-column: span 2;
  }
  .checks {
    display: flex;
    flex-wrap: wrap;
    gap: 6px 18px;
    padding: 2px 0;
  }
  .notes {
    height: auto;
    padding: 6px 8px;
    resize: vertical;
    font: inherit;
    font-size: 12px;
  }
  .with-btn {
    display: flex;
    gap: 4px;
  }
  .with-btn input {
    flex: 1;
  }
  .msg {
    font-size: 12px;
    padding: 6px 10px;
    border-radius: var(--r-s);
    background: var(--bg-subtle);
    white-space: pre-line;
  }
  .msg.error {
    color: var(--danger);
  }
  .security {
    margin-top: auto;
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 6px;
    font-size: 12px;
    color: var(--text-2);
  }
  .link {
    border: none;
    background: transparent;
    color: var(--accent-text);
    padding: 0 4px;
    text-decoration: underline;
    text-underline-offset: 2px;
  }
</style>
