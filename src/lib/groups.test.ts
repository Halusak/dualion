import { describe, expect, it } from 'vitest';
import { groupEntries, groupKey, renameGroup } from './groups';
import type { Entry } from './types';

const file = (name: string, size = 1): Entry => ({
  name,
  flags: 0,
  size,
  mtime: 0,
  attrs: 0,
  ext: name.includes('.') ? name.slice(name.lastIndexOf('.') + 1) : '',
  isDir: false,
  key: name,
});

describe('groupKey', () => {
  it('strips extensions, sidecars and subtitle languages', () => {
    expect(groupKey('Icon.PNG')).toBe('icon');
    expect(groupKey('IMG_0001.CR2.xmp')).toBe('img_0001');
    expect(groupKey('photo.jpg.json')).toBe('photo');
    expect(groupKey('Film.2024.cs.srt')).toBe('film.2024');
    expect(groupKey('Film.2024.en.forced.srt')).toBe('film.2024');
    expect(groupKey('Film.2024.mkv')).toBe('film.2024');
    expect(groupKey('.gitignore')).toBe('');
    expect(groupKey('README')).toBe('');
  });
});

describe('groupEntries', () => {
  const list = ['icon.ico', 'icon.png', 'icon.psd', 'other.txt', 'IMG_1.CR2', 'IMG_1.JPG', 'IMG_1.CR2.xmp'].map((n) => file(n, 10));
  it('collapses groups at the leader position', () => {
    const g = groupEntries(list, new Set());
    expect(g.list.map((e) => e.name)).toEqual(['icon.png', 'other.txt', 'IMG_1.JPG']);
    expect(g.list[0].groupSize).toBe(3);
    expect(g.list[0].groupBytes).toBe(30);
    expect(g.members.get('IMG_1.JPG')!.map((e) => e.name)).toEqual(['IMG_1.JPG', 'IMG_1.CR2', 'IMG_1.CR2.xmp']);
  });
  it('lists members of expanded groups', () => {
    const g = groupEntries(list, new Set(['icon.png']));
    expect(g.list.slice(0, 3).map((e) => [e.name, e.groupOf])).toEqual([
      ['icon.png', undefined],
      ['icon.ico', 'icon.png'],
      ['icon.psd', 'icon.png'],
    ]);
  });
});

describe('renameGroup', () => {
  it('keeps each member suffix', () => {
    const ms = ['IMG_1.JPG', 'IMG_1.CR2', 'IMG_1.CR2.xmp'].map((n) => file(n));
    expect(renameGroup(ms, ms[0], 'Hory.JPG')).toEqual([
      { from: 'IMG_1.JPG', to: 'Hory.JPG' },
      { from: 'IMG_1.CR2', to: 'Hory.CR2' },
      { from: 'IMG_1.CR2.xmp', to: 'Hory.CR2.xmp' },
    ]);
    expect(renameGroup(ms, ms[0], 'Hory.jpeg')[0]).toEqual({ from: 'IMG_1.JPG', to: 'Hory.JPG' });
  });
});
