//! Duplicate finder: same size -> same head/tail hash -> same full hash.
//! Optionally groups visually similar images by a perceptual hash (dHash).

use crate::error::{AppError, AppResult};
use crate::index::data::{flags, NONE};
use crate::index::scan::{self, Excludes, Progress};
use crate::index::IndexManager;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::time::Instant;
use tauri::ipc::Channel;

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct DupeParams {
    pub roots: Vec<String>,
    pub min_size: u64,
    pub hidden: bool,
    /// Also group visually similar images.
    pub similar: bool,
    /// Max. differing bits of the 64-bit image hash (0–16).
    pub threshold: u32,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DupeFile {
    pub path: String,
    pub size: u64,
    pub mtime: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DupeGroup {
    /// "exact" | "similar"
    pub kind: &'static str,
    pub files: Vec<DupeFile>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum DupeEvent {
    Progress { phase: &'static str, done: u64, total: u64 },
    #[serde(rename_all = "camelCase")]
    Done { groups: Vec<DupeGroup>, elapsed_ms: u64 },
}

type Candidate = (String, u64, i64);

fn mtime_of(p: &str) -> i64 {
    std::fs::metadata(p).and_then(|m| m.modified()).map(crate::util::system_time_ms).unwrap_or(0)
}

/// Files of the roots grouped by size (only sizes with ≥ 2 files), plus all images.
fn candidates(params: &DupeParams, index: &IndexManager, cancel: &AtomicBool, progress: &dyn Fn(u64)) -> (Vec<Vec<Candidate>>, Vec<Candidate>) {
    let min = params.min_size.max(1);
    let is_image = |name: &str| {
        let ext = name.rsplit_once('.').map(|(_, e)| e.to_lowercase()).unwrap_or_default();
        matches!(crate::meta::kind_of(&ext, false), "image" | "raw") && !matches!(ext.as_str(), "svg" | "ico" | "psd" | "psb")
    };
    // Fast path: the file index already knows every size.
    if !params.similar {
        if let Some(groups) = index.same_size_groups(&params.roots, min) {
            return (groups.into_iter().map(|(s, ps)| ps.into_iter().map(|p| { let m = mtime_of(&p); (p, s, m) }).collect()).collect(), Vec::new());
        }
    }
    let roots: Vec<PathBuf> = params.roots.iter().map(|r| crate::util::expand_user_path(r)).filter(|p| p.is_dir()).collect();
    let p = Progress { dirs: AtomicUsize::new(0), cancel: AtomicBool::new(false) };
    let d = std::thread::scope(|s| {
        let h = s.spawn(|| scan::scan(&roots, &Excludes::parse(""), params.hidden, &p));
        while !h.is_finished() {
            std::thread::sleep(std::time::Duration::from_millis(200));
            if cancel.load(Ordering::Relaxed) {
                p.cancel.store(true, Ordering::Relaxed);
            }
            progress(p.dirs.load(Ordering::Relaxed) as u64);
        }
        h.join().unwrap_or_default()
    });
    let mut by_size: HashMap<u64, Vec<Candidate>> = HashMap::new();
    let mut images = Vec::new();
    for id in 0..d.nodes.len() as u32 {
        let n = d.nodes[id as usize];
        if n.flags & (flags::DIR | flags::LINK | flags::DEAD) != 0 || n.parent == NONE || n.size < min {
            continue;
        }
        let c = (d.path(id), n.size, n.mtime);
        if params.similar && is_image(d.name(id)) {
            images.push(c.clone());
        }
        by_size.entry(n.size).or_default().push(c);
    }
    (by_size.into_values().filter(|v| v.len() > 1).collect(), images)
}

fn head_tail_hash(path: &str, size: u64) -> Option<u64> {
    let mut f = std::fs::File::open(path).ok()?;
    let mut buf = vec![0u8; 16 * 1024];
    let mut h = xxhash_rust::xxh3::Xxh3::new();
    let n = f.read(&mut buf).ok()?;
    h.update(&buf[..n]);
    if size > 32 * 1024 {
        f.seek(SeekFrom::End(-16 * 1024)).ok()?;
        let n = f.read(&mut buf).ok()?;
        h.update(&buf[..n]);
    }
    Some(h.digest())
}

fn full_hash(path: &str, cancel: &AtomicBool, hashed: &AtomicU64) -> Option<u128> {
    let mut f = std::fs::File::open(path).ok()?;
    let mut buf = vec![0u8; 1 << 20];
    let mut h = xxhash_rust::xxh3::Xxh3::new();
    loop {
        if cancel.load(Ordering::Relaxed) {
            return None;
        }
        let n = f.read(&mut buf).ok()?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
        hashed.fetch_add(n as u64, Ordering::Relaxed);
    }
    Some(h.digest128())
}

/// Splits each group by a key computed in parallel; keeps sub-groups with ≥ 2 files.
fn refine<K: std::hash::Hash + Eq + Send>(
    groups: Vec<Vec<Candidate>>,
    key: impl Fn(&Candidate) -> Option<K> + Sync,
    done: &AtomicU64,
) -> Vec<Vec<Candidate>> {
    let threads = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4).clamp(2, 8);
    let flat: Vec<(usize, Candidate)> = groups.into_iter().enumerate().flat_map(|(g, v)| v.into_iter().map(move |c| (g, c))).collect();
    let chunk = flat.len().div_ceil(threads).max(1);
    let keyed: Vec<(usize, K, Candidate)> = std::thread::scope(|s| {
        let hs: Vec<_> = flat
            .chunks(chunk)
            .map(|part| {
                let key = &key;
                s.spawn(move || {
                    part.iter()
                        .filter_map(|(g, c)| {
                            done.fetch_add(1, Ordering::Relaxed);
                            key(c).map(|k| (*g, k, c.clone()))
                        })
                        .collect::<Vec<_>>()
                })
            })
            .collect();
        hs.into_iter().flat_map(|h| h.join().unwrap_or_default()).collect()
    });
    let mut out: HashMap<(usize, K), Vec<Candidate>> = HashMap::new();
    for (g, k, c) in keyed {
        out.entry((g, k)).or_default().push(c);
    }
    out.into_values().filter(|v| v.len() > 1).collect()
}

/// 64-bit difference hash of an image (from a small thumbnail).
pub fn dhash(path: &Path) -> Option<u64> {
    let t = crate::thumbs::thumbnail(path, 64)?;
    let img = image::load_from_memory(&t.bytes).ok()?.into_luma8();
    let small = image::imageops::resize(&img, 9, 8, image::imageops::FilterType::Triangle);
    let mut h = 0u64;
    for y in 0..8 {
        for x in 0..8 {
            h = (h << 1) | (small.get_pixel(x, y)[0] > small.get_pixel(x + 1, y)[0]) as u64;
        }
    }
    Some(h)
}

fn similar_groups(images: Vec<Candidate>, threshold: u32, cancel: &AtomicBool, done: &AtomicU64) -> Vec<Vec<Candidate>> {
    let threads = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4).clamp(2, 8);
    let chunk = images.len().div_ceil(threads).max(1);
    let hashed: Vec<(u64, Candidate)> = std::thread::scope(|s| {
        let hs: Vec<_> = images
            .chunks(chunk)
            .map(|part| {
                s.spawn(move || {
                    part.iter()
                        .filter_map(|c| {
                            if cancel.load(Ordering::Relaxed) {
                                return None;
                            }
                            done.fetch_add(1, Ordering::Relaxed);
                            // Flat images (blank, single colour) hash to ~0 and match each other.
                            dhash(Path::new(&c.0)).filter(|h| (4..=60).contains(&h.count_ones())).map(|h| (h, c.clone()))
                        })
                        .collect::<Vec<_>>()
                })
            })
            .collect();
        hs.into_iter().flat_map(|h| h.join().unwrap_or_default()).collect()
    });
    // Pigeonhole: with ≤ 7 differing bits two hashes share at least one of 8 bytes.
    let mut parent: Vec<usize> = (0..hashed.len()).collect();
    fn root(p: &mut [usize], mut i: usize) -> usize {
        while p[i] != i {
            p[i] = p[p[i]];
            i = p[i];
        }
        i
    }
    let threshold = threshold.min(7);
    for band in 0..8 {
        let mut buckets: HashMap<u8, Vec<usize>> = HashMap::new();
        for (i, (h, _)) in hashed.iter().enumerate() {
            buckets.entry((h >> (band * 8)) as u8).or_default().push(i);
        }
        for b in buckets.values().filter(|b| b.len() > 1 && b.len() < 5000) {
            for (x, &i) in b.iter().enumerate() {
                for &j in &b[x + 1..] {
                    if (hashed[i].0 ^ hashed[j].0).count_ones() <= threshold {
                        let (a, c) = (root(&mut parent, i), root(&mut parent, j));
                        if a != c {
                            parent[a] = c;
                        }
                    }
                }
            }
        }
    }
    let mut groups: HashMap<usize, Vec<Candidate>> = HashMap::new();
    for (i, (_, c)) in hashed.iter().enumerate() {
        let r = root(&mut parent, i);
        groups.entry(r).or_default().push(c.clone());
    }
    groups.into_values().filter(|g| g.len() > 1).collect()
}

pub fn run(params: DupeParams, index: &IndexManager, channel: &Channel<DupeEvent>, cancel: &AtomicBool) -> AppResult<()> {
    let started = Instant::now();
    let send = |phase: &'static str, done: u64, total: u64| {
        let _ = channel.send(DupeEvent::Progress { phase, done, total });
    };
    let (groups, images) = candidates(&params, index, cancel, &|dirs| send("scan", dirs, 0));
    let check = || if cancel.load(Ordering::Relaxed) { Err(AppError::cancelled()) } else { Ok(()) };
    check()?;

    // Head + tail hash, then the full content.
    let done = AtomicU64::new(0);
    let total: u64 = groups.iter().map(|g| g.len() as u64).sum();
    let groups = std::thread::scope(|s| {
        let h = s.spawn(|| refine(groups, |c| head_tail_hash(&c.0, c.1), &done));
        while !h.is_finished() {
            std::thread::sleep(std::time::Duration::from_millis(200));
            send("quick", done.load(Ordering::Relaxed), total);
        }
        h.join().unwrap_or_default()
    });
    check()?;
    let bytes: u64 = groups.iter().flat_map(|g| g.iter().map(|c| c.1)).sum();
    let hashed = AtomicU64::new(0);
    let groups = std::thread::scope(|s| {
        let h = s.spawn(|| refine(groups, |c| full_hash(&c.0, cancel, &hashed), &AtomicU64::new(0)));
        while !h.is_finished() {
            std::thread::sleep(std::time::Duration::from_millis(200));
            send("hash", hashed.load(Ordering::Relaxed), bytes);
        }
        h.join().unwrap_or_default()
    });
    check()?;
    let mut out: Vec<DupeGroup> = groups
        .into_iter()
        .map(|g| DupeGroup { kind: "exact", files: g.into_iter().map(|(path, size, mtime)| DupeFile { path, size, mtime }).collect() })
        .collect();

    if params.similar && !images.is_empty() {
        let done = AtomicU64::new(0);
        let total = images.len() as u64;
        let sim = std::thread::scope(|s| {
            let h = s.spawn(|| similar_groups(images, params.threshold, cancel, &done));
            while !h.is_finished() {
                std::thread::sleep(std::time::Duration::from_millis(200));
                send("images", done.load(Ordering::Relaxed), total);
            }
            h.join().unwrap_or_default()
        });
        check()?;
        // Skip similar groups that are just an exact group again.
        let exact: std::collections::HashSet<&str> = out.iter().flat_map(|g| g.files.iter().map(|f| f.path.as_str())).collect();
        let sim: Vec<DupeGroup> = sim
            .into_iter()
            .filter(|g| !g.iter().all(|c| exact.contains(c.0.as_str())))
            .map(|g| DupeGroup { kind: "similar", files: g.into_iter().map(|(path, size, mtime)| DupeFile { path, size, mtime }).collect() })
            .collect();
        out.extend(sim);
    }
    // Most wasted space first; inside a group the oldest file first.
    for g in &mut out {
        g.files.sort_by_key(|f| (f.mtime, f.path.len()));
    }
    out.sort_by_key(|g| std::cmp::Reverse(g.files.iter().skip(1).map(|f| f.size).sum::<u64>()));
    let _ = channel.send(DupeEvent::Done { groups: out, elapsed_ms: started.elapsed().as_millis() as u64 });
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_groups_by_content() {
        let root = std::env::temp_dir().join(format!("dualion-dupes-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(root.join("sub")).unwrap();
        let big: Vec<u8> = (0..100_000u32).map(|i| (i % 251) as u8).collect();
        let mut other = big.clone();
        other[50_000] ^= 1; // same size, same head and tail, different middle
        std::fs::write(root.join("a.bin"), &big).unwrap();
        std::fs::write(root.join("sub/b.bin"), &big).unwrap();
        std::fs::write(root.join("c.bin"), &other).unwrap();
        std::fs::write(root.join("d.txt"), "unique").unwrap();
        let groups = vec![vec![
            (root.join("a.bin").to_string_lossy().into_owned(), 100_000, 0),
            (root.join("sub/b.bin").to_string_lossy().into_owned(), 100_000, 0),
            (root.join("c.bin").to_string_lossy().into_owned(), 100_000, 0),
        ]];
        let never = AtomicBool::new(false);
        let quick = refine(groups, |c| head_tail_hash(&c.0, c.1), &AtomicU64::new(0));
        assert_eq!(quick[0].len(), 3);
        let full = refine(quick, |c| full_hash(&c.0, &never, &AtomicU64::new(0)), &AtomicU64::new(0));
        assert_eq!(full.len(), 1);
        assert_eq!(full[0].len(), 2);
        std::fs::remove_dir_all(&root).unwrap();
    }
}
