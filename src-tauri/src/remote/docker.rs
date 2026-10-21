//! Files inside Docker (or Podman) containers through the CLI: listings via
//! `exec … stat` (works with busybox and coreutils), transfers via `cp`
//! with tar streams. Containers without a shell can only be copied from.

use super::{entry, tools, RemoteConfig, RemoteFs, Source, SpoolWriter};
use crate::error::{AppError, AppResult, ErrorCode};
use crate::jobs::vfs::{Meta, ReadStream, WriteStream};
use crate::location::Entry;
use std::io::Read;
use std::path::PathBuf;
use std::process::{Child, Stdio};
use std::time::Duration;

const LIST: &str = r#"cd -- "$1" || exit 2
for f in .* *; do
  case "$f" in .|..) continue;; esac
  [ -e "$f" ] || [ -L "$f" ] || continue
  l=0; [ -L "$f" ] && l=1
  stat -L -c "$l|%s|%Y|%f|%n" -- "$f" 2>/dev/null || stat -c "$l|%s|%Y|%f|%n" -- "$f"
done"#;

const STAT: &str = r#"[ -e "$1" ] || [ -L "$1" ] || exit 0
l=0; [ -L "$1" ] && l=1
stat -L -c "$l|%s|%Y|%f|%n" -- "$1" 2>/dev/null || stat -c "$l|%s|%Y|%f|%n" -- "$1""#;

/// Running containers.
pub fn containers() -> AppResult<Vec<Source>> {
    let Ok(bin) = tools::docker() else { return Ok(Vec::new()) };
    let mut c = tools::command(&bin);
    c.args(["ps", "--format", "{{.ID}}\t{{.Names}}\t{{.Image}}"]);
    let out = tools::run(c, Duration::from_secs(6))?;
    Ok(String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter_map(|l| {
            let mut p = l.split('\t');
            let id = p.next()?.trim().to_string();
            let name = p.next().unwrap_or(&id).trim().to_string();
            let image = p.next().unwrap_or("").trim().to_string();
            (!id.is_empty()).then_some(Source { protocol: super::Protocol::Docker, id, name, detail: image })
        })
        .collect())
}

/// "l|size|mtime|hexmode|name" lines → entries.
fn parse_stat(text: &str) -> Vec<Entry> {
    text.lines()
        .filter_map(|l| {
            let mut p = l.splitn(5, '|');
            let link = p.next()? == "1";
            let size: u64 = p.next()?.parse().ok()?;
            let mtime: i64 = p.next()?.parse().ok()?;
            let mode = u32::from_str_radix(p.next()?, 16).ok()?;
            let name = p.next()?.to_string();
            let is_dir = mode & 0o170000 == 0o040000;
            Some(entry(name, is_dir, size, mtime * 1000, mode, link))
        })
        .collect()
}

pub struct Docker {
    bin: PathBuf,
    id: String,
}

impl Docker {
    pub fn connect(cfg: &RemoteConfig) -> AppResult<Self> {
        let d = Self { bin: tools::docker()?, id: cfg.host.trim().to_string() };
        d.exec(&["true"])?;
        Ok(d)
    }

    fn exec(&self, args: &[&str]) -> AppResult<String> {
        let mut c = tools::command(&self.bin);
        c.args(["exec", &self.id]).args(args);
        let out = tools::run(c, Duration::from_secs(60))?;
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    }

    fn script(&self, script: &str, arg: &str) -> AppResult<String> {
        self.exec(&["sh", "-c", script, "sh", arg])
    }
}

impl RemoteFs for Docker {
    fn list(&self, path: &str) -> AppResult<Vec<Entry>> {
        self.script(LIST, path).map(|t| parse_stat(&t)).map_err(|e| if e.message.contains("can't cd") || e.message.contains("No such") { AppError::new(ErrorCode::NotFound, e.message) } else { e })
    }
    fn stat(&self, path: &str) -> AppResult<Option<Meta>> {
        Ok(parse_stat(&self.script(STAT, path)?).first().map(super::meta_of))
    }
    fn mkdir(&self, path: &str) -> AppResult<()> {
        self.exec(&["mkdir", "--", path]).map(|_| ())
    }
    fn remove_file(&self, path: &str) -> AppResult<()> {
        self.exec(&["rm", "-f", "--", path]).map(|_| ())
    }
    fn remove_dir(&self, path: &str) -> AppResult<()> {
        self.exec(&["rmdir", "--", path]).map(|_| ())
    }
    fn rename(&self, from: &str, to: &str) -> AppResult<()> {
        self.exec(&["mv", "--", from, to]).map(|_| ())
    }
    fn reader(&self, path: &str) -> AppResult<Box<dyn ReadStream + '_>> {
        let mut c = tools::command(&self.bin);
        c.args(["cp", &format!("{}:{path}", self.id), "-"]);
        let mut child = c.spawn().map_err(|e| AppError::other(e.to_string()))?;
        let stdout = child.stdout.take().ok_or_else(|| AppError::other("No output"))?;
        Ok(Box::new(TarFile { child, out: stdout, left: 0, started: false }))
    }
    fn writer(&self, path: &str) -> AppResult<Box<dyn WriteStream + '_>> {
        let dir = super::parent(path).unwrap_or_else(|| "/".into());
        let name = super::file_name(path).to_string();
        Ok(Box::new(SpoolWriter::new(move |mut file, size| {
            let mut c = tools::command(&self.bin);
            c.stdin(Stdio::piped()).args(["cp", "-", &format!("{}:{dir}", self.id)]);
            let mut child = c.spawn().map_err(|e| AppError::other(e.to_string()))?;
            let stdin = child.stdin.take().ok_or_else(|| AppError::other("No input"))?;
            let mut tar = tar::Builder::new(stdin);
            let mut h = tar::Header::new_gnu();
            h.set_size(size);
            h.set_mode(0o644);
            h.set_mtime((crate::util::now_ms() / 1000) as u64);
            h.set_entry_type(tar::EntryType::Regular);
            tar.append_data(&mut h, &name, &mut file).map_err(|e| AppError::new(ErrorCode::Io, e.to_string()))?;
            drop(tar.into_inner().map_err(|e| AppError::new(ErrorCode::Io, e.to_string()))?);
            wait(child)
        })?))
    }
}

