//! Parallel directory scanner that builds an [`IndexData`].
//! Worker threads read directories; a single assembler thread assigns node
//! ids and hands sub-directories back to the workers.

use super::data::{flags, Entry, IndexData, NONE};
use crate::util::system_time_ms;
use parking_lot::{Condvar, Mutex};
use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc;
use std::sync::Arc;
use std::time::{Duration, Instant};

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

/// Reads one directory into index entries. Mount points of network and
/// virtual file systems are left out: reading them can hang (a dead share)
/// or never end (/dev, /proc).
pub fn read_dir(path: &Path, hidden: bool) -> Vec<Entry> {
    let Ok(rd) = std::fs::read_dir(path) else { return Vec::new() };
    #[cfg(unix)]
    let dev = {
        use std::os::unix::fs::MetadataExt;
        std::fs::metadata(path).map(|m| m.dev()).ok()
    };
    let mut out = Vec::new();
    for de in rd.flatten() {
        let name = de.file_name().to_string_lossy().into_owned();
        let child = de.path();
        let md = de.metadata().ok();
        #[cfg(unix)]
        if let (Some(m), Some(dev)) = (&md, dev) {
            use std::os::unix::fs::MetadataExt;
            if m.is_dir() && (m.dev() != dev && foreign_mount(&child) || skipped_system_dir(&child)) {
                continue;
            }
        }
        if let Some(e) = entry_for(&child, name, md, hidden) {
            out.push(e);
        }
    }
    out
}

/// macOS: /System/Volumes holds the same data again (firmlinks), /Volumes
/// other disks; neither belongs to a scan of the startup disk. Linux: /dev
/// (devtmpfs shares its magic with tmpfs, so it is told by path).
#[cfg(unix)]
fn skipped_system_dir(p: &Path) -> bool {
    if cfg!(target_os = "macos") {
        p == Path::new("/System/Volumes") || p == Path::new("/Volumes")
    } else {
        p == Path::new("/dev")
    }
}

/// A mount point of a network / virtual file system.
#[cfg(target_os = "macos")]
fn foreign_mount(p: &Path) -> bool {
    use std::os::unix::ffi::OsStrExt;
    let Ok(c) = std::ffi::CString::new(p.as_os_str().as_bytes()) else { return false };
    let mut st: libc::statfs = unsafe { std::mem::zeroed() };
    if unsafe { libc::statfs(c.as_ptr(), &mut st) } != 0 {
        return true;
    }
    let ty = unsafe { std::ffi::CStr::from_ptr(st.f_fstypename.as_ptr()) }.to_string_lossy().to_lowercase();
    st.f_flags & libc::MNT_LOCAL as u32 == 0 || ["autofs", "devfs", "nfs", "smbfs", "afpfs", "webdav", "ftp", "cifs"].contains(&ty.as_str())
}

