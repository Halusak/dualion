//! Android devices through the ADB server (USB or Wi-Fi debugging). Files
//! move over ADB's sync protocol (like `adb push/pull`): exact sizes, times
//! and modes, streamed both ways. Folder operations run shell commands.

use super::{entry, RemoteConfig, RemoteFs, Source};
use crate::error::{AppError, AppResult, ErrorCode};
use crate::jobs::vfs::{Meta, ReadStream, WriteStream};
use crate::location::Entry;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::time::Duration;

const SERVER: &str = "127.0.0.1:5037";
const S_IFMT: u32 = 0o170000;
const S_IFDIR: u32 = 0o040000;
const S_IFLNK: u32 = 0o120000;

fn adb_err(msg: impl Into<String>) -> AppError {
    let msg = msg.into();
    let lower = msg.to_lowercase();
    let code = if lower.contains("no such file") || lower.contains("not found") {
        ErrorCode::NotFound
    } else if lower.contains("permission denied") || lower.contains("read-only") {
        ErrorCode::PermissionDenied
    } else if lower.contains("unauthorized") || lower.contains("offline") {
        ErrorCode::FtpAuth
    } else {
        ErrorCode::Io
    };
    AppError::new(code, msg)
}

fn io(e: std::io::Error) -> AppError {
    AppError::new(ErrorCode::FtpConnect, format!("ADB: {e}"))
}

/// Connects to the ADB server, starting it when it is not running.
fn server() -> AppResult<TcpStream> {
    let addr = SERVER.parse().expect("valid address");
    let s = match TcpStream::connect_timeout(&addr, Duration::from_secs(2)) {
        Ok(s) => s,
        Err(_) => {
            let adb = super::tools::adb()?;
            let mut c = super::tools::command(&adb);
            c.arg("start-server");
            super::tools::run(c, Duration::from_secs(20))?;
            TcpStream::connect_timeout(&addr, Duration::from_secs(5)).map_err(io)?
        }
    };
    let _ = s.set_read_timeout(Some(Duration::from_secs(60)));
    let _ = s.set_write_timeout(Some(Duration::from_secs(60)));
    Ok(s)
}

fn request(s: &mut TcpStream, req: &str) -> AppResult<()> {
    s.write_all(format!("{:04x}{req}", req.len()).as_bytes()).map_err(io)?;
    let mut status = [0u8; 4];
    s.read_exact(&mut status).map_err(io)?;
    if &status == b"OKAY" {
        return Ok(());
    }
    Err(adb_err(read_hex_string(s).unwrap_or_else(|_| "ADB request failed".into())))
}

fn read_hex_string(s: &mut TcpStream) -> AppResult<String> {
    let mut len = [0u8; 4];
    s.read_exact(&mut len).map_err(io)?;
    let n = usize::from_str_radix(std::str::from_utf8(&len).unwrap_or("0"), 16).unwrap_or(0);
    let mut buf = vec![0u8; n];
    s.read_exact(&mut buf).map_err(io)?;
    Ok(String::from_utf8_lossy(&buf).into_owned())
}

/// Device list (`adb devices -l`), ready devices first.
pub fn devices() -> AppResult<Vec<Source>> {
    // Do not start the ADB server just to look: only when it is already running or adb is installed.
    if super::tools::adb().is_err() && TcpStream::connect_timeout(&SERVER.parse().expect("addr"), Duration::from_millis(300)).is_err() {
        return Ok(Vec::new());
    }
    let mut s = server()?;
    request(&mut s, "host:devices-l")?;
    Ok(parse_devices(&read_hex_string(&mut s)?))
}

fn parse_devices(text: &str) -> Vec<Source> {
    text.lines()
        .filter_map(|l| {
            let mut parts = l.split_whitespace();
            let serial = parts.next()?.to_string();
            let state = parts.next()?.to_string();
            let mut model = String::new();
            for p in parts {
                if let Some(m) = p.strip_prefix("model:") {
                    model = m.replace('_', " ");
                }
            }
            let name = if model.is_empty() { serial.clone() } else { model };
            let detail = if state == "device" { serial.clone() } else { format!("{serial} · {state}") };
            Some(Source { protocol: super::Protocol::Adb, id: serial, name, detail })
        })
        .collect()
}

/// Quotes an argument for the device shell.
fn sh_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', r"'\''"))
}

pub struct Adb {
    serial: String,
    /// Device supports 64-bit stat / list (Android 10+).
    v2: bool,
}

/// A sync-protocol connection to one device.
struct SyncConn(TcpStream);

