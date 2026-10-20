export type Location =
  | { kind: 'local'; path: string }
  | { kind: 'ftp'; session: string; path: string }
  | { kind: 'archive'; archive: string; inner: string }
  | { kind: 'drives' }
  /** Frontend only: search results fed into a panel. */
  | { kind: 'search'; id: string; label: string };

export type RawEntry = [name: string, flags: number, size: number, mtime: number, attrs: number];

export const F = {
  DIR: 1,
  SYMLINK: 2,
  HIDDEN: 4,
  READONLY: 8,
  SYSTEM: 16,
  EXEC: 32,
  BROKEN: 64,
  PACKAGE: 128,
  VOLUME: 256,
  /** Frontend only: the ".." entry. */
  UP: 1 << 16,
} as const;

export interface Entry {
  name: string;
  flags: number;
  size: number;
  mtime: number;
  attrs: number;
  ext: string;
  isDir: boolean;
  /** Unique key inside the listing (the name, or the full path for search results). */
  key: string;
  /** Full path (search results only). */
  path?: string;
  /** Extra text shown for search results (folder / matched line). */
  note?: string;
}

export interface Crumb {
  label: string;
  icon: 'drives' | 'volume' | 'home' | 'server' | 'archive' | 'folder';
  location: Location;
}

export interface Listing {
  location: Location;
  displayPath: string;
  parent: Location | null;
  crumbs: Crumb[];
  entries: RawEntry[];
}

export type ErrorCode =
  | 'NotFound'
  | 'PermissionDenied'
  | 'AlreadyExists'
  | 'NotADirectory'
  | 'IsADirectory'
  | 'DirectoryNotEmpty'
  | 'InvalidName'
  | 'CrossesDevices'
  | 'StorageFull'
  | 'ReadOnly'
  | 'Busy'
  | 'Cancelled'
  | 'IntoItself'
  | 'VerifyFailed'
  | 'Ftp'
  | 'FtpConnect'
  | 'FtpAuth'
  | 'VaultLocked'
  | 'VaultWrongPassword'
  | 'Keychain'
  | 'Unsupported'
  | 'InvalidInput'
  | 'Archive'
  | 'Io'
  | 'Other';

export interface AppError {
  code: ErrorCode;
  message: string;
  path?: string;
}

export interface Place {
  key: 'home' | 'desktop' | 'documents' | 'downloads' | 'pictures' | 'music' | 'videos' | 'applications';
  path: string;
}

export interface AppInfo {
  platform: 'macos' | 'windows' | 'linux';
  version: string;
  home: string;
  places: Place[];
  accent: string | null;
  locale: string;
  effects: 'vibrancy' | 'mica' | 'none';
  osBuild: number;
  separator: string;
}

export interface Volume {
  name: string;
  path: string;
  total: number;
  free: number;
  kind: 'system' | 'fixed' | 'removable' | 'network' | 'cdrom' | 'ram';
}

export interface Space {
  total: number;
  free: number;
}

export interface DirSize {
  bytes: number;
  files: number;
  dirs: number;
}

export type JobOp = 'copy' | 'move' | 'delete' | 'pack' | 'extract';
export type ConflictPolicy = 'ask' | 'overwrite' | 'skip' | 'overwriteOlder' | 'rename';
export type JobStatus = 'queued' | 'scanning' | 'running' | 'paused' | 'waiting' | 'done' | 'failed' | 'cancelled';

export interface JobOptions {
  conflict?: ConflictPolicy;
  verify?: boolean;
  preserveTimes?: boolean;
  followLinks?: boolean;
  filter?: string | null;
  toTrash?: boolean;
  speedLimit?: number | null;
  level?: number | null;
}

export interface JobRequest {
  op: JobOp;
  sources: Location[];
  dest?: Location | null;
  newName?: string | null;
  options?: JobOptions;
  queue?: number | null;
}

export interface FileBrief {
  path: string;
  size: number;
  mtime: number;
  isDir: boolean;
}

export type Prompt =
  | { kind: 'conflict'; source: FileBrief; target: FileBrief; suggested: string }
  | { kind: 'error'; error: AppError; canRetry: boolean };

export type Answer =
  | { action: 'overwrite'; all: boolean }
  | { action: 'skip'; all: boolean }
  | { action: 'overwriteOlder' }
  | { action: 'rename'; name?: string | null; all?: boolean }
  | { action: 'retry' }
  | { action: 'cancel' };

export interface JobSnapshot {
  id: number;
  op: JobOp;
  status: JobStatus;
  queue: number | null;
  firstSource: string;
  sourceCount: number;
  dest: string;
  toTrash: boolean;
  totalBytes: number;
  doneBytes: number;
  totalFiles: number;
  doneFiles: number;
  current: string;
  speed: number;
  eta: number | null;
  error: AppError | null;
  skipped: number;
  errors: number;
  prompt: Prompt | null;
  created: number;
  started: number | null;
  finished: number | null;
}

