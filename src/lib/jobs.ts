import { t } from './i18n/index.svelte';
import type { JobSnapshot } from './types';

export const isFinished = (j: JobSnapshot) => j.status === 'done' || j.status === 'failed' || j.status === 'cancelled';

/** "Copying report.pdf", "Moving 12 items". */
export function jobTitle(j: JobSnapshot): string {
  const op = j.op === 'delete' && j.toTrash ? t('jobs.op.trash') : t(`jobs.op.${j.op}`);
  const what = j.sourceCount === 1 ? t('what.one', { name: j.firstSource }) : t('what.items', { count: j.sourceCount });
  return `${op} ${what}`;
}

export function jobStatus(j: JobSnapshot): string {
  if (j.held && !isFinished(j)) return t('jobs.held', { n: j.queue ?? 1 });
  return t(`jobs.st.${j.status}`);
}

/** Overall progress in percent. */
export function jobPct(j: JobSnapshot): number {
  if (j.status === 'done') return 100;
  if (j.totalBytes > 0) return Math.min(100, (100 * j.doneBytes) / j.totalBytes);
  if (j.totalFiles > 0) return Math.min(100, (100 * j.doneFiles) / j.totalFiles);
  return 0;
}

/** Seconds since the job started (until it finished). */
export function jobElapsed(j: JobSnapshot, now = Date.now()): number | null {
  if (j.started === null) return null;
  return Math.max(0, ((j.finished ?? now) - j.started) / 1000);
}

/** Average speed over the whole run (bytes per second). */
export function jobAvgSpeed(j: JobSnapshot, now = Date.now()): number {
  const s = jobElapsed(j, now);
  return s && s > 0.5 ? j.doneBytes / s : 0;
}
