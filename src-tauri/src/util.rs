use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

/// Milliseconds since the Unix epoch, negative for pre-1970 timestamps.
pub fn system_time_ms(t: SystemTime) -> i64 {
    match t.duration_since(UNIX_EPOCH) {
        Ok(d) => d.as_millis() as i64,
        Err(e) => -(e.duration().as_millis() as i64),
    }
}

pub fn now_ms() -> i64 {
    system_time_ms(SystemTime::now())
}

pub fn ms_to_system_time(ms: i64) -> SystemTime {
    if ms >= 0 {
        UNIX_EPOCH + std::time::Duration::from_millis(ms as u64)
    } else {
        UNIX_EPOCH - std::time::Duration::from_millis(ms.unsigned_abs())
    }
}

/// Splits "archive.tar.gz" style names into stem and extension the way
/// file managers do: the extension is everything after the last dot,
/// except for dot-files (".bashrc" has no extension).
pub fn split_name(name: &str) -> (&str, &str) {
    match name.rfind('.') {
        Some(0) | None => (name, ""),
        Some(i) => (&name[..i], &name[i + 1..]),
    }
}

/// "report.pdf" -> "report (2).pdf", "report (2).pdf" -> "report (3).pdf"
pub fn numbered_name(name: &str, n: u32) -> String {
    let (stem, ext) = split_name(name);
    let base = strip_counter(stem);
    if ext.is_empty() {
        format!("{base} ({n})")
    } else {
        format!("{base} ({n}).{ext}")
    }
}

fn strip_counter(stem: &str) -> &str {
    if let Some(open) = stem.rfind(" (") {
        let tail = &stem[open + 2..];
        if let Some(num) = tail.strip_suffix(')') {
            if !num.is_empty() && num.chars().all(|c| c.is_ascii_digit()) {
                return &stem[..open];
            }
        }
    }
    stem
}

/// Finds the first "name (n).ext" for which `exists` returns false.
pub fn unique_name(name: &str, mut exists: impl FnMut(&str) -> bool) -> String {
    if !exists(name) {
        return name.to_string();
    }
    for n in 2..100_000 {
        let candidate = numbered_name(name, n);
        if !exists(&candidate) {
            return candidate;
        }
    }
    format!("{}-{}", name, uuid::Uuid::new_v4())
}

/// Validates a single path component typed by the user.
pub fn validate_file_name(name: &str) -> Result<(), &'static str> {
    if name.is_empty() || name == "." || name == ".." {
        return Err("empty");
    }
    if name.contains('/') || name.contains('\0') {
        return Err("separator");
    }
    #[cfg(windows)]
    {
        if name.chars().any(|c| matches!(c, '<' | '>' | ':' | '"' | '\\' | '|' | '?' | '*') || (c as u32) < 32) {
            return Err("invalid-char");
        }
        if name.ends_with(' ') || name.ends_with('.') {
            return Err("trailing");
        }
        let stem = name.split('.').next().unwrap_or("").to_ascii_uppercase();
        const RESERVED: [&str; 22] = [
            "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8", "COM9",
            "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
        ];
        if RESERVED.contains(&stem.as_str()) {
            return Err("reserved");
        }
    }
    Ok(())
}

pub fn path_str(p: &Path) -> String {
    p.to_string_lossy().into_owned()
}

/// Expands `~`, `~/x`, `$VAR`, `${VAR}` and `%VAR%` in a user typed path.
pub fn expand_user_path(input: &str) -> PathBuf {
    let trimmed = input.trim();
    let unquoted = trimmed
        .strip_prefix('"')
        .and_then(|s| s.strip_suffix('"'))
        .unwrap_or(trimmed);
    let mut s = unquoted.to_string();
    if s == "~" || s.starts_with("~/") || s.starts_with("~\\") {
        if let Some(home) = dirs::home_dir() {
            s = format!("{}{}", home.to_string_lossy(), &s[1..]);
        }
    }
    PathBuf::from(expand_env(&s))
}

