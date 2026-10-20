//! Copy and move between any two file systems (local ↔ local, local ↔ FTP).

use super::vfs::{self, Meta, Vfs};
use super::{ConflictChoice, ErrorChoice, FileBrief, JobCtx, JobOp, JobStatus};
use crate::error::{AppError, AppResult, ErrorCode};
use crate::filter::NameFilter;
use crate::location::Location;
use crate::util;
use std::collections::HashSet;
use std::fs;
use std::io::{Read, Write};
use std::path::Path;

struct Item {
    src: String,
    /// Path components below the destination directory.
    rel: Vec<String>,
    meta: Meta,
}

pub fn run(ctx: &JobCtx) -> AppResult<()> {
    let req = ctx.req();
    if matches!(req.sources.first(), Some(Location::Archive { .. })) {
        return super::archive::extract(ctx);
    }
    let is_move = req.op == JobOp::Move;
    let dest_loc = req.dest.as_ref().ok_or_else(|| AppError::invalid("Missing destination"))?;
    let (src_fs, sources) = vfs::open_sources(&req.sources, ctx.ftp())?;
    let (dst_fs, dest_dir) = vfs::open_location(dest_loc, ctx.ftp())?;
    let same_fs = src_fs.id() == dst_fs.id();
    let opts = &req.options;
    let filter = match opts.filter.as_deref().map(str::trim).filter(|f| !f.is_empty()) {
        Some(f) => Some(NameFilter::parse(f, false)?).filter(|f| !f.is_noop()),
        None => None,
    };

    match dst_fs.stat(&dest_dir, true)? {
        Some(m) if m.is_dir => {}
        Some(_) => return Err(AppError::new(ErrorCode::NotADirectory, "Target is not a folder").with_path(&dest_dir)),
        None => ensure_dirs(&*dst_fs, &dest_dir)?,
    }

    let mut work: Vec<(String, String)> = Vec::with_capacity(sources.len());
    for s in &sources {
        let name = match (&req.new_name, sources.len()) {
            (Some(n), 1) if !n.trim().is_empty() => n.trim().to_string(),
            _ => src_fs.name(s),
        };
        if same_fs && dst_fs.is_local() && util::is_same_or_inside(Path::new(&dest_dir), Path::new(s)) {
            if let Ok(Some(m)) = src_fs.stat(s, false) {
                if m.is_dir && !m.is_link {
                    return Err(AppError::new(ErrorCode::IntoItself, "Cannot copy a folder into itself").with_path(s));
                }
            }
        }
        work.push((s.clone(), name));
    }

    // Moves inside one file system: rename whole items when nothing is in the way.
    let mut pending = Vec::new();
    if is_move && same_fs && filter.is_none() {
        ctx.set_status(JobStatus::Running);
        ctx.set_totals(0, work.len() as u64);
        for (src, name) in work {
            ctx.checkpoint()?;
            ctx.set_current(&src);
            let target = dst_fs.join(&dest_dir, &name);
            if same_path(&src, &target) {
                ctx.skip(0);
                continue;
            }
            let case_rename = dst_fs.is_local() && src.to_lowercase() == target.to_lowercase();
            if !case_rename && dst_fs.stat(&target, false)?.is_some() {
                pending.push((src, name));
                continue;
            }
            match src_fs.rename(&src, &target) {
                Ok(()) => ctx.file_done(),
                Err(e) if e.code == ErrorCode::CrossesDevices => pending.push((src, name)),
                Err(e) => match ctx.handle_error(e)? {
                    ErrorChoice::Retry => pending.push((src, name)),
                    ErrorChoice::Skip => ctx.skip(0),
                },
            }
        }
        if pending.is_empty() {
            return Ok(());
        }
    } else {
        pending = work;
    }

    // Scan
    ctx.set_status(JobStatus::Scanning);
    let mut items = Vec::new();
    for (src, name) in &pending {
        loop {
            match scan(ctx, &*src_fs, src, vec![name.clone()], &mut items, filter.as_ref(), true) {
                Ok(()) => break,
                Err(e) => match ctx.handle_error(e)? {
                    ErrorChoice::Retry => continue,
                    ErrorChoice::Skip => break,
                },
            }
        }
    }
    let (bytes, files) = items.iter().fold((0u64, 0u64), |(b, f), i| {
        if i.meta.is_dir && !i.meta.is_link { (b, f) } else { (b + i.meta.size, f + 1) }
    });
    if is_move && same_fs && filter.is_none() {
        ctx.add_totals(bytes, files);
    } else {
        ctx.set_totals(bytes, files);
    }
    ctx.set_status(JobStatus::Running);

    // Execute
    let follow = opts.follow_links;
    let mut skipped_dirs: Vec<String> = Vec::new();
    let mut made_dirs: HashSet<String> = HashSet::new();
    let mut src_dirs: Vec<String> = Vec::new();
    let mut buf = vec![0u8; ctx.buffer_size()];

    for item in &items {
        ctx.checkpoint()?;
        if skipped_dirs.iter().any(|d| is_under(&item.src, d)) {
            if !item.meta.is_dir {
                ctx.skip(item.meta.size);
            }
            continue;
        }
        let mut target = item.rel.iter().fold(dest_dir.clone(), |acc, part| dst_fs.join(&acc, part));
        ctx.set_current(&item.src);

        if item.meta.is_dir && !(item.meta.is_link && !follow) {
            loop {
                match ensure_dir_once(&*dst_fs, &target, &mut made_dirs) {
                    Ok(()) => break,
                    Err(e) => match ctx.handle_error(e)? {
                        ErrorChoice::Retry => continue,
                        ErrorChoice::Skip => {
                            skipped_dirs.push(item.src.clone());
                            break;
                        }
                    },
                }
            }
            if is_move {
                src_dirs.push(item.src.clone());
            }
            continue;
        }

        if filter.is_some() {
            if let Some(parent) = dst_fs.parent(&target) {
                let _ = ensure_dir_once(&*dst_fs, &parent, &mut made_dirs);
            }
        }

        // Conflict handling
        let existing = match dst_fs.stat(&target, false) {
            Ok(m) => m,
            Err(e) => match ctx.handle_error(e)? {
                ErrorChoice::Retry => dst_fs.stat(&target, false).ok().flatten(),
                ErrorChoice::Skip => {
                    ctx.skip(item.meta.size);
                    continue;
                }
            },
        };
        let mut overwrite = false;
        if let Some(t) = existing {
            if same_fs && same_path(&item.src, &target) {
                if is_move {
                    ctx.skip(item.meta.size);
                    continue;
                }
                target = unique_target(&*dst_fs, &target);
            } else {
                let source = FileBrief { path: item.src.clone(), size: item.meta.size, mtime: item.meta.mtime, is_dir: false };
                let tb = FileBrief { path: target.clone(), size: t.size, mtime: t.mtime, is_dir: t.is_dir };
                let fs_ref = &*dst_fs;
                let tgt = target.clone();
                match ctx.resolve_conflict(source, tb, || fs_ref.name(&unique_target(fs_ref, &tgt)))? {
                    ConflictChoice::Skip => {
                        ctx.skip(item.meta.size);
                        continue;
                    }
                    ConflictChoice::RenameTo(name) => {
                        let parent = dst_fs.parent(&target).unwrap_or_else(|| dest_dir.clone());
                        target = dst_fs.join(&parent, &name);
                        if dst_fs.stat(&target, false)?.is_some() {
                            target = unique_target(&*dst_fs, &target);
                        }
                    }
                    ConflictChoice::Overwrite => {
                        if t.is_dir && !t.is_link {
                            // A file cannot replace a folder; whatever the user answers, skip it.
                            let e = AppError::new(ErrorCode::IsADirectory, "A folder with this name exists").with_path(&target);
                            ctx.handle_error(e)?;
                            ctx.skip(item.meta.size);
                            continue;
                        }
                        overwrite = true;
                    }
                }
            }
        }

        // Transfer (with retry)
        let mut done = false;
        loop {
            let before = ctx_done_bytes(ctx);
            let res = if is_move && same_fs {
                if overwrite && !dst_fs.is_local() {
                    let _ = dst_fs.remove_file(&target);
                }
                src_fs.rename(&item.src, &target).map(|_| {
                    ctx.add_bytes(item.meta.size);
                    true
                })
            } else {
                transfer(ctx, &*src_fs, &*dst_fs, item, &target, overwrite, &mut buf).map(|_| false)
            };
            match res {
                Ok(renamed) => {
                    done = true;
                    if is_move && !renamed {
                        if let Err(e) = remove_source(ctx, &*src_fs, &item.src) {
                            if e.is_cancelled() {
                                return Err(e);
                            }
                        }
                    }
                    break;
                }
                Err(e) if e.is_cancelled() => return Err(e),
                Err(e) => {
                    let partial = ctx_done_bytes(ctx).saturating_sub(before);
                    ctx.sub_bytes(partial);
                    match ctx.handle_error(e)? {
                        ErrorChoice::Retry => continue,
                        ErrorChoice::Skip => break,
                    }
                }
            }
        }
        if done {
            ctx.file_done();
        } else {
            ctx.skip(item.meta.size);
        }
    }

    // Moves: remove emptied source folders, deepest first.
    for d in src_dirs.iter().rev() {
        let _ = src_fs.remove_dir(d);
    }
    Ok(())
}

