import type { Entry, SortKey } from './types';

const collators = new Map<string, Intl.Collator>();

export function collator(locale: string): Intl.Collator {
  let c = collators.get(locale);
  if (!c) {
    // numeric: "file2" < "file10"; base sensitivity: case-insensitive, accent-aware ordering
    c = new Intl.Collator(locale, { numeric: true, sensitivity: 'base' });
    collators.set(locale, c);
  }
  return c;
}

/**
 * Folders first (always), then files. Folders follow the chosen key when
 * it makes sense for them (date, size once calculated), otherwise the name.
 */
export function sortEntries(
  list: readonly Entry[],
  key: SortKey,
  desc: boolean,
  locale: string,
  dirSizes?: ReadonlyMap<string, number>,
): Entry[] {
  const c = collator(locale);
  const byName = (a: Entry, b: Entry) => c.compare(a.name, b.name) || (a.name < b.name ? -1 : a.name > b.name ? 1 : 0);
  const files: Entry[] = [];
  const dirs: Entry[] = [];
  for (const e of list) (e.isDir ? dirs : files).push(e);

  let fileCmp: (a: Entry, b: Entry) => number;
  switch (key) {
    case 'ext':
      fileCmp = (a, b) => c.compare(a.ext, b.ext) || byName(a, b);
      break;
    case 'size':
      fileCmp = (a, b) => a.size - b.size || byName(a, b);
      break;
    case 'mtime':
      fileCmp = (a, b) => a.mtime - b.mtime || byName(a, b);
      break;
    default:
      fileCmp = byName;
  }
  let dirCmp: (a: Entry, b: Entry) => number = byName;
  if (key === 'mtime') dirCmp = fileCmp;
  if (key === 'size' && dirSizes && dirSizes.size > 0) {
    dirCmp = (a, b) => (dirSizes.get(a.key) ?? -1) - (dirSizes.get(b.key) ?? -1) || byName(a, b);
  }
  dirs.sort(dirCmp);
  files.sort(fileCmp);
  if (desc) {
    dirs.reverse();
    files.reverse();
  }
  return dirs.concat(files);
}
