//! Remote files opened in an editor (or any application) are downloaded to
//! a temporary file; this watches those copies and uploads them back to the
//! server whenever the application saves them.

use crate::error::{AppError, AppResult};
use crate::remote::Registry;
use parking_lot::Mutex;
use serde::Serialize;
use std::collections::HashMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock};
use std::time::{Duration, Instant, SystemTime};
use tauri::{AppHandle, Emitter};

struct Watch {
    session: String,
    remote: String,
    /// Modification time and size last seen (after download or upload).
    seen: Option<(SystemTime, u64)>,
    /// A change seen but not uploaded yet: waits until the file stops changing.
    pending: Option<((SystemTime, u64), Instant)>,
    /// Since when the file is missing (editors save by replacing it).
    missing: Option<Instant>,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct Uploaded {
    session: String,
    name: String,
    error: Option<String>,
}

static WATCHES: LazyLock<Mutex<HashMap<PathBuf, Watch>>> = LazyLock::new(|| Mutex::new(HashMap::new()));
static POLLER: Mutex<bool> = Mutex::new(false);

fn state(p: &Path) -> Option<(SystemTime, u64)> {
    let md = std::fs::metadata(p).ok()?;
    Some((md.modified().ok()?, md.len()))
}

/// Stops watching `local` (before it is downloaded again); whether it was watched.
pub fn forget(local: &Path) -> bool {
    WATCHES.lock().remove(local).is_some()
}

/// Starts watching the freshly downloaded `local` copy of `remote`.
pub fn watch(app: &AppHandle, registry: Arc<Registry>, session: &str, remote: &str, local: &Path) {
    WATCHES.lock().insert(local.to_path_buf(), Watch { session: session.into(), remote: remote.into(), seen: state(local), pending: None, missing: None });
    let mut running = POLLER.lock();
    if !*running {
        *running = true;
        let app = app.clone();
        std::thread::spawn(move || poll(app, registry));
    }
}

fn poll(app: AppHandle, registry: Arc<Registry>) {
    loop {
        std::thread::sleep(Duration::from_millis(700));
        let mut due: Vec<(PathBuf, String, String, (SystemTime, u64))> = Vec::new();
        {
            let mut watches = WATCHES.lock();
            watches.retain(|local, w| {
                let Some(now) = state(local) else {
                    // Gone for a minute: the temporary copy was deleted.
                    return w.missing.get_or_insert_with(Instant::now).elapsed() < Duration::from_secs(60);
                };
                w.missing = None;
                if Some(now) == w.seen {
                    w.pending = None;
                    return true;
                }
                match w.pending {
                    // Unchanged for a moment: the save is complete.
                    Some((p, since)) if p == now && since.elapsed() >= Duration::from_millis(600) => {
                        w.seen = Some(now);
                        w.pending = None;
                        due.push((local.clone(), w.session.clone(), w.remote.clone(), now));
                    }
                    Some((p, _)) if p == now => {}
                    _ => w.pending = Some((now, Instant::now())),
                }
                true
            });
        }
        for (local, session, remote, _) in due {
            let name = crate::remote::file_name(&remote).to_string();
            let error = upload(&registry, &session, &remote, &local).err().map(|e| e.message);
            let _ = app.emit("remote-edit-uploaded", Uploaded { session, name, error });
        }
    }
}

fn upload(registry: &Registry, session: &str, remote: &str, local: &Path) -> AppResult<()> {
    let s = registry.get(session)?;
    let fs = s.transfer_fs()?;
    let mut src = std::fs::File::open(local).map_err(|e| AppError::io(e, local))?;
    let mut w = fs.writer(remote)?;
    std::io::copy(&mut src, &mut w).map_err(|e| AppError::io(e, local))?;
    w.flush().map_err(|e| AppError::io(e, local))?;
    w.finish()?;
    if s.cfg.protocol.is_ftp() {
        fs.close();
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::remote::{Protocol, RemoteConfig};

    /// `DUALION_FTP=127.0.0.1:2121 cargo test remote_edit -- --ignored`
    #[test]
    #[ignore]
    fn uploads_edited_copy() {
        let Ok(addr) = std::env::var("DUALION_FTP") else { return };
        let (host, port) = addr.rsplit_once(':').unwrap();
        let cfg = RemoteConfig { protocol: Protocol::Ftp, host: host.into(), port: port.parse().unwrap(), username: "jan".into(), password: "tajne".into(), passive: true, ..Default::default() };
        let reg = Registry::default();
        let session = reg.open(cfg).unwrap().session;
        let remote = format!("/edit-{}.txt", uuid::Uuid::new_v4().simple());
        let local = std::env::temp_dir().join(format!("dualion-edit-{}.txt", uuid::Uuid::new_v4()));
        std::fs::write(&local, "upraveno ěšč").unwrap();
        upload(&reg, &session, &remote, &local).unwrap();
        // The session's own connection still works and sees the new content.
        let fs = reg.get(&session).unwrap().fs();
        let mut r = fs.reader(&remote).unwrap();
        let mut got = String::new();
        std::io::Read::read_to_string(&mut r, &mut got).unwrap();
        r.finish().unwrap();
        assert_eq!(got, "upraveno ěšč");
        fs.remove_file(&remote).unwrap();
        reg.close(&session);
        std::fs::remove_file(&local).unwrap();
    }
}
