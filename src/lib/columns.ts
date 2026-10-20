// Metadata columns of the details view: registry, formatting, presets.

import { formatDate, formatDuration, formatNumber } from './format';
import type { Entry, FileMeta } from './types';

export type ColumnGroup = 'photo' | 'video' | 'audio' | 'doc' | 'other';

export interface MetaColumn {
  id: string;
  /** i18n key */
  label: string;
  group: ColumnGroup;
  width: number;
  right?: boolean;
  text: (m: FileMeta, locale: string) => string;
  /** Sort value; undefined sorts last. */
  value: (m: FileMeta) => number | string | undefined;
}

const dims = (m: FileMeta) => (m.width && m.height ? `${m.width} × ${m.height}` : '');
const num = (n: number | undefined, locale: string, digits = 0) => (n === undefined ? '' : formatNumber(Number(n.toFixed(digits)), locale));

export const META_COLUMNS: MetaColumn[] = [
  { id: 'dims', label: 'mcol.dims', group: 'photo', width: 104, right: true, text: dims, value: (m) => (m.width && m.height ? m.width * m.height : undefined) },
  { id: 'taken', label: 'mcol.taken', group: 'photo', width: 136, text: (m, l) => (m.taken ? formatDate(m.taken - tzOffset(m.taken), l) : ''), value: (m) => m.taken },
  { id: 'camera', label: 'mcol.camera', group: 'photo', width: 140, text: (m) => m.camera ?? '', value: (m) => m.camera },
  { id: 'lens', label: 'mcol.lens', group: 'photo', width: 150, text: (m) => m.lens ?? '', value: (m) => m.lens },
  { id: 'iso', label: 'mcol.iso', group: 'photo', width: 60, right: true, text: (m, l) => num(m.iso, l), value: (m) => m.iso },
  { id: 'aperture', label: 'mcol.aperture', group: 'photo', width: 64, right: true, text: (m, l) => (m.aperture ? `f/${num(m.aperture, l, 1)}` : ''), value: (m) => m.aperture },
  { id: 'exposure', label: 'mcol.exposure', group: 'photo', width: 70, right: true, text: (m) => m.exposure ?? '', value: (m) => m.exposure },
  { id: 'focal', label: 'mcol.focal', group: 'photo', width: 70, right: true, text: (m, l) => (m.focal ? `${num(m.focal, l, 1)} mm` : ''), value: (m) => m.focal },
  { id: 'gps', label: 'mcol.gps', group: 'photo', width: 150, text: (m) => (m.gps ? `${m.gps[0].toFixed(4)}, ${m.gps[1].toFixed(4)}` : ''), value: (m) => (m.gps ? m.gps[0] : undefined) },
  { id: 'duration', label: 'mcol.duration', group: 'video', width: 70, right: true, text: (m, l) => (m.duration ? formatDuration(m.duration, l) : ''), value: (m) => m.duration },
  { id: 'codec', label: 'mcol.codec', group: 'video', width: 110, text: (m) => [m.videoCodec, m.audioCodec].filter(Boolean).join(' / '), value: (m) => m.videoCodec ?? m.audioCodec },
  { id: 'fps', label: 'mcol.fps', group: 'video', width: 56, right: true, text: (m, l) => num(m.fps, l, 2), value: (m) => m.fps },
  { id: 'bitrate', label: 'mcol.bitrate', group: 'video', width: 90, right: true, text: (m, l) => (m.bitrate ? `${num(m.bitrate, l)} kb/s` : ''), value: (m) => m.bitrate },
  { id: 'artist', label: 'mcol.artist', group: 'audio', width: 140, text: (m) => m.artist ?? '', value: (m) => m.artist },
  { id: 'album', label: 'mcol.album', group: 'audio', width: 140, text: (m) => m.album ?? '', value: (m) => m.album },
  { id: 'title', label: 'mcol.title', group: 'audio', width: 160, text: (m) => m.title ?? '', value: (m) => m.title },
  { id: 'track', label: 'mcol.track', group: 'audio', width: 50, right: true, text: (m) => (m.track ? String(m.track) : ''), value: (m) => m.track },
  { id: 'year', label: 'mcol.year', group: 'audio', width: 56, right: true, text: (m) => (m.year ? String(m.year) : ''), value: (m) => m.year },
  { id: 'genre', label: 'mcol.genre', group: 'audio', width: 100, text: (m) => m.genre ?? '', value: (m) => m.genre },
  { id: 'samplerate', label: 'mcol.sampleRate', group: 'audio', width: 80, right: true, text: (m, l) => (m.sampleRate ? `${num(m.sampleRate / 1000, l, 1)} kHz` : ''), value: (m) => m.sampleRate },
  { id: 'pages', label: 'mcol.pages', group: 'doc', width: 60, right: true, text: (m, l) => num(m.pages, l), value: (m) => m.pages },
  { id: 'author', label: 'mcol.author', group: 'doc', width: 130, text: (m) => m.author ?? '', value: (m) => m.author },
  { id: 'doctitle', label: 'mcol.docTitle', group: 'doc', width: 180, text: (m) => m.title ?? '', value: (m) => m.title },
  { id: 'lines', label: 'mcol.lines', group: 'other', width: 70, right: true, text: (m, l) => num(m.lines, l), value: (m) => m.lines },
];

