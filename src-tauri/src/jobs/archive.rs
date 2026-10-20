//! Archive jobs: extraction and packing on top of `crate::arch` (all formats),
//! with the job's conflict prompts, progress, pause and cancel.

use super::vfs::local_meta;
use super::{ConflictChoice, ErrorChoice, FileBrief, JobCtx, JobStatus};
use crate::arch::{self, pack::Format, AEntry, Sink};
use crate::error::{AppError, AppResult};
use crate::location::Location;
use crate::util::{self, path_str};
use std::cell::Cell;
use std::fs;
use std::path::{Path, PathBuf};

pub use crate::arch::{list_archive, read_member};

pub fn extract(ctx: &JobCtx) -> AppResult<()> {
    let req = ctx.req();
    let dest = match &req.dest {
        Some(Location::Local { path }) => crate::local::normalize(path),
        _ => return Err(AppError::invalid("Extraction needs a local destination")),
    };
    ctx.set_status(JobStatus::Scanning);
    // Selected members grouped by archive (always one in practice).
    let mut groups: Vec<(String, Vec<String>)> = Vec::new();
    for src in &req.sources {
        let (archive, inner) = match src {
            Location::Archive { archive, inner } => (archive.clone(), inner.trim_matches('/').to_string()),
            Location::Local { path } => (path.clone(), String::new()),
            _ => return Err(AppError::invalid("Not an archive")),
        };
        match groups.iter_mut().find(|g| g.0 == archive) {
            Some(g) => g.1.push(inner),
            None => groups.push((archive, vec![inner])),
        }
    }
    let mut total = 0u64;
    let mut files = 0u64;
    for (archive, inners) in &groups {
        for e in arch::index(archive)?.iter().filter(|e| !e.is_dir && selected(inners, e)) {
            total += e.size;
            files += 1;
        }
    }
    ctx.set_totals(total, files);
    ctx.set_status(JobStatus::Running);

    for (archive, inners) in &groups {
        let written = Cell::new(0u64);
        let mut decide = |e: &AEntry| -> AppResult<Option<PathBuf>> {
            ctx.checkpoint()?;
            ctx.set_current(&e.path);
            let target = target_for(&dest, inners, e);
            if e.is_dir {
                return Ok(Some(target));
            }
            if let Ok(Some(t)) = local_meta(&target, false) {
                let source = FileBrief { path: format!("{archive}/{}", e.path), size: e.size, mtime: e.mtime, is_dir: false };
                let tb = FileBrief { path: path_str(&target), size: t.size, mtime: t.mtime, is_dir: t.is_dir };
                let tgt = target.clone();
                return Ok(match ctx.resolve_conflict(source, tb, || unique_local_name(&tgt))? {
                    ConflictChoice::Skip => {
                        ctx.skip(e.size);
                        None
                    }
                    ConflictChoice::RenameTo(n) => Some(target.with_file_name(n)),
                    ConflictChoice::Overwrite => Some(target),
                });
            }
            written.set(0);
            Ok(Some(target))
        };
        let progress = |n: u64| -> AppResult<()> {
            ctx.checkpoint()?;
            written.set(written.get() + n);
            ctx.add_bytes(n);
            Ok(())
        };
        let mut done = |e: &AEntry, _target: &Path, res: AppResult<()>| -> AppResult<()> {
            match res {
                Ok(()) => {
                    ctx.file_done();
                    Ok(())
                }
                Err(err) if err.is_cancelled() => Err(err),
                Err(err) => {
                    ctx.sub_bytes(written.get());
                    // Archives are read sequentially: a failed member is skipped.
                    match ctx.handle_error(err)? {
                        ErrorChoice::Retry | ErrorChoice::Skip => {
                            ctx.skip(e.size);
                            Ok(())
                        }
                    }
                }
            }
        };
        arch::extract(archive, &|e| selected(inners, e), Sink { decide: &mut decide, progress: &progress, done: &mut done })?;
    }
    Ok(())
}

