//! Tauri commands – thin wrappers that move blocking work off the IPC thread.

use crate::error::{AppError, AppResult, ErrorCode};
use crate::remote::{self, Connected, Registry as RemoteRegistry};
use crate::jobs::{Answer, JobManager, JobRequest, JobSnapshot};
use crate::local::{self, DirSize, Resolved, Space, Volume};
use crate::location::{Listing, Location};
use crate::menu::MenuNode;
use crate::platform::{self, AppInfo};
use crate::rename::{RenameOutcome, RenamePair};
use crate::search::{SearchEvent, SearchParams};
use crate::vault::{SiteInput, SiteSummary, Vault, VaultStatus};
use crate::viewer::{Probe, TextChunk};
use crate::watcher::DirWatcher;
use crate::{archive_index, checksum, rename, search, util, viewer};
use parking_lot::Mutex;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use tauri::ipc::{Channel, Response};
use tauri::{AppHandle, State};

pub struct AppState {
    pub remote: Arc<RemoteRegistry>,
    pub jobs: Arc<JobManager>,
    pub vault: Arc<Vault>,
    pub watcher: Arc<DirWatcher>,
    pub index: Arc<crate::index::IndexManager>,
    pub rules: Arc<crate::rules::RulesEngine>,
    pub semantic: Arc<crate::semantic::SemanticEngine>,
    pub ops: Mutex<HashMap<u64, Arc<AtomicBool>>>,
    pub next_op: AtomicU64,
    pub config_dir: PathBuf,
    pub effects: &'static str,
}

impl AppState {
    fn new_op(&self) -> (u64, Arc<AtomicBool>) {
        let id = self.next_op.fetch_add(1, Ordering::Relaxed);
        let flag = Arc::new(AtomicBool::new(false));
        self.ops.lock().insert(id, flag.clone());
        (id, flag)
    }
}

async fn blocking<T: Send + 'static>(f: impl FnOnce() -> AppResult<T> + Send + 'static) -> AppResult<T> {
    tauri::async_runtime::spawn_blocking(f).await.map_err(|e| AppError::other(e.to_string()))?
}

// ---------------------------------------------------------------------------
// App
// ---------------------------------------------------------------------------

#[tauri::command]
pub fn app_info(app: AppHandle, state: State<'_, AppState>) -> AppInfo {
    AppInfo {
        platform: platform::platform_name(),
        version: app.package_info().version.to_string(),
        home: dirs::home_dir().map(|p| util::path_str(&p)).unwrap_or_default(),
        places: platform::places(),
        accent: platform::accent_color(),
        locale: sys_locale::get_locale().unwrap_or_else(|| "en-US".into()),
        effects: state.effects,
        os_build: platform::os_build(),
        separator: std::path::MAIN_SEPARATOR.to_string(),
    }
}

#[tauri::command]
pub fn set_menu(app: AppHandle, spec: Vec<MenuNode>) -> AppResult<()> {
    let menu = crate::menu::build(&app, &spec)?;
    app.set_menu(menu)?;
    Ok(())
}

fn store_path(state: &AppState, name: &str) -> AppResult<PathBuf> {
    if name.is_empty() || !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') {
        return Err(AppError::invalid("Invalid store name"));
    }
    Ok(state.config_dir.join(format!("{name}.json")))
}

#[tauri::command]
pub fn store_load(state: State<'_, AppState>, name: String) -> AppResult<Option<serde_json::Value>> {
    let p = store_path(&state, &name)?;
    match std::fs::read(&p) {
        Ok(b) => Ok(serde_json::from_slice(&b).ok()),
        Err(_) => Ok(None),
    }
}

#[tauri::command]
pub fn store_save(state: State<'_, AppState>, name: String, value: serde_json::Value) -> AppResult<()> {
    let p = store_path(&state, &name)?;
    std::fs::create_dir_all(&state.config_dir).map_err(|e| AppError::io(e, &state.config_dir))?;
    let tmp = p.with_extension("json.tmp");
    std::fs::write(&tmp, serde_json::to_vec_pretty(&value).unwrap_or_default()).map_err(|e| AppError::io(e, &tmp))?;
    std::fs::rename(&tmp, &p).map_err(|e| AppError::io(e, &p))
}

#[tauri::command]
pub fn watch_dirs(state: State<'_, AppState>, dirs: Vec<String>) {
    state.watcher.set(dirs);
}

#[tauri::command]
pub fn cancel_op(state: State<'_, AppState>, id: u64) {
    if let Some(f) = state.ops.lock().remove(&id) {
        f.store(true, Ordering::Relaxed);
    }
}

#[tauri::command]
pub fn app_exit(app: AppHandle, state: State<'_, AppState>) {
    state.jobs.cancel_all();
    app.exit(0);
}

#[tauri::command]
pub fn active_jobs(state: State<'_, AppState>) -> usize {
    state.jobs.active_count()
}

// ---------------------------------------------------------------------------
// Shell integration
// ---------------------------------------------------------------------------

#[tauri::command]
pub fn open_path(path: String) -> AppResult<()> {
    #[cfg(windows)]
    {
        crate::winshell::open_default(&path)
    }
    #[cfg(not(windows))]
    {
        tauri_plugin_opener::open_path(&path, None::<&str>).map_err(|e| AppError::other(e.to_string()))
    }
}

