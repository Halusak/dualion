//! FTP / FTPS built on suppaftp (sync API, native TLS).

use super::{Protocol, RemoteConfig, RemoteFs};
use crate::error::{AppError, AppResult, ErrorCode};
use crate::jobs::vfs::{ReadStream, WriteStream};
use crate::location::{flags, Entry};
use crate::util::system_time_ms;
use parking_lot::Mutex;
use std::io::{Read, Write};
use std::net::ToSocketAddrs;
use std::str::FromStr;
use std::time::Duration;
use suppaftp::list::{File as FtpFile, ListParser};
use suppaftp::native_tls::TlsConnector;
use suppaftp::types::FileType;
use suppaftp::{Mode, NativeTlsConnector, NativeTlsFtpStream};

pub type FtpConn = NativeTlsFtpStream;

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

fn tls(cfg: &RemoteConfig) -> AppResult<NativeTlsConnector> {
    let mut b = TlsConnector::builder();
    if cfg.accept_invalid_certs {
        b.danger_accept_invalid_certs(true).danger_accept_invalid_hostnames(true);
    }
    let c = b.build().map_err(|e| AppError::new(ErrorCode::FtpConnect, e.to_string()))?;
    Ok(NativeTlsConnector::from(c))
}

pub fn connect(cfg: &RemoteConfig) -> AppResult<FtpConn> {
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
        Protocol::FtpsExplicit => FtpConn::connect_timeout(addr, timeout)?.into_secure(tls(cfg)?, host)?,
        Protocol::FtpsImplicit => FtpConn::connect_secure_implicit(addr, tls(cfg)?, host)?,
        _ => FtpConn::connect_timeout(addr, timeout)?,
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
    Entry { name, flags: fl, size: if f.is_directory() { 0 } else { f.size() as u64 }, mtime: system_time_ms(f.modified()), attrs: 0 }
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

fn is_connection_lost(e: &AppError) -> bool {
    e.code == ErrorCode::FtpConnect || e.message.starts_with("421") || e.message.contains("Broken pipe") || e.message.contains("connection reset")
}

// ---------------------------------------------------------------------------
// File system
// ---------------------------------------------------------------------------

/// suppaftp's transfer streams must be finished explicitly to read the
/// server's completion reply.
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

/// One FTP control connection (reconnects once when the server dropped it).
pub struct FtpRemote {
    cfg: RemoteConfig,
    conn: Mutex<Option<FtpConn>>,
    mlsd: Mutex<Option<bool>>,
}

impl FtpRemote {
    /// Connects; returns the file system, the server's current folder and greeting.
    pub fn connect(cfg: RemoteConfig) -> AppResult<(Self, String, Option<String>)> {
        let mut conn = connect(&cfg)?;
        let welcome = conn.get_welcome_msg().map(|s| s.to_string());
        let pwd = conn.pwd().unwrap_or_else(|_| "/".into());
        Ok((Self { cfg, conn: Mutex::new(Some(conn)), mlsd: Mutex::new(None) }, pwd, welcome))
    }

    fn run<T>(&self, mut f: impl FnMut(&mut FtpConn, bool) -> AppResult<T>) -> AppResult<T> {
        let mut guard = self.conn.lock();
        for attempt in 0..2 {
            if guard.is_none() {
                *guard = Some(connect(&self.cfg)?);
            }
            let c = guard.as_mut().expect("connected above");
            let mlsd = *self.mlsd.lock().get_or_insert_with(|| supports_mlsd(c));
            match f(c, mlsd) {
                Err(e) if attempt == 0 && is_connection_lost(&e) => *guard = None,
                other => return other,
            }
        }
        Err(AppError::new(ErrorCode::FtpConnect, "Connection lost"))
    }
}

impl Drop for FtpRemote {
    fn drop(&mut self) {
        if let Some(mut c) = self.conn.lock().take() {
            let _ = c.quit();
        }
    }
}

impl RemoteFs for FtpRemote {
    fn list(&self, path: &str) -> AppResult<Vec<Entry>> {
        self.run(|c, mlsd| list_dir(c, path, mlsd))
    }
    fn mkdir(&self, path: &str) -> AppResult<()> {
        self.run(|c, _| c.mkdir(path).map_err(AppError::from))
    }
    fn remove_file(&self, path: &str) -> AppResult<()> {
        self.run(|c, _| c.rm(path).map_err(AppError::from))
    }
    fn remove_dir(&self, path: &str) -> AppResult<()> {
        self.run(|c, _| c.rmdir(path).map_err(AppError::from))
    }
    fn rename(&self, from: &str, to: &str) -> AppResult<()> {
        self.run(|c, _| c.rename(from, to).map_err(AppError::from))
    }
    fn reader(&self, path: &str) -> AppResult<Box<dyn ReadStream + '_>> {
        let s = self.run(|c, _| c.retr_as_stream(path).map_err(AppError::from))?;
        Ok(Box::new(FtpStreamWrap(s)))
    }
    fn writer(&self, path: &str) -> AppResult<Box<dyn WriteStream + '_>> {
        let s = self.run(|c, _| c.put_with_stream(path).map_err(AppError::from))?;
        Ok(Box::new(FtpStreamWrap(s)))
    }
    fn set_mtime(&self, path: &str, mtime: i64) {
        let (y, mo, d, h, mi, s) = crate::util::utc_parts(mtime);
        let cmd = format!("MFMT {y:04}{mo:02}{d:02}{h:02}{mi:02}{s:02} {path}");
        let _ = self.run(|c, _| c.custom_command(cmd.clone(), &[suppaftp::Status::File]).map(|_| ()).map_err(AppError::from));
    }
    fn keepalive(&self) {
        if let Some(mut guard) = self.conn.try_lock() {
            if let Some(c) = guard.as_mut() {
                if c.noop().is_err() {
                    *guard = None;
                }
            }
        }
    }
    fn close(&self) {
        if let Some(mut c) = self.conn.lock().take() {
            let _ = c.quit();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn cp1250_roundtrip() {
        use suppaftp::Encoding;
        let enc = RsEncoding(encoding_rs::WINDOWS_1250);
        let bytes = enc.encode("Příliš žluťoučký kůň.txt").unwrap();
        assert_eq!(enc.decode(&bytes), "Příliš žluťoučký kůň.txt");
    }
}
