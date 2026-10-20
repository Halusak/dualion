//! File metadata beyond name/size/date: image dimensions and EXIF, video and
//! audio stream info, tags, document page counts, text line counts.
//! Everything is read from headers with our own light-weight parsers, so a
//! folder of thousands of files can be annotated quickly.

mod docs;
mod media;

use crate::util::system_time_ms;
use lru::LruCache;
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use std::num::NonZeroUsize;
use std::path::Path;
use std::sync::LazyLock;

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct FileMeta {
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub width: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub height: Option<u32>,
    /// Seconds.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub duration: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub video_codec: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub audio_codec: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub fps: Option<f32>,
    /// kbit/s
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub bitrate: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub sample_rate: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub channels: Option<u8>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub camera: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub lens: Option<String>,
    /// Date taken / recorded, ms since epoch (local wall time read as UTC).
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub taken: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub iso: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub focal: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub aperture: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub exposure: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub gps: Option<(f64, f64)>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub artist: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub album: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub genre: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub year: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub track: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub pages: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub author: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub lines: Option<u64>,
}

// ---------------------------------------------------------------- kinds

pub const IMAGE: &[&str] = &[
    "jpg", "jpeg", "jfif", "png", "apng", "gif", "webp", "bmp", "ico", "tif", "tiff", "heic", "heif", "avif", "jxl", "tga",
    "exr", "hdr", "psd", "psb", "qoi", "dds", "pnm", "ppm", "pgm", "pbm",
];
pub const RAW: &[&str] = &[
    "dng", "cr2", "cr3", "crw", "nef", "nrw", "arw", "srf", "sr2", "orf", "rw2", "raf", "pef", "srw", "x3f", "erf", "kdc",
    "dcr", "mrw", "3fr", "iiq", "rwl",
];
pub const VIDEO: &[&str] = &[
    "mp4", "m4v", "mov", "qt", "3gp", "3g2", "mkv", "webm", "avi", "wmv", "flv", "mpg", "mpeg", "m2ts", "mts", "ts", "vob",
    "ogv",
];
pub const AUDIO: &[&str] = &[
    "mp3", "m4a", "m4b", "aac", "flac", "wav", "wave", "aif", "aiff", "ogg", "oga", "opus", "wma", "ape", "wv", "mpc", "dsf",
];
pub const DOCUMENT: &[&str] = &["pdf", "doc", "docx", "docm", "odt", "rtf", "pages", "txt", "md", "epub"];
pub const SHEET: &[&str] = &["xls", "xlsx", "xlsm", "xlsb", "ods", "csv", "tsv", "numbers"];
pub const SLIDES: &[&str] = &["ppt", "pptx", "pptm", "odp", "key"];
pub const ARCHIVE: &[&str] = &["zip", "7z", "rar", "tar", "gz", "tgz", "bz2", "xz", "zst", "lz4", "iso", "dmg", "cab"];
pub const DESIGN: &[&str] = &[
    "psd", "psb", "ai", "eps", "indd", "af", "afphoto", "afdesign", "afpub", "sketch", "fig", "xd", "kra", "ora", "procreate",
    "clip", "svg", "blend", "c4d", "fbx", "obj", "stl", "3mf",
];
pub const FONT: &[&str] = &["ttf", "otf", "woff", "woff2", "ttc", "fon"];
pub const CODE: &[&str] = &[
    "rs", "c", "h", "cc", "cpp", "hpp", "cxx", "cs", "java", "kt", "kts", "swift", "m", "mm", "go", "py", "rb", "php", "js",
    "mjs", "cjs", "jsx", "ts", "tsx", "svelte", "vue", "html", "htm", "css", "scss", "sass", "less", "json", "jsonc", "yaml",
    "yml", "toml", "xml", "sql", "sh", "bash", "zsh", "fish", "ps1", "bat", "cmd", "lua", "dart", "scala", "r", "pl", "ex",
    "exs", "erl", "hs", "ml", "fs", "zig", "nim", "gradle", "cmake", "make", "mk", "dockerfile", "ini", "cfg", "conf",
    "properties", "tex",
];
pub const TEXT: &[&str] = &["txt", "md", "markdown", "rst", "log", "csv", "tsv", "srt", "vtt", "ass", "nfo", "rtf"];

