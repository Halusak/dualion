//! ISO 9660 images with Joliet (long Unicode names). UDF-only images
//! (e.g. Windows installation ISOs) are reported so they can be mounted.

use super::AEntry;
use crate::error::{AppError, AppResult, ErrorCode};
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

const SECTOR: u64 = 2048;

fn read_at(f: &mut File, pos: u64, len: usize) -> std::io::Result<Vec<u8>> {
    f.seek(SeekFrom::Start(pos))?;
    let mut buf = vec![0u8; len];
    f.read_exact(&mut buf)?;
    Ok(buf)
}

fn le32(b: &[u8], i: usize) -> u32 {
    u32::from_le_bytes([b[i], b[i + 1], b[i + 2], b[i + 3]])
}

fn record_time(b: &[u8]) -> i64 {
    // years since 1900, month, day, hour, minute, second, GMT offset (15 min)
    if b.len() < 7 || b[1] == 0 {
        return 0;
    }
    let days = crate::util::days_from_civil(1900 + b[0] as i32, b[1] as u32, b[2].max(1) as u32);
    let secs = days * 86400 + b[3] as i64 * 3600 + b[4] as i64 * 60 + b[5] as i64 - (b[6] as i8) as i64 * 900;
    secs * 1000
}

struct Root {
    extent: u64,
    size: u64,
    joliet: bool,
}

fn descriptors(f: &mut File) -> std::io::Result<(Option<Root>, bool)> {
    let mut primary = None;
    let mut joliet = None;
    let mut udf = false;
    for s in 16..64u64 {
        let d = read_at(f, s * SECTOR, SECTOR as usize)?;
        let id = &d[1..6];
        if id == b"NSR02" || id == b"NSR03" || id == b"BEA01" {
            udf = true;
            continue;
        }
        if id != b"CD001" {
            if udf {
                continue;
            }
            break;
        }
        let root = &d[156..190];
        let r = Root { extent: le32(root, 2) as u64, size: le32(root, 10) as u64, joliet: false };
        match d[0] {
            1 => primary = Some(r),
            2 if d[88] == 0x25 && d[89] == 0x2F && matches!(d[90], 0x40 | 0x43 | 0x45) => joliet = Some(Root { joliet: true, ..r }),
            255 => {}
            _ => {}
        }
    }
    Ok((joliet.or(primary), udf))
}

fn name_of(raw: &[u8], joliet: bool) -> String {
    let n = if joliet {
        let u: Vec<u16> = raw.chunks_exact(2).map(|c| u16::from_be_bytes([c[0], c[1]])).collect();
        String::from_utf16_lossy(&u)
    } else {
        String::from_utf8_lossy(raw).into_owned()
    };
    let n = n.split(';').next().unwrap_or("").to_string();
    // Plain ISO 9660 names are upper case with a trailing dot when there is no extension.
    let n = n.strip_suffix('.').map(str::to_string).unwrap_or(n);
    if joliet { n } else { n.to_lowercase() }
}

pub fn index(path: &Path) -> AppResult<Vec<AEntry>> {
    let mut f = File::open(path).map_err(|e| AppError::io(e, path))?;
    let (root, udf) = descriptors(&mut f).map_err(|e| AppError::io(e, path))?;
    let Some(root) = root else {
        return Err(AppError::new(ErrorCode::Unsupported, "This image has no ISO 9660 file system (UDF only) – mount it instead").with_path(path));
    };
    let mut out = Vec::new();
    let mut stack = vec![(root.extent, root.size, String::new(), 0usize)];
    let mut seen = std::collections::HashSet::new();
    while let Some((extent, size, prefix, depth)) = stack.pop() {
        if depth > 64 || !seen.insert(extent) || size > 64 << 20 {
            continue;
        }
        let data = read_at(&mut f, extent * SECTOR, size as usize).map_err(|e| AppError::io(e, path))?;
        let mut i = 0usize;
        while i < data.len() {
            let len = data[i] as usize;
            if len == 0 {
                // Records do not cross sectors: continue at the next one.
                i = (i / SECTOR as usize + 1) * SECTOR as usize;
                continue;
            }
            if i + len > data.len() || len < 34 {
                break;
            }
            let r = &data[i..i + len];
            i += len;
            let nlen = r[32] as usize;
            if 33 + nlen > r.len() {
                continue;
            }
            let raw = &r[33..33 + nlen];
            if raw == [0] || raw == [1] {
                continue; // "." and ".."
            }
            let name = name_of(raw, root.joliet);
            if name.is_empty() {
                continue;
            }
            let is_dir = r[25] & 2 != 0;
            let full = if prefix.is_empty() { name } else { format!("{prefix}/{name}") };
            let (ext, sz) = (le32(r, 2) as u64, le32(r, 10) as u64);
            if is_dir {
                stack.push((ext, sz, full.clone(), depth + 1));
            }
            out.push(AEntry { path: full, is_dir, size: if is_dir { 0 } else { sz }, mtime: record_time(&r[18..25]), mode: None, encrypted: false, index: 0, extent: ext });
            if out.len() > 2_000_000 {
                break;
            }
        }
    }
    // A UDF disc carries only a stub README in its ISO 9660 part.
    if udf && out.iter().filter(|e| !e.is_dir).count() <= 1 {
        return Err(AppError::new(ErrorCode::Unsupported, "This image uses the UDF file system – mount it instead").with_path(path));
    }
    Ok(out)
}

