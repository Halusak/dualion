//! Import of FTP connections from Total Commander's `wcx_ftp.ini`.
//!
//! Passwords saved without a TC master password are only obfuscated and can
//! be decoded (algorithm after PELock's Total Commander FTP Password Recovery
//! Tool, Apache-2.0). Passwords protected by a master password (`!` prefix)
//! are skipped: such sites are imported without a password.

use crate::remote::Protocol;
use crate::vault::Site;

/// Sections that hold settings, not connections.
const RESERVED: &[&str] = &["connections", "general", "default"];

struct Rng(u32);

impl Rng {
    fn below(&mut self, max: u32) -> u32 {
        self.0 = self.0.wrapping_mul(0x8088405).wrapping_add(1);
        ((self.0 as u64 * max as u64) >> 32) as u32
    }
}

/// Decodes an obfuscated TC password (hex, last 4 bytes are a checksum).
fn decode_bytes(hex: &str) -> Option<Vec<u8>> {
    let hex: String = hex.chars().filter(|c| !c.is_whitespace()).collect();
    if hex.len() % 2 != 0 || hex.len() <= 8 {
        return None;
    }
    let mut b = (0..hex.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(hex.get(i..i + 2)?, 16).ok())
        .collect::<Option<Vec<u8>>>()?;
    let n = b.len() - 4;
    b.truncate(n);
    let mut r = Rng(849521);
    for x in b.iter_mut() {
        *x = x.rotate_left(r.below(8));
    }
    let mut r = Rng(12345);
    for _ in 0..256 {
        let (i, j) = (r.below(n as u32) as usize, r.below(n as u32) as usize);
        b.swap(i, j);
    }
    let mut r = Rng(42340);
    for x in b.iter_mut() {
        *x ^= r.below(256) as u8;
    }
    let mut r = Rng(54321);
    for x in b.iter_mut() {
        *x = x.wrapping_sub(r.below(256) as u8);
    }
    Some(b)
}

fn decode_password(hex: &str) -> Option<String> {
    let b = decode_bytes(hex)?;
    Some(match String::from_utf8(b) {
        Ok(s) => s,
        Err(e) => encoding_rs::WINDOWS_1252.decode(e.as_bytes()).0.into_owned(),
    })
}

fn decode_text(bytes: &[u8]) -> String {
    if let Some(rest) = bytes.strip_prefix(&[0xFF, 0xFE]) {
        return encoding_rs::UTF_16LE.decode_without_bom_handling(rest).0.into_owned();
    }
    if let Some(rest) = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]) {
        return String::from_utf8_lossy(rest).into_owned();
    }
    match std::str::from_utf8(bytes) {
        Ok(s) => s.to_string(),
        Err(_) => encoding_rs::WINDOWS_1252.decode(bytes).0.into_owned(),
    }
}

#[derive(Default)]
pub struct Parsed {
    pub sites: Vec<Site>,
    /// Sites whose password is protected by the TC master password.
    pub without_password: usize,
}

pub fn parse(bytes: &[u8]) -> Parsed {
    let text = decode_text(bytes);
    let mut out = Parsed::default();
    let mut sections: Vec<(String, Vec<(String, String)>)> = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with(';') {
            continue;
        }
        if let Some(name) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
            sections.push((name.to_string(), Vec::new()));
        } else if let (Some((k, v)), Some(sec)) = (line.split_once('='), sections.last_mut()) {
            sec.1.push((k.trim().to_ascii_lowercase(), v.trim().to_string()));
        }
    }
    for (name, keys) in sections {
        if RESERVED.contains(&name.to_ascii_lowercase().as_str()) {
            continue;
        }
        let get = |k: &str| keys.iter().find(|(key, _)| key == k).map(|(_, v)| v.as_str()).unwrap_or("");
        let Some((protocol, host, port, path)) = split_host(get("host")) else { continue };
        let pw = get("password");
        let password = if pw.starts_with('!') {
            out.without_password += 1;
            String::new()
        } else {
            decode_password(pw).unwrap_or_default()
        };
        let directory = get("directory");
        out.sites.push(Site {
            name,
            group: "Total Commander".into(),
            protocol,
            host,
            port,
            username: get("username").to_string(),
            password,
            passive: get("pasvmode") != "0",
            remote_path: if directory.is_empty() { path } else { directory.to_string() },
            local_path: get("localdir").to_string(),
            ..Default::default()
        });
    }
    out
}

