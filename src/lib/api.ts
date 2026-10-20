import { Channel, invoke } from '@tauri-apps/api/core';
import type {
  AppInfo,
  ChecksumEvent,
  Connected,
  DirSize,
  DocumentPreview,
  FileMeta,
  IndexConfig,
  IndexResults,
  IndexStatus,
  SheetPreview,
  SlidePreview,
  JobRequest,
  JobSnapshot,
  Answer,
  Listing,
  Location,
  MenuNode,
  RenameOutcome,
  RenamePair,
  SearchEvent,
  SearchParams,
  SiteInput,
  SiteSummary,
  Space,
  TextChunk,
  VaultStatus,
  ViewerProbe,
  Volume,
} from './types';

/** Locations the backend understands (search results are frontend-only). */
export type BackendLocation = Exclude<Location, { kind: 'search' } | { kind: 'query' }>;

export const api = {
  appInfo: () => invoke<AppInfo>('app_info'),
  setMenu: (spec: MenuNode[]) => invoke<void>('set_menu', { spec }),
  storeLoad: <T>(name: string) => invoke<T | null>('store_load', { name }),
  storeSave: (name: string, value: unknown) => invoke<void>('store_save', { name, value }),
  watchDirs: (dirs: string[]) => invoke<void>('watch_dirs', { dirs }),
  cancelOp: (id: number) => invoke<void>('cancel_op', { id }),
  exit: () => invoke<void>('app_exit'),
  activeJobs: () => invoke<number>('active_jobs'),

  openPath: (path: string) => invoke<void>('open_path', { path }),
  revealPath: (path: string) => invoke<void>('reveal_path', { path }),
  openUrl: (url: string) => invoke<void>('open_url', { url }),
  qrSvg: (text: string) => invoke<string>('qr_svg', { text }),
  openTerminal: (dir: string, custom?: string | null) => invoke<void>('open_terminal', { dir, custom: custom || null }),
  editFile: (path: string, editor?: string | null) => invoke<void>('edit_file', { path, editor: editor || null }),
  openWith: (path: string, app: string) => invoke<void>('open_with', { path, app }),
  quickLook: (path: string) => invoke<void>('quick_look', { path }),
  showProperties: (path: string) => invoke<void>('show_properties', { path }),

  listDir: (location: BackendLocation) => invoke<Listing>('list_dir', { location }),
  volumes: () => invoke<Volume[]>('volumes'),
  space: (path: string) => invoke<Space | null>('space', { path }),
  dirSize: (path: string) => invoke<DirSize>('dir_size', { path }),
  resolvePath: (input: string, base?: string | null) =>
    invoke<{ location: Location; focus: string | null }>('resolve_path', { input, base: base ?? null }),
  completePath: (input: string, base: string | null, showHidden: boolean) =>
    invoke<string[]>('complete_path', { input, base, showHidden }),
  makeDir: (dir: BackendLocation, name: string) => invoke<string>('make_dir', { dir, name }),
  createFile: (dir: BackendLocation, name: string) => invoke<string>('create_file', { dir, name }),
  renameItem: (dir: BackendLocation, from: string, to: string) => invoke<void>('rename_item', { dir, from, to }),
  batchRename: (dir: string, pairs: RenamePair[]) => invoke<RenameOutcome[]>('batch_rename', { dir, pairs }),
  createLink: (target: string, dir: string, name: string, hard: boolean) =>
    invoke<void>('create_link', { target, dir, name, hard }),
  writeTextFile: (path: string, content: string) => invoke<void>('write_text_file', { path, content }),
  materialize: (location: BackendLocation) => invoke<string>('materialize', { location }),

  jobEnqueue: (request: JobRequest) => invoke<number>('job_enqueue', { request }),
  jobsList: () => invoke<JobSnapshot[]>('jobs_list'),
  jobPause: (id: number, paused: boolean) => invoke<void>('job_pause', { id, paused }),
  jobCancel: (id: number) => invoke<void>('job_cancel', { id }),
  jobAnswer: (id: number, answer: Answer) => invoke<void>('job_answer', { id, answer }),
  jobStartNow: (id: number) => invoke<void>('job_start_now', { id }),
  jobMove: (id: number, delta: number) => invoke<void>('job_move', { id, delta }),
  jobsClearFinished: () => invoke<void>('jobs_clear_finished'),

  searchStart: (params: SearchParams, onEvent: (e: SearchEvent) => void) => {
    const channel = new Channel<SearchEvent>();
    channel.onmessage = onEvent;
    return invoke<number>('search_start', { params, channel });
  },
  viewerProbe: (path: string) => invoke<ViewerProbe>('viewer_probe', { path }),
  viewerReadText: (path: string, offset: number, max: number, encoding: string) =>
    invoke<TextChunk>('viewer_read_text', { path, offset, max, encoding }),
  previewSheet: (path: string, sheet: number, maxRows: number) => invoke<SheetPreview>('preview_sheet', { path, sheet, maxRows }),
  previewDocument: (path: string) => invoke<DocumentPreview>('preview_document', { path }),
  previewSlides: (path: string) => invoke<SlidePreview[]>('preview_slides', { path }),
  viewerReadBytes: (path: string, offset: number, len: number) =>
    invoke<ArrayBuffer>('viewer_read_bytes', { path, offset, len }),
  iconPng: (kind: 'ext' | 'dir' | 'file' | 'path', value: string, px: number) =>
    invoke<ArrayBuffer>('icon_png', { kind, value, px }),
  checksumStart: (paths: string[], algo: string, onEvent: (e: ChecksumEvent) => void) => {
    const channel = new Channel<ChecksumEvent>();
    channel.onmessage = onEvent;
    return invoke<number>('checksum_start', { paths, algo, channel });
  },

  vaultStatus: () => invoke<VaultStatus>('vault_status'),
  vaultUnlock: (password: string) => invoke<void>('vault_unlock', { password }),
  vaultLock: () => invoke<void>('vault_lock'),
  vaultSetMaster: (password: string | null) => invoke<void>('vault_set_master', { password }),
  indexConfigure: (config: IndexConfig) => invoke<void>('index_configure', { config }),
  indexStatus: () => invoke<IndexStatus>('index_status'),
  indexDefaults: () => invoke<{ excludes: string }>('index_defaults'),
  indexRescan: () => invoke<void>('index_rescan'),
  indexSearch: (query: string, limit: number, under: string | null = null) => invoke<IndexResults>('index_search', { query, limit, under }),
  fileMeta: (paths: string[]) => invoke<(FileMeta | null)[]>('file_meta', { paths }),
  queryCheck: (query: string) => invoke<void>('query_check', { query }),
  queryFilter: (query: string, items: [string, boolean, number, number][]) => invoke<boolean[]>('query_filter', { query, items }),
  sitesList: () => invoke<SiteSummary[]>('sites_list'),
  siteSave: (site: SiteInput) => invoke<SiteSummary>('site_save', { site }),
  siteDelete: (id: string) => invoke<void>('site_delete', { id }),
  ftpConnect: (siteId: string | null, site: SiteInput | null) =>
    invoke<Connected>('ftp_connect', { siteId, site }),
  ftpTest: (site: SiteInput) => invoke<string>('ftp_test', { site }),
  ftpDisconnect: (session: string) => invoke<void>('ftp_disconnect', { session }),
};