fn ctx_done_bytes(ctx: &JobCtx) -> u64 {
    ctx.job.snapshot_done_bytes()
}

fn remove_source(ctx: &JobCtx, fs: &dyn Vfs, path: &str) -> AppResult<()> {
    loop {
        match fs.remove_file(path) {
            Ok(()) => return Ok(()),
            Err(e) => match ctx.handle_error(e)? {
                ErrorChoice::Retry => continue,
                ErrorChoice::Skip => return Ok(()),
            },
        }
    }
}

fn same_path(a: &str, b: &str) -> bool {
    if cfg!(any(windows, target_os = "macos")) {
        a.to_lowercase() == b.to_lowercase()
    } else {
        a == b
    }
}

fn is_under(path: &str, dir: &str) -> bool {
    path.len() > dir.len() && path.starts_with(dir) && matches!(path.as_bytes()[dir.len()], b'/' | b'\\')
}

fn unique_target(fs: &dyn Vfs, target: &str) -> String {
    let parent = fs.parent(target).unwrap_or_default();
    let name = fs.name(target);
    let unique = util::unique_name(&name, |n| matches!(fs.stat(&fs.join(&parent, n), false), Ok(Some(_))));
    fs.join(&parent, &unique)
}

fn ensure_dirs(fs: &dyn Vfs, path: &str) -> AppResult<()> {
    if fs.stat(path, true)?.is_some() {
        return Ok(());
    }
    if let Some(p) = fs.parent(path) {
        if p != path {
            ensure_dirs(fs, &p)?;
        }
    }
    fs.mkdir(path)
}

