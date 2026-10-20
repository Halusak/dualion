//! In-memory file index: a flat node table with parent links and name
//! arenas, scanned in parallel for searches (Everything style).

use crate::meta::FileMeta;
use crate::query::{fold, Query, Subject};
use std::collections::HashMap;
use std::io::{BufReader, BufWriter, Read, Write};
use std::path::{Path, PathBuf, MAIN_SEPARATOR};

pub const NONE: u32 = u32::MAX;

pub mod flags {
    pub const DIR: u8 = 1;
    pub const HIDDEN: u8 = 2;
    pub const LINK: u8 = 4;
    pub const DEAD: u8 = 128;
}

#[derive(Debug, Clone, Copy)]
pub struct Node {
    pub parent: u32,
    name: u32,
    fold: u32,
    name_len: u16,
    fold_len: u16,
    pub flags: u8,
    pub size: u64,
    pub mtime: i64,
}

#[derive(Default)]
pub struct IndexData {
    pub nodes: Vec<Node>,
    names: String,
    folds: String,
    /// Children of directories (by node id).
    children: HashMap<u32, Vec<u32>>,
    pub roots: Vec<u32>,
    /// Metadata of media / documents, filled in the background.
    pub meta: HashMap<u32, FileMeta>,
    pub files: usize,
    pub dirs: usize,
}

pub struct Entry {
    pub name: String,
    pub flags: u8,
    pub size: u64,
    pub mtime: i64,
}

impl IndexData {
    pub fn name(&self, id: u32) -> &str {
        let n = &self.nodes[id as usize];
        &self.names[n.name as usize..n.name as usize + n.name_len as usize]
    }

    pub fn folded(&self, id: u32) -> &str {
        let n = &self.nodes[id as usize];
        &self.folds[n.fold as usize..n.fold as usize + n.fold_len as usize]
    }

    pub fn is_dir(&self, id: u32) -> bool {
        self.nodes[id as usize].flags & flags::DIR != 0
    }

    pub fn alive(&self, id: u32) -> bool {
        self.nodes[id as usize].flags & flags::DEAD == 0
    }

    pub fn add(&mut self, parent: u32, e: &Entry) -> u32 {
        let id = self.nodes.len() as u32;
        let name = &e.name[..e.name.len().min(u16::MAX as usize)];
        let folded = fold(name);
        let folded = &folded[..floor_char(&folded, u16::MAX as usize)];
        let node = Node {
            parent,
            name: self.names.len() as u32,
            fold: self.folds.len() as u32,
            name_len: name.len() as u16,
            fold_len: folded.len() as u16,
            flags: e.flags,
            size: e.size,
            mtime: e.mtime,
        };
        self.names.push_str(name);
        self.folds.push_str(folded);
        self.nodes.push(node);
        if parent == NONE {
            self.roots.push(id);
        } else {
            self.children.entry(parent).or_default().push(id);
        }
        if e.flags & flags::DIR != 0 {
            self.dirs += 1;
        } else {
            self.files += 1;
        }
        id
    }

    pub fn children(&self, id: u32) -> &[u32] {
        self.children.get(&id).map(|v| v.as_slice()).unwrap_or(&[])
    }

    /// Live children of a directory by lower-case name (for bulk syncs).
    pub fn child_map(&self, parent: u32) -> HashMap<String, u32> {
        self.children(parent).iter().copied().filter(|&c| self.alive(c)).map(|c| (key_of(self.name(c)), c)).collect()
    }

    pub fn child_named(&self, parent: u32, name: &str) -> Option<u32> {
        let list = if parent == NONE { &self.roots[..] } else { self.children(parent) };
        list.iter().copied().find(|&c| self.alive(c) && names_equal(self.name(c), name))
    }

