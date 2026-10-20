//! Natural-language search ("najdi všechny .dart soubory z loňska",
//! "big videos from this week", "faktury za hosting"): the request is turned
//! into the query language (filters) plus a free-text topic. A local chat
//! model through Ollama does it when available, otherwise rules do.

use crate::query::{fold, Query};
use crate::util::{local_parts, now_ms};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Interpretation {
    /// Filters in the query language ("ext:dart modified:2025").
    pub query: String,
    /// What the content should be about (for semantic search), may be empty.
    pub topic: String,
    /// "llm" | "rules"
    pub via: &'static str,
}

const STOP: &[&str] = &[
    "najdi", "najit", "najdete", "hledej", "hledat", "vyhledej", "ukaz", "ukazte", "zobraz", "chci", "potrebuji", "mi", "me", "prosim",
    "vsechny", "vsechna", "vsechno", "vse", "vsech", "soubory", "souboru", "soubor", "souborech", "ktere", "ktery", "ktera", "co", "kde",
    "jsou", "je", "se", "s", "z", "ze", "do", "na", "v", "ve", "o", "a", "nebo", "the", "a", "an", "all", "find", "show", "me", "my",
    "files", "file", "search", "for", "please", "with", "of", "in", "from", "that", "which", "are", "is", "and", "or", "about", "to", "nejake",
    "nejaky", "some", "any", "typu", "type", "pripona", "priponou", "extension", "moje", "muj", "moji", "mych",
];

fn kind_word(w: &str) -> Option<&'static str> {
    Some(match w {
        "fotky" | "fotek" | "fotka" | "fotografie" | "fotografii" | "obrazky" | "obrazku" | "obrazek" | "images" | "image" | "photos" | "photo"
        | "pictures" | "picture" | "screenshoty" | "screenshots" => "image",
        "videa" | "video" | "videi" | "videos" | "filmy" | "filmu" | "film" | "movies" | "klipy" => "video",
        "hudba" | "hudbu" | "hudby" | "pisnicky" | "pisne" | "songs" | "song" | "music" | "audio" | "zvuky" | "nahravky" => "audio",
        "dokumenty" | "dokumentu" | "dokument" | "documents" | "document" | "docs" => "document",
        "tabulky" | "tabulek" | "spreadsheets" | "sheets" | "excely" => "sheet",
        "prezentace" | "prezentaci" | "presentations" | "slides" => "slides",
        "archivy" | "archivu" | "archives" | "zipy" => "archive",
        "slozky" | "slozek" | "slozka" | "adresare" | "folders" | "folder" | "directories" => "folder",
        "kod" | "kodu" | "zdrojaky" | "zdrojovy" | "zdrojove" | "code" | "source" | "skripty" | "scripts" => "code",
        "pisma" | "fonty" | "fonts" => "font",
        _ => return None,
    })
}

/// Extensions people say as words ("pdf soubory", "dart files").
const EXT_WORDS: &[&str] = &[
    "pdf", "dart", "rs", "py", "js", "ts", "tsx", "jsx", "java", "kt", "swift", "go", "php", "rb", "cs", "cpp", "c", "h", "html", "css", "json",
    "yaml", "yml", "toml", "xml", "sql", "md", "txt", "csv", "doc", "docx", "xls", "xlsx", "ppt", "pptx", "odt", "jpg", "jpeg", "png", "gif",
    "heic", "webp", "svg", "psd", "raw", "cr2", "nef", "dng", "mp4", "mov", "mkv", "avi", "mp3", "flac", "wav", "m4a", "zip", "rar", "7z",
    "iso", "dmg", "exe", "msi", "apk", "epub", "log", "sh", "bat", "ps1", "dartx",
];

fn size_value(num: &str, unit: &str) -> Option<String> {
    let n: f64 = num.replace(',', ".").parse().ok()?;
    let u = match unit {
        "kb" | "k" | "kib" => "kb",
        "mb" | "m" | "mib" | "mega" => "mb",
        "gb" | "g" | "gib" | "giga" => "gb",
        "tb" | "t" => "tb",
        "b" | "bajtu" | "bytes" => "",
        _ => return None,
    };
    Some(format!("{n}{u}"))
}

