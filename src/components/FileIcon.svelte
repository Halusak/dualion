<script lang="ts" module>
  import { isWindows } from '../lib/platform';

  const PER_FILE = new Set(
    isWindows ? ['exe', 'lnk', 'ico', 'url', 'msc', 'cpl', 'scr', 'appref-ms'] : ['icns', 'webloc', 'fileloc', 'terminal', 'app'],
  );
  const THUMBS = new Set([
    // images
    'jpg', 'jpeg', 'jfif', 'png', 'apng', 'gif', 'webp', 'bmp', 'ico', 'icns', 'tif', 'tiff', 'heic', 'heif', 'avif', 'jxl', 'tga', 'exr', 'hdr', 'svg',
    // camera raw
    'dng', 'cr2', 'cr3', 'crw', 'nef', 'nrw', 'arw', 'srf', 'sr2', 'orf', 'rw2', 'raf', 'pef', 'srw', 'x3f', 'erf', 'kdc', 'dcr', 'mrw', '3fr', 'iiq', 'rwl', 'mos',
    // design apps
    'psd', 'psb', 'af', 'afphoto', 'afdesign', 'afpub', 'aftemplate', 'clip', 'kra', 'krz', 'ora', 'sketch', 'procreate', 'xmind', 'ai', 'eps', 'indd',
    // video
    'mp4', 'mov', 'm4v', 'avi', 'mkv', 'webm', 'wmv', 'mpg', 'mpeg', '3gp', 'mts', 'm2ts', 'flv',
    // audio with cover art
    'mp3', 'm4a', 'm4b', 'aac', 'flac', 'aiff', 'aif', 'wma', 'dsf',
    // documents
    'pdf', 'doc', 'docx', 'docm', 'dotx', 'xls', 'xlsx', 'xlsm', 'xlsb', 'ppt', 'pptx', 'pptm', 'ppsx', 'pps', 'rtf', 'pages', 'numbers', 'key',
    'odt', 'ott', 'ods', 'ots', 'odp', 'otp', 'odg', 'otg', 'epub', 'mobi', 'azw3', 'vsd', 'pub',
    // 3D
    '3mf', 'stl', 'obj', 'usdz', 'fbx',
  ]);

  export function hasThumbnail(ext: string): boolean {
    return THUMBS.has(ext.toLowerCase());
  }
</script>

<script lang="ts">
  import CornerLeftUp from '@lucide/svelte/icons/corner-left-up';
  import FileIconLucide from '@lucide/svelte/icons/file';
  import Folder from '@lucide/svelte/icons/folder';
  import HardDrive from '@lucide/svelte/icons/hard-drive';
  import { joinLocal } from '../lib/paths';
  import { assetUrl, dpr, extIconUrl, kindIconUrl, pathIconUrl, thumbUrl } from '../lib/platform';
  import { app } from '../lib/state/app.svelte';
  import { F, type Entry, type Location } from '../lib/types';

  let { entry, location, size = 16, thumb = false }: { entry: Entry; location: Location; size?: number; thumb?: boolean } = $props();

  const px = $derived(Math.round(size * dpr()));
  const ext = $derived(entry.ext.toLowerCase());
  const special = $derived(new Set(app.info.places.map((p) => p.path.toLowerCase())));
  const localPath = $derived(
    entry.path ?? (location.kind === 'local' ? joinLocal(location.path, entry.name) : location.kind === 'drives' ? entry.name : null),
  );

  const src = $derived.by(() => {
    if (entry.flags & F.UP) return null;
    if (entry.flags & F.VOLUME) return pathIconUrl(entry.name, px);
    if (localPath && (entry.flags & F.PACKAGE || PER_FILE.has(ext) || (entry.isDir && special.has(localPath.toLowerCase())))) {
      return pathIconUrl(localPath, px);
    }
    if (entry.isDir) return kindIconUrl('dir', px);
    return ext ? extIconUrl(ext, px) : kindIconUrl('file', px);
  });
  // SVG renders crisply straight from disk; everything else goes through the thumbnail pipeline.
  const thumbSrc = $derived(
    !thumb || !localPath || entry.isDir || !hasThumbnail(ext) ? null : ext === 'svg' ? assetUrl(localPath) : thumbUrl(localPath, px, entry.mtime),
  );

  let failed = $state<string | null>(null);
  let thumbFailed = $state<string | null>(null);
</script>

{#if entry.flags & F.UP}
  <span class="glyph" style="width:{size}px;height:{size}px"><CornerLeftUp size={Math.round(size > 32 ? Math.max(28, size * 0.38) : size * 0.85)} strokeWidth={size > 32 ? 1.5 : 1.75} /></span>
{:else if thumbSrc && thumbFailed !== thumbSrc}
  <img
    class="thumb"
    src={thumbSrc}
    alt=""
    style="width:{size}px;height:{size}px"
    loading="lazy"
    decoding="async"
    draggable="false"
    onerror={() => (thumbFailed = thumbSrc)}
  />
{:else if src && failed !== src}
  <img {src} alt="" width={size} height={size} decoding="async" draggable="false" onerror={() => (failed = src)} />
{:else}
  <span class="glyph" style="width:{size}px;height:{size}px">
    {#if entry.flags & F.VOLUME}
      <HardDrive size={size} strokeWidth={1.5} />
    {:else if entry.isDir}
      <Folder size={size} strokeWidth={1.5} fill="currentColor" fill-opacity="0.15" />
    {:else}
      <FileIconLucide size={size} strokeWidth={1.5} />
    {/if}
  </span>
{/if}

<style>
  img {
    display: block;
    flex: none;
    object-fit: contain;
  }
  .thumb {
    object-fit: contain;
    border-radius: 3px;
  }
  .glyph {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    flex: none;
    color: var(--text-2);
  }
</style>