impl SyncConn {
    fn send(&mut self, id: &[u8; 4], data: &[u8]) -> AppResult<()> {
        let mut buf = Vec::with_capacity(8 + data.len());
        buf.extend_from_slice(id);
        buf.extend_from_slice(&(data.len() as u32).to_le_bytes());
        buf.extend_from_slice(data);
        self.0.write_all(&buf).map_err(io)
    }
    fn header(&mut self) -> AppResult<([u8; 4], u32)> {
        let mut h = [0u8; 8];
        self.0.read_exact(&mut h).map_err(io)?;
        Ok(([h[0], h[1], h[2], h[3]], u32::from_le_bytes([h[4], h[5], h[6], h[7]])))
    }
    fn bytes(&mut self, n: usize) -> AppResult<Vec<u8>> {
        let mut b = vec![0u8; n];
        self.0.read_exact(&mut b).map_err(io)?;
        Ok(b)
    }
    fn fail(&mut self, len: u32) -> AppError {
        adb_err(self.bytes(len as usize).map(|b| String::from_utf8_lossy(&b).into_owned()).unwrap_or_else(|_| "ADB sync failed".into()))
    }
}

fn u32le(b: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(b[at..at + 4].try_into().expect("4 bytes"))
}
fn u64le(b: &[u8], at: usize) -> u64 {
    u64::from_le_bytes(b[at..at + 8].try_into().expect("8 bytes"))
}

impl Adb {
    pub fn connect(cfg: &RemoteConfig) -> AppResult<Self> {
        let serial = cfg.host.trim().to_string();
        let mut s = server()?;
        request(&mut s, &format!("host-serial:{serial}:features"))?;
        let features = read_hex_string(&mut s)?;
        let v2 = features.split(',').any(|f| f == "stat_v2") && features.split(',').any(|f| f == "ls_v2");
        let adb = Self { serial, v2 };
        adb.shell("echo ok")?;
        Ok(adb)
    }

    /// Internal storage of the device user.
    pub fn home(&self) -> String {
        "/sdcard".into()
    }

    fn transport(&self) -> AppResult<TcpStream> {
        let mut s = server()?;
        request(&mut s, &format!("host:transport:{}", self.serial))?;
        Ok(s)
    }

    fn sync(&self) -> AppResult<SyncConn> {
        let mut s = self.transport()?;
        request(&mut s, "sync:")?;
        Ok(SyncConn(s))
    }

    /// Runs a shell command; fails with its output when it exits non-zero.
    fn shell(&self, cmd: &str) -> AppResult<String> {
        let mut s = self.transport()?;
        request(&mut s, &format!("shell:{cmd} 2>&1; echo \"<$?>\""))?;
        let mut out = Vec::new();
        s.read_to_end(&mut out).map_err(io)?;
        let text = String::from_utf8_lossy(&out).replace("\r\n", "\n");
        let text = text.trim_end();
        let (body, code) = match text.rfind('<') {
            Some(i) if text.ends_with('>') => (&text[..i], text[i + 1..text.len() - 1].trim().parse::<i32>().unwrap_or(1)),
            _ => (text, 0),
        };
        if code != 0 {
            return Err(adb_err(body.trim().lines().last().unwrap_or("Command failed").to_string()));
        }
        Ok(body.to_string())
    }

    fn stat_raw(&self, path: &str) -> AppResult<Option<(u32, u64, i64)>> {
        let mut s = self.sync()?;
        if self.v2 {
            s.send(b"STA2", path.as_bytes())?;
            let (id, _) = s.header()?;
            if &id != b"STA2" {
                return Err(adb_err("Unexpected ADB reply"));
            }
            // id + error were read as the header; then dev ino mode nlink uid gid size atime mtime ctime.
            let rest = s.bytes(64)?;
            let mode = u32le(&rest, 16);
            if mode == 0 {
                return Ok(None);
            }
            let size = u64le(&rest, 32);
            let mtime = u64le(&rest, 48) as i64;
            Ok(Some((mode, size, mtime.saturating_mul(1000))))
        } else {
            s.send(b"STAT", path.as_bytes())?;
            let b = s.bytes(16)?;
            let mode = u32le(&b, 4);
            if mode == 0 {
                return Ok(None);
            }
            Ok(Some((mode, u32le(&b, 8) as u64, u32le(&b, 12) as i64 * 1000)))
        }
    }

    fn is_dir_target(&self, path: &str) -> bool {
        self.stat_raw(&format!("{}/", path.trim_end_matches('/'))).ok().flatten().is_some_and(|(m, _, _)| m & S_IFMT == S_IFDIR)
    }
}

