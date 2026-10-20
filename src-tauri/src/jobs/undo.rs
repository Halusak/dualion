//! Operation journal for undo: what a job created, moved or put in the trash,
//! and how to reverse it safely (created items go to the trash, never
//! straight to oblivion).

use super::{JobOp, JobRequest};
use crate::error::{AppError, AppResult, ErrorCode};
use crate::location::Location;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum UndoAction {
    /// A new file / folder: undone by moving it to the trash.
    Created { path: String },
    /// Moved or renamed: undone by moving it back.
    #[serde(rename_all = "camelCase")]
    Moved { from: String, to: String },
    /// Put in the trash: undone by restoring it.
    #[serde(rename_all = "camelCase")]
    Trashed { path: String, trashed_to: Option<String>, time: i64 },
}

/// Moves a path to the trash; returns where it ended up when the system says so.
pub fn trash(path: &str) -> AppResult<Option<String>> {
    #[cfg(target_os = "macos")]
    {
        use objc2::rc::Retained;
        use objc2_foundation::{NSFileManager, NSString, NSURL};
        let url = NSURL::fileURLWithPath(&NSString::from_str(path));
        let fm = NSFileManager::defaultManager();
        let mut out: Option<Retained<NSURL>> = None;
        match fm.trashItemAtURL_resultingItemURL_error(&url, Some(&mut out)) {
            Ok(()) => Ok(out.and_then(|u| u.path()).map(|p| p.to_string())),
            Err(e) => {
                if !Path::new(path).exists() && std::fs::symlink_metadata(path).is_err() {
                    return Err(AppError::new(ErrorCode::NotFound, "Not found").with_path(path));
                }
                Err(AppError::new(ErrorCode::Io, e.localizedDescription().to_string()).with_path(path))
            }
        }
    }
    #[cfg(not(target_os = "macos"))]
    {
        trash::delete(path).map_err(|e| AppError::new(ErrorCode::Io, e.to_string()).with_path(path))?;
        Ok(None)
    }
}

/// State of the destination before a job runs, to tell what the job added.
pub struct Before {
    dest_dir: Option<PathBuf>,
    existing: HashSet<String>,
    archive_existed: bool,
}

fn local(loc: &Location) -> Option<&str> {
    match loc {
        Location::Local { path } => Some(path),
        _ => None,
    }
}

fn names(dir: &Path) -> HashSet<String> {
    std::fs::read_dir(dir)
        .map(|rd| rd.flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect())
        .unwrap_or_default()
}

pub fn before(req: &JobRequest) -> Option<Before> {
    let dest = local(req.dest.as_ref()?)?;
    let p = PathBuf::from(dest);
    match req.op {
        JobOp::Copy | JobOp::Move | JobOp::Extract => Some(Before { existing: names(&p), dest_dir: Some(p), archive_existed: false }),
        JobOp::Pack => Some(Before { dest_dir: None, existing: HashSet::new(), archive_existed: p.exists() }),
        JobOp::Delete => None,
    }
}

