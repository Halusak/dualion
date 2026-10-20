//! Photoshop PSD/PSB reader: the embedded JPEG thumbnail for small
//! previews, otherwise the merged ("composite") image, decoded directly at
//! a reduced size so huge documents stay cheap.

use image::{DynamicImage, RgbImage};
use std::fs::File;
use std::io::{BufReader, Read, Seek, SeekFrom};
use std::path::Path;

struct R<T: Read + Seek>(T);

impl<T: Read + Seek> R<T> {
    fn bytes<const N: usize>(&mut self) -> Option<[u8; N]> {
        let mut b = [0u8; N];
        self.0.read_exact(&mut b).ok()?;
        Some(b)
    }
    fn u16(&mut self) -> Option<u16> {
        self.bytes::<2>().map(u16::from_be_bytes)
    }
    fn u32(&mut self) -> Option<u32> {
        self.bytes::<4>().map(u32::from_be_bytes)
    }
    fn u64(&mut self) -> Option<u64> {
        self.bytes::<8>().map(u64::from_be_bytes)
    }
    fn skip(&mut self, n: u64) -> Option<()> {
        self.0.seek(SeekFrom::Current(n as i64)).ok().map(|_| ())
    }
    fn pos(&mut self) -> Option<u64> {
        self.0.stream_position().ok()
    }
    fn vec(&mut self, n: usize) -> Option<Vec<u8>> {
        let mut v = vec![0u8; n];
        self.0.read_exact(&mut v).ok()?;
        Some(v)
    }
}

struct Header {
    psb: bool,
    channels: usize,
    height: usize,
    width: usize,
    depth: u16,
    mode: u16,
}

/// PackBits decompression of one row into `out`.
fn unpack_bits(src: &[u8], out: &mut [u8]) {
    let (mut i, mut o) = (0, 0);
    while i < src.len() && o < out.len() {
        let n = src[i] as i8;
        i += 1;
        if n >= 0 {
            let c = (n as usize + 1).min(out.len() - o).min(src.len().saturating_sub(i));
            out[o..o + c].copy_from_slice(&src[i..i + c]);
            i += n as usize + 1;
            o += c;
        } else if n != -128 {
            let c = ((1 - n as isize) as usize).min(out.len() - o);
            let b = *src.get(i).unwrap_or(&0);
            i += 1;
            out[o..o + c].fill(b);
            o += c;
        }
    }
}

fn sample(row: &[u8], x: usize, depth: u16) -> u8 {
    match depth {
        1 => {
            // Bitmap mode: 1 = black.
            if row.get(x / 8).map(|b| (b >> (7 - x % 8)) & 1).unwrap_or(0) == 1 { 0 } else { 255 }
        }
        16 => *row.get(x * 2).unwrap_or(&0),
        32 => {
            let b = row.get(x * 4..x * 4 + 4).map(|s| f32::from_be_bytes([s[0], s[1], s[2], s[3]])).unwrap_or(0.0);
            (b.clamp(0.0, 1.0).powf(1.0 / 2.2) * 255.0).round() as u8
        }
        _ => *row.get(x).unwrap_or(&0),
    }
}

fn read_header<T: Read + Seek>(r: &mut R<T>) -> Option<Header> {
    if &r.bytes::<4>()? != b"8BPS" {
        return None;
    }
    let version = r.u16()?;
    r.skip(6)?;
    let channels = r.u16()? as usize;
    let height = r.u32()? as usize;
    let width = r.u32()? as usize;
    let depth = r.u16()?;
    let mode = r.u16()?;
    if !(version == 1 || version == 2) || width == 0 || height == 0 || channels == 0 {
        return None;
    }
    Some(Header { psb: version == 2, channels, height, width, depth, mode })
}

/// Decodes a PSD/PSB file so that its longer side is at most `max_px`.
pub fn decode(path: &Path, max_px: u32) -> Option<DynamicImage> {
    let mut r = R(BufReader::with_capacity(1 << 20, File::open(path).ok()?));
    let h = read_header(&mut r)?;

    let cm_len = r.u32()? as usize;
    let palette = if h.mode == 2 && cm_len >= 768 { Some(r.vec(cm_len)?) } else {
        r.skip(cm_len as u64)?;
        None
    };

    // Image resources: remember the thumbnail (1036 = JPEG RGB, 1033 = JPEG BGR).
    let ir_len = r.u32()? as u64;
    let ir_end = r.pos()? + ir_len;
    let mut thumb: Option<(Vec<u8>, bool)> = None;
    while r.pos()? + 12 <= ir_end {
        if &r.bytes::<4>()? != b"8BIM" {
            break;
        }
        let id = r.u16()?;
        let name_len = r.bytes::<1>()?[0] as u64;
        r.skip(name_len + (if name_len % 2 == 0 { 1 } else { 0 }))?;
        let size = r.u32()? as u64;
        let data_start = r.pos()?;
        if (id == 1036 || id == 1033) && size > 28 {
            r.skip(28)?;
            thumb = Some((r.vec(size as usize - 28)?, id == 1033));
        }
        r.0.seek(SeekFrom::Start(data_start + size + size % 2)).ok()?;
    }

    let thumbnail = || -> Option<DynamicImage> {
        let (jpeg, bgr) = thumb.as_ref()?;
        let img = image::load_from_memory(jpeg).ok()?;
        if *bgr {
            let mut rgb = img.to_rgb8();
            for p in rgb.pixels_mut() {
                p.0.swap(0, 2);
            }
            return Some(DynamicImage::ImageRgb8(rgb));
        }
        Some(img)
    };
    if max_px <= 160 {
        if let Some(t) = thumbnail() {
            return Some(t);
        }
    }

    r.0.seek(SeekFrom::Start(ir_end)).ok()?;
    let lm_len = if h.psb { r.u64()? } else { r.u32()? as u64 };
    r.skip(lm_len)?;
    let composite = decode_composite(&mut r, &h, palette.as_deref(), max_px);
    composite.or_else(thumbnail)
}