/// `[scheme://]host[:port][/path]` -> protocol, host, port (0 = default), path.
fn split_host(raw: &str) -> Option<(Protocol, String, u16, String)> {
    let raw = raw.trim();
    let (scheme, rest) = match raw.split_once("://") {
        Some((s, r)) => (s.to_ascii_lowercase(), r),
        None => (String::new(), raw),
    };
    let (hostport, path) = match rest.find('/') {
        Some(i) => (&rest[..i], rest[i..].to_string()),
        None => (rest, String::new()),
    };
    let (host, port) = match hostport.rsplit_once(':') {
        // `[::1]:21` or `host:21`; a bare IPv6 address has several colons.
        Some((h, p)) if !h.contains(':') || h.ends_with(']') => (h, p.parse().unwrap_or(0)),
        _ => (hostport, 0),
    };
    if host.is_empty() {
        return None;
    }
    let protocol = match scheme.as_str() {
        "sftp" => Protocol::Sftp,
        "ftps" if port == 990 => Protocol::FtpsImplicit,
        "ftps" => Protocol::FtpsExplicit,
        _ => Protocol::Ftp,
    };
    Some((protocol, host.to_string(), port, path))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn golden_vector() {
        // Same vector as the PELock decoder tests.
        let b = decode_bytes("00112233445566778899aabbccddeeff").unwrap();
        assert_eq!(b, [0xfd, 0xf3, 0xb3, 0x50, 0xe9, 0xb8, 0xc5, 0xfb, 0xe8, 0x2d, 0x47, 0x8d]);
        assert!(decode_bytes("00112233").is_none());
        assert!(decode_bytes("00112233445566778899aabbccddeefg").is_none());
    }

    #[test]
    fn parses_ini() {
        let ini = "[connections]\r\n1=Web\r\n2=Locked\r\n[default]\r\npasvmode=1\r\n\
                   [Web]\r\nhost=ftp.example.com:2121\r\nusername=joe\r\npassword=00112233445566778899aabbccddeeff\r\n\
                   directory=/www\r\nlocaldir=C:\\web\r\npasvmode=0\r\n\
                   [Locked]\r\nhost=ftps://secure.example.com/pub\r\npassword=!abcdef\r\n[Empty]\r\nusername=x\r\n";
        let p = parse(ini.as_bytes());
        assert_eq!(p.sites.len(), 2);
        assert_eq!(p.without_password, 1);
        let w = &p.sites[0];
        assert_eq!((w.name.as_str(), w.host.as_str(), w.port, w.username.as_str()), ("Web", "ftp.example.com", 2121, "joe"));
        assert_eq!((w.remote_path.as_str(), w.local_path.as_str(), w.passive), ("/www", "C:\\web", false));
        assert!(!w.password.is_empty());
        let l = &p.sites[1];
        assert_eq!((l.protocol, l.host.as_str(), l.remote_path.as_str()), (Protocol::FtpsExplicit, "secure.example.com", "/pub"));
        assert!(l.password.is_empty());
    }

    #[test]
    fn utf16_file() {
        let mut b = vec![0xFF, 0xFE];
        for u in "[Č]\nhost=a.cz\n".encode_utf16() {
            b.extend(u.to_le_bytes());
        }
        let p = parse(&b);
        assert_eq!(p.sites[0].name, "Č");
    }
}
