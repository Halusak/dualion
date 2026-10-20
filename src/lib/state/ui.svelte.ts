import type { Component } from 'svelte';
import { t } from '../i18n/index.svelte';
import type { AppError } from '../types';

export interface DialogEntry {
  id: number;
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  component: Component<any>;
  props: Record<string, unknown>;
  close: (value: unknown) => void;
}

export interface Toast {
  id: number;
  text: string;
  kind: 'info' | 'error';
}

export const ui = $state<{ dialogs: DialogEntry[]; toasts: Toast[] }>({ dialogs: [], toasts: [] });

let seq = 0;

/** Opens a modal dialog; resolves with the value passed to `close` (null = cancelled). */
// eslint-disable-next-line @typescript-eslint/no-explicit-any
export function openDialog<T>(component: Component<any>, props: Record<string, unknown> = {}): Promise<T | null> {
  return new Promise((resolve) => {
    const id = ++seq;
    const close = (value: unknown) => {
      ui.dialogs = ui.dialogs.filter((d) => d.id !== id);
      resolve((value ?? null) as T | null);
    };
    ui.dialogs = [...ui.dialogs, { id, component, props, close }];
  });
}

export function dialogOpen(): boolean {
  return ui.dialogs.length > 0;
}

export function toast(text: string, kind: Toast['kind'] = 'info', ms = 3200) {
  const id = ++seq;
  ui.toasts = [...ui.toasts.slice(-3), { id, text, kind }];
  setTimeout(() => {
    ui.toasts = ui.toasts.filter((x) => x.id !== id);
  }, ms);
}

export function isAppError(e: unknown): e is AppError {
  return typeof e === 'object' && e !== null && 'code' in e && 'message' in e;
}

export function errorText(e: unknown): string {
  if (isAppError(e)) {
    const known = t(`err.${e.code}`);
    const base = known.startsWith('err.') ? e.message : known;
    const detail = e.code === 'Other' || e.code === 'Io' || e.code === 'Ftp' || e.code === 'FtpConnect' ? ` (${e.message})` : '';
    return e.path ? `${base}${detail}\n${e.path}` : `${base}${detail}`;
  }
  return String((e as Error)?.message ?? e);
}

export function showError(e: unknown) {
  if (isAppError(e) && e.code === 'Cancelled') return;
  toast(errorText(e), 'error', 6000);
}
