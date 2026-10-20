//! File index for instant search: loaded from disk at start, refreshed by a
//! background rescan, kept current by file system notifications (FSEvents /
//! ReadDirectoryChangesW / inotify) and enriched with metadata of media files.

pub mod data;
pub mod scan;

use crate::error::{AppError, AppResult};
use crate::jobs::EventSink;
use crate::meta::{self, FileMeta};
use crate::query::Query;
use data::{flags, IndexData, NONE};
use notify::{RecommendedWatcher, RecursiveMode, Watcher};
use parking_lot::{Mutex, RwLock};
use scan::{Excludes, Progress};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// Folders that only hold caches, dependencies and VCS internals.
pub const DEFAULT_EXCLUDES: &str = if cfg!(target_os = "macos") {
    "node_modules;.git;.svn;.hg;.Trash;.cache;__pycache__;.venv;DerivedData;.gradle;.pub-cache;.npm;.cargo;.rustup;Pods;~/Library"
} else if cfg!(windows) {
    "node_modules;.git;.svn;.hg;$Recycle.Bin;__pycache__;.venv;.gradle;.pub-cache;.npm;.cargo;.rustup;~/AppData;C:\\Windows"
} else {
    "node_modules;.git;.svn;.hg;.cache;__pycache__;.venv;.gradle;.pub-cache;.npm;.cargo;.rustup;.local/share/Trash"
};

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct IndexConfig {
    pub enabled: bool,
    pub roots: Vec<String>,
    /// `;` separated names / absolute folders to skip.
    pub excludes: String,
    pub hidden: bool,
    /// Read metadata of media and documents in the background.
    pub meta: bool,
}

