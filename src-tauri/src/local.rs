//! Local file system: listings, volumes, path helpers.

use crate::error::{AppError, AppResult, ErrorCode};
use crate::location::{flags, Crumb, Entry, Listing, Location};
use crate::util::{self, path_str, system_time_ms};
use serde::Serialize;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;

#[cfg(target_os = "macos")]
const PACKAGE_EXTS: &[&str] = &[
    "app", "bundle", "framework", "plugin", "appex", "kext", "prefpane", "qlgenerator", "mdimporter",
    "saver", "xpc", "photoslibrary", "rtfd", "pages", "numbers", "key", "playground", "xcarchive",
    "imovielibrary", "musiclibrary", "band", "fcpbundle", "logicx", "scptd", "dSYM", "pkg", "mpkg",
];

#[cfg(windows)]
const EXEC_EXTS: &[&str] = &["exe", "bat", "cmd", "com", "msi", "ps1", "vbs", "scr"];

/// Normalizes a user/frontend supplied local path ("C:" -> "C:\").
pub fn normalize(path: &str) -> PathBuf {
    #[cfg(windows)]
    {
        let p = path.replace('/', "\\");
        if p.len() == 2 && p.ends_with(':') {
            return PathBuf::from(format!("{p}\\"));
        }
        PathBuf::from(p)
    }
    #[cfg(not(windows))]
    {
        if path.is_empty() {
            return PathBuf::from("/");
        }
        PathBuf::from(path)
    }
}

pub fn entry_from_metadata(name: String, path: &Path, meta: &fs::Metadata) -> Entry {
    let mut f = 0u32;
    let mut size = meta.len();
    let mut mtime = meta.modified().map(system_time_ms).unwrap_or(0);
    let mut is_dir = meta.is_dir();

    if meta.file_type().is_symlink() {
        f |= flags::SYMLINK;
        match fs::metadata(path) {
            Ok(target) => {
                is_dir = target.is_dir();
                size = target.len();
                mtime = target.modified().map(system_time_ms).unwrap_or(mtime);
            }
            Err(_) => f |= flags::BROKEN,
        }
    }
    if is_dir {
        f |= flags::DIR;
        size = 0;
    }
    if name.starts_with('.') {
        f |= flags::HIDDEN;
    }

    #[cfg(unix)]
    let attrs = {
        use std::os::unix::fs::PermissionsExt;
        let mode = meta.permissions().mode();
        if !is_dir && mode & 0o111 != 0 {
            f |= flags::EXEC;
        }
        if mode & 0o222 == 0 {
            f |= flags::READONLY;
        }
        mode
    };
    #[cfg(target_os = "macos")]
    {
        use std::os::macos::fs::MetadataExt;
        const UF_HIDDEN: u32 = 0x8000;
        if meta.st_flags() & UF_HIDDEN != 0 {
            f |= flags::HIDDEN;
        }
        if is_dir {
            let (_, ext) = util::split_name(&name);
            if !ext.is_empty() && PACKAGE_EXTS.iter().any(|e| e.eq_ignore_ascii_case(ext)) {
                f |= flags::PACKAGE;
            }
        }
    }
    #[cfg(windows)]
    let attrs = {
        use std::os::windows::fs::MetadataExt;
        let a = meta.file_attributes();
        if a & 0x2 != 0 {
            f |= flags::HIDDEN;
        }
        if a & 0x4 != 0 {
            f |= flags::SYSTEM;
        }
        if a & 0x1 != 0 {
            f |= flags::READONLY;
        }
        if !is_dir {
            let (_, ext) = util::split_name(&name);
            if EXEC_EXTS.iter().any(|e| e.eq_ignore_ascii_case(ext)) {
                f |= flags::EXEC;
            }
        }
        a
    };

    if !is_dir && util::is_cloud_only(meta) {
        f |= flags::CLOUD;
    }
    Entry { name, flags: f, size, mtime, attrs }
}

