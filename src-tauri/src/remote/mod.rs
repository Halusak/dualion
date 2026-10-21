//! Remote file systems behind one interface: FTP/FTPS, SFTP, WebDAV, S3,
//! Android devices (ADB), Docker containers and rclone remotes.
//!
//! Every protocol implements [`RemoteFs`] (list, read, write, rename…);
//! panels browse a [`Session`] and the job engine copies through
//! [`RemoteVfs`], so copying, moving, deleting, queues, previews and opening
//! files work the same everywhere. Paths are `/`-separated.

pub mod adb;
pub mod docker;
pub mod ftp;
#[cfg(windows)]
pub mod mtp;
pub mod rclone;
pub mod s3;
pub mod sftp;
pub mod tools;
pub mod webdav;

use crate::error::{AppError, AppResult, ErrorCode};
use crate::jobs::vfs::{Meta, ReadStream, Vfs, WriteStream};
use crate::location::{flags, Crumb, Entry, Listing, Location};
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::{Arc, LazyLock};
use std::time::{Duration, Instant};
use zeroize::Zeroize;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default, Hash)]
#[serde(rename_all = "camelCase")]
pub enum Protocol {
    #[default]
    Ftp,
    FtpsExplicit,
    FtpsImplicit,
    Sftp,
    /// WebDAV over HTTP.
    Webdav,
    /// WebDAV over HTTPS.
    Webdavs,
    S3,
    /// Android device through the ADB server (found automatically).
    Adb,
    /// Docker / Podman container (found automatically).
    Docker,
    /// A remote configured in rclone (found automatically).
    Rclone,
    /// Phone or camera over MTP (Windows, found automatically).
    Mtp,
}

impl Protocol {
    pub fn is_ftp(self) -> bool {
        matches!(self, Protocol::Ftp | Protocol::FtpsExplicit | Protocol::FtpsImplicit)
    }

    pub fn default_port(self) -> u16 {
        match self {
            Protocol::Ftp | Protocol::FtpsExplicit => 21,
            Protocol::FtpsImplicit => 990,
            Protocol::Sftp => 22,
            Protocol::Webdav => 80,
            Protocol::Webdavs | Protocol::S3 => 443,
            Protocol::Adb | Protocol::Docker | Protocol::Rclone | Protocol::Mtp => 0,
        }
    }

    fn scheme(self) -> &'static str {
        match self {
            Protocol::Ftp => "ftp",
            Protocol::FtpsExplicit | Protocol::FtpsImplicit => "ftps",
            Protocol::Sftp => "sftp",
            Protocol::Webdav => "dav",
            Protocol::Webdavs => "davs",
            Protocol::S3 => "s3",
            Protocol::Adb => "adb",
            Protocol::Docker => "docker",
            Protocol::Rclone => "rclone",
            Protocol::Mtp => "mtp",
        }
    }
}

fn yes() -> bool {
    true
}

/// A password that is wiped from memory when dropped.
#[derive(Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(transparent)]
pub struct Secret(String);

impl std::ops::Deref for Secret {
    type Target = String;
    fn deref(&self) -> &String {
        &self.0
    }
}

impl From<String> for Secret {
    fn from(s: String) -> Self {
        Secret(s)
    }
}

impl From<&str> for Secret {
    fn from(s: &str) -> Self {
        Secret(s.to_string())
    }
}

impl Drop for Secret {
    fn drop(&mut self) {
        self.0.zeroize();
    }
}

/// How to reach a remote file system. Saved sites (vault) and discovered
/// sources (ADB devices, containers, rclone remotes) both end up here.
#[derive(Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct RemoteConfig {
    #[serde(default)]
    pub protocol: Protocol,
    /// Host name; WebDAV may use a full URL; S3 the endpoint (empty = AWS);
    /// ADB the device serial, Docker the container id, rclone the remote name.
    pub host: String,
    #[serde(default)]
    pub port: u16,
    /// User name; S3 access key id.
    #[serde(default)]
    pub username: String,
    /// Password; S3 secret key; SFTP key passphrase when a key is used.
    #[serde(default)]
    pub password: Secret,
    #[serde(default = "yes")]
    pub passive: bool,
    /// FTP: "" / "utf-8" or any WHATWG label understood by encoding_rs.
    #[serde(default)]
    pub encoding: String,
    #[serde(default)]
    pub accept_invalid_certs: bool,
    /// Start folder; S3 `bucket/prefix`.
    #[serde(default)]
    pub remote_path: String,
    /// SFTP private key file (empty: SSH agent, then ~/.ssh/id_*).
    #[serde(default)]
    pub key_path: String,
    /// S3 region (empty = us-east-1).
    #[serde(default)]
    pub region: String,
    /// Name shown in the panel (discovered sources).
    #[serde(default)]
    pub display: String,
    /// SFTP host key fingerprint the user accepted for this connection.
    #[serde(default)]
    pub trust_key: String,
}

