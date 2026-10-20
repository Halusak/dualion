//! Archives: browse, read and extract ZIP, 7z, RAR, ISO, TAR (plain / gz /
//! bz2 / xz / zst) and single compressed files; pack ZIP, 7z and TAR*.
//! Passwords (ZIP, 7z, RAR) are kept in memory per archive for the session.

mod iso;
pub mod pack;

use crate::error::{AppError, AppResult, ErrorCode};
use crate::location::{flags, Crumb, Entry, Listing, Location};
use crate::util::{self, path_str, system_time_ms};
use lru::LruCache;
use parking_lot::Mutex;
use std::collections::{BTreeMap, HashMap};
use std::fs::{self, File};
use std::io::{BufReader, Read, Write};
use std::num::NonZeroUsize;
use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Comp {
    None,
    Gz,
    Bz2,
    Xz,
    Zst,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Zip,
    SevenZ,
    Rar,
    Iso,
    Tar(Comp),
    /// One compressed file (file.txt.gz).
    Single(Comp),
}

/// Archive type of a file name (None = not an archive we open).
pub fn kind_of(path: &str) -> Option<Kind> {
    let n = path.rsplit(['/', '\\']).next().unwrap_or(path).to_lowercase();
    let ext = n.rsplit_once('.').map(|(_, e)| e).unwrap_or("");
    let tar_inner = n.rsplit_once('.').map(|(s, _)| s.ends_with(".tar")).unwrap_or(false);
    Some(match ext {
        "zip" | "jar" | "apk" | "epub" | "nupkg" | "vsix" | "whl" | "xpi" | "ipa" | "aar" | "war" | "cbz" => Kind::Zip,
        "7z" | "cb7" => Kind::SevenZ,
        "rar" | "cbr" => Kind::Rar,
        "iso" => Kind::Iso,
        "tar" | "cbt" => Kind::Tar(Comp::None),
        "tgz" | "taz" => Kind::Tar(Comp::Gz),
        "tbz" | "tbz2" | "tb2" => Kind::Tar(Comp::Bz2),
        "txz" => Kind::Tar(Comp::Xz),
        "tzst" => Kind::Tar(Comp::Zst),
        "gz" if tar_inner => Kind::Tar(Comp::Gz),
        "bz2" if tar_inner => Kind::Tar(Comp::Bz2),
        "xz" if tar_inner => Kind::Tar(Comp::Xz),
        "zst" if tar_inner => Kind::Tar(Comp::Zst),
        "gz" => Kind::Single(Comp::Gz),
        "bz2" => Kind::Single(Comp::Bz2),
        "xz" => Kind::Single(Comp::Xz),
        "zst" => Kind::Single(Comp::Zst),
        _ => return None,
    })
}

#[derive(Debug, Clone)]
pub struct AEntry {
    /// Normalized path without leading/trailing slashes.
    pub path: String,
    pub is_dir: bool,
    pub size: u64,
    pub mtime: i64,
    pub mode: Option<u32>,
    pub encrypted: bool,
    /// Format specific position (ZIP index, ISO extent).
    pub index: usize,
    pub extent: u64,
}

// ---------------------------------------------------------------- passwords

static PASSWORDS: LazyLock<Mutex<HashMap<String, String>>> = LazyLock::new(|| Mutex::new(HashMap::new()));

pub fn password(archive: &str) -> Option<String> {
    PASSWORDS.lock().get(archive).cloned()
}

fn need_password(archive: &str) -> AppError {
    if password(archive).is_some() {
        AppError::new(ErrorCode::WrongPassword, "Wrong password").with_path(archive)
    } else {
        AppError::new(ErrorCode::PasswordRequired, "This archive is protected by a password").with_path(archive)
    }
}

/// Checks a password against the archive and remembers it for the session.
pub fn set_password(archive: &str, pw: &str) -> AppResult<()> {
    let old = PASSWORDS.lock().insert(archive.to_string(), pw.to_string());
    INDEX.lock().clear();
    let res = verify(archive);
    if res.is_err() {
        match old {
            Some(o) => PASSWORDS.lock().insert(archive.to_string(), o),
            None => PASSWORDS.lock().remove(archive),
        };
    }
    res
}

