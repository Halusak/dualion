//! File search: parallel traversal (ripgrep's `ignore` walker) with name
//! masks or regexes, size/date filters and content search via grep-searcher.
//! Results are streamed to the UI in batches over a Tauri channel.

use crate::error::{AppError, AppResult};
use crate::filter::NameFilter;
use crate::local;
use crate::util::system_time_ms;
use grep_regex::{RegexMatcher, RegexMatcherBuilder};
use grep_searcher::{sinks, BinaryDetection, Searcher, SearcherBuilder};
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tauri::ipc::Channel;

#[derive(Debug, Clone, Copy, Deserialize, Default, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum SearchKind {
    #[default]
    All,
    Files,
    Dirs,
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct SearchParams {
    pub roots: Vec<String>,
    pub name: String,
    pub name_regex: bool,
    pub name_case: bool,
    pub text: String,
    pub text_regex: bool,
    pub text_case: bool,
    pub text_word: bool,
    /// Report files that do NOT contain the text.
    pub text_not: bool,
    pub encoding: String,
    pub binary: bool,
    pub min_size: Option<u64>,
    pub max_size: Option<u64>,
    pub newer_than: Option<i64>,
    pub older_than: Option<i64>,
    pub max_depth: Option<usize>,
    pub hidden: bool,
    pub follow_links: bool,
    pub gitignore: bool,
    pub exclude_dirs: String,
    pub kind: SearchKind,
    pub max_results: Option<usize>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LineHit {
    pub line: u64,
    pub text: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Hit {
    pub path: String,
    pub name: String,
    pub is_dir: bool,
    pub size: u64,
    pub mtime: i64,
    pub matches: Vec<LineHit>,
    pub match_count: u32,
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum SearchEvent {
    Hits { items: Vec<Hit> },
    Progress { scanned: u64, current: String },
    #[serde(rename_all = "camelCase")]
    Done { found: u64, scanned: u64, cancelled: bool, truncated: bool, elapsed_ms: u64 },
}

enum NameMatch {
    Any,
    Mask(NameFilter),
    Regex(regex::Regex),
}

impl NameMatch {
    fn matches(&self, name: &str) -> bool {
        match self {
            NameMatch::Any => true,
            NameMatch::Mask(f) => f.matches(name),
            NameMatch::Regex(r) => r.is_match(name),
        }
    }
}

struct Compiled {
    name: NameMatch,
    exclude_dirs: Option<NameFilter>,
    text: Option<RegexMatcher>,
}

fn compile(p: &SearchParams) -> AppResult<Compiled> {
    let name = if p.name.trim().is_empty() {
        NameMatch::Any
    } else if p.name_regex {
        NameMatch::Regex(
            regex::RegexBuilder::new(p.name.trim())
                .case_insensitive(!p.name_case)
                .build()
                .map_err(|e| AppError::invalid(e.to_string()))?,
        )
    } else {
        let f = NameFilter::parse(&p.name, p.name_case)?;
        if f.is_noop() { NameMatch::Any } else { NameMatch::Mask(f) }
    };
    let exclude_dirs = if p.exclude_dirs.trim().is_empty() {
        None
    } else {
        // Every pattern is an exclusion, hence the leading '|'.
        Some(NameFilter::parse(&format!("|{}", p.exclude_dirs), false)?)
    };
    let text = if p.text.is_empty() {
        None
    } else {
        Some(
            RegexMatcherBuilder::new()
                .case_insensitive(!p.text_case)
                .word(p.text_word)
                .fixed_strings(!p.text_regex)
                .build(&p.text)
                .map_err(|e| AppError::invalid(e.to_string()))?,
        )
    };
    Ok(Compiled { name, exclude_dirs, text })
}

fn build_searcher(binary: bool, encoding: Option<&str>) -> Searcher {
    let mut b = SearcherBuilder::new();
    b.line_number(true).bom_sniffing(true);
    b.binary_detection(if binary { BinaryDetection::none() } else { BinaryDetection::quit(b'\x00') });
    if let Some(e) = encoding.and_then(|e| grep_searcher::Encoding::new(e).ok()) {
        b.encoding(Some(e));
    }
    b.build()
}

fn is_auto(enc: &str) -> bool {
    let e = enc.trim();
    e.is_empty() || e.eq_ignore_ascii_case("auto") || e.eq_ignore_ascii_case("utf-8")
}

/// Decodes a matched line: UTF-8 when valid, otherwise the most likely
/// legacy encoding (old Czech texts are usually windows-1250).
fn decode_line(bytes: &[u8]) -> String {
    match std::str::from_utf8(bytes) {
        Ok(s) => s.to_string(),
        Err(_) => {
            let name = crate::viewer::detect_encoding(bytes);
            let enc = encoding_rs::Encoding::for_label(name.as_bytes()).unwrap_or(encoding_rs::WINDOWS_1250);
            enc.decode_without_bom_handling(bytes).0.into_owned()
        }
    }
}

/// Legacy (non UTF-8, no BOM) encoding of a text file, if it has one.
fn legacy_encoding(path: &std::path::Path) -> Option<&'static encoding_rs::Encoding> {
    use std::io::Read;
    let mut head = vec![0u8; 64 * 1024];
    let n = std::fs::File::open(path).ok()?.read(&mut head).ok()?;
    head.truncate(n);
    let name = crate::viewer::detect_encoding(&head);
    let enc = encoding_rs::Encoding::for_label(name.as_bytes())?;
    (enc != encoding_rs::UTF_8 && enc != encoding_rs::UTF_16LE && enc != encoding_rs::UTF_16BE).then_some(enc)
}

/// Per-thread content searcher. In automatic mode a file that is not UTF-8
/// is searched again in its detected legacy encoding when the pattern
/// contains non-ASCII characters (diacritics).
struct Grepper {
    main: Searcher,
    legacy: std::collections::HashMap<&'static str, Searcher>,
    auto: bool,
    non_ascii: bool,
    binary: bool,
}

impl Grepper {
    fn new(p: &SearchParams) -> Self {
        let auto = is_auto(&p.encoding);
        Self {
            main: build_searcher(p.binary, if auto { None } else { Some(p.encoding.trim()) }),
            legacy: std::collections::HashMap::new(),
            auto,
            non_ascii: !p.text.is_ascii(),
            binary: p.binary,
        }
    }

    fn grep(&mut self, m: &RegexMatcher, path: &std::path::Path, stop_at_first: bool) -> (u32, Vec<LineHit>) {
        let found = grep_with(&mut self.main, m, path, stop_at_first);
        if found.0 == 0 && self.auto && self.non_ascii {
            if let Some(enc) = legacy_encoding(path) {
                let binary = self.binary;
                let s = self.legacy.entry(enc.name()).or_insert_with(|| build_searcher(binary, Some(enc.name())));
                return grep_with(s, m, path, stop_at_first);
            }
        }
        found
    }
}

/// Returns (match_count, first lines) for `path`.
fn grep_with(s: &mut Searcher, m: &RegexMatcher, path: &std::path::Path, stop_at_first: bool) -> (u32, Vec<LineHit>) {
    let mut count = 0u32;
    let mut lines = Vec::new();
    let _ = s.search_path(
        m,
        path,
        sinks::Bytes(|line_no, bytes| {
            count += 1;
            if lines.len() < 5 {
                let text = decode_line(bytes);
                let t = text.trim_end_matches(['\r', '\n']);
                let t: String = if t.chars().count() > 300 { t.chars().take(300).collect::<String>() + "…" } else { t.to_string() };
                lines.push(LineHit { line: line_no, text: t });
            }
            Ok(!stop_at_first && count < 1000)
        }),
    );
    (count, lines)
}

pub fn run(params: SearchParams, channel: Channel<SearchEvent>, cancel: Arc<AtomicBool>) -> AppResult<()> {
    let compiled = Arc::new(compile(&params)?);
    let roots: Vec<_> = params.roots.iter().map(|r| local::normalize(r)).filter(|r| r.exists()).collect();
    if roots.is_empty() {
        return Err(AppError::invalid("No folder to search in"));
    }
    std::thread::Builder::new()
        .name("search".into())
        .spawn(move || {
            let started = Instant::now();
            let params = Arc::new(params);
            let pending: Arc<Mutex<Vec<Hit>>> = Arc::new(Mutex::new(Vec::new()));
            let current: Arc<Mutex<String>> = Arc::new(Mutex::new(String::new()));
            let scanned = Arc::new(AtomicU64::new(0));
            let found = Arc::new(AtomicU64::new(0));
            let truncated = Arc::new(AtomicBool::new(false));
            let finished = Arc::new(AtomicBool::new(false));
            let limit = params.max_results.unwrap_or(100_000) as u64;

            // Flusher: batches hits so the UI is not flooded with messages.
            let flusher = {
                let (pending, channel, finished, scanned, current) =
                    (pending.clone(), channel.clone(), finished.clone(), scanned.clone(), current.clone());
                std::thread::spawn(move || {
                    let mut last_progress = Instant::now();
                    loop {
                        let done = finished.load(Ordering::Relaxed);
                        let batch: Vec<Hit> = std::mem::take(&mut *pending.lock());
                        if !batch.is_empty() {
                            let _ = channel.send(SearchEvent::Hits { items: batch });
                        }
                        if done {
                            break;
                        }
                        if last_progress.elapsed() > Duration::from_millis(250) {
                            let cur = current.lock().clone();
                            let _ = channel.send(SearchEvent::Progress { scanned: scanned.load(Ordering::Relaxed), current: cur });
                            last_progress = Instant::now();
                        }
                        std::thread::sleep(Duration::from_millis(120));
                    }
                })
            };

            let mut wb = ignore::WalkBuilder::new(&roots[0]);
            for r in &roots[1..] {
                wb.add(r);
            }
            wb.standard_filters(false)
                .hidden(!params.hidden)
                .git_ignore(params.gitignore)
                .git_global(params.gitignore)
                .git_exclude(params.gitignore)
                .ignore(params.gitignore)
                .parents(params.gitignore)
                .follow_links(params.follow_links)
                .max_depth(params.max_depth)
                .threads(std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4).min(12));
            if let Some(ex) = compiled.exclude_dirs.clone() {
                wb.filter_entry(move |e| {
                    !(e.file_type().map(|t| t.is_dir()).unwrap_or(false) && ex.excluded(&e.file_name().to_string_lossy()))
                });
            }

            wb.build_parallel().run(|| {
                let (params, compiled, pending, current, scanned, found, truncated, cancel) = (
                    params.clone(),
                    compiled.clone(),
                    pending.clone(),
                    current.clone(),
                    scanned.clone(),
                    found.clone(),
                    truncated.clone(),
                    cancel.clone(),
                );
                let mut grepper = Grepper::new(&params);
                Box::new(move |res| {
                    if cancel.load(Ordering::Relaxed) {
                        return ignore::WalkState::Quit;
                    }
                    let Ok(entry) = res else { return ignore::WalkState::Continue };
                    if entry.depth() == 0 {
                        return ignore::WalkState::Continue;
                    }
                    let n = scanned.fetch_add(1, Ordering::Relaxed);
                    let is_dir = entry.file_type().map(|t| t.is_dir()).unwrap_or(false);
                    if is_dir && n % 64 == 0 {
                        if let Some(mut c) = current.try_lock() {
                            *c = entry.path().to_string_lossy().into_owned();
                        }
                    }
                    match params.kind {
                        SearchKind::Files if is_dir => return ignore::WalkState::Continue,
                        SearchKind::Dirs if !is_dir => return ignore::WalkState::Continue,
                        _ => {}
                    }
                    if is_dir && compiled.text.is_some() {
                        return ignore::WalkState::Continue;
                    }
                    let name = entry.file_name().to_string_lossy();
                    if !compiled.name.matches(&name) {
                        return ignore::WalkState::Continue;
                    }
                    let Ok(meta) = entry.metadata() else { return ignore::WalkState::Continue };
                    let size = if is_dir { 0 } else { meta.len() };
                    if !is_dir && (params.min_size.is_some_and(|m| size < m) || params.max_size.is_some_and(|m| size > m)) {
                        return ignore::WalkState::Continue;
                    }
                    let mtime = meta.modified().map(system_time_ms).unwrap_or(0);
                    if params.newer_than.is_some_and(|t| mtime < t) || params.older_than.is_some_and(|t| mtime > t) {
                        return ignore::WalkState::Continue;
                    }
                    let (mut count, mut lines) = (0, Vec::new());
                    if let Some(m) = &compiled.text {
                        let (c, l) = grepper.grep(m, entry.path(), params.text_not);
                        if params.text_not {
                            if c > 0 {
                                return ignore::WalkState::Continue;
                            }
                        } else if c == 0 {
                            return ignore::WalkState::Continue;
                        }
                        count = c;
                        lines = l;
                    }
                    let hit = Hit {
                        path: entry.path().to_string_lossy().into_owned(),
                        name: name.into_owned(),
                        is_dir,
                        size,
                        mtime,
                        matches: lines,
                        match_count: count,
                    };
                    pending.lock().push(hit);
                    if found.fetch_add(1, Ordering::Relaxed) + 1 >= limit {
                        truncated.store(true, Ordering::Relaxed);
                        return ignore::WalkState::Quit;
                    }
                    ignore::WalkState::Continue
                })
            });

            finished.store(true, Ordering::Relaxed);
            let _ = flusher.join();
            let _ = channel.send(SearchEvent::Done {
                found: found.load(Ordering::Relaxed),
                scanned: scanned.load(Ordering::Relaxed),
                cancelled: cancel.load(Ordering::Relaxed),
                truncated: truncated.load(Ordering::Relaxed),
                elapsed_ms: started.elapsed().as_millis() as u64,
            });
        })
        .map_err(|e| AppError::other(e.to_string()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn grep_finds_text_and_cp1250() {
        let dir = std::env::temp_dir().join(format!("dualion-search-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("a.txt"), "first\nHello World\nlast\n").unwrap();
        let (bytes, _, _) = encoding_rs::WINDOWS_1250.encode("Příliš žluťoučký kůň\n");
        assert!(std::str::from_utf8(&bytes).is_err());
        fs::write(dir.join("cz.txt"), &bytes).unwrap();

        let p = SearchParams { text: "hello".into(), ..Default::default() };
        let c = compile(&p).unwrap();
        let mut g = Grepper::new(&p);
        let (count, lines) = g.grep(c.text.as_ref().unwrap(), &dir.join("a.txt"), false);
        assert_eq!(count, 1);
        assert_eq!(lines[0].line, 2);
        assert_eq!(lines[0].text, "Hello World");

        // Explicit encoding
        let p = SearchParams { text: "žluťoučký".into(), encoding: "windows-1250".into(), ..Default::default() };
        let c = compile(&p).unwrap();
        let mut g = Grepper::new(&p);
        assert_eq!(g.grep(c.text.as_ref().unwrap(), &dir.join("cz.txt"), false).0, 1);

        // Automatic mode finds diacritics in a windows-1250 file and decodes the line.
        let p = SearchParams { text: "žluťoučký".into(), ..Default::default() };
        let c = compile(&p).unwrap();
        let mut g = Grepper::new(&p);
        let (n, lines) = g.grep(c.text.as_ref().unwrap(), &dir.join("cz.txt"), false);
        assert_eq!(n, 1);
        assert_eq!(lines[0].text, "Příliš žluťoučký kůň");

        // ASCII match in a legacy file still shows readable text.
        let p = SearchParams { text: "kůň".into(), ..Default::default() };
        let c = compile(&p).unwrap();
        assert_eq!(Grepper::new(&p).grep(c.text.as_ref().unwrap(), &dir.join("cz.txt"), false).0, 1);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn name_matching() {
        let c = compile(&SearchParams { name: "*.rs;*.toml | test*".into(), ..Default::default() }).unwrap();
        assert!(c.name.matches("main.rs"));
        assert!(!c.name.matches("test_main.rs"));
        let c = compile(&SearchParams { name: r"^\d{4}-.*\.jpg$".into(), name_regex: true, ..Default::default() }).unwrap();
        assert!(c.name.matches("2024-dovolena.JPG"));
    }
}
