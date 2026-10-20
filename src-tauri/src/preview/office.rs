//! ZIP based document formats: embedded thumbnails (Office, OpenDocument,
//! iWork, Sketch, Krita, OpenRaster, Procreate, 3MF, XMind, EPUB) and the
//! readable content of Word / Writer documents and presentations.

use super::{Block, Slide};
use crate::error::{AppError, AppResult};
use image::DynamicImage;
use quick_xml::events::{BytesRef, BytesStart, Event};
use quick_xml::Reader;
use std::io::Read;
use std::path::Path;
use zip::ZipArchive;

type Zip = ZipArchive<std::io::BufReader<std::fs::File>>;

fn open(path: &Path) -> AppResult<Zip> {
    let f = std::fs::File::open(path).map_err(|e| AppError::io(e, path))?;
    Ok(ZipArchive::new(std::io::BufReader::new(f))?)
}

fn read_entry(zip: &mut Zip, name: &str, limit: u64) -> Option<Vec<u8>> {
    // Names are case sensitive inside ZIPs, but producers differ.
    let actual = zip.file_names().filter_map(Result::ok).find(|n| n.eq_ignore_ascii_case(name))?.to_string();
    let file = zip.by_name(&actual).ok()?;
    let mut out = Vec::new();
    file.take(limit).read_to_end(&mut out).ok()?;
    Some(out)
}

fn thumbnail_names(ext: &str) -> &'static [&'static str] {
    match ext {
        "docx" | "docm" | "dotx" | "xlsx" | "xlsm" | "xltx" | "pptx" | "pptm" | "ppsx" | "potx" | "vsdx" => &[
            "docProps/thumbnail.jpeg",
            "docProps/thumbnail.jpg",
            "docProps/thumbnail.png",
        ],
        "odt" | "ott" | "ods" | "ots" | "odp" | "otp" | "odg" | "otg" | "odf" => &["Thumbnails/thumbnail.png"],
        "pages" | "numbers" | "key" => &["preview.jpg", "QuickLook/Thumbnail.jpg", "QuickLook/Preview.jpg", "preview-web.jpg"],
        "sketch" => &["previews/preview.png"],
        "kra" | "krz" => &["mergedimage.png", "preview.png"],
        "ora" => &["mergedimage.png", "Thumbnails/thumbnail.png"],
        "procreate" => &["QuickLook/Thumbnail.png"],
        "3mf" => &["Metadata/thumbnail.png", "Metadata/plate_1.png"],
        "xmind" => &["Thumbnails/thumbnail.png"],
        _ => &[],
    }
}

pub fn has_zip_thumbnail(ext: &str) -> bool {
    ext == "epub" || !thumbnail_names(ext).is_empty()
}

/// The preview image stored inside a ZIP based document, if any.
pub fn zip_thumbnail(path: &Path, ext: &str) -> Option<DynamicImage> {
    let mut zip = open(path).ok()?;
    for name in thumbnail_names(ext) {
        if let Some(bytes) = read_entry(&mut zip, name, 64 << 20) {
            if let Ok(img) = image::load_from_memory(&bytes) {
                return Some(img);
            }
        }
    }
    if ext == "epub" {
        // Prefer an image called "cover", otherwise the first image.
        let names: Vec<String> = zip.file_names().filter_map(Result::ok).map(|n| n.to_string()).collect();
        let is_img = |n: &str| {
            let l = n.to_ascii_lowercase();
            l.ends_with(".jpg") || l.ends_with(".jpeg") || l.ends_with(".png")
        };
        let pick = names
            .iter()
            .find(|n| is_img(n) && n.to_ascii_lowercase().contains("cover"))
            .or_else(|| names.iter().find(|n| is_img(n)))?;
        let bytes = read_entry(&mut zip, pick, 32 << 20)?;
        return image::load_from_memory(&bytes).ok();
    }
    None
}

// ---------------------------------------------------------------------------
// XML helpers
// ---------------------------------------------------------------------------

fn local(name: &str) -> &str {
    name.rsplit(':').next().unwrap_or(name)
}