/// Opens a web link in the default browser (only http/https).
#[tauri::command]
pub fn open_url(url: String) -> AppResult<()> {
    let lower = url.to_ascii_lowercase();
    if !(lower.starts_with("https://") || lower.starts_with("http://")) {
        return Err(AppError::invalid("Only web links can be opened"));
    }
    tauri_plugin_opener::open_url(&url, None::<&str>).map_err(|e| AppError::other(e.to_string()))
}

/// QR code of `text` as an SVG document (donation addresses in About).
#[tauri::command]
pub fn qr_svg(text: String) -> AppResult<String> {
    use qrcode::render::svg;
    let code = qrcode::QrCode::new(text.as_bytes()).map_err(|e| AppError::invalid(e.to_string()))?;
    Ok(code
        .render::<svg::Color>()
        .min_dimensions(180, 180)
        .quiet_zone(true)
        .dark_color(svg::Color("#000000"))
        .light_color(svg::Color("#ffffff"))
        .build())
}

/// Native drag of local files out of the window (to Finder / Explorer / other apps).
/// Runs on the main thread; failures are reported, never panic the event loop.
#[tauri::command]
pub fn drag_out(window: tauri::WebviewWindow, paths: Vec<String>) -> AppResult<()> {
    let paths: Vec<String> = paths.into_iter().filter(|p| std::path::Path::new(p).is_absolute()).collect();
    if paths.is_empty() {
        return Ok(());
    }
    #[cfg(windows)]
    {
        let hwnd = crate::winshell::hwnd_of(&window)?.0 as usize;
        window
            .run_on_main_thread(move || {
                let hwnd = windows::Win32::Foundation::HWND(hwnd as *mut std::ffi::c_void);
                match std::panic::catch_unwind(|| crate::winshell::drag_out(hwnd, &paths)) {
                    Ok(Err(e)) => eprintln!("drag out: {e}"),
                    Err(_) => eprintln!("drag out: panicked"),
                    Ok(Ok(())) => {}
                }
            })
            .map_err(|e| AppError::other(e.to_string()))
    }
    #[cfg(target_os = "macos")]
    {
        let first = std::path::PathBuf::from(&paths[0]);
        let icon = crate::icons::icon_png(crate::icons::IconKey::Path(first.to_string_lossy().into_owned()), 48)
            .map(|p| p.to_vec())
            .unwrap_or_else(|| include_bytes!("../icons/32x32.png").to_vec());
        let files: Vec<std::path::PathBuf> = paths.into_iter().map(std::path::PathBuf::from).collect();
        let w = window.clone();
        window
            .run_on_main_thread(move || {
                let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    drag::start_drag(&w, drag::DragItem::Files(files), drag::Image::Raw(icon), |_, _| {}, drag::Options::default())
                }));
                match r {
                    Ok(Err(e)) => eprintln!("drag out: {e}"),
                    Err(_) => eprintln!("drag out: panicked"),
                    Ok(Ok(())) => {}
                }
            })
            .map_err(|e| AppError::other(e.to_string()))
    }
    #[cfg(not(any(windows, target_os = "macos")))]
    {
        let _ = window;
        Err(AppError::unsupported("Drag out is not supported here"))
    }
}

/// Connects a network share (system sign-in dialog when needed) and returns
/// the folder to open. Without `share`: Windows signs in to the computer
/// (to list its shares), macOS lets the user pick shares to mount.
#[tauri::command]
pub async fn network_connect(window: tauri::WebviewWindow, host: String, share: Option<String>) -> AppResult<String> {
    #[cfg(windows)]
    {
        let hwnd = crate::winshell::hwnd_of(&window)?.0 as usize;
        blocking(move || crate::network::connect(windows::Win32::Foundation::HWND(hwnd as *mut std::ffi::c_void), &host, share.as_deref())).await
    }
    #[cfg(target_os = "macos")]
    {
        let _ = window;
        blocking(move || crate::network::mount(&host, share.as_deref())).await
    }
    #[cfg(not(any(windows, target_os = "macos")))]
    {
        let _ = (window, host, share);
        Err(AppError::unsupported("Not supported here"))
    }
}

#[tauri::command]
pub async fn open_with_apps(path: String) -> Vec<platform::AppChoice> {
    tauri::async_runtime::spawn_blocking(move || platform::open_with_apps(&path)).await.unwrap_or_default()
}

/// Explorer's context menu for files (Windows). Returns one of our command
/// ids to run afterwards, or null. Errors make the frontend use its own menu.
#[tauri::command]
pub async fn shell_menu(
    window: tauri::WebviewWindow,
    paths: Vec<String>,
    all_dirs: bool,
    title: String,
    items: Vec<serde_json::Value>,
) -> AppResult<Option<String>> {
    #[cfg(windows)]
    {
        let items: Vec<crate::winshell::AppItem> = items.into_iter().filter_map(|v| serde_json::from_value(v).ok()).collect();
        let hwnd = crate::winshell::hwnd_of(&window)?.0 as usize;
        let (tx, rx) = std::sync::mpsc::channel();
        window
            .run_on_main_thread(move || {
                let hwnd = windows::Win32::Foundation::HWND(hwnd as *mut std::ffi::c_void);
                let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    crate::winshell::context_menu(hwnd, &paths, all_dirs, &title, &items)
                }))
                .unwrap_or_else(|_| Err(AppError::other("Shell menu panicked")));
                let _ = tx.send(r);
            })
            .map_err(|e| AppError::other(e.to_string()))?;
        tauri::async_runtime::spawn_blocking(move || rx.recv().unwrap_or_else(|_| Err(AppError::other("Shell menu closed"))))
            .await
            .map_err(|e| AppError::other(e.to_string()))?
    }
    #[cfg(not(windows))]
    {
        let _ = (window, paths, all_dirs, title, items);
        Err(AppError::unsupported("Shell menu is Windows only"))
    }
}

