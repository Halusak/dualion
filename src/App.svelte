<script lang="ts">
  import { listen } from '@tauri-apps/api/event';
  import { getCurrentWebview } from '@tauri-apps/api/webview';
  import { getCurrentWindow } from '@tauri-apps/api/window';
  import { ask } from '@tauri-apps/plugin-dialog';
  import { onMount } from 'svelte';
  import { api } from './lib/api';
  import { buildMenu, handleKey, isTextTarget, run } from './lib/commands';
  import { dropPaths } from './lib/dnd.svelte';
  import { i18n, langFromLocale, t } from './lib/i18n/index.svelte';
  import { panelKey, quick } from './lib/panelKeys.svelte';
  import { locKey, sameLoc } from './lib/paths';
  import { isMac, platform } from './lib/platform';
  import { app, restoreSession, sessionSnapshot } from './lib/state/app.svelte';
  import { applySettings, settings, type Settings } from './lib/state/settings.svelte';
  import { dialogOpen, openDialog, ui } from './lib/state/ui.svelte';
  import { applyAccent } from './lib/theme';
  import type { Answer, JobDone, JobSnapshot } from './lib/types';
  import ConflictDialog from './components/dialogs/ConflictDialog.svelte';
  import JobErrorDialog from './components/dialogs/JobErrorDialog.svelte';
  import DragGhost from './components/DragGhost.svelte';
  import FunctionBar from './components/FunctionBar.svelte';
  import JobsPanel from './components/JobsPanel.svelte';
  import Panel from './components/Panel.svelte';
  import Toasts from './components/Toasts.svelte';
  import Toolbar from './components/Toolbar.svelte';

  const root = document.documentElement;
  root.dataset.platform = platform;

  if (import.meta.env.DEV) {
    // Test hook for driving the UI from the dev tools (stripped from production builds).
    (window as unknown as Record<string, unknown>).__dualion = { app, settings, run, ui };
  }

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

  // Native menu follows language and check-mark state.
  $effect(() => {
    if (!app.ready) return;
    const spec = buildMenu();
    void api.setMenu(spec).catch((e) => console.warn('menu', e));
  });

  // Watch the folders on screen for external changes.
  $effect(() => {
    if (!app.ready) return;
    const dirs = app
      .visibleTabs()
      .map((t) => t.location)
      .filter((l) => l.kind === 'local')
      .map((l) => (l as { path: string }).path);
    void api.watchDirs(dirs);
  });

  // Persist settings and session (debounced).
  let settingsTimer: ReturnType<typeof setTimeout>;
  $effect(() => {
    const snap = JSON.stringify(settings);
    if (!app.ready) return;
    clearTimeout(settingsTimer);
    settingsTimer = setTimeout(() => void api.storeSave('settings', JSON.parse(snap)), 400);
  });
  let sessionTimer: ReturnType<typeof setTimeout>;
  $effect(() => {
    const snap = sessionSnapshot();
    if (!app.ready) return;
    clearTimeout(sessionTimer);
    sessionTimer = setTimeout(() => void api.storeSave('session', snap), 800);
  });

  // Job prompts (conflicts, errors) open as dialogs.
  const openPrompts = new Set<number>();
  async function showPrompt(job: JobSnapshot) {
    const p = job.prompt!;
    const answer = await openDialog<Answer>(p.kind === 'conflict' ? ConflictDialog : JobErrorDialog, { job, prompt: p });
    await api.jobAnswer(job.id, answer ?? { action: 'cancel' }).catch(() => {});
  }
  $effect(() => {
    for (const j of app.jobs) {
      if (j.prompt && !openPrompts.has(j.id)) {
        openPrompts.add(j.id);
        void showPrompt(j).finally(() => setTimeout(() => openPrompts.delete(j.id), 300));
      }
    }
  });

  // Splitter
  let panelsEl: HTMLElement;
  function startSplit(e: PointerEvent) {
    const rect = panelsEl.getBoundingClientRect();
    const target = e.currentTarget as HTMLElement;
    target.setPointerCapture(e.pointerId);
    const move = (ev: PointerEvent) => {
      settings.split = Math.min(0.85, Math.max(0.15, (ev.clientX - rect.left) / rect.width));
    };
    const up = () => {
      target.removeEventListener('pointermove', move);
      target.removeEventListener('pointerup', up);
    };
    target.addEventListener('pointermove', move);
    target.addEventListener('pointerup', up);
  }

  function onKeydown(e: KeyboardEvent) {
    if (e.defaultPrevented || dialogOpen() || e.isComposing) return;
    if (isTextTarget(e.target)) {
      handleKey(e);
      return;
    }
    // While typing a quick-search name, Backspace/Esc/arrows belong to it.
    if (quick.tab === app.panel && panelKey(e, app.panel)) {
      e.preventDefault();
      return;
    }
    if (handleKey(e)) return;
    if (panelKey(e, app.panel)) {
      e.preventDefault();
    }
  }

  async function confirmQuit() {
    const ok = await ask(t('close.message'), {
      title: t('close.title'),
      kind: 'warning',
      okLabel: t('close.quit'),
      cancelLabel: t('cancel'),
    });
    if (ok) await api.exit();
  }

  onMount(() => {
    const unlisteners: Promise<() => void>[] = [];
    unlisteners.push(listen<string>('menu', (e) => run(e.payload)));
    unlisteners.push(listen<JobSnapshot[]>('jobs', (e) => (app.jobs = e.payload)));
    unlisteners.push(
      listen<JobDone>('job-done', (e) => {
        for (const tab of app.visibleTabs()) {
          if (e.payload.touched.some((l) => sameLoc(l, tab.location))) void tab.reload();
        }
      }),
    );
    unlisteners.push(
      listen<string[]>('fs-changed', (e) => {
        const keys = new Set(e.payload.map((p) => locKey({ kind: 'local', path: p })));
        for (const tab of app.visibleTabs()) if (keys.has(locKey(tab.location))) void tab.reload();
      }),
    );
    unlisteners.push(listen('close-requested', () => void confirmQuit()));
    unlisteners.push(
      getCurrentWebview().onDragDropEvent((ev) => {
        if (ev.payload.type === 'drop') dropPaths(ev.payload.paths, ev.payload.position);
      }),
    );

    (async () => {
      try {
        app.info = await api.appInfo();
        root.dataset.effects = app.info.effects;
        applySettings(await api.storeLoad<Partial<Settings>>('settings').catch(() => null));
        await restoreSession();
        app.jobs = await api.jobsList();
      } finally {
        app.ready = true;
        const w = getCurrentWindow();
        await w.show();
        await w.setFocus();
      }
    })();

    window.addEventListener('keydown', onKeydown);
    // Native look: no browser context menu outside our own handlers.
    const noMenu = (e: MouseEvent) => {
      if (!isTextTarget(e.target)) e.preventDefault();
    };
    window.addEventListener('contextmenu', noMenu);
    return () => {
      window.removeEventListener('keydown', onKeydown);
      window.removeEventListener('contextmenu', noMenu);
      for (const u of unlisteners) void u.then((f) => f());
    };
  });
