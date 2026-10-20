//! Minimal file system abstraction shared by the job engine so that the
//! same copy/move/delete code works for local disks and FTP servers.

use crate::error::{AppError, AppResult, ErrorCode};
use crate::ftp::{self, FtpConfig, FtpConn, FtpRegistry};
use crate::jobs::{JobOp, JobRequest};
use crate::local;
use crate::location::Location;
use crate::util::{path_str, system_time_ms};
use parking_lot::Mutex;
use std::collections::HashMap;
use std::fs;
use std::io::{Read, Write};
use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Meta {
    pub is_dir: bool,
    pub is_link: bool,
    pub size: u64,
    pub mtime: i64,
}

pub trait ReadStream: Read + Send {
    fn finish(self: Box<Self>) -> AppResult<()>;
}

pub trait WriteStream: Write + Send {
    fn finish(self: Box<Self>) -> AppResult<()>;
}

pub trait Vfs: Send + Sync {
    fn is_local(&self) -> bool;
    /// Identifies the file system; equal ids allow renames instead of copies.
    fn id(&self) -> String;
    fn join(&self, dir: &str, name: &str) -> String;
    fn name(&self, path: &str) -> String;
    fn parent(&self, path: &str) -> Option<String>;
    /// `Ok(None)` when the path does not exist. Never follows a final symlink
    /// unless `follow` is set; for links `is_dir` reports the target type.
    fn stat(&self, path: &str, follow: bool) -> AppResult<Option<Meta>>;
    fn read_dir(&self, path: &str) -> AppResult<Vec<(String, Meta)>>;
    fn mkdir(&self, path: &str) -> AppResult<()>;
    fn remove_file(&self, path: &str) -> AppResult<()>;
    fn remove_dir(&self, path: &str) -> AppResult<()>;
    fn rename(&self, from: &str, to: &str) -> AppResult<()>;
    fn reader(&self, path: &str) -> AppResult<Box<dyn ReadStream + '_>>;
    fn writer(&self, path: &str) -> AppResult<Box<dyn WriteStream + '_>>;
    fn set_mtime(&self, path: &str, mtime: i64);
}

// ---------------------------------------------------------------------------
// Local
// ---------------------------------------------------------------------------

pub struct LocalFs;

struct LocalReader(fs::File);
impl Read for LocalReader {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        self.0.read(buf)
    }
}
impl ReadStream for LocalReader {
    fn finish(self: Box<Self>) -> AppResult<()> {
        Ok(())
    }
}

struct LocalWriter(fs::File, String);
impl Write for LocalWriter {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.write(buf)
    }
    fn flush(&mut self) -> std::io::Result<()> {
        self.0.flush()
    }
}
impl WriteStream for LocalWriter {
    fn finish(mut self: Box<Self>) -> AppResult<()> {
        self.0.flush().map_err(|e| AppError::io(e, &self.1))
    }
}

pub fn local_meta(path: &Path, follow: bool) -> std::io::Result<Option<Meta>> {
    let m = match if follow { fs::metadata(path) } else { fs::symlink_metadata(path) } {
        Ok(m) => m,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e),
    };
    let is_link = m.file_type().is_symlink();
    let is_dir = if is_link { fs::metadata(path).map(|t| t.is_dir()).unwrap_or(false) } else { m.is_dir() };
    Ok(Some(Meta { is_dir, is_link, size: if m.is_dir() { 0 } else { m.len() }, mtime: m.modified().map(system_time_ms).unwrap_or(0) }))
}