/// Text on the system clipboard (go to the path in the clipboard).
#[tauri::command]
pub fn clipboard_text() -> AppResult<String> {
    arboard::Clipboard::new().and_then(|mut c| c.get_text()).map_err(|e| AppError::other(e.to_string()))
}

#[tauri::command]
pub fn reveal_path(path: String) -> AppResult<()> {
    tauri_plugin_opener::reveal_item_in_dir(&path).map_err(|e| AppError::other(e.to_string()))
}

#[tauri::command]
pub fn open_terminal(dir: String, custom: Option<String>) -> AppResult<()> {
    platform::open_terminal(&dir, custom.as_deref())
}

#[tauri::command]
pub fn edit_file(path: String, editor: Option<String>) -> AppResult<()> {
    platform::edit_file(&path, editor.as_deref())
}

#[tauri::command]
pub fn open_with(path: String, app: String) -> AppResult<()> {
    platform::open_with(&path, &app)
}

#[tauri::command]
pub fn quick_look(path: String) -> AppResult<()> {
    platform::quick_look(&path)
}

#[tauri::command]
pub fn show_properties(path: String) -> AppResult<()> {
    platform::show_properties(&path)
}

// ---------------------------------------------------------------------------
// Listing & paths
// ---------------------------------------------------------------------------

#[tauri::command]
pub async fn list_dir(state: State<'_, AppState>, location: Location) -> AppResult<Listing> {
    match location {
        Location::Local { path } => blocking(move || local::list_local(&path)).await,
        Location::Drives => blocking(|| Ok(local::list_drives())).await,
        Location::Network { host } => blocking(move || crate::network::list(host.as_deref())).await,
        Location::Remote { session, path } => {
            let s = state.remote.get(&session)?;
            blocking(move || s.list(&path)).await
        }
        Location::Archive { archive, inner } => blocking(move || archive_index::list_archive(&archive, &inner)).await,
    }
}

#[tauri::command]
pub async fn volumes() -> AppResult<Vec<Volume>> {
    blocking(|| Ok(local::volumes())).await
}

#[tauri::command]
pub async fn space(path: String) -> AppResult<Option<Space>> {
    blocking(move || Ok(local::space(&local::normalize(&path)))).await
}

#[tauri::command]
pub async fn dir_size(state: State<'_, AppState>, path: String) -> AppResult<DirSize> {
    let (id, flag) = state.new_op();
    let r = blocking(move || Ok(local::dir_size(&local::normalize(&path), &flag))).await;
    state.ops.lock().remove(&id);
    r
}

#[tauri::command]
pub async fn resolve_path(input: String, base: Option<String>) -> AppResult<Resolved> {
    blocking(move || local::resolve_input(&input, base.as_deref())).await
}

#[tauri::command]
pub async fn complete_path(input: String, base: Option<String>, show_hidden: bool) -> AppResult<Vec<String>> {
    blocking(move || Ok(local::complete_dirs(&input, base.as_deref(), show_hidden))).await
}

#[tauri::command]
pub async fn make_dir(state: State<'_, AppState>, dir: Location, name: String) -> AppResult<String> {
    match dir {
        Location::Local { path } => blocking(move || local::make_dir(&path, &name)).await,
        Location::Remote { session, path } => {
            let s = state.remote.get(&session)?;
            blocking(move || {
                let fs = s.fs();
                let parts: Vec<&str> = name.split('/').filter(|p| !p.is_empty()).collect();
                let first = parts.first().map(|s| s.to_string()).ok_or_else(|| AppError::new(ErrorCode::InvalidName, "Empty name"))?;
                let mut cur = path.clone();
                for (i, part) in parts.iter().enumerate() {
                    local::check_name(part)?;
                    cur = remote::join(&cur, part);
                    // Intermediate folders may already exist; only the last one must be new.
                    if let Err(e) = fs.mkdir(&cur) {
                        if i + 1 == parts.len() {
                            return Err(e);
                        }
                    }
                }
                Ok(first)
            })
            .await
        }
        _ => Err(AppError::unsupported("Cannot create folders here")),
    }
}

#[tauri::command]
pub async fn create_file(state: State<'_, AppState>, dir: Location, name: String) -> AppResult<String> {
    match dir {
        Location::Local { path } => blocking(move || local::create_file(&path, &name)).await,
        Location::Remote { session, path } => {
            let s = state.remote.get(&session)?;
            blocking(move || {
                local::check_name(&name)?;
                let target = remote::join(&path, &name);
                let fs = s.fs();
                if fs.stat(&target)?.is_some() {
                    return Err(AppError::new(ErrorCode::AlreadyExists, "Already exists").with_path(&target));
                }
                fs.writer(&target)?.finish()?;
                Ok(target)
            })
            .await
        }
        _ => Err(AppError::unsupported("Cannot create files here")),
    }
}