</script>

<div class="shell" class:mac={isMac}>
  {#if settings.toolbar}
    <Toolbar />
  {:else if isMac}
    <div class="drag-strip" data-tauri-drag-region></div>
  {/if}
  <main class="panels" bind:this={panelsEl} style="--split: {settings.split}">
    <Panel side="left" />
    <!-- svelte-ignore a11y_no_static_element_interactions -->
    <div class="splitter" onpointerdown={startSplit} ondblclick={() => (settings.split = 0.5)}></div>
    <Panel side="right" />
  </main>
  {#if settings.functionBar}
    <FunctionBar />
  {/if}
  {#if app.showJobs}
    <JobsPanel />
  {/if}
</div>

{#each ui.dialogs as d (d.id)}
  {@const C = d.component}
  <C {...d.props} close={d.close} />
{/each}

<Toasts />
<DragGhost />

<style>
  .shell {
    height: 100%;
    display: flex;
    flex-direction: column;
    background: var(--bg-window);
  }
  .drag-strip {
    height: 28px;
    flex: none;
  }
  .panels {
    flex: 1;
    min-height: 0;
    display: grid;
    grid-template-columns: calc((100% - 5px) * var(--split)) 5px 1fr;
    padding: 0 6px;
  }
  .mac .panels {
    padding: 0 8px;
  }
  .splitter {
    cursor: col-resize;
    position: relative;
  }
  .splitter::after {
    content: '';
    position: absolute;
    inset: 30% 2px;
    border-radius: 2px;
  }
  .splitter:hover::after {
    background: var(--border-strong);
  }
</style>
