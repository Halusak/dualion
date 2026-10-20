// Mounted volumes, refreshed while something on screen shows them.

import { api } from '../api';
import type { Volume } from '../types';

export const volumes = $state<{ list: Volume[] }>({ list: [] });

let users = 0;
let timer: ReturnType<typeof setInterval> | undefined;

export async function refreshVolumes() {
  const list = await api.volumes().catch(() => null);
  if (list && JSON.stringify(list.map((v) => [v.path, v.name, v.kind])) !== JSON.stringify(volumes.list.map((v) => [v.path, v.name, v.kind]))) {
    volumes.list = list;
  } else if (list) {
    // Keep identity (no re-render) but update free space.
    list.forEach((v, i) => Object.assign(volumes.list[i], { free: v.free, total: v.total }));
  }
}

/** Starts polling for mounts / unmounts; returns a stop function. */
export function watchVolumes(): () => void {
  if (users++ === 0) {
    void refreshVolumes();
    timer = setInterval(refreshVolumes, 4000);
    window.addEventListener('focus', refreshVolumes);
  }
  return () => {
    if (--users === 0) {
      clearInterval(timer);
      window.removeEventListener('focus', refreshVolumes);
    }
  };
}

/** The volume a path lives on (longest mount point prefix). */
export function volumeOf(path: string): Volume | undefined {
  const p = path.toLowerCase().replace(/\\/g, '/');
  let best: Volume | undefined;
  for (const v of volumes.list) {
    const root = v.path.toLowerCase().replace(/\\/g, '/').replace(/\/$/, '');
    if ((p === root || p.startsWith(root + '/') || root === '') && (!best || v.path.length > best.path.length)) best = v;
  }
  return best;
}