pub fn read_entries(dir: &Path) -> AppResult<Vec<Entry>> {
    let rd = fs::read_dir(dir).map_err(|e| AppError::io(e, dir))?;
    let mut out = Vec::with_capacity(64);
    for item in rd {
        let Ok(item) = item else { continue };
        let name = item.file_name().to_string_lossy().into_owned();
        let path = item.path();
        // DirEntry::metadata is free on Windows (comes with FindNextFile) and an lstat elsewhere.
        let meta = match item.metadata() {
            Ok(m) => m,
            Err(_) => match fs::symlink_metadata(&path) {
                Ok(m) => m,
                Err(_) => {
                    out.push(Entry { name, flags: flags::BROKEN, size: 0, mtime: 0, attrs: 0 });
                    continue;
                }
            },
        };
        out.push(entry_from_metadata(name, &path, &meta));
    }
    Ok(out)
}

pub fn list_local(path: &str) -> AppResult<Listing> {
    let dir = normalize(path);
    let meta = fs::metadata(&dir).map_err(|e| AppError::io(e, &dir))?;
    if !meta.is_dir() {
        return Err(AppError::new(ErrorCode::NotADirectory, "Not a directory").with_path(&dir));
    }
    let entries = read_entries(&dir)?;
    let display = path_str(&dir);
    Ok(Listing {
        location: Location::local(display.clone()),
        parent: local_parent(&dir),
        crumbs: local_crumbs(&dir),
        display_path: display,
        entries,
    })
}

pub fn local_parent(dir: &Path) -> Option<Location> {
    match dir.parent() {
        Some(p) if !p.as_os_str().is_empty() => Some(Location::local(path_str(p))),
        _ => {
            if cfg!(windows) && !path_str(dir).starts_with("\\\\") {
                Some(Location::Drives)
            } else {
                None
            }
        }
    }
}

pub fn local_crumbs(dir: &Path) -> Vec<Crumb> {
    let mut crumbs = Vec::new();
    let home = dirs::home_dir();
    let mut acc = PathBuf::new();
    let comps: Vec<_> = dir.components().collect();
    let mut i = 0;

    #[cfg(windows)]
    {
        use std::path::{Component, Prefix};
        crumbs.push(Crumb { label: String::new(), icon: "drives", location: Location::Drives });
        if let Some(Component::Prefix(prefix)) = comps.first() {
            let label = match prefix.kind() {
                Prefix::Disk(letter) | Prefix::VerbatimDisk(letter) => {
                    let root = format!("{}:\\", letter as char);
                    let vol = volume_label(&root);
                    if vol.is_empty() {
                        format!("{}:", letter as char)
                    } else {
                        format!("{vol} ({}:)", letter as char)
                    }
                }
                _ => prefix.as_os_str().to_string_lossy().into_owned(),
            };
            acc.push(prefix.as_os_str());
            acc.push("\\");
            let icon = if matches!(prefix.kind(), Prefix::UNC(..) | Prefix::VerbatimUNC(..)) { "server" } else { "volume" };
            crumbs.push(Crumb { label, icon, location: Location::local(path_str(&acc)) });
            i = 2; // prefix + root dir
        }
    }
    #[cfg(not(windows))]
    {
        // macOS: "/Volumes/X/…" starts at the volume, everything else at the root volume.
        let s = path_str(dir);
        if let Some(rest) = s.strip_prefix("/Volumes/") {
            let vol = rest.split('/').next().unwrap_or("");
            if !vol.is_empty() {
                acc.push("/Volumes");
                acc.push(vol);
                crumbs.push(Crumb { label: vol.to_string(), icon: "volume", location: Location::local(path_str(&acc)) });
                i = 3;
            }
        }
        if crumbs.is_empty() {
            acc.push("/");
            crumbs.push(Crumb { label: root_volume_name(), icon: "volume", location: Location::local("/") });
            i = 1;
        }
    }

    for c in comps.iter().skip(i) {
        acc.push(c.as_os_str());
        let is_home = home.as_deref() == Some(acc.as_path());
        crumbs.push(Crumb {
            label: c.as_os_str().to_string_lossy().into_owned(),
            icon: if is_home { "home" } else { "folder" },
            location: Location::local(path_str(&acc)),
        });
    }
    crumbs
}

