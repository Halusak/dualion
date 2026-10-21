// Connecting to remote file systems: SSH host key confirmation and the
// sources found on this computer (Android devices, containers, rclone remotes).

import { ask } from '@tauri-apps/plugin-dialog';
import { api } from './api';
import { t } from './i18n/index.svelte';
import { isAppError } from './state/ui.svelte';
import type { RemoteSource } from './types';

/**
 * Runs `connect`; when an SSH server's key is not known yet, shows its
 * fingerprint and retries with it trusted. Null when the user declines.
 */
export async function withHostKey<T>(connect: (trustKey: string | null) => Promise<T>): Promise<T | null> {
  try {
    return await connect(null);
  } catch (e) {
    if (!isAppError(e) || e.code !== 'HostKeyUnknown') throw e;
    const ok = await ask(t('remote.hostKeyAsk', { host: e.path ?? '', fingerprint: e.message }), {
      title: t('remote.hostKeyTitle'),
      kind: 'warning',
      okLabel: t('remote.hostKeyTrust'),
      cancelLabel: t('cancel'),
    });
    return ok ? connect(e.message) : null;
  }
}

let cache: { at: number; list: RemoteSource[] } | null = null;
let pending: Promise<RemoteSource[]> | null = null;

function refresh(): Promise<RemoteSource[]> {
  pending ??= api
    .remoteSources()
    .catch(() => [] as RemoteSource[])
    .then((list) => {
      cache = { at: Date.now(), list };
      pending = null;
      return list;
    });
  return pending;
}

/**
 * Found sources for the Network menu: a fresh cached list right away,
 * otherwise waits briefly (slow tools must not hold the menu up).
 */
export async function remoteSources(): Promise<RemoteSource[]> {
  if (cache && Date.now() - cache.at < 15_000) {
    void refresh();
    return cache.list;
  }
  const timeout = new Promise<RemoteSource[] | null>((r) => setTimeout(() => r(null), 2500));
  return (await Promise.race([refresh(), timeout])) ?? cache?.list ?? [];
}

/** Starts looking for sources in the background (e.g. on startup). */
export function warmSources() {
  void refresh();
}