#[tauri::command]
pub async fn rename_item(state: State<'_, AppState>, dir: Location, from: String, to: String) -> AppResult<()> {
    match dir {
        Location::Local { path } => blocking(move || local::rename(&path, &from, &to)).await,
        Location::Remote { session, path } => {
            let s = state.remote.get(&session)?;
            blocking(move || {
                local::check_name(&to)?;
                let (a, b) = (remote::join(&path, &from), remote::join(&path, &to));
                let fs = s.fs();
                // Many servers (and S3, rclone, docker mv) replace an existing target silently.
                if from.to_lowercase() != to.to_lowercase() && fs.stat(&b)?.is_some() {
                    return Err(AppError::new(ErrorCode::AlreadyExists, "Target already exists").with_path(&b));
                }
                fs.rename(&a, &b).map_err(|e| if e.code == ErrorCode::CrossesDevices { AppError::unsupported("This server cannot rename folders – move them instead") } else { e })
            })
            .await
        }
        _ => Err(AppError::unsupported("Cannot rename here")),
    }
}

#[tauri::command]
pub async fn batch_rename(dir: String, pairs: Vec<RenamePair>) -> AppResult<Vec<RenameOutcome>> {
    blocking(move || rename::batch_rename(&dir, pairs)).await
}

#[tauri::command]
pub async fn create_link(target: String, dir: String, name: String, hard: bool) -> AppResult<()> {
    blocking(move || local::create_link(&target, &dir, &name, hard)).await
}

#[tauri::command]
pub async fn write_text_file(path: String, content: String) -> AppResult<()> {
    blocking(move || std::fs::write(&path, content).map_err(|e| AppError::io(e, &path))).await
}

/// Returns a local path for any location: FTP files are downloaded and
/// archive members extracted into a temporary folder (viewer, editor, open).
#[tauri::command]
pub async fn materialize(state: State<'_, AppState>, location: Location) -> AppResult<String> {
    match location {
        Location::Local { path } => Ok(path),
        Location::Archive { archive, inner } => blocking(move || {
            let name = inner.trim_end_matches('/').rsplit('/').next().unwrap_or("file").to_string();
            let dir = platform::temp_dir().join(format!("arc-{:x}", xxhash_rust::xxh3::xxh3_64(format!("{archive}|{inner}").as_bytes())));
            std::fs::create_dir_all(&dir).map_err(|e| AppError::io(e, &dir))?;
            let target = dir.join(&name);
            let data = archive_index::read_member(&archive, &inner, usize::MAX)?;
            std::fs::write(&target, data).map_err(|e| AppError::io(e, &target))?;
            Ok(util::path_str(&target))
        })
        .await,
        Location::Remote { session, path } => {
            let s = state.remote.get(&session)?;
            blocking(move || {
                let name = remote::file_name(&path).to_string();
                let dir = platform::temp_dir().join(format!("remote-{:x}", xxhash_rust::xxh3::xxh3_64(format!("{session}|{path}").as_bytes())));
                std::fs::create_dir_all(&dir).map_err(|e| AppError::io(e, &dir))?;
                let target = dir.join(&name);
                let mut out = std::fs::File::create(&target).map_err(|e| AppError::io(e, &target))?;
                let fs = s.fs();
                let mut stream = fs.reader(&path)?;
                std::io::copy(&mut stream, &mut out).map_err(|e| AppError::io(e, &target))?;
                stream.finish()?;
                Ok(util::path_str(&target))
            })
            .await
        }
        Location::Drives | Location::Network { .. } => Err(AppError::invalid("Not a file")),
    }
}

// ---------------------------------------------------------------------------
// Jobs
// ---------------------------------------------------------------------------

#[tauri::command]
pub fn job_enqueue(state: State<'_, AppState>, request: JobRequest) -> AppResult<u64> {
    state.jobs.enqueue(request)
}

#[tauri::command]
pub fn jobs_list(state: State<'_, AppState>) -> Vec<JobSnapshot> {
    state.jobs.snapshot()
}

#[tauri::command]
pub fn job_pause(state: State<'_, AppState>, id: u64, paused: bool) -> AppResult<()> {
    state.jobs.set_paused(id, paused)
}

#[tauri::command]
pub fn job_cancel(state: State<'_, AppState>, id: u64) -> AppResult<()> {
    state.jobs.cancel(id)
}

#[tauri::command]
pub fn job_answer(state: State<'_, AppState>, id: u64, answer: Answer) -> AppResult<()> {
    state.jobs.answer(id, answer)
}

#[tauri::command]
pub fn job_start_now(state: State<'_, AppState>, id: u64) -> AppResult<()> {
    state.jobs.start_now(id)
}

#[tauri::command]
pub fn job_move(state: State<'_, AppState>, id: u64, delta: i32) -> AppResult<()> {
    state.jobs.move_job(id, delta)
}

#[tauri::command]
pub fn jobs_clear_finished(state: State<'_, AppState>) {
    state.jobs.clear_finished()
}

// ---------------------------------------------------------------------------
// Search, viewer, checksums
// ---------------------------------------------------------------------------

#[tauri::command]
pub fn search_start(state: State<'_, AppState>, params: SearchParams, channel: Channel<SearchEvent>) -> AppResult<u64> {
    let (id, flag) = state.new_op();
    search::run(params, channel, flag)?;
    Ok(id)
}

