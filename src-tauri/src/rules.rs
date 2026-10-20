//! Folder rules ("Hazel" style): watch a folder, match new files with the
//! query language and move / copy / rename / trash them. Every change is
//! journaled so it can be undone from the operation history.

use crate::error::{AppError, AppResult};
use crate::jobs::undo::{self, UndoAction};
use crate::jobs::EventSink;
use crate::query::{FileSubject, Query, Subject};
use crate::util::{local_parts, system_time_ms};
use notify::{RecommendedWatcher, RecursiveMode, Watcher};
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Weak};
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Rule {
    pub id: String,
    pub name: String,
    pub enabled: bool,
    pub folder: String,
    pub recursive: bool,
    /// Condition in the query language; empty = every file.
    pub query: String,
    /// "move" | "copy" | "trash" | "rename"
    pub action: String,
    /// Destination folder template (move / copy).
    pub target: String,
    /// New name template (rename; optional for move / copy).
    pub rename: String,
    /// Run on new files automatically (otherwise only on demand).
    pub auto: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct RulesConfig {
    pub enabled: bool,
    pub rules: Vec<Rule>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RuleActivity {
    pub time: i64,
    pub rule: String,
    pub from: String,
    pub to: Option<String>,
    pub action: String,
    pub error: Option<String>,
    pub undo: Vec<UndoAction>,
}

pub struct RulesEngine {
    config: Mutex<RulesConfig>,
    watcher: Mutex<Option<RecommendedWatcher>>,
    /// Changed path -> first seen; processed once stable.
    pending: Arc<Mutex<HashMap<PathBuf, Instant>>>,
    /// Paths the rules just produced (not processed again).
    produced: Mutex<HashMap<PathBuf, Instant>>,
    log: Mutex<Vec<RuleActivity>>,
    sink: EventSink,
}

const SETTLE: Duration = Duration::from_secs(3);

impl RulesEngine {
    pub fn new(sink: EventSink) -> Arc<Self> {
        let me = Arc::new(RulesEngine {
            config: Mutex::new(RulesConfig::default()),
            watcher: Mutex::new(None),
            pending: Arc::new(Mutex::new(HashMap::new())),
            produced: Mutex::new(HashMap::new()),
            log: Mutex::new(Vec::new()),
            sink,
        });
        let weak: Weak<Self> = Arc::downgrade(&me);
        std::thread::Builder::new()
            .name("rules".into())
            .spawn(move || loop {
                std::thread::sleep(Duration::from_millis(800));
                let Some(me) = weak.upgrade() else { break };
                me.tick();
            })
            .expect("rules thread");
        me
    }

    pub fn configure(&self, cfg: RulesConfig) {
        let mut cur = self.config.lock();
        if *cur == cfg {
            return;
        }
        *cur = cfg.clone();
        drop(cur);
        *self.watcher.lock() = None;
        if !cfg.enabled {
            return;
        }
        let pending = self.pending.clone();
        let Ok(mut w) = notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
            if let Ok(ev) = res {
                if matches!(ev.kind, notify::EventKind::Create(_) | notify::EventKind::Modify(_)) {
                    let mut p = pending.lock();
                    for path in ev.paths {
                        p.entry(path).or_insert_with(Instant::now);
                    }
                }
            }
        }) else {
            return;
        };
        for r in cfg.rules.iter().filter(|r| r.enabled && r.auto) {
            let dir = crate::util::expand_user_path(&r.folder);
            let mode = if r.recursive { RecursiveMode::Recursive } else { RecursiveMode::NonRecursive };
            let _ = w.watch(&dir, mode);
        }
        *self.watcher.lock() = Some(w);
    }

    pub fn log(&self) -> Vec<RuleActivity> {
        self.log.lock().clone()
    }

    /// Processes changes that have settled.
    fn tick(&self) {
        let ready: Vec<PathBuf> = {
            let mut p = self.pending.lock();
            let now = Instant::now();
            let ready: Vec<PathBuf> = p.iter().filter(|(_, t)| now.duration_since(**t) >= SETTLE).map(|(k, _)| k.clone()).collect();
            for k in &ready {
                p.remove(k);
            }
            ready
        };
        if ready.is_empty() {
            return;
        }
        self.produced.lock().retain(|_, t| t.elapsed() < Duration::from_secs(30));
        let cfg = self.config.lock().clone();
        if !cfg.enabled {
            return;
        }
        for path in ready {
            if self.produced.lock().contains_key(&path) {
                continue;
            }
            // Still being written (size changing)? Look again later.
            let Ok(md) = std::fs::metadata(&path) else { continue };
            if md.is_dir() || name_is_temp(&path) {
                continue;
            }
            if md.modified().map(|m| m.elapsed().unwrap_or_default() < Duration::from_secs(2)).unwrap_or(false) {
                self.pending.lock().insert(path, Instant::now());
                continue;
            }
            for rule in cfg.rules.iter().filter(|r| r.enabled && r.auto) {
                if !in_rule_folder(rule, &path) {
                    continue;
                }
                if let Some(a) = self.apply(rule, &path, false) {
                    let moved = a.to.is_some() || a.action == "trash";
                    self.record(a);
                    if moved {
                        break;
                    }
                }
            }
        }
    }

    fn record(&self, a: RuleActivity) {
        if let Ok(v) = serde_json::to_value(&a) {
            (self.sink)("rules-activity", v);
        }
        let mut log = self.log.lock();
        log.insert(0, a);
        log.truncate(300);
    }

    /// Applies one rule to one file (or only plans it). None = rule does not match.
    fn apply(&self, rule: &Rule, path: &Path, dry: bool) -> Option<RuleActivity> {
        let md = std::fs::metadata(path).ok()?;
        let subject = FileSubject::new(path.to_string_lossy().into_owned(), md.is_dir(), md.len(), md.modified().map(system_time_ms).unwrap_or(0));
        if md.is_dir() {
            return None;
        }
        if !rule.query.trim().is_empty() {
            let q = Query::parse(&rule.query).ok()?;
            if !q.matches(&subject) {
                return None;
            }
        }
        let mut act = RuleActivity {
            time: crate::util::now_ms(),
            rule: rule.name.clone(),
            from: subject.path.clone(),
            to: None,
            action: rule.action.clone(),
            error: None,
            undo: Vec::new(),
        };
        let res: AppResult<()> = (|| {
            match rule.action.as_str() {
                "trash" => {
                    if !dry {
                        let to = undo::trash(&subject.path)?;
                        act.undo.push(UndoAction::Trashed { path: subject.path.clone(), trashed_to: to, time: crate::util::now_ms() });
                    }
                }
                "move" | "copy" | "rename" => {
                    let dir = if rule.action == "rename" {
                        path.parent().map(Path::to_path_buf).unwrap_or_default()
                    } else {
                        let t = expand(&rule.target, &subject);
                        if t.trim().is_empty() {
                            return Err(AppError::invalid("Missing target folder"));
                        }
                        crate::util::expand_user_path(&t)
                    };
                    let name = if rule.rename.trim().is_empty() { subject.name.clone() } else { sanitize(&expand(&rule.rename, &subject)) };
                    if name.is_empty() {
                        return Err(AppError::invalid("Empty file name"));
                    }
                    let mut target = dir.join(&name);
                    if target == path {
                        return Ok(());
                    }
                    if target.exists() {
                        let unique = crate::util::unique_name(&name, |n| dir.join(n).exists());
                        target = dir.join(unique);
                    }
                    act.to = Some(target.to_string_lossy().into_owned());
                    if dry {
                        return Ok(());
                    }
                    std::fs::create_dir_all(&dir).map_err(|e| AppError::io(e, &dir))?;
                    self.produced.lock().insert(target.clone(), Instant::now());
                    let to = target.to_string_lossy().into_owned();
                    if rule.action == "copy" {
                        std::fs::copy(path, &target).map_err(|e| AppError::io(e, path))?;
                        act.undo.push(UndoAction::Created { path: to });
                    } else {
                        if std::fs::rename(path, &target).is_err() {
                            std::fs::copy(path, &target).map_err(|e| AppError::io(e, path))?;
                            std::fs::remove_file(path).map_err(|e| AppError::io(e, path))?;
                        }
                        act.undo.push(UndoAction::Moved { from: subject.path.clone(), to });
                    }
                }
                other => return Err(AppError::invalid(format!("Unknown action {other}"))),
            }
            Ok(())
        })();
        if let Err(e) = res {
            act.error = Some(e.message);
        }
        Some(act)
    }

    /// Runs (or previews) a rule on the files already in its folder.
    pub fn run_now(&self, rule: &Rule, dry: bool) -> AppResult<Vec<RuleActivity>> {
        let dir = crate::util::expand_user_path(&rule.folder);
        if !dir.is_dir() {
            return Err(AppError::invalid("The rule folder does not exist").with_path(&dir));
        }
        let mut files = Vec::new();
        collect(&dir, rule.recursive, &mut files, 0);
        let mut out = Vec::new();
        for f in files.into_iter().take(20_000) {
            if let Some(a) = self.apply(rule, &f, dry) {
                if !dry {
                    self.record(a.clone());
                }
                out.push(a);
            }
        }
        Ok(out)
    }
}

