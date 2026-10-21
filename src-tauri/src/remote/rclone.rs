//! Every remote configured in rclone (Google Drive, OneDrive, Dropbox, Mega,
//! pCloud, Box, S3, SFTP… 70+ services) through the rclone CLI with the
//! user's own configuration. Reads and writes stream via `cat` / `rcat`.

use super::{entry, tools, RemoteConfig, RemoteFs, Source};
use crate::error::{AppError, AppResult, ErrorCode};
use crate::jobs::vfs::{Meta, ReadStream, WriteStream};
use crate::location::Entry;
use std::io::{Read, Write};
use std::path::PathBuf;
use std::process::{Child, Stdio};
use std::time::Duration;

/// Remotes from the user's rclone.conf.
pub fn remotes() -> AppResult<Vec<Source>> {
    let Ok(bin) = tools::rclone() else { return Ok(Vec::new()) };
    let mut c = tools::command(&bin);
    c.args(["listremotes", "--long"]);
    let out = tools::run(c, Duration::from_secs(10))?;
    Ok(parse_remotes(&String::from_utf8_lossy(&out.stdout)))
}

fn parse_remotes(text: &str) -> Vec<Source> {
    text.lines()
        .filter_map(|l| {
            let (name, kind) = l.split_once(':')?;
            let name = name.trim().to_string();
            (!name.is_empty()).then(|| Source { protocol: super::Protocol::Rclone, id: name.clone(), name, detail: kind.trim().to_string() })
        })
        .collect()
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "PascalCase")]
struct Item {
    name: String,
    #[serde(default)]
    size: i64,
    #[serde(default)]
    mod_time: String,
    #[serde(default)]
    is_dir: bool,
}

fn to_entry(i: Item) -> Entry {
    entry(i.name, i.is_dir, i.size.max(0) as u64, rfc3339(&i.mod_time).unwrap_or(0), 0, false)
}

/// "2017-05-31T16:15:57.034468261+01:00" / "…Z" → ms since epoch (UTC).
pub fn rfc3339(s: &str) -> Option<i64> {
    let (date, time) = s.split_once('T')?;
    let mut d = date.split('-').map(|x| x.parse::<i64>().ok());
    let (y, mo, day) = (d.next()??, d.next()??, d.next()??);
    let (clock, offset) = match time.find(['Z', 'z', '+', '-']) {
        Some(i) => (&time[..i], &time[i..]),
        None => (time, ""),
    };
    let mut c = clock.split(':');
    let h: i64 = c.next()?.parse().ok()?;
    let mi: i64 = c.next()?.parse().ok()?;
    let sec: f64 = c.next().unwrap_or("0").parse().ok()?;
    let off_min = match offset.as_bytes().first() {
        Some(b'+') | Some(b'-') => {
            let sign = if offset.starts_with('-') { -1 } else { 1 };
            let mut p = offset[1..].split(':');
            sign * (p.next()?.parse::<i64>().ok()? * 60 + p.next().unwrap_or("0").parse::<i64>().ok()?)
        }
        _ => 0,
    };
    let days = crate::util::days_from_civil(y as i32, mo as u32, day as u32);
    Some((days * 86400 + h * 3600 + mi * 60 - off_min * 60) * 1000 + (sec * 1000.0) as i64)
}

pub struct Rclone {
    bin: PathBuf,
    remote: String,
}

impl Rclone {
    pub fn connect(cfg: &RemoteConfig) -> AppResult<Self> {
        let r = Self { bin: tools::rclone()?, remote: cfg.host.trim().trim_end_matches(':').to_string() };
        r.list("/")?;
        Ok(r)
    }

    /// "remote:path" for a panel path.
    fn target(&self, path: &str) -> String {
        format!("{}:{}", self.remote, path.trim_start_matches('/'))
    }

    fn run(&self, args: &[&str], timeout: u64) -> AppResult<Vec<u8>> {
        let mut c = tools::command(&self.bin);
        c.args(args);
        tools::run(c, Duration::from_secs(timeout)).map(|o| o.stdout)
    }
}