fn selected(inners: &[String], e: &AEntry) -> bool {
    inners.iter().any(|i| i.is_empty() || e.path == *i || e.path.starts_with(&format!("{i}/")))
}

/// Where a member goes: paths are relative to the selected item's parent.
fn target_for(dest: &Path, inners: &[String], e: &AEntry) -> PathBuf {
    let inner = inners.iter().find(|i| i.is_empty() || e.path == **i || e.path.starts_with(&format!("{i}/"))).cloned().unwrap_or_default();
    let base = inner.rsplit_once('/').map(|(a, _)| a.to_string()).unwrap_or_default();
    let rel = if base.is_empty() { e.path.as_str() } else { &e.path[base.len() + 1..] };
    let mut target = dest.to_path_buf();
    for part in rel.split('/') {
        if part.is_empty() || part == "." || part == ".." {
            continue;
        }
        target.push(sanitize_component(part));
    }
    target
}

fn sanitize_component(part: &str) -> String {
    if cfg!(windows) {
        part.chars().map(|c| if matches!(c, '<' | '>' | ':' | '"' | '|' | '?' | '*') { '_' } else { c }).collect()
    } else {
        part.to_string()
    }
}

fn unique_local_name(target: &Path) -> String {
    let dir = target.parent().map(Path::to_path_buf).unwrap_or_default();
    let name = target.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    util::unique_name(&name, |n| dir.join(n).exists())
}

pub fn pack(ctx: &JobCtx) -> AppResult<()> {
    let req = ctx.req();
    let mut target = match &req.dest {
        Some(Location::Local { path }) => crate::local::normalize(path),
        _ => return Err(AppError::invalid("Choose a local archive name")),
    };
    let format = req
        .options
        .format
        .as_deref()
        .and_then(Format::parse)
        .or_else(|| Format::of_path(&target))
        .unwrap_or(Format::Zip);
    let sources: Vec<PathBuf> = req
        .sources
        .iter()
        .map(|s| match s {
            Location::Local { path } => Ok(crate::local::normalize(path)),
            _ => Err(AppError::unsupported("Only local files can be packed")),
        })
        .collect::<AppResult<_>>()?;

    if let Ok(Some(t)) = local_meta(&target, false) {
        let tb = FileBrief { path: path_str(&target), size: t.size, mtime: t.mtime, is_dir: t.is_dir };
        let src = FileBrief { path: path_str(&target), size: 0, mtime: util::now_ms(), is_dir: false };
        let tgt = target.clone();
        match ctx.resolve_conflict(src, tb, || unique_local_name(&tgt))? {
            ConflictChoice::Skip => return Ok(()),
            ConflictChoice::RenameTo(n) => target = target.with_file_name(n),
            ConflictChoice::Overwrite => {}
        }
    }

    ctx.set_status(JobStatus::Scanning);
    ctx.checkpoint()?;
    let items = arch::pack::collect(&sources, &target)?;
    let total: u64 = items.iter().map(|i| i.3).sum();
    ctx.set_totals(total, items.iter().filter(|i| !i.2).count() as u64);
    ctx.set_status(JobStatus::Running);

    let name = target.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let tmp = target.with_file_name(format!(".{name}.part"));
    let progress = |n: u64| -> AppResult<()> {
        ctx.checkpoint()?;
        ctx.add_bytes(n);
        Ok(())
    };
    let mut file_done = |it: &arch::pack::Item| -> AppResult<()> {
        ctx.set_current(&it.1);
        ctx.file_done();
        Ok(())
    };
    let o = &req.options;
    let res = arch::pack::pack(&tmp, format, &items, o.level.unwrap_or(6), o.password.as_deref(), o.encrypt_names, &progress, &mut file_done);
    match res {
        Ok(()) => {
            let _ = fs::remove_file(&target);
            fs::rename(&tmp, &target).map_err(|e| AppError::io(e, &target))
        }
        Err(e) => {
            let _ = fs::remove_file(&tmp);
            Err(e)
        }
    }
}