/// Undo actions for a finished (or stopped) job.
pub fn after(req: &JobRequest, before: Option<Before>, mut journal: Vec<UndoAction>) -> Vec<UndoAction> {
    let Some(b) = before else { return journal };
    match req.op {
        JobOp::Pack => {
            if let Some(dest) = req.dest.as_ref().and_then(local) {
                if !b.archive_existed && Path::new(dest).exists() {
                    journal.push(UndoAction::Created { path: dest.to_string() });
                }
            }
        }
        JobOp::Extract => {
            if let Some(dir) = &b.dest_dir {
                for n in names(dir).difference(&b.existing) {
                    journal.push(UndoAction::Created { path: dir.join(n).to_string_lossy().into_owned() });
                }
            }
        }
        JobOp::Copy | JobOp::Move => {
            let Some(dir) = &b.dest_dir else { return journal };
            let now = names(dir);
            let single = req.sources.len() == 1;
            for s in &req.sources {
                let Some(src) = local(s) else { continue };
                let name = match (&req.new_name, single) {
                    (Some(n), true) if !n.trim().is_empty() => n.trim().to_string(),
                    _ => Path::new(src).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(),
                };
                // Only items the job brought in (not merged into existing ones).
                if name.is_empty() || b.existing.contains(&name) || !now.contains(&name) {
                    continue;
                }
                let target = dir.join(&name).to_string_lossy().into_owned();
                if req.op == JobOp::Move {
                    if std::fs::symlink_metadata(src).is_err() {
                        journal.push(UndoAction::Moved { from: src.to_string(), to: target });
                    }
                } else {
                    journal.push(UndoAction::Created { path: target });
                }
            }
        }
        JobOp::Delete => {}
    }
    journal
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UndoReport {
    pub done: usize,
    pub failed: Vec<AppError>,
    /// Folders whose listings changed.
    pub touched: Vec<String>,
    /// Moves across volumes run as jobs.
    pub moves: Vec<(String, String)>,
}

/// Reverses actions (latest first). Cross-volume moves back are returned in
/// `moves` for the caller to queue as jobs.
pub fn apply(actions: &[UndoAction]) -> UndoReport {
    let mut r = UndoReport::default();
    let mut touched = HashSet::new();
    let parent = |p: &str| Path::new(p).parent().map(|d| d.to_string_lossy().into_owned()).unwrap_or_default();
    for a in actions.iter().rev() {
        let res: AppResult<()> = match a {
            UndoAction::Created { path } => {
                touched.insert(parent(path));
                match trash(path) {
                    Ok(_) => Ok(()),
                    Err(e) if e.code == ErrorCode::NotFound => Ok(()),
                    Err(e) => Err(e),
                }
            }
            UndoAction::Moved { from, to } => {
                touched.insert(parent(from));
                touched.insert(parent(to));
                if std::fs::symlink_metadata(from).is_ok() {
                    Err(AppError::new(ErrorCode::AlreadyExists, "Something already exists at the original place").with_path(from))
                } else {
                    let _ = std::fs::create_dir_all(parent(from));
                    match std::fs::rename(to, from) {
                        Ok(()) => Ok(()),
                        Err(e) if e.raw_os_error() == Some(libc::EXDEV) || e.kind() == std::io::ErrorKind::CrossesDevices => {
                            r.moves.push((to.clone(), from.clone()));
                            Ok(())
                        }
                        Err(e) => Err(AppError::io(e, to)),
                    }
                }
            }
            UndoAction::Trashed { path, trashed_to, time } => {
                touched.insert(parent(path));
                restore(path, trashed_to.as_deref(), *time)
            }
        };
        match res {
            Ok(()) => r.done += 1,
            Err(e) => r.failed.push(e),
        }
    }
    r.touched = touched.into_iter().filter(|t| !t.is_empty()).collect();
    r
}

fn restore(path: &str, trashed_to: Option<&str>, time: i64) -> AppResult<()> {
    if std::fs::symlink_metadata(path).is_ok() {
        return Err(AppError::new(ErrorCode::AlreadyExists, "Something already exists at the original place").with_path(path));
    }
    if let Some(t) = trashed_to {
        if let Some(p) = Path::new(path).parent() {
            let _ = std::fs::create_dir_all(p);
        }
        return std::fs::rename(t, path).map_err(|e| AppError::io(e, t));
    }
    #[cfg(any(windows, all(unix, not(target_os = "macos"))))]
    {
        let want = Path::new(path);
        let items = trash::os_limited::list().map_err(|e| AppError::other(e.to_string()))?;
        let best = items
            .into_iter()
            .filter(|i| i.original_parent.join(&i.name) == want)
            // The deletion closest to when we trashed it.
            .min_by_key(|i| (i.time_deleted - time / 1000).abs());
        let Some(item) = best else {
            return Err(AppError::new(ErrorCode::NotFound, "Not found in the trash").with_path(path));
        };
        trash::os_limited::restore_all([item]).map_err(|e| AppError::other(e.to_string()).with_path(path))
    }
    #[cfg(not(any(windows, all(unix, not(target_os = "macos")))))]
    {
        let _ = time;
        Err(AppError::new(ErrorCode::NotFound, "Not found in the trash").with_path(path))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::jobs::JobOptions;

    #[test]
    fn copy_and_move_journal() {
        let root = std::env::temp_dir().join(format!("dualion-undo-{}", uuid::Uuid::new_v4()));
        let (src, dst) = (root.join("src"), root.join("dst"));
        std::fs::create_dir_all(&src).unwrap();
        std::fs::create_dir_all(dst.join("old")).unwrap();
        std::fs::write(src.join("a.txt"), "a").unwrap();
        std::fs::create_dir_all(src.join("old")).unwrap();
        let s = |p: &Path| p.to_string_lossy().into_owned();
        let req = JobRequest {
            op: JobOp::Move,
            sources: vec![Location::local(s(&src.join("a.txt"))), Location::local(s(&src.join("old")))],
            dest: Some(Location::local(s(&dst))),
            new_name: None,
            options: JobOptions::default(),
            queue: None,
        };
        let b = before(&req);
        std::fs::rename(src.join("a.txt"), dst.join("a.txt")).unwrap();
        let acts = after(&req, b, vec![]);
        // "old" existed at the destination (merge) and is not journaled.
        assert_eq!(acts, vec![UndoAction::Moved { from: s(&src.join("a.txt")), to: s(&dst.join("a.txt")) }]);
        let rep = apply(&acts);
        assert_eq!((rep.done, rep.failed.len()), (1, 0));
        assert!(src.join("a.txt").exists() && !dst.join("a.txt").exists());
        std::fs::remove_dir_all(&root).unwrap();
    }
}