fn attr(e: &BytesStart, key: &str) -> Option<String> {
    // Only plain identifiers/numbers are read, so no unescaping is needed.
    e.attributes().flatten().find(|a| local(a.key.as_ref()) == key).map(|a| a.value.to_string())
}

fn entity(r: &BytesRef) -> String {
    if let Ok(Some(c)) = r.resolve_char_ref() {
        return c.to_string();
    }
    match r.xml10_content().as_ref() {
        "amp" => "&",
        "lt" => "<",
        "gt" => ">",
        "quot" => "\"",
        "apos" => "'",
        _ => "",
    }
    .to_string()
}

const MAX_TEXT: usize = 2_000_000;

fn push_block(blocks: &mut Vec<Block>, kind: &'static str, text: String) {
    super::push_block(blocks, kind, &text);
}

/// Headings made with direct formatting instead of styles: a short paragraph
/// whose text is all bold and large (sizes in half-points).
fn formatted_heading(text: &str, min_sz: usize, all_bold: bool) -> Option<&'static str> {
    if text.contains('\n') || text.trim().chars().count() > 150 || min_sz == usize::MAX {
        return None;
    }
    match (min_sz, all_bold) {
        (40.., true) | (52.., false) => Some("h1"),
        (30.., true) => Some("h2"),
        (26.., true) => Some("h3"),
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Word (DOCX)
// ---------------------------------------------------------------------------

fn heading_level(style: &str) -> Option<&'static str> {
    let s = style.to_ascii_lowercase();
    if s == "title" || s == "nzev" || s == "nazev" {
        return Some("h1");
    }
    if s == "subtitle" || s == "podtitul" {
        return Some("h2");
    }
    if s.contains("heading") || s.contains("nadpis") {
        return match s.chars().rev().find(|c| c.is_ascii_digit()) {
            Some('1') => Some("h1"),
            Some('2') => Some("h2"),
            _ => Some("h3"),
        };
    }
    None
}

pub fn docx_blocks(path: &Path) -> AppResult<(Vec<Block>, bool)> {
    let mut zip = open(path)?;
    let xml = read_entry(&mut zip, "word/document.xml", 200 << 20).ok_or_else(|| AppError::unsupported("Not a Word document"))?;
    let xml = String::from_utf8_lossy(&xml);
    let mut r = Reader::from_str(&xml);
    let mut blocks = Vec::new();
    let (mut text, mut kind) = (String::new(), "p");
    let mut in_t = false;
    let mut tbl_depth = 0usize;
    let mut rows: Vec<Vec<String>> = Vec::new();
    let mut size = 0usize;
    // Direct run formatting, for headings that do not use heading styles.
    let (mut run_sz, mut run_bold) = (0usize, false);
    let (mut min_sz, mut all_bold) = (usize::MAX, true);
    loop {
        match r.read_event() {
            Ok(Event::Start(e)) | Ok(Event::Empty(e)) => match local(e.name().as_ref()) {
                "p" if tbl_depth == 0 => {
                    text.clear();
                    kind = "p";
                    (min_sz, all_bold) = (usize::MAX, true);
                }
                "r" => (run_sz, run_bold) = (0, false),
                "sz" => run_sz = attr(&e, "val").and_then(|v| v.parse().ok()).unwrap_or(0),
                "b" => run_bold = !matches!(attr(&e, "val").as_deref(), Some("0" | "false" | "off")),
                "pStyle" => {
                    if let Some(k) = attr(&e, "val").as_deref().and_then(heading_level) {
                        kind = k;
                    }
                }
                "numPr" if kind == "p" => kind = "li",
                "t" => in_t = true,
                "tab" => text.push('\t'),
                "br" | "cr" => text.push('\n'),
                "tbl" => {
                    tbl_depth += 1;
                    if tbl_depth == 1 {
                        rows.clear();
                    }
                }
                "tr" if tbl_depth == 1 => rows.push(Vec::new()),
                "tc" if tbl_depth == 1 => {
                    if let Some(row) = rows.last_mut() {
                        row.push(String::new());
                    }
                    text.clear();
                }
                _ => {}
            },
            Ok(Event::End(e)) => match local(e.name().as_ref()) {
                "t" => in_t = false,
                "p" if tbl_depth == 0 => {
                    size += text.len();
                    if kind == "p" {
                        kind = formatted_heading(&text, min_sz, all_bold).unwrap_or("p");
                    }
                    push_block(&mut blocks, kind, std::mem::take(&mut text));
                }
                "p" => text.push('\n'),
                "tc" if tbl_depth == 1 => {
                    if let Some(cell) = rows.last_mut().and_then(|r| r.last_mut()) {
                        *cell = text.trim().to_string();
                    }
                    text.clear();
                }
                "tbl" => {
                    tbl_depth = tbl_depth.saturating_sub(1);
                    if tbl_depth == 0 && !rows.is_empty() {
                        blocks.push(Block { kind: "table", text: String::new(), rows: Some(std::mem::take(&mut rows)) });
                    }
                }
                _ => {}
            },
            Ok(Event::Text(t)) if in_t => {
                let t = t.xml10_content();
                if !t.trim().is_empty() {
                    min_sz = min_sz.min(if run_sz == 0 { 22 } else { run_sz });
                    all_bold &= run_bold;
                }
                text.push_str(&t);
            }
            Ok(Event::GeneralRef(e)) if in_t => text.push_str(&entity(&e)),
            Ok(Event::Eof) => break,
            Err(e) => return Err(AppError::other(format!("Invalid document: {e}"))),
            _ => {}
        }
        if size > MAX_TEXT {
            return Ok((blocks, true));
        }
    }
    Ok((blocks, false))
}