/// Scans a folder for the disk usage map; progress = folders read so far.
#[tauri::command]
pub async fn du_scan(state: State<'_, AppState>, path: String, channel: Channel<u64>) -> AppResult<crate::diskusage::DuNode> {
    let (id, flag) = state.new_op();
    let _ = channel.send(id);
    let r = blocking(move || {
        let p = crate::local::normalize(&path);
        crate::diskusage::scan(&p, &|dirs| {
            let _ = channel.send(dirs as u64);
        }, &flag)?;
        crate::diskusage::tree(&p, 3)
    })
    .await;
    state.ops.lock().remove(&id);
    r
}

/// Subtree of the last scan (drilling down); errors when not scanned.
#[tauri::command]
pub async fn du_tree(path: String) -> AppResult<crate::diskusage::DuNode> {
    blocking(move || {
        let p = crate::local::normalize(&path);
        if !crate::diskusage::cached(&p) {
            return Err(AppError::invalid("Not scanned"));
        }
        crate::diskusage::tree(&p, 3)
    })
    .await
}

#[tauri::command]
pub async fn semantic_configure(state: State<'_, AppState>, config: crate::semantic::SemanticConfig) -> AppResult<()> {
    let s = state.semantic.clone();
    blocking(move || {
        s.configure(config);
        Ok(())
    })
    .await
}

#[tauri::command]
pub fn semantic_status(state: State<'_, AppState>) -> crate::semantic::SemanticStatus {
    state.semantic.status()
}

#[tauri::command]
pub async fn semantic_search(state: State<'_, AppState>, query: String, limit: usize) -> AppResult<Vec<crate::semantic::SemanticHit>> {
    let s = state.semantic.clone();
    blocking(move || s.search(&query, limit.clamp(1, 500))).await
}

