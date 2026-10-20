//! Lister (F3) backend: content sniffing, encoding detection, chunked text
//! and binary reads so multi-gigabyte files open instantly.

use crate::error::{AppError, AppResult};
use crate::preview;
use crate::util::system_time_ms;
use serde::Serialize;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Probe {
    pub size: u64,
    pub mtime: i64,
    /// "text" | "binary" | "image" | "video" | "audio" | "pdf" |
    /// "render" | "sheet" | "document" | "slides" | "font"
    pub kind: &'static str,
    pub encoding: String,
    /// Whether the raw content is readable text (Text mode makes sense).
    pub textual: bool,
}

const IMAGE: &[&str] = &["jpg", "jpeg", "png", "gif", "webp", "bmp", "svg", "ico", "avif"];
#[cfg(target_os = "macos")]
const IMAGE_NATIVE: &[&str] = &["heic", "heif", "tif", "tiff"];
#[cfg(not(target_os = "macos"))]
const IMAGE_NATIVE: &[&str] = &[];
const VIDEO: &[&str] = &["mp4", "m4v", "mov", "webm"];
const AUDIO: &[&str] = &["mp3", "m4a", "aac", "wav", "flac", "ogg", "oga", "opus"];

pub fn detect_encoding(head: &[u8]) -> String {
    if head.starts_with(&[0xEF, 0xBB, 0xBF]) {
        return "utf-8".into();
    }
    if head.starts_with(&[0xFF, 0xFE]) {
        return "utf-16le".into();
    }
    if head.starts_with(&[0xFE, 0xFF]) {
        return "utf-16be".into();
    }
    match std::str::from_utf8(head) {
        Ok(_) => return "utf-8".into(),
        // Only an incomplete sequence at the very end: still UTF-8.
        Err(e) if e.error_len().is_none() && e.valid_up_to() + 4 >= head.len() => return "utf-8".into(),
        Err(_) => {}
    }
    let mut det = chardetng::EncodingDetector::new(chardetng::Iso2022JpDetection::Deny);
    det.feed(head, false);
    det.guess(Some(b"cz"), chardetng::Utf8Detection::Allow).name().to_ascii_lowercase()
}

pub fn probe(path: &Path) -> AppResult<Probe> {
    let meta = std::fs::metadata(path).map_err(|e| AppError::io(e, path))?;
    let ext = path.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
    let base = Probe { size: meta.len(), mtime: meta.modified().map(system_time_ms).unwrap_or(0), kind: "text", encoding: String::new(), textual: false };
    let e = ext.as_str();
    let kind = if IMAGE.contains(&e) || IMAGE_NATIVE.contains(&e) {
        Some("image")
    } else if VIDEO.contains(&e) {
        Some("video")
    } else if AUDIO.contains(&e) {
        Some("audio")
    } else if e == "pdf" {
        Some("pdf")
    } else if preview::SHEET.contains(&e) {
        Some("sheet")
    } else if preview::DOCUMENT.contains(&e) {
        Some("document")
    } else if preview::SLIDES.contains(&e) {
        Some("slides")
    } else if preview::FONT.contains(&e) {
        Some("font")
    } else if preview::is_renderable(e) {
        Some("render")
    } else {
        None
    };
    if let Some(kind) = kind {
        let textual = matches!(e, "csv" | "tsv" | "tab" | "rtf" | "svg");
        let encoding = if textual { probe_encoding(path).unwrap_or_default() } else { String::new() };
        return Ok(Probe { kind, textual, encoding, ..base });
    }
    let mut f = File::open(path).map_err(|e| AppError::io(e, path))?;
    let mut head = vec![0u8; 64 * 1024];
    let n = read_full(&mut f, &mut head).map_err(|e| AppError::io(e, path))?;
    head.truncate(n);
    let utf16 = head.starts_with(&[0xFF, 0xFE]) || head.starts_with(&[0xFE, 0xFF]);
    let nuls = head.iter().take(8192).filter(|&&b| b == 0).count();
    if !utf16 && nuls > 0 {
        return Ok(Probe { kind: "binary", ..base });
    }
    Ok(Probe { kind: "text", encoding: detect_encoding(&head), textual: true, ..base })
}

