//! SFTP over SSH (russh, pure Rust). Sign-in tries, in order: the given key
//! file, the SSH agent, the default keys in ~/.ssh, the password. Host keys
//! are checked against ~/.ssh/known_hosts and Dualion's own list; an unknown
//! key asks the user (its fingerprint comes back in the error), a changed key
//! is refused.

use super::{block_on, entry, PlainReader, RemoteConfig, RemoteFs};
use crate::error::{AppError, AppResult, ErrorCode};
use crate::jobs::vfs::{Meta, ReadStream, WriteStream};
use crate::location::Entry;
use parking_lot::Mutex;
use russh::keys::{HashAlg, PrivateKeyWithHashAlg, PublicKey, PublicKeyOrCertificate};
use russh_sftp::client::SftpSession;
use russh_sftp::protocol::{FileAttributes, StatusCode};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

/// Dualion's own known_hosts (keys the user accepted).
fn known_hosts_file() -> PathBuf {
    crate::platform::data_dir().join("known_hosts")
}

#[derive(Default)]
struct KeyCheck {
    /// Fingerprint of an unknown key (to ask the user).
    unknown: Option<String>,
    changed: bool,
}

struct Client {
    host: String,
    port: u16,
    trust: String,
    check: Arc<Mutex<KeyCheck>>,
}

impl russh::client::Handler for Client {
    type Error = russh::Error;

    async fn check_server_key(&mut self, key: &PublicKeyOrCertificate) -> Result<bool, Self::Error> {
        let key: PublicKey = match key {
            PublicKeyOrCertificate::PublicKey { key, .. } => key.clone(),
            PublicKeyOrCertificate::Certificate(c) => PublicKey::new(c.public_key().clone(), ""),
        };
        let fp = key.fingerprint(HashAlg::Sha256).to_string();
        let mut files = vec![known_hosts_file()];
        if let Some(home) = dirs::home_dir() {
            files.insert(0, home.join(".ssh").join("known_hosts"));
        }
        for f in &files {
            match russh::keys::check_known_hosts_path(&self.host, self.port, &key, f) {
                Ok(true) => return Ok(true),
                Err(russh::keys::Error::KeyChanged { .. }) => {
                    self.check.lock().changed = true;
                    return Ok(false);
                }
                _ => {}
            }
        }
        if !self.trust.is_empty() && self.trust == fp {
            let path = known_hosts_file();
            if let Some(dir) = path.parent() {
                let _ = std::fs::create_dir_all(dir);
            }
            let _ = russh::keys::known_hosts::learn_known_hosts_path(&self.host, self.port, &key, &path);
            return Ok(true);
        }
        self.check.lock().unknown = Some(fp);
        Ok(false)
    }
}

type Handle = russh::client::Handle<Client>;

fn ssh_err(e: russh::Error) -> AppError {
    AppError::new(ErrorCode::FtpConnect, e.to_string())
}

fn sftp_err(e: russh_sftp::client::error::Error) -> AppError {
    use russh_sftp::client::error::Error as E;
    match &e {
        E::Status(s) => {
            let code = match s.status_code {
                StatusCode::NoSuchFile => ErrorCode::NotFound,
                StatusCode::PermissionDenied => ErrorCode::PermissionDenied,
                StatusCode::OpUnsupported => ErrorCode::Unsupported,
                _ => ErrorCode::Io,
            };
            AppError::new(code, if s.error_message.is_empty() { format!("{:?}", s.status_code) } else { s.error_message.clone() })
        }
        E::Timeout => AppError::new(ErrorCode::FtpConnect, "The server stopped responding"),
        _ => AppError::new(ErrorCode::Io, e.to_string()),
    }
}

/// Keys tried when no key file is given.
fn default_keys() -> Vec<PathBuf> {
    let Some(home) = dirs::home_dir() else { return Vec::new() };
    ["id_ed25519", "id_ecdsa", "id_rsa"].iter().map(|n| home.join(".ssh").join(n)).filter(|p| p.is_file()).collect()
}

async fn authenticate(h: &mut Handle, cfg: &RemoteConfig) -> AppResult<()> {
    let user = cfg.username.trim().to_string();
    let user = if user.is_empty() { whoami() } else { user };
    let rsa_hash = h.best_supported_rsa_hash().await.ok().flatten().flatten();
    let key_path = cfg.key_path.trim();
    let mut key_error: Option<String> = None;

    // 1. Explicit key file (the password field holds its passphrase).
    if !key_path.is_empty() {
        let pass = (!cfg.password.is_empty()).then_some(cfg.password.as_str());
        match russh::keys::load_secret_key(expand(key_path), pass) {
            Ok(k) => {
                let r = h.authenticate_publickey(&user, PrivateKeyWithHashAlg::new(Arc::new(k), rsa_hash)).await.map_err(ssh_err)?;
                if r.success() {
                    return Ok(());
                }
            }
            Err(e) => key_error = Some(e.to_string()),
        }
    }
    // 2. SSH agent.
    if key_path.is_empty() && agent_auth(h, &user, rsa_hash).await {
        return Ok(());
    }
    // 3. Default keys without a passphrase.
    if key_path.is_empty() {
        for p in default_keys() {
            if let Ok(k) = russh::keys::load_secret_key(&p, None) {
                if h.authenticate_publickey(&user, PrivateKeyWithHashAlg::new(Arc::new(k), rsa_hash)).await.map(|r| r.success()).unwrap_or(false) {
                    return Ok(());
                }
            }
        }
    }
    // 4. Password.
    if !cfg.password.is_empty() && key_path.is_empty() && h.authenticate_password(&user, cfg.password.as_str()).await.map_err(ssh_err)?.success() {
        return Ok(());
    }
    Err(AppError::new(ErrorCode::FtpAuth, key_error.unwrap_or_else(|| "Login failed".into())))
}