    /// Removes a node and its subtree.
    pub fn remove(&mut self, id: u32) {
        let mut stack = vec![id];
        while let Some(n) = stack.pop() {
            if !self.alive(n) {
                continue;
            }
            if let Some(kids) = self.children.remove(&n) {
                stack.extend(kids);
            }
            let node = &mut self.nodes[n as usize];
            node.flags |= flags::DEAD;
            if node.flags & flags::DIR != 0 {
                self.dirs -= 1;
            } else {
                self.files -= 1;
            }
            self.meta.remove(&n);
        }
        let parent = self.nodes[id as usize].parent;
        if parent != NONE {
            if let Some(v) = self.children.get_mut(&parent) {
                v.retain(|&c| c != id);
            }
        } else {
            self.roots.retain(|&r| r != id);
        }
    }

    pub fn update(&mut self, id: u32, size: u64, mtime: i64, fl: u8) {
        let n = &mut self.nodes[id as usize];
        if n.size != size || n.mtime != mtime {
            self.meta.remove(&id);
        }
        n.size = size;
        n.mtime = mtime;
        n.flags = (n.flags & flags::DEAD) | fl;
    }

    /// Full path of a node.
    pub fn path(&self, id: u32) -> String {
        let mut parts = Vec::new();
        let mut cur = id;
        while cur != NONE {
            parts.push(self.name(cur));
            cur = self.nodes[cur as usize].parent;
        }
        let mut out = String::new();
        for (i, p) in parts.iter().rev().enumerate() {
            if i > 0 && !out.ends_with(MAIN_SEPARATOR) && !out.ends_with('/') {
                out.push(MAIN_SEPARATOR);
            }
            out.push_str(p);
        }
        out
    }

    /// Node of an absolute path, if indexed.
    pub fn find(&self, path: &Path) -> Option<u32> {
        let p = path.to_string_lossy();
        let root = self.roots.iter().copied().filter(|&r| self.alive(r)).find(|&r| {
            let rp = self.name(r);
            p == rp || p.strip_prefix(rp).is_some_and(|rest| rest.starts_with(['/', '\\']) || rp.ends_with(['/', '\\']))
        })?;
        let rest = &p[self.name(root).len()..];
        let mut cur = root;
        for comp in rest.split(['/', '\\']).filter(|c| !c.is_empty()) {
            cur = self.child_named(cur, comp)?;
        }
        Some(cur)
    }

    // ---------------------------------------------------------------- search

