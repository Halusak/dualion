<script lang="ts">
  import Copy from '@lucide/svelte/icons/copy';
  import ExternalLink from '@lucide/svelte/icons/external-link';
  import Heart from '@lucide/svelte/icons/heart';
  import MessageCircle from '@lucide/svelte/icons/message-circle';
  import QrCode from '@lucide/svelte/icons/qr-code';
  import icon from '../assets/app-icon.png';
  import { api } from '../lib/api';
  import { copyText } from '../lib/clipboard';
  import { t } from '../lib/i18n/index.svelte';
  import { author, crypto, links } from '../lib/links';
  import { app } from '../lib/state/app.svelte';
  import { showError, toast } from '../lib/state/ui.svelte';

  let qr = $state<Record<string, string>>({});
  let shown = $state<string | null>(null);

  async function toggleQr(id: string, uri: string) {
    if (shown === id) {
      shown = null;
      return;
    }
    if (!qr[id]) {
      try {
        // Generated locally from a fixed address – safe to inline.
        qr[id] = (await api.qrSvg(uri)).replace(/^<\?xml[^>]*>/, '');
      } catch (e) {
        showError(e);
        return;
      }
    }
    shown = id;
  }

  function open(url: string) {
    api.openUrl(url).catch(showError);
  }

  async function copy(address: string) {
    if (await copyText(address)) toast(t('about.addressCopied'));
  }
</script>

<div class="about">
  <header>
    <img src={icon} alt="" width="76" height="76" />
    <div class="title">
      <h1>Dualion</h1>
      <p class="muted">{t('settings.about', { version: app.info.version })}</p>
      <p>{t('about.tagline')}</p>
      <p class="muted small">{t('about.author')}: <strong>{author}</strong> · © 2026</p>
    </div>
  </header>

  <section>
    <h3>{t('about.community')}</h3>
    <button class="card" onclick={() => open(links.discord)}>
      <span class="badge discord"><MessageCircle size={18} strokeWidth={2} /></span>
      <span class="text">
        <strong>Discord</strong>
        <small class="muted">{t('about.discordHint')}</small>
      </span>
      <ExternalLink size={14} />
    </button>
  </section>

  <section>
    <h3>{t('about.support')}</h3>
    <p class="muted small">{t('about.supportHint')}</p>
    <div class="pay">
      <button class="card" onclick={() => open(links.revolut)}>
        <span class="badge revolut">R</span>
        <span class="text"><strong>Revolut</strong></span>
        <ExternalLink size={14} />
      </button>
      <button class="card" onclick={() => open(links.paypal)}>
        <span class="badge paypal"><Heart size={16} strokeWidth={2.2} /></span>
        <span class="text"><strong>PayPal</strong></span>
        <ExternalLink size={14} />
      </button>
    </div>
    {#each crypto as c (c.id)}
      <div class="crypto">
        <div class="row">
          <strong>{c.name}</strong>
          <code title={c.address}>{c.address}</code>
          <button class="icon-btn" title={t('about.copyAddress')} onclick={() => copy(c.address)}><Copy size={14} /></button>
          <button class="icon-btn" class:on={shown === c.id} title={t('about.qr')} onclick={() => toggleQr(c.id, c.uri)}><QrCode size={14} /></button>
        </div>
        {#if shown === c.id && qr[c.id]}
          <div class="qr">{@html qr[c.id]}</div>
        {/if}
      </div>
    {/each}
  </section>

  <p class="muted small">{t('about.tech')}</p>
</div>

<style>
  .about {
    display: flex;
    flex-direction: column;
    gap: 14px;
  }
  header {
    display: flex;
    gap: 16px;
    align-items: center;
  }
  header img {
    flex: none;
    filter: drop-shadow(0 2px 6px rgba(0, 0, 0, 0.2));
  }
  .title p {
    margin: 2px 0;
  }
  h1 {
    margin: 0;
    font-size: 22px;
  }
  h3 {
    margin: 0 0 6px;
    font-size: 12px;
    text-transform: uppercase;
    letter-spacing: 0.04em;
    color: var(--text-2);
  }
  .small {
    font-size: 11px;
    margin: 0 0 6px;
  }
  .card {
    display: flex;
    align-items: center;
    gap: 10px;
    width: 100%;
    padding: 8px 12px;
    border: 1px solid var(--border);
    border-radius: var(--r-m);
    background: var(--bg-panel);
    text-align: left;
    color: var(--text-2);
  }
  .card:hover {
    background: var(--bg-hover);
    border-color: var(--border-strong);
  }
  .card .text {
    flex: 1;
    display: flex;
    flex-direction: column;
    color: var(--text);
  }
  .badge {
    width: 30px;
    height: 30px;
    border-radius: 8px;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    color: #fff;
    font-weight: 800;
    flex: none;
  }
  .discord {
    background: #5865f2;
  }
  .revolut {
    background: #191c1f;
    font-size: 15px;
  }
  .paypal {
    background: #003087;
  }
  .pay {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 8px;
    margin-bottom: 8px;
  }
  .crypto {
    padding: 6px 4px;
  }
  .crypto + .crypto {
    border-top: 1px solid var(--border);
  }
  .crypto .row {
    display: grid;
    grid-template-columns: 110px 1fr auto auto;
    align-items: center;
    gap: 8px;
    font-size: 12px;
  }
  code {
    font-family: var(--font-mono);
    font-size: 11px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    -webkit-user-select: all;
    user-select: all;
  }
  .qr {
    display: flex;
    justify-content: center;
    padding: 10px;
  }
  .qr :global(svg) {
    width: 180px;
    height: 180px;
    border-radius: 6px;
    background: #fff;
    padding: 6px;
  }
</style>
