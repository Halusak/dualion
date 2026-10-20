//! Encrypted storage of FTP sites and their passwords.
//!
//! The vault file holds the site list encrypted with XChaCha20-Poly1305.
//! The 256-bit key either lives in the OS credential store (macOS Keychain,
//! Windows Credential Manager) or is derived from a master password with
//! Argon2id. Passwords never leave the backend: the UI only sees summaries.

use crate::error::{AppError, AppResult, ErrorCode};
use crate::ftp::{FtpConfig, FtpProtocol};
use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine;
use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use chacha20poly1305::{XChaCha20Poly1305, XNonce};
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;
use zeroize::{Zeroize, Zeroizing};

const KEYRING_SERVICE: &str = "Dualion";
const KEYRING_ACCOUNT: &str = "site-vault-key";
const AAD: &[u8] = b"dualion-vault-v1";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
enum Protection {
    Keychain,
    #[serde(rename_all = "camelCase")]
    Master { salt: String, m_cost: u32, t_cost: u32, p_cost: u32 },
}

#[derive(Serialize, Deserialize)]
struct VaultFile {
    version: u32,
    protection: Protection,
    nonce: String,
    data: String,
}

#[derive(Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct Site {
    pub id: String,
    pub name: String,
    pub group: String,
    pub protocol: FtpProtocol,
    pub host: String,
    pub port: u16,
    pub username: String,
    pub password: String,
    pub passive: bool,
    pub encoding: String,
    pub accept_invalid_certs: bool,
    pub remote_path: String,
    pub local_path: String,
    pub notes: String,
    pub last_used: i64,
}

impl Site {
    pub fn to_config(&self) -> FtpConfig {
        FtpConfig {
            protocol: self.protocol,
            host: self.host.clone(),
            port: self.port,
            username: self.username.clone(),
            password: self.password.clone(),
            passive: self.passive,
            encoding: self.encoding.clone(),
            accept_invalid_certs: self.accept_invalid_certs,
            remote_path: self.remote_path.clone(),
        }
    }
}

/// What the UI gets to see.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SiteSummary {
    pub id: String,
    pub name: String,
    pub group: String,
    pub protocol: FtpProtocol,
    pub host: String,
    pub port: u16,
    pub username: String,
    pub has_password: bool,
    pub passive: bool,
    pub encoding: String,
    pub accept_invalid_certs: bool,
    pub remote_path: String,
    pub local_path: String,
    pub notes: String,
    pub last_used: i64,
}

impl From<&Site> for SiteSummary {
    fn from(s: &Site) -> Self {
        Self {
            id: s.id.clone(),
            name: s.name.clone(),
            group: s.group.clone(),
            protocol: s.protocol,
            host: s.host.clone(),
            port: s.port,
            username: s.username.clone(),
            has_password: !s.password.is_empty(),
            passive: s.passive,
            encoding: s.encoding.clone(),
            accept_invalid_certs: s.accept_invalid_certs,
            remote_path: s.remote_path.clone(),
            local_path: s.local_path.clone(),
            notes: s.notes.clone(),
            last_used: s.last_used,
        }
    }
}

/// Site as edited in the UI. `password: None` keeps the stored one.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SiteInput {
    pub id: Option<String>,
    pub name: String,
    #[serde(default)]
    pub group: String,
    #[serde(default)]
    pub protocol: FtpProtocol,
    pub host: String,
    #[serde(default)]
    pub port: u16,
    #[serde(default)]
    pub username: String,
    pub password: Option<String>,
    #[serde(default = "default_true")]
    pub passive: bool,
    #[serde(default)]
    pub encoding: String,
    #[serde(default)]
    pub accept_invalid_certs: bool,
    #[serde(default)]
    pub remote_path: String,
    #[serde(default)]
    pub local_path: String,
    #[serde(default)]
    pub notes: String,
}

fn default_true() -> bool {
    true
}

#[derive(Serialize, Deserialize, Default)]
struct VaultData {
    sites: Vec<Site>,
}

