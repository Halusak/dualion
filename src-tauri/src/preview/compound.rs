//! Minimal, tolerant reader of OLE compound files ([MS-CFB]) – enough to
//! pull streams out of legacy Office documents. Unlike strict parsers it
//! ignores inconsistencies in unused allocation entries, which some
//! writers (e.g. macOS textutil) produce.

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

const END: u32 = 0xFFFF_FFFE;
const MAX_REGULAR: u32 = 0xFFFF_FFFA;

pub struct Compound {
    file: File,
    sector: usize,
    fat: Vec<u32>,
    minifat: Vec<u32>,
    mini_stream: Vec<u8>,
    cutoff: u64,
    entries: Vec<(String, u8, u32, u64)>,
}

fn u16le(d: &[u8], at: usize) -> u16 {
    u16::from_le_bytes([d[at], d[at + 1]])
}

fn u32le(d: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([d[at], d[at + 1], d[at + 2], d[at + 3]])
}

fn bad(msg: &str) -> std::io::Error {
    std::io::Error::new(std::io::ErrorKind::InvalidData, msg.to_string())
}

impl Compound {
    pub fn open(path: &Path) -> std::io::Result<Self> {
        let mut file = File::open(path)?;
        let mut h = [0u8; 512];
        file.read_exact(&mut h)?;
        if h[..8] != [0xD0, 0xCF, 0x11, 0xE0, 0xA1, 0xB1, 0x1A, 0xE1] {
            return Err(bad("not a compound file"));
        }
        let sector = match u16le(&h, 0x1E) {
            9 => 512,
            12 => 4096,
            _ => return Err(bad("unsupported sector size")),
        };
        let file_len = file.metadata()?.len();
        let max_sectors = (file_len / sector as u64 + 1) as usize;

        // DIFAT: 109 entries in the header, then a chain of DIFAT sectors.
        let mut difat: Vec<u32> = (0..109).map(|i| u32le(&h, 0x4C + i * 4)).collect();
        let mut next = u32le(&h, 0x44);
        let mut buf = vec![0u8; sector];
        let mut guard = 0;
        while next <= MAX_REGULAR && guard < max_sectors {
            Self::read_sector_into(&mut file, sector, next, &mut buf)?;
            let n = sector / 4 - 1;
            difat.extend((0..n).map(|i| u32le(&buf, i * 4)));
            next = u32le(&buf, n * 4);
            guard += 1;
        }
        let mut fat = Vec::new();
        for &s in difat.iter().filter(|&&s| s <= MAX_REGULAR) {
            if Self::read_sector_into(&mut file, sector, s, &mut buf).is_ok() {
                fat.extend((0..sector / 4).map(|i| u32le(&buf, i * 4)));
            }
        }

        let mut me = Compound { file, sector, fat, minifat: Vec::new(), mini_stream: Vec::new(), cutoff: u32le(&h, 0x38) as u64, entries: Vec::new() };

        let dir = me.read_chain(u32le(&h, 0x30), u64::MAX)?;
        for e in dir.chunks_exact(128) {
            let name_len = (u16le(e, 64) as usize).min(64);
            let units: Vec<u16> = e[..name_len.saturating_sub(2)].chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
            let kind = e[66];
            let start = u32le(e, 116);
            let size = u32le(e, 120) as u64 | if sector == 4096 { (u32le(e, 124) as u64) << 32 } else { 0 };
            me.entries.push((String::from_utf16_lossy(&units), kind, start, size));
        }
        let minifat = me.read_chain(u32le(&h, 0x3C), u64::MAX)?;
        me.minifat = minifat.chunks_exact(4).map(|c| u32le(c, 0)).collect();
        if let Some(&(_, 5, start, size)) = me.entries.first() {
            me.mini_stream = me.read_chain(start, size)?;
        }
        Ok(me)
    }

    fn read_sector_into(file: &mut File, sector: usize, id: u32, buf: &mut [u8]) -> std::io::Result<()> {
        file.seek(SeekFrom::Start((id as u64 + 1) * sector as u64))?;
        file.read_exact(buf)
    }

    fn read_chain(&mut self, start: u32, limit: u64) -> std::io::Result<Vec<u8>> {
        let mut out = Vec::new();
        let mut buf = vec![0u8; self.sector];
        let mut id = start;
        let mut steps = 0;
        while id <= MAX_REGULAR && (out.len() as u64) < limit && steps <= self.fat.len() {
            Self::read_sector_into(&mut self.file, self.sector, id, &mut buf)?;
            out.extend_from_slice(&buf);
            id = *self.fat.get(id as usize).unwrap_or(&END);
            steps += 1;
        }
        if limit != u64::MAX {
            out.truncate(limit as usize);
        }
        Ok(out)
    }

    fn read_mini_chain(&self, start: u32, size: u64) -> Vec<u8> {
        let mut out = Vec::new();
        let mut id = start;
        let mut steps = 0;
        while id <= MAX_REGULAR && (out.len() as u64) < size && steps <= self.minifat.len() {
            let at = id as usize * 64;
            match self.mini_stream.get(at..at + 64) {
                Some(chunk) => out.extend_from_slice(chunk),
                None => break,
            }
            id = *self.minifat.get(id as usize).unwrap_or(&END);
            steps += 1;
        }
        out.truncate(size as usize);
        out
    }

    /// Reads the stream called `name` (case-insensitive, any storage).
    pub fn stream(&mut self, name: &str) -> std::io::Result<Vec<u8>> {
        let (_, _, start, size) = self
            .entries
            .iter()
            .find(|(n, kind, _, _)| *kind == 2 && n.eq_ignore_ascii_case(name))
            .cloned()
            .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::NotFound, format!("stream {name} not found")))?;
        if size < self.cutoff {
            Ok(self.read_mini_chain(start, size))
        } else {
            self.read_chain(start, size)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn reads_regular_and_mini_streams() {
        let dir = std::env::temp_dir().join(format!("dualion-cfb-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("t.cfb");
        let big: Vec<u8> = (0..20_000u32).map(|i| (i % 251) as u8).collect();
        {
            let mut c = cfb::create(&p).unwrap();
            c.create_stream("/Big").unwrap().write_all(&big).unwrap();
            c.create_stream("/small").unwrap().write_all(b"hello mini stream").unwrap();
            c.flush().unwrap();
        }
        let mut c = Compound::open(&p).unwrap();
        assert_eq!(c.stream("Big").unwrap(), big);
        assert_eq!(c.stream("SMALL").unwrap(), b"hello mini stream");
        assert!(c.stream("missing").is_err());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