impl Vfs for LocalFs {
    fn is_local(&self) -> bool {
        true
    }
    fn id(&self) -> String {
        "local".into()
    }
    fn join(&self, dir: &str, name: &str) -> String {
        path_str(&local::normalize(dir).join(name))
    }
    fn name(&self, path: &str) -> String {
        Path::new(path).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| path.to_string())
    }
    fn parent(&self, path: &str) -> Option<String> {
        Path::new(path).parent().map(path_str)
    }
    fn stat(&self, path: &str, follow: bool) -> AppResult<Option<Meta>> {
        local_meta(Path::new(path), follow).map_err(|e| AppError::io(e, path))
    }
    fn read_dir(&self, path: &str) -> AppResult<Vec<(String, Meta)>> {
        let mut out = Vec::new();
        for e in fs::read_dir(path).map_err(|e| AppError::io(e, path))? {
            let e = e.map_err(|e| AppError::io(e, path))?;
            let p = e.path();
            if let Some(m) = local_meta(&p, false).map_err(|e| AppError::io(e, &p))? {
                out.push((e.file_name().to_string_lossy().into_owned(), m));
            }
        }
        Ok(out)
    }
    fn mkdir(&self, path: &str) -> AppResult<()> {
        fs::create_dir(path).map_err(|e| AppError::io(e, path))
    }
    fn remove_file(&self, path: &str) -> AppResult<()> {
        match fs::remove_file(path) {
            Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied && cfg!(windows) => {
                // Read-only files cannot be deleted on Windows; clear the flag and retry.
                if let Ok(m) = fs::metadata(path) {
                    let mut perms = m.permissions();
                    #[allow(clippy::permissions_set_readonly_false)]
                    perms.set_readonly(false);
                    let _ = fs::set_permissions(path, perms);
                }
                fs::remove_file(path).map_err(|e| AppError::io(e, path))
            }
            r => r.map_err(|e| AppError::io(e, path)),
        }
    }
    fn remove_dir(&self, path: &str) -> AppResult<()> {
        fs::remove_dir(path).map_err(|e| AppError::io(e, path))
    }
    fn rename(&self, from: &str, to: &str) -> AppResult<()> {
        fs::rename(from, to).map_err(|e| AppError::io(e, from))
    }
    fn reader(&self, path: &str) -> AppResult<Box<dyn ReadStream + '_>> {
        Ok(Box::new(LocalReader(fs::File::open(path).map_err(|e| AppError::io(e, path))?)))
    }
    fn writer(&self, path: &str) -> AppResult<Box<dyn WriteStream + '_>> {
        let f = fs::File::create(path).map_err(|e| AppError::io(e, path))?;
        Ok(Box::new(LocalWriter(f, path.to_string())))
    }
    fn set_mtime(&self, path: &str, mtime: i64) {
        if mtime != 0 {
            let t = filetime::FileTime::from_system_time(crate::util::ms_to_system_time(mtime));
            let _ = filetime::set_file_mtime(path, t);
        }
    }
}

// ---------------------------------------------------------------------------
// FTP (one dedicated connection per job, independent from the browsing one)
// ---------------------------------------------------------------------------

/// suppaftp's `TransferStream` must be finished explicitly to read the
/// server's completion reply; this trait lets us call it on the generic type.
pub trait FinishTransfer {
    fn finish_transfer(self) -> suppaftp::FtpResult<()>;
}

impl<T: suppaftp::TlsStream> FinishTransfer for suppaftp::TransferStream<T> {
    fn finish_transfer(self) -> suppaftp::FtpResult<()> {
        self.finish()
    }
}

struct FtpStreamWrap<S>(S);

impl<S: Read> Read for FtpStreamWrap<S> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        self.0.read(buf)
    }
}
impl<S: Write> Write for FtpStreamWrap<S> {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.write(buf)
    }
    fn flush(&mut self) -> std::io::Result<()> {
        self.0.flush()
    }
}
impl<S: Read + Send + FinishTransfer> ReadStream for FtpStreamWrap<S> {
    fn finish(self: Box<Self>) -> AppResult<()> {
        self.0.finish_transfer().map_err(AppError::from)
    }
}
impl<S: Write + Send + FinishTransfer> WriteStream for FtpStreamWrap<S> {
    fn finish(self: Box<Self>) -> AppResult<()> {
        self.0.finish_transfer().map_err(AppError::from)
    }
}

pub struct FtpFs {
    cfg: FtpConfig,
    session: String,
    conn: Mutex<Option<FtpConn>>,
    mlsd: Mutex<Option<bool>>,
    cache: Mutex<HashMap<String, Vec<(String, Meta)>>>,
}

impl FtpFs {
    pub fn connect(cfg: FtpConfig, session: String) -> AppResult<Self> {
        let conn = ftp::connect(&cfg)?;
        Ok(Self { cfg, session, conn: Mutex::new(Some(conn)), mlsd: Mutex::new(None), cache: Mutex::new(HashMap::new()) })
    }