fn ensure_dir_once(fs: &dyn Vfs, path: &str, made: &mut HashSet<String>) -> AppResult<()> {
    if made.contains(path) {
        return Ok(());
    }
    match fs.stat(path, true)? {
        Some(m) if m.is_dir => {}
        Some(_) => return Err(AppError::new(ErrorCode::AlreadyExists, "A file with this name exists").with_path(path)),
        None => ensure_dirs(fs, path)?,
    }
    made.insert(path.to_string());
    Ok(())
}

fn scan(
    ctx: &JobCtx,
    fs: &dyn Vfs,
    path: &str,
    rel: Vec<String>,
    out: &mut Vec<Item>,
    filter: Option<&NameFilter>,
    top: bool,
) -> AppResult<()> {
    ctx.checkpoint()?;
    let follow = ctx.req().options.follow_links;
    let meta = fs
        .stat(path, follow)?
        .ok_or_else(|| AppError::new(ErrorCode::NotFound, "Not found").with_path(path))?;
    if meta.is_dir && (!meta.is_link || follow) {
        if !top {
            if let Some(f) = filter {
                if f.excluded(&fs.name(path)) {
                    return Ok(());
                }
            }
        }
        ctx.set_current(path);
        out.push(Item { src: path.to_string(), rel: rel.clone(), meta });
        let mut children = fs.read_dir(path)?;
        children.sort_by(|a, b| b.1.is_dir.cmp(&a.1.is_dir).then_with(|| a.0.cmp(&b.0)));
        for (name, m) in children {
            let child = fs.join(path, &name);
            let mut r = rel.clone();
            r.push(name.clone());
            if m.is_dir && (!m.is_link || follow) {
                if let Err(e) = scan(ctx, fs, &child, r, out, filter, false) {
                    match ctx.handle_error(e)? {
                        ErrorChoice::Retry => scan(ctx, fs, &child, rel_with(&rel, &name), out, filter, false)?,
                        ErrorChoice::Skip => {}
                    }
                }
            } else if filter.map(|f| f.matches(&name)).unwrap_or(true) {
                let m = if m.is_link && follow { fs.stat(&child, true)?.unwrap_or(m) } else { m };
                out.push(Item { src: child, rel: r, meta: m });
            }
        }
    } else if top || filter.map(|f| f.matches(&fs.name(path))).unwrap_or(true) {
        out.push(Item { src: path.to_string(), rel, meta });
    }
    Ok(())
}

fn rel_with(rel: &[String], name: &str) -> Vec<String> {
    let mut r = rel.to_vec();
    r.push(name.to_string());
    r
}