/// Coarse type of a file, used by `type:` in queries and for column presets.
pub fn kind_of(ext: &str, is_dir: bool) -> &'static str {
    if is_dir {
        return "folder";
    }
    let e = ext;
    if RAW.contains(&e) {
        "raw"
    } else if IMAGE.contains(&e) {
        "image"
    } else if VIDEO.contains(&e) {
        "video"
    } else if AUDIO.contains(&e) {
        "audio"
    } else if e == "pdf" {
        "pdf"
    } else if SHEET.contains(&e) {
        "sheet"
    } else if SLIDES.contains(&e) {
        "slides"
    } else if DOCUMENT.contains(&e) {
        "document"
    } else if ARCHIVE.contains(&e) {
        "archive"
    } else if FONT.contains(&e) {
        "font"
    } else if DESIGN.contains(&e) {
        "design"
    } else if CODE.contains(&e) {
        "code"
    } else if TEXT.contains(&e) {
        "text"
    } else {
        "other"
    }
}

/// Whether reading metadata can yield anything for this extension.
pub fn has_meta(ext: &str) -> bool {
    !matches!(kind_of(ext, false), "other" | "archive" | "font")
}

// ---------------------------------------------------------------- extraction

pub fn ext_of(path: &Path) -> String {
    path.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default()
}

/// Reads metadata of one file (uncached).
pub fn extract(path: &Path) -> FileMeta {
    if crate::util::path_is_cloud_only(path) {
        return FileMeta::default();
    }
    let ext = ext_of(path);
    let mut m = FileMeta::default();
    match kind_of(&ext, false) {
        "image" | "raw" => image(path, &ext, &mut m),
        "video" => media::video(path, &ext, &mut m),
        "audio" => {
            audio(path, &mut m);
            if matches!(ext.as_str(), "m4a" | "m4b") {
                // lofty cannot tell AAC from ALAC; the MP4 sample entry can.
                let mut v = FileMeta::default();
                media::video(path, &ext, &mut v);
                m.audio_codec = v.audio_codec.or(m.audio_codec);
            }
        }
        "pdf" => docs::pdf(path, &mut m),
        "document" | "sheet" | "slides" => {
            docs::office(path, &ext, &mut m);
            if TEXT.contains(&ext.as_str()) || ext == "md" {
                m.lines = count_lines(path);
            }
        }
        "code" | "text" => m.lines = count_lines(path),
        "design" => {
            if let Ok(s) = imagesize::size(path) {
                m.width = Some(s.width as u32);
                m.height = Some(s.height as u32);
            }
        }
        _ => {}
    }
    m
}

fn image(path: &Path, ext: &str, m: &mut FileMeta) {
    if let Ok(s) = imagesize::size(path) {
        m.width = Some(s.width as u32);
        m.height = Some(s.height as u32);
    }
    if !matches!(ext, "gif" | "bmp" | "ico" | "tga" | "qoi" | "dds" | "exr" | "hdr" | "psd" | "psb" | "pnm" | "ppm" | "pgm" | "pbm") {
        exif(path, m);
    }
}

