import { api } from '../api';
import { sameLoc } from '../paths';
import type { AppInfo, JobSnapshot, Location } from '../types';
import { settings } from './settings.svelte';
import { Tab, type TabSnapshot } from './tab.svelte';

export type SideName = 'left' | 'right';

export class Side {
  tabs = $state<Tab[]>([]);
  active = $state(0);
  tab = $derived<Tab>(this.tabs[Math.min(this.active, this.tabs.length - 1)]);

  constructor(tabs: Tab[]) {
    for (const t of tabs) t.side = this;
    // Locked tabs always come first.
    this.tabs = [...tabs.filter((t) => t.locked), ...tabs.filter((t) => !t.locked)];
  }

  get lockedCount(): number {
    let n = 0;
    for (const t of this.tabs) if (t.locked) n++;
    return n;
  }

  private setTabs(tabs: Tab[], activeTab: Tab) {
    this.tabs = tabs;
    this.active = Math.max(0, tabs.indexOf(activeTab));
  }

  addTab(tab: Tab, activate = true) {
    tab.side = this;
    const current = this.tabs[this.active];
    const at = tab.locked ? this.lockedCount : Math.max(this.active + 1, this.lockedCount);
    const tabs = [...this.tabs.slice(0, at), tab, ...this.tabs.slice(at)];
    this.setTabs(tabs, activate ? tab : current);
  }

  /** Returns false when the tab is locked (it has to be unlocked first). */
  closeTab(index = this.active): boolean {
    const tab = this.tabs[index];
    if (!tab || tab.locked || this.tabs.length <= 1) return !tab?.locked;
    const current = this.tabs[this.active];
    const tabs = this.tabs.filter((_, i) => i !== index);
    if (current === tab) {
      this.tabs = tabs;
      this.active = Math.min(index, tabs.length - 1);
    } else this.setTabs(tabs, current);
    return true;
  }

  closeOthers(index = this.active) {
    const keep = this.tabs[index];
    this.setTabs(this.tabs.filter((t) => t === keep || t.locked), keep);
  }

  setLocked(index: number, locked: boolean) {
    const tab = this.tabs[index];
    if (!tab) return;
    const current = this.tabs[this.active];
    tab.locked = locked;
    // Locked: last of the locked group. Unlocked: first after it.
    const rest = this.tabs.filter((t) => t !== tab);
    this.setTabs([...rest.filter((t) => t.locked), tab, ...rest.filter((t) => !t.locked)], current);
  }

  moveTab(from: number, to: number) {
    const tab = this.tabs[from];
    if (!tab) return;
    // A tab cannot leave its group (locked tabs stay in front).
    const locked = this.lockedCount;
    to = tab.locked ? Math.min(to, locked - 1) : Math.max(to, locked);
    const t = [...this.tabs];
    t.splice(from, 1);
    t.splice(to, 0, tab);
    this.setTabs(t, this.tabs[this.active]);
  }
}

export interface FileClipboard {
  op: 'copy' | 'cut';
  items: Location[];
  names: string[];
}

class AppState {
  info = $state.raw<AppInfo>({
    platform: 'macos',
    version: '0.0.0',
    home: '/',
    places: [],
    accent: null,
    locale: 'en-US',
    effects: 'none',
    osBuild: 0,
    separator: '/',
  });
  left = $state.raw(new Side([new Tab()]));
  right = $state.raw(new Side([new Tab()]));
  activeSide = $state<SideName>('left');
  jobs = $state.raw<JobSnapshot[]>([]);
  showJobs = $state(false);
  quickView = $state(false);
  clipboard = $state.raw<FileClipboard | null>(null);
  ready = $state(false);
  /** Panel being edited in the address bar ("left"/"right"), focuses its input. */
  editPath = $state<SideName | null>(null);
  /** Drive/places popup requested for a side. */
  placesFor = $state<SideName | null>(null);

  side(name: SideName): Side {
    return name === 'left' ? this.left : this.right;
  }

  get activeSideObj(): Side {
    return this.side(this.activeSide);
  }

  get otherSide(): SideName {
    return this.activeSide === 'left' ? 'right' : 'left';
  }

  get panel(): Tab {
    return this.side(this.activeSide).tab;
  }

  get other(): Tab {
    return this.side(this.otherSide).tab;
  }

  /** Tabs currently on screen (one per side). */
  visibleTabs(): Tab[] {
    return [this.left.tab, this.right.tab];
  }

  allTabs(): Tab[] {
    return [...this.left.tabs, ...this.right.tabs];
  }

  /** Refreshes every visible tab showing `loc`. */
  refreshLocation(loc: Location) {
    for (const t of this.visibleTabs()) if (sameLoc(t.location, loc)) void t.reload();
  }

  get activeJobs(): JobSnapshot[] {
    return this.jobs.filter((j) => !['done', 'failed', 'cancelled'].includes(j.status));
  }
}

export const app = new AppState();

// ------------------------------------------------------------------ session

interface SessionSide {
  tabs: TabSnapshot[];
  active: number;
}

interface Session {
  left: SessionSide;
  right: SessionSide;
  activeSide: SideName;
}

export function sessionSnapshot(): Session {
  const side = (s: Side): SessionSide => ({ tabs: s.tabs.map((t) => t.snapshot()), active: s.active });
  return { left: side(app.left), right: side(app.right), activeSide: app.activeSide };
}

/** Restores tabs from the saved session (or opens home/documents). */
export async function restoreSession(): Promise<void> {
  const saved = settings.restoreSession ? await api.storeLoad<Session>('session').catch(() => null) : null;
  const home: Location = { kind: 'local', path: app.info.home };
  const docs = app.info.places.find((p) => p.key === 'documents' || p.key === 'downloads');
  const second: Location = docs ? { kind: 'local', path: docs.path } : home;

  const build = (s: SessionSide | undefined, fallback: Location): Side => {
    const snaps = s?.tabs?.filter((t) => t.location && (t.location.kind === 'local' || t.location.kind === 'drives' || t.location.kind === 'archive')) ?? [];
    const tabs = snaps.length ? snaps.map((sn) => new Tab(sn)) : [new Tab({ location: fallback })];
    const side = new Side(tabs);
    side.active = Math.min(Math.max(0, s?.active ?? 0), tabs.length - 1);
    return side;
  };
  app.left = build(saved?.left, home);
  app.right = build(saved?.right, second);
  app.activeSide = saved?.activeSide === 'right' ? 'right' : 'left';

  // Load the visible tabs first, the rest lazily when activated.
  const load = async (tab: Tab, fallback: Location) => {
    if (!(await tab.navigate(tab.location))) await tab.navigate(fallback);
  };
  await Promise.all([load(app.left.tab, home), load(app.right.tab, second)]);
}
