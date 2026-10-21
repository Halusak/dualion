//! S3 and compatible storage (AWS, Cloudflare R2, Backblaze B2, MinIO,
//! Wasabi…) with AWS Signature V4. The panel root lists buckets; folders
//! are key prefixes. Big uploads use multipart.

use super::webdav::{agent, status_error};
use super::{entry, PlainReader, RemoteConfig, RemoteFs, SpoolWriter};
use crate::error::{AppError, AppResult, ErrorCode};
use crate::jobs::vfs::{Meta, ReadStream, WriteStream};
use crate::location::Entry;
use hmac::{Hmac, KeyInit, Mac};
use percent_encoding::{utf8_percent_encode, AsciiSet, NON_ALPHANUMERIC};
use sha2::{Digest, Sha256};
use std::io::Read;
use ureq::http;

type HmacSha256 = Hmac<Sha256>;

/// RFC 3986 unreserved characters stay, everything else is encoded.
const UNRESERVED: &AsciiSet = &NON_ALPHANUMERIC.remove(b'-').remove(b'.').remove(b'_').remove(b'~');
const SINGLE_PUT: u64 = 100 << 20;
const PART: u64 = 16 << 20;

pub struct S3 {
    agent: ureq::Agent,
    scheme: &'static str,
    /// Custom endpoint ("minio.lan:9000"), None = AWS.
    endpoint: Option<String>,
    region: String,
    access: String,
    secret: String,
}

