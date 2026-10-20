// Renders scripts/app-icon.svg to the 1024px PNG that `tauri icon` turns into
// .icns/.ico and all PNG sizes.
import { readFileSync, writeFileSync } from 'node:fs';
import { Resvg } from '@resvg/resvg-js';

const svg = readFileSync(new URL('./app-icon.svg', import.meta.url), 'utf8');
const png = new Resvg(svg, { fitTo: { mode: 'width', value: 1024 } }).render().asPng();
writeFileSync(new URL('../src-tauri/icons/app-icon.png', import.meta.url), png);
console.log('src-tauri/icons/app-icon.png written');