fn collect(dir: &Path, recursive: bool, out: &mut Vec<PathBuf>, depth: usize) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let p = e.path();
        let Ok(ft) = e.file_type() else { continue };
        if ft.is_dir() {
            if recursive && depth < 32 {
                collect(&p, true, out, depth + 1);
            }
        } else if ft.is_file() && !name_is_temp(&p) {
            out.push(p);
        }
    }
}

fn in_rule_folder(rule: &Rule, path: &Path) -> bool {
    let dir = crate::util::expand_user_path(&rule.folder);
    match path.parent() {
        Some(p) if p == dir => true,
        Some(p) => rule.recursive && p.starts_with(&dir),
        None => false,
    }
}

/// Partial downloads and editor temp files.
fn name_is_temp(p: &Path) -> bool {
    let n = p.file_name().map(|n| n.to_string_lossy().to_lowercase()).unwrap_or_default();
    n.starts_with('.') || n.starts_with("~$") || [".crdownload", ".part", ".download", ".tmp", ".partial"].iter().any(|s| n.ends_with(s))
}

fn sanitize(name: &str) -> String {
    name.chars().map(|c| if matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|') { '_' } else { c }).collect::<String>().trim().to_string()
}

/// Fills `{name} {ext} {year} {month} {day} {type} {camera} {artist} {album}`.
pub fn expand(template: &str, s: &dyn Subject) -> String {
    let name = s.name();
    let (stem, ext) = match name.rfind('.') {
        Some(i) if i > 0 => (&name[..i], &name[i + 1..]),
        _ => (name, ""),
    };
    let m = s.meta();
    // Photos are filed by the date taken, everything else by modification.
    let when = m.and_then(|m| m.taken).map(|t| t - local_offset(t)).unwrap_or(s.mtime());
    let (y, mo, d, ..) = local_parts(when);
    let vars: HashMap<&str, String> = HashMap::from([
        ("name", stem.to_string()),
        ("ext", ext.to_string()),
        ("year", format!("{y:04}")),
        ("month", format!("{mo:02}")),
        ("day", format!("{d:02}")),
        ("type", crate::meta::kind_of(&ext.to_lowercase(), false).to_string()),
        ("camera", m.and_then(|m| m.camera.clone()).unwrap_or_default()),
        ("artist", m.and_then(|m| m.artist.clone()).unwrap_or_default()),
        ("album", m.and_then(|m| m.album.clone()).unwrap_or_default()),
    ]);
    let mut out = String::new();
    let mut rest = template;
    while let Some(i) = rest.find('{') {
        out.push_str(&rest[..i]);
        match rest[i..].find('}') {
            Some(j) => {
                let key = &rest[i + 1..i + j];
                match vars.get(key.trim().to_lowercase().as_str()) {
                    Some(v) => out.push_str(&sanitize(v)),
                    None => out.push_str(&rest[i..i + j + 1]),
                }
                rest = &rest[i + j + 1..];
            }
            None => {
                out.push_str(&rest[i..]);
                rest = "";
            }
        }
    }
    out.push_str(rest);
    out
}

/// EXIF times are wall-clock times stored as UTC; undo the local shift.
fn local_offset(ms: i64) -> i64 {
    let (y, mo, d, h, mi, s) = crate::util::utc_parts(ms);
    crate::util::local_naive_to_ms(y, mo, d, h, mi, s) - ms
}

pub fn check(rule: &Rule) -> AppResult<()> {
    if !rule.query.trim().is_empty() {
        Query::parse(&rule.query)?;
    }
    if matches!(rule.action.as_str(), "move" | "copy") && rule.target.trim().is_empty() {
        return Err(AppError::invalid("Missing target folder"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rule(dir: &Path, action: &str, query: &str, target: &str, rename: &str) -> Rule {
        Rule {
            id: "r".into(),
            name: "test".into(),
            enabled: true,
            folder: dir.to_string_lossy().into_owned(),
            recursive: false,
            query: query.into(),
            action: action.into(),
            target: target.into(),
            rename: rename.into(),
            auto: false,
        }
    }

    #[test]
    fn moves_matching_files_by_template() {
        let root = std::env::temp_dir().join(format!("dualion-rules-{}", uuid::Uuid::new_v4()));
        let inbox = root.join("inbox");
        std::fs::create_dir_all(&inbox).unwrap();
        std::fs::write(inbox.join("Faktura 12.pdf"), "%PDF-1.4").unwrap();
        std::fs::write(inbox.join("photo.txt"), "x").unwrap();
        let eng = RulesEngine::new(Arc::new(|_, _| {}));
        let target = format!("{}/{{type}}/{{year}}", root.join("archive").to_string_lossy());
        let r = rule(&inbox, "move", "ext:pdf faktura", &target, "{name} (scan).{ext}");
        let plan = eng.run_now(&r, true).unwrap();
        assert_eq!(plan.len(), 1);
        assert!(inbox.join("Faktura 12.pdf").exists(), "dry run must not touch files");
        let done = eng.run_now(&r, false).unwrap();
        let to = done[0].to.clone().unwrap();
        assert!(to.contains("/pdf/") || to.contains("\\pdf\\"));
        assert!(to.ends_with("Faktura 12 (scan).pdf"));
        assert!(Path::new(&to).exists() && !inbox.join("Faktura 12.pdf").exists());
        assert!(matches!(done[0].undo[0], UndoAction::Moved { .. }));
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn templates() {
        let s = FileSubject::new("/x/My Song.mp3".into(), false, 1, crate::util::local_naive_to_ms(2024, 3, 9, 12, 0, 0));
        assert_eq!(expand("{type}/{year}-{month}/{name}.{ext} {unknown}", &s), "audio/2024-03/My Song.mp3 {unknown}");
    }
}
