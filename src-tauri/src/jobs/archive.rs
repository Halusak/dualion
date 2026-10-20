//! ZIP archives: browsing as a folder, packing and extracting.

use super::vfs::local_meta;
use super::{ConflictChoice, ErrorChoice, FileBrief, JobCtx, JobStatus};
use crate::error::{AppError, AppResult, ErrorCode};
use crate::location::{flags, Crumb, Entry, Listing, Location};
use crate::util::{self, path_str, system_time_ms};
use lru::LruCache;
use parking_lot::Mutex;
use std::collections::BTreeMap;
use std::fs;
use std::io::{Read, Write};
use std::num::NonZeroUsize;
use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock};
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipArchive, ZipWriter};

#[derive(Debug, Clone)]
pub struct ZEntry {
    pub index: usize,
    /// Normalized path without leading/trailing slashes.
    pub path: String,
    pub is_dir: bool,
    pub size: u64,
    pub mtime: i64,
    pub mode: Option<u32>,
}

type IndexKey = (String, i64, u64);
static INDEX: LazyLock<Mutex<LruCache<IndexKey, Arc<Vec<ZEntry>>>>> =
    LazyLock::new(|| Mutex::new(LruCache::new(NonZeroUsize::new(8).unwrap())));

fn zip_time_ms(dt: Option<zip::DateTime>) -> i64 {
    match dt {
        Some(d) => util::local_naive_to_ms(
            d.year() as i32,
            d.month() as u32,
            d.day() as u32,
            d.hour() as u32,
            d.minute() as u32,
            d.second() as u32,
        ),
        None => 0,
    }
}

pub fn index(archive: &str) -> AppResult<Arc<Vec<ZEntry>>> {
    let meta = fs::metadata(archive).map_err(|e| AppError::io(e, archive))?;
    let key = (archive.to_string(), meta.modified().map(system_time_ms).unwrap_or(0), meta.len());
    if let Some(hit) = INDEX.lock().get(&key) {
        return Ok(hit.clone());
    }
    let f = fs::File::open(archive).map_err(|e| AppError::io(e, archive))?;
    let mut zip = ZipArchive::new(std::io::BufReader::new(f))?;
    let mut out = Vec::with_capacity(zip.len());
    for i in 0..zip.len() {
        let file = zip.by_index_raw(i)?;
        let raw = file.name()?.replace('\\', "/");
        let path = raw.trim_matches('/').to_string();
        if path.is_empty() || path.split('/').any(|c| c == "..") {
            continue;
        }
        out.push(ZEntry {
            index: i,
            is_dir: file.is_dir(),
            size: file.size(),
            mtime: zip_time_ms(file.last_modified()),
            mode: file.unix_mode(),
            path,
        });
    }
    let arc = Arc::new(out);
    INDEX.lock().put(key, arc.clone());
    Ok(arc)
}

/// Lists one directory level inside the archive, including folders that
/// only exist implicitly as prefixes of file paths.
pub fn list_archive(archive: &str, inner: &str) -> AppResult<Listing> {
    let inner = inner.trim_matches('/');
    let idx = index(archive)?;
    let prefix = if inner.is_empty() { String::new() } else { format!("{inner}/") };
    let mut children: BTreeMap<String, Entry> = BTreeMap::new();
    for e in idx.iter() {
        let Some(rest) = e.path.strip_prefix(&prefix) else { continue };
        if rest.is_empty() {
            continue;
        }
        match rest.split_once('/') {
            Some((dir, _)) => {
                children.entry(dir.to_string()).or_insert_with(|| Entry {
                    name: dir.to_string(),
                    flags: flags::DIR,
                    size: 0,
                    mtime: 0,
                    attrs: 0,
                });
            }
            None => {
                let mut fl = if e.is_dir { flags::DIR } else { 0 };
                if rest.starts_with('.') {
                    fl |= flags::HIDDEN;
                }
                children.insert(
                    rest.to_string(),
                    Entry { name: rest.to_string(), flags: fl, size: e.size, mtime: e.mtime, attrs: e.mode.unwrap_or(0) },
                );
            }
        }
    }

    let archive_name = Path::new(archive).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let mut crumbs = crate::local::local_crumbs(Path::new(archive).parent().unwrap_or(Path::new("/")));
    crumbs.push(Crumb {
        label: archive_name,
        icon: "archive",
        location: Location::Archive { archive: archive.to_string(), inner: String::new() },
    });
    let mut acc = String::new();
    for part in inner.split('/').filter(|s| !s.is_empty()) {
        if !acc.is_empty() {
            acc.push('/');
        }
        acc.push_str(part);
        crumbs.push(Crumb {
            label: part.to_string(),
            icon: "folder",
            location: Location::Archive { archive: archive.to_string(), inner: acc.clone() },
        });
    }
    let parent = if inner.is_empty() {
        Path::new(archive).parent().map(|p| Location::local(path_str(p)))
    } else {
        let p = inner.rsplit_once('/').map(|(a, _)| a.to_string()).unwrap_or_default();
        Some(Location::Archive { archive: archive.to_string(), inner: p })
    };
    Ok(Listing {
        location: Location::Archive { archive: archive.to_string(), inner: inner.to_string() },
        display_path: if inner.is_empty() { archive.to_string() } else { format!("{archive}/{inner}") },
        parent,
        crumbs,
        entries: children.into_values().collect(),
    })
}

