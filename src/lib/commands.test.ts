import { beforeEach, describe, expect, it } from 'vitest';
import { assignShortcut, command, commandForCombo, comboOf, keysOf, removeShortcut, resetShortcuts } from './commands';
import { settings } from './state/settings.svelte';

const key = (init: Partial<KeyboardEvent>) =>
  ({ metaKey: false, ctrlKey: false, altKey: false, shiftKey: false, key: '', code: '', ...init }) as KeyboardEvent;

describe('keymap', () => {
  beforeEach(() => resetShortcuts());

  it('has Total Commander defaults', () => {
    expect(commandForCombo('F5')?.id).toBe('copy');
    expect(commandForCombo('F7')?.id).toBe('mkdir');
    expect(commandForCombo('Alt+F7')?.id).toBe('search');
  });

  it('reassigns a combo and removes it from the previous command', () => {
    assignShortcut('move', 'F5');
    expect(commandForCombo('F5')?.id).toBe('move');
    expect(keysOf(command('move'))[0]).toBe('F5');
    expect(keysOf(command('copy'))).not.toContain('F5');
    expect(settings.keymap.copy).toEqual([]);
  });

  it('removes and resets shortcuts', () => {
    removeShortcut('mkdir', 'F7');
    expect(commandForCombo('F7')).toBeUndefined();
    resetShortcuts('mkdir');
    expect(commandForCombo('F7')?.id).toBe('mkdir');
  });

  it('normalizes key events (letters by layout, digits by position)', () => {
    expect(comboOf(key({ key: 'F5', code: 'F5' }))).toBe('F5');
    expect(comboOf(key({ key: '+', code: 'Digit1', shiftKey: false }))).toBe('1');
    expect(comboOf(key({ key: 'z', code: 'KeyY' }))).toBe('Z');
    expect(comboOf(key({ key: 'å', code: 'KeyA', altKey: true }))).toBe('Alt+A');
  });
});
