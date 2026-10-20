// Archive passwords and disk images.

import { ask } from '@tauri-apps/plugin-dialog';
import PromptDialog, { type PromptResult } from '../components/dialogs/PromptDialog.svelte';
import { api } from './api';
import { t } from './i18n/index.svelte';
import { baseName, isDiskImage } from './paths';
import { isAppError, openDialog, showError } from './state/ui.svelte';

/** Asks for an archive password until it is right (or the user cancels). */
export async function askArchivePassword(archive: string, wrong = false): Promise<boolean> {
  for (;;) {
    const r = await openDialog<PromptResult>(PromptDialog, {
      title: t('arc.pwTitle'),
      label: t('arc.pwLabel', { name: baseName(archive) }),
      password: true,
      hint: wrong ? t('err.WrongPassword') : '',
    });
    if (!r) return false;
    try {
      await api.archivePassword(archive, r.value);
      return true;
    } catch (e) {
      if (isAppError(e) && (e.code === 'WrongPassword' || e.code === 'PasswordRequired')) {
        wrong = true;
        continue;
      }
      showError(e);
      return false;
    }
  }
}

/**
 * Handles errors of opening an archive: asks for the password, offers to
 * mount UDF images. Returns true when the caller should try again.
 */
export async function recoverArchive(archive: string, err: unknown): Promise<boolean> {
  if (!isAppError(err)) return false;
  if (err.code === 'PasswordRequired' || err.code === 'WrongPassword') return askArchivePassword(archive, err.code === 'WrongPassword');
  if (err.code === 'Unsupported' && isDiskImage(archive)) {
    if (await ask(t('arc.mountAsk', { name: baseName(archive) }), { title: 'Dualion', okLabel: t('arc.mount'), cancelLabel: t('cancel') })) {
      await mount(archive);
    }
    return false;
  }
  return false;
}

/** Makes sure the content of an archive can be read before extracting. */
export async function ensureArchiveAccess(archive: string): Promise<boolean> {
  for (let i = 0; i < 2; i++) {
    try {
      await api.archiveCheck(archive);
      return true;
    } catch (e) {
      if (!(await recoverArchive(archive, e))) {
        if (isAppError(e) && !['PasswordRequired', 'WrongPassword', 'Unsupported'].includes(e.code)) showError(e);
        return false;
      }
    }
  }
  return true;
}

/** Mounts a disk image with the system and opens it in the active panel. */
export async function mount(path: string) {
  const { app } = await import('./state/app.svelte');
  try {
    const at = await api.mountImage(path);
    await app.panel.navigate({ kind: 'local', path: at });
  } catch (e) {
    showError(e);
  }
}
