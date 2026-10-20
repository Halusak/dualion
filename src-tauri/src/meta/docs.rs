//! Document metadata: PDF page count / title / author, Office and
//! OpenDocument statistics from their XML properties.

use super::FileMeta;
use regex::bytes::Regex;
use std::io::Read;
use std::path::Path;
use std::sync::LazyLock;

static COUNT: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"/Type\s*/Pages\b[^>]*?/Count\s+(\d+)|/Count\s+(\d+)[^>]*?/Type\s*/Pages\b").unwrap());
static PAGE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"/Type\s*/Page\b").unwrap());
static OBJSTM: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"/Type\s*/ObjStm\b[^>]*>>\s*stream\r?\n").unwrap());
static TITLE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?-u)/Title\s*\(((?:[^()\\]|\\.){1,300})\)").unwrap());
static AUTHOR: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?-u)/Author\s*\(((?:[^()\\]|\\.){1,200})\)").unwrap());

pub fn pdf(path: &Path, m: &mut FileMeta) {
    let Ok(mut f) = std::fs::File::open(path) else { return };
    let mut buf = Vec::new();
    if f.by_ref().take(200 << 20).read_to_end(&mut buf).is_err() || !buf.starts_with(b"%PDF") {
        return;
    }
    m.pages = page_count(&buf).or_else(|| {
        // PDF 1.5+: page objects may live in compressed object streams.
        let mut inflated = Vec::new();
        for c in OBJSTM.find_iter(&buf) {
            let mut d = flate2::read::ZlibDecoder::new(&buf[c.end()..]);
            let mut out = Vec::new();
            let _ = d.by_ref().take(16 << 20).read_to_end(&mut out);
            inflated.extend_from_slice(&out);
            if inflated.len() > 64 << 20 {
                break;
            }
        }
        page_count(&inflated)
    });
    m.title = TITLE.captures(&buf).and_then(|c| pdf_string(&c[1]));
    m.author = AUTHOR.captures(&buf).and_then(|c| pdf_string(&c[1]));
}

fn page_count(b: &[u8]) -> Option<u32> {
    // The page tree root has the largest /Count.
    let max = COUNT
        .captures_iter(b)
        .filter_map(|c| c.get(1).or(c.get(2)))
        .filter_map(|n| std::str::from_utf8(n.as_bytes()).ok()?.parse::<u32>().ok())
        .max();
    max.or_else(|| {
        let n = PAGE.find_iter(b).count() as u32;
        (n > 0).then_some(n)
    })
}

/// Literal PDF string: PDFDocEncoding / UTF-16BE with BOM, with escapes.
fn pdf_string(raw: &[u8]) -> Option<String> {
    let mut out = Vec::with_capacity(raw.len());
    let mut i = 0;
    while i < raw.len() {
        let c = raw[i];
        if c == b'\\' && i + 1 < raw.len() {
            i += 1;
            out.push(match raw[i] {
                b'n' => b'\n',
                b'r' => b'\r',
                b't' => b'\t',
                d @ b'0'..=b'7' => {
                    let mut v = (d - b'0') as u32;
                    for _ in 0..2 {
                        if i + 1 < raw.len() && (b'0'..=b'7').contains(&raw[i + 1]) {
                            i += 1;
                            v = v * 8 + (raw[i] - b'0') as u32;
                        }
                    }
                    v as u8
                }
                o => o,
            });
        } else {
            out.push(c);
        }
        i += 1;
    }
    let s = if out.starts_with(&[0xFE, 0xFF]) {
        let u: Vec<u16> = out[2..].chunks_exact(2).map(|c| u16::from_be_bytes([c[0], c[1]])).collect();
        String::from_utf16_lossy(&u)
    } else {
        out.iter().map(|&b| b as char).collect()
    };
    let s = s.trim().to_string();
    (!s.is_empty()).then_some(s)
}

fn tag<'a>(xml: &'a str, name: &str) -> Option<&'a str> {
    let open = format!("<{name}");
    let start = xml.find(&open)?;
    let gt = start + xml[start..].find('>')?;
    if xml[..gt].ends_with('/') {
        return None;
    }
    let end = gt + xml[gt..].find(&format!("</{name}>"))?;
    Some(xml[gt + 1..end].trim()).filter(|s| !s.is_empty())
}

fn unescape(s: &str) -> String {
    s.replace("&lt;", "<").replace("&gt;", ">").replace("&quot;", "\"").replace("&apos;", "'").replace("&amp;", "&")
}

pub fn office(path: &Path, ext: &str, m: &mut FileMeta) {
    let Ok(mut zip) = crate::preview::office::open(path) else { return };
    let num = |s: Option<&str>| s.and_then(|v| v.parse::<u32>().ok()).filter(|n| *n > 0);
    if matches!(ext, "odt" | "ods" | "odp" | "ott") {
        let Some(meta) = crate::preview::office::read_entry(&mut zip, "meta.xml", 4 << 20) else { return };
        let x = String::from_utf8_lossy(&meta);
        m.title = tag(&x, "dc:title").map(unescape);
        m.author = tag(&x, "dc:creator").or_else(|| tag(&x, "meta:initial-creator")).map(unescape);
        if let Some(i) = x.find("meta:page-count=\"") {
            let rest = &x[i + 17..];
            m.pages = num(rest.split('"').next());
        }
        return;
    }
    if let Some(app) = crate::preview::office::read_entry(&mut zip, "docProps/app.xml", 4 << 20) {
        let x = String::from_utf8_lossy(&app);
        m.pages = num(tag(&x, "Pages")).or_else(|| num(tag(&x, "Slides")));
    }
    if let Some(core) = crate::preview::office::read_entry(&mut zip, "docProps/core.xml", 4 << 20) {
        let x = String::from_utf8_lossy(&core);
        m.title = tag(&x, "dc:title").map(unescape);
        m.author = tag(&x, "dc:creator").map(unescape);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pdf_pages_and_info() {
        let pdf = b"%PDF-1.4\n1 0 obj << /Type /Catalog /Pages 2 0 R >> endobj\n2 0 obj << /Type /Pages /Kids [3 0 R 4 0 R] /Count 2 >> endobj\n3 0 obj << /Type /Page >> endobj\n4 0 obj << /Type /Page >> endobj\n5 0 obj << /Title (Zpr\\341va) /Author (\xfe\xff\x00H\x00a) >> endobj\n%%EOF";
        let p = std::env::temp_dir().join(format!("dualion-{}.pdf", uuid::Uuid::new_v4()));
        std::fs::write(&p, pdf).unwrap();
        let mut m = FileMeta::default();
        super::pdf(&p, &mut m);
        std::fs::remove_file(&p).unwrap();
        assert_eq!(m.pages, Some(2));
        assert_eq!(m.title.as_deref(), Some("Zpráva"));
        assert_eq!(m.author.as_deref(), Some("Ha"));
    }

    #[test]
    fn xml_tags() {
        let x = r#"<cp:coreProperties><dc:title>A &amp; B</dc:title><dc:creator/></cp:coreProperties>"#;
        assert_eq!(tag(x, "dc:title").map(unescape).as_deref(), Some("A & B"));
        assert_eq!(tag(x, "dc:creator"), None);
    }
}