fn decode_composite<T: Read + Seek>(r: &mut R<T>, h: &Header, palette: Option<&[u8]>, max_px: u32) -> Option<DynamicImage> {
    let compression = r.u16()?;
    if compression > 1 || ![1, 8, 16, 32].contains(&h.depth) {
        return None;
    }
    let step = h.width.max(h.height).div_ceil(max_px.max(1) as usize).max(1);
    let (ow, oh) = (h.width.div_ceil(step), h.height.div_ceil(step));
    let used = match h.mode {
        3 => 3.min(h.channels),
        4 => 4.min(h.channels),
        _ => 1,
    };
    let row_bytes = if h.depth == 1 { h.width.div_ceil(8) } else { h.width * h.depth as usize / 8 };
    let mut planes = vec![vec![0u8; ow * oh]; used];
    let mut row = vec![0u8; row_bytes];

    let keep = |c: usize, y: usize, row: &[u8], planes: &mut Vec<Vec<u8>>| {
        if c < used && y % step == 0 {
            let oy = y / step;
            let plane = &mut planes[c];
            for ox in 0..ow {
                plane[oy * ow + ox] = sample(row, ox * step, h.depth);
            }
        }
    };

    if compression == 0 {
        for c in 0..h.channels.min(used) {
            for y in 0..h.height {
                if y % step == 0 {
                    r.0.read_exact(&mut row).ok()?;
                    keep(c, y, &row, &mut planes);
                } else {
                    r.skip(row_bytes as u64)?;
                }
            }
        }
    } else {
        let rows = h.channels * h.height;
        let mut counts = Vec::with_capacity(rows);
        for _ in 0..rows {
            counts.push(if h.psb { r.u32()? as usize } else { r.u16()? as usize });
        }
        let mut packed = Vec::new();
        for c in 0..h.channels.min(used) {
            for y in 0..h.height {
                let n = counts[c * h.height + y];
                if y % step == 0 {
                    packed.resize(n, 0);
                    r.0.read_exact(&mut packed).ok()?;
                    unpack_bits(&packed, &mut row);
                    keep(c, y, &row, &mut planes);
                } else {
                    r.skip(n as u64)?;
                }
            }
        }
    }

    let mut out = RgbImage::new(ow as u32, oh as u32);
    for (i, px) in out.pixels_mut().enumerate() {
        px.0 = match (h.mode, used) {
            (3, 3) => [planes[0][i], planes[1][i], planes[2][i]],
            (4, 4) => {
                // CMYK is stored inverted (255 = no ink).
                let k = planes[3][i] as u16;
                let f = |v: u8| ((v as u16 * k) / 255) as u8;
                [f(planes[0][i]), f(planes[1][i]), f(planes[2][i])]
            }
            (2, _) => {
                let idx = planes[0][i] as usize;
                match palette {
                    Some(p) => [p[idx], p[256 + idx], p[512 + idx]],
                    None => [planes[0][i]; 3],
                }
            }
            _ => [planes[0][i]; 3],
        };
    }
    Some(DynamicImage::ImageRgb8(out))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Writes a minimal 8-bit RGB PSD (raw or PackBits).
    fn write_psd(path: &Path, w: u32, h: u32, rgb: [u8; 3], rle: bool) {
        let mut d = Vec::new();
        d.extend(b"8BPS");
        d.extend(1u16.to_be_bytes());
        d.extend([0u8; 6]);
        d.extend(3u16.to_be_bytes());
        d.extend(h.to_be_bytes());
        d.extend(w.to_be_bytes());
        d.extend(8u16.to_be_bytes());
        d.extend(3u16.to_be_bytes());
        d.extend(0u32.to_be_bytes()); // color mode data
        d.extend(0u32.to_be_bytes()); // image resources
        d.extend(0u32.to_be_bytes()); // layers
        if rle {
            d.extend(1u16.to_be_bytes());
            // Each row: run of w bytes -> (1 - w) as i8, value (w <= 128).
            for _ in 0..3 * h {
                d.extend(2u16.to_be_bytes());
            }
            for c in rgb {
                for _ in 0..h {
                    d.push((1 - w as i32) as i8 as u8);
                    d.push(c);
                }
            }
        } else {
            d.extend(0u16.to_be_bytes());
            for c in rgb {
                d.extend(std::iter::repeat_n(c, (w * h) as usize));
            }
        }
        std::fs::write(path, d).unwrap();
    }

    #[test]
    fn decodes_raw_and_rle_composites() {
        let dir = std::env::temp_dir().join(format!("dualion-psd-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        for rle in [false, true] {
            let p = dir.join(format!("t{rle}.psd"));
            write_psd(&p, 64, 40, [200, 100, 50], rle);
            let img = decode(&p, 1024).unwrap().to_rgb8();
            assert_eq!(img.dimensions(), (64, 40));
            assert_eq!(img.get_pixel(10, 10).0, [200, 100, 50]);
            let small = decode(&p, 16).unwrap();
            assert!(small.width() <= 16 && small.height() <= 16);
        }
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn packbits() {
        let mut out = [0u8; 6];
        unpack_bits(&[0x02, 1, 2, 3, 0xFE, 9], &mut out);
        assert_eq!(out, [1, 2, 3, 9, 9, 9]);
    }
}