/// Whether reading the content needs a password we do not have (or have wrong).
pub fn check(archive: &str) -> AppResult<()> {
    match index(archive) {
        Err(e) => Err(e),
        Ok(idx) => {
            if idx.iter().any(|e| e.encrypted) || matches!(kind_of(archive), Some(Kind::SevenZ)) {
                verify(archive)
            } else {
                Ok(())
            }
        }
    }
}

/// Reads the first bytes of the first encrypted member with the stored password.
fn verify(archive: &str) -> AppResult<()> {
    let idx = index(archive)?;
    let Some(first) = idx.iter().filter(|e| !e.is_dir && e.size > 0).min_by_key(|e| (!e.encrypted, e.size)) else { return Ok(()) };
    if matches!(kind_of(archive), Some(Kind::Zip)) && !first.encrypted {
        return Ok(());
    }
    read_member(archive, &first.path, 16).map(|_| ())
}

// ---------------------------------------------------------------- index

type IndexKey = (String, i64, u64);
static INDEX: LazyLock<Mutex<LruCache<IndexKey, Arc<Vec<AEntry>>>>> =
    LazyLock::new(|| Mutex::new(LruCache::new(NonZeroUsize::new(8).unwrap())));

fn clean(raw: &str) -> Option<String> {
    let p = raw.replace('\\', "/");
    let p = p.trim_matches('/').trim_start_matches("./").to_string();
    (!p.is_empty() && !p.split('/').any(|c| c == "..")).then_some(p)
}

pub fn index(archive: &str) -> AppResult<Arc<Vec<AEntry>>> {
    let meta = fs::metadata(archive).map_err(|e| AppError::io(e, archive))?;
    let key = (archive.to_string(), meta.modified().map(system_time_ms).unwrap_or(0), meta.len());
    if let Some(hit) = INDEX.lock().get(&key) {
        return Ok(hit.clone());
    }
    let kind = kind_of(archive).ok_or_else(|| AppError::unsupported("Not an archive"))?;
    let entries = match kind {
        Kind::Zip => zip_index(archive)?,
        Kind::SevenZ => sevenz_index(archive)?,
        Kind::Rar => rar_index(archive)?,
        Kind::Iso => iso::index(Path::new(archive))?,
        Kind::Tar(c) => tar_index(archive, c)?,
        Kind::Single(c) => single_index(archive, c, meta.len()),
    };
    let arc = Arc::new(entries);
    INDEX.lock().put(key, arc.clone());
    Ok(arc)
}

fn archive_err(e: impl std::fmt::Display, archive: &str) -> AppError {
    AppError::new(ErrorCode::Archive, e.to_string()).with_path(archive)
}

fn zip_open(archive: &str) -> AppResult<zip::ZipArchive<BufReader<File>>> {
    let f = File::open(archive).map_err(|e| AppError::io(e, archive))?;
    Ok(zip::ZipArchive::new(BufReader::new(f))?)
}

fn zip_time_ms(dt: Option<zip::DateTime>) -> i64 {
    dt.map(|d| util::local_naive_to_ms(d.year() as i32, d.month() as u32, d.day() as u32, d.hour() as u32, d.minute() as u32, d.second() as u32))
        .unwrap_or(0)
}

fn zip_index(archive: &str) -> AppResult<Vec<AEntry>> {
    let mut zip = zip_open(archive)?;
    let mut out = Vec::with_capacity(zip.len());
    for i in 0..zip.len() {
        let file = zip.by_index_raw(i)?;
        let Some(path) = clean(&file.name()?) else { continue };
        out.push(AEntry {
            index: i,
            is_dir: file.is_dir(),
            size: file.size(),
            mtime: zip_time_ms(file.last_modified()),
            mode: file.unix_mode(),
            encrypted: file.encrypted(),
            extent: 0,
            path,
        });
    }
    Ok(out)
}

fn sz_password(archive: &str) -> sevenz_rust2::Password {
    password(archive).map(|p| sevenz_rust2::Password::new(&p)).unwrap_or_default()
}

fn sz_err(e: sevenz_rust2::Error, archive: &str) -> AppError {
    match e {
        sevenz_rust2::Error::PasswordRequired | sevenz_rust2::Error::MaybeBadPassword(_) => need_password(archive),
        sevenz_rust2::Error::Io(io, _) => AppError::io(io, archive),
        other => archive_err(other, archive),
    }
}