// ---------------------------------------------------------------------------
// OpenDocument text (ODT)
// ---------------------------------------------------------------------------

pub fn odt_blocks(path: &Path) -> AppResult<(Vec<Block>, bool)> {
    let mut zip = open(path)?;
    let xml = read_entry(&mut zip, "content.xml", 200 << 20).ok_or_else(|| AppError::unsupported("Not an OpenDocument file"))?;
    let xml = String::from_utf8_lossy(&xml);
    let mut r = Reader::from_str(&xml);
    let mut blocks = Vec::new();
    let mut text = String::new();
    let mut kind = "p";
    let mut depth_p = 0usize;
    let mut list_depth = 0usize;
    let mut tbl_depth = 0usize;
    let mut rows: Vec<Vec<String>> = Vec::new();
    let mut size = 0usize;
    loop {
        match r.read_event() {
            Ok(Event::Start(e)) => match local(e.name().as_ref()) {
                "h" | "p" => {
                    if depth_p == 0 && tbl_depth == 0 {
                        text.clear();
                        kind = if local(e.name().as_ref()) == "h" {
                            match attr(&e, "outline-level").as_deref() {
                                Some("1") => "h1",
                                Some("2") => "h2",
                                _ => "h3",
                            }
                        } else if list_depth > 0 {
                            "li"
                        } else {
                            "p"
                        };
                    }
                    depth_p += 1;
                }
                "list-item" => list_depth += 1,
                "table" => {
                    tbl_depth += 1;
                    if tbl_depth == 1 {
                        rows.clear();
                    }
                }
                "table-row" if tbl_depth == 1 => rows.push(Vec::new()),
                "table-cell" if tbl_depth == 1 => {
                    if let Some(r) = rows.last_mut() {
                        r.push(String::new());
                    }
                    text.clear();
                }
                _ => {}
            },
            Ok(Event::Empty(e)) => match local(e.name().as_ref()) {
                "s" => {
                    let n = attr(&e, "c").and_then(|c| c.parse().ok()).unwrap_or(1usize);
                    text.extend(std::iter::repeat_n(' ', n.min(100)));
                }
                "tab" => text.push('\t'),
                "line-break" => text.push('\n'),
                _ => {}
            },
            Ok(Event::End(e)) => match local(e.name().as_ref()) {
                "h" | "p" => {
                    depth_p = depth_p.saturating_sub(1);
                    if depth_p == 0 {
                        if tbl_depth == 0 {
                            size += text.len();
                            push_block(&mut blocks, kind, std::mem::take(&mut text));
                        } else {
                            text.push('\n');
                        }
                    }
                }
                "list-item" => list_depth = list_depth.saturating_sub(1),
                "table-cell" if tbl_depth == 1 => {
                    if let Some(cell) = rows.last_mut().and_then(|r| r.last_mut()) {
                        *cell = text.trim().to_string();
                    }
                    text.clear();
                }
                "table" => {
                    tbl_depth = tbl_depth.saturating_sub(1);
                    if tbl_depth == 0 && !rows.is_empty() {
                        blocks.push(Block { kind: "table", text: String::new(), rows: Some(std::mem::take(&mut rows)) });
                    }
                }
                _ => {}
            },
            Ok(Event::Text(t)) if depth_p > 0 => text.push_str(&t.xml10_content()),
            Ok(Event::GeneralRef(e)) if depth_p > 0 => text.push_str(&entity(&e)),
            Ok(Event::Eof) => break,
            Err(e) => return Err(AppError::other(format!("Invalid document: {e}"))),
            _ => {}
        }
        if size > MAX_TEXT {
            return Ok((blocks, true));
        }
    }
    Ok((blocks, false))
}