impl RemoteFs for Rclone {
    fn list(&self, path: &str) -> AppResult<Vec<Entry>> {
        let out = self.run(&["lsjson", "--no-mimetype", &self.target(path)], 300)?;
        let items: Vec<Item> = serde_json::from_slice(&out).map_err(|e| AppError::other(e.to_string()))?;
        Ok(items.into_iter().map(to_entry).collect())
    }
    fn stat(&self, path: &str) -> AppResult<Option<Meta>> {
        if path.trim_matches('/').is_empty() {
            return Ok(Some(super::dir_meta()));
        }
        match self.run(&["lsjson", "--stat", "--no-mimetype", &self.target(path)], 120) {
            Ok(out) => {
                let item: Item = serde_json::from_slice(&out).map_err(|e| AppError::other(e.to_string()))?;
                Ok(Some(super::meta_of(&to_entry(item))))
            }
            Err(e) if e.code == ErrorCode::NotFound => Ok(None),
            Err(e) => Err(e),
        }
    }
    fn mkdir(&self, path: &str) -> AppResult<()> {
        self.run(&["mkdir", &self.target(path)], 120).map(|_| ())
    }
    fn remove_file(&self, path: &str) -> AppResult<()> {
        self.run(&["deletefile", &self.target(path)], 120).map(|_| ())
    }
    fn remove_dir(&self, path: &str) -> AppResult<()> {
        self.run(&["rmdir", &self.target(path)], 120).map(|_| ())
    }
    fn rename(&self, from: &str, to: &str) -> AppResult<()> {
        self.run(&["moveto", &self.target(from), &self.target(to)], 3600).map(|_| ())
    }
    fn reader(&self, path: &str) -> AppResult<Box<dyn ReadStream + '_>> {
        let mut c = tools::command(&self.bin);
        c.args(["cat", &self.target(path)]);
        let mut child = c.spawn().map_err(|e| AppError::other(e.to_string()))?;
        let out = child.stdout.take().ok_or_else(|| AppError::other("No output"))?;
        Ok(Box::new(ChildReader { child: Some(child), out }))
    }
    fn writer(&self, path: &str) -> AppResult<Box<dyn WriteStream + '_>> {
        let mut c = tools::command(&self.bin);
        c.stdin(Stdio::piped()).args(["rcat", &self.target(path)]);
        let mut child = c.spawn().map_err(|e| AppError::other(e.to_string()))?;
        let input = child.stdin.take().ok_or_else(|| AppError::other("No input"))?;
        Ok(Box::new(ChildWriter { child: Some(child), input: Some(input) }))
    }
}

fn finish_child(mut child: Child) -> AppResult<()> {
    let status = child.wait().map_err(|e| AppError::other(e.to_string()))?;
    if status.success() {
        return Ok(());
    }
    let mut err = Vec::new();
    if let Some(mut e) = child.stderr.take() {
        let _ = e.read_to_end(&mut err);
    }
    Err(tools::tool_error(&err))
}

/// stdout of a running process.
struct ChildReader {
    child: Option<Child>,
    out: std::process::ChildStdout,
}

impl Read for ChildReader {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        self.out.read(buf)
    }
}

impl ReadStream for ChildReader {
    fn finish(mut self: Box<Self>) -> AppResult<()> {
        let _ = std::io::copy(&mut self.out, &mut std::io::sink());
        finish_child(self.child.take().ok_or_else(|| AppError::other("finished"))?)
    }
}

impl Drop for ChildReader {
    fn drop(&mut self) {
        if let Some(mut c) = self.child.take() {
            let _ = c.kill();
            let _ = c.wait();
        }
    }
}

/// stdin of a running process; `finish` closes it and checks the exit code.
struct ChildWriter {
    child: Option<Child>,
    input: Option<std::process::ChildStdin>,
}

impl Write for ChildWriter {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.input.as_mut().ok_or_else(|| std::io::Error::other("closed"))?.write(buf)
    }
    fn flush(&mut self) -> std::io::Result<()> {
        self.input.as_mut().map_or(Ok(()), |i| i.flush())
    }
}

impl WriteStream for ChildWriter {
    fn finish(mut self: Box<Self>) -> AppResult<()> {
        drop(self.input.take());
        finish_child(self.child.take().ok_or_else(|| AppError::other("finished"))?)
    }
}

impl Drop for ChildWriter {
    fn drop(&mut self) {
        // Not finished (cancelled / failed): stop the upload.
        if let Some(mut c) = self.child.take() {
            drop(self.input.take());
            let _ = c.kill();
            let _ = c.wait();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses() {
        let r = parse_remotes("gdrive:  drive\nnas:     sftp\n\n");
        assert_eq!(r.len(), 2);
        assert_eq!((r[0].id.as_str(), r[0].detail.as_str()), ("gdrive", "drive"));
        assert_eq!(rfc3339("1970-01-01T00:01:00Z"), Some(60_000));
        assert_eq!(rfc3339("1970-01-01T01:01:00.5+01:00"), Some(60_500));
        assert_eq!(rfc3339("1970-01-01T00:00:00-00:30"), Some(1_800_000));
        let items: Vec<Item> = serde_json::from_str(r#"[{"Path":"a","Name":"a","Size":-1,"ModTime":"2000-01-01T00:00:00Z","IsDir":true}]"#).unwrap();
        assert!(to_entry(items.into_iter().next().unwrap()).is_dir());
    }
}
