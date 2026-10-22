import { describe, expect, it } from 'vitest';
import { baseName, childLoc, dirName, extOf, locKey, maskMatcher, parentLoc, sameLoc } from './paths';
import { sortEntries } from './sort';
import { F, type Entry } from './types';

const entry = (name: string, extra: Partial<Entry> = {}): Entry => ({
  name,
  flags: extra.isDir ? F.DIR : 0,
  size: 0,
  mtime: 0,
  attrs: 0,
  ext: extra.isDir ? '' : extOf(name),
  isDir: false,
  key: name,
  ...extra,
});

describe('paths', () => {
  it('splits names and folders', () => {
    expect(baseName('/a/b/c.txt')).toBe('c.txt');
    expect(dirName('/a/b/c.txt')).toBe('/a/b');
    expect(dirName('/a')).toBe('/');
    expect(dirName('C:\\Users')).toBe('C:\\');
    expect(extOf('archive.tar.gz')).toBe('gz');
    expect(extOf('.env')).toBe('');
  });

  it('navigates locations', () => {
    expect(parentLoc({ kind: 'remote', session: 's', path: '/pub/x' })).toEqual({ kind: 'remote', session: 's', path: '/pub' });
    expect(parentLoc({ kind: 'remote', session: 's', path: '/pub' })).toEqual({ kind: 'remote', session: 's', path: '/' });
    expect(parentLoc({ kind: 'archive', archive: '/d/a.zip', inner: '' })).toEqual({ kind: 'local', path: '/d' });
    expect(parentLoc({ kind: 'archive', archive: '/d/a.zip', inner: 'x/y' })).toEqual({ kind: 'archive', archive: '/d/a.zip', inner: 'x' });
    expect(childLoc({ kind: 'archive', archive: '/a.zip', inner: '' }, entry('in'))).toEqual({ kind: 'archive', archive: '/a.zip', inner: 'in' });
    expect(childLoc({ kind: 'remote', session: 's', path: '/' }, entry('f'))).toEqual({ kind: 'remote', session: 's', path: '/f' });
  });

  it('compares local paths case-insensitively and ignores trailing slashes', () => {
    expect(sameLoc({ kind: 'local', path: '/Users/X/' }, { kind: 'local', path: '/users/x' })).toBe(true);
    expect(locKey({ kind: 'drives' })).toBe('drives');
  });

  it('matches wildcard masks', () => {
    const m = maskMatcher('*.jpg;*.png | thumb*');
    expect(m('A.JPG')).toBe(true);
    expect(m('thumb.png')).toBe(false);
    expect(m('x.gif')).toBe(false);
    expect(maskMatcher('rep')('Report.pdf')).toBe(true);
    expect(maskMatcher('*.*')('anything')).toBe(true);
    expect(maskMatcher('a?c')('abc')).toBe(true);
  });
});

describe('sorting', () => {
  it('puts folders first and sorts naturally', () => {
    const list = [entry('file10.txt'), entry('file2.txt'), entry('Zeta', { isDir: true }), entry('alpha', { isDir: true }), entry('File1.txt')];
    expect(sortEntries(list, 'name', false, 'cs-CZ').map((e) => e.name)).toEqual(['alpha', 'Zeta', 'File1.txt', 'file2.txt', 'file10.txt']);
  });

  it('sorts Czech names correctly (ch after h)', () => {
    const list = [entry('chata.txt'), entry('hrad.txt'), entry('ivan.txt')];
    expect(sortEntries(list, 'name', false, 'cs-CZ').map((e) => e.name)).toEqual(['hrad.txt', 'chata.txt', 'ivan.txt']);
  });

  it('sorts by size and date, descending keeps folders on top', () => {
    const list = [entry('a', { size: 5 }), entry('b', { size: 50 }), entry('d', { isDir: true })];
    expect(sortEntries(list, 'size', true, 'en-US').map((e) => e.name)).toEqual(['d', 'b', 'a']);
  });
});