fn sevenz_index(archive: &str) -> AppResult<Vec<AEntry>> {
    let a = sevenz_rust2::Archive::open_with_password(archive, &sz_password(archive)).map_err(|e| sz_err(e, archive))?;
    let encrypted = a.blocks.iter().any(|b| b.coders.iter().any(|c| c.encoder_method_id() == sevenz_rust2::EncoderMethod::ID_AES256_SHA256));
    Ok(a.files
        .iter()
        .enumerate()
        .filter(|(_, f)| !f.is_anti_item())
        .filter_map(|(i, f)| {
            Some(AEntry {
                path: clean(f.name())?,
                is_dir: f.is_directory(),
                size: f.size,
                mtime: if f.has_last_modified_date { system_time_ms(f.last_modified_date.into()) } else { 0 },
                mode: None,
                encrypted: encrypted && f.has_stream,
                index: i,
                extent: 0,
            })
        })
        .collect())
}

fn dos_time_ms(t: u32) -> i64 {
    let (date, time) = ((t >> 16) as u16, t as u16);
    let y = 1980 + (date >> 9) as i32;
    let mo = ((date >> 5) & 0xF) as u32;
    let d = (date & 0x1F) as u32;
    if mo == 0 || d == 0 {
        return 0;
    }
    util::local_naive_to_ms(y, mo, d, (time >> 11) as u32, ((time >> 5) & 0x3F) as u32, ((time & 0x1F) * 2) as u32)
}

fn rar_err(e: unrar::error::UnrarError, archive: &str) -> AppError {
    use unrar::error::Code;
    match e.code {
        Code::MissingPassword | Code::BadPassword => need_password(archive),
        _ => archive_err(e, archive),
    }
}

fn rar_archive(archive: &str) -> unrar::Archive<'static> {
    // unrar borrows the path / password; leak small strings for the call duration-free API.
    let path: &'static str = Box::leak(archive.to_string().into_boxed_str());
    match password(archive) {
        Some(p) => {
            let pw: &'static str = Box::leak(p.into_boxed_str());
            unrar::Archive::with_password(path, pw)
        }
        None => unrar::Archive::new(path),
    }
}

fn rar_entry(h: &unrar::FileHeader, i: usize) -> Option<AEntry> {
    Some(AEntry {
        path: clean(&h.filename.to_string_lossy())?,
        is_dir: h.is_directory(),
        size: h.unpacked_size,
        mtime: dos_time_ms(h.file_time),
        mode: None,
        encrypted: h.is_encrypted(),
        index: i,
        extent: 0,
    })
}

fn rar_index(archive: &str) -> AppResult<Vec<AEntry>> {
    let mut a = rar_archive(archive).open_for_listing().map_err(|e| rar_err(e, archive))?;
    let mut out = Vec::new();
    let mut i = 0;
    while let Some(h) = a.read_header().map_err(|e| rar_err(e, archive))? {
        if let Some(e) = rar_entry(h.entry(), i) {
            if !h.entry().is_split_before() {
                out.push(e);
            }
        }
        i += 1;
        a = h.skip().map_err(|e| rar_err(e, archive))?;
    }
    Ok(out)
}

fn decompressor<'a>(r: impl Read + 'a, c: Comp) -> AppResult<Box<dyn Read + 'a>> {
    Ok(match c {
        Comp::None => Box::new(r),
        Comp::Gz => Box::new(flate2::read::MultiGzDecoder::new(r)),
        Comp::Bz2 => Box::new(bzip2::read::MultiBzDecoder::new(r)),
        Comp::Xz => Box::new(lzma_rust2::XzReader::new(r, true)),
        Comp::Zst => Box::new(zstd::stream::read::Decoder::new(r).map_err(|e| AppError::new(ErrorCode::Archive, e.to_string()))?),
    })
}

fn tar_open(archive: &str, c: Comp) -> AppResult<tar::Archive<Box<dyn Read>>> {
    let f = File::open(archive).map_err(|e| AppError::io(e, archive))?;
    Ok(tar::Archive::new(decompressor(BufReader::with_capacity(1 << 20, f), c)?))
}

fn tar_entry<R: Read>(e: &tar::Entry<'_, R>, i: usize) -> Option<AEntry> {
    let h = e.header();
    let path = clean(&e.path().ok()?.to_string_lossy())?;
    let kind = h.entry_type();
    if !(kind.is_file() || kind.is_dir() || kind.is_symlink() || kind.is_hard_link()) {
        return None;
    }
    Some(AEntry {
        path,
        is_dir: kind.is_dir(),
        size: h.size().unwrap_or(0),
        mtime: h.mtime().map(|t| t as i64 * 1000).unwrap_or(0),
        mode: h.mode().ok(),
        encrypted: false,
        index: i,
        extent: 0,
    })
}