fn hmac(key: &[u8], data: &str) -> Vec<u8> {
    let mut m = HmacSha256::new_from_slice(key).expect("any key length");
    m.update(data.as_bytes());
    m.finalize().into_bytes().to_vec()
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

fn sha_hex(data: &[u8]) -> String {
    hex(&Sha256::digest(data))
}

fn enc(s: &str) -> String {
    utf8_percent_encode(s, UNRESERVED).to_string()
}

/// Encodes a key, keeping its slashes.
fn enc_key(key: &str) -> String {
    key.split('/').map(enc).collect::<Vec<_>>().join("/")
}

fn http_err(e: ureq::Error) -> AppError {
    AppError::new(ErrorCode::FtpConnect, e.to_string())
}

/// "/bucket/a/b" → ("bucket", "a/b").
fn split(path: &str) -> (String, String) {
    let p = path.trim_matches('/');
    match p.split_once('/') {
        Some((b, k)) => (b.to_string(), k.to_string()),
        None => (p.to_string(), String::new()),
    }
}

impl S3 {
    pub fn connect(cfg: &RemoteConfig) -> AppResult<Self> {
        let host = cfg.host.trim();
        let (scheme, endpoint) = match host {
            "" => ("https", None),
            h if h.starts_with("http://") => ("http", Some(h["http://".len()..].trim_end_matches('/').to_string())),
            h => ("https", Some(h.strip_prefix("https://").unwrap_or(h).trim_end_matches('/').to_string())),
        };
        let endpoint = endpoint.map(|e| if cfg.port != 0 && !e.contains(':') { format!("{e}:{}", cfg.port) } else { e });
        let region = if cfg.region.trim().is_empty() { "us-east-1".to_string() } else { cfg.region.trim().to_string() };
        if cfg.username.trim().is_empty() || cfg.password.is_empty() {
            return Err(AppError::new(ErrorCode::FtpAuth, "Access key and secret key are required"));
        }
        let s3 = Self { agent: agent(cfg.accept_invalid_certs), scheme, endpoint, region, access: cfg.username.trim().to_string(), secret: cfg.password.to_string() };
        // Check the credentials: list the start bucket, or the buckets.
        let (bucket, _) = split(&cfg.remote_path);
        if bucket.is_empty() {
            s3.buckets()?;
        } else {
            s3.objects(&bucket, "", Some(1))?;
        }
        Ok(s3)
    }

    /// Host and path of a request (path-style for custom endpoints, virtual-hosted for AWS).
    fn target(&self, bucket: &str, key: &str) -> (String, String) {
        let key = enc_key(key);
        match (&self.endpoint, bucket) {
            (Some(ep), "") => (ep.clone(), "/".into()),
            (Some(ep), b) => (ep.clone(), format!("/{}/{key}", enc(b))),
            (None, "") => (format!("s3.{}.amazonaws.com", self.region), "/".into()),
            (None, b) => (format!("{b}.s3.{}.amazonaws.com", self.region), format!("/{key}")),
        }
    }

    /// Signs and sends a request. `query` is sorted here.
    #[allow(clippy::too_many_arguments)]
    fn call(&self, method: &str, bucket: &str, key: &str, query: &[(&str, String)], headers: &[(&str, String)], body: impl ureq::AsSendBody, what: &str) -> AppResult<http::Response<ureq::Body>> {
        let (host, path) = self.target(bucket, key);
        let mut q: Vec<(String, String)> = query.iter().map(|(k, v)| (enc(k), enc(v))).collect();
        q.sort();
        let qs = q.iter().map(|(k, v)| format!("{k}={v}")).collect::<Vec<_>>().join("&");

        let now = crate::util::utc_parts(crate::util::now_ms());
        let amz_date = format!("{:04}{:02}{:02}T{:02}{:02}{:02}Z", now.0, now.1, now.2, now.3, now.4, now.5);
        let date = &amz_date[..8];
        let payload = "UNSIGNED-PAYLOAD";
        let mut hs: Vec<(String, String)> = vec![("host".into(), host.clone()), ("x-amz-content-sha256".into(), payload.into()), ("x-amz-date".into(), amz_date.clone())];
        for (k, v) in headers {
            hs.push((k.to_ascii_lowercase(), v.trim().to_string()));
        }
        hs.sort();
        let canonical_headers: String = hs.iter().map(|(k, v)| format!("{k}:{v}\n")).collect();
        let signed: String = hs.iter().map(|(k, _)| k.as_str()).collect::<Vec<_>>().join(";");
        let creq = format!("{method}\n{path}\n{qs}\n{canonical_headers}\n{signed}\n{payload}");
        let scope = format!("{date}/{}/s3/aws4_request", self.region);
        let to_sign = format!("AWS4-HMAC-SHA256\n{amz_date}\n{scope}\n{}", sha_hex(creq.as_bytes()));
        let k = hmac(format!("AWS4{}", self.secret).as_bytes(), date);
        let k = hmac(&k, &self.region);
        let k = hmac(&k, "s3");
        let k = hmac(&k, "aws4_request");
        let signature = hex(&hmac(&k, &to_sign));
        let auth = format!("AWS4-HMAC-SHA256 Credential={}/{scope}, SignedHeaders={signed}, Signature={signature}", self.access);

        let url = if qs.is_empty() { format!("{}://{host}{path}", self.scheme) } else { format!("{}://{host}{path}?{qs}", self.scheme) };
        let mut b = http::Request::builder().method(method).uri(url).header("Authorization", auth);
        for (k, v) in &hs {
            if k != "host" {
                b = b.header(k.as_str(), v.as_str());
            }
        }
        let req = b.body(body).map_err(|e| AppError::invalid(e.to_string()))?;
        let mut resp = self.agent.run(req).map_err(http_err)?;
        let s = resp.status().as_u16();
        if (200..300).contains(&s) {
            return Ok(resp);
        }
        // S3 explains errors in an XML body (<Code>NoSuchKey</Code>).
        let text = resp.body_mut().with_config().limit(1 << 20).read_to_string().unwrap_or_default();
        let code = tag(&text, "Code").unwrap_or_default();
        Err(match code.as_str() {
            "NoSuchKey" | "NoSuchBucket" => AppError::new(ErrorCode::NotFound, code),
            "InvalidAccessKeyId" | "SignatureDoesNotMatch" | "AccessDenied" if s == 403 => AppError::new(ErrorCode::FtpAuth, code),
            "BucketAlreadyOwnedByYou" | "BucketAlreadyExists" => AppError::new(ErrorCode::AlreadyExists, code),
            "BucketNotEmpty" => AppError::new(ErrorCode::DirectoryNotEmpty, code),
            "" => status_error(s, what),
            _ => AppError::new(status_error(s, what).code, format!("{what}: {code}")),
        })
    }

    fn read_text(resp: http::Response<ureq::Body>) -> AppResult<String> {
        resp.into_body().into_with_config().limit(256 << 20).read_to_string().map_err(http_err)
    }

    fn buckets(&self) -> AppResult<Vec<Entry>> {
        let xml = Self::read_text(self.call("GET", "", "", &[], &[], (), "ListBuckets")?)?;
        Ok(blocks(&xml, "Bucket")
            .iter()
            .filter_map(|b| {
                let name = tag(b, "Name")?;
                Some(entry(name, true, 0, tag(b, "CreationDate").and_then(|d| iso_date(&d)).unwrap_or(0), 0, false))
            })
            .collect())
    }

    /// One page of ListObjectsV2: (sub-prefixes, objects (key, size, mtime), next token).
    #[allow(clippy::type_complexity)]
    fn page(&self, bucket: &str, prefix: &str, token: Option<&str>, max: Option<u32>) -> AppResult<(Vec<String>, Vec<(String, u64, i64)>, Option<String>)> {
        let mut q = vec![("list-type", "2".to_string()), ("delimiter", "/".to_string()), ("prefix", prefix.to_string())];
        if let Some(m) = max {
            q.push(("max-keys", m.to_string()));
        }
        if let Some(t) = token {
            q.push(("continuation-token", t.to_string()));
        }
        let xml = Self::read_text(self.call("GET", bucket, "", &q, &[], (), "ListObjects")?)?;
        let prefixes = blocks(&xml, "CommonPrefixes").iter().filter_map(|p| tag(p, "Prefix")).collect();
        let objects = blocks(&xml, "Contents")
            .iter()
            .filter_map(|c| {
                let key = tag(c, "Key")?;
                let size = tag(c, "Size").and_then(|s| s.parse().ok()).unwrap_or(0);
                let mtime = tag(c, "LastModified").and_then(|d| iso_date(&d)).unwrap_or(0);
                Some((key, size, mtime))
            })
            .collect();
        let next = if tag(&xml, "IsTruncated").as_deref() == Some("true") { tag(&xml, "NextContinuationToken") } else { None };
        Ok((prefixes, objects, next))
    }

    /// Objects and sub-prefixes directly under `prefix` ("a/b/" or "").
    fn objects(&self, bucket: &str, prefix: &str, max: Option<u32>) -> AppResult<Vec<Entry>> {
        let mut out = Vec::new();
        let mut token: Option<String> = None;
        loop {
            let (prefixes, objects, next) = self.page(bucket, prefix, token.as_deref(), max)?;
            for full in prefixes {
                let name = full[prefix.len().min(full.len())..].trim_end_matches('/').to_string();
                if !name.is_empty() {
                    out.push(entry(name, true, 0, 0, 0, false));
                }
            }
            for (key, size, mtime) in objects {
                let name = &key[prefix.len().min(key.len())..];
                if name.is_empty() || name.contains('/') {
                    continue; // the folder marker itself
                }
                out.push(entry(name.to_string(), false, size, mtime, 0, false));
            }
            token = next;
            if max.is_some() || token.is_none() {
                break;
            }
        }
        Ok(out)
    }

    fn put_object(&self, bucket: &str, key: &str, mut file: std::fs::File, size: u64) -> AppResult<()> {
        if size <= SINGLE_PUT {
            let h = [("content-length", size.to_string())];
            return self.call("PUT", bucket, key, &[], &h, ureq::SendBody::from_reader(&mut file), "PUT").map(|_| ());
        }
        // Multipart upload.
        let xml = Self::read_text(self.call("POST", bucket, key, &[("uploads", String::new())], &[], (), "CreateMultipartUpload")?)?;
        let upload = tag(&xml, "UploadId").ok_or_else(|| AppError::other("No upload id"))?;
        let result = (|| -> AppResult<String> {
            let mut parts = String::new();
            let mut n = 1u32;
            let mut left = size;
            while left > 0 {
                let len = left.min(PART);
                let mut chunk = (&mut file).take(len);
                let q = [("partNumber", n.to_string()), ("uploadId", upload.clone())];
                let resp = self.call("PUT", bucket, key, &q, &[("content-length", len.to_string())], ureq::SendBody::from_reader(&mut chunk), "UploadPart")?;
                let etag = resp.headers().get("etag").and_then(|v| v.to_str().ok()).unwrap_or("").to_string();
                parts.push_str(&format!("<Part><PartNumber>{n}</PartNumber><ETag>{etag}</ETag></Part>"));
                left -= len;
                n += 1;
            }
            Ok(parts)
        })();
        match result {
            Ok(parts) => {
                let body = format!("<CompleteMultipartUpload>{parts}</CompleteMultipartUpload>");
                let h = [("content-length", body.len().to_string())];
                self.call("POST", bucket, key, &[("uploadId", upload)], &h, body, "CompleteMultipartUpload").map(|_| ())
            }
            Err(e) => {
                let _ = self.call("DELETE", bucket, key, &[("uploadId", upload)], &[], (), "AbortMultipartUpload");
                Err(e)
            }
        }
    }
}

impl RemoteFs for S3 {
    fn list(&self, path: &str) -> AppResult<Vec<Entry>> {
        let (bucket, key) = split(path);
        if bucket.is_empty() {
            return self.buckets();
        }
        let prefix = if key.is_empty() { String::new() } else { format!("{key}/") };
        self.objects(&bucket, &prefix, None)
    }
    fn stat(&self, path: &str) -> AppResult<Option<Meta>> {
        let (bucket, key) = split(path);
        if bucket.is_empty() || key.is_empty() {
            return Ok(Some(super::dir_meta()));
        }
        // One listing tells files ("key") from folders ("key/") on every implementation.
        let (prefixes, objects, _) = self.page(&bucket, &key, None, Some(1000))?;
        if let Some((_, size, mtime)) = objects.iter().find(|(k, _, _)| *k == key) {
            return Ok(Some(Meta { is_dir: false, is_link: false, size: *size, mtime: *mtime }));
        }
        let folder = format!("{key}/");
        Ok((prefixes.contains(&folder) || objects.iter().any(|(k, _, _)| *k == folder)).then(super::dir_meta))
    }
    fn mkdir(&self, path: &str) -> AppResult<()> {
        let (bucket, key) = split(path);
        if key.is_empty() {
            // New bucket.
            let body = if self.region == "us-east-1" {
                String::new()
            } else {
                format!("<CreateBucketConfiguration><LocationConstraint>{}</LocationConstraint></CreateBucketConfiguration>", self.region)
            };
            let h = [("content-length", body.len().to_string())];
            return self.call("PUT", &bucket, "", &[], &h, body, "CreateBucket").map(|_| ());
        }
        self.call("PUT", &bucket, &format!("{key}/"), &[], &[("content-length", "0".into())], (), "PUT").map(|_| ())
    }
    fn remove_file(&self, path: &str) -> AppResult<()> {
        let (bucket, key) = split(path);
        self.call("DELETE", &bucket, &key, &[], &[], (), "DELETE").map(|_| ())
    }
    fn remove_dir(&self, path: &str) -> AppResult<()> {
        let (bucket, key) = split(path);
        if key.is_empty() {
            return self.call("DELETE", &bucket, "", &[], &[], (), "DeleteBucket").map(|_| ());
        }
        self.call("DELETE", &bucket, &format!("{key}/"), &[], &[], (), "DELETE").map(|_| ())
    }
    fn rename(&self, from: &str, to: &str) -> AppResult<()> {
        let (b1, k1) = split(from);
        let (b2, k2) = split(to);
        if k1.is_empty() || k2.is_empty() || self.stat(from)?.is_some_and(|m| m.is_dir) {
            // Folders have no server-side rename: copy and delete instead.
            return Err(AppError::new(ErrorCode::CrossesDevices, "S3 folders are copied and deleted"));
        }
        let src = format!("/{}/{}", b1, enc_key(&k1));
        self.call("PUT", &b2, &k2, &[], &[("x-amz-copy-source", src), ("content-length", "0".into())], (), "CopyObject")?;
        self.call("DELETE", &b1, &k1, &[], &[], (), "DELETE").map(|_| ())
    }
    fn reader(&self, path: &str) -> AppResult<Box<dyn ReadStream + '_>> {
        let (bucket, key) = split(path);
        let resp = self.call("GET", &bucket, &key, &[], &[], (), "GET")?;
        Ok(Box::new(PlainReader(resp.into_body().into_reader())))
    }
    fn writer(&self, path: &str) -> AppResult<Box<dyn WriteStream + '_>> {
        let (bucket, key) = split(path);
        if key.is_empty() {
            return Err(AppError::unsupported("Files go into a bucket"));
        }
        Ok(Box::new(SpoolWriter::new(move |file, size| self.put_object(&bucket, &key, file, size))?))
    }
}