fn exif(path: &Path, m: &mut FileMeta) {
    use exif::{In, Tag, Value};
    let Ok(file) = std::fs::File::open(path) else { return };
    let mut r = std::io::BufReader::new(file);
    let Ok(ex) = exif::Reader::new().read_from_container(&mut r) else { return };
    let text = |tag: Tag| -> Option<String> {
        let f = ex.get_field(tag, In::PRIMARY)?;
        match &f.value {
            Value::Ascii(v) => v.first().map(|s| String::from_utf8_lossy(s).trim().trim_end_matches('\0').to_string()),
            _ => Some(f.display_value().to_string()),
        }
        .filter(|s| !s.is_empty())
    };
    let uint = |tag: Tag| ex.get_field(tag, In::PRIMARY).and_then(|f| f.value.get_uint(0));
    let rational = |tag: Tag| -> Option<f64> {
        match &ex.get_field(tag, In::PRIMARY)?.value {
            Value::Rational(v) => v.first().map(|r| r.to_f64()),
            Value::SRational(v) => v.first().map(|r| r.to_f64()),
            _ => None,
        }
    };

    let make = text(Tag::Make).unwrap_or_default();
    if let Some(model) = text(Tag::Model) {
        // "Canon" + "Canon EOS R5" -> "Canon EOS R5"
        let first = make.split_whitespace().next().unwrap_or("");
        m.camera = Some(if first.is_empty() || model.to_lowercase().starts_with(&first.to_lowercase()) {
            model
        } else {
            format!("{} {}", title_case(first), model)
        });
    }
    m.lens = text(Tag::LensModel);
    m.iso = uint(Tag::PhotographicSensitivity);
    m.focal = rational(Tag::FocalLength).map(|v| v as f32);
    m.aperture = rational(Tag::FNumber).map(|v| (v * 10.0).round() as f32 / 10.0);
    if let Some(t) = rational(Tag::ExposureTime) {
        m.exposure = Some(if t > 0.0 && t < 1.0 { format!("1/{}", (1.0 / t).round()) } else { format!("{t}s") });
    }
    if let Some(d) = text(Tag::DateTimeOriginal).or_else(|| text(Tag::DateTime)) {
        m.taken = parse_exif_date(&d);
    }
    if m.width.is_none() {
        m.width = uint(Tag::PixelXDimension);
        m.height = uint(Tag::PixelYDimension);
    }
    // Rotated by 90°: report the displayed size.
    if matches!(uint(Tag::Orientation), Some(5..=8)) {
        if let (Some(w), Some(h)) = (m.width, m.height) {
            m.width = Some(h);
            m.height = Some(w);
        }
    }
    let coord = |tag: Tag, rf: Tag| -> Option<f64> {
        let Value::Rational(v) = &ex.get_field(tag, In::PRIMARY)?.value else { return None };
        if v.len() < 3 {
            return None;
        }
        let deg = v[0].to_f64() + v[1].to_f64() / 60.0 + v[2].to_f64() / 3600.0;
        let r = text(rf).unwrap_or_default();
        Some(if r.starts_with('S') || r.starts_with('W') { -deg } else { deg })
    };
    if let (Some(lat), Some(lon)) = (coord(Tag::GPSLatitude, Tag::GPSLatitudeRef), coord(Tag::GPSLongitude, Tag::GPSLongitudeRef)) {
        if lat.is_finite() && lon.is_finite() && (lat != 0.0 || lon != 0.0) {
            m.gps = Some(((lat * 1e6).round() / 1e6, (lon * 1e6).round() / 1e6));
        }
    }
}

fn title_case(s: &str) -> String {
    let lower = s.to_lowercase();
    let mut c = lower.chars();
    c.next().map(|f| f.to_uppercase().collect::<String>() + c.as_str()).unwrap_or_default()
}

/// "2024:05:17 14:03:22" -> ms
pub(crate) fn parse_exif_date(s: &str) -> Option<i64> {
    let d: Vec<u32> = s.split(|c: char| !c.is_ascii_digit()).filter(|p| !p.is_empty()).filter_map(|p| p.parse().ok()).collect();
    if d.len() < 3 || d[0] < 1900 || d[1] == 0 || d[1] > 12 || d[2] == 0 || d[2] > 31 {
        return None;
    }
    let g = |i: usize| d.get(i).copied().unwrap_or(0);
    Some(crate::util::days_from_civil(d[0] as i32, d[1], d[2]) * 86_400_000 + (g(3) * 3600 + g(4) * 60 + g(5)) as i64 * 1000)
}