impl RemoteFs for Adb {
    fn list(&self, path: &str) -> AppResult<Vec<Entry>> {
        let mut s = self.sync()?;
        let dir = format!("{}/", path.trim_end_matches('/'));
        let mut out = Vec::new();
        let mut links = Vec::new();
        if self.v2 {
            s.send(b"LIS2", dir.as_bytes())?;
            loop {
                let (id, _err) = s.header()?;
                match &id {
                    b"DNT2" => {
                        // dev ino mode nlink uid gid size atime mtime ctime namelen
                        let b = s.bytes(68)?;
                        let mode = u32le(&b, 16);
                        let size = u64le(&b, 32);
                        let mtime = (u64le(&b, 48) as i64).saturating_mul(1000);
                        let name = String::from_utf8_lossy(&s.bytes(u32le(&b, 64) as usize)?).into_owned();
                        if name != "." && name != ".." {
                            push(&mut out, &mut links, name, mode, size, mtime);
                        }
                    }
                    b"DONE" => {
                        s.bytes(68)?;
                        break;
                    }
                    b"FAIL" => return Err(s.fail(_err)),
                    _ => return Err(adb_err("Unexpected ADB reply")),
                }
            }
        } else {
            s.send(b"LIST", dir.as_bytes())?;
            loop {
                let (id, mode) = s.header()?;
                let b = s.bytes(12)?;
                match &id {
                    b"DENT" => {
                        let size = u32le(&b, 0) as u64;
                        let mtime = u32le(&b, 4) as i64 * 1000;
                        let name = String::from_utf8_lossy(&s.bytes(u32le(&b, 8) as usize)?).into_owned();
                        if name != "." && name != ".." {
                            push(&mut out, &mut links, name, mode, size, mtime);
                        }
                    }
                    b"DONE" => break,
                    _ => return Err(adb_err("Unexpected ADB reply")),
                }
            }
        }
        // Links to folders (e.g. /sdcard) can be entered.
        for i in links {
            if self.is_dir_target(&super::join(path, &out[i].name)) {
                out[i].flags |= crate::location::flags::DIR;
                out[i].size = 0;
            }
        }
        if out.is_empty() && self.stat_raw(&dir)?.is_none() {
            return Err(AppError::new(ErrorCode::NotFound, "No such folder").with_path(path));
        }
        Ok(out)
    }
    fn stat(&self, path: &str) -> AppResult<Option<Meta>> {
        Ok(self.stat_raw(path)?.map(|(mode, size, mtime)| {
            let link = mode & S_IFMT == S_IFLNK;
            let is_dir = mode & S_IFMT == S_IFDIR || (link && self.is_dir_target(path));
            Meta { is_dir, is_link: link, size: if is_dir { 0 } else { size }, mtime }
        }))
    }
    fn mkdir(&self, path: &str) -> AppResult<()> {
        self.shell(&format!("mkdir {}", sh_quote(path))).map(|_| ())
    }
    fn remove_file(&self, path: &str) -> AppResult<()> {
        self.shell(&format!("rm -f {}", sh_quote(path))).map(|_| ())
    }
    fn remove_dir(&self, path: &str) -> AppResult<()> {
        self.shell(&format!("rmdir {}", sh_quote(path))).map(|_| ())
    }
    fn rename(&self, from: &str, to: &str) -> AppResult<()> {
        self.shell(&format!("mv -n {} {}", sh_quote(from), sh_quote(to))).map(|_| ())
    }
    fn reader(&self, path: &str) -> AppResult<Box<dyn ReadStream + '_>> {
        let mut s = self.sync()?;
        s.send(b"RECV", path.as_bytes())?;
        Ok(Box::new(Recv { s, left: 0, done: false }))
    }
    fn writer(&self, path: &str) -> AppResult<Box<dyn WriteStream + '_>> {
        let mut s = self.sync()?;
        s.send(b"SEND", format!("{path},{}", 0o100644).as_bytes())?;
        Ok(Box::new(SendStream { s, buf: Vec::with_capacity(64 * 1024) }))
    }
    fn set_mtime(&self, path: &str, mtime: i64) {
        // Already set by SEND; folders get it via touch.
        let (y, mo, d, h, mi, s) = crate::util::utc_parts(mtime);
        let _ = self.shell(&format!("TZ=UTC touch -m -t {y:04}{mo:02}{d:02}{h:02}{mi:02}.{s:02} {}", sh_quote(path)));
    }
}

fn push(out: &mut Vec<Entry>, links: &mut Vec<usize>, name: String, mode: u32, size: u64, mtime: i64) {
    let kind = mode & S_IFMT;
    if kind == S_IFLNK {
        links.push(out.len());
    }
    out.push(entry(name, kind == S_IFDIR, size, mtime, mode, kind == S_IFLNK));
}

/// RECV stream: DATA chunks until DONE.
struct Recv {
    s: SyncConn,
    left: usize,
    done: bool,
}

