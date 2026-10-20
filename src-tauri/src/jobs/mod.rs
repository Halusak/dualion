//! Background file operations with queues, pause/resume, conflict prompts
//! and throttled progress reporting.
//!
//! Every job runs on its own thread. Jobs without a queue start right away;
//! jobs sharing a queue id run one after another in queue order.

pub mod archive;
pub mod copy;
pub mod delete;
#[cfg(test)]
mod ftp_tests;
#[cfg(test)]
mod tests;
pub mod plan;
pub mod undo;
pub mod vfs;

use crate::error::{AppError, AppResult, ErrorCode};
use crate::ftp::FtpRegistry;
use crate::location::Location;
use crate::util::now_ms;
use parking_lot::{Condvar, Mutex};
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// Where job events go (the Tauri app in production, a no-op in tests).
pub type EventSink = Arc<dyn Fn(&'static str, serde_json::Value) + Send + Sync>;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum JobOp {
    Copy,
    Move,
    Delete,
    Pack,
    Extract,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub enum ConflictPolicy {
    #[default]
    Ask,
    Overwrite,
    Skip,
    OverwriteOlder,
    Rename,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct JobOptions {
    pub conflict: ConflictPolicy,
    pub verify: bool,
    pub preserve_times: bool,
    pub follow_links: bool,
    /// Wildcard mask ("*.jpg;*.png") restricting which files are copied.
    pub filter: Option<String>,
    pub to_trash: bool,
    /// Bytes per second, None = unlimited.
    pub speed_limit: Option<u64>,
    /// Compression level for packing (0–9).
    pub level: Option<u8>,
}

impl Default for JobOptions {
    fn default() -> Self {
        Self {
            conflict: ConflictPolicy::Ask,
            verify: false,
            preserve_times: true,
            follow_links: false,
            filter: None,
            to_trash: true,
            speed_limit: None,
            level: None,
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct JobRequest {
    pub op: JobOp,
    pub sources: Vec<Location>,
    /// Target directory (copy/move/extract) or archive file (pack).
    pub dest: Option<Location>,
    /// New name when a single item is copied/moved under a different name.
    pub new_name: Option<String>,
    #[serde(default)]
    pub options: JobOptions,
    pub queue: Option<u32>,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum JobStatus {
    Queued,
    Scanning,
    Running,
    Paused,
    Waiting,
    Done,
    Failed,
    Cancelled,
}

impl JobStatus {
    pub fn is_finished(self) -> bool {
        matches!(self, JobStatus::Done | JobStatus::Failed | JobStatus::Cancelled)
    }
    pub fn is_active(self) -> bool {
        !self.is_finished() && self != JobStatus::Queued
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FileBrief {
    pub path: String,
    pub size: u64,
    pub mtime: i64,
    pub is_dir: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Prompt {
    #[serde(rename_all = "camelCase")]
    Conflict { source: FileBrief, target: FileBrief, suggested: String },
    #[serde(rename_all = "camelCase")]
    Error { error: AppError, can_retry: bool },
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "action", rename_all = "camelCase")]
pub enum Answer {
    Overwrite { all: bool },
    Skip { all: bool },
    OverwriteOlder,
    Rename {
        name: Option<String>,
        #[serde(default)]
        all: bool,
    },
    Retry,
    Cancel,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct JobSnapshot {
    pub id: u64,
    pub op: JobOp,
    pub status: JobStatus,
    pub queue: Option<u32>,
    pub first_source: String,
    pub source_count: usize,
    pub dest: String,
    pub to_trash: bool,
    pub total_bytes: u64,
    pub done_bytes: u64,
    pub total_files: u64,
    pub done_files: u64,
    pub current: String,
    pub speed: f64,
    pub eta: Option<u64>,
    pub error: Option<AppError>,
    pub skipped: u64,
    pub errors: u64,
    pub prompt: Option<Prompt>,
    pub created: i64,
    pub started: Option<i64>,
    pub finished: Option<i64>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct JobDone {
    pub id: u64,
    pub op: JobOp,
    pub status: JobStatus,
    /// Locations whose listings changed (source parents and destination).
    pub touched: Vec<Location>,
    /// How to reverse what the job did (empty when nothing can be undone).
    pub undo: Vec<undo::UndoAction>,
}

struct Progress {
    status: JobStatus,
    total_bytes: u64,
    done_bytes: u64,
    total_files: u64,
    done_files: u64,
    current: String,
    error: Option<AppError>,
    skipped: u64,
    errors: u64,
    prompt: Option<Prompt>,
    started: Option<i64>,
    finished: Option<i64>,
    speed: f64,
    sample: Option<(Instant, u64)>,
}

#[derive(Default)]
struct CtlState {
    paused: bool,
    answer: Option<Answer>,
}

pub struct Job {
    pub id: u64,
    pub req: JobRequest,
    created: i64,
    cancel: AtomicBool,
    paused_flag: AtomicBool,
    ctl: Mutex<CtlState>,
    cv: Condvar,
    progress: Mutex<Progress>,
    labels: (String, usize, String),
    journal: Mutex<Vec<undo::UndoAction>>,
}

impl Job {
    fn status(&self) -> JobStatus {
        self.progress.lock().status
    }

    pub fn snapshot_done_bytes(&self) -> u64 {
        self.progress.lock().done_bytes
    }

    fn snapshot(&self) -> JobSnapshot {
        let mut p = self.progress.lock();
        let now = Instant::now();
        if p.status == JobStatus::Running {
            match p.sample {
                Some((t, b)) if now.duration_since(t) >= Duration::from_millis(400) => {
                    let dt = now.duration_since(t).as_secs_f64();
                    let inst = (p.done_bytes.saturating_sub(b)) as f64 / dt;
                    p.speed = if p.speed == 0.0 { inst } else { p.speed * 0.6 + inst * 0.4 };
                    p.sample = Some((now, p.done_bytes));
                }
                None => p.sample = Some((now, p.done_bytes)),
                _ => {}
            }
        } else if p.status != JobStatus::Waiting && p.status != JobStatus::Paused {
            p.speed = 0.0;
        }
        let eta = (p.speed > 1.0 && p.total_bytes > p.done_bytes)
            .then(|| ((p.total_bytes - p.done_bytes) as f64 / p.speed) as u64);
        JobSnapshot {
            id: self.id,
            op: self.req.op,
            status: p.status,
            queue: self.req.queue,
            first_source: self.labels.0.clone(),
            source_count: self.labels.1,
            dest: self.labels.2.clone(),
            to_trash: self.req.options.to_trash,
            total_bytes: p.total_bytes,
            done_bytes: p.done_bytes,
            total_files: p.total_files,
            done_files: p.done_files,
            current: p.current.clone(),
            speed: p.speed,
            eta,
            error: p.error.clone(),
            skipped: p.skipped,
            errors: p.errors,
            prompt: p.prompt.clone(),
            created: self.created,
            started: p.started,
            finished: p.finished,
        }
    }
}

pub enum ErrorChoice {
    Retry,
    Skip,
}

pub enum ConflictChoice {
    Overwrite,
    Skip,
    RenameTo(String),
}

/// Handle given to the operation code while a job runs.
pub struct JobCtx<'a> {
    pub job: &'a Arc<Job>,
    pub mgr: &'a Arc<JobManager>,
    skip_all_errors: AtomicBool,
    conflict: Mutex<ConflictPolicy>,
    throttle: Mutex<(Instant, u64)>,
}

impl<'a> JobCtx<'a> {
    fn new(job: &'a Arc<Job>, mgr: &'a Arc<JobManager>) -> Self {
        Self {
            job,
            mgr,
            skip_all_errors: AtomicBool::new(false),
            conflict: Mutex::new(job.req.options.conflict),
            throttle: Mutex::new((Instant::now(), 0)),
        }
    }

    pub fn req(&self) -> &JobRequest {
        &self.job.req
    }

    /// Notes a change for undo.
    pub fn record(&self, a: undo::UndoAction) {
        self.job.journal.lock().push(a);
    }

    pub fn ftp(&self) -> &FtpRegistry {
        &self.mgr.ftp
    }

    fn touch(&self) {
        self.mgr.dirty.store(true, Ordering::Relaxed);
    }

    /// Cancellation / pause point. Cheap when nothing is pending.
    pub fn checkpoint(&self) -> AppResult<()> {
        if self.job.cancel.load(Ordering::Relaxed) {
            return Err(AppError::cancelled());
        }
        if self.job.paused_flag.load(Ordering::Relaxed) {
            let prev = {
                let mut p = self.job.progress.lock();
                let prev = p.status;
                p.status = JobStatus::Paused;
                prev
            };
            self.touch();
            let mut ctl = self.job.ctl.lock();
            while ctl.paused && !self.job.cancel.load(Ordering::Relaxed) {
                self.job.cv.wait(&mut ctl);
            }
            drop(ctl);
            self.job.progress.lock().status = prev;
            self.touch();
            if self.job.cancel.load(Ordering::Relaxed) {
                return Err(AppError::cancelled());
            }
        }
        Ok(())
    }

    pub fn set_status(&self, s: JobStatus) {
        self.job.progress.lock().status = s;
        self.touch();
    }

    pub fn set_totals(&self, bytes: u64, files: u64) {
        let mut p = self.job.progress.lock();
        p.total_bytes = bytes;
        p.total_files = files;
        drop(p);
        self.touch();
    }

    pub fn add_totals(&self, bytes: u64, files: u64) {
        let mut p = self.job.progress.lock();
        p.total_bytes += bytes;
        p.total_files += files;
        drop(p);
        self.touch();
    }

    pub fn add_bytes(&self, n: u64) {
        self.job.progress.lock().done_bytes += n;
        self.touch();
        if let Some(limit) = self.job.req.options.speed_limit.filter(|l| *l > 0) {
            self.throttle(n, limit);
        }
    }

    pub fn sub_bytes(&self, n: u64) {
        let mut p = self.job.progress.lock();
        p.done_bytes = p.done_bytes.saturating_sub(n);
    }

    fn throttle(&self, n: u64, limit: u64) {
        let mut t = self.throttle.lock();
        t.1 += n;
        let window = Duration::from_millis(250);
        let allowed = limit / 4;
        if t.1 >= allowed {
            let elapsed = t.0.elapsed();
            if elapsed < window {
                std::thread::sleep(window - elapsed);
            }
            *t = (Instant::now(), 0);
        } else if t.0.elapsed() >= window {
            *t = (Instant::now(), 0);
        }
    }

    pub fn file_done(&self) {
        self.job.progress.lock().done_files += 1;
        self.touch();
    }

    /// A skipped file still counts towards overall progress.
    pub fn skip(&self, bytes: u64) {
        let mut p = self.job.progress.lock();
        p.done_bytes += bytes;
        p.done_files += 1;
        p.skipped += 1;
        drop(p);
        self.touch();
    }

    pub fn set_current(&self, s: &str) {
        let mut p = self.job.progress.lock();
        if p.current != s {
            p.current.clear();
            p.current.push_str(s);
        }
        drop(p);
        self.touch();
    }

    /// Blocks the worker until the user answers the prompt (or cancels the job).
    pub fn ask(&self, prompt: Prompt) -> AppResult<Answer> {
        {
            let mut p = self.job.progress.lock();
            p.prompt = Some(prompt);
            p.status = JobStatus::Waiting;
        }
        self.touch();
        (self.mgr.emit)("job-prompt", serde_json::json!(self.job.id));
        let mut ctl = self.job.ctl.lock();
        ctl.answer = None;
        let answer = loop {
            if self.job.cancel.load(Ordering::Relaxed) {
                break Answer::Cancel;
            }
            if let Some(a) = ctl.answer.take() {
                break a;
            }
            self.job.cv.wait(&mut ctl);
        };
        drop(ctl);
        {
            let mut p = self.job.progress.lock();
            p.prompt = None;
            p.status = JobStatus::Running;
            p.sample = None;
        }
        self.touch();
        if matches!(answer, Answer::Cancel) {
            self.job.cancel.store(true, Ordering::Relaxed);
            return Err(AppError::cancelled());
        }
        Ok(answer)
    }

    /// Asks what to do about a failed item: retry, skip (once / always) or cancel.
    pub fn handle_error(&self, err: AppError) -> AppResult<ErrorChoice> {
        if err.is_cancelled() {
            return Err(err);
        }
        self.job.progress.lock().errors += 1;
        if self.skip_all_errors.load(Ordering::Relaxed) {
            return Ok(ErrorChoice::Skip);
        }
        let can_retry = err.code != ErrorCode::IntoItself;
        match self.ask(Prompt::Error { error: err, can_retry })? {
            Answer::Retry => {
                self.job.progress.lock().errors -= 1;
                Ok(ErrorChoice::Retry)
            }
            Answer::Skip { all } => {
                if all {
                    self.skip_all_errors.store(true, Ordering::Relaxed);
                }
                Ok(ErrorChoice::Skip)
            }
            _ => Err(AppError::cancelled()),
        }
    }

    /// Decides what to do when the target already exists.
    pub fn resolve_conflict(
        &self,
        source: FileBrief,
        target: FileBrief,
        suggested: impl FnOnce() -> String,
    ) -> AppResult<ConflictChoice> {
        let policy = *self.conflict.lock();
        let older = |s: &FileBrief, t: &FileBrief| s.mtime > t.mtime;
        match policy {
            ConflictPolicy::Overwrite => return Ok(ConflictChoice::Overwrite),
            ConflictPolicy::Skip => return Ok(ConflictChoice::Skip),
            ConflictPolicy::OverwriteOlder => {
                return Ok(if older(&source, &target) { ConflictChoice::Overwrite } else { ConflictChoice::Skip });
            }
            ConflictPolicy::Rename => return Ok(ConflictChoice::RenameTo(suggested())),
            ConflictPolicy::Ask => {}
        }
        let suggested = suggested();
        let src_newer = older(&source, &target);
        let answer = self.ask(Prompt::Conflict { source, target, suggested: suggested.clone() })?;
        Ok(match answer {
            Answer::Overwrite { all } => {
                if all {
                    *self.conflict.lock() = ConflictPolicy::Overwrite;
                }
                ConflictChoice::Overwrite
            }
            Answer::Skip { all } => {
                if all {
                    *self.conflict.lock() = ConflictPolicy::Skip;
                }
                ConflictChoice::Skip
            }
            Answer::OverwriteOlder => {
                *self.conflict.lock() = ConflictPolicy::OverwriteOlder;
                if src_newer { ConflictChoice::Overwrite } else { ConflictChoice::Skip }
            }
            Answer::Rename { name, all } => {
                if all {
                    *self.conflict.lock() = ConflictPolicy::Rename;
                }
                ConflictChoice::RenameTo(name.filter(|n| !n.trim().is_empty()).unwrap_or(suggested))
            }
            Answer::Retry => ConflictChoice::Overwrite,
            Answer::Cancel => return Err(AppError::cancelled()),
        })
    }

    pub fn buffer_size(&self) -> usize {
        4 * 1024 * 1024
    }
}

pub struct JobManager {
    emit: EventSink,
    pub ftp: Arc<FtpRegistry>,
    jobs: Mutex<Vec<Arc<Job>>>,
    next_id: Mutex<u64>,
    dirty: AtomicBool,
}

impl JobManager {
    pub fn new(emit: EventSink, ftp: Arc<FtpRegistry>) -> Arc<Self> {
        let mgr = Arc::new(Self {
            emit,
            ftp,
            jobs: Mutex::new(Vec::new()),
            next_id: Mutex::new(1),
            dirty: AtomicBool::new(false),
        });
        let weak = Arc::downgrade(&mgr);
        std::thread::Builder::new()
            .name("job-reporter".into())
            .spawn(move || loop {
                std::thread::sleep(Duration::from_millis(200));
                let Some(m) = weak.upgrade() else { break };
                let running = m.jobs.lock().iter().any(|j| j.status() == JobStatus::Running);
                if m.dirty.swap(false, Ordering::Relaxed) || running {
                    (m.emit)("jobs", serde_json::to_value(m.snapshot()).unwrap_or_default());
                }
            })
            .expect("spawn reporter");
        mgr
    }

    pub fn snapshot(&self) -> Vec<JobSnapshot> {
        self.jobs.lock().iter().map(|j| j.snapshot()).collect()
    }

    pub fn active_count(&self) -> usize {
        self.jobs.lock().iter().filter(|j| !j.status().is_finished()).count()
    }

    pub fn enqueue(self: &Arc<Self>, req: JobRequest) -> AppResult<u64> {
        if req.sources.is_empty() {
            return Err(AppError::invalid("Nothing to do"));
        }
        let id = {
            let mut n = self.next_id.lock();
            let id = *n;
            *n += 1;
            id
        };
        let labels = vfs::describe(&req, &self.ftp);
        let job = Arc::new(Job {
            id,
            created: now_ms(),
            cancel: AtomicBool::new(false),
            paused_flag: AtomicBool::new(false),
            ctl: Mutex::new(CtlState::default()),
            cv: Condvar::new(),
            progress: Mutex::new(Progress {
                status: JobStatus::Queued,
                total_bytes: 0,
                done_bytes: 0,
                total_files: 0,
                done_files: 0,
                current: String::new(),
                error: None,
                skipped: 0,
                errors: 0,
                prompt: None,
                started: None,
                finished: None,
                speed: 0.0,
                sample: None,
            }),
            labels,
            req,
            journal: Mutex::new(Vec::new()),
        });
        self.jobs.lock().push(job);
        self.dirty.store(true, Ordering::Relaxed);
        self.schedule();
        Ok(id)
    }

    /// Starts immediate jobs and the head of every idle queue.
    fn schedule(self: &Arc<Self>) {
        let to_start: Vec<Arc<Job>> = {
            let jobs = self.jobs.lock();
            let mut busy_queues: Vec<u32> = jobs
                .iter()
                .filter(|j| j.status().is_active())
                .filter_map(|j| j.req.queue)
                .collect();
            let mut start = Vec::new();
            for j in jobs.iter() {
                let mut p = j.progress.lock();
                if p.status != JobStatus::Queued {
                    continue;
                }
                match j.req.queue {
                    None => {}
                    Some(q) if busy_queues.contains(&q) => continue,
                    Some(q) => busy_queues.push(q),
                }
                p.status = JobStatus::Scanning;
                p.started = Some(now_ms());
                start.push(j.clone());
            }
            start
        };
        for job in to_start {
            self.spawn(job);
        }
    }

    fn spawn(self: &Arc<Self>, job: Arc<Job>) {
        let mgr = self.clone();
        std::thread::Builder::new()
            .name(format!("job-{}", job.id))
            .stack_size(8 * 1024 * 1024)
            .spawn(move || {
                let before = undo::before(&job.req);
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    let ctx = JobCtx::new(&job, &mgr);
                    match job.req.op {
                        JobOp::Copy | JobOp::Move => copy::run(&ctx),
                        JobOp::Delete => delete::run(&ctx),
                        JobOp::Pack => archive::pack(&ctx),
                        JobOp::Extract => archive::extract(&ctx),
                    }
                }));
                let result = result.unwrap_or_else(|_| Err(AppError::other("Internal error (panic)")));
                let status = {
                    let mut p = job.progress.lock();
                    p.status = match &result {
                        Ok(()) => JobStatus::Done,
                        Err(e) if e.is_cancelled() => JobStatus::Cancelled,
                        Err(_) => JobStatus::Failed,
                    };
                    if let Err(e) = &result {
                        if !e.is_cancelled() {
                            p.error = Some(e.clone());
                        }
                    }
                    p.prompt = None;
                    p.finished = Some(now_ms());
                    p.current.clear();
                    p.status
                };
                mgr.dirty.store(true, Ordering::Relaxed);
                let journal = std::mem::take(&mut *job.journal.lock());
                let undo = undo::after(&job.req, before, journal);
                let done = JobDone { id: job.id, op: job.req.op, status, touched: vfs::touched_locations(&job.req), undo };
                (mgr.emit)("job-done", serde_json::to_value(done).unwrap_or_default());
                mgr.schedule();
            })
            .expect("spawn job thread");
    }

    fn find(&self, id: u64) -> AppResult<Arc<Job>> {
        self.jobs
            .lock()
            .iter()
            .find(|j| j.id == id)
            .cloned()
            .ok_or_else(|| AppError::new(ErrorCode::NotFound, "Job not found"))
    }

    pub fn set_paused(&self, id: u64, paused: bool) -> AppResult<()> {
        let job = self.find(id)?;
        job.paused_flag.store(paused, Ordering::Relaxed);
        job.ctl.lock().paused = paused;
        job.cv.notify_all();
        self.dirty.store(true, Ordering::Relaxed);
        Ok(())
    }

    pub fn cancel(self: &Arc<Self>, id: u64) -> AppResult<()> {
        let job = self.find(id)?;
        job.cancel.store(true, Ordering::Relaxed);
        {
            let mut p = job.progress.lock();
            if p.status == JobStatus::Queued {
                p.status = JobStatus::Cancelled;
                p.finished = Some(now_ms());
            }
        }
        job.cv.notify_all();
        self.dirty.store(true, Ordering::Relaxed);
        self.schedule();
        Ok(())
    }

    pub fn answer(&self, id: u64, answer: Answer) -> AppResult<()> {
        let job = self.find(id)?;
        job.ctl.lock().answer = Some(answer);
        job.cv.notify_all();
        Ok(())
    }

    /// Takes a queued job out of its queue and starts it immediately.
    pub fn start_now(self: &Arc<Self>, id: u64) -> AppResult<()> {
        let job = self.find(id)?;
        if job.status() != JobStatus::Queued {
            return Ok(());
        }
        {
            let mut p = job.progress.lock();
            p.status = JobStatus::Scanning;
            p.started = Some(now_ms());
        }
        self.dirty.store(true, Ordering::Relaxed);
        self.spawn(job);
        Ok(())
    }

    /// Moves a queued job up (-1) or down (+1) within its queue.
    pub fn move_job(&self, id: u64, delta: i32) -> AppResult<()> {
        let mut jobs = self.jobs.lock();
        let Some(pos) = jobs.iter().position(|j| j.id == id) else { return Ok(()) };
        let queue = jobs[pos].req.queue;
        let same: Vec<usize> = (0..jobs.len())
            .filter(|&i| jobs[i].req.queue == queue && jobs[i].status() == JobStatus::Queued)
            .collect();
        let Some(k) = same.iter().position(|&i| i == pos) else { return Ok(()) };
        let target = k as i32 + delta;
        if target < 0 || target as usize >= same.len() {
            return Ok(());
        }
        jobs.swap(pos, same[target as usize]);
        self.dirty.store(true, Ordering::Relaxed);
        Ok(())
    }

    pub fn clear_finished(&self) {
        self.jobs.lock().retain(|j| !j.status().is_finished());
        self.dirty.store(true, Ordering::Relaxed);
    }

    pub fn cancel_all(&self) {
        for j in self.jobs.lock().iter() {
            j.cancel.store(true, Ordering::Relaxed);
            j.cv.notify_all();
        }
    }
}