fn audio(path: &Path, m: &mut FileMeta) {
    use lofty::file::{AudioFile, TaggedFileExt};
    use lofty::tag::{Accessor, ItemKey};
    let Ok(f) = lofty::read_from_path(path) else { return };
    let p = f.properties();
    let d = p.duration().as_secs_f64();
    if d > 0.0 {
        m.duration = Some((d * 100.0).round() / 100.0);
    }
    m.bitrate = p.audio_bitrate().or(p.overall_bitrate()).filter(|b| *b > 0);
    m.sample_rate = p.sample_rate();
    m.channels = p.channels();
    m.audio_codec = Some(
        match f.file_type() {
            lofty::file::FileType::Mpeg => "MP3",
            lofty::file::FileType::Flac => "FLAC",
            lofty::file::FileType::Opus => "Opus",
            lofty::file::FileType::Vorbis => "Vorbis",
            lofty::file::FileType::Wav => "PCM",
            lofty::file::FileType::Aiff => "PCM",
            lofty::file::FileType::Ape => "APE",
            lofty::file::FileType::WavPack => "WavPack",
            lofty::file::FileType::Mpc => "Musepack",
            lofty::file::FileType::Aac => "AAC",
            lofty::file::FileType::Mp4 => "AAC/ALAC",
            _ => "",
        }
        .to_string(),
    )
    .filter(|s| !s.is_empty());
    if let Some(tag) = f.primary_tag().or_else(|| f.first_tag()) {
        let s = |v: Option<std::borrow::Cow<'_, str>>| v.map(|c| c.trim().to_string()).filter(|s| !s.is_empty());
        m.artist = s(tag.artist());
        m.album = s(tag.album());
        m.title = s(tag.title());
        m.genre = s(tag.genre());
        m.track = tag.track();
        m.year = tag
            .get_string(ItemKey::RecordingDate)
            .or_else(|| tag.get_string(ItemKey::Year))
            .and_then(|d| d.get(..4)?.parse().ok());
    }
}

/// Counts lines of text files up to 64 MB.
fn count_lines(path: &Path) -> Option<u64> {
    use std::io::Read;
    let mut f = std::fs::File::open(path).ok()?;
    if f.metadata().ok()?.len() > 64 << 20 {
        return None;
    }
    let mut buf = vec![0u8; 256 * 1024];
    let (mut n, mut last) = (0u64, b'\n');
    let mut any = false;
    loop {
        let r = f.read(&mut buf).ok()?;
        if r == 0 {
            break;
        }
        any = true;
        if buf[..r].contains(&0) {
            return None; // binary
        }
        n += buf[..r].iter().filter(|&&b| b == b'\n').count() as u64;
        last = buf[r - 1];
    }
    Some(if any && last != b'\n' { n + 1 } else { n })
}

// ---------------------------------------------------------------- cache

type Key = (String, u64, i64);
static CACHE: LazyLock<Mutex<LruCache<Key, FileMeta>>> =
    LazyLock::new(|| Mutex::new(LruCache::new(NonZeroUsize::new(50_000).unwrap())));

/// Cached metadata for `path` with known size and mtime (avoids a stat).
pub fn get_known(path: &Path, size: u64, mtime: i64) -> FileMeta {
    // Online-only cloud files: never read (and never cache the empty result).
    if crate::util::path_is_cloud_only(path) {
        return FileMeta::default();
    }
    let key = (path.to_string_lossy().into_owned(), size, mtime);
    if let Some(m) = CACHE.lock().get(&key) {
        return m.clone();
    }
    let m = extract(path);
    CACHE.lock().put(key, m.clone());
    m
}

/// Cached metadata for `path`.
pub fn get(path: &Path) -> Option<FileMeta> {
    let md = std::fs::metadata(path).ok()?;
    if md.is_dir() {
        return None;
    }
    let mtime = md.modified().map(system_time_ms).unwrap_or(0);
    Some(get_known(path, md.len(), mtime))
}