    pub fn search(&self, q: &Query, words: &[String], under: Option<&str>, limit: usize, cancel: &(dyn Fn() -> bool + Sync)) -> (Vec<u32>, usize) {
        let under_node = under.and_then(|u| self.find(Path::new(u)));
        if under.is_some() && under_node.is_none() {
            return (Vec::new(), 0);
        }
        let n = self.nodes.len();
        let threads = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4).clamp(1, 16);
        let chunk = n.div_ceil(threads).max(4096);
        let cap = (limit * 20).max(20_000);
        let results: Vec<(Vec<u32>, usize)> = std::thread::scope(|s| {
            let handles: Vec<_> = (0..n)
                .step_by(chunk)
                .map(|start| {
                    let end = (start + chunk).min(n);
                    s.spawn(move || {
                        let mut out = Vec::new();
                        let mut count = 0usize;
                        for id in start as u32..end as u32 {
                            if id % 65_536 == 0 && cancel() {
                                break;
                            }
                            let node = &self.nodes[id as usize];
                            if node.flags & flags::DEAD != 0 || node.parent == NONE {
                                continue;
                            }
                            let f = self.folded(id);
                            if !words.iter().all(|w| f.contains(w.as_str())) {
                                continue;
                            }
                            if let Some(u) = under_node {
                                if !self.is_under(id, u) {
                                    continue;
                                }
                            }
                            if !q.matches(&IndexSubject::new(self, id)) {
                                continue;
                            }
                            count += 1;
                            if out.len() < cap {
                                out.push(id);
                            }
                        }
                        (out, count)
                    })
                })
                .collect();
            handles.into_iter().map(|h| h.join().unwrap_or_default()).collect()
        });
        let total = results.iter().map(|r| r.1).sum();
        let mut ids: Vec<u32> = results.into_iter().flat_map(|r| r.0).collect();
        // Rank: exact name, prefix, word start, substring; then shallower and shorter.
        let first = words.first().map(|s| s.as_str()).unwrap_or("");
        let score = |id: u32| -> (u8, u32, usize) {
            let f = self.folded(id);
            let stem = f.rsplit_once('.').map(|(s, _)| s).unwrap_or(f);
            let rank = if first.is_empty() {
                2
            } else if f == first || stem == first {
                0
            } else if f.starts_with(first) {
                1
            } else if f.contains(&format!(" {first}")) || f.contains(&format!("_{first}")) || f.contains(&format!("-{first}")) {
                2
            } else {
                3
            };
            (rank, self.depth(id), f.len())
        };
        if ids.len() > limit {
            ids.select_nth_unstable_by_key(limit, |&id| score(id));
            ids.truncate(limit);
        }
        ids.sort_by_cached_key(|&id| score(id));
        (ids, total)
    }

    fn depth(&self, mut id: u32) -> u32 {
        let mut d = 0;
        while id != NONE {
            id = self.nodes[id as usize].parent;
            d += 1;
        }
        d
    }

    pub fn is_under(&self, mut id: u32, ancestor: u32) -> bool {
        while id != NONE {
            if id == ancestor {
                return true;
            }
            id = self.nodes[id as usize].parent;
        }
        false
    }

    // ---------------------------------------------------------------- persistence

    const MAGIC: &'static [u8; 6] = b"DIDX02";

    pub fn save(&self, path: &Path, tag: u64) -> std::io::Result<()> {
        let tmp = path.with_extension("tmp");
        {
            let mut w = BufWriter::with_capacity(1 << 20, std::fs::File::create(&tmp)?);
            w.write_all(Self::MAGIC)?;
            w.write_all(&tag.to_le_bytes())?;
            // Renumber live nodes so the file has no holes; parents precede children.
            let mut map = vec![NONE; self.nodes.len()];
            let mut order = Vec::with_capacity(self.nodes.len());
            let mut stack: Vec<u32> = self.roots.iter().rev().copied().collect();
            while let Some(id) = stack.pop() {
                if !self.alive(id) {
                    continue;
                }
                map[id as usize] = order.len() as u32;
                order.push(id);
                stack.extend(self.children(id).iter().rev().copied());
            }
            w.write_all(&(order.len() as u32).to_le_bytes())?;
            for &id in &order {
                let n = &self.nodes[id as usize];
                let parent = if n.parent == NONE { NONE } else { map[n.parent as usize] };
                let name = self.name(id).as_bytes();
                w.write_all(&parent.to_le_bytes())?;
                w.write_all(&[n.flags])?;
                w.write_all(&n.size.to_le_bytes())?;
                w.write_all(&n.mtime.to_le_bytes())?;
                w.write_all(&(name.len() as u16).to_le_bytes())?;
                w.write_all(name)?;
            }
            // Metadata as JSON lines: new id, then the record.
            let metas: Vec<(u32, &FileMeta)> = self.meta.iter().filter(|(id, _)| map[**id as usize] != NONE).map(|(id, m)| (map[*id as usize], m)).collect();
            w.write_all(&(metas.len() as u32).to_le_bytes())?;
            for (id, m) in metas {
                let j = serde_json::to_vec(m).unwrap_or_default();
                w.write_all(&id.to_le_bytes())?;
                w.write_all(&(j.len() as u32).to_le_bytes())?;
                w.write_all(&j)?;
            }
            w.flush()?;
        }
        std::fs::rename(&tmp, path)
    }

    pub fn load(path: &Path, tag: u64) -> std::io::Result<IndexData> {
        let mut r = BufReader::with_capacity(1 << 20, std::fs::File::open(path)?);
        let bad = || std::io::Error::new(std::io::ErrorKind::InvalidData, "index file");
        let mut magic = [0u8; 6];
        r.read_exact(&mut magic)?;
        let mut b8 = [0u8; 8];
        let mut b4 = [0u8; 4];
        let mut b2 = [0u8; 2];
        let mut b1 = [0u8; 1];
        r.read_exact(&mut b8)?;
        if &magic != Self::MAGIC || u64::from_le_bytes(b8) != tag {
            return Err(bad());
        }
        r.read_exact(&mut b4)?;
        let count = u32::from_le_bytes(b4) as usize;
        let mut d = IndexData { nodes: Vec::with_capacity(count), ..Default::default() };
        let mut name = Vec::new();
        for _ in 0..count {
            r.read_exact(&mut b4)?;
            let parent = u32::from_le_bytes(b4);
            r.read_exact(&mut b1)?;
            r.read_exact(&mut b8)?;
            let size = u64::from_le_bytes(b8);
            r.read_exact(&mut b8)?;
            let mtime = i64::from_le_bytes(b8);
            r.read_exact(&mut b2)?;
            name.resize(u16::from_le_bytes(b2) as usize, 0);
            r.read_exact(&mut name)?;
            if parent != NONE && parent as usize >= d.nodes.len() {
                return Err(bad());
            }
            let name = String::from_utf8(std::mem::take(&mut name)).map_err(|_| bad())?;
            d.add(parent, &Entry { name, flags: b1[0], size, mtime });
        }
        if r.read_exact(&mut b4).is_ok() {
            let n = u32::from_le_bytes(b4);
            let mut buf = Vec::new();
            for _ in 0..n {
                r.read_exact(&mut b4)?;
                let id = u32::from_le_bytes(b4);
                r.read_exact(&mut b4)?;
                buf.resize(u32::from_le_bytes(b4) as usize, 0);
                r.read_exact(&mut buf)?;
                if let Ok(m) = serde_json::from_slice::<FileMeta>(&buf) {
                    if (id as usize) < d.nodes.len() {
                        d.meta.insert(id, m);
                    }
                }
            }
        }
        Ok(d)
    }
}

