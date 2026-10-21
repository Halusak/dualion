//! WebDAV (Nextcloud, ownCloud, Synology, Apache, nginx, SharePoint…).
//! The host may be a full URL (`https://cloud.example.com/remote.php/dav/files/jan`);
//! panel paths are relative to it.

use super::{entry, PlainReader, Protocol, RemoteConfig, RemoteFs, SpoolWriter};
use crate::error::{AppError, AppResult, ErrorCode};
use crate::jobs::vfs::{Meta, ReadStream, WriteStream};
use crate::location::Entry;
use base64::Engine;
use percent_encoding::{percent_decode_str, utf8_percent_encode, AsciiSet, NON_ALPHANUMERIC};
use std::time::Duration;
use ureq::http;

const SEGMENT: &AsciiSet = &NON_ALPHANUMERIC.remove(b'-').remove(b'.').remove(b'_').remove(b'~');

const PROPFIND: &str = r#"<?xml version="1.0" encoding="utf-8"?><d:propfind xmlns:d="DAV:"><d:prop><d:resourcetype/><d:getcontentlength/><d:getlastmodified/></d:prop></d:propfind>"#;

pub struct WebDav {
    agent: ureq::Agent,
    /// "https://host:port" without a trailing slash.
    origin: String,
    /// Path of the root on the server, without a trailing slash ("" or "/dav").
    base: String,
    auth: Option<String>,
}

/// Builds an agent for HTTP(S) services (native TLS, our own status handling).
pub fn agent(insecure: bool) -> ureq::Agent {
    let tls = ureq::tls::TlsConfig::builder().provider(ureq::tls::TlsProvider::NativeTls).disable_verification(insecure).build();
    ureq::Agent::config_builder()
        .tls_config(tls)
        .http_status_as_error(false)
        .allow_non_standard_methods(true)
        .timeout_connect(Some(Duration::from_secs(20)))
        .timeout_recv_response(Some(Duration::from_secs(120)))
        .user_agent(concat!("Dualion/", env!("CARGO_PKG_VERSION")))
        .build()
        .into()
}

/// Maps an HTTP status to an error.
pub fn status_error(status: u16, what: &str) -> AppError {
    let code = match status {
        401 => ErrorCode::FtpAuth,
        403 => ErrorCode::PermissionDenied,
        404 | 409 => ErrorCode::NotFound,
        405 | 412 => ErrorCode::AlreadyExists,
        423 => ErrorCode::Busy,
        507 => ErrorCode::StorageFull,
        _ => ErrorCode::Io,
    };
    AppError::new(code, format!("{what}: HTTP {status}"))
}

fn http_err(e: ureq::Error) -> AppError {
    AppError::new(ErrorCode::FtpConnect, e.to_string())
}

/// Splits "https://h:1/a/b" into ("https://h:1", "/a/b").
fn split_url(url: &str) -> (String, String) {
    let url = url.trim().trim_end_matches('/');
    let after = url.find("://").map(|i| i + 3).unwrap_or(0);
    match url[after..].find('/') {
        Some(i) => (url[..after + i].to_string(), url[after + i..].to_string()),
        None => (url.to_string(), String::new()),
    }
}

impl WebDav {
    pub fn connect(cfg: &RemoteConfig) -> AppResult<Self> {
        let host = cfg.host.trim();
        let url = if host.contains("://") {
            host.to_string()
        } else {
            let scheme = if cfg.protocol == Protocol::Webdav { "http" } else { "https" };
            let port = cfg.effective_port();
            let port_part = if port == cfg.protocol.default_port() { String::new() } else { format!(":{port}") };
            format!("{scheme}://{host}{port_part}")
        };
        let (origin, base) = split_url(&url);
        let user = cfg.username.trim();
        let auth = (!user.is_empty()).then(|| format!("Basic {}", base64::engine::general_purpose::STANDARD.encode(format!("{user}:{}", cfg.password.as_str()))));
        let dav = Self { agent: agent(cfg.accept_invalid_certs), origin, base, auth };
        // Fail early on a wrong address or credentials.
        match dav.stat("/")? {
            Some(m) if m.is_dir => Ok(dav),
            _ => Err(AppError::new(ErrorCode::FtpConnect, "No WebDAV folder at this address")),
        }
    }

    /// Encoded server path of a panel path.
    fn href(&self, path: &str) -> String {
        let mut out = self.base.clone();
        for seg in path.split('/').filter(|s| !s.is_empty()) {
            out.push('/');
            out.push_str(&utf8_percent_encode(seg, SEGMENT).to_string());
        }
        if out.is_empty() {
            out.push('/');
        }
        out
    }

