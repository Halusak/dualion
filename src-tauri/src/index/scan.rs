//! Parallel directory scanner that builds an [`IndexData`].
//! Worker threads read directories; a single assembler thread assigns node
//! ids and hands sub-directories back to the workers.

use super::data::{flags, Entry, IndexData, NONE};
use crate::util::system_time_ms;
use parking_lot::{Condvar, Mutex};
use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc;
use std::sync::Arc;

#[derive(Clone)]
pub struct Excludes {
    names: Vec<String>,
    paths: Vec<PathBuf>,
}

impl Excludes {
    /// `;`-separated list: plain names ("node_modules") match anywhere,
    /// entries with a path separator or `~` are absolute folders.
    pub fn parse(spec: &str) -> Excludes {
        let mut names = Vec::new();
        let mut paths = Vec::new();
        for item in spec.split([';', '\n']).map(str::trim).filter(|s| !s.is_empty()) {
            if item.contains('/') || item.contains('\\') || item.starts_with('~') {
                paths.push(crate::util::expand_user_path(item));
            } else {
                names.push(item.to_lowercase());
            }
        }
        Excludes { names, paths }
    }

    pub fn excluded(&self, name: &str, path: &Path) -> bool {
        (!self.names.is_empty() && self.names.iter().any(|n| n.eq_ignore_ascii_case(name))) || self.paths.iter().any(|p| p == path)
    }
}

/// Reads one directory into index entries.
pub fn read_dir(path: &Path, hidden: bool) -> Vec<Entry> {
    let Ok(rd) = std::fs::read_dir(path) else { return Vec::new() };
    let mut out = Vec::new();
    for de in rd.flatten() {
        let name = de.file_name().to_string_lossy().into_owned();
        if let Some(e) = entry_for(&de.path(), name, de.metadata().ok(), hidden) {
            out.push(e);
        }
    }
    out
}

pub fn entry_for(path: &Path, name: String, md: Option<std::fs::Metadata>, hidden: bool) -> Option<Entry> {
    use crate::location::flags as lf;
    let le = crate::local::entry_from_metadata(name, path, &md?);
    let mut fl = 0u8;
    if le.flags & lf::HIDDEN != 0 {
        if !hidden {
            return None;
        }
        fl |= flags::HIDDEN;
    }
    if le.flags & lf::SYMLINK != 0 {
        fl |= flags::LINK;
    }
    if le.flags & lf::CLOUD != 0 {
        fl |= flags::CLOUD;
    }
    // Bundles (.app …) are indexed as files; their insides are noise.
    if le.flags & lf::DIR != 0 && le.flags & lf::PACKAGE == 0 {
        fl |= flags::DIR;
    }
    Some(Entry { name: le.name, flags: fl, size: le.size, mtime: le.mtime })
}

/// Directories waiting to be read: (node id, path), with a wake-up signal.
type WorkQueue = (Mutex<VecDeque<(u32, PathBuf)>>, Condvar);

pub struct Progress {
    pub dirs: AtomicUsize,
    pub cancel: AtomicBool,
}

