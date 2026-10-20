import { describe, expect, it } from 'vitest';
import { defaultRule, previewRename, type RenameItem } from './multirename';

const opts = { parent: 'Dovolená', locale: 'cs-CZ', windows: false, caseInsensitive: true };
const mtime = new Date(2024, 6, 9, 14, 5, 3).getTime();
const files: RenameItem[] = [
  { name: 'IMG_0001.JPG', isDir: false, mtime },
  { name: 'IMG_0002.JPG', isDir: false, mtime },
  { name: 'notes', isDir: true, mtime },
];

describe('multi-rename', () => {
  it('keeps names with the default masks', () => {
    const p = previewRename(files, defaultRule(), opts);
    expect(p.map((x) => x.to)).toEqual(['IMG_0001.JPG', 'IMG_0002.JPG', 'notes']);
    expect(p.every((x) => x.status === 'unchanged')).toBe(true);
  });

  it('supports counters, dates, parent folder and ranges', () => {
    const rule = { ...defaultRule(), nameMask: '[P] [Y]-[M]-[D] [C] [N5-8]', extMask: '[E]', counterDigits: 3, counterStart: 7 };
    const p = previewRename(files.slice(0, 2), rule, opts);
    expect(p[0].to).toBe('Dovolená 2024-07-09 007 0001.JPG');
    expect(p[1].to).toBe('Dovolená 2024-07-09 008 0002.JPG');
  });

  it('handles negative ranges and lengths', () => {
    const rule = { ...defaultRule(), nameMask: '[N-4,4]', extMask: '[E1]' };
    expect(previewRename([files[0]], rule, opts)[0].to).toBe('0001.J');
  });

  it('search & replace with regex and case conversion', () => {
    const rule = { ...defaultRule(), search: '^IMG_(\\d+)', replace: 'foto-$1', regex: true, caseMode: 'lower' as const };
    expect(previewRename([files[0]], rule, opts)[0].to).toBe('foto-0001.jpg');
    const title = { ...defaultRule(), caseMode: 'title' as const };
    expect(previewRename([{ name: 'žluťoučký kůň.txt', isDir: false, mtime }], title, opts)[0].to).toBe('Žluťoučký Kůň.Txt');
  });

  it('flags duplicates and invalid names', () => {
    const rule = { ...defaultRule(), nameMask: 'same' };
    const p = previewRename(files.slice(0, 2), rule, opts);
    expect(p.map((x) => x.status)).toEqual(['duplicate', 'duplicate']);
    const bad = previewRename([files[0]], { ...defaultRule(), nameMask: 'a/b' }, opts);
    expect(bad[0].status).toBe('invalid');
    const win = previewRename([files[0]], { ...defaultRule(), nameMask: 'what?' }, { ...opts, windows: true });
    expect(win[0].status).toBe('invalid');
  });
});