/// Reads a single archive member into memory (viewer, quick view).
pub fn read_member(archive: &str, inner: &str, limit: usize) -> AppResult<Vec<u8>> {
    let idx = index(archive)?;
    let e = idx
        .iter()
        .find(|e| e.path == inner.trim_matches('/'))
        .ok_or_else(|| AppError::new(ErrorCode::NotFound, "Not found in archive"))?;
    let f = fs::File::open(archive).map_err(|e| AppError::io(e, archive))?;
    let mut zip = ZipArchive::new(std::io::BufReader::new(f))?;
    let file = zip.by_index(e.index)?;
    let mut out = Vec::with_capacity((e.size as usize).min(limit));
    file.take(limit as u64).read_to_end(&mut out).map_err(|e| AppError::io(e, archive))?;
    Ok(out)
}

// ---------------------------------------------------------------------------
// Extract
// ---------------------------------------------------------------------------

pub fn extract(ctx: &JobCtx) -> AppResult<()> {
    let req = ctx.req();
    let dest = match &req.dest {
        Some(Location::Local { path }) => crate::local::normalize(path),
        _ => return Err(AppError::invalid("Extraction needs a local destination")),
    };
    ctx.set_status(JobStatus::Scanning);

    // Group selected members by archive (always one in practice).
    let mut plan: Vec<(String, ZEntry, PathBuf)> = Vec::new();
    for src in &req.sources {
        let (archive, inner) = match src {
            Location::Archive { archive, inner } => (archive.clone(), inner.trim_matches('/').to_string()),
            Location::Local { path } => (path.clone(), String::new()),
            _ => return Err(AppError::invalid("Not an archive")),
        };
        let idx = index(&archive)?;
        let base = inner.rsplit_once('/').map(|(a, _)| a.to_string()).unwrap_or_default();
        for e in idx.iter() {
            let selected = inner.is_empty() || e.path == inner || e.path.starts_with(&format!("{inner}/"));
            if !selected {
                continue;
            }
            let rel = if base.is_empty() { e.path.as_str() } else { &e.path[base.len() + 1..] };
            let mut target = dest.clone();
            for part in rel.split('/') {
                if part.is_empty() || part == "." || part == ".." {
                    continue;
                }
                target.push(sanitize_component(part));
            }
            plan.push((archive.clone(), e.clone(), target));
        }
    }
    let total: u64 = plan.iter().filter(|p| !p.1.is_dir).map(|p| p.1.size).sum();
    let files = plan.iter().filter(|p| !p.1.is_dir).count() as u64;
    ctx.set_totals(total, files);
    ctx.set_status(JobStatus::Running);

    let mut open: Option<(String, ZipArchive<std::io::BufReader<fs::File>>)> = None;
    let mut buf = vec![0u8; 1024 * 1024];
    for (archive, e, target) in plan {
        ctx.checkpoint()?;
        ctx.set_current(&e.path);
        if e.is_dir {
            let _ = fs::create_dir_all(&target);
            continue;
        }
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent).map_err(|err| AppError::io(err, parent))?;
        }
        let mut target = target;
        if let Ok(Some(t)) = local_meta(&target, false) {
            let source = FileBrief { path: format!("{archive}/{}", e.path), size: e.size, mtime: e.mtime, is_dir: false };
            let tb = FileBrief { path: path_str(&target), size: t.size, mtime: t.mtime, is_dir: t.is_dir };
            let tgt = target.clone();
            match ctx.resolve_conflict(source, tb, || unique_local_name(&tgt))? {
                ConflictChoice::Skip => {
                    ctx.skip(e.size);
                    continue;
                }
                ConflictChoice::RenameTo(n) => target = target.with_file_name(n),
                ConflictChoice::Overwrite => {}
            }
        }
        if open.as_ref().map(|(a, _)| a != &archive).unwrap_or(true) {
            let f = fs::File::open(&archive).map_err(|err| AppError::io(err, &archive))?;
            open = Some((archive.clone(), ZipArchive::new(std::io::BufReader::new(f))?));
        }
        let zip = &mut open.as_mut().expect("opened above").1;
        loop {
            let res = extract_one(ctx, zip, &e, &target, &mut buf);
            match res {
                Ok(()) => {
                    ctx.file_done();
                    break;
                }
                Err(err) if err.is_cancelled() => {
                    let _ = fs::remove_file(&target);
                    return Err(err);
                }
                Err(err) => {
                    let _ = fs::remove_file(&target);
                    match ctx.handle_error(err)? {
                        ErrorChoice::Retry => continue,
                        ErrorChoice::Skip => {
                            ctx.skip(e.size);
                            break;
                        }
                    }
                }
            }
        }
    }
    Ok(())
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