// ---------------------------------------------------------------------------
// Presentations (PPTX, ODP)
// ---------------------------------------------------------------------------

fn pptx_slide(xml: &str) -> Slide {
    let mut r = Reader::from_str(xml);
    let mut title: Option<String> = None;
    let mut lines = Vec::new();
    let mut shape_title = false;
    let mut shape_paras: Vec<String> = Vec::new();
    let mut in_shape = false;
    let mut para = String::new();
    let mut in_t = false;
    loop {
        match r.read_event() {
            Ok(Event::Start(e)) | Ok(Event::Empty(e)) => match local(e.name().as_ref()) {
                "sp" => {
                    in_shape = true;
                    shape_title = false;
                    shape_paras.clear();
                }
                "ph" => {
                    if matches!(attr(&e, "type").as_deref(), Some("title") | Some("ctrTitle")) {
                        shape_title = true;
                    }
                }
                "p" => para.clear(),
                "t" => in_t = true,
                "br" => para.push('\n'),
                _ => {}
            },
            Ok(Event::End(e)) => match local(e.name().as_ref()) {
                "t" => in_t = false,
                "p" => {
                    let p = para.trim().to_string();
                    if !p.is_empty() {
                        if in_shape { shape_paras.push(p) } else { lines.push(p) }
                    }
                }
                "sp" => {
                    in_shape = false;
                    if shape_title && title.is_none() && !shape_paras.is_empty() {
                        title = Some(shape_paras.join(" "));
                    } else {
                        lines.append(&mut shape_paras);
                    }
                }
                _ => {}
            },
            Ok(Event::Text(t)) if in_t => para.push_str(&t.xml10_content()),
            Ok(Event::GeneralRef(e)) if in_t => para.push_str(&entity(&e)),
            Ok(Event::Eof) | Err(_) => break,
            _ => {}
        }
    }
    if title.is_none() && !lines.is_empty() {
        title = Some(lines.remove(0));
    }
    Slide { title: title.unwrap_or_default(), lines }
}

pub fn pptx_slides(path: &Path) -> AppResult<Vec<Slide>> {
    let mut zip = open(path)?;
    let mut names: Vec<(u32, String)> = zip
        .file_names()
        .filter_map(Result::ok)
        .filter_map(|n| {
            let num = n.strip_prefix("ppt/slides/slide")?.strip_suffix(".xml")?.parse().ok()?;
            Some((num, n.to_string()))
        })
        .collect();
    names.sort();
    let mut slides = Vec::new();
    for (_, name) in names.iter().take(500) {
        if let Some(xml) = read_entry(&mut zip, name, 50 << 20) {
            slides.push(pptx_slide(&String::from_utf8_lossy(&xml)));
        }
    }
    Ok(slides)
}