impl std::fmt::Debug for RemoteConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RemoteConfig")
            .field("protocol", &self.protocol)
            .field("host", &self.host)
            .field("port", &self.port)
            .field("username", &self.username)
            .finish_non_exhaustive()
    }
}

impl RemoteConfig {
    pub fn effective_port(&self) -> u16 {
        if self.port == 0 {
            self.protocol.default_port()
        } else {
            self.port
        }
    }

    pub fn label(&self) -> String {
        let host = self.host.trim();
        match self.protocol {
            Protocol::Adb | Protocol::Docker | Protocol::Rclone | Protocol::Mtp => {
                format!("{}://{}", self.protocol.scheme(), if self.display.is_empty() { host } else { &self.display })
            }
            Protocol::Webdav | Protocol::Webdavs if host.contains("://") => host.trim_end_matches('/').to_string(),
            Protocol::S3 if host.is_empty() => "s3://aws".into(),
            p => {
                let port = self.effective_port();
                let port_part = if port == p.default_port() { String::new() } else { format!(":{port}") };
                let user = self.username.trim();
                if user.is_empty() || p == Protocol::S3 {
                    format!("{}://{host}{port_part}", p.scheme())
                } else {
                    format!("{}://{user}@{host}{port_part}", p.scheme())
                }
            }
        }
    }

    /// First crumb of the path bar.
    fn root_label(&self) -> String {
        if !self.display.is_empty() {
            return self.display.clone();
        }
        let host = self.host.trim();
        match self.protocol {
            Protocol::S3 if host.is_empty() => "Amazon S3".into(),
            _ => host.split("://").last().unwrap_or(host).split('/').next().unwrap_or(host).to_string(),
        }
    }
}

// ---------------------------------------------------------------------------
// File system interface
// ---------------------------------------------------------------------------

/// Operations every remote protocol provides. Paths are absolute, `/`-separated.
pub trait RemoteFs: Send + Sync {
    fn list(&self, path: &str) -> AppResult<Vec<Entry>>;
    /// `Ok(None)` when the path does not exist.
    fn stat(&self, path: &str) -> AppResult<Option<Meta>> {
        let Some(parent) = parent(path) else { return Ok(Some(dir_meta())) };
        let name = file_name(path);
        match self.list(&parent) {
            Ok(list) => Ok(list.iter().find(|e| e.name == name).map(meta_of)),
            Err(e) if e.code == ErrorCode::NotFound => Ok(None),
            Err(e) => Err(e),
        }
    }
    fn mkdir(&self, path: &str) -> AppResult<()>;
    fn remove_file(&self, path: &str) -> AppResult<()>;
    fn remove_dir(&self, path: &str) -> AppResult<()>;
    /// Renames within the file system. `CrossesDevices` makes the copy
    /// engine copy and delete instead (e.g. S3 folders).
    fn rename(&self, from: &str, to: &str) -> AppResult<()>;
    fn reader(&self, path: &str) -> AppResult<Box<dyn ReadStream + '_>>;
    fn writer(&self, path: &str) -> AppResult<Box<dyn WriteStream + '_>>;
    fn set_mtime(&self, _path: &str, _mtime: i64) {}
    /// Called every ~45 s on idle sessions.
    fn keepalive(&self) {}
    fn close(&self) {}
}

pub fn dir_meta() -> Meta {
    Meta { is_dir: true, is_link: false, size: 0, mtime: 0 }
}

pub fn meta_of(e: &Entry) -> Meta {
    Meta { is_dir: e.is_dir(), is_link: e.flags & flags::SYMLINK != 0, size: e.size, mtime: e.mtime }
}

