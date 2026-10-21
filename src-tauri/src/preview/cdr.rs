//! CorelDRAW previews. X4 and newer files are ZIPs with a bitmap thumbnail
//! (`metadata/thumbnails/*.bmp`, newer `previews/*.png`); older RIFF files
//! (CDR 5 – X3) keep a DIB without file header in their `DISP` chunk.

use image::DynamicImage;
use std::fs::File;
use std::io::Read;
use std::path::Path;

const ZIP_NAMES: &[&str] = &[
    "metadata/thumbnails/thumbnail.bmp",
    "previews/thumbnail.png",
    "metadata/thumbnails/page1.bmp",
    "previews/page1.png",
];

pub fn thumbnail(path: &Path) -> Option<DynamicImage> {
    let mut head = [0u8; 4];
    File::open(path).ok()?.read_exact(&mut head).ok()?;
    if &head == b"PK\x03\x04" {
        let mut zip = super::office::open(path).ok()?;
        return ZIP_NAMES.iter().find_map(|n| image::load_from_memory(&super::office::read_entry(&mut zip, n, 64 << 20)?).ok());
    }
    if &head != b"RIFF" {
        return None;
    }
    let mut data = Vec::new();
    File::open(path).ok()?.take(8 << 20).read_to_end(&mut data).ok()?;
    disp_image(&data)
}

/// Decodes the `DISP` chunk of a RIFF CorelDRAW file.
fn disp_image(d: &[u8]) -> Option<DynamicImage> {
    let at = d.windows(4).position(|w| w == b"DISP")?;
    let len = u32::from_le_bytes(d.get(at + 4..at + 8)?.try_into().ok()?) as usize;
    let chunk = d.get(at + 8..(at + 8).checked_add(len)?.min(d.len()))?;
    // 4 bytes of CorelDRAW data, then BITMAPINFOHEADER + palette + pixels.
    let dib = chunk.get(4..)?;
    let header = u32::from_le_bytes(dib.get(0..4)?.try_into().ok()?) as usize;
    if !(12..=124).contains(&header) {
        return None;
    }
    let bpp = u16::from_le_bytes(dib.get(14..16)?.try_into().ok()?) as usize;
    let used = u32::from_le_bytes(dib.get(32..36)?.try_into().ok()?) as usize;
    let colors = if used > 0 { used } else if bpp <= 8 { 1 << bpp } else { 0 };
    let offset = 14 + header + colors * 4;
    let mut bmp = Vec::with_capacity(14 + dib.len());
    bmp.extend_from_slice(b"BM");
    bmp.extend_from_slice(&((14 + dib.len()) as u32).to_le_bytes());
    bmp.extend_from_slice(&[0; 4]);
    bmp.extend_from_slice(&(offset as u32).to_le_bytes());
    bmp.extend_from_slice(dib);
    image::load_from_memory_with_format(&bmp, image::ImageFormat::Bmp).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::path::PathBuf;

    fn tmp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("dualion-cdr-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        dir.join(name)
    }

    /// 24-bit DIB (bottom-up) of w×h pixels in one colour.
    fn dib(w: u32, h: u32, rgb: [u8; 3]) -> Vec<u8> {
        let stride = (w * 3).div_ceil(4) * 4;
        let mut v = Vec::new();
        v.extend_from_slice(&40u32.to_le_bytes());
        v.extend_from_slice(&(w as i32).to_le_bytes());
        v.extend_from_slice(&(h as i32).to_le_bytes());
        v.extend_from_slice(&1u16.to_le_bytes());
        v.extend_from_slice(&24u16.to_le_bytes());
        v.extend_from_slice(&[0; 24]);
        for _ in 0..h {
            let mut row: Vec<u8> = (0..w).flat_map(|_| [rgb[2], rgb[1], rgb[0]]).collect();
            row.resize(stride as usize, 0);
            v.extend_from_slice(&row);
        }
        v
    }

    #[test]
    fn riff_disp_preview() {
        let mut disp = vec![1, 0, 0, 0];
        disp.extend(dib(5, 3, [200, 10, 20]));
        let mut body = b"CDR9vrsn".to_vec();
        body.extend_from_slice(&2u32.to_le_bytes());
        body.extend_from_slice(&[0x84, 0x03]);
        body.extend_from_slice(b"DISP");
        body.extend_from_slice(&(disp.len() as u32).to_le_bytes());
        body.extend(disp);
        let mut file = b"RIFF".to_vec();
        file.extend_from_slice(&(body.len() as u32).to_le_bytes());
        file.extend(body);

        let p = tmp("old.cdr");
        std::fs::write(&p, &file).unwrap();
        let img = thumbnail(&p).expect("preview").to_rgb8();
        assert_eq!((img.width(), img.height()), (5, 3));
        assert_eq!(img.get_pixel(4, 2).0, [200, 10, 20]);
    }

    #[test]
    fn zip_thumbnail_bmp() {
        let mut bmp = b"BM".to_vec();
        let d = dib(4, 4, [0, 128, 255]);
        bmp.extend_from_slice(&((14 + d.len()) as u32).to_le_bytes());
        bmp.extend_from_slice(&[0; 4]);
        bmp.extend_from_slice(&54u32.to_le_bytes());
        bmp.extend(d);

        let p = tmp("new.cdr");
        let mut z = zip::ZipWriter::new(File::create(&p).unwrap());
        let o = zip::write::SimpleFileOptions::default();
        z.start_file("content/riffData.cdr", o).unwrap();
        z.write_all(b"RIFF\0\0\0\0CDRF").unwrap();
        z.start_file("metadata/thumbnails/thumbnail.bmp", o).unwrap();
        z.write_all(&bmp).unwrap();
        z.finish().unwrap();
        let img = thumbnail(&p).expect("preview").to_rgb8();
        assert_eq!(img.get_pixel(0, 0).0, [0, 128, 255]);
    }

    /// A real file: DUALION_CDR=<file.cdr> cargo test cdr_sample -- --ignored
    #[test]
    #[ignore]
    fn cdr_sample() {
        let p = PathBuf::from(std::env::var("DUALION_CDR").expect("DUALION_CDR"));
        let img = thumbnail(&p).expect("preview");
        println!("{}×{}", img.width(), img.height());
    }

    #[test]
    fn rejects_other_files() {
        let p = tmp("x.cdr");
        std::fs::write(&p, b"not a corel file").unwrap();
        assert!(thumbnail(&p).is_none());
    }
}
