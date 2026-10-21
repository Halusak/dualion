//! Dry run of a copy / move / delete: what would happen, without touching
//! anything (counts, sizes, conflicts with existing files, free space).

use super::vfs::{self, Vfs};
use super::{JobOp, JobRequest};
use crate::error::{AppError, AppResult};
use crate::filter::NameFilter;
use crate::remote::Registry as RemoteRegistry;
use serde::Serialize;
use std::sync::atomic::{AtomicBool, Ordering};

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Conflict {
    /// Path relative to the destination folder.
    pub path: String,
    pub src_size: u64,
    pub src_mtime: i64,
    pub dst_size: u64,
    pub dst_mtime: i64,
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Plan {
    pub files: u64,
    pub dirs: u64,
    pub bytes: u64,
    pub conflicts: Vec<Conflict>,
    pub conflicts_total: u64,
    /// Conflicts where the existing file is newer than the incoming one.
    pub target_newer: u64,
    /// Conflicts with identical size and time (probably the same file).
    pub identical: u64,
    /// Bytes the destination needs (0 for moves within a volume).
    pub needed: u64,
    pub free: Option<u64>,
    /// Moves within one volume are instant renames.
    pub rename_only: bool,
    pub truncated: bool,
}

const MAX_ITEMS: u64 = 2_000_000;

pub fn plan(req: &JobRequest, reg: &RemoteRegistry, cancel: &AtomicBool) -> AppResult<Plan> {
    let (src_fs, sources) = vfs::open_sources(&req.sources, reg)?;
    let mut p = Plan::default();
    if req.op == JobOp::Delete {
        for s in &sources {
            walk(&*src_fs, s, None, "", &mut p, None, cancel)?;
        }
        return Ok(p);
    }
    let dest = req.dest.as_ref().ok_or_else(|| AppError::invalid("Missing destination"))?;
    let (dst_fs, dest_dir) = vfs::open_location(dest, reg)?;
    let filter = match req.options.filter.as_deref().map(str::trim).filter(|f| !f.is_empty()) {
        Some(f) => Some(NameFilter::parse(f, false)?).filter(|f| !f.is_noop()),
        None => None,
    };
    let single = sources.len() == 1;
    for s in &sources {
        let name = match (&req.new_name, single) {
            (Some(n), true) if !n.trim().is_empty() => n.trim().to_string(),
            _ => src_fs.name(s),
        };
        let target = dst_fs.join(&dest_dir, &name);
        walk(&*src_fs, s, Some((&*dst_fs, target)), &name, &mut p, filter.as_ref(), cancel)?;
    }
    p.rename_only = req.op == JobOp::Move && filter.is_none() && same_volume(&*src_fs, &*dst_fs, &sources, &dest_dir);
    p.needed = if p.rename_only {
        0
    } else {
        // Overwritten files free their old size.
        let replaced: u64 = p.conflicts.iter().map(|c| c.dst_size).sum::<u64>();
        p.bytes.saturating_sub(replaced.min(p.bytes))
    };
    if dst_fs.is_local() {
        p.free = crate::local::space(std::path::Path::new(&dest_dir)).map(|s| s.free);
    }
    Ok(p)
}

fn same_volume(src: &dyn Vfs, dst: &dyn Vfs, sources: &[String], dest: &str) -> bool {
    if src.id() != dst.id() {
        return false;
    }
    if !src.is_local() {
        return true;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let dev = |p: &str| std::fs::symlink_metadata(p).map(|m| m.dev()).ok();
        let d = dev(dest);
        d.is_some() && sources.iter().all(|s| dev(s) == d)
    }
    #[cfg(windows)]
    {
        let root = |p: &str| p.get(..2).map(|s| s.to_ascii_uppercase());
        sources.iter().all(|s| root(s) == root(dest))
    }
}

fn walk(
    fs: &dyn Vfs,
    path: &str,
    target: Option<(&dyn Vfs, String)>,
    rel: &str,
    p: &mut Plan,
    filter: Option<&NameFilter>,
    cancel: &AtomicBool,
) -> AppResult<()> {
    if cancel.load(Ordering::Relaxed) {
        return Err(AppError::cancelled());
    }
    if p.files + p.dirs > MAX_ITEMS {
        p.truncated = true;
        return Ok(());
    }
    let Some(m) = fs.stat(path, false)? else { return Ok(()) };
    if m.is_dir && !m.is_link {
        p.dirs += 1;
        for (name, _) in fs.read_dir(path)? {
            let child = fs.join(path, &name);
            let t = target.as_ref().map(|(tfs, t)| (*tfs, tfs.join(t, &name)));
            walk(fs, &child, t, &format!("{rel}/{name}"), p, filter, cancel)?;
        }
        return Ok(());
    }
    if let Some(f) = filter {
        if !f.matches(&fs.name(path)) {
            return Ok(());
        }
    }
    p.files += 1;
    p.bytes += m.size;
    if let Some((tfs, t)) = target {
        if let Ok(Some(tm)) = tfs.stat(&t, false) {
            if !tm.is_dir {
                p.conflicts_total += 1;
                if tm.mtime > m.mtime + 2000 {
                    p.target_newer += 1;
                }
                if tm.size == m.size && (tm.mtime - m.mtime).abs() <= 2000 {
                    p.identical += 1;
                }
                if p.conflicts.len() < 300 {
                    p.conflicts.push(Conflict { path: rel.to_string(), src_size: m.size, src_mtime: m.mtime, dst_size: tm.size, dst_mtime: tm.mtime });
                }
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::jobs::JobOptions;
    use crate::location::Location;

    #[test]
    fn counts_and_conflicts() {
        let root = std::env::temp_dir().join(format!("dualion-plan-{}", uuid::Uuid::new_v4()));
        let (src, dst) = (root.join("src/pack"), root.join("dst/pack"));
        std::fs::create_dir_all(src.join("sub")).unwrap();
        std::fs::create_dir_all(&dst).unwrap();
        std::fs::write(src.join("a.txt"), "aaaa").unwrap();
        std::fs::write(src.join("sub/b.txt"), "bb").unwrap();
        std::fs::write(dst.join("a.txt"), "old").unwrap();
        let req = JobRequest {
            op: JobOp::Copy,
            sources: vec![Location::local(src.to_string_lossy())],
            dest: Some(Location::local(root.join("dst").to_string_lossy())),
            new_name: None,
            options: JobOptions::default(),
            queue: None,
        };
        let p = plan(&req, &RemoteRegistry::default(), &AtomicBool::new(false)).unwrap();
        std::fs::remove_dir_all(&root).unwrap();
        assert_eq!((p.files, p.dirs, p.bytes), (2, 2, 6));
        assert_eq!(p.conflicts_total, 1);
        assert_eq!(p.conflicts[0].path, "pack/a.txt");
        assert_eq!(p.needed, 3);
        assert!(p.free.is_some());
    }
}
