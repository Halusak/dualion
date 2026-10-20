import { convertFileSrc } from '@tauri-apps/api/core';

export type Platform = 'macos' | 'windows' | 'linux';

function detect(): Platform {
  const ua = typeof navigator === 'undefined' ? '' : navigator.userAgent;
  if (/Mac OS X|Macintosh/.test(ua)) return 'macos';
  if (/Windows/.test(ua)) return 'windows';
  return 'linux';
}

export const platform: Platform = detect();
export const isMac = platform === 'macos';
export const isWindows = platform === 'windows';

/** Display form of a shortcut ("Mod+Shift+N" -> "⇧⌘N" on macOS, "Ctrl+Shift+N" elsewhere). */
export function shortcutLabel(combo: string): string {
  const parts = combo.split('+').filter(Boolean);
  const key = parts.pop() ?? '';
  const mods = new Set(parts);
  const keyName = (k: string) => {
    const map: Record<string, string> = isMac
      ? {
          ArrowUp: '↑', ArrowDown: '↓', ArrowLeft: '←', ArrowRight: '→', Enter: '↩', Backspace: '⌫', Delete: '⌦',
          Escape: 'Esc', Tab: '⇥', Space: '␣', PageUp: 'PgUp', PageDown: 'PgDn', Home: '↖', End: '↘',
        }
      : { ArrowUp: '↑', ArrowDown: '↓', ArrowLeft: '←', ArrowRight: '→', Escape: 'Esc', Space: 'Space', Delete: 'Del', Insert: 'Ins', PageUp: 'PgUp', PageDown: 'PgDn' };
    const num: Record<string, string> = { NumAdd: 'Num +', NumSub: 'Num −', NumMul: 'Num *' };
    return num[k] ?? map[k] ?? k;
  };
  if (isMac) {
    let s = '';
    if (mods.has('Ctrl')) s += '⌃';
    if (mods.has('Alt')) s += '⌥';
    if (mods.has('Shift')) s += '⇧';
    if (mods.has('Mod') || mods.has('Meta')) s += '⌘';
    return s + keyName(key);
  }
  const out: string[] = [];
  if (mods.has('Mod') || mods.has('Ctrl')) out.push('Ctrl');
  if (mods.has('Meta')) out.push('Win');
  if (mods.has('Alt')) out.push('Alt');
  if (mods.has('Shift')) out.push('Shift');
  out.push(keyName(key));
  return out.join('+');
}

function base(protocol: string): string {
  // "icon://localhost/" on macOS/Linux, "http://icon.localhost/" on Windows.
  return convertFileSrc('', protocol);
}

let iconBase: string | null = null;
let thumbBase: string | null = null;

export function extIconUrl(ext: string, px: number): string {
  iconBase ??= base('icon');
  return `${iconBase}ext/${encodeURIComponent(ext.toLowerCase())}?s=${px}`;
}

export function kindIconUrl(kind: 'dir' | 'file', px: number): string {
  iconBase ??= base('icon');
  return `${iconBase}${kind}?s=${px}`;
}

export function pathIconUrl(path: string, px: number): string {
  iconBase ??= base('icon');
  return `${iconBase}path/${encodeURIComponent(path)}?s=${px}`;
}

export function thumbUrl(path: string, px: number, mtime: number): string {
  thumbBase ??= base('thumb');
  return `${thumbBase}${encodeURIComponent(path)}?s=${px}&m=${mtime}`;
}

export function assetUrl(path: string): string {
  return convertFileSrc(path);
}

export const dpr = () => Math.min(3, Math.max(1, Math.round(window.devicePixelRatio || 1)));