#[cfg(unix)]
async fn agent_auth(h: &mut Handle, user: &str, rsa_hash: Option<HashAlg>) -> bool {
    let Ok(mut agent) = russh::keys::agent::client::AgentClient::connect_env().await else { return false };
    let Ok(ids) = agent.request_identities().await else { return false };
    for id in ids {
        let key = id.public_key().into_owned();
        if h.authenticate_publickey_with(user, key, rsa_hash, &mut agent).await.map(|r| r.success()).unwrap_or(false) {
            return true;
        }
    }
    false
}

#[cfg(windows)]
async fn agent_auth(h: &mut Handle, user: &str, rsa_hash: Option<HashAlg>) -> bool {
    // Windows OpenSSH agent, then Pageant.
    if let Ok(mut agent) = russh::keys::agent::client::AgentClient::connect_named_pipe(r"\\.\pipe\openssh-ssh-agent").await {
        if let Ok(ids) = agent.request_identities().await {
            for id in ids {
                let key = id.public_key().into_owned();
                if h.authenticate_publickey_with(user, key, rsa_hash, &mut agent).await.map(|r| r.success()).unwrap_or(false) {
                    return true;
                }
            }
        }
    }
    if let Ok(mut agent) = russh::keys::agent::client::AgentClient::connect_pageant().await {
        if let Ok(ids) = agent.request_identities().await {
            for id in ids {
                let key = id.public_key().into_owned();
                if h.authenticate_publickey_with(user, key, rsa_hash, &mut agent).await.map(|r| r.success()).unwrap_or(false) {
                    return true;
                }
            }
        }
    }
    false
}

fn whoami() -> String {
    std::env::var("USER").or_else(|_| std::env::var("USERNAME")).unwrap_or_else(|_| "root".into())
}

fn expand(p: &str) -> PathBuf {
    match (p.strip_prefix("~/").or_else(|| p.strip_prefix("~\\")), dirs::home_dir()) {
        (Some(rest), Some(home)) => home.join(rest),
        _ => PathBuf::from(p),
    }
}

pub struct SftpRemote {
    sftp: SftpSession,
    ssh: Mutex<Option<Handle>>,
}

impl SftpRemote {
    /// Connects and signs in; returns the file system and the home folder.
    pub fn connect(cfg: &RemoteConfig) -> AppResult<(Self, String)> {
        block_on(async {
            let host = cfg.host.trim().to_string();
            let port = cfg.effective_port();
            let check = Arc::new(Mutex::new(KeyCheck::default()));
            let client = Client { host: host.clone(), port, trust: cfg.trust_key.clone(), check: check.clone() };
            let config = russh::client::Config {
                inactivity_timeout: None,
                keepalive_interval: Some(Duration::from_secs(30)),
                keepalive_max: 4,
                ..Default::default()
            };
            let connecting = russh::client::connect(Arc::new(config), (host.as_str(), port), client);
            let mut h = match tokio::time::timeout(Duration::from_secs(20), connecting).await {
                Err(_) => return Err(AppError::new(ErrorCode::FtpConnect, "Connection timed out")),
                Ok(Ok(h)) => h,
                Ok(Err(e)) => {
                    let c = check.lock();
                    if c.changed {
                        return Err(AppError::new(ErrorCode::HostKeyChanged, "The server's host key has changed – possible attack, connection refused"));
                    }
                    if let Some(fp) = &c.unknown {
                        return Err(AppError::new(ErrorCode::HostKeyUnknown, fp.clone()).with_path(format!("{host}:{port}")));
                    }
                    return Err(ssh_err(e));
                }
            };
            authenticate(&mut h, cfg).await?;
            let channel = h.channel_open_session().await.map_err(ssh_err)?;
            channel.request_subsystem(true, "sftp").await.map_err(ssh_err)?;
            let sftp = SftpSession::new(channel.into_stream()).await.map_err(sftp_err)?;
            sftp.set_timeout(60);
            let home = sftp.canonicalize(".").await.unwrap_or_else(|_| "/".into());
            Ok((Self { sftp, ssh: Mutex::new(Some(h)) }, home))
        })
    }