#[cfg(not(windows))]
pub fn root_volume_name() -> String {
    use std::sync::OnceLock;
    static NAME: OnceLock<String> = OnceLock::new();
    NAME.get_or_init(|| {
        if let Ok(rd) = fs::read_dir("/Volumes") {
            for e in rd.flatten() {
                if let Ok(target) = fs::read_link(e.path()) {
                    if target == Path::new("/") {
                        return e.file_name().to_string_lossy().into_owned();
                    }
                }
            }
        }
        if cfg!(target_os = "macos") { "Macintosh HD".into() } else { "/".into() }
    })
    .clone()
}

// ---------------------------------------------------------------------------
// Volumes
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Volume {
    pub name: String,
    pub path: String,
    pub total: u64,
    pub free: u64,
    /// "system" | "fixed" | "removable" | "network" | "cdrom" | "ram"
    pub kind: &'static str,
}

#[derive(Debug, Clone, Copy, Serialize, Default)]
pub struct Space {
    pub total: u64,
    pub free: u64,
}

#[cfg(unix)]
pub fn space(path: &Path) -> Option<Space> {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;
    let c = CString::new(path.as_os_str().as_bytes()).ok()?;
    let mut st: libc::statvfs = unsafe { std::mem::zeroed() };
    if unsafe { libc::statvfs(c.as_ptr(), &mut st) } != 0 {
        return None;
    }
    let frsize = st.f_frsize as u64;
    Some(Space { total: st.f_blocks as u64 * frsize, free: st.f_bavail as u64 * frsize })
}

#[cfg(windows)]
pub fn space(path: &Path) -> Option<Space> {
    use windows::core::HSTRING;
    use windows::Win32::Storage::FileSystem::GetDiskFreeSpaceExW;
    let mut free = 0u64;
    let mut total = 0u64;
    unsafe { GetDiskFreeSpaceExW(&HSTRING::from(path.as_os_str()), Some(&mut free), Some(&mut total), None) }.ok()?;
    Some(Space { total, free })
}

#[cfg(target_os = "macos")]
pub fn volumes() -> Vec<Volume> {
    use std::ffi::CStr;
    let mut out = Vec::new();
    let mut buf: *mut libc::statfs = std::ptr::null_mut();
    let n = unsafe { libc::getmntinfo(&mut buf, libc::MNT_NOWAIT) };
    if n <= 0 || buf.is_null() {
        return out;
    }
    let mounts = unsafe { std::slice::from_raw_parts(buf, n as usize) };
    const MNT_DONTBROWSE: u32 = 0x0010_0000;
    for m in mounts {
        let on = unsafe { CStr::from_ptr(m.f_mntonname.as_ptr()) }.to_string_lossy().into_owned();
        let is_root = on == "/";
        if !is_root && !on.starts_with("/Volumes/") {
            continue;
        }
        if m.f_flags & MNT_DONTBROWSE != 0 {
            continue;
        }
        let local = m.f_flags & libc::MNT_LOCAL as u32 != 0;
        let fstype = unsafe { CStr::from_ptr(m.f_fstypename.as_ptr()) }.to_string_lossy().into_owned();
        let kind = if is_root {
            "system"
        } else if !local {
            "network"
        } else if fstype == "cd9660" || fstype == "udf" {
            "cdrom"
        } else {
            "removable"
        };
        let bsize = m.f_bsize as u64;
        out.push(Volume {
            name: if is_root { root_volume_name() } else { on.trim_start_matches("/Volumes/").to_string() },
            path: on,
            total: m.f_blocks * bsize,
            free: m.f_bavail * bsize,
            kind,
        });
    }
    out
}

#[cfg(all(unix, not(target_os = "macos")))]
pub fn volumes() -> Vec<Volume> {
    let s = space(Path::new("/")).unwrap_or_default();
    vec![Volume { name: "/".into(), path: "/".into(), total: s.total, free: s.free, kind: "system" }]
}