#[tauri::command]
pub async fn semantic_rebuild(state: State<'_, AppState>) -> AppResult<()> {
    let s = state.semantic.clone();
    blocking(move || {
        s.rebuild();
        Ok(())
    })
    .await
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AskResult {
    pub interpretation: crate::ask::Interpretation,
    pub hits: Vec<crate::semantic::SemanticHit>,
}

/// Natural-language search: request -> filters (+ topic) -> index and semantic results.
#[tauri::command]
pub async fn semantic_ask(state: State<'_, AppState>, text: String, limit: usize) -> AppResult<AskResult> {
    let sem = state.semantic.clone();
    let index = state.index.clone();
    blocking(move || {
        let cfg = sem.config();
        let limit = limit.clamp(1, 500);
        let llm = (cfg.enabled && sem.has_assistant(&cfg)).then(|| crate::ask::llm(&cfg.endpoint, &cfg.assistant_model, &text)).flatten();
        let rules = crate::ask::rules(&text);
        // The model's reading first; when it finds nothing, the rules'; finally meaning only.
        let mut tried = Vec::new();
        for interp in llm.into_iter().chain(std::iter::once(rules)) {
            if tried.contains(&interp) {
                continue;
            }
            let hits = ask_run(&sem, &index, &cfg, &interp, limit);
            if !hits.is_empty() {
                return Ok(AskResult { interpretation: interp, hits });
            }
            tried.push(interp);
        }
        let first = tried.into_iter().next().unwrap_or_default();
        if cfg.enabled {
            if let Ok(hits) = sem.search(text.trim_start_matches('?'), limit) {
                if !hits.is_empty() {
                    let interp = crate::ask::Interpretation { query: String::new(), topic: text.trim().to_string(), via: first.via };
                    return Ok(AskResult { interpretation: interp, hits });
                }
            }
        }
        Ok(AskResult { interpretation: first, hits: Vec::new() })
    })
    .await
}

fn ask_run(
    sem: &crate::semantic::SemanticEngine,
    index: &crate::index::IndexManager,
    cfg: &crate::semantic::SemanticConfig,
    interp: &crate::ask::Interpretation,
    limit: usize,
) -> Vec<crate::semantic::SemanticHit> {
    let filter = (!interp.query.is_empty()).then(|| crate::query::Query::parse(&interp.query).ok()).flatten();
    let mut hits = Vec::new();
    // Distinctive words of the topic (domains, codes, numbers) often appear in file names.
    let mut named: Vec<crate::semantic::SemanticHit> = Vec::new();
    for w in interp.topic.split_whitespace() {
        let w = w.trim_matches(|c: char| !c.is_alphanumeric() && c != '.');
        let distinctive = (w.contains('.') && w.len() > 4) || (w.len() >= 4 && w.chars().any(|c| c.is_ascii_digit()));
        if !distinctive {
            continue;
        }
        let base = w.split('.').next().unwrap_or(w);
        let q = if interp.query.is_empty() { base.to_string() } else { format!("{} {base}", interp.query) };
        if let Ok(r) = index.search(&q, 20, None, &AtomicBool::new(false)) {
            for h in r.hits.into_iter().filter(|h| !h.is_dir) {
                if !named.iter().any(|n| n.path == h.path) {
                    named.push(crate::semantic::SemanticHit { path: h.path, name: h.name, size: h.size, mtime: h.mtime, score: 1.0, snippet: String::new() });
                }
            }
        }
    }
    if !interp.topic.is_empty() && cfg.enabled {
        if let Ok(found) = sem.search(&interp.topic, limit * 4) {
            hits = found
                .into_iter()
                .filter(|h| {
                    filter.as_ref().is_none_or(|q| q.matches(&crate::query::FileSubject::new(h.path.clone(), false, h.size, h.mtime)))
                })
                .take(limit)
                .collect();
        }
    }
    if !named.is_empty() {
        hits.retain(|h: &crate::semantic::SemanticHit| !named.iter().any(|n| n.path == h.path));
        named.extend(hits);
        hits = named;
        hits.truncate(limit);
    }
    if hits.is_empty() {
        // Filters alone (or topic words as names) over the file index.
        let q = match (interp.query.is_empty(), interp.topic.is_empty()) {
            (false, true) => interp.query.clone(),
            (false, false) => format!("{} {}", interp.query, interp.topic),
            (true, false) => interp.topic.clone(),
            (true, true) => String::new(),
        };
        if !q.is_empty() {
            let never = AtomicBool::new(false);
            let mut r = index.search(&q, limit, None, &never);
            if r.as_ref().is_ok_and(|r| r.total == 0) && !interp.topic.is_empty() && !interp.query.is_empty() {
                r = index.search(&interp.query, limit, None, &never);
            }
            if let Ok(r) = r {
                hits = r
                    .hits
                    .into_iter()
                    .map(|h| crate::semantic::SemanticHit { path: h.path, name: h.name, size: h.size, mtime: h.mtime, score: 0.0, snippet: String::new() })
                    .collect();
            }
        }
    }
    hits
}

/// Downloads a model into Ollama, then re-checks the setup.
#[tauri::command]
pub async fn semantic_pull(state: State<'_, AppState>, model: String) -> AppResult<()> {
    let s = state.semantic.clone();
    blocking(move || {
        s.pull(&model)?;
        s.rebuild();
        Ok(())
    })
    .await
}

#[tauri::command]
pub fn rules_configure(state: State<'_, AppState>, config: crate::rules::RulesConfig) {
    state.rules.configure(config);
}

#[tauri::command]
pub fn rules_check(rule: crate::rules::Rule) -> AppResult<()> {
    crate::rules::check(&rule)
}

/// Runs a rule on the files already in its folder, or previews it (`dry`).
#[tauri::command]
pub async fn rules_run(state: State<'_, AppState>, rule: crate::rules::Rule, dry: bool) -> AppResult<Vec<crate::rules::RuleActivity>> {
    let rules = state.rules.clone();
    blocking(move || {
        crate::rules::check(&rule)?;
        rules.run_now(&rule, dry)
    })
    .await
}

#[tauri::command]
pub fn rules_log(state: State<'_, AppState>) -> Vec<crate::rules::RuleActivity> {
    state.rules.log()
}

/// Duplicate search; returns an op id for `cancel_op`.
#[tauri::command]
pub fn dupes_start(state: State<'_, AppState>, params: crate::dupes::DupeParams, channel: Channel<crate::dupes::DupeEvent>) -> AppResult<u64> {
    let (id, flag) = state.new_op();
    let index = state.index.clone();
    std::thread::Builder::new()
        .name("dupes".into())
        .spawn(move || {
            if let Err(e) = crate::dupes::run(params, &index, &channel, &flag) {
                if !e.is_cancelled() {
                    let _ = channel.send(crate::dupes::DupeEvent::Progress { phase: "error", done: 0, total: 0 });
                }
            }
        })
        .map_err(|e| AppError::other(e.to_string()))?;
    Ok(id)
}

/// Checks an archive password and remembers it for this session.
#[tauri::command]
pub async fn archive_password(archive: String, password: String) -> AppResult<()> {
    blocking(move || crate::arch::set_password(&archive, &password)).await
}

/// Ok when the archive can be read (no password needed or the stored one works).
#[tauri::command]
pub async fn archive_check(archive: String) -> AppResult<()> {
    blocking(move || crate::arch::check(&archive)).await
}

/// Archive kind of a file name ("zip", "7z", "rar", "iso", "tar", "single") or null.
#[tauri::command]
pub fn archive_kind(path: String) -> Option<&'static str> {
    use crate::arch::Kind;
    crate::arch::kind_of(&path).map(|k| match k {
        Kind::Zip => "zip",
        Kind::SevenZ => "7z",
        Kind::Rar => "rar",
        Kind::Iso => "iso",
        Kind::Tar(_) => "tar",
        Kind::Single(_) => "single",
    })
}

#[tauri::command]
pub async fn mount_image(path: String) -> AppResult<String> {
    blocking(move || platform::mount_image(&path)).await
}

#[tauri::command]
pub async fn viewer_probe(path: String) -> AppResult<Probe> {
    blocking(move || viewer::probe(std::path::Path::new(&path))).await
}

#[tauri::command]
pub async fn viewer_read_text(path: String, offset: u64, max: usize, encoding: String) -> AppResult<TextChunk> {
    blocking(move || viewer::read_text(std::path::Path::new(&path), offset, max, &encoding)).await
}

#[tauri::command]
pub async fn preview_sheet(path: String, sheet: usize, max_rows: usize) -> AppResult<crate::preview::sheet::SheetPreview> {
    blocking(move || crate::preview::sheet::workbook(std::path::Path::new(&path), sheet, max_rows.clamp(1, 20_000))).await
}

#[tauri::command]
pub async fn preview_document(path: String) -> AppResult<crate::preview::DocumentPreview> {
    blocking(move || crate::preview::document(std::path::Path::new(&path))).await
}

#[tauri::command]
pub async fn preview_slides(path: String) -> AppResult<Vec<crate::preview::Slide>> {
    blocking(move || crate::preview::slides(std::path::Path::new(&path))).await
}

