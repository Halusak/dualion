<script lang="ts">
  import { t } from '../../lib/i18n/index.svelte';
  import AboutContent from '../AboutContent.svelte';
  import Modal from './Modal.svelte';

  let { close }: { close: (r: null) => void } = $props();
</script>

<Modal title={t('settings.aboutSection')} width={560} onclose={() => close(null)}>
  <AboutContent />
  {#snippet footer()}
    <button class="btn primary b-primary" onclick={() => close(null)}>{t('ok')}</button>
  {/snippet}
</Modal>