fn expand_env(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let chars: Vec<char> = s.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c == '$' {
            let (name, next) = if chars.get(i + 1) == Some(&'{') {
                match chars[i + 2..].iter().position(|&c| c == '}') {
                    Some(end) => (chars[i + 2..i + 2 + end].iter().collect::<String>(), i + 3 + end),
                    None => (String::new(), i + 1),
                }
            } else {
                let len = chars[i + 1..]
                    .iter()
                    .take_while(|c| c.is_ascii_alphanumeric() || **c == '_')
                    .count();
                (chars[i + 1..i + 1 + len].iter().collect(), i + 1 + len)
            };
            if let Some(v) = (!name.is_empty()).then(|| std::env::var(&name).ok()).flatten() {
                out.push_str(&v);
                i = next;
                continue;
            }
        } else if c == '%' {
            if let Some(end) = chars[i + 1..].iter().position(|&c| c == '%') {
                let name: String = chars[i + 1..i + 1 + end].iter().collect();
                if !name.is_empty() {
                    if let Ok(v) = std::env::var(&name) {
                        out.push_str(&v);
                        i += end + 2;
                        continue;
                    }
                }
            }
        }
        out.push(c);
        i += 1;
    }
    out
}

/// True when `child` equals `ancestor` or lies inside it. Purely lexical,
/// case-insensitive on Windows and macOS (default APFS is case-insensitive).
pub fn is_same_or_inside(child: &Path, ancestor: &Path) -> bool {
    let norm = |p: &Path| -> Vec<String> {
        p.components()
            .filter(|c| !matches!(c, std::path::Component::CurDir))
            .map(|c| {
                let s = c.as_os_str().to_string_lossy();
                if cfg!(any(windows, target_os = "macos")) {
                    s.to_lowercase()
                } else {
                    s.into_owned()
                }
            })
            .collect()
    };
    let c = norm(child);
    let a = norm(ancestor);
    c.len() >= a.len() && c[..a.len()] == a[..]
}

/// Converts a naive local date-time (as stored in ZIP archives) to epoch ms.
pub fn local_naive_to_ms(year: i32, month: u32, day: u32, hour: u32, min: u32, sec: u32) -> i64 {
    #[cfg(unix)]
    {
        let mut tm: libc::tm = unsafe { std::mem::zeroed() };
        tm.tm_year = year - 1900;
        tm.tm_mon = month as i32 - 1;
        tm.tm_mday = day as i32;
        tm.tm_hour = hour as i32;
        tm.tm_min = min as i32;
        tm.tm_sec = sec as i32;
        tm.tm_isdst = -1;
        let t = unsafe { libc::mktime(&mut tm) };
        if t != -1 {
            return t as i64 * 1000;
        }
    }
    days_from_civil(year, month, day) * 86_400_000
        + (hour as i64 * 3600 + min as i64 * 60 + sec as i64) * 1000
}

/// Epoch ms -> UTC (year, month, day, hour, minute, second).
pub fn utc_parts(ms: i64) -> (i32, u32, u32, u32, u32, u32) {
    let secs = ms.div_euclid(1000);
    let days = secs.div_euclid(86_400);
    let rem = secs.rem_euclid(86_400);
    let (y, m, d) = civil_from_days(days);
    (y, m, d, (rem / 3600) as u32, ((rem % 3600) / 60) as u32, (rem % 60) as u32)
}