fn tar_index(archive: &str, c: Comp) -> AppResult<Vec<AEntry>> {
    let mut a = tar_open(archive, c)?;
    let mut out = Vec::new();
    for (i, e) in a.entries().map_err(|e| AppError::io(e, archive))?.enumerate() {
        let e = e.map_err(|e| archive_err(e, archive))?;
        if let Some(x) = tar_entry(&e, i) {
            out.push(x);
        }
    }
    Ok(out)
}

fn single_name(archive: &str) -> String {
    let n = Path::new(archive).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    n.rsplit_once('.').map(|(s, _)| s.to_string()).filter(|s| !s.is_empty()).unwrap_or(n)
}

fn single_index(archive: &str, c: Comp, len: u64) -> Vec<AEntry> {
    // gzip stores the (mod 2^32) original size in its last four bytes.
    let size = if c == Comp::Gz && len >= 4 {
        crate::viewer::read_bytes(Path::new(archive), len - 4, 4).ok().filter(|b| b.len() == 4).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]) as u64).unwrap_or(0)
    } else {
        0
    };
    let mtime = fs::metadata(archive).and_then(|m| m.modified()).map(system_time_ms).unwrap_or(0);
    vec![AEntry { path: single_name(archive), is_dir: false, size, mtime, mode: None, encrypted: false, index: 0, extent: 0 }]
}

