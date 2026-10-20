<script lang="ts">
  import KeyRound from '@lucide/svelte/icons/key-round';
  import { api } from '../../lib/api';
  import { t } from '../../lib/i18n/index.svelte';
  import { errorText, isAppError } from '../../lib/state/ui.svelte';
  import Modal from './Modal.svelte';

  let { mode, close }: { mode: 'unlock' | 'set'; close: (ok: boolean | null) => void } = $props();

  let password = $state('');
  let confirm = $state('');
  let error = $state('');
  let busy = $state(false);

  async function submit() {
    if (busy) return;
    error = '';
    if (mode === 'set') {
      if (password.length < 8) return void (error = t('vault.tooShort'));
      if (password !== confirm) return void (error = t('vault.mismatch'));
    }
    busy = true;
    try {
      if (mode === 'unlock') await api.vaultUnlock(password);
      else await api.vaultSetMaster(password);
      close(true);
    } catch (e) {
      error = isAppError(e) && e.code === 'VaultWrongPassword' ? t('err.VaultWrongPassword') : errorText(e);
    } finally {
      busy = false;
    }
  }
</script>

<Modal title={mode === 'unlock' ? t('vault.unlockTitle') : t('vault.setTitle')} width={420} onclose={() => close(null)} onsubmit={submit}>
  <div class="body">
    <span class="icon"><KeyRound size={30} strokeWidth={1.5} /></span>
    <div class="fields">
      <p>{mode === 'unlock' ? t('vault.unlockMessage') : t('vault.setMessage')}</p>
      <input class="input" type="password" bind:value={password} placeholder={t('vault.password')} data-autofocus />
      {#if mode === 'set'}
        <input class="input" type="password" bind:value={confirm} placeholder={t('vault.confirm')} />
      {/if}
      {#if error}<div class="error">{error}</div>{/if}
    </div>
  </div>
  {#snippet footer()}
    <button class="btn b-cancel" onclick={() => close(null)}>{t('cancel')}</button>
    <button class="btn primary b-primary" disabled={busy} onclick={submit}>{t('ok')}</button>
  {/snippet}
</Modal>

<style>
  .body {
    display: flex;
    gap: 14px;
  }
  .icon {
    color: var(--accent);
  }
  .fields {
    flex: 1;
    display: grid;
    gap: 8px;
  }
  p {
    margin: 0;
    color: var(--text-2);
  }
  .error {
    color: var(--danger);
    font-size: 12px;
  }
</style>
