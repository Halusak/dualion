//! Native previews of non-image formats: thumbnails for the thumbnail view
//! and rendered / structured previews for the viewer (F3) and Quick View.

pub mod cdr;
pub mod compound;
pub mod embedded;
pub mod msdoc;
pub mod office;
pub mod psd;
pub mod rtf;
pub mod sheet;

use crate::error::{AppError, AppResult};
use image::DynamicImage;
use serde::Serialize;
use std::path::Path;

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Block {
    /// "h1" | "h2" | "h3" | "p" | "li" | "table"
    pub kind: &'static str,
    pub text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rows: Option<Vec<Vec<String>>>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Slide {
    pub title: String,
    pub lines: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DocumentPreview {
    pub blocks: Vec<Block>,
    pub truncated: bool,
}

pub const SHEET: &[&str] = &["xls", "xlsx", "xlsm", "xlsb", "xla", "xlam", "ods", "ots", "csv", "tsv", "tab"];
pub const DOCUMENT: &[&str] = &["doc", "dot", "docx", "docm", "dotx", "dotm", "odt", "ott", "rtf"];
pub const SLIDES: &[&str] = &["pptx", "pptm", "ppsx", "potx", "odp", "otp"];
pub const FONT: &[&str] = &["ttf", "otf", "woff", "woff2"];

const AFFINITY: &[&str] = &["af", "afphoto", "afdesign", "afpub", "aftemplate", "afassets", "afbrushes"];
const RAW: &[&str] = &[
    "dng", "cr2", "cr3", "crw", "nef", "nrw", "arw", "srf", "sr2", "orf", "rw2", "raf", "pef", "srw", "x3f", "erf",
    "kdc", "dcr", "mrw", "3fr", "iiq", "rwl", "mos",
];
const AUDIO_ART: &[&str] = &["mp3", "m4a", "m4b", "aac", "flac", "aiff", "aif", "wma", "dsf"];
/// Rendered only by the OS (QuickLook / Explorer) when a handler exists.
const OS_RENDERED: &[&str] = &["ppt", "pps", "pot", "eps", "ai", "indd", "vsd", "pub", "mobi", "azw3", "stl", "obj", "usdz", "fbx"];

pub fn ext_of(path: &Path) -> String {
    path.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default()
}

/// Formats the webview cannot display itself but we can turn into an image.
pub fn is_renderable(ext: &str) -> bool {
    ext == "psd"
        || ext == "psb"
        || ext == "clip"
        || ext == "cdr"
        || AFFINITY.contains(&ext)
        || RAW.contains(&ext)
        || OS_RENDERED.contains(&ext)
        || office::has_zip_thumbnail(ext) && !SHEET.contains(&ext) && !DOCUMENT.contains(&ext) && !SLIDES.contains(&ext)
        || (cfg!(not(target_os = "macos")) && matches!(ext, "tif" | "tiff" | "heic" | "heif"))
}

/// Our own image extraction (no OS help needed): PSD composites, embedded
/// previews and thumbnails stored inside ZIP based documents.
pub fn extract_image(path: &Path, max_px: u32) -> Option<DynamicImage> {
    let ext = ext_of(path);
    let ext = ext.as_str();
    let img = if ext == "psd" || ext == "psb" {
        psd::decode(path, max_px)
    } else if AFFINITY.contains(&ext) {
        embedded::largest_image(path, embedded::Scan::Tail(8 << 20)).or_else(|| embedded::largest_image(path, embedded::Scan::Head(256 << 20)))
    } else if RAW.contains(&ext) {
        embedded::largest_image(path, embedded::Scan::Head(48 << 20))
    } else if AUDIO_ART.contains(&ext) {
        embedded::largest_image(path, embedded::Scan::Head(16 << 20))
    } else if ext == "cdr" {
        cdr::image(path, max_px)
    } else if ext == "clip" {
        embedded::largest_image(path, embedded::Scan::Head(256 << 20))
    } else if office::has_zip_thumbnail(ext) {
        office::zip_thumbnail(path, ext)
    } else {
        None
    }?;
    Some(if img.width() > max_px || img.height() > max_px { img.thumbnail(max_px, max_px) } else { img })
}

/// Formats where our extraction is at least as good as the OS (tried first).
pub fn prefer_own(ext: &str) -> bool {
    AFFINITY.contains(&ext) || ext == "clip" || ext == "cdr" || matches!(ext, "kra" | "krz" | "ora" | "sketch" | "procreate" | "3mf" | "xmind")
}

/// Whether a thumbnail could exist at all (avoids pointless work).
pub fn may_have_image(ext: &str) -> bool {
    is_renderable(ext) || AUDIO_ART.contains(&ext) || office::has_zip_thumbnail(ext)
}

pub fn document(path: &Path) -> AppResult<DocumentPreview> {
    let ext = ext_of(path);
    let (blocks, truncated) = match ext.as_str() {
        "docx" | "docm" | "dotx" | "dotm" => office::docx_blocks(path)?,
        "odt" | "ott" => office::odt_blocks(path)?,
        "doc" | "dot" => paragraphs(&msdoc::doc_text(path)?),
        "rtf" => {
            let bytes = crate::viewer::read_bytes(path, 0, 64 * 1024 * 1024)?;
            paragraphs(&rtf::rtf_text(&bytes))
        }
        _ => return Err(AppError::unsupported("Not a document")),
    };
    Ok(DocumentPreview { blocks, truncated })
}

/// Adds a paragraph, turning hand-made bullets ("\t•\tItem") into list items.
pub(crate) fn push_block(blocks: &mut Vec<Block>, kind: &'static str, text: &str) {
    let t = text.trim_end();
    if t.trim().is_empty() {
        return;
    }
    if kind == "p" {
        let lead = t.trim_start();
        let mut chars = lead.chars();
        if let (Some(b), Some(sep)) = (chars.next(), chars.next()) {
            if matches!(b, '•' | '◦' | '▪' | '▫' | '‣' | '●' | '○' | '■' | '–' | '·' | '\u{F0B7}' | '\u{F0A7}') && (sep == '\t' || sep == ' ') {
                let rest = chars.as_str().trim_start();
                if !rest.is_empty() {
                    blocks.push(Block { kind: "li", text: rest.to_string(), rows: None });
                    return;
                }
            }
        }
    }
    blocks.push(Block { kind, text: t.to_string(), rows: None });
}

fn paragraphs(text: &str) -> (Vec<Block>, bool) {
    let mut blocks = Vec::new();
    for line in text.split('\n') {
        push_block(&mut blocks, "p", line);
        if blocks.len() >= 20_000 {
            return (blocks, true);
        }
    }
    (blocks, false)
}

pub fn slides(path: &Path) -> AppResult<Vec<Slide>> {
    match ext_of(path).as_str() {
        "odp" | "otp" => office::odp_slides(path),
        _ => office::pptx_slides(path),
    }
}

#[cfg(test)]
mod fixture_tests {
    /// Manual check against real files:
    /// `DUALION_FIXTURES=<dir> cargo test preview_fixtures -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn preview_fixtures() {
        let Ok(dir) = std::env::var("DUALION_FIXTURES") else { return };
        let mut files: Vec<_> = std::fs::read_dir(dir).unwrap().flatten().map(|e| e.path()).collect();
        files.sort();
        for p in files {
            let probe = crate::viewer::probe(&p).unwrap();
            let name = p.file_name().unwrap().to_string_lossy();
            let detail = match probe.kind {
                "sheet" => {
                    let s = super::sheet::workbook(&p, 0, 10).map(|s| format!("sheets={:?} rows={:?}", s.sheets, s.rows));
                    s.unwrap_or_else(|e| format!("ERR {e}"))
                }
                "document" => super::document(&p)
                    .map(|d| d.blocks.iter().take(6).map(|b| format!("{}:{}{}", b.kind, b.text, b.rows.as_ref().map(|r| format!("{r:?}")).unwrap_or_default())).collect::<Vec<_>>().join(" | "))
                    .unwrap_or_else(|e| format!("ERR {e}")),
                "slides" => super::slides(&p).map(|s| format!("{s:?}")).unwrap_or_else(|e| format!("ERR {e}")),
                _ => String::new(),
            };
            let own = super::extract_image(&p, 512).map(|i| format!("{}x{}", i.width(), i.height()));
            let thumb = crate::thumbs::thumbnail(&p, 256).map(|t| format!("{} {}B", t.mime, t.bytes.len()));
            println!("{name:16} kind={:9} own={own:?} thumb={thumb:?} {detail}", probe.kind);
        }
    }
}