pub fn odp_slides(path: &Path) -> AppResult<Vec<Slide>> {
    let mut zip = open(path)?;
    let xml = read_entry(&mut zip, "content.xml", 200 << 20).ok_or_else(|| AppError::unsupported("Not an OpenDocument file"))?;
    let xml = String::from_utf8_lossy(&xml);
    let mut r = Reader::from_str(&xml);
    let mut slides: Vec<Slide> = Vec::new();
    let mut in_page = false;
    let mut title_frame = false;
    let mut para = String::new();
    let mut depth_p = 0usize;
    loop {
        match r.read_event() {
            Ok(Event::Start(e)) => match local(e.name().as_ref()) {
                "page" => {
                    in_page = true;
                    slides.push(Slide { title: String::new(), lines: Vec::new() });
                }
                "frame" => title_frame = attr(&e, "class").as_deref() == Some("title"),
                "p" | "h" => {
                    if depth_p == 0 {
                        para.clear();
                    }
                    depth_p += 1;
                }
                _ => {}
            },
            Ok(Event::Empty(e)) => match local(e.name().as_ref()) {
                "s" => para.push(' '),
                "tab" => para.push('\t'),
                "line-break" => para.push('\n'),
                _ => {}
            },
            Ok(Event::End(e)) => match local(e.name().as_ref()) {
                "page" => in_page = false,
                "frame" => title_frame = false,
                "p" | "h" => {
                    depth_p = depth_p.saturating_sub(1);
                    if depth_p == 0 && in_page {
                        let p = para.trim().to_string();
                        if let (Some(s), false) = (slides.last_mut(), p.is_empty()) {
                            if title_frame && s.title.is_empty() { s.title = p } else { s.lines.push(p) }
                        }
                    }
                }
                _ => {}
            },
            Ok(Event::Text(t)) if depth_p > 0 => para.push_str(&t.xml10_content()),
            Ok(Event::GeneralRef(e)) if depth_p > 0 => para.push_str(&entity(&e)),
            Ok(Event::Eof) | Err(_) => break,
            _ => {}
        }
    }
    for s in &mut slides {
        if s.title.is_empty() && !s.lines.is_empty() {
            s.title = s.lines.remove(0);
        }
    }
    Ok(slides)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use zip::write::SimpleFileOptions;

    fn make_zip(path: &Path, files: &[(&str, &[u8])]) {
        let mut z = zip::ZipWriter::new(std::fs::File::create(path).unwrap());
        for (name, data) in files {
            z.start_file(*name, SimpleFileOptions::default()).unwrap();
            z.write_all(data).unwrap();
        }
        z.finish().unwrap();
    }

    fn tmp() -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("dualion-office-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn docx_headings_lists_tables() {
        let dir = tmp();
        let p = dir.join("a.docx");
        let xml = r#"<?xml version="1.0"?><w:document xmlns:w="w"><w:body>
<w:p><w:pPr><w:pStyle w:val="Nadpis1"/></w:pPr><w:r><w:t>Zpráva &amp; plán</w:t></w:r></w:p>
<w:p><w:r><w:t xml:space="preserve">Příliš </w:t></w:r><w:r><w:t>žluťoučký</w:t></w:r></w:p>
<w:p><w:pPr><w:numPr><w:ilvl w:val="0"/></w:numPr></w:pPr><w:r><w:t>položka</w:t></w:r></w:p>
<w:tbl><w:tr><w:tc><w:p><w:r><w:t>A1</w:t></w:r></w:p></w:tc><w:tc><w:p><w:r><w:t>B1</w:t></w:r></w:p></w:tc></w:tr></w:tbl>
</w:body></w:document>"#;
        make_zip(&p, &[("word/document.xml", xml.as_bytes())]);
        let (b, truncated) = docx_blocks(&p).unwrap();
        assert!(!truncated);
        assert_eq!((b[0].kind, b[0].text.as_str()), ("h1", "Zpráva & plán"));
        assert_eq!((b[1].kind, b[1].text.as_str()), ("p", "Příliš žluťoučký"));
        assert_eq!(b[2].kind, "li");
        assert_eq!(b[3].rows.as_ref().unwrap(), &vec![vec!["A1".to_string(), "B1".to_string()]]);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn docx_direct_formatting() {
        let dir = tmp();
        let p = dir.join("b.docx");
        let xml = r#"<w:document xmlns:w="w"><w:body>
<w:p><w:pPr><w:rPr><w:sz w:val="20"/></w:rPr></w:pPr><w:r><w:rPr><w:b/><w:sz w:val="48"/></w:rPr><w:t>Titulek</w:t></w:r></w:p>
<w:p><w:r><w:rPr><w:b/><w:sz w:val="32"/></w:rPr><w:t>Sekce</w:t></w:r></w:p>
<w:p><w:r><w:rPr><w:b w:val="0"/><w:sz w:val="32"/></w:rPr><w:t>Jen velké</w:t></w:r></w:p>
<w:p><w:r><w:rPr><w:b/></w:rPr><w:t>Tučný text</w:t></w:r></w:p>
<w:p><w:r><w:tab/><w:t>•</w:t><w:tab/></w:r><w:r><w:t>Odrážka</w:t></w:r></w:p>
</w:body></w:document>"#;
        make_zip(&p, &[("word/document.xml", xml.as_bytes())]);
        let (b, _) = docx_blocks(&p).unwrap();
        let kinds: Vec<_> = b.iter().map(|b| (b.kind, b.text.as_str())).collect();
        assert_eq!(kinds, [("h1", "Titulek"), ("h2", "Sekce"), ("p", "Jen velké"), ("p", "Tučný text"), ("li", "Odrážka")]);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn pptx_titles_and_text() {
        let dir = tmp();
        let p = dir.join("a.pptx");
        let slide = |t: &str, body: &str| {
            format!(r#"<p:sld xmlns:p="p" xmlns:a="a"><p:cSld><p:spTree>
<p:sp><p:nvSpPr><p:nvPr><p:ph type="title"/></p:nvPr></p:nvSpPr><p:txBody><a:p><a:r><a:t>{t}</a:t></a:r></a:p></p:txBody></p:sp>
<p:sp><p:txBody><a:p><a:r><a:t>{body}</a:t></a:r></a:p></p:txBody></p:sp></p:spTree></p:cSld></p:sld>"#)
        };
        let (s1, s2) = (slide("Úvod", "Ahoj"), slide("Závěr", "Díky"));
        make_zip(&p, &[("ppt/slides/slide10.xml", s2.as_bytes()), ("ppt/slides/slide2.xml", s1.as_bytes())]);
        let s = pptx_slides(&p).unwrap();
        assert_eq!(s.len(), 2);
        assert_eq!((s[0].title.as_str(), s[0].lines[0].as_str()), ("Úvod", "Ahoj"));
        assert_eq!(s[1].title, "Závěr");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn odt_and_thumbnail() {
        let dir = tmp();
        let p = dir.join("a.odt");
        let xml = r#"<office:document-content xmlns:office="o" xmlns:text="t"><office:body><office:text>
<text:h text:outline-level="2">Kapitola</text:h><text:p>Jedna<text:s text:c="2"/>dvě</text:p>
<text:list><text:list-item><text:p>bod</text:p></text:list-item></text:list></office:text></office:body></office:document-content>"#;
        let mut png = std::io::Cursor::new(Vec::new());
        image::RgbImage::new(8, 4).write_to(&mut png, image::ImageFormat::Png).unwrap();
        make_zip(&p, &[("content.xml", xml.as_bytes()), ("Thumbnails/thumbnail.png", &png.into_inner())]);
        let (b, _) = odt_blocks(&p).unwrap();
        assert_eq!((b[0].kind, b[0].text.as_str()), ("h2", "Kapitola"));
        assert_eq!(b[1].text, "Jedna  dvě");
        assert_eq!(b[2].kind, "li");
        assert_eq!(zip_thumbnail(&p, "odt").unwrap().width(), 8);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