    fn run<T>(&self, mut f: impl FnMut(&mut FtpConn, bool) -> AppResult<T>) -> AppResult<T> {
        let mut guard = self.conn.lock();
        for attempt in 0..2 {
            if guard.is_none() {
                *guard = Some(ftp::connect(&self.cfg)?);
            }
            let c = guard.as_mut().expect("connected");
            let mlsd = *self.mlsd.lock().get_or_insert_with(|| ftp::supports_mlsd(c));
            match f(c, mlsd) {
                Err(e) if attempt == 0 && e.code == ErrorCode::FtpConnect => *guard = None,
                other => return other,
            }
        }
        Err(AppError::new(ErrorCode::FtpConnect, "Connection lost"))
    }

    fn forget(&self, path: &str) {
        if let Some(parent) = ftp::parent(path) {
            self.cache.lock().remove(&parent);
        }
    }
}

impl Drop for FtpFs {
    fn drop(&mut self) {
        if let Some(mut c) = self.conn.lock().take() {
            let _ = c.quit();
        }
    }
}

impl Vfs for FtpFs {
    fn is_local(&self) -> bool {
        false
    }
    fn id(&self) -> String {
        format!("ftp:{}", self.session)
    }
    fn join(&self, dir: &str, name: &str) -> String {
        ftp::join(dir, name)
    }
    fn name(&self, path: &str) -> String {
        ftp::file_name(path).to_string()
    }
    fn parent(&self, path: &str) -> Option<String> {
        ftp::parent(path)
    }
    fn stat(&self, path: &str, _follow: bool) -> AppResult<Option<Meta>> {
        let Some(parent) = ftp::parent(path) else {
            return Ok(Some(Meta { is_dir: true, is_link: false, size: 0, mtime: 0 }));
        };
        let name = ftp::file_name(path).to_string();
        let listing = match self.read_dir(&parent) {
            Ok(l) => l,
            Err(e) if e.code == ErrorCode::NotFound => return Ok(None),
            Err(e) => return Err(e),
        };
        Ok(listing.into_iter().find(|(n, _)| *n == name).map(|(_, m)| m))
    }
    fn read_dir(&self, path: &str) -> AppResult<Vec<(String, Meta)>> {
        if let Some(hit) = self.cache.lock().get(path) {
            return Ok(hit.clone());
        }
        let entries = self.run(|c, mlsd| ftp::list_dir(c, path, mlsd))?;
        let list: Vec<(String, Meta)> = entries
            .into_iter()
            .map(|e| {
                let link = e.flags & crate::location::flags::SYMLINK != 0;
                (e.name.clone(), Meta { is_dir: e.is_dir(), is_link: link, size: e.size, mtime: e.mtime })
            })
            .collect();
        self.cache.lock().insert(path.to_string(), list.clone());
        Ok(list)
    }
    fn mkdir(&self, path: &str) -> AppResult<()> {
        self.forget(path);
        self.run(|c, _| c.mkdir(path).map_err(AppError::from)).map_err(|e| e.with_path(path))
    }
    fn remove_file(&self, path: &str) -> AppResult<()> {
        self.forget(path);
        self.run(|c, _| c.rm(path).map_err(AppError::from)).map_err(|e| e.with_path(path))
    }
    fn remove_dir(&self, path: &str) -> AppResult<()> {
        self.forget(path);
        self.cache.lock().remove(path);
        self.run(|c, _| c.rmdir(path).map_err(AppError::from)).map_err(|e| e.with_path(path))
    }
    fn rename(&self, from: &str, to: &str) -> AppResult<()> {
        self.forget(from);
        self.forget(to);
        self.run(|c, _| c.rename(from, to).map_err(AppError::from)).map_err(|e| e.with_path(from))
    }
    fn reader(&self, path: &str) -> AppResult<Box<dyn ReadStream + '_>> {
        let s = self.run(|c, _| c.retr_as_stream(path).map_err(AppError::from)).map_err(|e| e.with_path(path))?;
        Ok(Box::new(FtpStreamWrap(s)))
    }
    fn writer(&self, path: &str) -> AppResult<Box<dyn WriteStream + '_>> {
        self.forget(path);
        let s = self.run(|c, _| c.put_with_stream(path).map_err(AppError::from)).map_err(|e| e.with_path(path))?;
        Ok(Box::new(FtpStreamWrap(s)))
    }
    fn set_mtime(&self, path: &str, mtime: i64) {
        if mtime <= 0 {
            return;
        }
        let (y, mo, d, h, mi, s) = crate::util::utc_parts(mtime);
        let cmd = format!("MFMT {y:04}{mo:02}{d:02}{h:02}{mi:02}{s:02} {path}");
        let _ = self.run(|c, _| {
            c.custom_command(cmd.clone(), &[suppaftp::Status::File])
                .map(|_| ())
                .map_err(AppError::from)
        });
    }
}

