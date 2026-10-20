//! FTP / FTPS end-to-end tests against a real server. Ignored by default;
//! run with e.g.
//! `DUALION_FTP=127.0.0.1:2121 DUALION_FTPS=127.0.0.1:2122 cargo test ftp_e2e -- --ignored`
//! (user `jan`, password `tajne`, write access).

use super::*;
use crate::ftp::{FtpConfig, FtpProtocol};
use crate::util::path_str;
use std::fs;

fn cfg(var: &str, protocol: FtpProtocol) -> Option<FtpConfig> {
    let addr = std::env::var(var).ok()?;
    let (host, port) = addr.split_once(':')?;
    let mut c = FtpConfig::default();
    c.protocol = protocol;
    c.host = host.into();
    c.port = port.parse().ok()?;
    c.username = "jan".into();
    c.password = "tajne".into();
    c.passive = true;
    c.accept_invalid_certs = true;
    Some(c)
}

fn wait(mgr: &Arc<JobManager>, id: u64) -> JobSnapshot {
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        let s = mgr.snapshot().into_iter().find(|j| j.id == id).unwrap();
        if s.status.is_finished() {
            return s;
        }
        assert!(Instant::now() < deadline, "job timeout: {s:?}");
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn roundtrip(cfg: FtpConfig) {
    let reg = Arc::new(FtpRegistry::default());
    let mgr = JobManager::new(Arc::new(|_, _| {}), reg.clone());
    let conn = reg.open(cfg).expect("connect");
    let session = conn.session.clone();
    let remote_dir = format!("/e2e-{}", uuid::Uuid::new_v4().simple());
    let s = reg.get(&session).unwrap();
    s.run(|c, _| c.mkdir(&remote_dir).map_err(AppError::from)).unwrap();

    // Upload a small tree with a Czech file name.
    let local = std::env::temp_dir().join(format!("dualion-ftp-{}", uuid::Uuid::new_v4()));
    fs::create_dir_all(local.join("data/vnořená")).unwrap();
    fs::write(local.join("data/příliš žluťoučký.txt"), "kůň").unwrap();
    fs::write(local.join("data/vnořená/big.bin"), vec![42u8; 3 * 1024 * 1024]).unwrap();
    let up = mgr
        .enqueue(JobRequest {
            op: JobOp::Copy,
            sources: vec![Location::local(path_str(&local.join("data")))],
            dest: Some(Location::Ftp { session: session.clone(), path: remote_dir.clone() }),
            new_name: None,
            options: JobOptions { verify: true, ..Default::default() },
            queue: None,
        })
        .unwrap();
    let r = wait(&mgr, up);
    assert_eq!(r.status, JobStatus::Done, "{:?}", r.error);

    let listing = s.list(&format!("{remote_dir}/data")).unwrap();
    let names: Vec<_> = listing.entries.iter().map(|e| e.name.clone()).collect();
    assert!(names.contains(&"příliš žluťoučký.txt".to_string()), "{names:?}");
    assert!(names.contains(&"vnořená".to_string()));

    // Rename on the server (move within the same session).
    let mv = mgr
        .enqueue(JobRequest {
            op: JobOp::Move,
            sources: vec![Location::Ftp { session: session.clone(), path: format!("{remote_dir}/data/příliš žluťoučký.txt") }],
            dest: Some(Location::Ftp { session: session.clone(), path: format!("{remote_dir}/data") }),
            new_name: Some("kun.txt".into()),
            options: JobOptions::default(),
            queue: None,
        })
        .unwrap();
    assert_eq!(wait(&mgr, mv).status, JobStatus::Done);

    // Download it back.
    let back = local.join("back");
    fs::create_dir_all(&back).unwrap();
    let down = mgr
        .enqueue(JobRequest {
            op: JobOp::Copy,
            sources: vec![Location::Ftp { session: session.clone(), path: format!("{remote_dir}/data") }],
            dest: Some(Location::local(path_str(&back))),
            new_name: None,
            options: JobOptions::default(),
            queue: None,
        })
        .unwrap();
    let r = wait(&mgr, down);
    assert_eq!(r.status, JobStatus::Done, "{:?}", r.error);
    assert_eq!(fs::read_to_string(back.join("data/kun.txt")).unwrap(), "kůň");
    assert_eq!(fs::metadata(back.join("data/vnořená/big.bin")).unwrap().len(), 3 * 1024 * 1024);

    // Delete the remote tree.
    let del = mgr
        .enqueue(JobRequest {
            op: JobOp::Delete,
            sources: vec![Location::Ftp { session: session.clone(), path: remote_dir.clone() }],
            dest: None,
            new_name: None,
            options: JobOptions { to_trash: false, ..Default::default() },
            queue: None,
        })
        .unwrap();
    assert_eq!(wait(&mgr, del).status, JobStatus::Done);
    let root = s.list("/").unwrap();
    assert!(!root.entries.iter().any(|e| format!("/{}", e.name) == remote_dir));

    reg.close(&session);
    fs::remove_dir_all(&local).unwrap();
}

#[test]
#[ignore]
fn ftp_e2e_plain() {
    let Some(c) = cfg("DUALION_FTP", FtpProtocol::Ftp) else { return };
    roundtrip(c);
}

#[test]
#[ignore]
fn ftp_e2e_explicit_tls() {
    let Some(c) = cfg("DUALION_FTPS", FtpProtocol::FtpsExplicit) else { return };
    roundtrip(c);
}