#[cfg(windows)]
pub fn volume_label(root: &str) -> String {
    use windows::core::HSTRING;
    use windows::Win32::Storage::FileSystem::GetVolumeInformationW;
    let mut name = [0u16; 261];
    let ok = unsafe { GetVolumeInformationW(&HSTRING::from(root), Some(&mut name), None, None, None, None) };
    if ok.is_err() {
        return String::new();
    }
    let len = name.iter().position(|&c| c == 0).unwrap_or(name.len());
    String::from_utf16_lossy(&name[..len])
}

#[cfg(windows)]
pub fn volumes() -> Vec<Volume> {
    use windows::core::HSTRING;
    use windows::Win32::Storage::FileSystem::{GetDriveTypeW, GetLogicalDrives};
    let mask = unsafe { GetLogicalDrives() };
    let mut out = Vec::new();
    for i in 0..26u32 {
        if mask & (1 << i) == 0 {
            continue;
        }
        let letter = (b'A' + i as u8) as char;
        let root = format!("{letter}:\\");
        let kind = match unsafe { GetDriveTypeW(&HSTRING::from(root.as_str())) } {
            2 => "removable",
            3 => "fixed",
            4 => "network",
            5 => "cdrom",
            6 => "ram",
            _ => "fixed",
        };
        let label = volume_label(&root);
        let sp = space(Path::new(&root)).unwrap_or_default();
        let windir = std::env::var("SystemDrive").unwrap_or_else(|_| "C:".into());
        let kind = if root.starts_with(&windir) { "system" } else { kind };
        out.push(Volume {
            name: if label.is_empty() { format!("{letter}:") } else { format!("{label} ({letter}:)") },
            path: root,
            total: sp.total,
            free: sp.free,
            kind,
        });
    }
    out
}

pub fn list_drives() -> Listing {
    let entries = volumes()
        .into_iter()
        .map(|v| Entry { name: v.path, flags: flags::DIR | flags::VOLUME, size: v.total, mtime: 0, attrs: 0 })
        .collect();
    Listing {
        location: Location::Drives,
        display_path: String::new(),
        parent: None,
        crumbs: vec![Crumb { label: String::new(), icon: "drives", location: Location::Drives }],
        entries,
    }
}

// ---------------------------------------------------------------------------
// Directory sizes
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct DirSize {
    pub bytes: u64,
    pub files: u64,
    pub dirs: u64,
}

/// Recursively sums file sizes using a parallel walker. Symlinks are not followed.
pub fn dir_size(path: &Path, cancel: &Arc<AtomicBool>) -> DirSize {
    let bytes = Arc::new(AtomicU64::new(0));
    let files = Arc::new(AtomicU64::new(0));
    let dirs = Arc::new(AtomicU64::new(0));
    let walker = ignore::WalkBuilder::new(path)
        .standard_filters(false)
        .hidden(false)
        .follow_links(false)
        .threads(std::thread::available_parallelism().map(|n| n.get().min(8)).unwrap_or(4))
        .build_parallel();
    walker.run(|| {
        let (bytes, files, dirs, cancel) = (bytes.clone(), files.clone(), dirs.clone(), cancel.clone());
        Box::new(move |res| {
            if cancel.load(Ordering::Relaxed) {
                return ignore::WalkState::Quit;
            }
            if let Ok(e) = res {
                if e.depth() == 0 {
                    return ignore::WalkState::Continue;
                }
                match e.file_type() {
                    Some(ft) if ft.is_dir() => {
                        dirs.fetch_add(1, Ordering::Relaxed);
                    }
                    Some(_) => {
                        files.fetch_add(1, Ordering::Relaxed);
                        if let Ok(m) = e.metadata() {
                            bytes.fetch_add(allocated_or_len(&m), Ordering::Relaxed);
                        }
                    }
                    None => {}
                }
            }
            ignore::WalkState::Continue
        })
    });
    DirSize { bytes: bytes.load(Ordering::Relaxed), files: files.load(Ordering::Relaxed), dirs: dirs.load(Ordering::Relaxed) }
}