/// Metadata of many files, read in parallel.
pub fn get_many(paths: &[String]) -> Vec<Option<FileMeta>> {
    let threads = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4).clamp(2, 8);
    let chunk = paths.len().div_ceil(threads).max(1);
    let mut out: Vec<Option<FileMeta>> = vec![None; paths.len()];
    std::thread::scope(|s| {
        for (part, slot) in paths.chunks(chunk).zip(out.chunks_mut(chunk)) {
            s.spawn(move || {
                for (p, o) in part.iter().zip(slot.iter_mut()) {
                    let path = Path::new(p);
                    if has_meta(&ext_of(path)) {
                        *o = get(path);
                    }
                }
            });
        }
    });
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exif_dates() {
        assert_eq!(parse_exif_date("1970:01:02 00:00:01"), Some(86_401_000));
        assert_eq!(parse_exif_date("2024:02:29 12:00:00"), Some(1_709_208_000_000));
        assert_eq!(parse_exif_date("0000:00:00 00:00:00"), None);
    }

    #[test]
    fn kinds() {
        assert_eq!(kind_of("cr2", false), "raw");
        assert_eq!(kind_of("mkv", false), "video");
        assert_eq!(kind_of("xlsx", false), "sheet");
        assert_eq!(kind_of("rs", false), "code");
        assert_eq!(kind_of("", true), "folder");
    }

    #[test]
    fn lines_and_png() {
        let dir = std::env::temp_dir().join(format!("dualion-meta-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let t = dir.join("a.rs");
        std::fs::write(&t, "fn a() {}\nfn b() {}\nlast").unwrap();
        assert_eq!(extract(&t).lines, Some(3));
        let p = dir.join("b.png");
        image::RgbImage::new(17, 9).save(&p).unwrap();
        let m = extract(&p);
        assert_eq!((m.width, m.height), (Some(17), Some(9)));
        std::fs::remove_dir_all(&dir).unwrap();
    }
}

/// Searchable text of a file for `content:` queries: plain text and code
/// (up to 16 MB) and the text of office documents.
pub fn text_content(path: &Path) -> Option<String> {
    if crate::util::path_is_cloud_only(path) {
        return None;
    }
    let ext = ext_of(path);
    let plain = TEXT.contains(&ext.as_str()) || CODE.contains(&ext.as_str()) || ext == "md";
    match if plain && ext != "rtf" { "text" } else { kind_of(&ext, false) } {
        "code" | "text" => {
            let bytes = crate::viewer::read_bytes(path, 0, 16 << 20).ok()?;
            if bytes.contains(&0) {
                return None;
            }
            let enc = crate::viewer::detect_encoding(&bytes[..bytes.len().min(64 * 1024)]);
            let enc = encoding_rs::Encoding::for_label(enc.as_bytes()).unwrap_or(encoding_rs::UTF_8);
            Some(enc.decode(&bytes).0.into_owned())
        }
        "document" if ext != "pdf" => {
            let doc = crate::preview::document(path).ok()?;
            Some(doc.blocks.iter().map(|b| b.rows.as_ref().map(|r| r.iter().map(|c| c.join(" ")).collect::<Vec<_>>().join("\n")).unwrap_or_else(|| b.text.clone())).collect::<Vec<_>>().join("\n"))
        }
        "pdf" => {
            if std::fs::metadata(path).ok()?.len() > 50 << 20 {
                return None;
            }
            // The PDF parser may panic on broken files.
            let p = path.to_path_buf();
            let text = std::panic::catch_unwind(move || pdf_extract::extract_text(&p).ok()).ok()??;
            Some(text.chars().take(200_000).collect())
        }
        "slides" => {
            let s = crate::preview::slides(path).ok()?;
            Some(s.iter().map(|s| format!("{}\n{}", s.title, s.lines.join("\n"))).collect::<Vec<_>>().join("\n"))
        }
        _ => None,
    }
}

#[cfg(test)]
mod sample_tests {
    /// Prints metadata of files listed (one per line) in $DUALION_META_FILES.
    #[test]
    #[ignore]
    fn print_samples() {
        let list = std::fs::read_to_string(std::env::var("DUALION_META_FILES").unwrap()).unwrap();
        for p in list.lines().filter(|l| !l.is_empty()) {
            let t = std::time::Instant::now();
            let m = super::extract(std::path::Path::new(p));
            println!("{p}\n  {:?} {}\n", t.elapsed(), serde_json::to_string(&m).unwrap());
        }
    }
}

#[cfg(test)]
mod pdf_text_tests {
    /// DUALION_PDF=<file> cargo test pdf_text_live -- --ignored --nocapture
    #[test]
    #[ignore]
    fn pdf_text_live() {
        let p = std::env::var("DUALION_PDF").unwrap();
        let t = super::text_content(std::path::Path::new(&p)).unwrap_or_default();
        println!("CHARS={} SAMPLE={:?}", t.len(), t.split_whitespace().take(40).collect::<Vec<_>>().join(" "));
    }
}