    fn url(&self, path: &str) -> String {
        format!("{}{}", self.origin, self.href(path))
    }

    fn request(&self, method: &str, path: &str) -> http::request::Builder {
        let mut b = http::Request::builder().method(method).uri(self.url(path));
        if let Some(a) = &self.auth {
            b = b.header("Authorization", a);
        }
        b
    }

    fn send(&self, req: http::request::Builder, body: impl ureq::AsSendBody, what: &str) -> AppResult<http::Response<ureq::Body>> {
        let req = req.body(body).map_err(|e| AppError::invalid(e.to_string()))?;
        let resp = self.agent.run(req).map_err(http_err)?;
        let s = resp.status().as_u16();
        if (200..300).contains(&s) {
            Ok(resp)
        } else {
            Err(status_error(s, what))
        }
    }

    fn propfind(&self, path: &str, depth: &str) -> AppResult<Vec<(String, Entry)>> {
        let req = self.request("PROPFIND", path).header("Depth", depth).header("Content-Type", "application/xml; charset=utf-8");
        let mut resp = self.send(req, PROPFIND, "PROPFIND")?;
        let xml = resp.body_mut().with_config().limit(64 << 20).read_to_string().map_err(http_err)?;
        Ok(parse_multistatus(&xml))
    }

    /// Server path (decoded) → panel path.
    fn panel_path(&self, href: &str) -> String {
        let href = if let Some(rest) = href.strip_prefix(&self.origin) { rest } else { href };
        let decoded = percent_decode_str(href).decode_utf8_lossy().into_owned();
        let base = percent_decode_str(&self.base).decode_utf8_lossy().into_owned();
        super::norm(decoded.strip_prefix(&base).unwrap_or(&decoded))
    }
}

impl RemoteFs for WebDav {
    fn list(&self, path: &str) -> AppResult<Vec<Entry>> {
        let me = super::norm(path);
        let mut out = Vec::new();
        for (href, e) in self.propfind(path, "1")? {
            if self.panel_path(&href) == me {
                continue;
            }
            out.push(e);
        }
        Ok(out)
    }
    fn stat(&self, path: &str) -> AppResult<Option<Meta>> {
        match self.propfind(path, "0") {
            Ok(list) => Ok(list.first().map(|(_, e)| super::meta_of(e))),
            Err(e) if e.code == ErrorCode::NotFound => Ok(None),
            Err(e) => Err(e),
        }
    }
    fn mkdir(&self, path: &str) -> AppResult<()> {
        self.send(self.request("MKCOL", path), (), "MKCOL").map(|_| ())
    }
    fn remove_file(&self, path: &str) -> AppResult<()> {
        self.send(self.request("DELETE", path), (), "DELETE").map(|_| ())
    }
    fn remove_dir(&self, path: &str) -> AppResult<()> {
        self.send(self.request("DELETE", &format!("{}/", path.trim_end_matches('/'))), (), "DELETE").map(|_| ())
    }
    fn rename(&self, from: &str, to: &str) -> AppResult<()> {
        let req = self.request("MOVE", from).header("Destination", self.url(to)).header("Overwrite", "F");
        self.send(req, (), "MOVE").map(|_| ())
    }
    fn reader(&self, path: &str) -> AppResult<Box<dyn ReadStream + '_>> {
        let resp = self.send(self.request("GET", path), (), "GET")?;
        Ok(Box::new(PlainReader(resp.into_body().into_reader())))
    }
    fn writer(&self, path: &str) -> AppResult<Box<dyn WriteStream + '_>> {
        let path = path.to_string();
        Ok(Box::new(SpoolWriter::new(move |file, size| {
            let req = self.request("PUT", &path).header("Content-Length", size.to_string()).header("Content-Type", "application/octet-stream");
            self.send(req, ureq::SendBody::from_owned_reader(file), "PUT").map(|_| ())
        })?))
    }
}