/// Text of the first `<name>` element.
fn tag(xml: &str, name: &str) -> Option<String> {
    let open = format!("<{name}>");
    let start = xml.find(&open)? + open.len();
    let end = xml[start..].find(&format!("</{name}>"))? + start;
    Some(quick_xml::escape::unescape(&xml[start..end]).map(|c| c.into_owned()).unwrap_or_else(|_| xml[start..end].to_string()))
}

/// Inner XML of every `<name>…</name>`.
fn blocks(xml: &str, name: &str) -> Vec<String> {
    let (open, close) = (format!("<{name}>"), format!("</{name}>"));
    let mut out = Vec::new();
    let mut rest = xml;
    while let Some(i) = rest.find(&open) {
        let after = &rest[i + open.len()..];
        let Some(j) = after.find(&close) else { break };
        out.push(after[..j].to_string());
        rest = &after[j + close.len()..];
    }
    out
}

/// "2026-10-21T07:28:00.000Z" → ms since epoch.
fn iso_date(s: &str) -> Option<i64> {
    let (d, t) = s.split_once('T')?;
    let mut dp = d.split('-').map(|x| x.parse::<i64>().ok());
    let (y, m, day) = (dp.next()??, dp.next()??, dp.next()??);
    let t = t.trim_end_matches('Z');
    let mut tp = t.split(':');
    let h: i64 = tp.next()?.parse().ok()?;
    let mi: i64 = tp.next()?.parse().ok()?;
    let sec: f64 = tp.next()?.parse().ok()?;
    let days = crate::util::days_from_civil(y as i32, m as u32, day as u32);
    Some((days * 86400 + h * 3600 + mi * 60) * 1000 + (sec * 1000.0) as i64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn helpers() {
        assert_eq!(split("/bucket/a/b"), ("bucket".into(), "a/b".into()));
        assert_eq!(split("/"), (String::new(), String::new()));
        assert_eq!(enc_key("a b/č.txt"), "a%20b/%C4%8D.txt");
        assert_eq!(iso_date("1970-01-01T00:01:00.500Z"), Some(60_500));
        let xml = "<R><Contents><Key>a&amp;b</Key></Contents><Contents><Key>c</Key></Contents></R>";
        assert_eq!(blocks(xml, "Contents").len(), 2);
        assert_eq!(tag(xml, "Key").as_deref(), Some("a&b"));
    }

    /// Multipart upload against a real server:
    /// `DUALION_S3=http://127.0.0.1:8092 DUALION_S3_KEY=… DUALION_S3_SECRET=… cargo test s3_multipart -- --ignored`
    #[test]
    #[ignore]
    fn s3_multipart() {
        use super::super::RemoteFs;
        use std::io::Write;
        let var = |k: &str| std::env::var(k).unwrap_or_default();
        let cfg = RemoteConfig {
            protocol: super::super::Protocol::S3,
            host: var("DUALION_S3"),
            username: var("DUALION_S3_KEY"),
            password: var("DUALION_S3_SECRET").into(),
            remote_path: "dualion".into(),
            ..Default::default()
        };
        let s3 = S3::connect(&cfg).unwrap();
        let size = SINGLE_PUT + PART + 12345;
        let mut w = s3.writer("/dualion/multipart.bin").unwrap();
        let chunk: Vec<u8> = (0..1 << 20).map(|i: u32| (i % 251) as u8).collect();
        let mut left = size;
        while left > 0 {
            let n = left.min(chunk.len() as u64) as usize;
            w.write_all(&chunk[..n]).unwrap();
            left -= n as u64;
        }
        w.finish().unwrap();
        assert_eq!(s3.stat("/dualion/multipart.bin").unwrap().unwrap().size, size);
        let mut r = s3.reader("/dualion/multipart.bin").unwrap();
        let mut head = vec![0u8; 1000];
        r.read_exact(&mut head).unwrap();
        assert_eq!(&head[..], &chunk[..1000]);
        drop(r);
        s3.remove_file("/dualion/multipart.bin").unwrap();
    }

    /// Example from the AWS SigV4 documentation (signing key derivation).
    #[test]
    fn signing_key() {
        let k = hmac(b"AWS4wJalrXUtnFEMI/K7MDENG+bPxRfiCYEXAMPLEKEY", "20120215");
        let k = hmac(&k, "us-east-1");
        let k = hmac(&k, "iam");
        let k = hmac(&k, "aws4_request");
        assert_eq!(hex(&k), "f4780e2d9f65fa895f9c67b32ce1baf0b0d8a43505a000a1a9e090d414db404d");
    }
}