fn allocated_or_len(m: &fs::Metadata) -> u64 {
    m.len()
}

// ---------------------------------------------------------------------------
// Path input helpers (address bar)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Resolved {
    pub location: Location,
    /// When the user typed a path to a file, the panel opens its folder and focuses it.
    pub focus: Option<String>,
}

pub fn resolve_input(input: &str, base: Option<&str>) -> AppResult<Resolved> {
    let mut p = util::expand_user_path(input);
    if p.is_relative() {
        if let Some(b) = base {
            p = normalize(b).join(p);
        }
    }
    let p = normalize(&path_str(&p));
    let meta = fs::metadata(&p).map_err(|e| AppError::io(e, &p))?;
    let clean = fs::canonicalize(&p).map(strip_verbatim).unwrap_or(p.clone());
    if meta.is_dir() {
        Ok(Resolved { location: Location::local(path_str(&clean)), focus: None })
    } else {
        let parent = clean.parent().map(Path::to_path_buf).unwrap_or_else(|| clean.clone());
        Ok(Resolved {
            location: Location::local(path_str(&parent)),
            focus: clean.file_name().map(|n| n.to_string_lossy().into_owned()),
        })
    }
}

/// Removes the `\\?\` prefix `canonicalize` adds on Windows.
pub fn strip_verbatim(p: PathBuf) -> PathBuf {
    #[cfg(windows)]
    {
        let s = path_str(&p);
        if let Some(rest) = s.strip_prefix(r"\\?\UNC\") {
            return PathBuf::from(format!(r"\\{rest}"));
        }
        if let Some(rest) = s.strip_prefix(r"\\?\") {
            return PathBuf::from(rest);
        }
    }
    p
}

/// Directory completions for the address bar: subdirectories of the typed
/// parent whose name starts with the typed fragment (case-insensitive).
pub fn complete_dirs(input: &str, base: Option<&str>, show_hidden: bool) -> Vec<String> {
    let expanded = util::expand_user_path(input);
    let typed = path_str(&expanded);
    let ends_with_sep = typed.ends_with('/') || typed.ends_with(std::path::MAIN_SEPARATOR);
    let mut full = expanded.clone();
    if full.is_relative() {
        if let Some(b) = base {
            full = normalize(b).join(full);
        }
    }
    let (dir, prefix) = if ends_with_sep {
        (full, String::new())
    } else {
        match (full.parent(), full.file_name()) {
            (Some(d), Some(f)) => (d.to_path_buf(), f.to_string_lossy().to_lowercase()),
            _ => (full, String::new()),
        }
    };
    let Ok(rd) = fs::read_dir(&dir) else { return vec![] };
    let mut out: Vec<String> = rd
        .flatten()
        .filter(|e| e.file_type().map(|t| t.is_dir() || t.is_symlink()).unwrap_or(false))
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            if !show_hidden && name.starts_with('.') && !prefix.starts_with('.') {
                return None;
            }
            if !name.to_lowercase().starts_with(&prefix) {
                return None;
            }
            let p = e.path();
            p.is_dir().then(|| path_str(&p))
        })
        .collect();
    out.sort_by_key(|s| s.to_lowercase());
    out.truncate(60);
    out
}

// ---------------------------------------------------------------------------
// Simple operations
// ---------------------------------------------------------------------------

pub fn check_name(name: &str) -> AppResult<()> {
    util::validate_file_name(name)
        .map_err(|why| AppError::new(ErrorCode::InvalidName, format!("Invalid name ({why})")))
}

pub fn make_dir(parent: &str, name: &str) -> AppResult<String> {
    // Allow "a/b/c" to create nested folders like Total Commander does.
    let parts: Vec<&str> = name.split(['/', '\\']).filter(|s| !s.is_empty()).collect();
    if parts.is_empty() {
        return Err(AppError::new(ErrorCode::InvalidName, "Empty name"));
    }
    for p in &parts {
        check_name(p)?;
    }
    let mut target = normalize(parent);
    for p in &parts {
        target.push(p);
    }
    if target.exists() {
        return Err(AppError::new(ErrorCode::AlreadyExists, "Already exists").with_path(&target));
    }
    fs::create_dir_all(&target).map_err(|e| AppError::io(e, &target))?;
    Ok(parts[0].to_string())
}

