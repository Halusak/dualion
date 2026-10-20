//! FTP / FTPS client sessions built on suppaftp (sync API, native TLS).

use crate::error::{AppError, AppResult, ErrorCode};
use crate::location::{flags, Crumb, Entry, Listing, Location};
use crate::util::system_time_ms;
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::net::ToSocketAddrs;
use std::str::FromStr;
use std::sync::Arc;
use std::time::{Duration, Instant};
use suppaftp::list::{File as FtpFile, ListParser};
use suppaftp::native_tls::TlsConnector;
use suppaftp::types::FileType;
use suppaftp::{Mode, NativeTlsConnector, NativeTlsFtpStream};
use zeroize::Zeroize;

pub type FtpConn = NativeTlsFtpStream;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub enum FtpProtocol {
    #[default]
    Ftp,
    FtpsExplicit,
    FtpsImplicit,
}

fn yes() -> bool {
    true
}

#[derive(Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct FtpConfig {
    #[serde(default)]
    pub protocol: FtpProtocol,
    pub host: String,
    #[serde(default)]
    pub port: u16,
    #[serde(default)]
    pub username: String,
    #[serde(default)]
    pub password: String,
    #[serde(default = "yes")]
    pub passive: bool,
    /// "" / "utf-8" or any WHATWG label understood by encoding_rs ("windows-1250", "iso-8859-2"…)
    #[serde(default)]
    pub encoding: String,
    #[serde(default)]
    pub accept_invalid_certs: bool,
    #[serde(default)]
    pub remote_path: String,
}

impl std::fmt::Debug for FtpConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FtpConfig")
            .field("protocol", &self.protocol)
            .field("host", &self.host)
            .field("port", &self.port)
            .field("username", &self.username)
            .finish_non_exhaustive()
    }
}

impl Drop for FtpConfig {
    fn drop(&mut self) {
        self.password.zeroize();
    }
}

impl FtpConfig {
    pub fn effective_port(&self) -> u16 {
        match (self.port, self.protocol) {
            (0, FtpProtocol::FtpsImplicit) => 990,
            (0, _) => 21,
            (p, _) => p,
        }
    }

    pub fn label(&self) -> String {
        let scheme = match self.protocol {
            FtpProtocol::Ftp => "ftp",
            _ => "ftps",
        };
        let host = self.host.trim();
        let port = self.effective_port();
        let default_port = if self.protocol == FtpProtocol::FtpsImplicit { 990 } else { 21 };
        let port_part = if port == default_port { String::new() } else { format!(":{port}") };
        if self.username.is_empty() {
            format!("{scheme}://{host}{port_part}")
        } else {
            format!("{scheme}://{}@{host}{port_part}", self.username)
        }
    }
}