    fn entry_of(name: String, a: &FileAttributes) -> Entry {
        let link = a.is_symlink();
        entry(name, a.is_dir(), a.size.unwrap_or(0), a.mtime.map(|t| t as i64 * 1000).unwrap_or(0), a.permissions.unwrap_or(0), link)
    }
}

impl RemoteFs for SftpRemote {
    fn list(&self, path: &str) -> AppResult<Vec<Entry>> {
        block_on(async {
            let dir = self.sftp.read_dir(path).await.map_err(sftp_err)?;
            let mut out = Vec::new();
            for e in dir {
                let name = e.file_name();
                if name == "." || name == ".." {
                    continue;
                }
                let mut a = e.metadata();
                let mut item = Self::entry_of(name.clone(), &a);
                if a.is_symlink() {
                    // Show where the link points (folder links can be entered).
                    if let Ok(t) = self.sftp.metadata(super::join(path, &name)).await {
                        a = t;
                        item = Self::entry_of(name, &a);
                        item.flags |= crate::location::flags::SYMLINK;
                    }
                }
                out.push(item);
            }
            Ok(out)
        })
    }
    fn stat(&self, path: &str) -> AppResult<Option<Meta>> {
        block_on(async {
            match self.sftp.symlink_metadata(path).await {
                Ok(a) => {
                    let link = a.is_symlink();
                    let t = if link { self.sftp.metadata(path).await.unwrap_or(a) } else { a };
                    Ok(Some(Meta { is_dir: t.is_dir(), is_link: link, size: t.size.unwrap_or(0), mtime: t.mtime.map(|m| m as i64 * 1000).unwrap_or(0) }))
                }
                Err(e) => {
                    let e = sftp_err(e);
                    if e.code == ErrorCode::NotFound {
                        Ok(None)
                    } else {
                        Err(e)
                    }
                }
            }
        })
    }
    fn mkdir(&self, path: &str) -> AppResult<()> {
        block_on(self.sftp.create_dir(path)).map_err(sftp_err)
    }
    fn remove_file(&self, path: &str) -> AppResult<()> {
        block_on(self.sftp.remove_file(path)).map_err(sftp_err)
    }
    fn remove_dir(&self, path: &str) -> AppResult<()> {
        block_on(self.sftp.remove_dir(path)).map_err(sftp_err)
    }
    fn rename(&self, from: &str, to: &str) -> AppResult<()> {
        block_on(self.sftp.rename(from, to)).map_err(sftp_err)
    }
    fn reader(&self, path: &str) -> AppResult<Box<dyn ReadStream + '_>> {
        let file = block_on(self.sftp.open(path)).map_err(sftp_err)?;
        Ok(Box::new(PlainReader(SyncFile(file))))
    }
    fn writer(&self, path: &str) -> AppResult<Box<dyn WriteStream + '_>> {
        let file = block_on(self.sftp.create(path)).map_err(sftp_err)?;
        Ok(Box::new(SyncFile(file)))
    }
    fn set_mtime(&self, path: &str, mtime: i64) {
        let secs = (mtime / 1000).clamp(0, u32::MAX as i64) as u32;
        let attrs = FileAttributes { atime: Some(secs), mtime: Some(secs), ..FileAttributes::empty() };
        let _ = block_on(self.sftp.set_metadata(path, attrs));
    }
    fn close(&self) {
        let _ = block_on(self.sftp.close());
        if let Some(h) = self.ssh.lock().take() {
            let _ = block_on(h.disconnect(russh::Disconnect::ByApplication, "", "en"));
        }
    }
}

/// Blocking view of an SFTP file.
struct SyncFile(russh_sftp::client::fs::File);

impl std::io::Read for SyncFile {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        block_on(self.0.read(buf))
    }
}

impl std::io::Write for SyncFile {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        block_on(self.0.write(buf))
    }
    fn flush(&mut self) -> std::io::Result<()> {
        block_on(self.0.flush())
    }
}

impl WriteStream for SyncFile {
    fn finish(mut self: Box<Self>) -> AppResult<()> {
        block_on(self.0.shutdown()).map_err(|e| AppError::new(ErrorCode::Io, e.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Connects once and prints the outcome (host key checks):
    /// `DUALION_SFTP=127.0.0.1:2222 DUALION_SFTP_KEY=… cargo test sftp_connect_live -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn sftp_connect_live() {
        let var = |k: &str| std::env::var(k).unwrap_or_default();
        let addr = var("DUALION_SFTP");
        let (host, port) = addr.rsplit_once(':').unwrap();
        let cfg = RemoteConfig { protocol: super::super::Protocol::Sftp, host: host.into(), port: port.parse().unwrap(), username: var("DUALION_SFTP_USER"), key_path: var("DUALION_SFTP_KEY"), ..Default::default() };
        match SftpRemote::connect(&cfg) {
            Ok((_, home)) => println!("connected, home {home}"),
            Err(e) => println!("error {:?}: {}", e.code, e.message),
        }
    }
}
