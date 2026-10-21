//! End-to-end job tests against real servers (copy up, rename, copy back,
//! delete). Ignored by default; each runs when its variable is set:
//!
//! - `DUALION_FTP=127.0.0.1:2121`, `DUALION_FTPS=127.0.0.1:2122` (user `jan`, password `tajne`)
//! - `DUALION_SFTP=127.0.0.1:2222` (+ `DUALION_SFTP_USER`, `DUALION_SFTP_KEY`, `DUALION_SFTP_DIR`)
//! - `DUALION_WEBDAV=http://127.0.0.1:8080` (+ `DUALION_WEBDAV_USER`, `DUALION_WEBDAV_PASS`)
//! - `DUALION_S3=http://127.0.0.1:9000` (+ `DUALION_S3_KEY`, `DUALION_S3_SECRET`, `DUALION_S3_BUCKET`)
//! - `DUALION_RCLONE=<remote>`, `DUALION_ADB_SERIAL=emulator-5554`, `DUALION_DOCKER=<container>`
//!
//! `cargo test e2e -- --ignored --nocapture`

use super::*;
use crate::remote::{Protocol, RemoteConfig, Registry};
use crate::util::path_str;
use std::fs;

fn env(var: &str) -> Option<String> {
    std::env::var(var).ok().filter(|v| !v.is_empty())
}

