// Multi-rename engine ([N] [E] [C] [Y][M][D]… mask syntax).
//
//   [N] name without extension     [N2-5] characters 2–5     [N3] character 3
//   [N2,4] 4 characters from 2     [N-3,3] last three characters
//   [E] extension (same ranges)    [C] counter               [P] parent folder
//   [Y] [M] [D] date, [h] [m] [s] time of last modification

export type CaseMode = 'keep' | 'lower' | 'upper' | 'title' | 'first';

export interface RenameRule {
  nameMask: string;
  extMask: string;
  search: string;
  replace: string;
  regex: boolean;
  caseSensitive: boolean;
  caseMode: CaseMode;
  counterStart: number;
  counterStep: number;
  counterDigits: number;
}

export const defaultRule = (): RenameRule => ({
  nameMask: '[N]',
  extMask: '[E]',
  search: '',
  replace: '',
  regex: false,
  caseSensitive: false,
  caseMode: 'keep',
  counterStart: 1,
  counterStep: 1,
  counterDigits: 1,
});

export interface RenameItem {
  name: string;
  isDir: boolean;
  mtime: number;
}

export interface Preview {
  from: string;
  to: string;
  status: 'ok' | 'unchanged' | 'duplicate' | 'invalid';
}

function split(name: string, isDir: boolean): [string, string] {
  if (isDir) return [name, ''];
  const i = name.lastIndexOf('.');
  return i > 0 ? [name.slice(0, i), name.slice(i + 1)] : [name, ''];
}

/** Applies a [Nx-y] style range to `s` (1-based, inclusive; negative = from end). */
function range(s: string, spec: string): string {
  const chars = [...s];
  if (!spec) return s;
  let m = /^(-?\d+),(\d+)$/.exec(spec);
  if (m) {
    let start = parseInt(m[1], 10);
    const len = parseInt(m[2], 10);
    if (start < 0) start = chars.length + start + 1;
    return chars.slice(Math.max(0, start - 1), Math.max(0, start - 1) + len).join('');
  }
  m = /^(-?\d+)-(-?\d+)?$/.exec(spec);
  if (m) {
    let a = parseInt(m[1], 10);
    let b = m[2] !== undefined ? parseInt(m[2], 10) : chars.length;
    if (a < 0) a = chars.length + a + 1;
    if (b < 0) b = chars.length + b + 1;
    return chars.slice(Math.max(0, a - 1), Math.max(0, b)).join('');
  }
  m = /^(-?\d+)$/.exec(spec);
  if (m) {
    let a = parseInt(m[1], 10);
    if (a < 0) a = chars.length + a + 1;
    return chars[a - 1] ?? '';
  }
  return s;
}

const two = (n: number) => String(n).padStart(2, '0');

function expand(mask: string, stem: string, ext: string, counter: string, parent: string, mtime: number): string {
  const d = new Date(mtime);
  return mask.replace(/\[([NECPYMDhms])([^\]]*)\]/g, (_, key: string, spec: string) => {
    switch (key) {
      case 'N':
        return range(stem, spec);
      case 'E':
        return range(ext, spec);
      case 'C':
        return counter;
      case 'P':
        return parent;
      case 'Y':
        return String(d.getFullYear());
      case 'M':
        return two(d.getMonth() + 1);
      case 'D':
        return two(d.getDate());
      case 'h':
        return two(d.getHours());
      case 'm':
        return two(d.getMinutes());
      case 's':
        return two(d.getSeconds());
    }
    return '';
  });
}

function applyCase(s: string, mode: CaseMode, locale: string): string {
  switch (mode) {
    case 'lower':
      return s.toLocaleLowerCase(locale);
    case 'upper':
      return s.toLocaleUpperCase(locale);
    case 'title':
      return s.toLocaleLowerCase(locale).replace(/(^|[\s_\-.(\[])(\p{L})/gu, (_, sep: string, ch: string) => sep + ch.toLocaleUpperCase(locale));
    case 'first': {
      const lower = s.toLocaleLowerCase(locale);
      return lower.charAt(0).toLocaleUpperCase(locale) + lower.slice(1);
    }
    default:
      return s;
  }
}

const INVALID = /[\\/\u0000]|^\.{1,2}$|^$/;
const INVALID_WIN = /[<>:"|?*\u0001-\u001f]|[ .]$/;

export function previewRename(
  items: RenameItem[],
  rule: RenameRule,
  opts: { parent: string; locale: string; windows: boolean; caseInsensitive: boolean },
): Preview[] {
  let search: RegExp | null = null;
  if (rule.search) {
    try {
      const pattern = rule.regex ? rule.search : rule.search.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
      search = new RegExp(pattern, rule.caseSensitive ? 'g' : 'gi');
    } catch {
      search = null;
    }
  }
  const out: Preview[] = items.map((item, i) => {
    const [stem, ext] = split(item.name, item.isDir);
    const n = rule.counterStart + i * rule.counterStep;
    const counter = String(n).padStart(Math.max(1, rule.counterDigits), '0');
    const newStem = expand(rule.nameMask || '[N]', stem, ext, counter, opts.parent, item.mtime);
    const newExt = item.isDir ? '' : expand(rule.extMask, stem, ext, counter, opts.parent, item.mtime);
    let name = newExt ? `${newStem}.${newExt}` : newStem;
    if (search) name = name.replace(search, rule.replace);
    name = applyCase(name, rule.caseMode, opts.locale);
    return { from: item.name, to: name, status: 'ok' as Preview['status'] };
  });

  const fold = (s: string) => (opts.caseInsensitive ? s.toLowerCase() : s);
  const seen = new Map<string, number>();
  for (const p of out) {
    if (INVALID.test(p.to) || (opts.windows && INVALID_WIN.test(p.to))) {
      p.status = 'invalid';
      continue;
    }
    if (p.to === p.from) p.status = 'unchanged';
    const k = fold(p.to);
    seen.set(k, (seen.get(k) ?? 0) + 1);
  }
  for (const p of out) {
    if (p.status !== 'invalid' && (seen.get(fold(p.to)) ?? 0) > 1) p.status = 'duplicate';
  }
  return out;
}