fn transfer(
    ctx: &JobCtx,
    src_fs: &dyn Vfs,
    dst_fs: &dyn Vfs,
    item: &Item,
    target: &str,
    overwrite: bool,
    buf: &mut [u8],
) -> AppResult<()> {
    let opts = &ctx.req().options;
    if src_fs.is_local() && dst_fs.is_local() {
        let (src, dst) = (Path::new(&item.src), Path::new(target));
        if item.meta.is_link && !opts.follow_links {
            return copy_symlink(src, dst, overwrite);
        }
        let cloned = copy_local_file(ctx, src, dst, item.meta.size, overwrite, buf)?;
        if opts.verify && !cloned {
            ctx.set_current(&item.src);
            if hash_file(ctx, src)? != hash_file(ctx, dst)? {
                return Err(AppError::new(ErrorCode::VerifyFailed, "Copied file differs from the source").with_path(dst));
            }
        }
        return Ok(());
    }

    let mut reader = src_fs.reader(&item.src)?;
    let mut writer = match dst_fs.writer(target) {
        Ok(w) => w,
        Err(e) => {
            drop(reader);
            return Err(e);
        }
    };
    let res = pump(ctx, &mut *reader, &mut *writer, buf, &item.src, target);
    match res {
        Ok(()) => {
            writer.finish()?;
            reader.finish()?;
            if opts.preserve_times {
                dst_fs.set_mtime(target, item.meta.mtime);
            }
            if opts.verify {
                if let Ok(Some(m)) = dst_fs.stat(target, false) {
                    if m.size != item.meta.size {
                        return Err(AppError::new(ErrorCode::VerifyFailed, "Size mismatch after transfer").with_path(target));
                    }
                }
            }
            Ok(())
        }
        Err(e) => {
            drop(writer);
            drop(reader);
            let _ = dst_fs.remove_file(target);
            Err(e)
        }
    }
}

fn pump(ctx: &JobCtx, r: &mut dyn Read, w: &mut dyn Write, buf: &mut [u8], src: &str, dst: &str) -> AppResult<()> {
    loop {
        ctx.checkpoint()?;
        let n = match r.read(buf) {
            Ok(0) => return Ok(()),
            Ok(n) => n,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(AppError::io(e, src)),
        };
        w.write_all(&buf[..n]).map_err(|e| AppError::io(e, dst))?;
        ctx.add_bytes(n as u64);
    }
}

fn copy_symlink(src: &Path, dst: &Path, overwrite: bool) -> AppResult<()> {
    let target = fs::read_link(src).map_err(|e| AppError::io(e, src))?;
    if overwrite {
        let _ = fs::remove_file(dst);
    }
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(&target, dst).map_err(|e| AppError::io(e, dst))
    }
    #[cfg(windows)]
    {
        let r = if fs::metadata(src).map(|m| m.is_dir()).unwrap_or(false) {
            std::os::windows::fs::symlink_dir(&target, dst)
        } else {
            std::os::windows::fs::symlink_file(&target, dst)
        };
        // Creating links needs developer mode or admin rights; fall back to copying content.
        r.or_else(|_| fs::copy(src, dst).map(|_| ())).map_err(|e| AppError::io(e, dst))
    }
}

/// Copies a local file with progress. Returns `true` when the file was
/// cloned (APFS copy-on-write), which makes verification pointless.
pub fn copy_local_file(ctx: &JobCtx, src: &Path, dst: &Path, size: u64, overwrite: bool, buf: &mut [u8]) -> AppResult<bool> {
    let preserve = ctx.req().options.preserve_times;

    #[cfg(target_os = "macos")]
    {
        if overwrite {
            let _ = fs::remove_file(dst);
        }
        if clonefile(src, dst) {
            ctx.add_bytes(size);
            return Ok(true);
        }
    }
    #[cfg(windows)]
    {
        if size >= 8 * 1024 * 1024 && ctx.req().options.speed_limit.is_none() {
            match win::copy_file_ex(ctx, src, dst) {
                Ok(()) => return Ok(false),
                Err(e) if e.is_cancelled() => return Err(e),
                Err(_) => {} // fall through to the portable path
            }
        }
    }
    let _ = overwrite;

    if size <= 2 * 1024 * 1024 {
        // Small files: let the OS fast path (fcopyfile / CopyFileExW / copy_file_range) do it.
        fs::copy(src, dst).map_err(|e| AppError::io(e, src))?;
        ctx.add_bytes(size);
    } else {
        let mut r = fs::File::open(src).map_err(|e| AppError::io(e, src))?;
        let mut w = fs::File::create(dst).map_err(|e| AppError::io(e, dst))?;
        let res = (|| -> AppResult<()> {
            loop {
                ctx.checkpoint()?;
                let n = match r.read(buf) {
                    Ok(0) => break,
                    Ok(n) => n,
                    Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                    Err(e) => return Err(AppError::io(e, src)),
                };
                w.write_all(&buf[..n]).map_err(|e| AppError::io(e, dst))?;
                ctx.add_bytes(n as u64);
            }
            w.flush().map_err(|e| AppError::io(e, dst))
        })();
        if let Err(e) = res {
            drop(w);
            let _ = fs::remove_file(dst);
            return Err(e);
        }
        drop(w);
        if let Ok(m) = fs::metadata(src) {
            let _ = fs::set_permissions(dst, m.permissions());
        }
    }
    if preserve {
        if let Ok(m) = fs::metadata(src) {
            let mtime = filetime::FileTime::from_last_modification_time(&m);
            let atime = filetime::FileTime::from_last_access_time(&m);
            let _ = filetime::set_file_times(dst, atime, mtime);
        }
    }
    Ok(false)
}

