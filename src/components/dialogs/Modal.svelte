<script lang="ts">
  import { onMount, type Snippet } from 'svelte';

  let {
    title = '',
    width = 460,
    onclose,
    onsubmit,
    children,
    footer,
    fill = false,
  }: {
    title?: string;
    width?: number;
    onclose: () => void;
    onsubmit?: () => void;
    children: Snippet;
    footer?: Snippet;
    /** Use most of the window (viewer, search results). */
    fill?: boolean;
  } = $props();

  let dialog: HTMLDivElement;

  onMount(() => {
    // When the focused control disappears (content re-rendered), focus falls to
    // <body> and keys would go nowhere: keep Esc working and refocus the dialog.
    const stray = (e: KeyboardEvent) => {
      const a = document.activeElement;
      if (a && a !== document.body && a !== document.documentElement) return;
      const backs = document.querySelectorAll('.backdrop');
      if (backs[backs.length - 1] !== dialog.parentElement) return;
      dialog.focus();
      if (e.key === 'Escape') {
        e.preventDefault();
        e.stopPropagation();
        onclose();
      }
    };
    window.addEventListener('keydown', stray, true);
    const prev = document.activeElement as HTMLElement | null;
    const first =
      dialog.querySelector<HTMLElement>('[data-autofocus]') ??
      dialog.querySelector<HTMLElement>('input:not([type=checkbox]):not([type=radio]), select, textarea, button.primary');
    (first ?? dialog).focus();
    return () => {
      window.removeEventListener('keydown', stray, true);
      // Panels refocus themselves (the active one); restoring focus to a
      // panel here could silently switch the active side.
      if (prev && document.contains(prev) && !prev.closest('[data-side]')) prev.focus({ preventScroll: true });
    };
  });

  function focusables(): HTMLElement[] {
    return [...dialog.querySelectorAll<HTMLElement>('button, input, select, textarea, [tabindex]:not([tabindex="-1"])')].filter(
      (el) => !el.hasAttribute('disabled') && el.offsetParent !== null,
    );
  }

  function onKey(e: KeyboardEvent) {
    e.stopPropagation();
    if (e.key === 'Escape') {
      e.preventDefault();
      onclose();
    } else if (e.key === 'Enter' && onsubmit && !e.shiftKey && !e.altKey) {
      const el = e.target as HTMLElement;
      if (el.tagName === 'TEXTAREA' || el.tagName === 'BUTTON' || el.dataset.noSubmit !== undefined) return;
      e.preventDefault();
      onsubmit();
    } else if (e.key === 'Tab') {
      // Keep keyboard focus inside the dialog.
      const f = focusables();
      if (!f.length) return;
      const i = f.indexOf(document.activeElement as HTMLElement);
      if (e.shiftKey && i <= 0) {
        e.preventDefault();
        f[f.length - 1].focus();
      } else if (!e.shiftKey && i === f.length - 1) {
        e.preventDefault();
        f[0].focus();
      }
    }
  }
</script>

<div class="backdrop" class:fill>
  <!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
  <div
    class="dialog"
    class:fill
    role="dialog"
    aria-modal="true"
    aria-label={title}
    tabindex="-1"
    bind:this={dialog}
    style={fill ? '' : `width: min(${width}px, calc(100vw - 32px))`}
    onkeydown={onKey}
  >
    {#if title}
      <h2>{title}</h2>
    {/if}
    <div class="content">
      {@render children()}
    </div>
    {#if footer}
      <div class="footer">{@render footer()}</div>
    {/if}
  </div>
</div>

<style>
  .backdrop {
    position: fixed;
    inset: 0;
    z-index: 60;
    display: flex;
    align-items: flex-start;
    justify-content: center;
    background: rgba(0, 0, 0, 0.06);
  }
  :global(:root[data-theme='dark']) .backdrop {
    background: rgba(0, 0, 0, 0.22);
  }
  .dialog {
    margin-top: 44px;
    max-height: calc(100vh - 64px);
    display: flex;
    flex-direction: column;
    background: var(--bg-elevated);
    border-radius: var(--r-l);
    box-shadow: var(--shadow-dialog);
    outline: none;
    font-size: var(--fs-dialog);
    animation: sheet 0.2s cubic-bezier(0.2, 0.9, 0.3, 1);
  }
  .dialog.fill {
    margin: 36px 24px 24px;
    width: calc(100vw - 48px);
    height: calc(100vh - 60px);
    max-height: none;
  }
  h2 {
    margin: 0;
    padding: 18px 20px 4px;
    font-size: 13px;
    font-weight: 700;
  }
  .content {
    padding: 10px 20px 14px;
    overflow: auto;
    min-height: 0;
    flex: 1;
  }
  .fill .content {
    padding: 0;
    display: flex;
    flex-direction: column;
  }
  .footer {
    display: flex;
    align-items: center;
    justify-content: flex-end;
    gap: 8px;
    padding: 4px 20px 18px;
  }
  .footer :global(.b-extra) {
    order: 0;
  }
  .footer :global(.b-extra:last-of-type) {
    margin-right: auto;
  }
  .footer :global(.b-cancel) {
    order: 2;
  }
  .footer :global(.b-primary) {
    order: 3;
  }
  @keyframes sheet {
    from {
      opacity: 0;
      transform: translateY(-14px);
    }
  }

  /* Windows 11 ContentDialog */
  :global(:root[data-platform='windows']) .backdrop {
    align-items: center;
    background: rgba(0, 0, 0, 0.3);
  }
  :global(:root[data-platform='windows']) .dialog {
    margin-top: 0;
    overflow: hidden;
    animation: pop 0.16s ease-out;
  }
  :global(:root[data-platform='windows']) .dialog.fill {
    margin: 24px;
    height: calc(100vh - 48px);
  }
  :global(:root[data-platform='windows']) h2 {
    padding: 22px 24px 4px;
    font-size: 20px;
    font-weight: 600;
  }
  :global(:root[data-platform='windows']) .content {
    padding: 12px 24px 20px;
  }
  :global(:root[data-platform='windows']) .fill .content {
    padding: 0;
  }
  :global(:root[data-platform='windows']) .footer {
    padding: 20px 24px;
    background: var(--bg-window);
    border-top: 1px solid var(--border);
  }
  :global(:root[data-platform='windows'][data-effects='mica']) .footer {
    background: var(--bg-subtle);
  }
  :global(:root[data-platform='windows']) .footer :global(.b-primary) {
    order: 1;
  }
  :global(:root[data-platform='windows']) .footer :global(.b-extra) {
    order: 2;
  }
  :global(:root[data-platform='windows']) .footer :global(.b-extra:last-of-type) {
    margin-right: 0;
  }
  :global(:root[data-platform='windows']) .footer :global(.b-cancel) {
    order: 3;
  }
  @keyframes pop {
    from {
      opacity: 0;
      transform: scale(1.04);
    }
  }
</style>