pub fn create_file(parent: &str, name: &str) -> AppResult<String> {
    check_name(name)?;
    let target = normalize(parent).join(name);
    fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&target)
        .map_err(|e| AppError::io(e, &target))?;
    Ok(path_str(&target))
}

pub fn rename(dir: &str, from: &str, to: &str) -> AppResult<()> {
    check_name(to)?;
    let d = normalize(dir);
    let src = d.join(from);
    let dst = d.join(to);
    let case_only = from != to && from.to_lowercase() == to.to_lowercase();
    if dst.exists() && !case_only {
        return Err(AppError::new(ErrorCode::AlreadyExists, "Target already exists").with_path(&dst));
    }
    if case_only && cfg!(any(windows, target_os = "macos")) {
        // Case-insensitive file systems need a detour for "readme" -> "README".
        let tmp = d.join(format!(".{}.dualion-rename", uuid::Uuid::new_v4()));
        fs::rename(&src, &tmp).map_err(|e| AppError::io(e, &src))?;
        return fs::rename(&tmp, &dst).map_err(|e| {
            let _ = fs::rename(&tmp, &src);
            AppError::io(e, &dst)
        });
    }
    fs::rename(&src, &dst).map_err(|e| AppError::io(e, &src))
}

pub fn create_link(target: &str, link_dir: &str, link_name: &str, hard: bool) -> AppResult<()> {
    check_name(link_name)?;
    let target_p = normalize(target);
    let link = normalize(link_dir).join(link_name);
    if hard {
        return fs::hard_link(&target_p, &link).map_err(|e| AppError::io(e, &link));
    }
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(&target_p, &link).map_err(|e| AppError::io(e, &link))
    }
    #[cfg(windows)]
    {
        let r = if target_p.is_dir() {
            std::os::windows::fs::symlink_dir(&target_p, &link)
        } else {
            std::os::windows::fs::symlink_file(&target_p, &link)
        };
        r.map_err(|e| AppError::io(e, &link))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lists_and_flags() {
        let dir = std::env::temp_dir().join(format!("dualion-test-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(dir.join("sub")).unwrap();
        fs::write(dir.join("a.txt"), b"hello").unwrap();
        fs::write(dir.join(".hidden"), b"").unwrap();
        let listing = list_local(&path_str(&dir)).unwrap();
        let get = |n: &str| listing.entries.iter().find(|e| e.name == n).unwrap().clone();
        assert!(get("sub").is_dir());
        assert_eq!(get("a.txt").size, 5);
        assert!(get(".hidden").flags & flags::HIDDEN != 0);
        assert!(listing.crumbs.len() >= 2);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn mkdir_nested_and_rename_case() {
        let dir = std::env::temp_dir().join(format!("dualion-test-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&dir).unwrap();
        let d = path_str(&dir);
        assert_eq!(make_dir(&d, "x/y/z").unwrap(), "x");
        assert!(dir.join("x/y/z").is_dir());
        assert!(make_dir(&d, "x").is_err());
        fs::write(dir.join("readme"), b"").unwrap();
        rename(&d, "readme", "README").unwrap();
        let names: Vec<String> = fs::read_dir(&dir).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
        assert!(names.contains(&"README".to_string()));
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn dir_size_counts() {
        let dir = std::env::temp_dir().join(format!("dualion-test-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(dir.join("a/b")).unwrap();
        fs::write(dir.join("a/1.bin"), vec![0u8; 100]).unwrap();
        fs::write(dir.join("a/b/2.bin"), vec![0u8; 50]).unwrap();
        let s = dir_size(&dir, &Arc::new(AtomicBool::new(false)));
        assert_eq!(s.bytes, 150);
        assert_eq!(s.files, 2);
        assert_eq!(s.dirs, 2);
        fs::remove_dir_all(&dir).unwrap();
    }
}
