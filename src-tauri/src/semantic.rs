//! Semantic search ("invoice for hosting from last year", "dog on a beach"):
//! documents (and optionally photo captions) are embedded by a local model
//! served by Ollama; nothing leaves the computer. Vectors are kept in memory
//! and on disk, queries are matched by cosine similarity.

use crate::error::{AppError, AppResult};
use crate::index::data::{flags, NONE};
use crate::index::scan::{self, Excludes, Progress};
use crate::jobs::EventSink;
use parking_lot::{Mutex, RwLock};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::io::{BufReader, BufWriter, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct SemanticConfig {
    pub enabled: bool,
    pub endpoint: String,
    pub model: String,
    pub roots: Vec<String>,
    /// Describe photos with a vision model (slow) so they can be found too.
    pub images: bool,
    pub vision_model: String,
}

impl Default for SemanticConfig {
    fn default() -> Self {
        SemanticConfig {
            enabled: false,
            endpoint: "http://localhost:11434".into(),
            model: "embeddinggemma".into(),
            roots: Vec::new(),
            images: false,
            vision_model: "gemma3:4b".into(),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SemanticStatus {
    /// "off" | "unavailable" | "missingModel" | "indexing" | "ready" | "error"
    pub state: &'static str,
    pub files: usize,
    pub done: usize,
    pub total: usize,
    pub error: Option<String>,
    pub models: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SemanticHit {
    pub path: String,
    pub name: String,
    pub size: u64,
    pub mtime: i64,
    pub score: f32,
    pub snippet: String,
}

struct Doc {
    path: String,
    size: u64,
    mtime: i64,
    snippet: String,
    vec: Vec<f32>,
}

pub struct SemanticEngine {
    config: Mutex<SemanticConfig>,
    docs: RwLock<Vec<Doc>>,
    state: Mutex<&'static str>,
    error: Mutex<Option<String>>,
    models: Mutex<Vec<String>>,
    done: AtomicUsize,
    total: AtomicUsize,
    generation: AtomicU64,
    file: PathBuf,
    sink: EventSink,
}

const BATCH: usize = 16;
const TEXT_CHARS: usize = 1800;

fn agent(timeout: u64) -> ureq::Agent {
    ureq::Agent::config_builder().timeout_global(Some(Duration::from_secs(timeout))).build().into()
}

impl SemanticEngine {
    pub fn new(file: PathBuf, sink: EventSink) -> Arc<Self> {
        let docs = load(&file).unwrap_or_default();
        Arc::new(SemanticEngine {
            config: Mutex::new(SemanticConfig::default()),
            docs: RwLock::new(docs),
            state: Mutex::new("off"),
            error: Mutex::new(None),
            models: Mutex::new(Vec::new()),
            done: AtomicUsize::new(0),
            total: AtomicUsize::new(0),
            generation: AtomicU64::new(0),
            file,
            sink,
        })
    }

    pub fn status(&self) -> SemanticStatus {
        SemanticStatus {
            state: *self.state.lock(),
            files: self.docs.try_read().map(|d| d.len()).unwrap_or(0),
            done: self.done.load(Ordering::Relaxed),
            total: self.total.load(Ordering::Relaxed),
            error: self.error.lock().clone(),
            models: self.models.lock().clone(),
        }
    }

    fn emit(&self) {
        if let Ok(v) = serde_json::to_value(self.status()) {
            (self.sink)("semantic-status", v);
        }
    }

    fn set(&self, state: &'static str, err: Option<String>) {
        *self.state.lock() = state;
        *self.error.lock() = err;
        self.emit();
    }

    pub fn configure(self: &Arc<Self>, cfg: SemanticConfig) {
        {
            let mut cur = self.config.lock();
            if *cur == cfg && *self.state.lock() != "off" {
                return;
            }
            *cur = cfg.clone();
        }
        let gen = self.generation.fetch_add(1, Ordering::Relaxed) + 1;
        if !cfg.enabled {
            self.set("off", None);
            return;
        }
        let me = self.clone();
        std::thread::Builder::new().name("semantic".into()).spawn(move || me.build(gen, cfg)).ok();
    }

    pub fn rebuild(self: &Arc<Self>) {
        let cfg = self.config.lock().clone();
        self.docs.write().clear();
        let gen = self.generation.fetch_add(1, Ordering::Relaxed) + 1;
        if cfg.enabled {
            let me = self.clone();
            std::thread::spawn(move || me.build(gen, cfg));
        }
    }

    fn current(&self, gen: u64) -> bool {
        self.generation.load(Ordering::Relaxed) == gen
    }

    /// Models installed in Ollama.
    fn list_models(cfg: &SemanticConfig) -> AppResult<Vec<String>> {
        #[derive(Deserialize)]
        struct Tags {
            models: Vec<Model>,
        }
        #[derive(Deserialize)]
        struct Model {
            name: String,
        }
        let mut res = agent(3).get(format!("{}/api/tags", cfg.endpoint.trim_end_matches('/'))).call().map_err(|e| AppError::other(e.to_string()))?;
        let t: Tags = res.body_mut().read_json().map_err(|e| AppError::other(e.to_string()))?;
        Ok(t.models.into_iter().map(|m| m.name).collect())
    }

    fn has_model(models: &[String], want: &str) -> bool {
        models.iter().any(|m| m == want || m.split(':').next() == Some(want) || (want.contains(':') && m == want))
    }

    fn build(self: Arc<Self>, gen: u64, cfg: SemanticConfig) {
        let models = match Self::list_models(&cfg) {
            Ok(m) => m,
            Err(e) => return self.set("unavailable", Some(e.message)),
        };
        *self.models.lock() = models.clone();
        if !Self::has_model(&models, &cfg.model) {
            return self.set("missingModel", None);
        }
        self.set("indexing", None);
        // Candidates: documents (and photos) in the chosen folders.
        let roots: Vec<PathBuf> = cfg.roots.iter().map(|r| crate::util::expand_user_path(r)).filter(|p| p.is_dir()).collect();
        let p = Progress { dirs: AtomicUsize::new(0), cancel: AtomicBool::new(false) };
        let d = scan::scan(&roots, &Excludes::parse(crate::index::DEFAULT_EXCLUDES), false, &p);
        let vision = cfg.images && Self::has_model(&models, &cfg.vision_model);
        let mut files: Vec<(String, u64, i64)> = Vec::new();
        for id in 0..d.nodes.len() as u32 {
            let n = d.nodes[id as usize];
            if n.flags & (flags::DIR | flags::LINK) != 0 || n.parent == NONE || n.size == 0 || n.size > 100 << 20 {
                continue;
            }
            let name = d.name(id);
            let ext = name.rsplit_once('.').map(|(_, e)| e.to_lowercase()).unwrap_or_default();
            let kind = crate::meta::kind_of(&ext, false);
            let wanted = matches!(kind, "document" | "pdf" | "sheet" | "slides") || (kind == "text" && n.size < 4 << 20) || (vision && kind == "image" && n.size > 30_000);
            if wanted {
                files.push((d.path(id), n.size, n.mtime));
            }
        }
        // Keep vectors of unchanged files, drop vanished ones.
        let known: HashMap<String, (u64, i64)> = self.docs.read().iter().map(|d| (d.path.clone(), (d.size, d.mtime))).collect();
        let wanted: std::collections::HashSet<&str> = files.iter().map(|f| f.0.as_str()).collect();
        self.docs.write().retain(|d| wanted.contains(d.path.as_str()));
        let todo: Vec<(String, u64, i64)> = files.into_iter().filter(|(p, s, m)| known.get(p) != Some(&(*s, *m))).collect();
        self.total.store(todo.len(), Ordering::Relaxed);
        self.done.store(0, Ordering::Relaxed);
        self.emit();
        let mut last_emit = Instant::now();
        let mut last_save = Instant::now();
        for chunk in todo.chunks(BATCH) {
            if !self.current(gen) {
                return;
            }
            let texts: Vec<(usize, String, String)> = chunk
                .iter()
                .enumerate()
                .filter_map(|(i, (path, _, _))| {
                    let (text, snippet) = describe(Path::new(path), &cfg, vision)?;
                    Some((i, text, snippet))
                })
                .collect();
            if !texts.is_empty() {
                let input: Vec<String> = texts.iter().map(|(_, t, _)| doc_prompt(&cfg.model, t)).collect();
                match embed(&cfg, &input) {
                    Ok(vecs) => {
                        let mut docs = self.docs.write();
                        for ((i, _, snippet), v) in texts.into_iter().zip(vecs) {
                            let (path, size, mtime) = &chunk[i];
                            docs.retain(|d| &d.path != path);
                            docs.push(Doc { path: path.clone(), size: *size, mtime: *mtime, snippet, vec: v });
                        }
                    }
                    Err(e) => return self.set("error", Some(e.message)),
                }
            }
            self.done.fetch_add(chunk.len(), Ordering::Relaxed);
            if last_emit.elapsed() > Duration::from_millis(700) {
                self.emit();
                last_emit = Instant::now();
            }
            if last_save.elapsed() > Duration::from_secs(60) {
                let _ = save(&self.file, &self.docs.read());
                last_save = Instant::now();
            }
        }
        let _ = save(&self.file, &self.docs.read());
        self.set("ready", None);
    }

    pub fn search(&self, query: &str, limit: usize) -> AppResult<Vec<SemanticHit>> {
        let cfg = self.config.lock().clone();
        if !cfg.enabled {
            return Err(AppError::unsupported("Semantic search is turned off"));
        }
        let q = embed(&cfg, &[query_prompt(&cfg.model, query)])?.pop().ok_or_else(|| AppError::other("No embedding"))?;
        let docs = self.docs.read();
        let words: Vec<String> = query.split_whitespace().map(crate::query::fold).filter(|w| w.len() > 2).collect();
        let mut scored: Vec<(f32, usize)> = docs
            .iter()
            .enumerate()
            .map(|(i, d)| {
                let mut s: f32 = d.vec.iter().zip(&q).map(|(a, b)| a * b).sum();
                // A literal word in the file name is a strong hint.
                let name = crate::query::fold(Path::new(&d.path).file_name().map(|n| n.to_string_lossy()).unwrap_or_default().as_ref());
                if words.iter().any(|w| name.contains(w.as_str())) {
                    s += 0.05;
                }
                (s, i)
            })
            .collect();
        scored.sort_by(|a, b| b.0.total_cmp(&a.0));
        // Scores are model specific: keep what is reasonably close to the best match.
        let top = scored.first().map(|s| s.0).unwrap_or(0.0);
        Ok(scored
            .into_iter()
            .filter(|(s, _)| *s >= top * 0.55)
            .take(limit)
            .map(|(s, i)| {
                let d = &docs[i];
                SemanticHit {
                    path: d.path.clone(),
                    name: Path::new(&d.path).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(),
                    size: d.size,
                    mtime: d.mtime,
                    score: s,
                    snippet: d.snippet.clone(),
                }
            })
            .collect())
    }

    /// Downloads a model into Ollama (user initiated).
    pub fn pull(&self, model: &str) -> AppResult<()> {
        let cfg = self.config.lock().clone();
        agent(3600)
            .post(format!("{}/api/pull", cfg.endpoint.trim_end_matches('/')))
            .send_json(serde_json::json!({ "model": model, "stream": false }))
            .map_err(|e| AppError::other(e.to_string()))?;
        Ok(())
    }
}

/// Model specific task prefixes improve retrieval quality.
fn doc_prompt(model: &str, text: &str) -> String {
    if model.starts_with("nomic") {
        format!("search_document: {text}")
    } else if model.starts_with("embeddinggemma") {
        format!("title: none | text: {text}")
    } else {
        text.to_string()
    }
}

fn query_prompt(model: &str, text: &str) -> String {
    if model.starts_with("nomic") {
        format!("search_query: {text}")
    } else if model.starts_with("embeddinggemma") {
        format!("task: search result | query: {text}")
    } else {
        text.to_string()
    }
}

/// Text to embed for a file and a short snippet to show.
fn describe(path: &Path, cfg: &SemanticConfig, vision: bool) -> Option<(String, String)> {
    let name = path.file_name()?.to_string_lossy().into_owned();
    let folder = path.parent().and_then(|p| p.file_name()).map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let ext = name.rsplit_once('.').map(|(_, e)| e.to_lowercase()).unwrap_or_default();
    let kind = crate::meta::kind_of(&ext, false);
    let body = if kind == "image" {
        if !vision {
            return None;
        }
        let m = crate::meta::get(path);
        let caption = caption(path, cfg).unwrap_or_default();
        let camera = m.as_ref().and_then(|m| m.camera.clone()).unwrap_or_default();
        format!("{caption} {camera}")
    } else {
        let m = crate::meta::get(path);
        let title = m.as_ref().and_then(|m| m.title.clone()).unwrap_or_default();
        let text = crate::meta::text_content(path).unwrap_or_default();
        let text: String = text.split_whitespace().collect::<Vec<_>>().join(" ").chars().take(TEXT_CHARS).collect();
        format!("{title}\n{text}")
    };
    let snippet: String = body.trim().chars().take(160).collect();
    Some((format!("{name}\n{folder}\n{body}"), snippet))
}

fn caption(path: &Path, cfg: &SemanticConfig) -> Option<String> {
    use base64::Engine;
    let t = crate::thumbs::thumbnail(path, 512)?;
    let img = base64::engine::general_purpose::STANDARD.encode(&t.bytes);
    #[derive(Deserialize)]
    struct R {
        response: String,
    }
    let mut res = agent(120)
        .post(format!("{}/api/generate", cfg.endpoint.trim_end_matches('/')))
        .send_json(serde_json::json!({
            "model": cfg.vision_model,
            "prompt": "Describe this photo in one short sentence: what, where, who. No preamble.",
            "images": [img],
            "stream": false,
        }))
        .ok()?;
    res.body_mut().read_json::<R>().ok().map(|r| r.response.trim().to_string())
}

fn embed(cfg: &SemanticConfig, input: &[String]) -> AppResult<Vec<Vec<f32>>> {
    #[derive(Deserialize)]
    struct R {
        embeddings: Vec<Vec<f32>>,
    }
    let mut res = agent(300)
        .post(format!("{}/api/embed", cfg.endpoint.trim_end_matches('/')))
        .send_json(serde_json::json!({ "model": cfg.model, "input": input, "truncate": true }))
        .map_err(|e| AppError::other(e.to_string()))?;
    let r: R = res.body_mut().read_json().map_err(|e| AppError::other(e.to_string()))?;
    Ok(r.embeddings.into_iter().map(normalize).collect())
}

fn normalize(mut v: Vec<f32>) -> Vec<f32> {
    let n = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    if n > 0.0 {
        v.iter_mut().for_each(|x| *x /= n);
    }
    v
}

const MAGIC: &[u8; 6] = b"DSEM01";

fn save(path: &Path, docs: &[Doc]) -> std::io::Result<()> {
    let tmp = path.with_extension("tmp");
    if let Some(d) = path.parent() {
        std::fs::create_dir_all(d)?;
    }
    {
        let mut w = BufWriter::new(std::fs::File::create(&tmp)?);
        w.write_all(MAGIC)?;
        w.write_all(&(docs.len() as u32).to_le_bytes())?;
        for d in docs {
            for s in [&d.path, &d.snippet] {
                w.write_all(&(s.len() as u32).to_le_bytes())?;
                w.write_all(s.as_bytes())?;
            }
            w.write_all(&d.size.to_le_bytes())?;
            w.write_all(&d.mtime.to_le_bytes())?;
            w.write_all(&(d.vec.len() as u32).to_le_bytes())?;
            for x in &d.vec {
                w.write_all(&x.to_le_bytes())?;
            }
        }
        w.flush()?;
    }
    std::fs::rename(tmp, path)
}

fn load(path: &Path) -> std::io::Result<Vec<Doc>> {
    let mut r = BufReader::new(std::fs::File::open(path)?);
    let bad = || std::io::Error::new(std::io::ErrorKind::InvalidData, "semantic index");
    let mut m = [0u8; 6];
    r.read_exact(&mut m)?;
    if &m != MAGIC {
        return Err(bad());
    }
    let mut b4 = [0u8; 4];
    let mut b8 = [0u8; 8];
    let mut read_str = |r: &mut BufReader<std::fs::File>| -> std::io::Result<String> {
        r.read_exact(&mut b4)?;
        let mut s = vec![0u8; u32::from_le_bytes(b4) as usize];
        r.read_exact(&mut s)?;
        String::from_utf8(s).map_err(|_| bad())
    };
    let mut n4 = [0u8; 4];
    r.read_exact(&mut n4)?;
    let n = u32::from_le_bytes(n4) as usize;
    let mut out = Vec::with_capacity(n.min(1_000_000));
    for _ in 0..n {
        let path = read_str(&mut r)?;
        let snippet = read_str(&mut r)?;
        r.read_exact(&mut b8)?;
        let size = u64::from_le_bytes(b8);
        r.read_exact(&mut b8)?;
        let mtime = i64::from_le_bytes(b8);
        r.read_exact(&mut n4)?;
        let dim = u32::from_le_bytes(n4) as usize;
        if dim > 8192 {
            return Err(bad());
        }
        let mut raw = vec![0u8; dim * 4];
        r.read_exact(&mut raw)?;
        let vec = raw.chunks_exact(4).map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect();
        out.push(Doc { path, size, mtime, snippet, vec });
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn persist_and_rank() {
        let f = std::env::temp_dir().join(format!("dualion-sem-{}", uuid::Uuid::new_v4()));
        let docs = vec![
            Doc { path: "/a/invoice.pdf".into(), size: 1, mtime: 2, snippet: "hosting".into(), vec: normalize(vec![1.0, 0.0, 0.2]) },
            Doc { path: "/a/cat.jpg".into(), size: 3, mtime: 4, snippet: "cat".into(), vec: normalize(vec![0.0, 1.0, 0.0]) },
        ];
        save(&f, &docs).unwrap();
        let l = load(&f).unwrap();
        std::fs::remove_file(&f).unwrap();
        assert_eq!(l.len(), 2);
        assert_eq!(l[1].snippet, "cat");
        assert!((l[0].vec.iter().map(|x| x * x).sum::<f32>() - 1.0).abs() < 1e-5);
    }

    /// Needs a running Ollama with `all-minilm`: DUALION_OLLAMA=1 cargo test semantic -- --ignored
    #[test]
    #[ignore]
    fn ollama_embeddings_rank_related_text_higher() {
        let cfg = SemanticConfig { enabled: true, model: "all-minilm".into(), ..Default::default() };
        let v = embed(&cfg, &["invoice for web hosting services".into(), "a cute cat sleeping on a sofa".into(), "bill from the hosting provider".into()]).unwrap();
        let dot = |a: &[f32], b: &[f32]| a.iter().zip(b).map(|(x, y)| x * y).sum::<f32>();
        assert!(dot(&v[0], &v[2]) > dot(&v[0], &v[1]));
    }
}