fn wait(mgr: &Arc<JobManager>, id: u64) -> JobSnapshot {
    let deadline = Instant::now() + Duration::from_secs(120);
    loop {
        let s = mgr.snapshot().into_iter().find(|j| j.id == id).unwrap();
        if s.status.is_finished() {
            return s;
        }
        assert!(Instant::now() < deadline, "job timeout: {s:?}");
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn job(mgr: &Arc<JobManager>, op: JobOp, sources: Vec<Location>, dest: Option<Location>, new_name: Option<&str>) -> JobSnapshot {
    let id = mgr
        .enqueue(JobRequest {
            op,
            sources,
            dest,
            new_name: new_name.map(str::to_string),
            options: JobOptions { verify: op == JobOp::Copy, to_trash: false, ..Default::default() },
            queue: None,
        })
        .unwrap();
    wait(mgr, id)
}

/// Copies a small tree up into `base`, renames, copies it back, deletes it.
fn roundtrip(cfg: RemoteConfig, base: &str) {
    let reg = Arc::new(Registry::default());
    let mgr = JobManager::new(Arc::new(|_, _| {}), reg.clone());
    let conn = reg.open(cfg).expect("connect");
    let session = conn.session.clone();
    let s = reg.get(&session).unwrap();
    let remote = |p: &str| Location::Remote { session: session.clone(), path: p.to_string() };
    let dir = crate::remote::join(base, &format!("e2e-{}", uuid::Uuid::new_v4().simple()));
    s.fs().mkdir(&dir).unwrap();

    // Upload a small tree with Czech names.
    let local = std::env::temp_dir().join(format!("dualion-remote-{}", uuid::Uuid::new_v4()));
    fs::create_dir_all(local.join("data/vnořená")).unwrap();
    fs::write(local.join("data/příliš žluťoučký.txt"), "kůň").unwrap();
    fs::write(local.join("data/vnořená/big.bin"), vec![42u8; 3 * 1024 * 1024]).unwrap();
    let r = job(&mgr, JobOp::Copy, vec![Location::local(path_str(&local.join("data")))], Some(remote(&dir)), None);
    assert_eq!(r.status, JobStatus::Done, "upload: {:?}", r.error);

    let listing = s.list(&format!("{dir}/data")).unwrap();
    let names: Vec<_> = listing.entries.iter().map(|e| e.name.clone()).collect();
    assert!(names.contains(&"příliš žluťoučký.txt".to_string()), "{names:?}");
    assert!(names.contains(&"vnořená".to_string()), "{names:?}");

    // Rename on the server (move within the session).
    let r = job(&mgr, JobOp::Move, vec![remote(&format!("{dir}/data/příliš žluťoučký.txt"))], Some(remote(&format!("{dir}/data"))), Some("kun.txt"));
    assert_eq!(r.status, JobStatus::Done, "rename: {:?}", r.error);

    // Download it back.
    let back = local.join("back");
    fs::create_dir_all(&back).unwrap();
    let r = job(&mgr, JobOp::Copy, vec![remote(&format!("{dir}/data"))], Some(Location::local(path_str(&back))), None);
    assert_eq!(r.status, JobStatus::Done, "download: {:?}", r.error);
    assert_eq!(fs::read_to_string(back.join("data/kun.txt")).unwrap(), "kůň");
    assert_eq!(fs::metadata(back.join("data/vnořená/big.bin")).unwrap().len(), 3 * 1024 * 1024);

    // Delete the remote tree.
    let r = job(&mgr, JobOp::Delete, vec![remote(&dir)], None, None);
    assert_eq!(r.status, JobStatus::Done, "delete: {:?}", r.error);
    let parent = s.list(base).unwrap();
    assert!(!parent.entries.iter().any(|e| crate::remote::join(base, &e.name) == dir), "still there");

    reg.close(&session);
    fs::remove_dir_all(&local).unwrap();
}

fn hostport(v: &str) -> (String, u16) {
    let (h, p) = v.rsplit_once(':').unwrap_or((v, "0"));
    (h.to_string(), p.parse().unwrap_or(0))
}

fn ftp(var: &str, protocol: Protocol) {
    let Some(addr) = env(var) else { return };
    let (host, port) = hostport(&addr);
    let cfg = RemoteConfig { protocol, host, port, username: "jan".into(), password: "tajne".into(), passive: true, accept_invalid_certs: true, ..Default::default() };
    roundtrip(cfg, "/");
}

#[test]
#[ignore]
fn ftp_e2e_plain() {
    ftp("DUALION_FTP", Protocol::Ftp);
}

#[test]
#[ignore]
fn ftp_e2e_explicit_tls() {
    ftp("DUALION_FTPS", Protocol::FtpsExplicit);
}

#[test]
#[ignore]
fn sftp_e2e() {
    let Some(addr) = env("DUALION_SFTP") else { return };
    let (host, port) = hostport(&addr);
    let mut cfg = RemoteConfig { protocol: Protocol::Sftp, host, port, username: env("DUALION_SFTP_USER").unwrap_or_default(), key_path: env("DUALION_SFTP_KEY").unwrap_or_default(), ..Default::default() };
    // Accept the test server's key: ask once, then trust the fingerprint.
    if let Err(e) = crate::remote::open(&cfg) {
        assert_eq!(e.code, ErrorCode::HostKeyUnknown, "{e:?}");
        cfg.trust_key = e.message;
    }
    let base = env("DUALION_SFTP_DIR").unwrap_or_else(|| "/tmp".into());
    roundtrip(cfg, &base);
}

#[test]
#[ignore]
fn webdav_e2e() {
    let Some(url) = env("DUALION_WEBDAV") else { return };
    let cfg = RemoteConfig { protocol: Protocol::Webdav, host: url, username: env("DUALION_WEBDAV_USER").unwrap_or_default(), password: env("DUALION_WEBDAV_PASS").unwrap_or_default().into(), ..Default::default() };
    roundtrip(cfg, "/");
}

#[test]
#[ignore]
fn s3_e2e() {
    let Some(url) = env("DUALION_S3") else { return };
    let bucket = env("DUALION_S3_BUCKET").unwrap_or_else(|| "dualion".into());
    let cfg = RemoteConfig {
        protocol: Protocol::S3,
        host: url,
        username: env("DUALION_S3_KEY").unwrap_or_default(),
        password: env("DUALION_S3_SECRET").unwrap_or_default().into(),
        remote_path: bucket.clone(),
        ..Default::default()
    };
    roundtrip(cfg, &format!("/{bucket}"));
}

#[test]
#[ignore]
fn rclone_e2e() {
    let Some(name) = env("DUALION_RCLONE") else { return };
    roundtrip(crate::remote::source_config(Protocol::Rclone, &name, &name), "/");
}

#[test]
#[ignore]
fn adb_e2e() {
    let Some(serial) = env("DUALION_ADB_SERIAL") else { return };
    roundtrip(crate::remote::source_config(Protocol::Adb, &serial, &serial), "/sdcard/Download");
}

#[test]
#[ignore]
fn docker_e2e() {
    let Some(id) = env("DUALION_DOCKER") else { return };
    roundtrip(crate::remote::source_config(Protocol::Docker, &id, &id), "/tmp");
}