/// Scans `roots` into a fresh index.
pub fn scan(roots: &[PathBuf], excludes: &Excludes, hidden: bool, progress: &Progress) -> IndexData {
    let mut d = IndexData::default();
    let queue: Arc<WorkQueue> = Arc::new((Mutex::new(VecDeque::new()), Condvar::new()));
    let done = Arc::new(AtomicBool::new(false));
    let (tx, rx) = mpsc::channel::<(u32, PathBuf, Vec<Entry>)>();
    let mut outstanding = 0usize;
    for r in roots {
        let Ok(md) = std::fs::metadata(r) else { continue };
        if !md.is_dir() {
            continue;
        }
        let id = d.add(NONE, &Entry { name: r.to_string_lossy().into_owned(), flags: flags::DIR, size: 0, mtime: md.modified().map(system_time_ms).unwrap_or(0) });
        queue.0.lock().push_back((id, r.clone()));
        outstanding += 1;
    }
    let threads = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4).clamp(2, 12);
    std::thread::scope(|s| {
        for _ in 0..threads {
            let (queue, done, tx) = (queue.clone(), done.clone(), tx.clone());
            s.spawn(move || loop {
                let job = {
                    let mut q = queue.0.lock();
                    loop {
                        if let Some(j) = q.pop_front() {
                            break Some(j);
                        }
                        if done.load(Ordering::Relaxed) {
                            break None;
                        }
                        queue.1.wait(&mut q);
                    }
                };
                let Some((id, path)) = job else { break };
                let entries = if progress.cancel.load(Ordering::Relaxed) { Vec::new() } else { read_dir(&path, hidden) };
                if tx.send((id, path, entries)).is_err() {
                    break;
                }
            });
        }
        drop(tx);
        while outstanding > 0 {
            let Ok((parent, path, entries)) = rx.recv() else { break };
            outstanding -= 1;
            progress.dirs.fetch_add(1, Ordering::Relaxed);
            let mut q = Vec::new();
            for e in entries {
                let is_dir = e.flags & flags::DIR != 0 && e.flags & flags::LINK == 0;
                let child_path = path.join(&e.name);
                if is_dir && excludes.excluded(&e.name, &child_path) {
                    continue;
                }
                let id = d.add(parent, &e);
                if is_dir {
                    q.push((id, child_path));
                }
            }
            if !q.is_empty() {
                outstanding += q.len();
                queue.0.lock().extend(q);
                queue.1.notify_all();
            }
        }
        done.store(true, Ordering::Relaxed);
        queue.1.notify_all();
    });
    d
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::query::Query;

    #[test]
    fn scans_tree_with_excludes() {
        let root = std::env::temp_dir().join(format!("dualion-scan-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(root.join("a/b")).unwrap();
        std::fs::create_dir_all(root.join("node_modules/x")).unwrap();
        std::fs::write(root.join("a/b/hello.txt"), "hi").unwrap();
        std::fs::write(root.join("a/top.md"), "x").unwrap();
        std::fs::write(root.join("node_modules/x/skip.js"), "x").unwrap();
        let p = Progress { dirs: AtomicUsize::new(0), cancel: AtomicBool::new(false) };
        let d = scan(std::slice::from_ref(&root), &Excludes::parse("node_modules"), false, &p);
        assert_eq!((d.files, d.dirs), (2, 3));
        let q = Query::parse("hello").unwrap();
        let (ids, _) = d.search(&q, &q.name_words(), None, 10, &|| false);
        assert_eq!(d.path(ids[0]), root.join("a/b/hello.txt").to_string_lossy());
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    #[ignore]
    fn bench_home() {
        let home = dirs::home_dir().unwrap();
        let p = Progress { dirs: AtomicUsize::new(0), cancel: AtomicBool::new(false) };
        let t = std::time::Instant::now();
        let d = scan(&[home], &Excludes::parse(super::super::DEFAULT_EXCLUDES), true, &p);
        println!("scan: {:?} files={} dirs={}", t.elapsed(), d.files, d.dirs);
        for q in ["readme", "dovolena", "type:video", "ext:pdf size>1mb", "*.rs"] {
            let qq = Query::parse(q).unwrap();
            let t = std::time::Instant::now();
            let (ids, total) = d.search(&qq, &qq.name_words(), None, 200, &|| false);
            println!("{q:24} {:>8?} total={total} first={:?}", t.elapsed(), ids.first().map(|&i| d.path(i)));
        }
        let f = std::env::temp_dir().join("dualion-bench.idx");
        let t = std::time::Instant::now();
        d.save(&f, 1).unwrap();
        println!("save {:?} {} MB", t.elapsed(), std::fs::metadata(&f).unwrap().len() >> 20);
        let t = std::time::Instant::now();
        let l = IndexData::load(&f, 1).unwrap();
        println!("load {:?} files={}", t.elapsed(), l.files);
        std::fs::remove_file(&f).unwrap();
    }
}
