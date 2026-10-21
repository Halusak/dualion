import { describe, expect, it } from 'vitest';
import { formatDate, formatPattern } from './format';
import { settings } from './state/settings.svelte';

const t = new Date(2026, 9, 21, 18, 48, 5).getTime();

describe('formatPattern', () => {
  it('formats the offered patterns', () => {
    expect(formatPattern(t, 'DD.MM.YYYY HH:mm')).toBe('21.10.2026 18:48');
    expect(formatPattern(t, 'D. M. YYYY H:mm')).toBe('21. 10. 2026 18:48');
    expect(formatPattern(t, 'YYYY-MM-DD HH:mm')).toBe('2026-10-21 18:48');
    expect(formatPattern(t, 'MM/DD/YYYY h:mm A')).toBe('10/21/2026 6:48 PM');
    expect(formatPattern(t, 'DD.MM.YY HH:mm')).toBe('21.10.26 18:48');
  });

  it('adds seconds when asked', () => {
    expect(formatPattern(t, 'DD.MM.YYYY HH:mm', true)).toBe('21.10.2026 18:48:05');
    expect(formatPattern(t, 'HH:mm:ss', true)).toBe('18:48:05');
  });

  it('formatDate follows the setting', () => {
    settings.dateFormat = 'YYYY-MM-DD HH:mm';
    expect(formatDate(t, 'en-US')).toBe('2026-10-21 18:48');
    settings.dateFormat = 'auto';
    expect(formatDate(t, 'cs-CZ')).not.toBe('2026-10-21 18:48');
    expect(formatDate(0, 'cs-CZ')).toBe('');
  });
});
