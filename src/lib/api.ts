import { Channel, invoke } from '@tauri-apps/api/core';
import type {
  AppInfo,
  ChecksumEvent,
  Connected,
  DirSize,
  DocumentPreview,
  FileMeta,
  IndexConfig,
  JobPlan,
  Rule,
  RuleActivity,
  SemanticHit,
  SemanticStatus,
  DuNode,
  DupeEvent,
  DupeParams,
  UndoAction,
  UndoReport,
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
  RemoteSource,
  SiteInput,
  SiteSummary,
  Space,
  TextChunk,
  VaultStatus,
  ViewerProbe,
  Volume,
} from './types';

export interface SemanticConfig {
  enabled: boolean;
  endpoint: string;
  model: string;
  roots: string[];
  images: boolean;
  visionModel: string;
  assistantModel: string;
}

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
  shellMenu: (paths: string[], allDirs: boolean, title: string, items: { id: string; label?: string; enabled?: boolean }[]) =>
    invoke<string | null>('shell_menu', { paths, allDirs, title, items }),
  openWithApps: (path: string) => invoke<{ name: string; path: string; default: boolean; icon: number[] }[]>('open_with_apps', { path }),
  networkConnect: (host: string, share: string | null) => invoke<string>('network_connect', { host, share }),
  clipboardText: () => invoke<string>('clipboard_text'),
  dragOut: (paths: string[]) => invoke<void>('drag_out', { paths }),
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
  /** First channel message is the op id (for cancelOp), then folder counts. */
  duScan: (path: string, onProgress: (n: number) => void) => {
    const channel = new Channel<number>();
    channel.onmessage = onProgress;
    return invoke<DuNode>('du_scan', { path, channel });
  },
  duTree: (path: string) => invoke<DuNode>('du_tree', { path }),
  dupesStart: (params: DupeParams, onEvent: (e: DupeEvent) => void) => {
    const channel = new Channel<DupeEvent>();
    channel.onmessage = onEvent;
    return invoke<number>('dupes_start', { params, channel });
  },
  semanticConfigure: (config: SemanticConfig) => invoke<void>('semantic_configure', { config }),
  semanticStatus: () => invoke<SemanticStatus>('semantic_status'),
  semanticSearch: (query: string, limit: number) => invoke<SemanticHit[]>('semantic_search', { query, limit }),
  semanticAsk: (text: string, limit: number) =>
    invoke<{ interpretation: { query: string; topic: string; via: 'llm' | 'rules' }; hits: SemanticHit[] }>('semantic_ask', { text, limit }),
  semanticRebuild: () => invoke<void>('semantic_rebuild'),
  semanticPull: (model: string) => invoke<void>('semantic_pull', { model }),
  rulesConfigure: (config: { enabled: boolean; rules: Rule[] }) => invoke<void>('rules_configure', { config }),
  rulesCheck: (rule: Rule) => invoke<void>('rules_check', { rule }),
  rulesRun: (rule: Rule, dry: boolean) => invoke<RuleActivity[]>('rules_run', { rule, dry }),
  rulesLog: () => invoke<RuleActivity[]>('rules_log'),
  archivePassword: (archive: string, password: string) => invoke<void>('archive_password', { archive, password }),
  archiveCheck: (archive: string) => invoke<void>('archive_check', { archive }),
  mountImage: (path: string) => invoke<string>('mount_image', { path }),
  jobPlan: (req: JobRequest) => invoke<JobPlan>('job_plan', { req }),
  undoApply: (actions: UndoAction[]) => invoke<UndoReport>('undo_apply', { actions }),
  trashItem: (path: string) => invoke<UndoAction>('trash_item', { path }),
  indexConfigure: (config: IndexConfig) => invoke<void>('index_configure', { config }),
  indexStatus: () => invoke<IndexStatus>('index_status'),
  indexDefaults: () => invoke<{ excludes: string; cloud: string[] }>('index_defaults'),
  indexRescan: () => invoke<void>('index_rescan'),
  indexSearch: (query: string, limit: number, under: string | null = null) => invoke<IndexResults>('index_search', { query, limit, under }),
  fileMeta: (paths: string[]) => invoke<(FileMeta | null)[]>('file_meta', { paths }),
  queryCheck: (query: string) => invoke<void>('query_check', { query }),
  queryFilter: (query: string, items: [string, boolean, number, number][]) => invoke<boolean[]>('query_filter', { query, items }),
  sitesList: () => invoke<SiteSummary[]>('sites_list'),
  siteSave: (site: SiteInput) => invoke<SiteSummary>('site_save', { site }),
  siteDelete: (id: string) => invoke<void>('site_delete', { id }),
  remoteConnect: (siteId: string | null, site: SiteInput | null, trustKey: string | null = null) =>
    invoke<Connected>('remote_connect', { siteId, site, trustKey }),
  remoteTest: (site: SiteInput, trustKey: string | null = null) => invoke<string>('remote_test', { site, trustKey }),
  remoteDisconnect: (session: string) => invoke<void>('remote_disconnect', { session }),
  remoteSources: () => invoke<RemoteSource[]>('remote_sources'),
  remoteOpenSource: (s: RemoteSource) => invoke<Connected>('remote_open_source', { protocol: s.protocol, id: s.id, name: s.name }),
};