export const columnById = new Map(META_COLUMNS.map((c) => [c.id, c]));

/** EXIF dates are wall-clock times stored as UTC; show them unshifted. */
function tzOffset(ms: number): number {
  return new Date(ms).getTimezoneOffset() * -60_000;
}

// ---------------------------------------------------------------- kinds (mirror of meta::kind_of)

const KINDS: Record<string, string[]> = {
  raw: 'dng cr2 cr3 crw nef nrw arw srf sr2 orf rw2 raf pef srw x3f erf kdc dcr mrw 3fr iiq rwl'.split(' '),
  image: 'jpg jpeg jfif png apng gif webp bmp ico tif tiff heic heif avif jxl tga exr hdr psd psb qoi dds pnm ppm pgm pbm'.split(' '),
  video: 'mp4 m4v mov qt 3gp 3g2 mkv webm avi wmv flv mpg mpeg m2ts mts ts vob ogv'.split(' '),
  audio: 'mp3 m4a m4b aac flac wav wave aif aiff ogg oga opus wma ape wv mpc dsf'.split(' '),
  doc: 'pdf doc docx docm odt rtf pages epub xls xlsx xlsm xlsb ods numbers ppt pptx pptm odp key'.split(' '),
  code: 'rs c h cc cpp hpp cs java kt swift m mm go py rb php js mjs jsx ts tsx svelte vue html css scss json yaml yml toml xml sql sh lua dart txt md log csv'.split(' '),
};
const kindByExt = new Map<string, string>();
for (const [k, exts] of Object.entries(KINDS)) for (const e of exts) if (!kindByExt.has(e)) kindByExt.set(e, k);

export function kindOf(ext: string): string {
  return kindByExt.get(ext.toLowerCase()) ?? 'other';
}

export function hasMeta(ext: string): boolean {
  return kindByExt.has(ext.toLowerCase());
}

const PRESETS: Record<string, string[]> = {
  photo: ['dims', 'taken', 'camera'],
  video: ['duration', 'dims', 'codec'],
  audio: ['artist', 'album', 'title', 'duration'],
  doc: ['pages', 'author'],
  code: ['lines'],
};

/** Columns that fit a folder's content: the dominant kind among its files. */
export function autoColumns(entries: readonly Entry[]): string[] {
  const counts = new Map<string, number>();
  let files = 0;
  for (const e of entries) {
    if (e.isDir) continue;
    files++;
    let k = kindOf(e.ext);
    if (k === 'raw' || k === 'image') k = 'photo';
    counts.set(k, (counts.get(k) ?? 0) + 1);
  }
  if (files === 0) return [];
  let best = '';
  let n = 0;
  for (const [k, c] of counts) if (c > n && k !== 'other') [best, n] = [k, c];
  return n / files >= 0.4 ? (PRESETS[best] ?? []) : [];
}