// ---------------------------------------------------------------------------
// Opening locations
// ---------------------------------------------------------------------------

fn ftp_cfg(reg: &FtpRegistry, session: &str) -> AppResult<FtpConfig> {
    Ok(reg.get(session)?.cfg.clone())
}

pub fn open_location(loc: &Location, reg: &FtpRegistry) -> AppResult<(Box<dyn Vfs>, String)> {
    match loc {
        Location::Local { path } => Ok((Box::new(LocalFs), path_str(&local::normalize(path)))),
        Location::Ftp { session, path } => {
            Ok((Box::new(FtpFs::connect(ftp_cfg(reg, session)?, session.clone())?), path.clone()))
        }
        Location::Archive { .. } => Err(AppError::unsupported("Writing into archives is not supported")),
        Location::Drives => Err(AppError::invalid("Choose a folder")),
    }
}

/// Opens the file system of the (homogeneous) source list.
pub fn open_sources(sources: &[Location], reg: &FtpRegistry) -> AppResult<(Box<dyn Vfs>, Vec<String>)> {
    let first = sources.first().ok_or_else(|| AppError::invalid("Nothing selected"))?;
    let (fs, _) = match first {
        Location::Ftp { session, .. } => open_location(&Location::Ftp { session: session.clone(), path: "/".into() }, reg)?,
        other => open_location(other, reg)?,
    };
    let mut paths = Vec::with_capacity(sources.len());
    for s in sources {
        match (s, first) {
            (Location::Local { path }, Location::Local { .. }) => paths.push(path_str(&local::normalize(path))),
            (Location::Ftp { session, path }, Location::Ftp { session: s0, .. }) if session == s0 => paths.push(path.clone()),
            _ => return Err(AppError::invalid("Mixed sources")),
        }
    }
    Ok((fs, paths))
}

fn loc_name(loc: &Location) -> String {
    match loc {
        Location::Local { path } => {
            Path::new(path).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| path.clone())
        }
        Location::Ftp { path, .. } => ftp::file_name(path).to_string(),
        Location::Archive { inner, archive } => {
            if inner.is_empty() {
                loc_name(&Location::local(archive.clone()))
            } else {
                inner.trim_end_matches('/').rsplit('/').next().unwrap_or(inner).to_string()
            }
        }
        Location::Drives => String::new(),
    }
}

pub fn loc_display(loc: &Location, reg: &FtpRegistry) -> String {
    match loc {
        Location::Local { path } => path.clone(),
        Location::Ftp { session, path } => match reg.get(session) {
            Ok(s) => format!("{}{}", s.label, path),
            Err(_) => path.clone(),
        },
        Location::Archive { archive, inner } => format!("{archive}/{inner}"),
        Location::Drives => String::new(),
    }
}

pub fn describe(req: &JobRequest, reg: &FtpRegistry) -> (String, usize, String) {
    let first = req.sources.first().map(loc_name).unwrap_or_default();
    let dest = req.dest.as_ref().map(|d| loc_display(d, reg)).unwrap_or_default();
    (first, req.sources.len(), dest)
}

pub fn parent_location(loc: &Location) -> Option<Location> {
    match loc {
        Location::Local { path } => Path::new(path).parent().map(|p| Location::local(path_str(p))),
        Location::Ftp { session, path } => ftp::parent(path).map(|p| Location::Ftp { session: session.clone(), path: p }),
        Location::Archive { archive, .. } => Path::new(archive).parent().map(|p| Location::local(path_str(p))),
        Location::Drives => None,
    }
}

pub fn touched_locations(req: &JobRequest) -> Vec<Location> {
    let mut out: Vec<Location> = Vec::new();
    if req.op != JobOp::Copy && req.op != JobOp::Extract {
        for s in &req.sources {
            if let Some(p) = parent_location(s) {
                if !out.contains(&p) {
                    out.push(p);
                }
            }
        }
    }
    if let Some(d) = &req.dest {
        let d = if req.op == JobOp::Pack { parent_location(d).unwrap_or_else(|| d.clone()) } else { d.clone() };
        if !out.contains(&d) {
            out.push(d);
        }
    }
    out
}