/// Builds a listing entry (hidden = dot files).
pub fn entry(name: String, is_dir: bool, size: u64, mtime: i64, mode: u32, link: bool) -> Entry {
    let mut fl = 0;
    if is_dir {
        fl |= flags::DIR;
    }
    if link {
        fl |= flags::SYMLINK;
    }
    if name.starts_with('.') {
        fl |= flags::HIDDEN;
    }
    if !is_dir && mode & 0o111 != 0 {
        fl |= flags::EXEC;
    }
    Entry { name, flags: fl, size: if is_dir { 0 } else { size }, mtime, attrs: mode & 0o7777 }
}

// ---------------------------------------------------------------------------
// Paths
// ---------------------------------------------------------------------------

pub fn join(dir: &str, name: &str) -> String {
    if dir.ends_with('/') {
        format!("{dir}{name}")
    } else {
        format!("{dir}/{name}")
    }
}

pub fn parent(path: &str) -> Option<String> {
    let p = path.trim_end_matches('/');
    if p.is_empty() {
        return None;
    }
    match p.rfind('/') {
        Some(0) | None => Some("/".into()),
        Some(i) => Some(p[..i].to_string()),
    }
}

pub fn file_name(path: &str) -> &str {
    let p = path.trim_end_matches('/');
    p.rsplit('/').next().unwrap_or(p)
}

/// "/a/b/" → "/a/b"; "" → "/".
pub fn norm(path: &str) -> String {
    let p = path.trim();
    let p = if p.starts_with('/') { p.to_string() } else { format!("/{p}") };
    let p = p.trim_end_matches('/');
    if p.is_empty() {
        "/".into()
    } else {
        p.to_string()
    }
}

// ---------------------------------------------------------------------------
// Async runtime for protocols built on tokio (SFTP)
// ---------------------------------------------------------------------------

static RT: LazyLock<tokio::runtime::Runtime> = LazyLock::new(|| {
    tokio::runtime::Builder::new_multi_thread().worker_threads(2).thread_name("remote-io").enable_all().build().expect("remote runtime")
});

/// Runs a future to completion from blocking code (never from async code).
pub fn block_on<F: std::future::Future>(f: F) -> F::Output {
    RT.block_on(f)
}

// ---------------------------------------------------------------------------
// Connecting
// ---------------------------------------------------------------------------

/// Opens the file system; returns it with the start folder and a server greeting.
pub fn open(cfg: &RemoteConfig) -> AppResult<(Arc<dyn RemoteFs>, String, Option<String>)> {
    if cfg.host.trim().is_empty() && !matches!(cfg.protocol, Protocol::S3) {
        return Err(AppError::invalid("Missing host"));
    }
    let start = |default: &str| {
        let p = cfg.remote_path.trim();
        if p.is_empty() {
            default.to_string()
        } else {
            norm(p)
        }
    };
    Ok(match cfg.protocol {
        Protocol::Ftp | Protocol::FtpsExplicit | Protocol::FtpsImplicit => {
            let (fs, pwd, welcome) = ftp::FtpRemote::connect(cfg.clone())?;
            (Arc::new(fs), start(&pwd), welcome)
        }
        Protocol::Sftp => {
            let (fs, home) = sftp::SftpRemote::connect(cfg)?;
            (Arc::new(fs), start(&home), None)
        }
        Protocol::Webdav | Protocol::Webdavs => {
            let fs = webdav::WebDav::connect(cfg)?;
            (Arc::new(fs), start("/"), None)
        }
        Protocol::S3 => {
            let fs = s3::S3::connect(cfg)?;
            (Arc::new(fs), start("/"), None)
        }
        Protocol::Adb => {
            let fs = adb::Adb::connect(cfg)?;
            let home = fs.home();
            (Arc::new(fs), start(&home), None)
        }
        Protocol::Docker => {
            let fs = docker::Docker::connect(cfg)?;
            (Arc::new(fs), start("/"), None)
        }
        Protocol::Rclone => {
            let fs = rclone::Rclone::connect(cfg)?;
            (Arc::new(fs), start("/"), None)
        }
        #[cfg(windows)]
        Protocol::Mtp => (Arc::new(mtp::Mtp::connect(cfg)?), start("/"), None),
        #[cfg(not(windows))]
        Protocol::Mtp => return Err(AppError::unsupported("MTP devices are available on Windows; on a Mac use ADB")),
    })
}

