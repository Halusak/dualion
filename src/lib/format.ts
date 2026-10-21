import { isMac } from './platform';
import { settings } from './state/settings.svelte';

export type SizeFormat = 'auto' | 'binary' | 'decimal' | 'bytes';

const numberFormats = new Map<string, Intl.NumberFormat>();
function nf(locale: string, digits: number): Intl.NumberFormat {
  const key = `${locale}|${digits}`;
  let f = numberFormats.get(key);
  if (!f) {
    f = new Intl.NumberFormat(locale, { maximumFractionDigits: digits, minimumFractionDigits: 0 });
    numberFormats.set(key, f);
  }
  return f;
}

export function formatNumber(n: number, locale: string): string {
  return nf(locale, 0).format(n);
}

/**
 * Human readable size. "auto" follows the platform: macOS Finder counts in
 * powers of 1000, Windows Explorer in powers of 1024 (labelled KB/MB).
 */
export function formatSize(bytes: number, locale: string, mode: SizeFormat = 'auto'): string {
  if (mode === 'bytes') return `${formatNumber(bytes, locale)} B`;
  const decimal = mode === 'decimal' || (mode === 'auto' && isMac);
  const step = decimal ? 1000 : 1024;
  const units = decimal ? ['B', 'kB', 'MB', 'GB', 'TB', 'PB'] : ['B', 'KB', 'MB', 'GB', 'TB', 'PB'];
  if (bytes < step) return `${formatNumber(bytes, locale)} ${units[0]}`;
  let v = bytes;
  let i = 0;
  while (v >= step && i < units.length - 1) {
    v /= step;
    i++;
  }
  const digits = v >= 100 ? 0 : 1;
  return `${nf(locale, digits).format(v)} ${units[i]}`;
}

const dateFormats = new Map<string, Intl.DateTimeFormat>();
/** Ready-made date patterns offered in Settings ('auto' follows the language). */
export const DATE_PATTERNS = ['DD.MM.YYYY HH:mm', 'D. M. YYYY H:mm', 'YYYY-MM-DD HH:mm', 'DD/MM/YYYY HH:mm', 'MM/DD/YYYY h:mm A', 'DD.MM.YY HH:mm'];

const TOKENS = /YYYY|YY|MM|M|DD|D|HH|H|hh|h|mm|ss|A/g;

/** Formats a timestamp with a pattern: YYYY YY MM M DD D HH H hh h mm ss A. */
export function formatPattern(ms: number, pattern: string, withSeconds = false): string {
  const d = new Date(ms);
  if (withSeconds && !pattern.includes('ss')) pattern = pattern.replace(/mm/, 'mm:ss');
  const p2 = (n: number) => String(n).padStart(2, '0');
  const h = d.getHours();
  return pattern.replace(TOKENS, (tok) => {
    switch (tok) {
      case 'YYYY': return String(d.getFullYear());
      case 'YY': return p2(d.getFullYear() % 100);
      case 'MM': return p2(d.getMonth() + 1);
      case 'M': return String(d.getMonth() + 1);
      case 'DD': return p2(d.getDate());
      case 'D': return String(d.getDate());
      case 'HH': return p2(h);
      case 'H': return String(h);
      case 'hh': return p2(h % 12 || 12);
      case 'h': return String(h % 12 || 12);
      case 'mm': return p2(d.getMinutes());
      case 'ss': return p2(d.getSeconds());
      default: return h < 12 ? 'AM' : 'PM';
    }
  });
}

export function formatDate(ms: number, locale: string, withSeconds = false): string {
  if (!ms) return '';
  const pattern = settings.dateFormat;
  if (pattern && pattern !== 'auto') return formatPattern(ms, pattern, withSeconds);
  const key = `${locale}|${withSeconds}`;
  let f = dateFormats.get(key);
  if (!f) {
    f = new Intl.DateTimeFormat(locale, {
      year: 'numeric',
      month: '2-digit',
      day: '2-digit',
      hour: '2-digit',
      minute: '2-digit',
      second: withSeconds ? '2-digit' : undefined,
    });
    dateFormats.set(key, f);
  }
  return f.format(ms);
}

export function formatDuration(secs: number, locale: string): string {
  secs = Math.max(0, Math.round(secs));
  const h = Math.floor(secs / 3600);
  const m = Math.floor((secs % 3600) / 60);
  const s = secs % 60;
  const two = (n: number) => String(n).padStart(2, '0');
  void locale;
  return h > 0 ? `${h}:${two(m)}:${two(s)}` : `${m}:${two(s)}`;
}

export function formatSpeed(bytesPerSec: number, locale: string, mode: SizeFormat = 'auto'): string {
  return `${formatSize(Math.round(bytesPerSec), locale, mode)}/s`;
}

/** Unix permission bits as "rwxr-xr-x", Windows attributes as "RHSA". */
export function formatAttrs(attrs: number, windows: boolean): string {
  if (windows) {
    let s = '';
    s += attrs & 0x1 ? 'r' : '-';
    s += attrs & 0x2 ? 'h' : '-';
    s += attrs & 0x4 ? 's' : '-';
    s += attrs & 0x20 ? 'a' : '-';
    return s;
  }
  const chars = 'rwxrwxrwx';
  let s = '';
  for (let i = 0; i < 9; i++) s += attrs & (1 << (8 - i)) ? chars[i] : '-';
  return s;
}