// ---------------------------------------------------------------- listing

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
                children
                    .entry(dir.to_string())
                    .or_insert_with(|| Entry { name: dir.to_string(), flags: flags::DIR, size: 0, mtime: 0, attrs: 0 });
            }
            None => {
                let mut fl = if e.is_dir { flags::DIR } else { 0 };
                if rest.starts_with('.') {
                    fl |= flags::HIDDEN;
                }
                if e.encrypted {
                    fl |= flags::READONLY;
                }
                children.insert(rest.to_string(), Entry { name: rest.to_string(), flags: fl, size: e.size, mtime: e.mtime, attrs: e.mode.unwrap_or(0) });
            }
        }
    }
    let archive_name = Path::new(archive).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let mut crumbs = crate::local::local_crumbs(Path::new(archive).parent().unwrap_or(Path::new("/")));
    crumbs.push(Crumb { label: archive_name, icon: "archive", location: Location::Archive { archive: archive.to_string(), inner: String::new() } });
    let mut acc = String::new();
    for part in inner.split('/').filter(|s| !s.is_empty()) {
        if !acc.is_empty() {
            acc.push('/');
        }
        acc.push_str(part);
        crumbs.push(Crumb { label: part.to_string(), icon: "folder", location: Location::Archive { archive: archive.to_string(), inner: acc.clone() } });
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

// ---------------------------------------------------------------- reading

/// Visits members in archive order: `want` picks them, `f` gets a reader for
/// each picked file. RAR members are handed over through a temporary file.
fn walk(archive: &str, want: &mut dyn FnMut(&AEntry) -> bool, f: &mut dyn FnMut(&AEntry, &mut dyn Read) -> AppResult<()>) -> AppResult<()> {
    let kind = kind_of(archive).ok_or_else(|| AppError::unsupported("Not an archive"))?;
    match kind {
        Kind::Zip => {
            let idx = index(archive)?;
            let mut zip = zip_open(archive)?;
            let pw = password(archive);
            for e in idx.iter().filter(|e| !e.is_dir) {
                if !want(e) {
                    continue;
                }
                let file = match (&pw, e.encrypted) {
                    (Some(p), true) => zip.by_index_decrypt(e.index, p.as_bytes()),
                    _ => zip.by_index(e.index),
                };
                let mut file = file.map_err(|err| match err {
                    zip::result::ZipError::UnsupportedArchive(m) if m == zip::result::ZipError::PASSWORD_REQUIRED => need_password(archive),
                    zip::result::ZipError::InvalidPassword => need_password(archive),
                    other => other.into(),
                })?;
                f(e, &mut file)?;
            }
            Ok(())
        }
        Kind::SevenZ => {
            let mut r = sevenz_rust2::ArchiveReader::open(archive, sz_password(archive)).map_err(|e| sz_err(e, archive))?;
            let mut failure: Option<AppError> = None;
            let res = r.for_each_entries(|entry, reader| {
                if entry.is_directory() || entry.is_anti_item() || !entry.has_stream {
                    return Ok(true);
                }
                let Some(path) = clean(entry.name()) else { return Ok(true) };
                let e = AEntry {
                    path,
                    is_dir: false,
                    size: entry.size,
                    mtime: if entry.has_last_modified_date { system_time_ms(entry.last_modified_date.into()) } else { 0 },
                    mode: None,
                    encrypted: false,
                    index: 0,
                    extent: 0,
                };
                if !want(&e) {
                    // Solid blocks must be read through anyway.
                    std::io::copy(reader, &mut std::io::sink()).map_err(|e| sevenz_rust2::Error::Io(e, "".into()))?;
                    return Ok(true);
                }
                match f(&e, reader) {
                    Ok(()) => Ok(true),
                    Err(err) => {
                        failure = Some(err);
                        Err(sevenz_rust2::Error::Other("stopped".into()))
                    }
                }
            });
            if let Some(e) = failure {
                return Err(e);
            }
            res.map_err(|e| sz_err(e, archive))
        }
        Kind::Rar => {
            let tmp = crate::platform::temp_dir().join(format!("rar-{:x}", xxhash_rust::xxh3::xxh3_64(archive.as_bytes())));
            fs::create_dir_all(&tmp).map_err(|e| AppError::io(e, &tmp))?;
            let mut a = rar_archive(archive).open_for_processing().map_err(|e| rar_err(e, archive))?;
            let mut i = 0;
            while let Some(h) = a.read_header().map_err(|e| rar_err(e, archive))? {
                let picked = rar_entry(h.entry(), i).filter(|e| !e.is_dir && want(e));
                i += 1;
                match picked {
                    Some(e) => {
                        let part = tmp.join(format!("{i}.part"));
                        a = h.extract_to(&part).map_err(|err| rar_err(err, archive))?;
                        let res = File::open(&part).map_err(|err| AppError::io(err, &part)).and_then(|mut file| f(&e, &mut file));
                        let _ = fs::remove_file(&part);
                        res?;
                    }
                    None => a = h.skip().map_err(|e| rar_err(e, archive))?,
                }
            }
            Ok(())
        }
        Kind::Iso => {
            let idx = index(archive)?;
            let mut file = File::open(archive).map_err(|e| AppError::io(e, archive))?;
            for e in idx.iter().filter(|e| !e.is_dir) {
                if want(e) {
                    let mut r = iso::member(&mut file, e).map_err(|err| AppError::io(err, archive))?;
                    f(e, &mut r)?;
                }
            }
            Ok(())
        }
        Kind::Tar(c) => {
            let mut a = tar_open(archive, c)?;
            for (i, entry) in a.entries().map_err(|e| AppError::io(e, archive))?.enumerate() {
                let mut entry = entry.map_err(|e| archive_err(e, archive))?;
                let Some(e) = tar_entry(&entry, i) else { continue };
                if e.is_dir || !entry.header().entry_type().is_file() && !entry.header().entry_type().is_hard_link() {
                    continue;
                }
                if want(&e) {
                    f(&e, &mut entry)?;
                }
            }
            Ok(())
        }
        Kind::Single(c) => {
            let idx = index(archive)?;
            let e = &idx[0];
            if want(e) {
                let file = File::open(archive).map_err(|err| AppError::io(err, archive))?;
                let mut r = decompressor(BufReader::new(file), c)?;
                f(e, &mut r)?;
            }
            Ok(())
        }
    }
}

/// Reads a single archive member into memory (viewer, quick view, open).
pub fn read_member(archive: &str, inner: &str, limit: usize) -> AppResult<Vec<u8>> {
    let inner = inner.trim_matches('/').to_string();
    let mut out: Option<Vec<u8>> = None;
    let found = std::cell::Cell::new(false);
    walk(
        archive,
        &mut |e| !found.get() && e.path == inner,
        &mut |e, r| {
            found.set(true);
            let mut buf = Vec::with_capacity((e.size as usize).min(limit).min(64 << 20));
            r.take(limit as u64).read_to_end(&mut buf).map_err(|err| archive_io(err, archive))?;
            out = Some(buf);
            Ok(())
        },
    )?;
    out.ok_or_else(|| AppError::new(ErrorCode::NotFound, "Not found in archive"))
}

fn archive_io(e: std::io::Error, archive: &str) -> AppError {
    // Decryption failures surface as data errors while reading.
    if password(archive).is_some() && matches!(e.kind(), std::io::ErrorKind::InvalidData | std::io::ErrorKind::InvalidInput) {
        return need_password(archive);
    }
    AppError::io(e, archive)
}

/// Hooks used while extracting (the job supplies conflict handling and progress).
pub struct Sink<'a> {
    /// Target path for a member, or None to skip it.
    pub decide: &'a mut dyn FnMut(&AEntry) -> AppResult<Option<PathBuf>>,
    /// Bytes written (returns an error to cancel).
    pub progress: &'a dyn Fn(u64) -> AppResult<()>,
    /// Result per file; returning an error aborts the extraction.
    pub done: &'a mut FileDone<'a>,
}