/// Checks that a site works without keeping the connection.
pub fn test(cfg: &RemoteConfig) -> AppResult<String> {
    let (fs, start, welcome) = open(cfg)?;
    let n = fs.list(&start)?.len();
    fs.close();
    Ok(format!("{}\n{start} · {n}", welcome.unwrap_or_default()).trim().to_string())
}

// ---------------------------------------------------------------------------
// Sessions (one per connection opened from a panel)
// ---------------------------------------------------------------------------

pub struct Session {
    pub id: String,
    pub cfg: RemoteConfig,
    pub label: String,
    fs: Arc<dyn RemoteFs>,
    last_used: Mutex<Instant>,
}

impl Session {
    pub fn fs(&self) -> Arc<dyn RemoteFs> {
        *self.last_used.lock() = Instant::now();
        self.fs.clone()
    }

    pub fn list(&self, path: &str) -> AppResult<Listing> {
        let path = norm(path);
        let entries = self.fs().list(&path)?;
        Ok(Listing {
            location: Location::Remote { session: self.id.clone(), path: path.clone() },
            display_path: format!("{}{}", self.label, if path == "/" { "" } else { &path }),
            parent: parent(&path).map(|p| Location::Remote { session: self.id.clone(), path: p }),
            crumbs: self.crumbs(&path),
            entries,
        })
    }

    fn crumbs(&self, path: &str) -> Vec<Crumb> {
        let mut out = vec![Crumb { label: self.cfg.root_label(), icon: "server", location: Location::Remote { session: self.id.clone(), path: "/".into() } }];
        let mut acc = String::new();
        for part in path.split('/').filter(|s| !s.is_empty()) {
            acc.push('/');
            acc.push_str(part);
            out.push(Crumb { label: part.to_string(), icon: "folder", location: Location::Remote { session: self.id.clone(), path: acc.clone() } });
        }
        out
    }

    /// File system for a job. FTP needs its own connection (one transfer at a
    /// time); the other protocols share the session's.
    pub fn job_fs(&self) -> AppResult<Box<dyn Vfs>> {
        let fs: Arc<dyn RemoteFs> = if self.cfg.protocol.is_ftp() { Arc::new(ftp::FtpRemote::connect(self.cfg.clone())?.0) } else { self.fs() };
        Ok(Box::new(RemoteVfs::new(fs, &self.id)))
    }
}

#[derive(Default)]
pub struct Registry {
    sessions: Mutex<HashMap<String, Arc<Session>>>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Connected {
    pub session: String,
    pub label: String,
    pub path: String,
    pub welcome: Option<String>,
}

impl Registry {
    pub fn open(&self, cfg: RemoteConfig) -> AppResult<Connected> {
        let (fs, path, welcome) = open(&cfg)?;
        let id = uuid::Uuid::new_v4().to_string();
        let label = cfg.label();
        let session = Arc::new(Session { id: id.clone(), label: label.clone(), cfg, fs, last_used: Mutex::new(Instant::now()) });
        self.sessions.lock().insert(id.clone(), session);
        Ok(Connected { session: id, label, path, welcome })
    }

    pub fn get(&self, id: &str) -> AppResult<Arc<Session>> {
        self.sessions.lock().get(id).cloned().ok_or_else(|| AppError::new(ErrorCode::FtpConnect, "The connection is closed"))
    }

    pub fn close(&self, id: &str) {
        if let Some(s) = self.sessions.lock().remove(id) {
            s.fs.close();
        }
    }

