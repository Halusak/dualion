import CopyProgress from '../components/jobs/CopyProgress.svelte';
import { isFinished } from './jobs';
import { app } from './state/app.svelte';
import { settings } from './state/settings.svelte';
import { openDialog } from './state/ui.svelte';

const shown = new Set<number>();

/** Opens the progress window of a job (once). */
export function showProgress(id: number) {
  if (shown.has(id)) return;
  shown.add(id);
  app.showJobs = false;
  void openDialog(CopyProgress, { jobId: id }).finally(() => shown.delete(id));
}

/**
 * A job started in the foreground (F5/F6, paste, drop): show its progress
 * window unless it finishes almost at once (no flashing for small copies).
 */
export function trackJob(id: number, delay = 450) {
  if (!settings.progressDialog) return;
  setTimeout(() => {
    const j = app.jobs.find((j) => j.id === id);
    if (j && (isFinished(j) || j.queue !== null)) return;
    // A conflict / error question comes first; the progress window would cover it.
    if (j?.prompt) return trackJob(id, 300);
    showProgress(id);
  }, delay);
}
