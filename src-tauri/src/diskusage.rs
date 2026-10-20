//! Disk usage analysis for the treemap: a parallel scan of a folder (reusing
//! the index scanner), sizes summed bottom-up, cached for drilling down.

use crate::error::{AppError, AppResult};
use crate::index::data::{flags, IndexData, NONE};
use crate::index::scan::{self, Excludes, Progress};
use parking_lot::Mutex;
use serde::Serialize;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::LazyLock;
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DuNode {
    pub name: String,
    pub path: String,
    pub size: u64,
    pub files: u64,
    pub is_dir: bool,
    /// Several small items merged into one tile.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub other: bool,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub children: Vec<DuNode>,
    pub mtime: i64,
}

struct Scanned {
    root: PathBuf,
    data: IndexData,
    sizes: Vec<u64>,
    files: Vec<u64>,
    at: Instant,
}

static CACHE: LazyLock<Mutex<Option<Scanned>>> = LazyLock::new(|| Mutex::new(None));

pub fn scan(root: &Path, progress: &dyn Fn(usize), cancel: &AtomicBool) -> AppResult<()> {
    let p = Progress { dirs: AtomicUsize::new(0), cancel: AtomicBool::new(false) };
    let data = std::thread::scope(|s| {
        let h = s.spawn(|| scan::scan(&[root.to_path_buf()], &Excludes::parse(""), true, &p));
        while !h.is_finished() {
            std::thread::sleep(Duration::from_millis(250));
            if cancel.load(Ordering::Relaxed) {
                p.cancel.store(true, Ordering::Relaxed);
            }
            progress(p.dirs.load(Ordering::Relaxed));
        }
        h.join().unwrap_or_default()
    });
    if cancel.load(Ordering::Relaxed) {
        return Err(AppError::cancelled());
    }
    // Parents are always added before their children: sum in reverse order.
    let n = data.nodes.len();
    let mut sizes = vec![0u64; n];
    let mut files = vec![0u64; n];
    for id in (0..n).rev() {
        let node = &data.nodes[id];
        if node.flags & flags::DIR == 0 {
            sizes[id] += node.size;
            files[id] += 1;
        }
        if node.parent != NONE {
            sizes[node.parent as usize] += sizes[id];
            files[node.parent as usize] += files[id];
        }
    }
    *CACHE.lock() = Some(Scanned { root: root.to_path_buf(), data, sizes, files, at: Instant::now() });
    Ok(())
}

/// Whether `path` can be served from the last scan.
pub fn cached(path: &Path) -> bool {
    CACHE.lock().as_ref().is_some_and(|s| path.starts_with(&s.root) && s.at.elapsed() < Duration::from_secs(600) && s.data.find(path).is_some())
}

/// Tree below `path` from the cached scan, limited for drawing.
pub fn tree(path: &Path, depth: u32) -> AppResult<DuNode> {
    let guard = CACHE.lock();
    let s = guard.as_ref().ok_or_else(|| AppError::invalid("Not scanned"))?;
    let id = s.data.find(path).ok_or_else(|| AppError::invalid("Not scanned"))?;
    let mut budget = 4000usize;
    Ok(build(s, id, depth, &mut budget))
}

fn build(s: &Scanned, id: u32, depth: u32, budget: &mut usize) -> DuNode {
    let d = &s.data;
    let node = &d.nodes[id as usize];
    let is_dir = node.flags & flags::DIR != 0;
    let mut out = DuNode {
        name: d.name(id).to_string(),
        path: d.path(id),
        size: s.sizes[id as usize],
        files: s.files[id as usize],
        is_dir,
        other: false,
        children: Vec::new(),
        mtime: node.mtime,
    };
    if !is_dir || depth == 0 || *budget == 0 {
        return out;
    }
    let mut kids: Vec<u32> = d.children(id).iter().copied().filter(|&c| d.alive(c)).collect();
    kids.sort_by_key(|&c| std::cmp::Reverse(s.sizes[c as usize]));
    let total = out.size.max(1);
    let mut rest_size = 0u64;
    let mut rest_files = 0u64;
    let mut rest_n = 0usize;
    for (i, c) in kids.into_iter().enumerate() {
        let sz = s.sizes[c as usize];
        // Tiles under ~0.3 % of the parent are too small to see.
        if i < 80 && *budget > 0 && (sz * 1000 / total >= 3 || i < 6) && sz > 0 {
            *budget -= 1;
            out.children.push(build(s, c, depth - 1, budget));
        } else {
            rest_size += sz;
            rest_files += s.files[c as usize];
            rest_n += 1;
        }
    }
    if rest_size > 0 {
        out.children.push(DuNode {
            name: format!("{rest_n}"),
            path: String::new(),
            size: rest_size,
            files: rest_files,
            is_dir: false,
            other: true,
            children: Vec::new(),
            mtime: 0,
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sums_sizes() {
        let root = std::env::temp_dir().join(format!("dualion-du-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(root.join("a/b")).unwrap();
        std::fs::write(root.join("a/b/x.bin"), vec![0u8; 3000]).unwrap();
        std::fs::write(root.join("a/y.bin"), vec![0u8; 1000]).unwrap();
        std::fs::write(root.join("z.bin"), vec![0u8; 500]).unwrap();
        scan(&root, &|_| {}, &AtomicBool::new(false)).unwrap();
        let t = tree(&root, 3).unwrap();
        assert_eq!((t.size, t.files), (4500, 3));
        assert_eq!(t.children[0].name, "a");
        assert_eq!(t.children[0].size, 4000);
        assert!(cached(&root.join("a")));
        let sub = tree(&root.join("a"), 2).unwrap();
        assert_eq!(sub.children[0].name, "b");
        std::fs::remove_dir_all(&root).unwrap();
    }
}