impl Drop for VaultData {
    fn drop(&mut self) {
        for s in &mut self.sites {
            s.password.zeroize();
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VaultStatus {
    pub exists: bool,
    /// "keychain" | "master" | "none"
    pub protection: &'static str,
    pub unlocked: bool,
    pub keychain_available: bool,
}

struct State {
    key: Option<Zeroizing<[u8; 32]>>,
    protection: Option<Protection>,
    data: Option<VaultData>,
}

pub struct Vault {
    path: PathBuf,
    state: Mutex<State>,
}

fn random<const N: usize>() -> AppResult<[u8; N]> {
    let mut b = [0u8; N];
    getrandom::fill(&mut b).map_err(|e| AppError::other(format!("RNG failure: {e}")))?;
    Ok(b)
}

fn keychain_entry() -> AppResult<keyring::Entry> {
    keyring::Entry::new(KEYRING_SERVICE, KEYRING_ACCOUNT).map_err(|e| AppError::new(ErrorCode::Keychain, e.to_string()))
}

fn keychain_get() -> AppResult<Option<Zeroizing<[u8; 32]>>> {
    match keychain_entry()?.get_secret() {
        Ok(mut secret) => {
            if secret.len() != 32 {
                secret.zeroize();
                return Err(AppError::new(ErrorCode::Keychain, "Corrupted key in the keychain"));
            }
            let mut k = Zeroizing::new([0u8; 32]);
            k.copy_from_slice(&secret);
            secret.zeroize();
            Ok(Some(k))
        }
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(e) => Err(AppError::new(ErrorCode::Keychain, e.to_string())),
    }
}

fn keychain_set(key: &[u8; 32]) -> AppResult<()> {
    keychain_entry()?.set_secret(key).map_err(|e| AppError::new(ErrorCode::Keychain, e.to_string()))
}

fn derive(password: &str, salt: &[u8], m: u32, t: u32, p: u32) -> AppResult<Zeroizing<[u8; 32]>> {
    let params = argon2::Params::new(m, t, p, Some(32)).map_err(|e| AppError::other(e.to_string()))?;
    let a = argon2::Argon2::new(argon2::Algorithm::Argon2id, argon2::Version::V0x13, params);
    let mut out = Zeroizing::new([0u8; 32]);
    a.hash_password_into(password.as_bytes(), salt, &mut out[..]).map_err(|e| AppError::other(e.to_string()))?;
    Ok(out)
}

fn encrypt(key: &[u8; 32], plain: &[u8]) -> AppResult<(String, String)> {
    let cipher = XChaCha20Poly1305::new(key.into());
    let nonce_bytes = random::<24>()?;
    let nonce = XNonce::from(nonce_bytes);
    let ct = cipher
        .encrypt(&nonce, Payload { msg: plain, aad: AAD })
        .map_err(|_| AppError::other("Encryption failed"))?;
    Ok((B64.encode(nonce_bytes), B64.encode(ct)))
}

fn decrypt(key: &[u8; 32], nonce_b64: &str, data_b64: &str) -> AppResult<Zeroizing<Vec<u8>>> {
    let nonce_bytes: [u8; 24] = B64
        .decode(nonce_b64)
        .ok()
        .and_then(|v| v.try_into().ok())
        .ok_or_else(|| AppError::other("Corrupted vault"))?;
    let ct = B64.decode(data_b64).map_err(|_| AppError::other("Corrupted vault"))?;
    let cipher = XChaCha20Poly1305::new(key.into());
    cipher
        .decrypt(&XNonce::from(nonce_bytes), Payload { msg: &ct, aad: AAD })
        .map(Zeroizing::new)
        .map_err(|_| AppError::new(ErrorCode::VaultWrongPassword, "Wrong password"))
}

impl Vault {
    pub fn new(path: PathBuf) -> Self {
        let protection = fs::read(&path)
            .ok()
            .and_then(|b| serde_json::from_slice::<VaultFile>(&b).ok())
            .map(|f| f.protection);
        Self { path, state: Mutex::new(State { key: None, protection, data: None }) }
    }

    pub fn status(&self) -> VaultStatus {
        let s = self.state.lock();
        VaultStatus {
            exists: s.protection.is_some(),
            protection: match s.protection {
                Some(Protection::Keychain) => "keychain",
                Some(Protection::Master { .. }) => "master",
                None => "none",
            },
            unlocked: s.data.is_some(),
            keychain_available: keyring::Entry::store_status().is_ok(),
        }
    }

    fn read_file(&self) -> AppResult<VaultFile> {
        let bytes = fs::read(&self.path).map_err(|e| AppError::io(e, &self.path))?;
        serde_json::from_slice(&bytes).map_err(|e| AppError::other(format!("Corrupted vault: {e}")))
    }

    fn persist(&self, s: &State) -> AppResult<()> {
        let (Some(key), Some(protection), Some(data)) = (&s.key, &s.protection, &s.data) else {
            return Err(AppError::new(ErrorCode::VaultLocked, "Vault is locked"));
        };
        let plain = Zeroizing::new(serde_json::to_vec(data).map_err(|e| AppError::other(e.to_string()))?);
        let (nonce, ct) = encrypt(key, &plain)?;
        let file = VaultFile { version: 1, protection: protection.clone(), nonce, data: ct };
        if let Some(dir) = self.path.parent() {
            fs::create_dir_all(dir).map_err(|e| AppError::io(e, dir))?;
        }
        let tmp = self.path.with_extension("tmp");
        fs::write(&tmp, serde_json::to_vec_pretty(&file).unwrap_or_default()).map_err(|e| AppError::io(e, &tmp))?;
        fs::rename(&tmp, &self.path).map_err(|e| AppError::io(e, &self.path))
    }

    /// Opens the vault without user interaction when possible (keychain
    /// mode or a brand new vault). Master-password vaults report `VaultLocked`.
    fn ensure_open<'a>(&self, s: &'a mut State) -> AppResult<&'a mut VaultData> {
        if s.data.is_none() {
            match s.protection.clone() {
                None => {
                    let key = Zeroizing::new(random::<32>()?);
                    keychain_set(&key).map_err(|e| {
                        AppError::new(ErrorCode::VaultLocked, format!("Keychain unavailable, set a master password ({})", e.message))
                    })?;
                    s.key = Some(key);
                    s.protection = Some(Protection::Keychain);
                    s.data = Some(VaultData::default());
                    self.persist(s)?;
                }
                Some(Protection::Keychain) => {
                    let key = keychain_get()?
                        .ok_or_else(|| AppError::new(ErrorCode::Keychain, "Vault key missing from the keychain"))?;
                    let file = self.read_file()?;
                    let plain = decrypt(&key, &file.nonce, &file.data)?;
                    let data: VaultData = serde_json::from_slice(&plain).map_err(|e| AppError::other(e.to_string()))?;
                    s.key = Some(key);
                    s.data = Some(data);
                }
                Some(Protection::Master { .. }) => {
                    return Err(AppError::new(ErrorCode::VaultLocked, "Vault is locked"));
                }
            }
        }
        Ok(s.data.as_mut().expect("opened above"))
    }

    pub fn unlock(&self, password: &str) -> AppResult<()> {
        let mut s = self.state.lock();
        let file = self.read_file()?;
        let Protection::Master { salt, m_cost, t_cost, p_cost } = &file.protection else {
            drop(s);
            return self.sites().map(|_| ());
        };
        let salt = B64.decode(salt).map_err(|_| AppError::other("Corrupted vault"))?;
        let key = derive(password, &salt, *m_cost, *t_cost, *p_cost)?;
        let plain = decrypt(&key, &file.nonce, &file.data)?;
        let data: VaultData = serde_json::from_slice(&plain).map_err(|e| AppError::other(e.to_string()))?;
        s.key = Some(key);
        s.protection = Some(file.protection);
        s.data = Some(data);
        Ok(())
    }

    pub fn lock(&self) {
        let mut s = self.state.lock();
        if matches!(s.protection, Some(Protection::Master { .. })) {
            s.key = None;
            s.data = None;
        }
    }

    /// Switches between keychain protection and a master password
    /// (`Some(pw)`); re-encrypts everything with a fresh key.
    pub fn set_master_password(&self, password: Option<&str>) -> AppResult<()> {
        let mut s = self.state.lock();
        if s.protection.is_some() {
            self.ensure_open(&mut s)?;
        } else {
            s.data = Some(VaultData::default());
        }
        match password.filter(|p| !p.is_empty()) {
            Some(pw) => {
                let salt = random::<16>()?;
                let (m, t, p) = (64 * 1024, 3, 1);
                s.key = Some(derive(pw, &salt, m, t, p)?);
                s.protection = Some(Protection::Master { salt: B64.encode(salt), m_cost: m, t_cost: t, p_cost: p });
                let _ = keychain_entry().and_then(|e| e.delete_credential().map_err(|e| AppError::other(e.to_string())));
            }
            None => {
                let key = Zeroizing::new(random::<32>()?);
                keychain_set(&key)?;
                s.key = Some(key);
                s.protection = Some(Protection::Keychain);
            }
        }
        self.persist(&s)
    }

    pub fn sites(&self) -> AppResult<Vec<SiteSummary>> {
        let mut s = self.state.lock();
        // No vault yet: nothing to list, and no reason to touch the keychain.
        if s.protection.is_none() && s.data.is_none() {
            return Ok(Vec::new());
        }
        let data = self.ensure_open(&mut s)?;
        let mut v: Vec<SiteSummary> = data.sites.iter().map(SiteSummary::from).collect();
        v.sort_by(|a, b| a.group.to_lowercase().cmp(&b.group.to_lowercase()).then(a.name.to_lowercase().cmp(&b.name.to_lowercase())));
        Ok(v)
    }

    pub fn site(&self, id: &str) -> AppResult<Site> {
        let mut s = self.state.lock();
        if s.protection.is_none() {
            return Err(AppError::new(ErrorCode::NotFound, "Site not found"));
        }
        let data = self.ensure_open(&mut s)?;
        data.sites.iter().find(|x| x.id == id).cloned().ok_or_else(|| AppError::new(ErrorCode::NotFound, "Site not found"))
    }

    pub fn touch(&self, id: &str) {
        let mut s = self.state.lock();
        if s.protection.is_none() {
            return;
        }
        if let Ok(data) = self.ensure_open(&mut s) {
            if let Some(site) = data.sites.iter_mut().find(|x| x.id == id) {
                site.last_used = crate::util::now_ms();
            }
            let _ = self.persist(&s);
        }
    }

    pub fn save(&self, input: SiteInput) -> AppResult<SiteSummary> {
        if input.host.trim().is_empty() {
            return Err(AppError::invalid("Missing host"));
        }
        let mut s = self.state.lock();
        let data = self.ensure_open(&mut s)?;
        let id = input.id.clone().filter(|i| !i.is_empty()).unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
        let existing_pw = data.sites.iter().find(|x| x.id == id).map(|x| x.password.clone()).unwrap_or_default();
        let site = Site {
            id: id.clone(),
            name: if input.name.trim().is_empty() { input.host.trim().to_string() } else { input.name.trim().to_string() },
            group: input.group.trim().to_string(),
            protocol: input.protocol,
            host: input.host.trim().to_string(),
            port: input.port,
            username: input.username.trim().to_string(),
            password: input.password.clone().unwrap_or(existing_pw),
            passive: input.passive,
            encoding: input.encoding,
            accept_invalid_certs: input.accept_invalid_certs,
            remote_path: input.remote_path,
            local_path: input.local_path,
            notes: input.notes,
            last_used: data.sites.iter().find(|x| x.id == id).map(|x| x.last_used).unwrap_or(0),
        };
        let summary = SiteSummary::from(&site);
        match data.sites.iter_mut().find(|x| x.id == id) {
            Some(slot) => *slot = site,
            None => data.sites.push(site),
        }
        self.persist(&s)?;
        Ok(summary)
    }

    pub fn delete(&self, id: &str) -> AppResult<()> {
        let mut s = self.state.lock();
        if s.protection.is_none() {
            return Ok(());
        }
        let data = self.ensure_open(&mut s)?;
        data.sites.retain(|x| x.id != id);
        self.persist(&s)
    }
}

/// Builds a connection config from an edited (possibly unsaved) site,
/// falling back to the stored password when none was typed.
pub fn config_from_input(vault: &Vault, input: &SiteInput) -> AppResult<FtpConfig> {
    let stored = match (&input.password, &input.id) {
        (None, Some(id)) if !id.is_empty() => vault.site(id).map(|s| s.password.clone()).unwrap_or_default(),
        _ => String::new(),
    };
    Ok(FtpConfig {
        protocol: input.protocol,
        host: input.host.trim().to_string(),
        port: input.port,
        username: input.username.trim().to_string(),
        password: input.password.clone().unwrap_or(stored),
        passive: input.passive,
        encoding: input.encoding.clone(),
        accept_invalid_certs: input.accept_invalid_certs,
        remote_path: input.remote_path.clone(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encrypt_roundtrip_and_tamper() {
        let key = [7u8; 32];
        let (n, c) = encrypt(&key, b"secret sites").unwrap();
        assert_eq!(&decrypt(&key, &n, &c).unwrap()[..], b"secret sites");
        let wrong = [8u8; 32];
        assert_eq!(decrypt(&wrong, &n, &c).unwrap_err().code, ErrorCode::VaultWrongPassword);
        let mut raw = B64.decode(&c).unwrap();
        raw[0] ^= 1;
        assert!(decrypt(&key, &n, &B64.encode(raw)).is_err());
    }

    #[test]
    fn master_password_vault() {
        let dir = std::env::temp_dir().join(format!("dualion-vault-{}", uuid::Uuid::new_v4()));
        let path = dir.join("sites.vault");
        let v = Vault::new(path.clone());
        v.set_master_password(Some("correct horse")).unwrap();
        let saved = v
            .save(SiteInput {
                id: None,
                name: "Test".into(),
                group: String::new(),
                protocol: FtpProtocol::Ftp,
                host: "ftp.example.com".into(),
                port: 21,
                username: "jan".into(),
                password: Some("tajne".into()),
                passive: true,
                encoding: String::new(),
                accept_invalid_certs: false,
                remote_path: String::new(),
                local_path: String::new(),
                notes: String::new(),
            })
            .unwrap();
        assert!(saved.has_password);
        let raw = fs::read_to_string(&path).unwrap();
        assert!(!raw.contains("tajne") && !raw.contains("ftp.example.com"));

        let v2 = Vault::new(path.clone());
        assert_eq!(v2.status().protection, "master");
        assert_eq!(v2.sites().unwrap_err().code, ErrorCode::VaultLocked);
        assert_eq!(v2.unlock("wrong").unwrap_err().code, ErrorCode::VaultWrongPassword);
        v2.unlock("correct horse").unwrap();
        assert_eq!(v2.site(&saved.id).unwrap().password, "tajne");
        fs::remove_dir_all(&dir).unwrap();
    }
}
