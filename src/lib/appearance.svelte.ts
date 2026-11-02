import { getCurrentWindow } from '@tauri-apps/api/window';
import { i18n, langFromLocale } from './i18n/index.svelte';
import { platform } from './platform';
import { app } from './state/app.svelte';
import { settings } from './state/settings.svelte';
import { applyAccent } from './theme';

/**
 * Theme, accent colour, density, font size and language of a window, kept
 * in sync with the settings. Call while a root component initialises.
 */
export function useAppearance() {
  const root = document.documentElement;
  root.dataset.platform = platform;

  const mq = window.matchMedia('(prefers-color-scheme: dark)');
  let systemDark = $state(mq.matches);
  mq.addEventListener('change', (e) => (systemDark = e.matches));
  const dark = $derived(settings.theme === 'dark' || (settings.theme === 'system' && systemDark));

  $effect(() => {
    root.dataset.theme = dark ? 'dark' : 'light';
    applyAccent(app.info.accent, dark);
  });
  $effect(() => {
    void getCurrentWindow()
      .setTheme(settings.theme === 'system' ? null : settings.theme)
      .catch(() => {});
  });
  $effect(() => {
    root.dataset.density = settings.density;
    root.style.setProperty('--fs', settings.fontSize > 0 ? `${settings.fontSize}px` : '');
  });
  $effect(() => {
    i18n.lang = settings.lang === 'auto' ? langFromLocale(app.info.locale) : settings.lang;
    root.lang = i18n.lang;
  });
}