/// Callback with the result of one extracted file.
pub type FileDone<'a> = dyn FnMut(&AEntry, &Path, AppResult<()>) -> AppResult<()> + 'a;

/// Extracts the members chosen by `selected` (folders are created as needed).
pub fn extract(archive: &str, selected: &dyn Fn(&AEntry) -> bool, sink: Sink<'_>) -> AppResult<()> {
    let Sink { decide, progress, done } = sink;
    let targets: std::cell::RefCell<HashMap<String, PathBuf>> = Default::default();
    // Empty folders.
    for e in index(archive)?.iter().filter(|e| e.is_dir && selected(e)) {
        if let Some(t) = decide(e)? {
            let _ = fs::create_dir_all(&t);
        }
    }
    let mut buf = vec![0u8; 1 << 20];
    walk(
        archive,
        &mut |e| {
            if !selected(e) {
                return false;
            }
            match decide(e) {
                Ok(Some(t)) => {
                    targets.borrow_mut().insert(e.path.clone(), t);
                    true
                }
                _ => false,
            }
        },
        &mut |e, r| {
            let Some(target) = targets.borrow_mut().remove(&e.path) else { return Ok(()) };
            let res = write_member(r, &target, e, &mut buf, progress, archive);
            if res.is_err() {
                let _ = fs::remove_file(&target);
            }
            done(e, &target, res)
        },
    )
}