/// encoding_rs backed codec for servers that do not speak UTF-8
/// (old Czech servers frequently use windows-1250).
#[derive(Debug)]
struct RsEncoding(&'static encoding_rs::Encoding);

impl suppaftp::Encoding for RsEncoding {
    fn decode(&self, bytes: &[u8]) -> String {
        self.0.decode_without_bom_handling(bytes).0.into_owned()
    }

    fn encode(&self, text: &str) -> std::io::Result<Vec<u8>> {
        let (bytes, _, had_errors) = self.0.encode(text);
        if had_errors {
            return Err(std::io::Error::new(std::io::ErrorKind::InvalidInput, "character not representable"));
        }
        Ok(bytes.into_owned())
    }
}

fn tls(cfg: &FtpConfig) -> AppResult<NativeTlsConnector> {
    let mut b = TlsConnector::builder();
    if cfg.accept_invalid_certs {
        b.danger_accept_invalid_certs(true).danger_accept_invalid_hostnames(true);
    }
    let c = b.build().map_err(|e| AppError::new(ErrorCode::FtpConnect, e.to_string()))?;
    Ok(NativeTlsConnector::from(c))
}

pub fn connect(cfg: &FtpConfig) -> AppResult<FtpConn> {
    let host = cfg.host.trim();
    if host.is_empty() {
        return Err(AppError::invalid("Missing host"));
    }
    let port = cfg.effective_port();
    let addr = (host, port)
        .to_socket_addrs()
        .map_err(|e| AppError::new(ErrorCode::FtpConnect, e.to_string()))?
        .next()
        .ok_or_else(|| AppError::new(ErrorCode::FtpConnect, format!("Cannot resolve {host}")))?;
    let timeout = Duration::from_secs(20);
    let mut ftp = match cfg.protocol {
        FtpProtocol::Ftp => FtpConn::connect_timeout(addr, timeout)?,
        FtpProtocol::FtpsExplicit => FtpConn::connect_timeout(addr, timeout)?.into_secure(tls(cfg)?, host)?,
        FtpProtocol::FtpsImplicit => FtpConn::connect_secure_implicit(addr, tls(cfg)?, host)?,
    };
    {
        let sock = ftp.get_ref();
        let _ = sock.set_read_timeout(Some(Duration::from_secs(60)));
        let _ = sock.set_write_timeout(Some(Duration::from_secs(60)));
    }
    let anonymous = cfg.username.trim().is_empty();
    let user = if anonymous { "anonymous" } else { cfg.username.trim() };
    let pass = if anonymous && cfg.password.is_empty() { "anonymous@" } else { cfg.password.as_str() };
    ftp.login(user, pass)?;

    if cfg.passive {
        if addr.is_ipv6() {
            ftp.set_mode(Mode::ExtendedPassive);
        }
        // Servers behind NAT often announce a private address in PASV replies.
        ftp.set_passive_nat_workaround(true);
    } else {
        ftp = ftp.active_mode(Duration::from_secs(30));
    }

    let enc = cfg.encoding.trim().to_ascii_lowercase();
    if enc.is_empty() || enc == "utf-8" || enc == "utf8" {
        let _ = ftp.opts("UTF8", Some("ON"));
    } else if let Some(e) = encoding_rs::Encoding::for_label(enc.as_bytes()) {
        ftp.set_encoding(RsEncoding(e));
    }
    ftp.transfer_type(FileType::Binary)?;
    Ok(ftp)
}

pub fn supports_mlsd(ftp: &mut FtpConn) -> bool {
    ftp.feat().map(|f| f.keys().any(|k| k.eq_ignore_ascii_case("MLSD") || k.eq_ignore_ascii_case("MLST"))).unwrap_or(false)
}

pub fn file_to_entry(f: &FtpFile) -> Entry {
    let mut fl = 0;
    if f.is_directory() {
        fl |= flags::DIR;
    }
    if f.is_symlink() {
        fl |= flags::SYMLINK;
        // Most servers point directory links to directories; we let the user try to enter them.
        fl |= flags::DIR;
    }
    let name = f.name().to_string();
    if name.starts_with('.') {
        fl |= flags::HIDDEN;
    }
    Entry {
        name,
        flags: fl,
        size: if f.is_directory() { 0 } else { f.size() as u64 },
        mtime: system_time_ms(f.modified()),
        attrs: 0,
    }
}

/// Lists a remote directory. Uses MLSD when available (exact, machine
/// readable), otherwise `LIST -a` with the Unix/DOS parsers.
pub fn list_dir(ftp: &mut FtpConn, path: &str, use_mlsd: bool) -> AppResult<Vec<Entry>> {
    let lines = if use_mlsd {
        ftp.mlsd(Some(path))?
    } else {
        ftp.cwd(path)?;
        match ftp.list(Some("-a")) {
            Ok(l) if !l.is_empty() => l,
            _ => ftp.list(None)?,
        }
    };
    let mut out = Vec::with_capacity(lines.len());
    for line in lines {
        let parsed = if use_mlsd { ListParser::parse_mlsd(&line) } else { FtpFile::from_str(&line) };
        if let Ok(f) = parsed {
            if f.name() == "." || f.name() == ".." || f.name().is_empty() {
                continue;
            }
            out.push(file_to_entry(&f));
        }
    }
    Ok(out)
}

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
        Some(0) => Some("/".into()),
        Some(i) => Some(p[..i].to_string()),
        None => Some("/".into()),
    }
}

pub fn file_name(path: &str) -> &str {
    let p = path.trim_end_matches('/');
    p.rsplit('/').next().unwrap_or(p)
}

fn is_connection_lost(e: &AppError) -> bool {
    e.code == ErrorCode::FtpConnect
        || e.message.starts_with("421")
        || e.message.contains("Broken pipe")
        || e.message.contains("connection reset")
}

// ---------------------------------------------------------------------------
// Sessions (one per connected panel)
// ---------------------------------------------------------------------------

pub struct FtpSession {
    pub id: String,
    pub cfg: FtpConfig,
    pub label: String,
    conn: Mutex<Option<FtpConn>>,
    mlsd: Mutex<Option<bool>>,
    last_used: Mutex<Instant>,
}

impl FtpSession {
    /// Runs `f` on the session connection, reconnecting once when the server
    /// dropped the idle connection.
    pub fn run<T>(&self, mut f: impl FnMut(&mut FtpConn, bool) -> AppResult<T>) -> AppResult<T> {
        let mut guard = self.conn.lock();
        for attempt in 0..2 {
            if guard.is_none() {
                *guard = Some(connect(&self.cfg)?);
            }
            let conn = guard.as_mut().expect("connected above");
            let mlsd = {
                let mut m = self.mlsd.lock();
                *m.get_or_insert_with(|| supports_mlsd(conn))
            };
            match f(conn, mlsd) {
                Ok(v) => {
                    *self.last_used.lock() = Instant::now();
                    return Ok(v);
                }
                Err(e) if attempt == 0 && is_connection_lost(&e) => {
                    *guard = None;
                }
                Err(e) => return Err(e),
            }
        }
        Err(AppError::new(ErrorCode::FtpConnect, "Connection lost"))
    }

