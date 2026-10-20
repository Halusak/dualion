import { isMac } from './platform';

function parseHex(hex: string): [number, number, number] | null {
  const m = /^#?([0-9a-f]{6})$/i.exec(hex.trim());
  if (!m) return null;
  const n = parseInt(m[1], 16);
  return [(n >> 16) & 255, (n >> 8) & 255, n & 255];
}

function luminance([r, g, b]: [number, number, number]): number {
  const lin = (c: number) => {
    const s = c / 255;
    return s <= 0.03928 ? s / 12.92 : ((s + 0.055) / 1.055) ** 2.4;
  };
  return 0.2126 * lin(r) + 0.7152 * lin(g) + 0.0722 * lin(b);
}

/** Applies the system accent colour (and readable derived shades) as CSS variables. */
export function applyAccent(hex: string | null, dark: boolean) {
  const fallback = isMac ? '#0a64d6' : '#005fb8';
  const rgb = parseHex(hex ?? '') ?? parseHex(fallback)!;
  const [r, g, b] = rgb;
  const root = document.documentElement.style;
  root.setProperty('--accent', `rgb(${r}, ${g}, ${b})`);
  root.setProperty('--accent-12', `rgba(${r}, ${g}, ${b}, 0.12)`);
  root.setProperty('--accent-20', `rgba(${r}, ${g}, ${b}, ${dark ? 0.28 : 0.2})`);
  root.setProperty('--accent-30', `rgba(${r}, ${g}, ${b}, ${dark ? 0.42 : 0.3})`);
  root.setProperty('--accent-fg', luminance(rgb) > 0.45 ? '#000000' : '#ffffff');
  // Text in the accent colour must stay readable on the panel background.
  const mix = (c: number, to: number, f: number) => Math.round(c + (to - c) * f);
  const text = dark ? [mix(r, 255, 0.35), mix(g, 255, 0.35), mix(b, 255, 0.35)] : [mix(r, 0, 0.15), mix(g, 0, 0.15), mix(b, 0, 0.15)];
  root.setProperty('--accent-text', `rgb(${text[0]}, ${text[1]}, ${text[2]})`);
}

/** Row height in px for the current density (mirrors app.css). */
export function rowHeight(density: string, windows: boolean): number {
  if (density === 'compact') return windows ? 21 : 19;
  if (density === 'comfortable') return windows ? 30 : 26;
  return windows ? 24 : 22;
}
