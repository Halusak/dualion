//! Finds images embedded in other files: the PNG preview Affinity stores
//! at the end of `.af*` documents, JPEG previews inside camera RAW files,
//! cover art in audio files…

use image::DynamicImage;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

#[derive(Debug, Clone, Copy)]
pub enum Scan {
    /// First n bytes.
    Head(u64),
    /// Last n bytes.
    Tail(u64),
}

const PNG_SIG: &[u8] = b"\x89PNG\r\n\x1a\n";

/// Length of a PNG stream starting at `d[0]` (through its IEND chunk).
pub fn png_len(d: &[u8]) -> Option<usize> {
    if !d.starts_with(PNG_SIG) {
        return None;
    }
    let mut i = 8;
    let mut first = true;
    loop {
        let len = u32::from_be_bytes(d.get(i..i + 4)?.try_into().ok()?) as usize;
        let ty = d.get(i + 4..i + 8)?;
        if !ty.iter().all(u8::is_ascii_alphabetic) || (first && ty != b"IHDR") {
            return None;
        }
        first = false;
        let next = i.checked_add(12)?.checked_add(len)?;
        if next > d.len() {
            return None;
        }
        if ty == b"IEND" {
            return Some(next);
        }
        i = next;
    }
}

/// Length of a JPEG stream starting at `d[0]` (through its EOI marker),
/// following segment lengths so embedded thumbnails do not end it early.
pub fn jpeg_len(d: &[u8]) -> Option<usize> {
    if d.len() < 4 || d[0] != 0xFF || d[1] != 0xD8 {
        return None;
    }
    let mut i = 2;
    loop {
        while d.get(i) == Some(&0xFF) && d.get(i + 1) == Some(&0xFF) {
            i += 1;
        }
        if *d.get(i)? != 0xFF {
            return None;
        }
        let marker = *d.get(i + 1)?;
        match marker {
            0xD9 => return Some(i + 2),
            0xD8 => return None,
            0x01 | 0xD0..=0xD7 => {
                i += 2;
                continue;
            }
            _ => {}
        }
        let len = u16::from_be_bytes([*d.get(i + 2)?, *d.get(i + 3)?]) as usize;
        if len < 2 {
            return None;
        }
        i += 2 + len;
        if marker == 0xDA {
            // Entropy-coded data runs until the next real marker.
            loop {
                let b = *d.get(i)?;
                if b == 0xFF {
                    let n = *d.get(i + 1)?;
                    if n == 0x00 || (0xD0..=0xD7).contains(&n) {
                        i += 2;
                        continue;
                    }
                    if n == 0xFF {
                        i += 1;
                        continue;
                    }
                    break;
                }
                i += 1;
            }
        }
    }
}

fn read_region(path: &Path, scan: Scan) -> Option<Vec<u8>> {
    let mut f = File::open(path).ok()?;
    let size = f.metadata().ok()?.len();
    let (start, len) = match scan {
        Scan::Head(n) => (0, n.min(size)),
        Scan::Tail(n) => (size.saturating_sub(n), n.min(size)),
    };
    f.seek(SeekFrom::Start(start)).ok()?;
    let mut buf = vec![0u8; len as usize];
    f.read_exact(&mut buf).ok()?;
    Some(buf)
}

/// All PNG / JPEG streams in `d` (outer streams only).
pub fn candidates(d: &[u8]) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    let mut i = 0;
    while i + 8 < d.len() && out.len() < 64 {
        let b = d[i];
        let found = if b == 0x89 && d[i..].starts_with(PNG_SIG) {
            png_len(&d[i..])
        } else if b == 0xFF && d[i + 1] == 0xD8 && d[i + 2] == 0xFF {
            jpeg_len(&d[i..]).filter(|&l| l > 512)
        } else {
            None
        };
        match found {
            Some(len) => {
                out.push((i, len));
                i += len;
            }
            None => i += 1,
        }
    }
    out
}

/// Decodes the largest embedded image found in the scanned region.
pub fn largest_image(path: &Path, scan: Scan) -> Option<DynamicImage> {
    let data = read_region(path, scan)?;
    let mut best: Option<(u64, (usize, usize))> = None;
    for (off, len) in candidates(&data) {
        let slice = &data[off..off + len];
        let dims = image::ImageReader::new(std::io::Cursor::new(slice))
            .with_guessed_format()
            .ok()
            .and_then(|r| r.into_dimensions().ok());
        if let Some((w, h)) = dims {
            let area = w as u64 * h as u64;
            if best.map(|(a, _)| area > a).unwrap_or(true) {
                best = Some((area, (off, len)));
            }
        }
    }
    let (_, (off, len)) = best?;
    image::load_from_memory(&data[off..off + len]).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn png(w: u32, h: u32) -> Vec<u8> {
        let mut out = std::io::Cursor::new(Vec::new());
        image::RgbImage::from_pixel(w, h, image::Rgb([10, 200, 30])).write_to(&mut out, image::ImageFormat::Png).unwrap();
        out.into_inner()
    }

    fn jpeg(w: u32, h: u32) -> Vec<u8> {
        let mut out = std::io::Cursor::new(Vec::new());
        image::RgbImage::from_pixel(w, h, image::Rgb([200, 30, 30])).write_to(&mut out, image::ImageFormat::Jpeg).unwrap();
        out.into_inner()
    }

    #[test]
    fn stream_lengths() {
        let p = png(20, 10);
        assert_eq!(png_len(&p), Some(p.len()));
        let j = jpeg(64, 64);
        let mut padded = j.clone();
        padded.extend_from_slice(b"trailing garbage");
        assert_eq!(jpeg_len(&padded), Some(j.len()));
    }

    #[test]
    fn finds_largest_embedded_image() {
        let dir = std::env::temp_dir().join(format!("dualion-emb-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut file = b"header-bytes".repeat(100);
        file.extend(jpeg(32, 32));
        file.extend(b"middle".repeat(50));
        file.extend(png(300, 200));
        file.extend(b"end".repeat(10));
        let p = dir.join("doc.af");
        std::fs::write(&p, &file).unwrap();
        let img = largest_image(&p, Scan::Tail(1 << 20)).unwrap();
        assert_eq!((img.width(), img.height()), (300, 200));
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
