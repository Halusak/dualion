// File groups: files that belong together by name (icon.png + icon.psd +
// icon.ico, IMG_1.CR2 + IMG_1.JPG + IMG_1.CR2.xmp, film.mkv + film.cs.srt)
// are shown as one item and copied / moved / renamed / deleted together.

import { kindOf } from './columns';
import type { Entry } from './types';

/** Companion files that never lead a group. */
const SIDECAR = new Set(['xmp', 'aae', 'thm', 'lrv', 'pp3', 'dop', 'nfo', 'json', 'srt', 'vtt', 'ass', 'ssa', 'sub', 'idx', 'sfv', 'md5', 'sha1', 'sha256', 'log', 'aux', 'toc', 'out', 'bbl', 'blg', 'synctex', 'mtl', 'cue', 'lrc', 'url']);
const SUBTITLE = new Set(['srt', 'vtt', 'ass', 'ssa', 'sub', 'idx']);
const SUB_TAGS = /^(forced|sdh|cc|hi|default|[a-z]{2,3}(-[a-z]{2,4})?)$/;

function splitExt(name: string): [string, string] {
  const i = name.lastIndexOf('.');
  return i > 0 ? [name.slice(0, i), name.slice(i + 1)] : [name, ''];
}

/** Group key of a file name (lower-case stem without sidecar / language suffixes). */
export function groupKey(name: string): string {
  let [stem, ext] = splitExt(name.toLowerCase());
  if (!ext || !stem) return '';
  if (SUBTITLE.has(ext)) {
    // film.cs.srt, film.en.forced.srt
    for (let k = 0; k < 2; k++) {
      const [s, tag] = splitExt(stem);
      if (tag && SUB_TAGS.test(tag) && s) stem = s;
      else break;
    }
  }
  if (SIDECAR.has(ext)) {
    // photo.jpg.xmp, photo.cr2.json
    const [s, inner] = splitExt(stem);
    if (inner && s && kindOf(inner) !== 'other') stem = s;
  }
  return stem;
}

/** Which member represents the group (lower = better). */
function rank(e: Entry): number {
  const ext = e.ext.toLowerCase();
  if (SIDECAR.has(ext)) return 90;
  switch (kindOf(ext)) {
    case 'video':
      return 0;
    case 'image':
      return ['jpg', 'jpeg', 'heic', 'heif', 'png', 'webp', 'avif'].includes(ext) ? 1 : ['psd', 'psb'].includes(ext) ? 4 : 3;
    case 'raw':
      return 2;
    case 'audio':
      return 5;
    case 'doc':
      return ext === 'pdf' ? 6 : 7;
    default:
      return 10;
  }
}

export interface Grouped {
  /** Visible list: group leaders (with `groupSize`), expanded members follow. */
  list: Entry[];
  /** Members of each group by the leader's key. */
  members: Map<string, Entry[]>;
}

/**
 * Collapses related files. `list` must already be sorted; a group appears at
 * its leader's position. Expanded groups list their other members right after.
 */
export function groupEntries(list: readonly Entry[], expanded: ReadonlySet<string>): Grouped {
  const byKey = new Map<string, Entry[]>();
  for (const e of list) {
    if (e.isDir || e.path) continue;
    const k = groupKey(e.name);
    if (!k) continue;
    let g = byKey.get(k);
    if (!g) byKey.set(k, (g = []));
    g.push(e);
  }
  const leaderOf = new Map<string, Entry>();
  const members = new Map<string, Entry[]>();
  for (const g of byKey.values()) {
    if (g.length < 2) continue;
    const sorted = [...g].sort((a, b) => rank(a) - rank(b) || a.name.length - b.name.length);
    const leader = sorted[0];
    members.set(leader.key, sorted);
    for (const m of g) leaderOf.set(m.key, leader);
  }
  if (members.size === 0) return { list: list as Entry[], members };
  const out: Entry[] = [];
  for (const e of list) {
    const leader = leaderOf.get(e.key);
    if (!leader) {
      out.push(e);
      continue;
    }
    if (leader !== e) continue;
    const ms = members.get(e.key)!;
    const size = ms.reduce((a, m) => a + m.size, 0);
    out.push({ ...e, groupSize: ms.length, groupBytes: size });
    if (expanded.has(e.key)) for (const m of ms) if (m !== e) out.push({ ...m, groupOf: e.key });
  }
  return { list: out, members };
}

/** New names for a renamed group: each member keeps its own suffix. */
export function renameGroup(members: readonly Entry[], leader: Entry, newLeaderName: string): { from: string; to: string }[] {
  const stemLen = groupKey(leader.name).length;
  const [newStem] = newLeaderName.toLowerCase().endsWith(leader.name.slice(stemLen).toLowerCase())
    ? [newLeaderName.slice(0, newLeaderName.length - (leader.name.length - stemLen))]
    : [splitExt(newLeaderName)[0]];
  return members.map((m) => ({ from: m.name, to: newStem + m.name.slice(stemLen) })).filter((p) => p.from !== p.to);
}