    pub fn list(&self, path: &str) -> AppResult<Listing> {
        let path = if path.is_empty() { "/" } else { path };
        let entries = self.run(|c, mlsd| list_dir(c, path, mlsd))?;
        Ok(Listing {
            location: Location::Ftp { session: self.id.clone(), path: path.to_string() },
            display_path: format!("{}{}", self.label, path),
            parent: parent(path).map(|p| Location::Ftp { session: self.id.clone(), path: p }),
            crumbs: self.crumbs(path),
            entries,
        })
    }

    fn crumbs(&self, path: &str) -> Vec<Crumb> {
        let mut out = vec![Crumb {
            label: self.cfg.host.trim().to_string(),
            icon: "server",
            location: Location::Ftp { session: self.id.clone(), path: "/".into() },
        }];
        let mut acc = String::new();
        for part in path.split('/').filter(|s| !s.is_empty()) {
            acc.push('/');
            acc.push_str(part);
            out.push(Crumb {
                label: part.to_string(),
                icon: "folder",
                location: Location::Ftp { session: self.id.clone(), path: acc.clone() },
            });
        }
        out
    }
}

#[derive(Default)]
pub struct FtpRegistry {
    sessions: Mutex<HashMap<String, Arc<FtpSession>>>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Connected {
    pub session: String,
    pub label: String,
    pub path: String,
    pub welcome: Option<String>,
}

impl FtpRegistry {
    pub fn open(&self, cfg: FtpConfig) -> AppResult<Connected> {
        let mut conn = connect(&cfg)?;
        let welcome = conn.get_welcome_msg().map(|s| s.to_string());
        let start = if cfg.remote_path.trim().is_empty() {
            conn.pwd().unwrap_or_else(|_| "/".into())
        } else {
            cfg.remote_path.trim().to_string()
        };
        let id = uuid::Uuid::new_v4().to_string();
        let label = cfg.label();
        let session = Arc::new(FtpSession {
            id: id.clone(),
            label: label.clone(),
            cfg,
            conn: Mutex::new(Some(conn)),
            mlsd: Mutex::new(None),
            last_used: Mutex::new(Instant::now()),
        });
        self.sessions.lock().insert(id.clone(), session);
        Ok(Connected { session: id, label, path: start, welcome })
    }

    pub fn get(&self, id: &str) -> AppResult<Arc<FtpSession>> {
        self.sessions
            .lock()
            .get(id)
            .cloned()
            .ok_or_else(|| AppError::new(ErrorCode::FtpConnect, "FTP session is closed"))
    }

    pub fn close(&self, id: &str) {
        if let Some(s) = self.sessions.lock().remove(id) {
            if let Some(mut c) = s.conn.lock().take() {
                let _ = c.quit();
            }
        }
    }

    /// Sends NOOP on idle sessions so servers do not drop them.
    pub fn keepalive(&self) {
        let sessions: Vec<_> = self.sessions.lock().values().cloned().collect();
        for s in sessions {
            if s.last_used.lock().elapsed() < Duration::from_secs(45) {
                continue;
            }
            if let Some(mut guard) = s.conn.try_lock() {
                if let Some(c) = guard.as_mut() {
                    if c.noop().is_err() {
                        *guard = None;
                    }
                }
                *s.last_used.lock() = Instant::now();
            }
        }
    }
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
    }

    #[test]
    fn parses_listings() {
        let unix = FtpFile::from_str("drwxr-xr-x 2 user group 4096 Nov 5 2018 docs").unwrap();
        assert!(file_to_entry(&unix).is_dir());
        let mlsd = ListParser::parse_mlsd("type=file;size=42;modify=20200101120000; notes.txt").unwrap();
        let e = file_to_entry(&mlsd);
        assert_eq!((e.name.as_str(), e.size), ("notes.txt", 42));
        let dos = FtpFile::from_str("04-27-20  10:11AM       <DIR>          Projekty").unwrap();
        assert!(file_to_entry(&dos).is_dir());
    }

    #[test]
    fn labels() {
        let mut cfg = FtpConfig::default();
        cfg.host = "example.com".into();
        cfg.username = "jan".into();
        assert_eq!(cfg.label(), "ftp://jan@example.com");
        let mut cfg = FtpConfig::default();
        cfg.host = "h".into();
        cfg.port = 2121;
        cfg.protocol = FtpProtocol::FtpsExplicit;
        assert_eq!(cfg.label(), "ftps://h:2121");
    }

    #[test]
    fn cp1250_roundtrip() {
        use suppaftp::Encoding;
        let enc = RsEncoding(encoding_rs::WINDOWS_1250);
        let bytes = enc.encode("Příliš žluťoučký kůň.txt").unwrap();
        assert_eq!(enc.decode(&bytes), "Příliš žluťoučký kůň.txt");
    }
}