/// Dry run of a job: counts, conflicts, space. A newer plan cancels an older one.
#[tauri::command]
pub async fn job_plan(state: State<'_, AppState>, req: crate::jobs::JobRequest) -> AppResult<crate::jobs::plan::Plan> {
    static CANCEL: parking_lot::Mutex<Option<Arc<AtomicBool>>> = parking_lot::Mutex::new(None);
    let flag = Arc::new(AtomicBool::new(false));
    if let Some(old) = CANCEL.lock().replace(flag.clone()) {
        old.store(true, Ordering::Relaxed);
    }
    let reg = state.remote.clone();
    blocking(move || crate::jobs::plan::plan(&req, &reg, &flag)).await
}

/// Reverses journaled actions; moves across volumes are queued as jobs.
#[tauri::command]
pub async fn undo_apply(state: State<'_, AppState>, actions: Vec<crate::jobs::undo::UndoAction>) -> AppResult<crate::jobs::undo::UndoReport> {
    let report = blocking(move || Ok(crate::jobs::undo::apply(&actions))).await?;
    for (from, to) in &report.moves {
        let p = std::path::Path::new(to);
        let req = crate::jobs::JobRequest {
            op: crate::jobs::JobOp::Move,
            sources: vec![crate::location::Location::local(from.clone())],
            dest: p.parent().map(|d| crate::location::Location::local(d.to_string_lossy())),
            new_name: p.file_name().map(|n| n.to_string_lossy().into_owned()),
            options: Default::default(),
            queue: None,
        };
        state.jobs.enqueue(req)?;
    }
    Ok(report)
}

/// Moves one path to the trash, returning the undo record.
#[tauri::command]
pub async fn trash_item(path: String) -> AppResult<crate::jobs::undo::UndoAction> {
    blocking(move || {
        let to = crate::jobs::undo::trash(&path)?;
        Ok(crate::jobs::undo::UndoAction::Trashed { path, trashed_to: to, time: crate::util::now_ms() })
    })
    .await
}

/// Off the UI thread: reconfiguring may wait for the index lock.
#[tauri::command]
pub async fn index_configure(state: State<'_, AppState>, config: crate::index::IndexConfig) -> AppResult<()> {
    let index = state.index.clone();
    blocking(move || {
        index.configure(config);
        Ok(())
    })
    .await
}

#[tauri::command]
pub fn index_status(state: State<'_, AppState>) -> crate::index::IndexStatus {
    state.index.status()
}

#[tauri::command]
pub fn index_defaults() -> serde_json::Value {
    let cloud: Vec<String> = crate::util::cloud_roots().iter().map(|p| p.to_string_lossy().into_owned()).collect();
    serde_json::json!({ "excludes": crate::index::DEFAULT_EXCLUDES, "cloud": cloud })
}

#[tauri::command]
pub async fn index_rescan(state: State<'_, AppState>) -> AppResult<()> {
    let index = state.index.clone();
    blocking(move || {
        index.rescan();
        Ok(())
    })
    .await
}

/// Instant search over the index. A newer search cancels older ones.
#[tauri::command]
pub async fn index_search(
    state: State<'_, AppState>,
    query: String,
    limit: usize,
    under: Option<String>,
) -> AppResult<crate::index::IndexResults> {
    static CANCEL: parking_lot::Mutex<Option<Arc<AtomicBool>>> = parking_lot::Mutex::new(None);
    let flag = Arc::new(AtomicBool::new(false));
    if let Some(old) = CANCEL.lock().replace(flag.clone()) {
        old.store(true, Ordering::Relaxed);
    }
    let index = state.index.clone();
    blocking(move || {
        let r = index.search(&query, limit, under.as_deref(), &flag);
        if flag.load(Ordering::Relaxed) {
            return Err(AppError::cancelled());
        }
        r
    })
    .await
}

#[tauri::command]
pub async fn file_meta(paths: Vec<String>) -> AppResult<Vec<Option<crate::meta::FileMeta>>> {
    blocking(move || Ok(crate::meta::get_many(&paths))).await
}

/// Validates a query; returns the parse error message if any.
#[tauri::command]
pub fn query_check(query: String) -> AppResult<()> {
    crate::query::Query::parse(&query).map(|_| ())
}

/// Evaluates a query against listed items: (path, is_dir, size, mtime).
#[tauri::command]
pub async fn query_filter(query: String, items: Vec<(String, bool, u64, i64)>) -> AppResult<Vec<bool>> {
    blocking(move || {
        let q = crate::query::Query::parse(&query)?;
        let threads = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4).clamp(2, 8);
        let chunk = items.len().div_ceil(threads).max(1);
        let mut out = vec![false; items.len()];
        std::thread::scope(|s| {
            for (part, slot) in items.chunks(chunk).zip(out.chunks_mut(chunk)) {
                let q = &q;
                s.spawn(move || {
                    for ((path, is_dir, size, mtime), o) in part.iter().zip(slot.iter_mut()) {
                        *o = q.matches(&crate::query::FileSubject::new(path.clone(), *is_dir, *size, *mtime));
                    }
                });
            }
        });
        Ok(out)
    })
    .await
}

#[tauri::command]
pub async fn viewer_read_bytes(path: String, offset: u64, len: usize) -> AppResult<Response> {
    let bytes = blocking(move || viewer::read_bytes(std::path::Path::new(&path), offset, len)).await?;
    Ok(Response::new(bytes))
}

