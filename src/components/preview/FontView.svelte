<script lang="ts">
  import { t } from '../../lib/i18n/index.svelte';
  import { assetUrl } from '../../lib/platform';

  let { path, name, compact = false }: { path: string; name: string; compact?: boolean } = $props();

  let family = $state('');
  let failed = $state(false);

  $effect(() => {
    const p = path;
    const id = `dualion-font-${Math.random().toString(36).slice(2)}`;
    const face = new FontFace(id, `url("${assetUrl(p)}")`);
    let cancelled = false;
    failed = false;
    family = '';
    face
      .load()
      .then((f) => {
        if (cancelled) return;
        document.fonts.add(f);
        family = id;
      })
      .catch(() => !cancelled && (failed = true));
    return () => {
      cancelled = true;
      document.fonts.delete(face);
    };
  });

  const sizes = $derived(compact ? [14, 22, 34] : [12, 16, 24, 36, 56, 80]);
</script>

<div class="font" class:compact>
  {#if failed}
    <div class="muted">{t('pv.failed')}</div>
  {:else if family}
    <h2 style="font-family: '{family}'">{name}</h2>
    <p class="abc" style="font-family: '{family}'">ABCDEFGHIJKLMNOPQRSTUVWXYZ<br />abcdefghijklmnopqrstuvwxyz<br />ÁČĎÉĚÍŇÓŘŠŤÚŮÝŽ áčďéěíňóřšťúůýž<br />0123456789 &amp;@?!%€$(){'{}'}[]</p>
    {#each sizes as s (s)}
      <p class="sample" style="font-family: '{family}'; font-size: {s}px"><span class="sz">{s}</span>{t('pv.fontSample')}</p>
    {/each}
  {/if}
</div>

<style>
  .font {
    flex: 1;
    min-height: 0;
    overflow: auto;
    padding: 28px 36px;
    background: var(--bg-panel);
    -webkit-user-select: text;
    user-select: text;
  }
  .compact {
    padding: 14px;
  }
  h2 {
    margin: 0 0 16px;
    font-size: 30px;
    font-weight: normal;
  }
  .abc {
    font-size: 20px;
    line-height: 1.5;
    margin: 0 0 18px;
  }
  .sample {
    margin: 0 0 10px;
    line-height: 1.2;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .sz {
    display: inline-block;
    width: 34px;
    font-family: var(--font-ui);
    font-size: 10px;
    color: var(--text-3);
    vertical-align: middle;
  }
</style>