    /// Keeps idle connections alive (FTP NOOP, SSH keepalive…).
    pub fn keepalive(&self) {
        let sessions: Vec<_> = self.sessions.lock().values().cloned().collect();
        for s in sessions {
            if s.last_used.lock().elapsed() >= Duration::from_secs(45) {
                s.fs.keepalive();
                *s.last_used.lock() = Instant::now();
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Job engine adapter
// ---------------------------------------------------------------------------

/// [`Vfs`] over a [`RemoteFs`], caching listings for the duration of a job.
pub struct RemoteVfs {
    fs: Arc<dyn RemoteFs>,
    id: String,
    cache: Mutex<HashMap<String, Vec<(String, Meta)>>>,
}

impl RemoteVfs {
    pub fn new(fs: Arc<dyn RemoteFs>, session: &str) -> Self {
        Self { fs, id: format!("remote:{session}"), cache: Mutex::new(HashMap::new()) }
    }

    fn forget(&self, path: &str) {
        if let Some(p) = parent(path) {
            self.cache.lock().remove(&p);
        }
    }
}

impl Vfs for RemoteVfs {
    fn is_local(&self) -> bool {
        false
    }
    fn id(&self) -> String {
        self.id.clone()
    }
    fn join(&self, dir: &str, name: &str) -> String {
        join(dir, name)
    }
    fn name(&self, path: &str) -> String {
        file_name(path).to_string()
    }
    fn parent(&self, path: &str) -> Option<String> {
        parent(path)
    }
    fn stat(&self, path: &str, _follow: bool) -> AppResult<Option<Meta>> {
        let Some(p) = parent(path) else { return Ok(Some(dir_meta())) };
        let name = file_name(path).to_string();
        match self.read_dir(&p) {
            Ok(list) => Ok(list.into_iter().find(|(n, _)| *n == name).map(|(_, m)| m)),
            Err(e) if e.code == ErrorCode::NotFound => Ok(None),
            Err(e) => Err(e),
        }
    }
    fn read_dir(&self, path: &str) -> AppResult<Vec<(String, Meta)>> {
        if let Some(hit) = self.cache.lock().get(path) {
            return Ok(hit.clone());
        }
        let list: Vec<(String, Meta)> = self.fs.list(path)?.iter().map(|e| (e.name.clone(), meta_of(e))).collect();
        self.cache.lock().insert(path.to_string(), list.clone());
        Ok(list)
    }
    fn mkdir(&self, path: &str) -> AppResult<()> {
        self.forget(path);
        self.fs.mkdir(path).map_err(|e| e.with_path(path))
    }
    fn remove_file(&self, path: &str) -> AppResult<()> {
        self.forget(path);
        self.fs.remove_file(path).map_err(|e| e.with_path(path))
    }
    fn remove_dir(&self, path: &str) -> AppResult<()> {
        self.forget(path);
        self.cache.lock().remove(path);
        self.fs.remove_dir(path).map_err(|e| e.with_path(path))
    }
    fn rename(&self, from: &str, to: &str) -> AppResult<()> {
        self.forget(from);
        self.forget(to);
        self.fs.rename(from, to).map_err(|e| if e.code == ErrorCode::CrossesDevices { e } else { e.with_path(from) })
    }
    fn reader(&self, path: &str) -> AppResult<Box<dyn ReadStream + '_>> {
        self.fs.reader(path).map_err(|e| e.with_path(path))
    }
    fn writer(&self, path: &str) -> AppResult<Box<dyn WriteStream + '_>> {
        self.forget(path);
        self.fs.writer(path).map_err(|e| e.with_path(path))
    }
    fn set_mtime(&self, path: &str, mtime: i64) {
        if mtime > 0 {
            self.fs.set_mtime(path, mtime);
        }
    }
}

// ---------------------------------------------------------------------------
// Shared stream helpers
// ---------------------------------------------------------------------------

/// Reader that needs no completion step.
pub struct PlainReader<R>(pub R);

impl<R: std::io::Read> std::io::Read for PlainReader<R> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        self.0.read(buf)
    }
}
impl<R: std::io::Read + Send> ReadStream for PlainReader<R> {
    fn finish(self: Box<Self>) -> AppResult<()> {
        Ok(())
    }
}

/// Writer that spools to a temporary file and uploads it in `finish`
/// (protocols that need the size up front: S3, Docker, WebDAV).
type Upload<'a> = Box<dyn FnOnce(std::fs::File, u64) -> AppResult<()> + Send + 'a>;

type UploadPath<'a> = Box<dyn FnOnce(&std::path::Path) -> AppResult<()> + Send + 'a>;

pub struct SpoolWriter<'a> {
    file: std::fs::File,
    path: std::path::PathBuf,
    upload: Option<Upload<'a>>,
    upload_path: Option<UploadPath<'a>>,
}

impl<'a> SpoolWriter<'a> {
    /// Spools, then hands the file (rewound) and its size to `upload`.
    pub fn new(upload: impl FnOnce(std::fs::File, u64) -> AppResult<()> + Send + 'a) -> AppResult<Self> {
        let (file, path) = Self::temp()?;
        Ok(Self { file, path, upload: Some(Box::new(upload)), upload_path: None })
    }

    /// Spools, then hands the finished temporary file's path to `upload`.
    #[cfg_attr(not(windows), allow(dead_code))]
    pub fn new_path(upload: impl FnOnce(&std::path::Path) -> AppResult<()> + Send + 'a) -> AppResult<Self> {
        let (file, path) = Self::temp()?;
        Ok(Self { file, path, upload: None, upload_path: Some(Box::new(upload)) })
    }