fn write_member(r: &mut dyn Read, target: &Path, e: &AEntry, buf: &mut [u8], progress: &dyn Fn(u64) -> AppResult<()>, archive: &str) -> AppResult<()> {
    if let Some(parent) = target.parent() {
        fs::create_dir_all(parent).map_err(|err| AppError::io(err, parent))?;
    }
    let mut out = File::create(target).map_err(|err| AppError::io(err, target))?;
    loop {
        let n = r.read(buf).map_err(|err| archive_io(err, archive))?;
        if n == 0 {
            break;
        }
        out.write_all(&buf[..n]).map_err(|err| AppError::io(err, target))?;
        progress(n as u64)?;
    }
    drop(out);
    if e.mtime != 0 {
        let _ = filetime::set_file_mtime(target, filetime::FileTime::from_system_time(util::ms_to_system_time(e.mtime)));
    }
    #[cfg(unix)]
    if let Some(mode) = e.mode {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(target, fs::Permissions::from_mode(mode & 0o7777));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp() -> PathBuf {
        let d = std::env::temp_dir().join(format!("dualion-arch-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&d).unwrap();
        d
    }

    fn sample(dir: &Path) -> Vec<pack::Item> {
        let src = dir.join("src");
        fs::create_dir_all(src.join("sub/empty")).unwrap();
        fs::write(src.join("a.txt"), "hello archive").unwrap();
        fs::write(src.join("sub/b.bin"), vec![7u8; 100_000]).unwrap();
        pack::collect(&[src], Path::new("/nonexistent")).unwrap()
    }

    fn roundtrip(name: &str, fmt: pack::Format, pw: Option<&str>) {
        let dir = tmp();
        let items = sample(&dir);
        let out = dir.join(name);
        let never = |_: u64| Ok(());
        pack::pack(&out, fmt, &items, 6, pw, false, &never, &mut |_| Ok(())).unwrap();
        let a = out.to_string_lossy().into_owned();
        if let Some(p) = pw {
            assert!(matches!(check(&a), Err(e) if e.code == ErrorCode::PasswordRequired), "{name}: password must be required");
            assert!(set_password(&a, "wrong").is_err(), "{name}: wrong password accepted");
            set_password(&a, p).unwrap();
        }
        let idx = index(&a).unwrap();
        assert!(idx.iter().any(|e| e.path == "src/sub/b.bin" && e.size == 100_000), "{name}: {idx:?}");
        assert_eq!(read_member(&a, "src/a.txt", 1000).unwrap(), b"hello archive");
        let l = list_archive(&a, "src").unwrap();
        assert!(l.entries.iter().any(|e| e.name == "sub" && e.flags & flags::DIR != 0));
        let dest = dir.join("out");
        let mut n = 0;
        extract(
            &a,
            &|e| e.path.starts_with("src/sub"),
            Sink {
                decide: &mut |e| Ok(Some(dest.join(&e.path))),
                progress: &|_| Ok(()),
                done: &mut |_, _, r| {
                    n += 1;
                    r
                },
            },
        )
        .unwrap();
        assert_eq!(fs::read(dest.join("src/sub/b.bin")).unwrap().len(), 100_000);
        assert!(!dest.join("src/a.txt").exists());
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn zip_roundtrip() {
        roundtrip("t.zip", pack::Format::Zip, None);
    }
    #[test]
    fn zip_aes_roundtrip() {
        roundtrip("t-aes.zip", pack::Format::Zip, Some("tajne heslo"));
    }
    #[test]
    fn sevenz_roundtrip() {
        roundtrip("t.7z", pack::Format::SevenZ, None);
    }
    #[test]
    fn sevenz_aes_roundtrip() {
        roundtrip("t-aes.7z", pack::Format::SevenZ, Some("heslo"));
    }
    #[test]
    fn tar_roundtrips() {
        roundtrip("t.tar", pack::Format::Tar(Comp::None), None);
        roundtrip("t.tar.gz", pack::Format::Tar(Comp::Gz), None);
        roundtrip("t.tar.bz2", pack::Format::Tar(Comp::Bz2), None);
        roundtrip("t.tar.xz", pack::Format::Tar(Comp::Xz), None);
        roundtrip("t.tar.zst", pack::Format::Tar(Comp::Zst), None);
    }

    #[test]
    fn single_gz() {
        let dir = tmp();
        let p = dir.join("notes.txt.gz");
        let mut w = flate2::write::GzEncoder::new(File::create(&p).unwrap(), flate2::Compression::default());
        w.write_all(b"plain text").unwrap();
        w.finish().unwrap();
        let a = p.to_string_lossy().into_owned();
        let idx = index(&a).unwrap();
        assert_eq!((idx[0].path.as_str(), idx[0].size), ("notes.txt", 10));
        assert_eq!(read_member(&a, "notes.txt", 100).unwrap(), b"plain text");
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn kinds() {
        assert_eq!(kind_of("/x/a.tar.gz"), Some(Kind::Tar(Comp::Gz)));
        assert_eq!(kind_of("a.TGZ"), Some(Kind::Tar(Comp::Gz)));
        assert_eq!(kind_of("a.log.gz"), Some(Kind::Single(Comp::Gz)));
        assert_eq!(kind_of("a.part1.rar"), Some(Kind::Rar));
        assert_eq!(kind_of("a.txt"), None);
    }
}

#[cfg(test)]
mod rar_tests {
    use super::*;

    /// Needs the unrar crate's test data: DUALION_RAR_DATA=<…/unrar-0.5.8/data> cargo test rar -- --ignored
    #[test]
    #[ignore]
    fn rar_list_read_and_password() {
        let data_dir = std::env::var("DUALION_RAR_DATA").unwrap();
        let data_dir = data_dir.as_str();
        let solid = format!("{data_dir}/solid.rar");
        let idx = index(&solid).unwrap();
        assert!(!idx.is_empty());
        let first = idx.iter().find(|e| !e.is_dir).unwrap().clone();
        let data = read_member(&solid, &first.path, 1 << 20).unwrap();
        assert_eq!(data.len() as u64, first.size.min(1 << 20));
        let enc = format!("{data_dir}/crypted.rar");
        if Path::new(&enc).exists() {
            assert!(check(&enc).is_err());
            assert!(set_password(&enc, "unrar").is_ok() || set_password(&enc, "password").is_ok());
        }
    }
}
