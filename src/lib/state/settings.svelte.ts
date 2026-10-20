import type { SizeFormat } from '../format';
import type { Location, Rule, SortKey, ViewMode } from '../types';

export interface HotlistItem {
  name: string;
  location: Location;
}

export interface Settings {
  lang: 'auto' | 'cs' | 'en';
  theme: 'system' | 'light' | 'dark';
  density: 'compact' | 'normal' | 'comfortable';
  fontSize: number;
  showHidden: boolean;
  showUp: boolean;
  /** Show the tab bar even with a single tab. */
  alwaysShowTabs: boolean;
  /** Drive buttons above each panel. */
  driveBar: boolean;
  /** How many lines a file name may take in the lists (1 = never wrap). */
  nameLines: number;
  /** Show related files (icon.png + icon.psd …) as one group. */
  groupFiles: boolean;
  /** Metadata columns (dimensions, duration, artist …) in the details view. */
  metaColumns: boolean;
  /** Columns chosen by hand for a folder (path -> column ids); others use the automatic preset. */
  folderColumns: Record<string, string[]>;
  /** File index for instant search. `roots` empty = home folder, `excludes` null = defaults. */
  index: { enabled: boolean; roots: string[]; excludes: string | null; hidden: boolean; meta: boolean; skipCloud: boolean };
  /** Semantic (AI) search through a local Ollama. `roots` empty = Documents, Desktop, Downloads. */
  semantic: { enabled: boolean; endpoint: string; model: string; roots: string[]; images: boolean; visionModel: string; assistantModel: string };
  /** Folder rules (automatic sorting). */
  rules: { enabled: boolean; list: Rule[] };
  /** Column widths of the search results (px). */
  searchWidths: Record<string, number>;
  /** Widths of metadata columns (id -> px). */
  metaWidths: Record<string, number>;
  functionBar: boolean;
  toolbar: boolean;
  restoreSession: boolean;
  confirmDelete: boolean;
  useTrash: boolean;
  confirmCopy: boolean;
  verify: boolean;
  editor: string;
  terminal: string;
  sizeFormat: SizeFormat;
  typeAhead: 'jump' | 'filter';
  thumbSize: number;
  defaultView: ViewMode;
  defaultSort: SortKey;
  /** Column widths in px; `name` 0 = fill the remaining space. */
  colWidths: { name: number; folder: number; ext: number; size: number; date: number; attrs: number };
  showAttrs: boolean;
  hotlist: HotlistItem[];
  split: number;
  /** User overrides of keyboard shortcuts: command id -> combos. */
  keymap: Record<string, string[]>;
}

export const defaultSettings = (): Settings => ({
  lang: 'auto',
  theme: 'system',
  density: 'normal',
  fontSize: 0,
  showHidden: false,
  showUp: true,
  alwaysShowTabs: true,
  driveBar: true,
  metaColumns: true,
  groupFiles: true,
  nameLines: 1,
  folderColumns: {},
  metaWidths: {},
  searchWidths: {},
  rules: { enabled: true, list: [] },
  semantic: { enabled: false, endpoint: 'http://localhost:11434', model: 'embeddinggemma', roots: [], images: false, visionModel: 'gemma3:4b', assistantModel: 'gemma3:4b' },
  index: { enabled: true, roots: [], excludes: null, hidden: false, meta: true, skipCloud: false },
  functionBar: true,
  toolbar: true,
  restoreSession: true,
  confirmDelete: true,
  useTrash: true,
  confirmCopy: true,
  verify: false,
  editor: '',
  terminal: '',
  sizeFormat: 'auto',
  typeAhead: 'jump',
  thumbSize: 128,
  defaultView: 'details',
  defaultSort: 'name',
  colWidths: { name: 0, folder: 220, ext: 64, size: 92, date: 136, attrs: 84 },
  showAttrs: false,
  hotlist: [],
  split: 0.5,
  keymap: {},
});

export const settings: Settings = $state(defaultSettings());

export function applySettings(saved: Partial<Settings> | null) {
  if (!saved) return;
  const d = defaultSettings();
  for (const k of Object.keys(d) as (keyof Settings)[]) {
    if (saved[k] !== undefined && typeof saved[k] === typeof d[k]) {
      (settings as unknown as Record<string, unknown>)[k] = saved[k];
    }
  }
  settings.colWidths = { ...d.colWidths, ...(saved.colWidths ?? {}) };
  settings.rules = { ...d.rules, ...(saved.rules && typeof saved.rules === 'object' ? saved.rules : {}) };
  if (!Array.isArray(settings.rules.list)) settings.rules.list = [];
  settings.semantic = { ...d.semantic, ...(saved.semantic && typeof saved.semantic === 'object' ? saved.semantic : {}) };
  settings.index = { ...d.index, ...(saved.index && typeof saved.index === 'object' ? saved.index : {}) };
  if (saved.keymap && typeof saved.keymap === 'object' && !Array.isArray(saved.keymap)) {
    const km: Record<string, string[]> = {};
    for (const [k, v] of Object.entries(saved.keymap)) if (Array.isArray(v)) km[k] = v.filter((x) => typeof x === 'string');
    settings.keymap = km;
  }
}