#[cfg(all(unix, not(target_os = "macos")))]
fn foreign_mount(p: &Path) -> bool {
    use std::os::unix::ffi::OsStrExt;
    let Ok(c) = std::ffi::CString::new(p.as_os_str().as_bytes()) else { return false };
    let mut st: libc::statfs = unsafe { std::mem::zeroed() };
    if unsafe { libc::statfs(c.as_ptr(), &mut st) } != 0 {
        return true;
    }
    const SKIP: [i64; 11] = [
        0x6969,     // nfs
        0x517B,     // smb
        0xFF534D42, // cifs
        0xFE534D42, // smb2
        0x0187,     // autofs
        0x9fa0,     // proc
        0x62656572, // sysfs
        0x1cd1,     // devpts
        0x27e0eb,   // cgroup
        0x63677270, // cgroup2
        0x64626720, // debugfs
    ];
    SKIP.contains(&(st.f_type as i64))
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

/// A directory read taking longer than this is given up on (a hung mount, an
/// iCloud folder waiting for the network …) so the scan can finish.
const STALL: Duration = Duration::from_secs(20);

struct Shared {
    queue: WorkQueue,
    /// Set when the scan is over: idle workers exit.
    done: AtomicBool,
    /// Directories being read, with the time the read began.
    inflight: Mutex<HashMap<u32, Instant>>,
}

/// Workers are detached: one stuck in a system call must not hold up the
/// scan, which abandons its directory and starts a replacement.
fn spawn_worker(shared: Arc<Shared>, tx: mpsc::Sender<(u32, PathBuf, Vec<Entry>)>, hidden: bool) {
    std::thread::spawn(move || loop {
        let job = {
            let mut q = shared.queue.0.lock();
            loop {
                if shared.done.load(Ordering::Relaxed) {
                    break None;
                }
                if let Some(j) = q.pop_front() {
                    break Some(j);
                }
                shared.queue.1.wait(&mut q);
            }
        };
        let Some((id, path)) = job else { break };
        shared.inflight.lock().insert(id, Instant::now());
        let entries = read_dir(&path, hidden);
        if shared.inflight.lock().remove(&id).is_none() {
            // Abandoned as stalled; a replacement worker took this one's place.
            break;
        }
        if tx.send((id, path, entries)).is_err() {
            break;
        }
    });
}

/// Scans `roots` into a fresh index.
pub fn scan(roots: &[PathBuf], excludes: &Excludes, hidden: bool, progress: &Progress) -> IndexData {
    let mut d = IndexData::default();
    let shared = Arc::new(Shared { queue: (Mutex::new(VecDeque::new()), Condvar::new()), done: AtomicBool::new(false), inflight: Mutex::new(HashMap::new()) });
    let (tx, rx) = mpsc::channel::<(u32, PathBuf, Vec<Entry>)>();
    let mut outstanding = 0usize;
    for r in roots {
        let Ok(md) = std::fs::metadata(r) else { continue };
        if !md.is_dir() {
            continue;
        }
        let id = d.add(NONE, &Entry { name: r.to_string_lossy().into_owned(), flags: flags::DIR, size: 0, mtime: md.modified().map(system_time_ms).unwrap_or(0) });
        shared.queue.0.lock().push_back((id, r.clone()));
        outstanding += 1;
    }
    let threads = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4).clamp(2, 12);
    for _ in 0..threads {
        spawn_worker(shared.clone(), tx.clone(), hidden);
    }
    while outstanding > 0 && !progress.cancel.load(Ordering::Relaxed) {
        let (parent, path, entries) = match rx.recv_timeout(Duration::from_millis(250)) {
            Ok(m) => m,
            Err(mpsc::RecvTimeoutError::Timeout) => {
                let stalled: Vec<u32> = {
                    let mut inflight = shared.inflight.lock();
                    let ids: Vec<u32> = inflight.iter().filter(|(_, t)| t.elapsed() > STALL).map(|(&id, _)| id).collect();
                    for id in &ids {
                        inflight.remove(id);
                    }
                    ids
                };
                for _ in &stalled {
                    outstanding -= 1;
                    spawn_worker(shared.clone(), tx.clone(), hidden);
                }
                continue;
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        };
        outstanding -= 1;
        progress.dirs.fetch_add(1, Ordering::Relaxed);
        let mut q = Vec::new();
        for e in entries {
            // Online-only cloud folders: listing one makes the sync client
            // fetch it from the network, and nothing in it takes up space here.
            let is_dir = e.flags & flags::DIR != 0 && e.flags & (flags::LINK | flags::CLOUD) == 0;
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
            shared.queue.0.lock().extend(q);
            shared.queue.1.notify_all();
        }
    }
    {
        let _q = shared.queue.0.lock();
        shared.done.store(true, Ordering::Relaxed);
    }
    shared.queue.1.notify_all();
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
        assert_eq!(d.path(ids[0]), root.join("a").join("b").join("hello.txt").to_string_lossy());
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