fn floor_char(s: &str, max: usize) -> usize {
    if s.len() <= max {
        return s.len();
    }
    let mut i = max;
    while !s.is_char_boundary(i) {
        i -= 1;
    }
    i
}

/// Name as compared by the file system.
#[cfg(any(target_os = "macos", windows))]
pub fn key_of(name: &str) -> String {
    name.to_lowercase()
}
#[cfg(not(any(target_os = "macos", windows)))]
pub fn key_of(name: &str) -> String {
    name.to_string()
}

#[cfg(any(target_os = "macos", windows))]
fn names_equal(a: &str, b: &str) -> bool {
    a.eq_ignore_ascii_case(b) || (a.len() == b.len() && a.to_lowercase() == b.to_lowercase())
}
#[cfg(not(any(target_os = "macos", windows)))]
fn names_equal(a: &str, b: &str) -> bool {
    a == b
}

/// A node as a query subject.
pub struct IndexSubject<'a> {
    d: &'a IndexData,
    id: u32,
    path: std::cell::OnceCell<String>,
    lazy_meta: std::cell::OnceCell<Option<FileMeta>>,
}

impl<'a> IndexSubject<'a> {
    pub fn new(d: &'a IndexData, id: u32) -> Self {
        IndexSubject { d, id, path: Default::default(), lazy_meta: Default::default() }
    }
}