/// Rule based interpretation (Czech and English).
pub fn rules(text: &str) -> Interpretation {
    let raw = text.trim().trim_start_matches('?').trim();
    // Explicit query syntax is passed through.
    if (raw.contains(':') || raw.contains('>') || raw.contains('<')) && Query::parse(raw).is_ok() {
        return Interpretation { query: raw.to_string(), topic: String::new(), via: "rules" };
    }
    let f = fold(raw);
    let words: Vec<String> = f
        .split(|c: char| c.is_whitespace() || matches!(c, ',' | ';' | '!' | '?' | '"' | '(' | ')'))
        .filter(|w| !w.is_empty())
        .map(|w| w.trim_end_matches('.').to_string())
        .collect();
    let mut filters: Vec<String> = Vec::new();
    let mut exts: Vec<String> = Vec::new();
    let mut kinds: Vec<&str> = Vec::new();
    let mut used = vec![false; words.len()];
    let (year, ..) = local_parts(now_ms());
    let mut i = 0;
    while i < words.len() {
        let w = words[i].as_str();
        let next = words.get(i + 1).map(String::as_str).unwrap_or("");
        let next2 = words.get(i + 2).map(String::as_str).unwrap_or("");
        let take = |n: usize, used: &mut Vec<bool>| {
            for u in used.iter_mut().skip(i).take(n) {
                *u = true;
            }
        };
        // ".dart", "*.dart"
        if let Some(e) = w.trim_start_matches('*').strip_prefix('.') {
            if !e.is_empty() && e.len() <= 8 && e.chars().all(|c| c.is_ascii_alphanumeric()) {
                exts.push(e.to_string());
                take(1, &mut used);
            }
        } else if let Some(k) = kind_word(w) {
            kinds.push(k);
            take(1, &mut used);
        } else if EXT_WORDS.contains(&w) && (matches!(next, "soubory" | "souboru" | "soubor" | "files" | "file" | "dokumenty") || words.len() <= 3) {
            exts.push(w.to_string());
            take(1, &mut used);
        } else if matches!(w, "dnes" | "dneska" | "today") {
            filters.push("modified:today".into());
            take(1, &mut used);
        } else if matches!(w, "vcera" | "yesterday") {
            filters.push("modified:yesterday".into());
            take(1, &mut used);
        } else if matches!(w, "letos" | "letosni" | "letosnich") || (w == "this" && next == "year") || (w == "tento" && next == "rok") {
            filters.push(format!("modified:{year}"));
            take(if w == "letos" || w.starts_with("letosni") { 1 } else { 2 }, &mut used);
        } else if matches!(w, "loni" | "lonsky" | "lonske" | "lonskych" | "lonska" | "lonskeho") || (w == "last" && next == "year") || (w == "minuly" && next == "rok") {
            filters.push(format!("modified:{}", year - 1));
            take(if w == "last" || w == "minuly" { 2 } else { 1 }, &mut used);
        } else if (w == "tento" || w == "this" || w == "minuly" || w == "posledni" || w == "last") && matches!(next, "tyden" | "week" | "tydnu") {
            filters.push("modified:week".into());
            take(2, &mut used);
        } else if (w == "tento" || w == "this" || w == "minuly" || w == "posledni" || w == "last") && matches!(next, "mesic" | "month" | "mesice") {
            filters.push("modified:month".into());
            take(2, &mut used);
        } else if matches!(w, "posledni" | "poslednich" | "last" | "past") && next.parse::<u32>().is_ok() && matches!(next2, "dni" | "dny" | "days" | "dnu") {
            filters.push(format!("modified:{next}d"));
            take(3, &mut used);
        } else if w.len() == 4 && w.parse::<i32>().is_ok_and(|y| (1980..=year + 1).contains(&y)) {
            filters.push(format!("modified:{w}"));
            take(1, &mut used);
        } else if matches!(w, "vetsi" | "vetsich" | "nad" | "larger" | "bigger" | "over" | "mensi" | "mensich" | "pod" | "smaller" | "under") {
            // "větší než 100 MB", "larger than 1gb", "nad 2 GB"
            let mut j = i + 1;
            if matches!(words.get(j).map(String::as_str), Some("nez" | "than")) {
                j += 1;
            }
            let (num, unit, n) = match (words.get(j), words.get(j + 1)) {
                (Some(a), Some(b)) if a.parse::<f64>().is_ok() => (a.clone(), b.clone(), j + 2 - i),
                (Some(a), _) => {
                    let split = a.find(|c: char| c.is_alphabetic()).unwrap_or(a.len());
                    (a[..split].to_string(), a[split..].to_string(), j + 1 - i)
                }
                _ => (String::new(), String::new(), 0),
            };
            if let Some(v) = size_value(&num, &unit) {
                let op = if matches!(w, "mensi" | "mensich" | "pod" | "smaller" | "under") { "<" } else { ">" };
                filters.push(format!("size{op}{v}"));
                take(n, &mut used);
            } else if matches!(w, "vetsi" | "larger" | "bigger") && matches!(next, "soubory" | "files" | "videa" | "fotky") {
                filters.push("size>100mb".into());
                take(1, &mut used);
            }
        } else if matches!(w, "velke" | "velkych" | "obri" | "big" | "large" | "huge") {
            filters.push("size>100mb".into());
            take(1, &mut used);
        } else if matches!(w, "4k" | "uhd") {
            filters.push("res>=4k".into());
            take(1, &mut used);
        } else if matches!(w, "fullhd" | "1080p") {
            filters.push("res>=1080p".into());
            take(1, &mut used);
        }
        i += 1;
    }
    if !exts.is_empty() {
        exts.dedup();
        filters.insert(0, format!("ext:{}", exts.join(",")));
    } else if !kinds.is_empty() {
        kinds.dedup();
        filters.insert(0, format!("type:{}", kinds.join(",")));
    }
    let topic: Vec<&str> = words
        .iter()
        .zip(&used)
        .filter(|(w, u)| !**u && !STOP.contains(&w.as_str()) && w.len() > 1 && !matches!(w.as_str(), "nez" | "than" | "mb" | "gb" | "kb"))
        .map(|(w, _)| w.as_str())
        .collect();
    // Only filler words left ("za", "na"…): no topic.
    let topic = if topic.iter().all(|w| w.chars().count() <= 2) { String::new() } else { topic.join(" ") };
    Interpretation { query: filters.join(" "), topic, via: "rules" }
}