/// Reader over one file of the image.
pub fn member<'a>(f: &'a mut File, e: &AEntry) -> std::io::Result<std::io::Take<&'a mut File>> {
    f.seek(SeekFrom::Start(e.extent * SECTOR))?;
    Ok(f.take(e.size))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Builds a tiny ISO with a primary descriptor, a Joliet descriptor and
    /// one file in a subfolder.
    fn build(path: &Path) {
        let mut img = vec![0u8; 32 * SECTOR as usize];
        let rec = |extent: u32, size: u32, dir: bool, name: &[u8]| -> Vec<u8> {
            let len = 33 + name.len() + (name.len() + 1) % 2;
            let mut r = vec![0u8; len];
            r[0] = len as u8;
            r[2..6].copy_from_slice(&extent.to_le_bytes());
            r[10..14].copy_from_slice(&size.to_le_bytes());
            r[18..25].copy_from_slice(&[124, 10, 20, 12, 0, 0, 4]);
            r[25] = if dir { 2 } else { 0 };
            r[32] = name.len() as u8;
            r[33..33 + name.len()].copy_from_slice(name);
            r
        };
        let put = |img: &mut Vec<u8>, sector: u32, data: &[u8]| {
            let o = sector as usize * SECTOR as usize;
            img[o..o + data.len()].copy_from_slice(data);
        };
        let ucs = |s: &str| s.encode_utf16().flat_map(|c| c.to_be_bytes()).collect::<Vec<u8>>();
        // Joliet tree: root(20) -> Složka(21) -> Příliš.txt(22)
        let mut root = rec(20, 2048, true, &[0]);
        root.extend(rec(20, 2048, true, &[1]));
        root.extend(rec(21, 2048, true, &ucs("Složka")));
        let mut sub = rec(21, 2048, true, &[0]);
        sub.extend(rec(20, 2048, true, &[1]));
        sub.extend(rec(22, 5, false, &ucs("Příliš.txt;1")));
        put(&mut img, 20, &root);
        put(&mut img, 21, &sub);
        put(&mut img, 22, b"hello");
        for (s, t) in [(16u32, 1u8), (17, 2)] {
            let mut d = vec![0u8; SECTOR as usize];
            d[0] = t;
            d[1..6].copy_from_slice(b"CD001");
            if t == 2 {
                d[88..91].copy_from_slice(&[0x25, 0x2F, 0x45]);
            }
            d[156..190].copy_from_slice(&rec(20, 2048, true, &[0])[..34]);
            put(&mut img, s, &d);
        }
        let mut term = vec![0u8; SECTOR as usize];
        term[0] = 255;
        term[1..6].copy_from_slice(b"CD001");
        put(&mut img, 18, &term);
        std::fs::write(path, img).unwrap();
    }

    #[test]
    fn reads_joliet_tree() {
        let p = std::env::temp_dir().join(format!("dualion-{}.iso", uuid::Uuid::new_v4()));
        build(&p);
        let idx = index(&p).unwrap();
        let a = p.to_string_lossy().into_owned();
        assert!(idx.iter().any(|e| e.path == "Složka" && e.is_dir));
        let file = idx.iter().find(|e| e.path == "Složka/Příliš.txt").expect("file");
        assert_eq!(file.size, 5);
        assert_eq!(super::super::read_member(&a, "Složka/Příliš.txt", 100).unwrap(), b"hello");
        std::fs::remove_file(&p).unwrap();
    }
}