    fn temp() -> AppResult<(std::fs::File, std::path::PathBuf)> {
        let dir = crate::platform::temp_dir().join("upload");
        std::fs::create_dir_all(&dir).map_err(|e| AppError::io(e, &dir))?;
        let path = dir.join(uuid::Uuid::new_v4().to_string());
        let file = std::fs::OpenOptions::new().read(true).write(true).create_new(true).open(&path).map_err(|e| AppError::io(e, &path))?;
        Ok((file, path))
    }
}

impl std::io::Write for SpoolWriter<'_> {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.file.write(buf)
    }
    fn flush(&mut self) -> std::io::Result<()> {
        self.file.flush()
    }
}

impl WriteStream for SpoolWriter<'_> {
    fn finish(mut self: Box<Self>) -> AppResult<()> {
        use std::io::{Seek, Write};
        if let Some(upload) = self.upload_path.take() {
            self.file.flush().map_err(|e| AppError::io(e, &self.path))?;
            return upload(&self.path);
        }
        let size = self.file.stream_position().map_err(|e| AppError::io(e, &self.path))?;
        self.file.rewind().map_err(|e| AppError::io(e, &self.path))?;
        let file = self.file.try_clone().map_err(|e| AppError::io(e, &self.path))?;
        let upload = self.upload.take().expect("finished once");
        upload(file, size)
    }
}

impl Drop for SpoolWriter<'_> {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

// ---------------------------------------------------------------------------
// Discovered sources (Network button)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Source {
    pub protocol: Protocol,
    /// Device serial / container id / remote name.
    pub id: String,
    pub name: String,
    pub detail: String,
}

/// Android devices, running containers and rclone remotes (each looked up
/// in parallel; a missing tool simply contributes nothing).
pub fn sources() -> Vec<Source> {
    let jobs: Vec<std::thread::JoinHandle<Vec<Source>>> = vec![
        std::thread::spawn(|| adb::devices().unwrap_or_default()),
        std::thread::spawn(|| docker::containers().unwrap_or_default()),
        std::thread::spawn(|| rclone::remotes().unwrap_or_default()),
        #[cfg(windows)]
        std::thread::spawn(|| mtp::devices().unwrap_or_default()),
    ];
    jobs.into_iter().flat_map(|j| j.join().unwrap_or_default()).collect()
}

/// Connection config of a discovered source.
pub fn source_config(protocol: Protocol, id: &str, name: &str) -> RemoteConfig {
    RemoteConfig { protocol, host: id.to_string(), display: name.to_string(), passive: true, ..Default::default() }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn path_helpers() {
        assert_eq!(join("/", "a"), "/a");
        assert_eq!(join("/a", "b"), "/a/b");
        assert_eq!(parent("/a/b"), Some("/a".into()));
        assert_eq!(parent("/a"), Some("/".into()));
        assert_eq!(parent("/"), None);
        assert_eq!(file_name("/a/b.txt"), "b.txt");
        assert_eq!(norm("a/b/"), "/a/b");
        assert_eq!(norm(""), "/");
    }

    #[test]
    fn labels() {
        let mut cfg = RemoteConfig { host: "example.com".into(), username: "jan".into(), ..Default::default() };
        assert_eq!(cfg.label(), "ftp://jan@example.com");
        cfg.protocol = Protocol::Sftp;
        cfg.port = 2222;
        assert_eq!(cfg.label(), "sftp://jan@example.com:2222");
        let cfg = RemoteConfig { protocol: Protocol::FtpsExplicit, host: "h".into(), port: 2121, ..Default::default() };
        assert_eq!(cfg.label(), "ftps://h:2121");
        let cfg = RemoteConfig { protocol: Protocol::Webdavs, host: "https://cloud.example.com/dav/".into(), ..Default::default() };
        assert_eq!(cfg.label(), "https://cloud.example.com/dav");
        assert_eq!(cfg.root_label(), "cloud.example.com");
        let cfg = source_config(Protocol::Adb, "emulator-5554", "Pixel 9a");
        assert_eq!((cfg.label().as_str(), cfg.root_label().as_str()), ("adb://Pixel 9a", "Pixel 9a"));
    }
}
