// Builds every app icon from scripts/app-icon.svg (`npm run icons`).
//
// The artwork sits on Apple's icon grid: an 824px body inside a 1024px canvas.
// macOS wants that margin and shadow, but on Windows it makes the icon look
// much smaller than other apps' icons. So there are two variants:
// - full-bleed (body cropped to the edges, no shadow): .ico and all PNGs
// - padded (as drawn): .icns and the About dialog image
import { readFileSync, writeFileSync, mkdtempSync, copyFileSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { execSync } from 'node:child_process';
import { Resvg } from '@resvg/resvg-js';

const root = fileURLToPath(new URL('..', import.meta.url));
const icons = join(root, 'src-tauri/icons');
const svg = readFileSync(join(root, 'scripts/app-icon.svg'), 'utf8');

const render = (src, size) => new Resvg(src, { fitTo: { mode: 'width', value: size } }).render().asPng();
// 4px of the canvas around the body keeps the anti-aliased edge from being clipped.
const full = svg
  .split('\n')
  .filter((l) => !l.includes('mac-only'))
  .join('\n')
  .replace('viewBox="0 0 1024 1024"', 'viewBox="96 96 832 832"');

const tmp = mkdtempSync(join(tmpdir(), 'dualion-icon-'));
try {
  writeFileSync(join(icons, 'app-icon.png'), render(full, 1024));
  writeFileSync(join(tmp, 'app-icon-mac.png'), render(svg, 1024));
  writeFileSync(join(root, 'src/assets/app-icon.png'), render(svg, 256));

  const tauri = (args) => execSync(`npx tauri icon ${args}`, { cwd: root, stdio: 'inherit' });
  tauri(`"${join(icons, 'app-icon.png')}"`);
  tauri(`"${join(tmp, 'app-icon-mac.png')}" -o "${join(tmp, 'mac')}"`);
  copyFileSync(join(tmp, 'mac/icon.icns'), join(icons, 'icon.icns'));
  // No mobile targets.
  for (const dir of ['android', 'ios']) rmSync(join(icons, dir), { recursive: true, force: true });
} finally {
  rmSync(tmp, { recursive: true, force: true });
}
console.log('icons written');
