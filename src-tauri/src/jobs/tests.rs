//! End-to-end tests of the job engine on the local file system.

use super::*;
use crate::util::path_str;
use std::fs;
use std::path::{Path, PathBuf};

fn manager() -> Arc<JobManager> {
    JobManager::new(Arc::new(|_, _| {}), Arc::new(RemoteRegistry::default()))
}

fn tmp() -> PathBuf {
    let d = std::env::temp_dir().join(format!("dualion-jobs-{}", uuid::Uuid::new_v4()));
    fs::create_dir_all(&d).unwrap();
    d
}

fn loc(p: &Path) -> Location {
    Location::local(path_str(p))
}

fn run(mgr: &Arc<JobManager>, req: JobRequest) -> JobSnapshot {
    let id = mgr.enqueue(req).unwrap();
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        let snap = mgr.snapshot().into_iter().find(|j| j.id == id).unwrap();
        if snap.status.is_finished() {
            return snap;
        }
        assert!(Instant::now() < deadline, "job did not finish: {snap:?}");
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn req(op: JobOp, sources: Vec<Location>, dest: Option<Location>, conflict: ConflictPolicy) -> JobRequest {
    JobRequest {
        op,
        sources,
        dest,
        new_name: None,
        options: JobOptions { conflict, to_trash: false, ..Default::default() },
        queue: None,
    }
}

#[test]
fn copies_tree_with_progress_and_times() {
    let mgr = manager();
    let root = tmp();
    let src = root.join("src");
    fs::create_dir_all(src.join("sub/deep")).unwrap();
    fs::write(src.join("a.txt"), "alpha").unwrap();
    fs::write(src.join("sub/deep/b.bin"), vec![7u8; 3 * 1024 * 1024]).unwrap();
    let old = filetime::FileTime::from_unix_time(1_600_000_000, 0);
    filetime::set_file_mtime(src.join("a.txt"), old).unwrap();
    let dst = root.join("dst");
    fs::create_dir_all(&dst).unwrap();

    let snap = run(&mgr, req(JobOp::Copy, vec![loc(&src)], Some(loc(&dst)), ConflictPolicy::Ask));
    assert_eq!(snap.status, JobStatus::Done, "{:?}", snap.error);
    assert_eq!(fs::read_to_string(dst.join("src/a.txt")).unwrap(), "alpha");
    assert_eq!(fs::metadata(dst.join("src/sub/deep/b.bin")).unwrap().len(), 3 * 1024 * 1024);
    assert_eq!(snap.total_files, 2);
    assert_eq!(snap.done_bytes, snap.total_bytes);
    let m = fs::metadata(dst.join("src/a.txt")).unwrap();
    assert_eq!(filetime::FileTime::from_last_modification_time(&m).unix_seconds(), 1_600_000_000);
    assert!(src.join("a.txt").exists());
    fs::remove_dir_all(&root).unwrap();
}

#[test]
fn conflict_policies() {
    let mgr = manager();
    let root = tmp();
    fs::write(root.join("f.txt"), "new").unwrap();
    let dst = root.join("dst");
    fs::create_dir_all(&dst).unwrap();
    fs::write(dst.join("f.txt"), "old").unwrap();

    let s = run(&mgr, req(JobOp::Copy, vec![loc(&root.join("f.txt"))], Some(loc(&dst)), ConflictPolicy::Skip));
    assert_eq!((s.status, s.skipped), (JobStatus::Done, 1));
    assert_eq!(fs::read_to_string(dst.join("f.txt")).unwrap(), "old");

    run(&mgr, req(JobOp::Copy, vec![loc(&root.join("f.txt"))], Some(loc(&dst)), ConflictPolicy::Rename));
    assert_eq!(fs::read_to_string(dst.join("f (2).txt")).unwrap(), "new");

    run(&mgr, req(JobOp::Copy, vec![loc(&root.join("f.txt"))], Some(loc(&dst)), ConflictPolicy::Overwrite));
    assert_eq!(fs::read_to_string(dst.join("f.txt")).unwrap(), "new");
    fs::remove_dir_all(&root).unwrap();
}

#[test]
fn copy_into_same_folder_duplicates() {
    let mgr = manager();
    let root = tmp();
    fs::write(root.join("doc.md"), "x").unwrap();
    let s = run(&mgr, req(JobOp::Copy, vec![loc(&root.join("doc.md"))], Some(loc(&root)), ConflictPolicy::Ask));
    assert_eq!(s.status, JobStatus::Done);
    assert!(root.join("doc (2).md").exists());
    fs::remove_dir_all(&root).unwrap();
}

#[test]
fn refuses_copy_into_itself() {
    let mgr = manager();
    let root = tmp();
    fs::create_dir_all(root.join("a/b")).unwrap();
    let s = run(&mgr, req(JobOp::Copy, vec![loc(&root.join("a"))], Some(loc(&root.join("a/b"))), ConflictPolicy::Ask));
    assert_eq!(s.status, JobStatus::Failed);
    assert_eq!(s.error.unwrap().code, ErrorCode::IntoItself);
    fs::remove_dir_all(&root).unwrap();
}

#[test]
fn moves_and_merges_folders() {
    let mgr = manager();
    let root = tmp();
    fs::create_dir_all(root.join("src/photos")).unwrap();
    fs::write(root.join("src/photos/1.jpg"), "one").unwrap();
    fs::create_dir_all(root.join("dst/photos")).unwrap();
    fs::write(root.join("dst/photos/2.jpg"), "two").unwrap();

    let s = run(&mgr, req(JobOp::Move, vec![loc(&root.join("src/photos"))], Some(loc(&root.join("dst"))), ConflictPolicy::Ask));
    assert_eq!(s.status, JobStatus::Done, "{:?}", s.error);
    assert_eq!(fs::read_to_string(root.join("dst/photos/1.jpg")).unwrap(), "one");
    assert!(root.join("dst/photos/2.jpg").exists());
    assert!(!root.join("src/photos").exists(), "emptied source folder is removed");
    fs::remove_dir_all(&root).unwrap();
}

/// Moves to another volume fall back to copy + delete, also for items that
/// are merged into existing folders there.
/// DUALION_OTHER_VOLUME=/Volumes/Test cargo test move_across_volumes -- --ignored
#[test]
#[ignore]
fn move_across_volumes() {
    let other = PathBuf::from(std::env::var("DUALION_OTHER_VOLUME").expect("DUALION_OTHER_VOLUME"));
    let mgr = manager();
    let root = tmp();
    let dst = other.join(format!("dualion-move-{}", uuid::Uuid::new_v4()));
    fs::create_dir_all(root.join("photos/sub")).unwrap();
    fs::write(root.join("photos/1.jpg"), "one").unwrap();
    fs::write(root.join("photos/sub/2.jpg"), "two").unwrap();
    fs::write(root.join("123.pqm700"), "file").unwrap();
    fs::create_dir_all(dst.join("photos")).unwrap();
    fs::write(dst.join("photos/old.jpg"), "old").unwrap();

    let sources = vec![loc(&root.join("123.pqm700")), loc(&root.join("photos"))];
    let s = run(&mgr, req(JobOp::Move, sources, Some(loc(&dst)), ConflictPolicy::Ask));
    assert_eq!(s.status, JobStatus::Done, "{:?}", s.error);
    assert_eq!(fs::read_to_string(dst.join("123.pqm700")).unwrap(), "file");
    assert_eq!(fs::read_to_string(dst.join("photos/sub/2.jpg")).unwrap(), "two");
    assert!(dst.join("photos/old.jpg").exists());
    assert!(!root.join("123.pqm700").exists() && !root.join("photos").exists(), "sources are removed");
    fs::remove_dir_all(&root).unwrap();
    fs::remove_dir_all(&dst).unwrap();
}

/// Copying / moving a file onto itself through another path (a symlinked
/// folder) with "overwrite" must never destroy it.
#[cfg(unix)]
#[test]
fn never_overwrites_the_source_through_an_alias() {
    let mgr = manager();
    let root = tmp();
    fs::create_dir_all(root.join("real")).unwrap();
    let big = vec![9u8; 3 * 1024 * 1024];
    fs::write(root.join("real/small.txt"), "keep me").unwrap();
    fs::write(root.join("real/big.bin"), &big).unwrap();
    std::os::unix::fs::symlink(root.join("real"), root.join("alias")).unwrap();
    let sources = vec![loc(&root.join("real/small.txt")), loc(&root.join("real/big.bin"))];
    for op in [JobOp::Copy, JobOp::Move] {
        let s = run(&mgr, req(op, sources.clone(), Some(loc(&root.join("alias"))), ConflictPolicy::Overwrite));
        assert_eq!(s.status, JobStatus::Done, "{op:?}: {:?}", s.error);
        assert_eq!(fs::read_to_string(root.join("real/small.txt")).unwrap(), "keep me", "{op:?}");
        assert_eq!(fs::read(root.join("real/big.bin")).unwrap(), big, "{op:?}");
    }
    fs::remove_dir_all(&root).unwrap();
}

#[test]
fn move_with_new_name_is_rename() {
    let mgr = manager();
    let root = tmp();
    fs::write(root.join("a.txt"), "x").unwrap();
    let mut r = req(JobOp::Move, vec![loc(&root.join("a.txt"))], Some(loc(&root)), ConflictPolicy::Ask);
    r.new_name = Some("b.txt".into());
    assert_eq!(run(&mgr, r).status, JobStatus::Done);
    assert!(!root.join("a.txt").exists() && root.join("b.txt").exists());
    fs::remove_dir_all(&root).unwrap();
}

#[test]
fn filtered_copy_only_matching_files() {
    let mgr = manager();
    let root = tmp();
    fs::create_dir_all(root.join("src/x")).unwrap();
    fs::write(root.join("src/a.jpg"), "1").unwrap();
    fs::write(root.join("src/b.txt"), "2").unwrap();
    fs::write(root.join("src/x/c.JPG"), "3").unwrap();
    let dst = root.join("dst");
    fs::create_dir_all(&dst).unwrap();
    let mut r = req(JobOp::Copy, vec![loc(&root.join("src"))], Some(loc(&dst)), ConflictPolicy::Ask);
    r.options.filter = Some("*.jpg".into());
    assert_eq!(run(&mgr, r).status, JobStatus::Done);
    assert!(dst.join("src/a.jpg").exists());
    assert!(dst.join("src/x/c.JPG").exists());
    assert!(!dst.join("src/b.txt").exists());
    fs::remove_dir_all(&root).unwrap();
}

#[test]
fn deletes_permanently() {
    let mgr = manager();
    let root = tmp();
    fs::create_dir_all(root.join("d/e")).unwrap();
    fs::write(root.join("d/e/f"), "x").unwrap();
    fs::write(root.join("g"), "y").unwrap();
    let s = run(&mgr, req(JobOp::Delete, vec![loc(&root.join("d")), loc(&root.join("g"))], None, ConflictPolicy::Ask));
    assert_eq!(s.status, JobStatus::Done);
    assert!(!root.join("d").exists() && !root.join("g").exists());
    fs::remove_dir_all(&root).unwrap();
}

#[test]
fn pack_and_extract_roundtrip() {
    let mgr = manager();
    let root = tmp();
    fs::create_dir_all(root.join("data/inner")).unwrap();
    fs::write(root.join("data/readme.txt"), "hello zip").unwrap();
    fs::write(root.join("data/inner/n.bin"), vec![1u8; 100_000]).unwrap();
    let zip = root.join("out.zip");
    let s = run(&mgr, req(JobOp::Pack, vec![loc(&root.join("data"))], Some(loc(&zip)), ConflictPolicy::Ask));
    assert_eq!(s.status, JobStatus::Done, "{:?}", s.error);
    assert!(zip.exists());

    let out = root.join("unpacked");
    fs::create_dir_all(&out).unwrap();
    let arch = Location::Archive { archive: path_str(&zip), inner: "data/inner".into() };
    let s = run(&mgr, req(JobOp::Extract, vec![arch], Some(loc(&out)), ConflictPolicy::Ask));
    assert_eq!(s.status, JobStatus::Done, "{:?}", s.error);
    assert_eq!(fs::metadata(out.join("inner/n.bin")).unwrap().len(), 100_000);

    let whole = Location::Archive { archive: path_str(&zip), inner: String::new() };
    run(&mgr, req(JobOp::Extract, vec![whole], Some(loc(&out)), ConflictPolicy::Ask));
    assert_eq!(fs::read_to_string(out.join("data/readme.txt")).unwrap(), "hello zip");
    fs::remove_dir_all(&root).unwrap();
}

#[test]
fn queue_runs_jobs_in_order() {
    let mgr = manager();
    let root = tmp();
    fs::write(root.join("big.bin"), vec![0u8; 8 * 1024 * 1024]).unwrap();
    let (d1, d2) = (root.join("d1"), root.join("d2"));
    fs::create_dir_all(&d1).unwrap();
    fs::create_dir_all(&d2).unwrap();
    let mut a = req(JobOp::Copy, vec![loc(&root.join("big.bin"))], Some(loc(&d1)), ConflictPolicy::Ask);
    a.queue = Some(1);
    let mut b = req(JobOp::Copy, vec![loc(&root.join("big.bin"))], Some(loc(&d2)), ConflictPolicy::Ask);
    b.queue = Some(1);
    let ida = mgr.enqueue(a).unwrap();
    let idb = mgr.enqueue(b).unwrap();
    let snap = |id| mgr.snapshot().into_iter().find(|j| j.id == id).unwrap();
    // While the first one runs the second waits in the queue.
    if !snap(ida).status.is_finished() {
        assert_eq!(snap(idb).status, JobStatus::Queued);
    }
    let deadline = Instant::now() + Duration::from_secs(20);
    while !(snap(ida).status.is_finished() && snap(idb).status.is_finished()) {
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(snap(idb).started.unwrap() >= snap(ida).finished.unwrap());
    assert!(d1.join("big.bin").exists() && d2.join("big.bin").exists());
    fs::remove_dir_all(&root).unwrap();
}

#[test]
fn pause_and_cancel() {
    let mgr = manager();
    let root = tmp();
    fs::create_dir_all(root.join("src")).unwrap();
    for i in 0..40 {
        fs::write(root.join(format!("src/{i}.bin")), vec![0u8; 512 * 1024]).unwrap();
    }
    let dst = root.join("dst");
    fs::create_dir_all(&dst).unwrap();
    let mut r = req(JobOp::Copy, vec![loc(&root.join("src"))], Some(loc(&dst)), ConflictPolicy::Ask);
    r.options.speed_limit = Some(4 * 1024 * 1024);
    let id = mgr.enqueue(r).unwrap();
    std::thread::sleep(Duration::from_millis(300));
    mgr.set_paused(id, true).unwrap();
    std::thread::sleep(Duration::from_millis(400));
    let a = mgr.snapshot().into_iter().find(|j| j.id == id).unwrap();
    assert_eq!(a.status, JobStatus::Paused);
    std::thread::sleep(Duration::from_millis(300));
    let b = mgr.snapshot().into_iter().find(|j| j.id == id).unwrap();
    assert_eq!(a.done_bytes, b.done_bytes, "no progress while paused");
    mgr.cancel(id).unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let s = mgr.snapshot().into_iter().find(|j| j.id == id).unwrap();
        if s.status.is_finished() {
            assert_eq!(s.status, JobStatus::Cancelled);
            break;
        }
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(10));
    }
    fs::remove_dir_all(&root).unwrap();
}
