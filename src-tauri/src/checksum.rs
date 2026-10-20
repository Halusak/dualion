//! Checksums (MD5, SHA-1, SHA-256, SHA-512, CRC32, XXH3) with progress.

use crate::error::{AppError, AppResult};
use serde::Serialize;
use sha2::Digest;
use std::io::Read;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};

enum Hasher {
    Md5(md5::Md5),
    Sha1(sha1::Sha1),
    Sha256(sha2::Sha256),
    Sha512(sha2::Sha512),
    Crc32(crc32fast::Hasher),
    Xxh3(Box<xxhash_rust::xxh3::Xxh3>),
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

impl Hasher {
    fn new(algo: &str) -> AppResult<Self> {
        Ok(match algo {
            "md5" => Hasher::Md5(md5::Md5::new()),
            "sha1" => Hasher::Sha1(sha1::Sha1::new()),
            "sha256" => Hasher::Sha256(sha2::Sha256::new()),
            "sha512" => Hasher::Sha512(sha2::Sha512::new()),
            "crc32" => Hasher::Crc32(crc32fast::Hasher::new()),
            "xxh3" => Hasher::Xxh3(Box::new(xxhash_rust::xxh3::Xxh3::new())),
            other => return Err(AppError::invalid(format!("Unknown algorithm {other}"))),
        })
    }

    fn update(&mut self, data: &[u8]) {
        match self {
            Hasher::Md5(h) => h.update(data),
            Hasher::Sha1(h) => h.update(data),
            Hasher::Sha256(h) => h.update(data),
            Hasher::Sha512(h) => h.update(data),
            Hasher::Crc32(h) => h.update(data),
            Hasher::Xxh3(h) => h.update(data),
        }
    }

    fn finish(self) -> String {
        match self {
            Hasher::Md5(h) => hex(&h.finalize()),
            Hasher::Sha1(h) => hex(&h.finalize()),
            Hasher::Sha256(h) => hex(&h.finalize()),
            Hasher::Sha512(h) => hex(&h.finalize()),
            Hasher::Crc32(h) => format!("{:08x}", h.finalize()),
            Hasher::Xxh3(h) => format!("{:016x}", h.digest()),
        }
    }
}

pub fn hash_file(path: &Path, algo: &str, cancel: &AtomicBool, mut progress: impl FnMut(u64)) -> AppResult<String> {
    let mut h = Hasher::new(algo)?;
    let mut f = std::fs::File::open(path).map_err(|e| AppError::io(e, path))?;
    let mut buf = vec![0u8; 1024 * 1024];
    loop {
        if cancel.load(Ordering::Relaxed) {
            return Err(AppError::cancelled());
        }
        let n = match f.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => n,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(AppError::io(e, path)),
        };
        h.update(&buf[..n]);
        progress(n as u64);
    }
    Ok(h.finish())
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum ChecksumEvent {
    #[serde(rename_all = "camelCase")]
    Progress { done_bytes: u64, total_bytes: u64, current: String },
    #[serde(rename_all = "camelCase")]
    Result { path: String, hash: Option<String>, error: Option<AppError> },
    Done { cancelled: bool },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_digests() {
        let dir = std::env::temp_dir().join(format!("dualion-sum-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("abc.txt");
        std::fs::write(&p, b"abc").unwrap();
        let c = AtomicBool::new(false);
        assert_eq!(hash_file(&p, "md5", &c, |_| {}).unwrap(), "900150983cd24fb0d6963f7d28e17f72");
        assert_eq!(hash_file(&p, "sha1", &c, |_| {}).unwrap(), "a9993e364706816aba3e25717850c26c9cd0d89d");
        assert_eq!(
            hash_file(&p, "sha256", &c, |_| {}).unwrap(),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert_eq!(hash_file(&p, "crc32", &c, |_| {}).unwrap(), "352441c2");
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