impl Subject for IndexSubject<'_> {
    fn name(&self) -> &str {
        self.d.name(self.id)
    }
    fn path(&self) -> &str {
        self.path.get_or_init(|| self.d.path(self.id))
    }
    fn is_dir(&self) -> bool {
        self.d.is_dir(self.id)
    }
    fn size(&self) -> u64 {
        self.d.nodes[self.id as usize].size
    }
    fn mtime(&self) -> i64 {
        self.d.nodes[self.id as usize].mtime
    }
    fn meta(&self) -> Option<&FileMeta> {
        if let Some(m) = self.d.meta.get(&self.id) {
            return Some(m);
        }
        // Not indexed yet: read it now (only media / documents get here).
        self.lazy_meta
            .get_or_init(|| {
                let name = self.name();
                let ext = name.rsplit_once('.').map(|(_, e)| e.to_lowercase()).unwrap_or_default();
                if self.is_dir() || !crate::meta::has_meta(&ext) {
                    return None;
                }
                Some(crate::meta::get_known(&PathBuf::from(self.path()), self.size(), self.mtime()))
            })
            .as_ref()
    }
    fn tags(&self) -> Vec<String> {
        crate::tags::tags_of(self.path())
    }
    fn contains_text(&self, folded: &str) -> bool {
        crate::meta::text_content(Path::new(self.path())).is_some_and(|t| fold(&t).contains(folded))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn e(name: &str, dir: bool, size: u64) -> Entry {
        Entry { name: name.into(), flags: if dir { flags::DIR } else { 0 }, size, mtime: 0 }
    }

    fn sample() -> IndexData {
        let mut d = IndexData::default();
        let root = d.add(NONE, &e("/data", true, 0));
        let photos = d.add(root, &e("Fotky", true, 0));
        d.add(photos, &e("Dovolená.jpg", false, 10));
        d.add(photos, &e("dovolena-2.jpg", false, 20));
        let docs = d.add(root, &e("Dokumenty", true, 0));
        d.add(docs, &e("Faktura dovolená.pdf", false, 5));
        d
    }

    #[test]
    fn search_rank_and_paths() {
        let d = sample();
        let q = Query::parse("dovolena").unwrap();
        let (ids, total) = d.search(&q, &["dovolena".into()], None, 10, &|| false);
        assert_eq!(total, 3);
        assert_eq!(d.name(ids[0]), "Dovolená.jpg");
        assert_eq!(d.path(ids[0]), format!("/data{MAIN_SEPARATOR}Fotky{MAIN_SEPARATOR}Dovolená.jpg"));
        let q = Query::parse("dovolena ext:pdf").unwrap();
        assert_eq!(d.search(&q, &["dovolena".into()], None, 10, &|| false).1, 1);
        let q = Query::parse("type:folder").unwrap();
        assert_eq!(d.search(&q, &[], None, 10, &|| false).1, 2);
    }

    #[test]
    fn find_remove_and_persist() {
        let mut d = sample();
        let id = d.find(Path::new(&format!("/data{MAIN_SEPARATOR}Fotky"))).unwrap();
        assert_eq!(d.name(id), "Fotky");
        assert!(d.find(Path::new("/data/fotky/dovolená.JPG")).is_some() || cfg!(not(any(target_os = "macos", windows))));
        d.remove(id);
        assert_eq!((d.files, d.dirs), (1, 2));
        let p = std::env::temp_dir().join(format!("dualion-idx-{}", uuid::Uuid::new_v4()));
        d.meta.insert(5, FileMeta { pages: Some(3), ..Default::default() });
        d.save(&p, 7).unwrap();
        let l = IndexData::load(&p, 7).unwrap();
        assert!(IndexData::load(&p, 8).is_err());
        std::fs::remove_file(&p).unwrap();
        assert_eq!((l.files, l.dirs), (1, 2));
        let pdf = l.find(Path::new(&format!("/data{MAIN_SEPARATOR}Dokumenty{MAIN_SEPARATOR}Faktura dovolená.pdf"))).unwrap();
        assert_eq!(l.meta.get(&pdf).and_then(|m| m.pages), Some(3));
    }
}
