//! Watches the folders visible in the panels and tells the UI to refresh
//! them (debounced), so external changes show up immediately.

use notify::{RecommendedWatcher, RecursiveMode, Watcher};
use parking_lot::Mutex;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;
use tauri::{AppHandle, Emitter};

pub struct DirWatcher {
    watcher: Mutex<Option<RecommendedWatcher>>,
    watched: Mutex<HashSet<PathBuf>>,
}

impl DirWatcher {
    pub fn new(app: AppHandle) -> Arc<Self> {
        let pending: Arc<Mutex<HashSet<PathBuf>>> = Arc::new(Mutex::new(HashSet::new()));
        let p2 = pending.clone();
        let watcher = notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
            if let Ok(ev) = res {
                if matches!(ev.kind, notify::EventKind::Access(_)) {
                    return;
                }
                let mut p = p2.lock();
                for path in ev.paths {
                    if let Some(parent) = path.parent() {
                        p.insert(parent.to_path_buf());
                    }
                    p.insert(path);
                }
            }
        })
        .ok();
        let me = Arc::new(Self { watcher: Mutex::new(watcher), watched: Mutex::new(HashSet::new()) });
        let weak = Arc::downgrade(&me);
        std::thread::Builder::new()
            .name("fs-watch".into())
            .spawn(move || loop {
                std::thread::sleep(Duration::from_millis(300));
                let Some(me) = weak.upgrade() else { break };
                let changed: Vec<PathBuf> = std::mem::take(&mut *pending.lock()).into_iter().collect();
                if changed.is_empty() {
                    continue;
                }
                let watched = me.watched.lock().clone();
                let dirs: Vec<String> = watched
                    .iter()
                    .filter(|w| changed.iter().any(|c| same(c, w)))
                    .map(|w| w.to_string_lossy().into_owned())
                    .collect();
                if !dirs.is_empty() {
                    let _ = app.emit("fs-changed", dirs);
                }
            })
            .expect("watch thread");
        me
    }

    /// Replaces the set of watched folders.
    pub fn set(&self, dirs: Vec<String>) {
        let want: HashSet<PathBuf> = dirs.into_iter().map(PathBuf::from).filter(|p| p.is_dir()).collect();
        let mut watched = self.watched.lock();
        let mut guard = self.watcher.lock();
        let Some(w) = guard.as_mut() else { return };
        for old in watched.difference(&want) {
            let _ = w.unwatch(old);
        }
        for new in want.difference(&watched) {
            let _ = w.watch(new, RecursiveMode::NonRecursive);
        }
        *watched = want;
    }
}

fn same(a: &Path, b: &Path) -> bool {
    if a == b {
        return true;
    }
    // FSEvents reports /private/var/... for /var/..., compare canonical forms too.
    match (a.canonicalize(), b.canonicalize()) {
        (Ok(x), Ok(y)) => x == y,
        _ => false,
    }
}