/// Native icon as PNG bytes (used for icons in native popup menus).
#[tauri::command]
pub async fn icon_png(kind: String, value: String, px: u32) -> AppResult<Response> {
    let key = match kind.as_str() {
        "ext" => crate::icons::IconKey::Ext(value.to_lowercase()),
        "dir" => crate::icons::IconKey::Dir,
        "file" => crate::icons::IconKey::File,
        _ => crate::icons::IconKey::Path(value),
    };
    let png = blocking(move || crate::icons::icon_png(key, px).ok_or_else(|| AppError::new(ErrorCode::NotFound, "No icon"))).await?;
    Ok(Response::new(png.to_vec()))
}

#[tauri::command]
pub fn checksum_start(
    state: State<'_, AppState>,
    paths: Vec<String>,
    algo: String,
    channel: Channel<checksum::ChecksumEvent>,
) -> AppResult<u64> {
    let (id, flag) = state.new_op();
    std::thread::spawn(move || {
        use checksum::ChecksumEvent as E;
        let total: u64 = paths.iter().filter_map(|p| std::fs::metadata(p).ok()).map(|m| m.len()).sum();
        let mut done = 0u64;
        let mut last = std::time::Instant::now();
        for p in &paths {
            let r = checksum::hash_file(std::path::Path::new(p), &algo, &flag, |n| {
                done += n;
                if last.elapsed().as_millis() > 150 {
                    last = std::time::Instant::now();
                    let _ = channel.send(E::Progress { done_bytes: done, total_bytes: total, current: p.clone() });
                }
            });
            if flag.load(Ordering::Relaxed) {
                break;
            }
            let _ = match r {
                Ok(h) => channel.send(E::Result { path: p.clone(), hash: Some(h), error: None }),
                Err(e) => channel.send(E::Result { path: p.clone(), hash: None, error: Some(e) }),
            };
        }
        let _ = channel.send(E::Done { cancelled: flag.load(Ordering::Relaxed) });
    });
    Ok(id)
}

// ---------------------------------------------------------------------------
// FTP & vault
// ---------------------------------------------------------------------------

#[tauri::command]
pub fn vault_status(state: State<'_, AppState>) -> VaultStatus {
    state.vault.status()
}

#[tauri::command]
pub async fn vault_unlock(state: State<'_, AppState>, password: String) -> AppResult<()> {
    let v = state.vault.clone();
    blocking(move || v.unlock(&password)).await
}

#[tauri::command]
pub fn vault_lock(state: State<'_, AppState>) {
    state.vault.lock()
}

#[tauri::command]
pub async fn vault_set_master(state: State<'_, AppState>, password: Option<String>) -> AppResult<()> {
    let v = state.vault.clone();
    blocking(move || v.set_master_password(password.as_deref())).await
}

#[tauri::command]
pub async fn sites_list(state: State<'_, AppState>) -> AppResult<Vec<SiteSummary>> {
    let v = state.vault.clone();
    blocking(move || v.sites()).await
}

#[tauri::command]
pub async fn site_save(state: State<'_, AppState>, site: SiteInput) -> AppResult<SiteSummary> {
    let v = state.vault.clone();
    blocking(move || v.save(site)).await
}

#[tauri::command]
pub async fn site_delete(state: State<'_, AppState>, id: String) -> AppResult<()> {
    let v = state.vault.clone();
    blocking(move || v.delete(&id)).await
}

/// Connects to a saved or edited site. `trust_key`: SSH host key
/// fingerprint the user accepted after a `HostKeyUnknown` error.
#[tauri::command]
pub async fn remote_connect(state: State<'_, AppState>, site_id: Option<String>, site: Option<SiteInput>, trust_key: Option<String>) -> AppResult<Connected> {
    let (v, reg) = (state.vault.clone(), state.remote.clone());
    blocking(move || {
        let mut cfg = match (&site_id, &site) {
            (Some(id), _) => {
                let s = v.site(id)?;
                v.touch(id);
                s.to_config()
            }
            (None, Some(input)) => crate::vault::config_from_input(&v, input)?,
            _ => return Err(AppError::invalid("Nothing to connect to")),
        };
        cfg.trust_key = trust_key.unwrap_or_default();
        reg.open(cfg)
    })
    .await
}

#[tauri::command]
pub async fn remote_test(state: State<'_, AppState>, site: SiteInput, trust_key: Option<String>) -> AppResult<String> {
    let v = state.vault.clone();
    blocking(move || {
        let mut cfg = crate::vault::config_from_input(&v, &site)?;
        cfg.trust_key = trust_key.unwrap_or_default();
        remote::test(&cfg)
    })
    .await
}

#[tauri::command]
pub fn remote_disconnect(state: State<'_, AppState>, session: String) {
    state.remote.close(&session)
}

/// Android devices, running containers and rclone remotes.
#[tauri::command]
pub async fn remote_sources() -> AppResult<Vec<remote::Source>> {
    blocking(|| Ok(remote::sources())).await
}

/// Opens a discovered source (see `remote_sources`).
#[tauri::command]
pub async fn remote_open_source(state: State<'_, AppState>, protocol: remote::Protocol, id: String, name: String) -> AppResult<Connected> {
    let reg = state.remote.clone();
    blocking(move || reg.open(remote::source_config(protocol, &id, &name))).await
}