fn wait(child: Child) -> AppResult<()> {
    let out = child.wait_with_output().map_err(|e| AppError::other(e.to_string()))?;
    if out.status.success() {
        Ok(())
    } else {
        Err(tools::tool_error(&out.stderr))
    }
}

/// The one file of `docker cp … -` (a tar stream).
struct TarFile {
    child: Child,
    out: std::process::ChildStdout,
    left: u64,
    started: bool,
}

fn octal(field: &[u8]) -> u64 {
    // Sizes over 8 GiB use base-256 (high bit set).
    if field[0] & 0x80 != 0 {
        return field[1..].iter().fold(0u64, |a, b| (a << 8) | *b as u64);
    }
    let s = String::from_utf8_lossy(field);
    u64::from_str_radix(s.trim_matches(|c: char| c == '\0' || c == ' '), 8).unwrap_or(0)
}

impl TarFile {
    /// Skips pax / long-name headers up to the file itself.
    fn start(&mut self) -> std::io::Result<()> {
        loop {
            let mut h = [0u8; 512];
            self.out.read_exact(&mut h)?;
            let size = octal(&h[124..136]);
            match h[156] {
                b'x' | b'g' | b'L' | b'K' => {
                    let skip = size.div_ceil(512) * 512;
                    std::io::copy(&mut (&mut self.out).take(skip), &mut std::io::sink())?;
                }
                b'0' | 0 => {
                    self.left = size;
                    return Ok(());
                }
                b'5' => return Err(std::io::Error::other("This is a folder")),
                _ => return Err(std::io::Error::other("Unsupported file type")),
            }
        }
    }
}

impl Read for TarFile {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        if !self.started {
            self.started = true;
            self.start()?;
        }
        if self.left == 0 {
            return Ok(0);
        }
        let n = (buf.len() as u64).min(self.left) as usize;
        let n = self.out.read(&mut buf[..n])?;
        if n == 0 {
            return Err(std::io::Error::new(std::io::ErrorKind::UnexpectedEof, "Transfer interrupted"));
        }
        self.left -= n as u64;
        Ok(n)
    }
}

impl ReadStream for TarFile {
    fn finish(mut self: Box<Self>) -> AppResult<()> {
        let _ = std::io::copy(&mut self.out, &mut std::io::sink());
        let status = self.child.wait().map_err(|e| AppError::other(e.to_string()))?;
        if status.success() {
            Ok(())
        } else {
            let mut err = Vec::new();
            if let Some(mut e) = self.child.stderr.take() {
                let _ = e.read_to_end(&mut err);
            }
            Err(tools::tool_error(&err))
        }
    }
}

impl Drop for TarFile {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_stat_lines() {
        let text = "0|4096|1700000000|41ed|etc\n1|12|1700000001|a1ff|lib|x\n0|5|1700000002|81ed|run.sh\n";
        let e = parse_stat(text);
        assert_eq!(e.len(), 3);
        assert!(e[0].is_dir());
        assert_eq!(e[1].name, "lib|x");
        assert!(e[1].flags & crate::location::flags::SYMLINK != 0);
        assert_eq!((e[2].size, e[2].mtime), (5, 1_700_000_002_000));
        assert!(e[2].flags & crate::location::flags::EXEC != 0);
    }

    #[test]
    fn reads_single_file_from_tar() {
        let mut b = tar::Builder::new(Vec::new());
        let mut h = tar::Header::new_gnu();
        h.set_size(5);
        h.set_mode(0o644);
        h.set_entry_type(tar::EntryType::Regular);
        b.append_data(&mut h, "a-very-long-name-".repeat(10), &b"hello"[..]).unwrap();
        let data = b.into_inner().unwrap();
        // Parse the same way TarFile does, on an in-memory stream.
        let mut cur = std::io::Cursor::new(data);
        let size;
        loop {
            let mut h = [0u8; 512];
            cur.read_exact(&mut h).unwrap();
            let s = octal(&h[124..136]);
            if h[156] == b'L' {
                cur.set_position(cur.position() + s.div_ceil(512) * 512);
                continue;
            }
            size = s;
            break;
        }
        let mut out = vec![0u8; size as usize];
        cur.read_exact(&mut out).unwrap();
        assert_eq!(out, b"hello");
    }
}
