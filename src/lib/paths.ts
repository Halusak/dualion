import type { Entry, Location } from './types';
import { isWindows } from './platform';

export const SEP = isWindows ? '\\' : '/';

export function joinLocal(dir: string, name: string): string {
  if (dir.endsWith('/') || dir.endsWith('\\')) return dir + name;
  return dir + SEP + name;
}

export function joinFtp(dir: string, name: string): string {
  return dir.endsWith('/') ? dir + name : `${dir}/${name}`;
}

export function baseName(path: string): string {
  const p = path.replace(/[\\/]+$/, '');
  const i = Math.max(p.lastIndexOf('/'), p.lastIndexOf('\\'));
  return i >= 0 ? p.slice(i + 1) : p;
}

export function dirName(path: string): string {
  const p = path.replace(/[\\/]+$/, '');
  const i = Math.max(p.lastIndexOf('/'), p.lastIndexOf('\\'));
  if (i < 0) return p;
  if (i === 0) return p[0];
  const d = p.slice(0, i);
  return /^[A-Za-z]:$/.test(d) ? d + '\\' : d;
}

export function extOf(name: string): string {
  const i = name.lastIndexOf('.');
  return i > 0 ? name.slice(i + 1) : '';
}

export function stemOf(name: string): string {
  const i = name.lastIndexOf('.');
  return i > 0 ? name.slice(0, i) : name;
}

const caseInsensitive = true; // APFS and NTFS are case-insensitive by default

/** Stable identity of a location (used for equality and watcher matching). */
export function locKey(loc: Location): string {
  switch (loc.kind) {
    case 'local': {
      let p = loc.path.replace(/[\\/]+$/, '') || loc.path;
      if (caseInsensitive) p = p.toLowerCase();
      return `local:${p}`;
    }
    case 'ftp':
      return `ftp:${loc.session}:${loc.path.replace(/\/+$/, '') || '/'}`;
    case 'archive':
      return `arc:${loc.archive}!${loc.inner.replace(/^\/+|\/+$/g, '')}`;
    case 'drives':
      return 'drives';
    case 'network':
      return `net:${(loc.host ?? '').toLowerCase()}`;
    case 'search':
      return `search:${loc.id}`;
    case 'query':
      return `query:${loc.query}`;
  }
}

export function sameLoc(a: Location | null | undefined, b: Location | null | undefined): boolean {
  return !!a && !!b && locKey(a) === locKey(b);
}

/** Location of an entry inside `dir` (the item itself, not its folder). */
export function childLoc(dir: Location, e: Entry): Location {
  if (e.path) return { kind: 'local', path: e.path };
  switch (dir.kind) {
    case 'local':
      return { kind: 'local', path: joinLocal(dir.path, e.name) };
    case 'ftp':
      return { kind: 'ftp', session: dir.session, path: joinFtp(dir.path, e.name) };
    case 'archive':
      return { kind: 'archive', archive: dir.archive, inner: dir.inner ? `${dir.inner}/${e.name}` : e.name };
    case 'drives':
      return { kind: 'local', path: e.name };
    case 'network':
      // Shares open by UNC path on Windows; macOS mounts them first (see openEntry).
      if (!dir.host) return { kind: 'network', host: e.name };
      return isWindows ? { kind: 'local', path: `\\\\${dir.host}\\${e.name}` } : dir;
    case 'search':
    case 'query':
      return { kind: 'local', path: e.name };
  }
}

/** Parent folder of a location, if any. */
export function parentLoc(loc: Location): Location | null {
  switch (loc.kind) {
    case 'local': {
      const d = dirName(loc.path);
      return d && d !== loc.path ? { kind: 'local', path: d } : null;
    }
    case 'ftp': {
      const p = loc.path.replace(/\/+$/, '');
      if (!p) return null;
      const i = p.lastIndexOf('/');
      return { kind: 'ftp', session: loc.session, path: i <= 0 ? '/' : p.slice(0, i) };
    }
    case 'archive': {
      if (!loc.inner) return { kind: 'local', path: dirName(loc.archive) };
      const i = loc.inner.lastIndexOf('/');
      return { kind: 'archive', archive: loc.archive, inner: i < 0 ? '' : loc.inner.slice(0, i) };
    }
    case 'network':
      return loc.host ? { kind: 'network' } : null;
    default:
      return null;
  }
}

/** Plain path of a location for display / clipboard. */
export function locPath(loc: Location): string {
  switch (loc.kind) {
    case 'local':
      return loc.path;
    case 'ftp':
      return loc.path;
    case 'archive':
      return loc.inner ? `${loc.archive}${SEP}${loc.inner.replaceAll('/', SEP)}` : loc.archive;
    case 'drives':
      return '';
    case 'network':
      return loc.host ? `\\\\${loc.host}` : '';
    case 'search':
    case 'query':
      return loc.label;
  }
}

export const ARCHIVE_EXTS = new Set([
  'zip', 'jar', 'apk', 'epub', 'nupkg', 'vsix', 'whl', 'xpi', 'ipa', 'aar', 'war', 'cbz',
  '7z', 'cb7', 'rar', 'cbr', 'iso', 'tar', 'cbt', 'tgz', 'taz', 'tbz', 'tbz2', 'tb2', 'txz', 'tzst', 'gz', 'bz2', 'xz', 'zst',
]);

/** Archives Dualion opens like folders (ZIP, 7z, RAR, ISO, TAR*, single .gz …). */
export function isArchiveName(name: string): boolean {
  return ARCHIVE_EXTS.has(extOf(name).toLowerCase());
}

/** Disk images the system can mount. */
export function isDiskImage(name: string): boolean {
  return ['iso', 'dmg', 'img', 'vhd', 'vhdx', 'sparseimage', 'sparsebundle'].includes(extOf(name).toLowerCase());
}

/** Wildcard mask matcher compatible with the backend (TC syntax). */
export function maskMatcher(mask: string, caseSensitive = false): (name: string) => boolean {
  const [inc, exc = ''] = mask.split('|');
  const compile = (part: string) =>
    part
      .split(/[;,]/)
      .map((p) => p.trim())
      .filter((p) => p && p !== '*' && p !== '*.*')
      .map((p) => {
        const pat = /[*?]/.test(p) ? p : `*${p}*`;
        const re = pat.replace(/[.+^${}()|[\]\\]/g, '\\$&').replaceAll('*', '.*').replaceAll('?', '.');
        return new RegExp(`^${re}$`, caseSensitive ? '' : 'i');
      });
  const incs = compile(inc);
  const excs = compile(exc);
  return (name) => (incs.length === 0 || incs.some((r) => r.test(name))) && !excs.some((r) => r.test(name));
}

/** Locations that list search results (files from many folders). */
export function isResults(loc: Location): boolean {
  return loc.kind === 'search' || loc.kind === 'query';
}
