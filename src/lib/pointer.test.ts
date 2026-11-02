import { describe, expect, it } from 'vitest';
import { canOpenContextMenu } from './pointer';
import { settings } from './state/settings.svelte';
import type { Entry } from './types';

const sampleEntry: Entry = {
  name: 'test.txt',
  flags: 0,
  size: 100,
  mtime: Date.now(),
  attrs: 0,
  ext: 'txt',
  isDir: false,
  key: 'test.txt',
};

const dummyEvent = {} as MouseEvent;

describe('pointer right-click selection', () => {
  it('allows context menu when rightClickSelect is disabled', () => {
    settings.rightClickSelect = false;
    expect(canOpenContextMenu(dummyEvent, sampleEntry)).toBe(true);
    expect(canOpenContextMenu(dummyEvent, null)).toBe(true);
  });

  it('allows background context menu even when rightClickSelect is enabled', () => {
    settings.rightClickSelect = true;
    expect(canOpenContextMenu(dummyEvent, null)).toBe(true);
  });
});

