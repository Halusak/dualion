use super::vfs::{self, Vfs};
use super::{ErrorChoice, JobCtx, JobStatus};
use crate::error::{AppResult, ErrorCode};

#[derive(Clone, Copy)]
enum Kind {
    File,
    Dir,
    /// Symlink pointing to a directory: removed as a link, never recursed.
    LinkDir,
}

pub fn run(ctx: &JobCtx) -> AppResult<()> {
    let req = ctx.req();
    let (fs, paths) = vfs::open_sources(&req.sources, ctx.ftp())?;

    if req.options.to_trash && fs.is_local() {
        ctx.set_totals(0, paths.len() as u64);
        ctx.set_status(JobStatus::Running);
        for p in &paths {
            ctx.checkpoint()?;
            ctx.set_current(p);
            loop {
                match super::undo::trash(p) {
                    Ok(trashed_to) => {
                        ctx.record(super::undo::UndoAction::Trashed { path: p.clone(), trashed_to, time: crate::util::now_ms() });
                        ctx.file_done();
                        break;
                    }
                    Err(e) => match ctx.handle_error(e)? {
                        ErrorChoice::Retry => continue,
                        ErrorChoice::Skip => {
                            ctx.skip(0);
                            break;
                        }
                    },
                }
            }
        }
        return Ok(());
    }

    ctx.set_status(JobStatus::Scanning);
    let mut list: Vec<(String, Kind)> = Vec::new();
    for p in &paths {
        loop {
            match collect(ctx, &*fs, p, &mut list) {
                Ok(()) => break,
                Err(e) => match ctx.handle_error(e)? {
                    ErrorChoice::Retry => continue,
                    ErrorChoice::Skip => break,
                },
            }
        }
    }
    ctx.set_totals(0, list.len() as u64);
    ctx.set_status(JobStatus::Running);
    let mut failed_dirs: Vec<String> = Vec::new();
    for (path, kind) in &list {
        ctx.checkpoint()?;
        ctx.set_current(path);
        // A folder whose content could not be deleted cannot be deleted either.
        if matches!(kind, Kind::Dir) && failed_dirs.iter().any(|f| f.starts_with(path.as_str())) {
            ctx.skip(0);
            continue;
        }
        loop {
            let r = match kind {
                Kind::File => fs.remove_file(path),
                Kind::Dir => fs.remove_dir(path),
                Kind::LinkDir => {
                    // Windows directory links / junctions are removed as folders; remote links as files.
                    if cfg!(windows) && fs.is_local() {
                        fs.remove_dir(path)
                    } else {
                        fs.remove_file(path)
                    }
                }
            };
            match r {
                Ok(()) => {
                    ctx.file_done();
                    break;
                }
                Err(e) if e.code == ErrorCode::NotFound => {
                    ctx.file_done();
                    break;
                }
                Err(e) => match ctx.handle_error(e)? {
                    ErrorChoice::Retry => continue,
                    ErrorChoice::Skip => {
                        failed_dirs.push(path.clone());
                        ctx.skip(0);
                        break;
                    }
                },
            }
        }
    }
    Ok(())
}

/// Post-order listing: children before their folder.
fn collect(ctx: &JobCtx, fs: &dyn Vfs, path: &str, out: &mut Vec<(String, Kind)>) -> AppResult<()> {
    ctx.checkpoint()?;
    let Some(meta) = fs.stat(path, false)? else { return Ok(()) };
    if meta.is_link && meta.is_dir {
        out.push((path.to_string(), Kind::LinkDir));
    } else if meta.is_dir {
        ctx.set_current(path);
        for (name, _) in fs.read_dir(path)? {
            collect(ctx, fs, &fs.join(path, &name), out)?;
        }
        out.push((path.to_string(), Kind::Dir));
    } else {
        out.push((path.to_string(), Kind::File));
    }
    Ok(())
}