fn probe_encoding(path: &Path) -> Option<String> {
    let head = read_bytes(path, 0, 64 * 1024).ok()?;
    Some(detect_encoding(&head))
}

fn read_full(f: &mut File, buf: &mut [u8]) -> std::io::Result<usize> {
    let mut total = 0;
    while total < buf.len() {
        match f.read(&mut buf[total..]) {
            Ok(0) => break,
            Ok(n) => total += n,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
            Err(e) => return Err(e),
        }
    }
    Ok(total)
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TextChunk {
    pub text: String,
    pub start: u64,
    pub end: u64,
    pub eof: bool,
}

pub fn read_text(path: &Path, offset: u64, max: usize, encoding: &str) -> AppResult<TextChunk> {
    let mut f = File::open(path).map_err(|e| AppError::io(e, path))?;
    let size = f.metadata().map(|m| m.len()).unwrap_or(0);
    f.seek(SeekFrom::Start(offset)).map_err(|e| AppError::io(e, path))?;
    let mut buf = vec![0u8; max.min(64 * 1024 * 1024)];
    let n = read_full(&mut f, &mut buf).map_err(|e| AppError::io(e, path))?;
    buf.truncate(n);
    let eof = offset + n as u64 >= size;
    let enc = encoding_rs::Encoding::for_label(encoding.as_bytes()).unwrap_or(encoding_rs::UTF_8);
    let mut used = buf.len();
    if !eof {
        if enc == encoding_rs::UTF_8 {
            if let Err(e) = std::str::from_utf8(&buf) {
                if e.error_len().is_none() {
                    used = e.valid_up_to();
                }
            }
        } else if enc == encoding_rs::UTF_16LE || enc == encoding_rs::UTF_16BE {
            used -= used % 2;
        }
    }
    let bytes = &buf[..used];
    let text = if offset == 0 {
        enc.decode_with_bom_removal(bytes).0.into_owned()
    } else {
        enc.decode_without_bom_handling(bytes).0.into_owned()
    };
    Ok(TextChunk { text, start: offset, end: offset + used as u64, eof: eof && used == buf.len() })
}

pub fn read_bytes(path: &Path, offset: u64, len: usize) -> AppResult<Vec<u8>> {
    let mut f = File::open(path).map_err(|e| AppError::io(e, path))?;
    f.seek(SeekFrom::Start(offset)).map_err(|e| AppError::io(e, path))?;
    let mut buf = vec![0u8; len.min(16 * 1024 * 1024)];
    let n = read_full(&mut f, &mut buf).map_err(|e| AppError::io(e, path))?;
    buf.truncate(n);
    Ok(buf)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_encodings() {
        assert_eq!(detect_encoding("Ahoj světe".as_bytes()), "utf-8");
        let (cp, _, _) = encoding_rs::WINDOWS_1250.encode("Příliš žluťoučký kůň úpěl ďábelské ódy. Šťastný Čech řekl, že je to ďábelské.");
        assert_eq!(detect_encoding(&cp), "windows-1250");
        assert_eq!(detect_encoding(&[0xFF, 0xFE, b'a', 0]), "utf-16le");
    }

    #[test]
    fn chunked_utf8_never_splits_chars() {
        let dir = std::env::temp_dir().join(format!("dualion-view-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("t.txt");
        std::fs::write(&p, "ččččč").unwrap(); // 2 bytes per char
        let c = read_text(&p, 0, 5, "utf-8").unwrap();
        assert_eq!(c.text, "čč");
        assert_eq!(c.end, 4);
        let c2 = read_text(&p, c.end, 100, "utf-8").unwrap();
        assert_eq!(c2.text, "ččč");
        assert!(c2.eof);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