#[derive(Deserialize)]
struct LlmOut {
    #[serde(default)]
    filters: String,
    #[serde(default)]
    topic: String,
}

/// Asks a local chat model; None when it is not available or answers nonsense.
pub fn llm(endpoint: &str, model: &str, text: &str) -> Option<Interpretation> {
    let (y, m, d, ..) = local_parts(now_ms());
    let prompt = format!(
        "Convert a file search request (Czech or English) into JSON {{\"filters\": string, \"topic\": string}}.\n\
filters uses this syntax (space separated, all must match): ext:pdf,docx  type:image|video|audio|document|sheet|slides|archive|code|folder  \
size>100mb size<1gb  modified:2025 modified:2025-03 modified:today modified:yesterday modified:week modified:month modified:30d  \
taken:2024 (photo date)  width>=3000  res>=4k  duration>10m  camera:canon  artist:\"name\"  name words without key match file names.\n\
topic = what the content is about in a few words (for semantic search), empty if the request is only about names/types/dates.\n\
Today is {y}-{m:02}-{d:02}. Answer with JSON only.\n\nRequest: {text}"
    );
    #[derive(Deserialize)]
    struct R {
        response: String,
    }
    let agent: ureq::Agent = ureq::Agent::config_builder().timeout_global(Some(std::time::Duration::from_secs(30))).build().into();
    let mut res = agent
        .post(format!("{}/api/generate", endpoint.trim_end_matches('/')))
        .send_json(serde_json::json!({ "model": model, "prompt": prompt, "format": "json", "stream": false, "options": { "temperature": 0 } }))
        .ok()?;
    let r: R = res.body_mut().read_json().ok()?;
    let out: LlmOut = serde_json::from_str(r.response.trim()).ok()?;
    let filters = out.filters.trim().to_string();
    if !filters.is_empty() && Query::parse(&filters).is_err() {
        return None;
    }
    Some(Interpretation { query: filters, topic: out.topic.trim().to_string(), via: "llm" })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn q(s: &str) -> (String, String) {
        let i = rules(s);
        (i.query, i.topic)
    }

    #[test]
    fn czech_requests() {
        assert_eq!(q("najdi mi všechny .dart soubory"), ("ext:dart".into(), String::new()));
        assert_eq!(q("pdf soubory faktury za hosting"), ("ext:pdf".into(), "faktury za hosting".into()));
        let (y, ..) = local_parts(now_ms());
        assert_eq!(q("fotky z loňska").0, format!("type:image modified:{}", y - 1));
        assert_eq!(q("videa větší než 2 GB").0, "type:video size>2gb");
        assert_eq!(q("dokumenty za poslední 3 dny").0, "type:document modified:3d");
        assert_eq!(q("ext:rs size>1mb").0, "ext:rs size>1mb");
    }

    #[test]
    fn english_requests() {
        assert_eq!(q("show me big videos from this week").0, "type:video size>100mb modified:week");
        assert_eq!(q("find all *.ts and .tsx files").0, "ext:ts,tsx");
        for s in ["najdi mi všechny .dart soubory", "videa větší než 2 GB", "invoice for hosting"] {
            let i = rules(s);
            assert!(i.query.is_empty() || Query::parse(&i.query).is_ok(), "{s}: {}", i.query);
        }
    }
}