/// `<d:multistatus>` → (href, entry) per `<d:response>`.
fn parse_multistatus(xml: &str) -> Vec<(String, Entry)> {
    use quick_xml::events::Event;
    let mut r = quick_xml::Reader::from_str(xml);
    let mut out = Vec::new();
    let (mut href, mut is_dir, mut size, mut mtime) = (String::new(), false, 0u64, 0i64);
    let mut text = String::new();
    loop {
        match r.read_event() {
            Ok(Event::Start(e)) => {
                text.clear();
                let name = e.local_name();
                let name = name.as_ref();
                if name.eq_ignore_ascii_case("response") {
                    (href, is_dir, size, mtime) = (String::new(), false, 0, 0);
                } else if name.eq_ignore_ascii_case("collection") {
                    is_dir = true;
                }
            }
            Ok(Event::Empty(e)) => {
                if e.local_name().as_ref().eq_ignore_ascii_case("collection") {
                    is_dir = true;
                }
            }
            Ok(Event::Text(t)) => text.push_str(&t.xml10_content()),
            Ok(Event::GeneralRef(e)) => {
                if let Ok(Some(c)) = e.resolve_char_ref() {
                    text.push(c);
                } else {
                    text.push_str(match e.xml10_content().as_ref() {
                        "amp" => "&",
                        "lt" => "<",
                        "gt" => ">",
                        "quot" => "\"",
                        "apos" => "'",
                        _ => "",
                    });
                }
            }
            Ok(Event::End(e)) => {
                let name = e.local_name();
                let name = name.as_ref().to_ascii_lowercase();
                match name.as_str() {
                    "href" => href = text.trim().to_string(),
                    "getcontentlength" => size = text.trim().parse().unwrap_or(0),
                    "getlastmodified" => mtime = http_date(text.trim()).unwrap_or(0),
                    "response" if !href.is_empty() => {
                        let path = percent_decode_str(href.trim_end_matches('/')).decode_utf8_lossy().into_owned();
                        let name = path.rsplit('/').next().unwrap_or("").to_string();
                        out.push((href.clone(), entry(name, is_dir, size, mtime, 0, false)));
                    }
                    _ => {}
                }
                text.clear();
            }
            Ok(Event::Eof) | Err(_) => break,
            _ => {}
        }
    }
    out
}

/// RFC 1123 date ("Wed, 21 Oct 2026 07:28:00 GMT") → ms since epoch.
pub fn http_date(s: &str) -> Option<i64> {
    let parts: Vec<&str> = s.split_whitespace().collect();
    let [_, day, mon, year, time, ..] = parts.as_slice() else { return None };
    let month = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"].iter().position(|m| m.eq_ignore_ascii_case(mon))? as u32 + 1;
    let mut t = time.split(':').map(|x| x.parse::<i64>().ok());
    let (h, m, sec) = (t.next()??, t.next()??, t.next()??);
    let days = crate::util::days_from_civil(year.parse().ok()?, month, day.parse().ok()?);
    Some(((days * 86400) + h * 3600 + m * 60 + sec) * 1000)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls() {
        assert_eq!(split_url("https://h.cz:8443/remote.php/dav/"), ("https://h.cz:8443".into(), "/remote.php/dav".into()));
        assert_eq!(split_url("http://h"), ("http://h".into(), String::new()));
    }

    #[test]
    fn dates() {
        assert_eq!(http_date("Thu, 01 Jan 1970 00:01:00 GMT"), Some(60_000));
        assert_eq!(http_date("Wed, 21 Oct 2026 07:28:00 GMT"), Some(1_792_567_680_000));
    }

    #[test]
    fn parses_propfind() {
        let xml = r#"<?xml version="1.0"?><d:multistatus xmlns:d="DAV:">
          <d:response><d:href>/dav/Docs/</d:href><d:propstat><d:prop><d:resourcetype><d:collection/></d:resourcetype></d:prop><d:status>HTTP/1.1 200 OK</d:status></d:propstat></d:response>
          <d:response><d:href>/dav/Docs/P%C5%99%C3%ADloha%201.pdf</d:href><d:propstat><d:prop><d:resourcetype/><d:getcontentlength>1234</d:getcontentlength>
            <d:getlastmodified>Thu, 01 Jan 1970 00:01:00 GMT</d:getlastmodified></d:prop><d:status>HTTP/1.1 200 OK</d:status></d:propstat></d:response>
          <d:response><d:href>/dav/Docs/Sub/</d:href><d:propstat><d:prop><d:resourcetype><d:collection/></d:resourcetype></d:prop><d:status>HTTP/1.1 200 OK</d:status></d:propstat>
            <d:propstat><d:prop><d:getcontentlength/></d:prop><d:status>HTTP/1.1 404 Not Found</d:status></d:propstat></d:response>
        </d:multistatus>"#;
        let list = parse_multistatus(xml);
        assert_eq!(list.len(), 3);
        assert!(list[0].1.is_dir());
        assert_eq!((list[1].1.name.as_str(), list[1].1.size, list[1].1.mtime), ("Příloha 1.pdf", 1234, 60_000));
        assert!(list[2].1.is_dir() && list[2].1.name == "Sub");
    }
}