impl IndexConfig {
    /// Identifies what the stored index was built with.
    fn tag(&self) -> u64 {
        let mut h = std::collections::hash_map::DefaultHasher::new();
        (&self.roots, &self.excludes, self.hidden).hash(&mut h);
        h.finish()
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IndexStatus {
    /// "off" | "loading" | "scanning" | "ready"
    pub state: &'static str,
    /// Whether searching is possible (an index is loaded).
    pub ready: bool,
    pub files: usize,
    pub dirs: usize,
    pub scanned_dirs: usize,
    pub meta_done: usize,
    pub meta_total: usize,
    pub last_scan: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IndexHit {
    pub path: String,
    pub name: String,
    pub is_dir: bool,
    pub size: u64,
    pub mtime: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IndexResults {
    pub hits: Vec<IndexHit>,
    pub total: usize,
    pub elapsed_ms: u64,
}

pub struct IndexManager {
    data: RwLock<IndexData>,
    loaded: AtomicBool,
    config: Mutex<IndexConfig>,
    state: Mutex<&'static str>,
    generation: AtomicU64,
    progress: Mutex<Arc<Progress>>,
    meta_done: AtomicUsize,
    meta_total: AtomicUsize,
    last_scan: AtomicI64,
    dirty: AtomicBool,
    scanning: AtomicBool,
    file: PathBuf,
    watcher: Mutex<Option<RecommendedWatcher>>,
    pending: Arc<Mutex<HashSet<PathBuf>>>,
    /// Changes seen while a full rescan runs, replayed on the new index.
    replay: Mutex<Option<HashSet<PathBuf>>>,
    sink: EventSink,
}

impl IndexManager {
    pub fn new(file: PathBuf, sink: EventSink) -> Arc<Self> {
        let me = Arc::new(IndexManager {
            data: RwLock::new(IndexData::default()),
            loaded: AtomicBool::new(false),
            config: Mutex::new(IndexConfig::default()),
            state: Mutex::new("off"),
            generation: AtomicU64::new(0),
            progress: Mutex::new(Arc::new(Progress { dirs: AtomicUsize::new(0), cancel: AtomicBool::new(false) })),
            meta_done: AtomicUsize::new(0),
            meta_total: AtomicUsize::new(0),
            last_scan: AtomicI64::new(0),
            dirty: AtomicBool::new(false),
            scanning: AtomicBool::new(false),
            file,
            watcher: Mutex::new(None),
            pending: Arc::new(Mutex::new(HashSet::new())),
            replay: Mutex::new(None),
            sink,
        });
        // Applies file system changes and saves now and then.
        let weak = Arc::downgrade(&me);
        std::thread::Builder::new()
            .name("index-updates".into())
            .spawn(move || {
                let mut last_save = Instant::now();
                loop {
                    std::thread::sleep(Duration::from_millis(700));
                    let Some(me) = weak.upgrade() else { break };
                    let changed: Vec<PathBuf> = std::mem::take(&mut *me.pending.lock()).into_iter().collect();
                    if !changed.is_empty() {
                        if let Some(r) = me.replay.lock().as_mut() {
                            r.extend(changed.iter().cloned());
                        }
                        me.apply_changes(changed);
                        me.emit_status();
                    }
                    if me.dirty.load(Ordering::Relaxed) && last_save.elapsed() > Duration::from_secs(300) && !me.scanning.load(Ordering::Relaxed) {
                        me.save();
                        last_save = Instant::now();
                    }
                }
            })
            .expect("index thread");
        me
    }

    pub fn status(&self) -> IndexStatus {
        let d = self.data.read();
        IndexStatus {
            state: *self.state.lock(),
            ready: self.loaded.load(Ordering::Relaxed),
            files: d.files,
            dirs: d.dirs,
            scanned_dirs: self.progress.lock().dirs.load(Ordering::Relaxed),
            meta_done: self.meta_done.load(Ordering::Relaxed),
            meta_total: self.meta_total.load(Ordering::Relaxed),
            last_scan: self.last_scan.load(Ordering::Relaxed),
        }
    }

    fn emit_status(&self) {
        if let Ok(v) = serde_json::to_value(self.status()) {
            (self.sink)("index-status", v);
        }
    }

    fn set_state(&self, s: &'static str) {
        *self.state.lock() = s;
        self.emit_status();
    }

    fn current(&self, gen: u64) -> bool {
        self.generation.load(Ordering::Relaxed) == gen
    }

    /// Applies a new configuration (from the settings).
    pub fn configure(self: &Arc<Self>, cfg: IndexConfig) {
        {
            let mut cur = self.config.lock();
            if *cur == cfg && *self.state.lock() != "off" {
                return;
            }
            let rebuild = cur.tag() != cfg.tag() || cur.enabled != cfg.enabled || *self.state.lock() == "off";
            let meta_changed = cur.meta != cfg.meta;
            *cur = cfg.clone();
            if !rebuild {
                if meta_changed && cfg.meta {
                    let gen = self.generation.load(Ordering::Relaxed);
                    let me = self.clone();
                    std::thread::spawn(move || me.index_meta(gen));
                }
                return;
            }
        }
        let gen = self.generation.fetch_add(1, Ordering::Relaxed) + 1;
        self.progress.lock().cancel.store(true, Ordering::Relaxed);
        *self.watcher.lock() = None;
        if !cfg.enabled || cfg.roots.is_empty() {
            *self.data.write() = IndexData::default();
            self.loaded.store(false, Ordering::Relaxed);
            self.set_state("off");
            return;
        }
        let me = self.clone();
        std::thread::Builder::new()
            .name("index-build".into())
            .spawn(move || me.build(gen, cfg))
            .ok();
    }

    pub fn rescan(self: &Arc<Self>) {
        let cfg = self.config.lock().clone();
        if !cfg.enabled {
            return;
        }
        let gen = self.generation.load(Ordering::Relaxed);
        let me = self.clone();
        std::thread::spawn(move || me.full_scan(gen, &cfg));
    }

    fn roots(cfg: &IndexConfig) -> Vec<PathBuf> {
        let mut roots: Vec<PathBuf> = cfg.roots.iter().map(|r| crate::util::expand_user_path(r)).filter(|p| p.is_dir()).collect();
        roots.sort();
        roots.dedup();
        // Drop roots nested in other roots.
        let copy = roots.clone();
        roots.retain(|r| !copy.iter().any(|o| o != r && r.starts_with(o)));
        roots
    }

    fn build(self: Arc<Self>, gen: u64, cfg: IndexConfig) {
        self.set_state("loading");
        if let Ok(d) = IndexData::load(&self.file, cfg.tag()) {
            if !self.current(gen) {
                return;
            }
            self.meta_done.store(d.meta.len(), Ordering::Relaxed);
            *self.data.write() = d;
            self.loaded.store(true, Ordering::Relaxed);
            self.set_state("ready");
        }
        self.watch(&cfg);
        self.full_scan(gen, &cfg);
    }

    fn watch(self: &Arc<Self>, cfg: &IndexConfig) {
        let pending = self.pending.clone();
        let w = notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
            if let Ok(ev) = res {
                if matches!(ev.kind, notify::EventKind::Access(_)) {
                    return;
                }
                pending.lock().extend(ev.paths);
            }
        });
        if let Ok(mut w) = w {
            for r in Self::roots(cfg) {
                let _ = w.watch(&r, RecursiveMode::Recursive);
            }
            *self.watcher.lock() = Some(w);
        }
    }

    fn full_scan(self: &Arc<Self>, gen: u64, cfg: &IndexConfig) {
        if self.scanning.swap(true, Ordering::Relaxed) {
            return;
        }
        let progress = Arc::new(Progress { dirs: AtomicUsize::new(0), cancel: AtomicBool::new(false) });
        *self.progress.lock() = progress.clone();
        *self.replay.lock() = Some(HashSet::new());
        self.set_state("scanning");
        // Report progress while scanning.
        let ticker = {
            let me = Arc::downgrade(self);
            let p = progress.clone();
            std::thread::spawn(move || {
                while !p.cancel.load(Ordering::Relaxed) {
                    std::thread::sleep(Duration::from_millis(600));
                    match me.upgrade() {
                        Some(m) if m.scanning.load(Ordering::Relaxed) => m.emit_status(),
                        _ => break,
                    }
                }
            })
        };
        let roots = Self::roots(cfg);
        let mut fresh = scan::scan(&roots, &Excludes::parse(&cfg.excludes), cfg.hidden, &progress);
        let cancelled = progress.cancel.swap(true, Ordering::Relaxed);
        let _ = ticker.join();
        self.scanning.store(false, Ordering::Relaxed);
        let replay = self.replay.lock().take().unwrap_or_default();
        if cancelled || !self.current(gen) {
            return;
        }
        // Carry over metadata of unchanged files.
        {
            let old = self.data.read();
            if !old.meta.is_empty() {
                let by_path: HashMap<String, (u64, i64, &FileMeta)> = old
                    .meta
                    .iter()
                    .filter(|(id, _)| old.alive(**id))
                    .map(|(id, m)| (old.path(*id), (old.nodes[*id as usize].size, old.nodes[*id as usize].mtime, m)))
                    .collect();
                let mut carried = HashMap::new();
                for id in 0..fresh.nodes.len() as u32 {
                    if fresh.is_dir(id) {
                        continue;
                    }
                    let n = fresh.nodes[id as usize];
                    if let Some((s, m, meta)) = by_path.get(&fresh.path(id)) {
                        if *s == n.size && *m == n.mtime {
                            carried.insert(id, (*meta).clone());
                        }
                    }
                }
                fresh.meta = carried;
            }
        }
        self.meta_done.store(fresh.meta.len(), Ordering::Relaxed);
        *self.data.write() = fresh;
        self.loaded.store(true, Ordering::Relaxed);
        self.last_scan.store(crate::util::now_ms(), Ordering::Relaxed);
        self.apply_changes(replay.into_iter().collect());
        self.dirty.store(true, Ordering::Relaxed);
        self.save();
        self.set_state("ready");
        if cfg.meta {
            self.index_meta(gen);
        }
    }

    /// Reads metadata of media files that are not indexed yet.
    fn index_meta(self: &Arc<Self>, gen: u64) {
        let todo: Vec<(u32, String, u64, i64)> = {
            let d = self.data.read();
            (0..d.nodes.len() as u32)
                .filter(|&id| d.alive(id) && !d.is_dir(id) && !d.meta.contains_key(&id))
                .filter(|&id| {
                    let n = d.name(id);
                    let ext = n.rsplit_once('.').map(|(_, e)| e.to_lowercase()).unwrap_or_default();
                    matches!(meta::kind_of(&ext, false), "image" | "raw" | "video" | "audio" | "pdf" | "document" | "sheet" | "slides")
                })
                .map(|id| (id, d.path(id), d.nodes[id as usize].size, d.nodes[id as usize].mtime))
                .collect()
        };
        let done0 = self.meta_done.load(Ordering::Relaxed);
        self.meta_total.store(done0 + todo.len(), Ordering::Relaxed);
        let mut last_emit = Instant::now();
        for chunk in todo.chunks(64) {
            if !self.current(gen) || !self.config.lock().meta {
                return;
            }
            let out: Vec<(u32, u64, i64, FileMeta)> = std::thread::scope(|s| {
                let hs: Vec<_> = chunk
                    .chunks(16)
                    .map(|part| s.spawn(move || part.iter().map(|(id, p, sz, mt)| (*id, *sz, *mt, meta::get_known(Path::new(p), *sz, *mt))).collect::<Vec<_>>()))
                    .collect();
                hs.into_iter().flat_map(|h| h.join().unwrap_or_default()).collect()
            });
            {
                let mut d = self.data.write();
                for (id, sz, mt, m) in out {
                    let n = d.nodes.get(id as usize).copied();
                    if n.is_some_and(|n| n.flags & flags::DEAD == 0 && n.size == sz && n.mtime == mt) {
                        d.meta.insert(id, m);
                    }
                }
            }
            self.meta_done.fetch_add(chunk.len(), Ordering::Relaxed);
            self.dirty.store(true, Ordering::Relaxed);
            if last_emit.elapsed() > Duration::from_millis(800) {
                self.emit_status();
                last_emit = Instant::now();
            }
            // Stay in the background.
            std::thread::sleep(Duration::from_millis(5));
        }
        self.save();
        self.emit_status();
    }

    pub fn save(&self) {
        if !self.loaded.load(Ordering::Relaxed) || !self.dirty.swap(false, Ordering::Relaxed) {
            return;
        }
        let tag = self.config.lock().tag();
        let d = self.data.read();
        if let Some(dir) = self.file.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if d.save(&self.file, tag).is_err() {
            self.dirty.store(true, Ordering::Relaxed);
        }
    }

    // ---------------------------------------------------------------- live updates

    fn apply_changes(&self, changed: Vec<PathBuf>) {
        if changed.is_empty() || !self.loaded.load(Ordering::Relaxed) {
            return;
        }
        let cfg = self.config.lock().clone();
        let ex = Excludes::parse(&cfg.excludes);
        // Collapse to the set of folders/files to reconcile.
        let mut paths: Vec<PathBuf> = changed;
        paths.sort();
        paths.dedup();
        let mut d = self.data.write();
        for p in paths {
            self.refresh_path(&mut d, &p, &ex, cfg.hidden);
        }
        self.dirty.store(true, Ordering::Relaxed);
    }

    fn refresh_path(&self, d: &mut IndexData, path: &Path, ex: &Excludes, hidden: bool) {
        match d.find(path) {
            Some(id) => {
                let md = std::fs::symlink_metadata(path).ok();
                let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                match md.and_then(|m| scan::entry_for(path, name, Some(m), hidden)) {
                    None => {
                        if d.nodes[id as usize].parent != NONE {
                            d.remove(id);
                        }
                    }
                    Some(e) => {
                        let was_dir = d.is_dir(id);
                        let is_dir = e.flags & flags::DIR != 0;
                        if was_dir != is_dir {
                            let parent = d.nodes[id as usize].parent;
                            d.remove(id);
                            if let Some(pp) = path.parent() {
                                if parent != NONE {
                                    reconcile(d, parent, pp, ex, hidden);
                                }
                            }
                        } else {
                            d.update(id, e.size, e.mtime, e.flags);
                            if is_dir && e.flags & flags::LINK == 0 {
                                reconcile(d, id, path, ex, hidden);
                            }
                        }
                    }
                }
            }
            None => {
                // New item: reconcile the nearest indexed ancestor.
                let mut cur = path.parent();
                while let Some(a) = cur {
                    if let Some(id) = d.find(a) {
                        if d.is_dir(id) {
                            reconcile(d, id, a, ex, hidden);
                        }
                        return;
                    }
                    cur = a.parent();
                }
            }
        }
    }

    // ---------------------------------------------------------------- search

    pub fn search(&self, query: &str, limit: usize, under: Option<&str>, cancel: &AtomicBool) -> AppResult<IndexResults> {
        if !self.loaded.load(Ordering::Relaxed) {
            return Err(AppError::unsupported(if *self.state.lock() == "off" { "The file index is turned off" } else { "The file index is not ready yet" }));
        }
        let started = Instant::now();
        let q = Query::parse(query)?;
        let words = q.name_words();
        let d = self.data.read();
        let (ids, total) = d.search(&q, &words, under, limit.clamp(1, 20_000), &|| cancel.load(Ordering::Relaxed));
        let hits = ids
            .into_iter()
            .map(|id| {
                let n = d.nodes[id as usize];
                IndexHit { path: d.path(id), name: d.name(id).to_string(), is_dir: d.is_dir(id), size: n.size, mtime: n.mtime }
            })
            .collect();
        Ok(IndexResults { hits, total, elapsed_ms: started.elapsed().as_millis() as u64 })
    }
}

/// Syncs the children of directory `id` with the disk (recursing into new folders).
fn reconcile(d: &mut IndexData, id: u32, path: &Path, ex: &Excludes, hidden: bool) {
    let entries = scan::read_dir(path, hidden);
    let mut seen = HashSet::new();
    let mut new_dirs = Vec::new();
    for e in entries {
        let child_path = path.join(&e.name);
        let is_dir = e.flags & flags::DIR != 0;
        if is_dir && e.flags & flags::LINK == 0 && ex.excluded(&e.name, &child_path) {
            continue;
        }
        match d.child_named(id, &e.name) {
            Some(c) if d.is_dir(c) == is_dir => {
                d.update(c, e.size, e.mtime, e.flags);
                seen.insert(c);
            }
            other => {
                if let Some(c) = other {
                    d.remove(c);
                }
                let c = d.add(id, &e);
                seen.insert(c);
                if is_dir && e.flags & flags::LINK == 0 {
                    new_dirs.push((c, child_path));
                }
            }
        }
    }
    let gone: Vec<u32> = d.children(id).iter().copied().filter(|c| !seen.contains(c)).collect();
    for c in gone {
        d.remove(c);
    }
    for (c, p) in new_dirs {
        reconcile(d, c, &p, ex, hidden);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn live_updates_and_search() {
        let root = std::env::temp_dir().join(format!("dualion-index-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(root.join("docs")).unwrap();
        std::fs::write(root.join("docs/report.pdf"), "x").unwrap();
        let file = root.with_extension("idx");
        let sink: EventSink = Arc::new(|_, _| {});
        let m = IndexManager::new(file.clone(), sink);
        let cfg = IndexConfig { enabled: true, roots: vec![root.to_string_lossy().into_owned()], meta: false, ..Default::default() };
        m.configure(cfg.clone());
        let t = Instant::now();
        while m.status().state != "ready" && t.elapsed() < Duration::from_secs(10) {
            std::thread::sleep(Duration::from_millis(20));
        }
        let never = AtomicBool::new(false);
        assert_eq!(m.search("report", 10, None, &never).unwrap().total, 1);
        // Simulated change notification.
        std::fs::create_dir_all(root.join("new/deep")).unwrap();
        std::fs::write(root.join("new/deep/Report 2.pdf"), "y").unwrap();
        std::fs::remove_file(root.join("docs/report.pdf")).unwrap();
        m.apply_changes(vec![root.join("new/deep/Report 2.pdf"), root.join("docs/report.pdf")]);
        let r = m.search("report ext:pdf", 10, None, &never).unwrap();
        assert_eq!(r.total, 1);
        assert!(r.hits[0].path.ends_with("Report 2.pdf"));
        assert!(file.exists());
        // A second manager loads the saved index.
        let m2 = IndexManager::new(file.clone(), Arc::new(|_, _| {}));
        m2.configure(cfg);
        let t = Instant::now();
        while !m2.status().ready && t.elapsed() < Duration::from_secs(10) {
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(m2.status().ready);
        std::fs::remove_dir_all(&root).unwrap();
        let _ = std::fs::remove_file(&file);
    }
}