impl Read for Recv {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        while self.left == 0 {
            if self.done {
                return Ok(0);
            }
            let (id, len) = self.s.header().map_err(|e| std::io::Error::other(e.message))?;
            match &id {
                b"DATA" => self.left = len as usize,
                b"DONE" => {
                    self.done = true;
                    return Ok(0);
                }
                b"FAIL" => return Err(std::io::Error::other(self.s.fail(len).message)),
                _ => return Err(std::io::Error::other("Unexpected ADB reply")),
            }
        }
        let n = buf.len().min(self.left);
        self.s.0.read_exact(&mut buf[..n])?;
        self.left -= n;
        Ok(n)
    }
}

impl ReadStream for Recv {
    fn finish(mut self: Box<Self>) -> AppResult<()> {
        // Drain what is left so the device is not left mid-transfer.
        let mut sink = [0u8; 64 * 1024];
        while self.read(&mut sink).map_err(|e| AppError::new(ErrorCode::Io, e.to_string()))? > 0 {}
        Ok(())
    }
}

/// SEND stream: 64 KiB DATA chunks, then DONE with the mtime.
struct SendStream {
    s: SyncConn,
    buf: Vec<u8>,
}

impl SendStream {
    fn flush_chunk(&mut self) -> std::io::Result<()> {
        if !self.buf.is_empty() {
            let data = std::mem::take(&mut self.buf);
            self.s.send(b"DATA", &data).map_err(|e| std::io::Error::other(e.message))?;
        }
        Ok(())
    }
}

impl Write for SendStream {
    fn write(&mut self, data: &[u8]) -> std::io::Result<usize> {
        let room = 64 * 1024 - self.buf.len();
        let n = data.len().min(room);
        self.buf.extend_from_slice(&data[..n]);
        if self.buf.len() == 64 * 1024 {
            self.flush_chunk()?;
        }
        Ok(n)
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl WriteStream for SendStream {
    fn finish(mut self: Box<Self>) -> AppResult<()> {
        self.flush_chunk().map_err(|e| AppError::new(ErrorCode::Io, e.to_string()))?;
        let now = (crate::util::now_ms() / 1000) as u32;
        let mut done = Vec::new();
        done.extend_from_slice(b"DONE");
        done.extend_from_slice(&now.to_le_bytes());
        self.s.0.write_all(&done).map_err(io)?;
        let (id, len) = self.s.header()?;
        match &id {
            b"OKAY" => Ok(()),
            b"FAIL" => Err(self.s.fail(len)),
            _ => Err(adb_err("Unexpected ADB reply")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_devices() {
        let text = "emulator-5554          device product:sdk_gphone64_arm64 model:sdk_gphone64_arm64 device:emu64a transport_id:1\n\
                    R5CT12345      unauthorized usb:1-1 transport_id:2\n";
        let d = parse_devices(text);
        assert_eq!(d.len(), 2);
        assert_eq!((d[0].id.as_str(), d[0].name.as_str()), ("emulator-5554", "sdk gphone64 arm64"));
        assert_eq!(d[1].detail, "R5CT12345 · unauthorized");
    }

    #[test]
    fn quotes_for_shell() {
        assert_eq!(sh_quote("/sdcard/it's here"), r"'/sdcard/it'\''s here'");
    }

    /// Needs a device or a running emulator:
    /// `DUALION_ADB_SERIAL=emulator-5554 cargo test adb_live -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn adb_live() {
        let serial = std::env::var("DUALION_ADB_SERIAL").expect("DUALION_ADB_SERIAL");
        println!("{:?}", devices().unwrap());
        let a = Adb::connect(&super::super::source_config(super::super::Protocol::Adb, &serial, "test")).unwrap();
        println!("v2: {}", a.v2);
        let dir = "/sdcard/Download/dualion-test";
        let _ = a.shell(&format!("rm -rf {dir}"));
        a.mkdir(dir).unwrap();
        let data: Vec<u8> = (0..300_000u32).map(|i| (i % 251) as u8).collect();
        let mut w = a.writer(&format!("{dir}/a b.bin")).unwrap();
        w.write_all(&data).unwrap();
        w.finish().unwrap();
        let list = a.list(dir).unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!((list[0].name.as_str(), list[0].size), ("a b.bin", 300_000));
        let mut r = a.reader(&format!("{dir}/a b.bin")).unwrap();
        let mut back = Vec::new();
        r.read_to_end(&mut back).unwrap();
        r.finish().unwrap();
        assert_eq!(back, data);
        a.rename(&format!("{dir}/a b.bin"), &format!("{dir}/c.bin")).unwrap();
        assert!(a.stat(&format!("{dir}/c.bin")).unwrap().is_some());
        assert!(a.stat(&format!("{dir}/a b.bin")).unwrap().is_none());
        assert!(a.list("/").unwrap().iter().any(|e| e.name == "sdcard" && e.is_dir()));
        a.remove_file(&format!("{dir}/c.bin")).unwrap();
        a.remove_dir(dir).unwrap();
        assert!(a.list(dir).is_err());
    }
}