export interface JobDone {
  id: number;
  op: JobOp;
  status: JobStatus;
  touched: Location[];
}

export interface SearchParams {
  roots: string[];
  name: string;
  nameRegex: boolean;
  nameCase: boolean;
  text: string;
  textRegex: boolean;
  textCase: boolean;
  textWord: boolean;
  textNot: boolean;
  encoding: string;
  binary: boolean;
  minSize: number | null;
  maxSize: number | null;
  newerThan: number | null;
  olderThan: number | null;
  maxDepth: number | null;
  hidden: boolean;
  followLinks: boolean;
  gitignore: boolean;
  excludeDirs: string;
  kind: 'all' | 'files' | 'dirs';
  maxResults: number | null;
  /** Query language filter (see query.rs). */
  query?: string;
}

export interface SearchHit {
  path: string;
  name: string;
  isDir: boolean;
  size: number;
  mtime: number;
  matches: { line: number; text: string }[];
  matchCount: number;
}

export type SearchEvent =
  | { type: 'hits'; items: SearchHit[] }
  | { type: 'progress'; scanned: number; current: string }
  | { type: 'done'; found: number; scanned: number; cancelled: boolean; truncated: boolean; elapsedMs: number };

export type FtpProtocol = 'ftp' | 'ftpsExplicit' | 'ftpsImplicit';

export interface SiteSummary {
  id: string;
  name: string;
  group: string;
  protocol: FtpProtocol;
  host: string;
  port: number;
  username: string;
  hasPassword: boolean;
  passive: boolean;
  encoding: string;
  acceptInvalidCerts: boolean;
  remotePath: string;
  localPath: string;
  notes: string;
  lastUsed: number;
}

export interface SiteInput {
  id?: string | null;
  name: string;
  group: string;
  protocol: FtpProtocol;
  host: string;
  port: number;
  username: string;
  /** undefined/null keeps the stored password */
  password?: string | null;
  passive: boolean;
  encoding: string;
  acceptInvalidCerts: boolean;
  remotePath: string;
  localPath: string;
  notes: string;
}

export interface VaultStatus {
  exists: boolean;
  protection: 'keychain' | 'master' | 'none';
  unlocked: boolean;
  keychainAvailable: boolean;
}

export interface Connected {
  session: string;
  label: string;
  path: string;
  welcome: string | null;
}

export type PreviewKind = 'text' | 'binary' | 'image' | 'video' | 'audio' | 'pdf' | 'render' | 'sheet' | 'document' | 'slides' | 'font';

export interface ViewerProbe {
  size: number;
  mtime: number;
  kind: PreviewKind;
  encoding: string;
  /** Raw content is readable text (Text mode makes sense). */
  textual: boolean;
}

export interface SheetPreview {
  sheets: string[];
  sheet: number;
  rows: string[][];
  startRow: number;
  startCol: number;
  totalRows: number;
  totalCols: number;
  truncated: boolean;
}

export interface DocBlock {
  kind: 'h1' | 'h2' | 'h3' | 'p' | 'li' | 'table';
  text: string;
  rows?: string[][];
}

export interface DocumentPreview {
  blocks: DocBlock[];
  truncated: boolean;
}

export interface SlidePreview {
  title: string;
  lines: string[];
}

export interface TextChunk {
  text: string;
  start: number;
  end: number;
  eof: boolean;
}

export type ChecksumEvent =
  | { type: 'progress'; doneBytes: number; totalBytes: number; current: string }
  | { type: 'result'; path: string; hash: string | null; error: AppError | null }
  | { type: 'done'; cancelled: boolean };

export interface RenamePair {
  from: string;
  to: string;
}

export interface RenameOutcome {
  from: string;
  to: string;
  error: AppError | null;
}

export type MenuNode =
  | { type: 'item'; id: string; label: string; accel?: string | null; enabled?: boolean }
  | { type: 'check'; id: string; label: string; accel?: string | null; checked: boolean; enabled?: boolean }
  | { type: 'separator' }
  | { type: 'submenu'; label: string; items: MenuNode[]; role?: 'window' | 'help' | null }
  | { type: 'predefined'; name: string; label?: string | null };

export type ViewMode = 'details' | 'list' | 'icons' | 'thumbs';
/** Built-in keys, or `m:<column>` for metadata columns. */
export type SortKey = 'name' | 'ext' | 'size' | 'mtime' | `m:${string}`;

export interface FileMeta {
  width?: number;
  height?: number;
  duration?: number;
  videoCodec?: string;
  audioCodec?: string;
  fps?: number;
  bitrate?: number;
  sampleRate?: number;
  channels?: number;
  camera?: string;
  lens?: string;
  taken?: number;
  iso?: number;
  focal?: number;
  aperture?: number;
  exposure?: string;
  gps?: [number, number];
  artist?: string;
  album?: string;
  title?: string;
  genre?: string;
  year?: number;
  track?: number;
  pages?: number;
  author?: string;
  lines?: number;
}