fn extract_one(
    ctx: &JobCtx,
    zip: &mut ZipArchive<std::io::BufReader<fs::File>>,
    e: &ZEntry,
    target: &Path,
    buf: &mut [u8],
) -> AppResult<()> {
    let mut file = zip.by_index(e.index)?;
    let mut out = fs::File::create(target).map_err(|err| AppError::io(err, target))?;
    let mut written = 0u64;
    let res = (|| -> AppResult<()> {
        loop {
            ctx.checkpoint()?;
            let n = file.read(buf).map_err(|err| AppError::new(ErrorCode::Archive, err.to_string()).with_path(&e.path))?;
            if n == 0 {
                break;
            }
            out.write_all(&buf[..n]).map_err(|err| AppError::io(err, target))?;
            written += n as u64;
            ctx.add_bytes(n as u64);
        }
        Ok(())
    })();
    if res.is_err() {
        ctx.sub_bytes(written);
    }
    res?;
    drop(out);
    if e.mtime != 0 {
        let t = filetime::FileTime::from_system_time(util::ms_to_system_time(e.mtime));
        let _ = filetime::set_file_mtime(target, t);
    }
    #[cfg(unix)]
    if let Some(mode) = e.mode {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(target, fs::Permissions::from_mode(mode & 0o7777));
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Pack
// ---------------------------------------------------------------------------

pub fn pack(ctx: &JobCtx) -> AppResult<()> {
    let req = ctx.req();
    let mut target = match &req.dest {
        Some(Location::Local { path }) => crate::local::normalize(path),
        _ => return Err(AppError::invalid("Choose a local archive name")),
    };
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
    let mut items: Vec<(PathBuf, String, bool, u64)> = Vec::new();
    for src in &sources {
        let base = src.parent().map(Path::to_path_buf).unwrap_or_default();
        let walker = ignore::WalkBuilder::new(src).standard_filters(false).hidden(false).follow_links(false).build();
        for entry in walker {
            ctx.checkpoint()?;
            let Ok(entry) = entry else { continue };
            let p = entry.path().to_path_buf();
            if p == target {
                continue;
            }
            let rel = p.strip_prefix(&base).unwrap_or(&p);
            let name = rel.components().map(|c| c.as_os_str().to_string_lossy()).collect::<Vec<_>>().join("/");
            let is_dir = entry.file_type().map(|t| t.is_dir()).unwrap_or(false);
            let size = if is_dir { 0 } else { entry.metadata().map(|m| m.len()).unwrap_or(0) };
            items.push((p, name, is_dir, size));
        }
    }
    let total: u64 = items.iter().map(|i| i.3).sum();
    ctx.set_totals(total, items.iter().filter(|i| !i.2).count() as u64);
    ctx.set_status(JobStatus::Running);

    let tmp = target.with_extension("zip.part");
    let res = write_zip(ctx, &tmp, &items);
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

fn write_zip(ctx: &JobCtx, path: &Path, items: &[(PathBuf, String, bool, u64)]) -> AppResult<()> {
    let level = ctx.req().options.level.unwrap_or(6).min(9);
    let file = fs::File::create(path).map_err(|e| AppError::io(e, path))?;
    let mut zip = ZipWriter::new(std::io::BufWriter::with_capacity(1 << 20, file));
    let base_opts = if level == 0 {
        SimpleFileOptions::default().compression_method(CompressionMethod::Stored)
    } else {
        SimpleFileOptions::default().compression_method(CompressionMethod::Deflated).compression_level(Some(level as i64))
    };
    let mut buf = vec![0u8; 1024 * 1024];
    for (p, name, is_dir, size) in items {
        ctx.checkpoint()?;
        ctx.set_current(name);
        let meta = fs::symlink_metadata(p).ok();
        let mut opts = base_opts.large_file(*size >= 0xFFFF_FFFF);
        if let Some(m) = &meta {
            if let Ok(t) = m.modified() {
                let (y, mo, d, h, mi, s) = util::local_parts(system_time_ms(t));
                if let Ok(dt) = zip::DateTime::from_date_and_time(y.clamp(1980, 2107) as u16, mo as u8, d as u8, h as u8, mi as u8, s as u8) {
                    opts = opts.last_modified_time(dt);
                }
            }
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                opts = opts.unix_permissions(m.permissions().mode() & 0o7777);
            }
        }
        if *is_dir {
            zip.add_directory(format!("{name}/"), opts)?;
            continue;
        }
        let mut f = loop {
            match fs::File::open(p) {
                Ok(f) => break Some(f),
                Err(e) => match ctx.handle_error(AppError::io(e, p))? {
                    ErrorChoice::Retry => continue,
                    ErrorChoice::Skip => break None,
                },
            }
        };
        let Some(f) = f.as_mut() else {
            ctx.skip(*size);
            continue;
        };
        zip.start_file(name.clone(), opts)?;
        loop {
            ctx.checkpoint()?;
            let n = f.read(&mut buf).map_err(|e| AppError::io(e, p))?;
            if n == 0 {
                break;
            }
            zip.write_all(&buf[..n]).map_err(|e| AppError::io(e, path))?;
            ctx.add_bytes(n as u64);
        }
        ctx.file_done();
    }
    let mut inner = zip.finish()?;
    inner.flush().map_err(|e| AppError::io(e, path))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn browse_zip_with_implicit_dirs() {
        let dir = std::env::temp_dir().join(format!("dualion-zip-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("t.zip");
        {
            let mut z = ZipWriter::new(fs::File::create(&path).unwrap());
            let o = SimpleFileOptions::default();
            z.start_file("a/b/c.txt", o).unwrap();
            z.write_all(b"hello").unwrap();
            z.start_file("root.txt", o).unwrap();
            z.write_all(b"x").unwrap();
            z.finish().unwrap();
        }
        let p = path_str(&path);
        let root = list_archive(&p, "").unwrap();
        let names: Vec<_> = root.entries.iter().map(|e| (e.name.clone(), e.is_dir())).collect();
        assert_eq!(names, vec![("a".to_string(), true), ("root.txt".to_string(), false)]);
        let ab = list_archive(&p, "a/b").unwrap();
        assert_eq!(ab.entries[0].name, "c.txt");
        assert_eq!(ab.entries[0].size, 5);
        assert_eq!(read_member(&p, "a/b/c.txt", 100).unwrap(), b"hello");
        fs::remove_dir_all(&dir).unwrap();
    }
}