/// Epoch ms -> local (year, month, day, hour, minute, second).
pub fn local_parts(ms: i64) -> (i32, u32, u32, u32, u32, u32) {
    #[cfg(unix)]
    {
        let t: libc::time_t = ms.div_euclid(1000) as libc::time_t;
        let mut tm: libc::tm = unsafe { std::mem::zeroed() };
        if !unsafe { libc::localtime_r(&t, &mut tm) }.is_null() {
            return (
                tm.tm_year + 1900,
                tm.tm_mon as u32 + 1,
                tm.tm_mday as u32,
                tm.tm_hour as u32,
                tm.tm_min as u32,
                tm.tm_sec as u32,
            );
        }
    }
    #[cfg(windows)]
    {
        use windows::Win32::Foundation::SYSTEMTIME;
        use windows::Win32::System::Time::SystemTimeToTzSpecificLocalTime;
        let (y, mo, d, h, mi, s) = utc_parts(ms);
        let utc = SYSTEMTIME {
            wYear: y as u16,
            wMonth: mo as u16,
            wDay: d as u16,
            wHour: h as u16,
            wMinute: mi as u16,
            wSecond: s as u16,
            ..Default::default()
        };
        let mut local = SYSTEMTIME::default();
        if unsafe { SystemTimeToTzSpecificLocalTime(None, &utc, &mut local) }.is_ok() {
            return (
                local.wYear as i32,
                local.wMonth as u32,
                local.wDay as u32,
                local.wHour as u32,
                local.wMinute as u32,
                local.wSecond as u32,
            );
        }
    }
    #[allow(unreachable_code)]
    utc_parts(ms)
}

fn civil_from_days(z: i64) -> (i32, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    ((if m <= 2 { y + 1 } else { y }) as i32, m as u32, d as u32)
}

pub fn days_from_civil(y: i32, m: u32, d: u32) -> i64 {
    let y = if m <= 2 { y - 1 } else { y } as i64;
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let m = m as i64;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d as i64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbered_names() {
        assert_eq!(numbered_name("a.txt", 2), "a (2).txt");
        assert_eq!(numbered_name("a (2).txt", 3), "a (3).txt");
        assert_eq!(numbered_name("folder", 2), "folder (2)");
        assert_eq!(numbered_name(".bashrc", 2), ".bashrc (2)");
        assert_eq!(numbered_name("x (draft).md", 2), "x (draft) (2).md");
    }

    #[test]
    fn unique_name_skips_existing() {
        let existing = ["a.txt", "a (2).txt"];
        assert_eq!(unique_name("a.txt", |n| existing.contains(&n)), "a (3).txt");
        assert_eq!(unique_name("b.txt", |n| existing.contains(&n)), "b.txt");
    }

    #[test]
    fn split_names() {
        assert_eq!(split_name("a.tar.gz"), ("a.tar", "gz"));
        assert_eq!(split_name(".env"), (".env", ""));
        assert_eq!(split_name("Makefile"), ("Makefile", ""));
    }

    #[test]
    fn inside_checks() {
        assert!(is_same_or_inside(Path::new("/a/b/c"), Path::new("/a/b")));
        assert!(is_same_or_inside(Path::new("/a/b"), Path::new("/a/b")));
        assert!(!is_same_or_inside(Path::new("/a/bc"), Path::new("/a/b")));
        assert!(!is_same_or_inside(Path::new("/a"), Path::new("/a/b")));
    }

    #[test]
    fn env_expansion() {
        std::env::set_var("DUALION_TEST_VAR", "xyz");
        assert_eq!(expand_env("/a/$DUALION_TEST_VAR/b"), "/a/xyz/b");
        assert_eq!(expand_env("/a/${DUALION_TEST_VAR}b"), "/a/xyzb");
        assert_eq!(expand_env("C:\\%DUALION_TEST_VAR%\\x"), "C:\\xyz\\x");
        assert_eq!(expand_env("100% sure"), "100% sure");
        assert_eq!(expand_env("$NOT_SET_DUALION"), "$NOT_SET_DUALION");
    }

    #[test]
    fn file_name_validation() {
        assert!(validate_file_name("ok.txt").is_ok());
        assert!(validate_file_name("").is_err());
        assert!(validate_file_name("..").is_err());
        assert!(validate_file_name("a/b").is_err());
    }

    #[test]
    fn civil_days() {
        assert_eq!(days_from_civil(1970, 1, 1), 0);
        assert_eq!(days_from_civil(2000, 3, 1), 11017);
        assert_eq!(civil_from_days(11017), (2000, 3, 1));
        assert_eq!(utc_parts(1_700_000_000_000), (2023, 11, 14, 22, 13, 20));
    }
}