#[cfg(target_os = "macos")]
fn clonefile(src: &Path, dst: &Path) -> bool {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;
    extern "C" {
        fn clonefile(src: *const libc::c_char, dst: *const libc::c_char, flags: u32) -> libc::c_int;
    }
    const CLONE_NOFOLLOW: u32 = 0x0001;
    let (Ok(s), Ok(d)) = (CString::new(src.as_os_str().as_bytes()), CString::new(dst.as_os_str().as_bytes())) else {
        return false;
    };
    unsafe { clonefile(s.as_ptr(), d.as_ptr(), CLONE_NOFOLLOW) == 0 }
}

fn hash_file(ctx: &JobCtx, path: &Path) -> AppResult<u128> {
    let mut f = fs::File::open(path).map_err(|e| AppError::io(e, path))?;
    let mut h = xxhash_rust::xxh3::Xxh3::new();
    let mut buf = vec![0u8; 1024 * 1024];
    loop {
        ctx.checkpoint()?;
        let n = f.read(&mut buf).map_err(|e| AppError::io(e, path))?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    Ok(h.digest128())
}

#[cfg(windows)]
mod win {
    use super::*;
    use std::os::windows::ffi::OsStrExt;
    use windows::core::{BOOL, PCWSTR};
    use windows::Win32::Foundation::HANDLE;
    use windows::Win32::Storage::FileSystem::{
        CopyFileExW, COPYFILE_FLAGS, COPYPROGRESSROUTINE_PROGRESS, LPPROGRESS_ROUTINE_CALLBACK_REASON, PROGRESS_CANCEL,
        PROGRESS_CONTINUE,
    };

    struct CbState<'a, 'b> {
        ctx: &'a JobCtx<'b>,
        last: u64,
        cancelled: bool,
    }

    unsafe extern "system" fn progress(
        _total: i64,
        transferred: i64,
        _stream_size: i64,
        _stream_transferred: i64,
        _stream: u32,
        _reason: LPPROGRESS_ROUTINE_CALLBACK_REASON,
        _src: HANDLE,
        _dst: HANDLE,
        data: *const core::ffi::c_void,
    ) -> COPYPROGRESSROUTINE_PROGRESS {
        let state = unsafe { &mut *(data as *mut CbState) };
        let t = transferred.max(0) as u64;
        if t > state.last {
            state.ctx.add_bytes(t - state.last);
            state.last = t;
        }
        if state.ctx.checkpoint().is_err() {
            state.cancelled = true;
            return PROGRESS_CANCEL;
        }
        PROGRESS_CONTINUE
    }

    fn wide(p: &Path) -> Vec<u16> {
        p.as_os_str().encode_wide().chain(std::iter::once(0)).collect()
    }

    pub fn copy_file_ex(ctx: &JobCtx, src: &Path, dst: &Path) -> AppResult<()> {
        let s = wide(src);
        let d = wide(dst);
        let mut state = CbState { ctx, last: 0, cancelled: false };
        let mut cancel = BOOL(0);
        let r = unsafe {
            CopyFileExW(
                PCWSTR(s.as_ptr()),
                PCWSTR(d.as_ptr()),
                Some(progress),
                Some(&mut state as *mut CbState as *const core::ffi::c_void),
                Some(&mut cancel),
                COPYFILE_FLAGS(0),
            )
        };
        match r {
            Ok(()) => Ok(()),
            Err(_) if state.cancelled => {
                ctx.sub_bytes(state.last);
                Err(AppError::cancelled())
            }
            Err(e) => {
                ctx.sub_bytes(state.last);
                Err(AppError::io(std::io::Error::from(e), src))
            }
        }
    }
}
