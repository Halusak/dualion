//! Linux desktop integration (freedesktop standards): mounted volumes, file
//! icons from the user's icon theme, the terminal, the file manager's
//! properties window, disk images (udisks) and the SMB network (Avahi,
//! smbclient, GVFS). Missing tools only switch the matching feature off.

use crate::error::{AppError, AppResult, ErrorCode};
use crate::local::{space, Volume};
use parking_lot::Mutex;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::LazyLock;
use std::time::Duration;

// ---------------------------------------------------------------------------
// Volumes
// ---------------------------------------------------------------------------

/// File systems shown as drives (pseudo file systems are left out).
const REAL_FS: &[&str] = &[
    "ext2", "ext3", "ext4", "btrfs", "xfs", "zfs", "f2fs", "jfs", "reiserfs", "bcachefs", "vfat", "exfat", "ntfs", "ntfs3", "fuseblk", "hfsplus",
    "apfs", "iso9660", "udf", "nfs", "nfs4", "cifs", "smb3", "fuse.sshfs", "fuse.rclone", "9p", "virtiofs",
];

/// "\040" escapes in /proc/self/mounts.
fn unescape(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'\\' && i + 3 < b.len() && b[i + 1..i + 4].iter().all(|c| (b'0'..=b'7').contains(c)) {
            out.push(u8::from_str_radix(std::str::from_utf8(&b[i + 1..i + 4]).unwrap_or("0"), 8).unwrap_or(b'?'));
            i += 4;
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Device → file system label (/dev/disk/by-label).
fn labels() -> HashMap<PathBuf, String> {
    let mut m = HashMap::new();
    if let Ok(rd) = std::fs::read_dir("/dev/disk/by-label") {
        for e in rd.flatten() {
            if let Ok(dev) = std::fs::canonicalize(e.path()) {
                m.insert(dev, unescape(&e.file_name().to_string_lossy().replace("\\x20", " ")));
            }
        }
    }
    m
}

/// One /proc/self/mounts line → (device, mount point, fs type).
fn parse_mount(line: &str) -> Option<(String, String, String)> {
    let mut p = line.split_whitespace();
    Some((unescape(p.next()?), unescape(p.next()?), p.next()?.to_string()))
}

fn shown(mount: &str, fs: &str) -> bool {
    if !REAL_FS.contains(&fs) {
        return false;
    }
    if mount == "/" || mount.starts_with("/media/") || mount.starts_with("/run/media/") || mount.starts_with("/mnt/") || mount == "/home" {
        return true;
    }
    // System areas (boot partitions, snaps, containers…) are not places to browse.
    !["/boot", "/efi", "/snap", "/var", "/usr", "/opt", "/srv", "/tmp", "/proc", "/sys", "/dev", "/run"].iter().any(|s| mount == *s || mount.starts_with(&format!("{s}/")))
}

pub fn volumes() -> Vec<Volume> {
    let text = std::fs::read_to_string("/proc/self/mounts").unwrap_or_default();
    let labels = labels();
    let mut seen_dev: Vec<String> = Vec::new();
    let mut out = Vec::new();
    for line in text.lines() {
        let Some((dev, mount, fs)) = parse_mount(line) else { continue };
        if !shown(&mount, &fs) {
            continue;
        }
        // Bind mounts and btrfs subvolumes repeat a device: keep its first place.
        let local_dev = dev.starts_with("/dev/");
        if local_dev && mount != "/" && seen_dev.contains(&dev) {
            continue;
        }
        seen_dev.push(dev.clone());
        let kind = match fs.as_str() {
            _ if mount == "/" => "system",
            "nfs" | "nfs4" | "cifs" | "smb3" | "fuse.sshfs" | "fuse.rclone" | "9p" | "virtiofs" => "network",
            "iso9660" | "udf" => "cdrom",
            _ if mount.starts_with("/media/") || mount.starts_with("/run/media/") => "removable",
            _ => "fixed",
        };
        let label = std::fs::canonicalize(&dev).ok().and_then(|d| labels.get(&d).cloned());
        let name = label.unwrap_or_else(|| if mount == "/" { "/".into() } else { Path::new(&mount).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| mount.clone()) });
        let s = space(Path::new(&mount)).unwrap_or_default();
        out.push(Volume { name, path: mount, total: s.total, free: s.free, kind });
    }
    if out.is_empty() {
        let s = space(Path::new("/")).unwrap_or_default();
        out.push(Volume { name: "/".into(), path: "/".into(), total: s.total, free: s.free, kind: "system" });
    }
    out
}

/// Servers of mounted SMB / NFS shares ("//nas/share", "nas:/export").
pub fn network_servers() -> Vec<String> {
    let text = std::fs::read_to_string("/proc/self/mounts").unwrap_or_default();
    let mut out: Vec<String> = text
        .lines()
        .filter_map(parse_mount)
        .filter_map(|(dev, _, fs)| match fs.as_str() {
            "cifs" | "smb3" => dev.strip_prefix("//").and_then(|r| r.split('/').next()).map(str::to_string),
            "nfs" | "nfs4" => dev.split(':').next().map(str::to_string),
            _ => None,
        })
        .collect();
    // Shares connected through GVFS (file managers, our own connect).
    if let Ok(rd) = std::fs::read_dir(gvfs_dir()) {
        for e in rd.flatten() {
            let n = e.file_name().to_string_lossy().into_owned();
            if let Some(server) = n.strip_prefix("smb-share:server=").and_then(|r| r.split(',').next()) {
                out.push(server.to_string());
            }
        }
    }
    out.sort();
    out.dedup();
    out
}

fn gvfs_dir() -> PathBuf {
    PathBuf::from(format!("/run/user/{}/gvfs", unsafe { libc::getuid() }))
}

// ---------------------------------------------------------------------------
// Tools
// ---------------------------------------------------------------------------

fn which(cmd: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH").unwrap_or_default();
    std::env::split_paths(&path).chain(["/usr/bin", "/usr/local/bin", "/bin"].map(PathBuf::from)).map(|d| d.join(cmd)).find(|p| p.is_file())
}

/// Runs a tool with a timeout; stdout on success, stderr as the error.
fn run(cmd: &str, args: &[&str], timeout: Duration) -> AppResult<String> {
    let bin = which(cmd).ok_or_else(|| AppError::new(ErrorCode::ToolMissing, format!("{cmd} is not installed")))?;
    let mut c = Command::new(bin);
    c.args(args).stdin(Stdio::null());
    let out = crate::remote::tools::run(c, timeout)?;
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

fn spawn(c: &mut Command) -> AppResult<()> {
    c.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).spawn().map(|_| ()).map_err(|e| AppError::other(e.to_string()))
}

/// Opens the user's terminal in `dir` (custom command, $TERMINAL or the first one found).
pub fn open_terminal(dir: &str, custom: Option<&str>) -> AppResult<()> {
    if let Some(c) = custom.filter(|c| !c.trim().is_empty()) {
        return spawn(Command::new(c).current_dir(dir));
    }
    if let Ok(t) = std::env::var("TERMINAL") {
        if which(&t).is_some() {
            return spawn(Command::new(t).current_dir(dir));
        }
    }
    // (command, flag that takes the working directory)
    const KNOWN: &[(&str, Option<&str>)] = &[
        ("x-terminal-emulator", None),
        ("gnome-terminal", Some("--working-directory")),
        ("kgx", Some("--working-directory")),
        ("ptyxis", Some("--working-directory")),
        ("konsole", Some("--workdir")),
        ("xfce4-terminal", Some("--working-directory")),
        ("mate-terminal", Some("--working-directory")),
        ("tilix", Some("--working-directory")),
        ("kitty", Some("--directory")),
        ("alacritty", Some("--working-directory")),
        ("wezterm", None),
        ("foot", None),
        ("xterm", None),
    ];
    for (cmd, flag) in KNOWN {
        if let Some(bin) = which(cmd) {
            let mut c = Command::new(bin);
            if let Some(f) = flag {
                c.arg(format!("{f}={dir}"));
            }
            return spawn(c.current_dir(dir));
        }
    }
    Err(AppError::new(ErrorCode::ToolMissing, "No terminal found"))
}

fn file_uri(path: &str) -> String {
    const SET: &percent_encoding::AsciiSet = &percent_encoding::NON_ALPHANUMERIC.remove(b'/').remove(b'-').remove(b'_').remove(b'.').remove(b'~');
    format!("file://{}", percent_encoding::utf8_percent_encode(path, SET))
}

/// The file manager's properties window (org.freedesktop.FileManager1).
pub fn show_properties(path: &str) -> AppResult<()> {
    let uri = file_uri(path);
    run(
        "gdbus",
        &[
            "call",
            "--session",
            "--dest",
            "org.freedesktop.FileManager1",
            "--object-path",
            "/org/freedesktop/FileManager1",
            "--method",
            "org.freedesktop.FileManager1.ShowItemProperties",
            &format!("['{}']", uri.replace('\'', "%27")),
            "",
        ],
        Duration::from_secs(10),
    )
    .map(|_| ())
    .map_err(|_| AppError::unsupported("The file manager cannot show properties"))
}

/// Mounts a disk image with udisks (no root needed); returns the mount point.
pub fn mount_image(path: &str) -> AppResult<String> {
    let out = run("udisksctl", &["loop-setup", "--read-only", "--no-user-interaction", "-f", path], Duration::from_secs(30))?;
    // "Mapped file X as /dev/loop5."
    let dev = out.split_whitespace().find(|w| w.starts_with("/dev/loop")).map(|w| w.trim_end_matches('.').to_string()).ok_or_else(|| AppError::other(out.trim().to_string()))?;
    // Images with a partition table mount their first partition.
    for d in [dev.clone(), format!("{dev}p1")] {
        if let Ok(m) = run("udisksctl", &["mount", "--no-user-interaction", "-b", &d], Duration::from_secs(30)) {
            // "Mounted /dev/loop5 at /media/jan/LABEL"
            if let Some((_, at)) = m.trim().trim_end_matches('.').split_once(" at ") {
                return Ok(at.trim().to_string());
            }
        }
    }
    Err(AppError::other("The image contains no file system that can be mounted").with_path(path))
}

// ---------------------------------------------------------------------------
// SMB network
// ---------------------------------------------------------------------------

/// SMB servers announced via Avahi (mDNS), as host names.
pub fn smb_hosts() -> Vec<String> {
    let Ok(out) = run("avahi-browse", &["--terminate", "--resolve", "--parsable", "_smb._tcp"], Duration::from_secs(6)) else { return Vec::new() };
    parse_avahi(&out)
}

/// "=;eth0;IPv4;NAS;_smb._tcp;local;nas.local;192.168.1.5;445;" → "nas.local".
fn parse_avahi(out: &str) -> Vec<String> {
    let me = std::fs::read_to_string("/etc/hostname").map(|h| format!("{}.local", h.trim())).unwrap_or_default();
    let mut v: Vec<String> = out
        .lines()
        .filter(|l| l.starts_with('='))
        .filter_map(|l| l.split(';').nth(6).map(str::to_string))
        .filter(|h| !h.is_empty() && !h.eq_ignore_ascii_case(&me))
        .collect();
    v.sort();
    v.dedup();
    v
}

/// Disk shares of `host` (guest listing).
pub fn smb_shares(host: &str) -> AppResult<Vec<String>> {
    let out = run("smbclient", &["-L", &format!("//{host}"), "-N", "-g"], Duration::from_secs(15))
        .map_err(|e| if e.code == ErrorCode::ToolMissing { e } else { AppError::new(ErrorCode::PermissionDenied, "Sign in to see the shared folders").with_path(host) })?;
    Ok(parse_smbclient(&out))
}

/// "Disk|Fotky|Family photos" lines → share names (no `$` shares).
fn parse_smbclient(out: &str) -> Vec<String> {
    out.lines()
        .filter_map(|l| {
            let mut p = l.split('|');
            (p.next()? == "Disk").then(|| p.next().map(str::to_string)).flatten()
        })
        .filter(|n| !n.is_empty() && !n.ends_with('$'))
        .collect()
}

/// Connects `smb://host/share` through GVFS; returns its local folder.
pub fn smb_mount(host: &str, share: Option<&str>) -> AppResult<String> {
    let share = share.ok_or_else(|| AppError::unsupported("Open the computer's shares first").with_path(host))?;
    let dir = gvfs_dir().join(format!("smb-share:server={},share={}", host.to_lowercase(), share.to_lowercase()));
    if !dir.is_dir() {
        let url = format!("smb://{host}/{share}");
        run("gio", &["mount", &url], Duration::from_secs(60))
            .map_err(|e| if e.code == ErrorCode::ToolMissing { e } else { AppError::new(ErrorCode::PermissionDenied, format!("Connect {url} in your file manager first (sign-in needed)")) })?;
    }
    if dir.is_dir() {
        Ok(crate::util::path_str(&dir))
    } else {
        Err(AppError::other("The share was connected but its folder was not found").with_path(&dir))
    }
}

// ---------------------------------------------------------------------------
// File icons (freedesktop icon themes)
// ---------------------------------------------------------------------------

/// Icon theme of the desktop (GNOME / KDE settings), with fallbacks.
fn themes() -> Vec<String> {
    let mut v = Vec::new();
    if let Ok(out) = run("gsettings", &["get", "org.gnome.desktop.interface", "icon-theme"], Duration::from_secs(3)) {
        let t = out.trim().trim_matches('\'').to_string();
        if !t.is_empty() {
            v.push(t);
        }
    }
    if let Some(home) = dirs::home_dir() {
        if let Ok(kde) = std::fs::read_to_string(home.join(".config/kdeglobals")) {
            let mut in_icons = false;
            for l in kde.lines() {
                let l = l.trim();
                if l.starts_with('[') {
                    in_icons = l == "[Icons]";
                } else if in_icons {
                    if let Some(t) = l.strip_prefix("Theme=") {
                        v.push(t.trim().to_string());
                    }
                }
            }
        }
    }
    v.extend(["Adwaita", "breeze", "Papirus", "Yaru", "elementary", "hicolor"].map(String::from));
    v.dedup();
    v
}

fn icon_roots() -> Vec<PathBuf> {
    let mut v = Vec::new();
    if let Some(home) = dirs::home_dir() {
        v.push(home.join(".local/share/icons"));
        v.push(home.join(".icons"));
    }
    let data = std::env::var("XDG_DATA_DIRS").unwrap_or_else(|_| "/usr/local/share:/usr/share".into());
    v.extend(data.split(':').filter(|d| !d.is_empty()).map(|d| PathBuf::from(d).join("icons")));
    v
}

/// One icon file of a theme: size in px (0 = scalable) and path.
type Found = (u32, PathBuf);

/// All icons of the contexts we use, per theme (built once, in the icon thread).
static INDEX: LazyLock<Vec<HashMap<String, Vec<Found>>>> = LazyLock::new(|| {
    let roots = icon_roots();
    let mut order: Vec<String> = Vec::new();
    for t in themes() {
        // Inherited themes come right after their child.
        let mut stack = vec![t];
        while let Some(t) = stack.pop() {
            if order.contains(&t) {
                continue;
            }
            for r in &roots {
                if let Ok(idx) = std::fs::read_to_string(r.join(&t).join("index.theme")) {
                    if let Some(l) = idx.lines().find(|l| l.starts_with("Inherits=")) {
                        stack.extend(l["Inherits=".len()..].split(',').rev().map(|s| s.trim().to_string()).filter(|s| !s.is_empty()));
                    }
                    break;
                }
            }
            order.push(t);
        }
    }
    order.iter().map(|t| index_theme(&roots, t)).filter(|m| !m.is_empty()).collect()
});

fn index_theme(roots: &[PathBuf], theme: &str) -> HashMap<String, Vec<Found>> {
    let mut m: HashMap<String, Vec<Found>> = HashMap::new();
    for r in roots {
        let base = r.join(theme);
        if !base.is_dir() {
            continue;
        }
        for e in walkdir_shallow(&base, 3) {
            let p = e.to_string_lossy();
            if !(p.contains("mimetypes") || p.contains("places") || p.contains("devices")) || p.contains("symbolic") {
                continue;
            }
            let Some(ext) = e.extension().and_then(|x| x.to_str()) else { continue };
            if ext != "png" && ext != "svg" {
                continue;
            }
            let name = e.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
            let size = if ext == "svg" { 0 } else { size_from_path(&e).unwrap_or(48) };
            m.entry(name).or_default().push((size, e));
        }
    }
    m
}

/// "…/48x48/mimetypes/x.png", "…/mimetypes/48/x.png" → 48.
fn size_from_path(p: &Path) -> Option<u32> {
    p.components().rev().skip(1).take(3).find_map(|c| {
        let s = c.as_os_str().to_string_lossy();
        let s = s.split('@').next().unwrap_or(&s);
        s.split('x').next().and_then(|n| n.parse().ok())
    })
}

/// Files up to `depth` levels below `dir`.
fn walkdir_shallow(dir: &Path, depth: usize) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![(dir.to_path_buf(), 0usize)];
    while let Some((d, level)) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&d) else { continue };
        for e in rd.flatten() {
            let p = e.path();
            let Ok(ft) = e.file_type() else { continue };
            if ft.is_dir() || (ft.is_symlink() && p.is_dir()) {
                if level < depth {
                    stack.push((p, level + 1));
                }
            } else {
                out.push(p);
            }
        }
    }
    out
}

/// Extension → MIME type (shared-mime-info globs).
static GLOBS: LazyLock<HashMap<String, String>> = LazyLock::new(|| {
    let mut m = HashMap::new();
    let data = std::env::var("XDG_DATA_DIRS").unwrap_or_else(|_| "/usr/local/share:/usr/share".into());
    for d in data.split(':').rev() {
        let Ok(text) = std::fs::read_to_string(Path::new(d).join("mime/globs2")) else { continue };
        for l in text.lines() {
            // "weight:mime:glob[:flags]"
            let mut p = l.split(':');
            let (_, Some(mime), Some(glob)) = (p.next(), p.next(), p.next()) else { continue };
            if let Some(ext) = glob.strip_prefix("*.") {
                if !ext.contains(['*', '?', '[']) {
                    m.entry(ext.to_lowercase()).or_insert_with(|| mime.to_string());
                }
            }
        }
    }
    m
});

/// MIME type → generic icon (shared-mime-info generic-icons).
static GENERIC: LazyLock<HashMap<String, String>> = LazyLock::new(|| {
    let mut m = HashMap::new();
    for d in ["/usr/share", "/usr/local/share"] {
        if let Ok(text) = std::fs::read_to_string(Path::new(d).join("mime/generic-icons")) {
            for l in text.lines() {
                if let Some((mime, icon)) = l.split_once(':') {
                    m.insert(mime.to_string(), icon.to_string());
                }
            }
        }
    }
    m
});

fn names_for_ext(ext: &str) -> Vec<String> {
    let mut v = Vec::new();
    if let Some(mime) = GLOBS.get(&ext.to_lowercase()) {
        v.push(mime.replace('/', "-"));
        if let Some(g) = GENERIC.get(mime) {
            v.push(g.clone());
        }
        if let Some(top) = mime.split('/').next() {
            v.push(format!("{top}-x-generic"));
        }
    }
    v.push("text-x-generic".into());
    v.push("application-x-generic".into());
    v
}

fn names_for_path(path: &str) -> Vec<String> {
    let p = Path::new(path);
    if p.is_dir() {
        // Drives and special folders.
        if let Some(v) = volumes().into_iter().find(|v| v.path == path) {
            let n = match v.kind {
                "removable" => "drive-removable-media",
                "cdrom" => "media-optical",
                "network" => "folder-remote",
                _ => "drive-harddisk",
            };
            return vec![n.into(), "drive-harddisk".into(), "folder".into()];
        }
        let special = [
            (dirs::home_dir(), "user-home"),
            (dirs::desktop_dir(), "user-desktop"),
            (dirs::document_dir(), "folder-documents"),
            (dirs::download_dir(), "folder-download"),
            (dirs::picture_dir(), "folder-pictures"),
            (dirs::audio_dir(), "folder-music"),
            (dirs::video_dir(), "folder-videos"),
        ];
        let mut v: Vec<String> = special.iter().filter(|(d, _)| d.as_deref() == Some(p)).map(|(_, n)| n.to_string()).collect();
        v.push("folder".into());
        return v;
    }
    names_for_ext(p.extension().and_then(|e| e.to_str()).unwrap_or(""))
}

/// (icon name, px) → chosen file.
type Chosen = HashMap<(String, u32), Option<PathBuf>>;
static CHOSEN: LazyLock<Mutex<Chosen>> = LazyLock::new(|| Mutex::new(HashMap::new()));

/// Best file for an icon name at `px`: scalable first, else the nearest size.
fn lookup(name: &str, px: u32) -> Option<PathBuf> {
    let key = (name.to_string(), px);
    if let Some(hit) = CHOSEN.lock().get(&key) {
        return hit.clone();
    }
    let found = INDEX.iter().find_map(|theme| {
        let list = theme.get(name)?;
        list.iter()
            .min_by_key(|(s, _)| if *s == 0 { 0 } else if *s >= px { (s - px) as i64 + 1 } else { 1000 + (px - s) as i64 })
            .map(|(_, p)| p.clone())
    });
    CHOSEN.lock().insert(key, found.clone());
    found
}

/// Icon bytes (PNG or SVG) for the icon protocol.
pub fn render_icon(key: &crate::icons::IconKey, px: u32) -> Option<Vec<u8>> {
    use crate::icons::IconKey;
    let names = match key {
        IconKey::Dir => vec!["folder".to_string()],
        IconKey::File => vec!["text-x-generic".to_string(), "application-x-generic".to_string()],
        IconKey::Ext(e) => names_for_ext(e),
        IconKey::Path(p) => names_for_path(p),
    };
    names.iter().find_map(|n| lookup(n, px)).and_then(|p| std::fs::read(p).ok())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_mounts() {
        assert_eq!(parse_mount("/dev/sdb1 /media/jan/USB\\040DISK vfat rw 0 0"), Some(("/dev/sdb1".into(), "/media/jan/USB DISK".into(), "vfat".into())));
        assert!(shown("/", "ext4") && shown("/media/jan/USB", "vfat") && shown("/data", "xfs"));
        assert!(!shown("/boot/efi", "vfat") && !shown("/proc", "proc") && !shown("/snap/core/1", "squashfs") && !shown("/run/user/1000", "tmpfs"));
    }

    #[test]
    fn parses_tools() {
        let avahi = "+;eth0;IPv4;NAS;_smb._tcp;local\n=;eth0;IPv4;NAS;_smb._tcp;local;nas.local;192.168.1.5;445;\n=;eth0;IPv6;NAS;_smb._tcp;local;nas.local;fe80::1;445;\n";
        assert_eq!(parse_avahi(avahi), vec!["nas.local"]);
        let smb = "Disk|Fotky|Family photos\nIPC|IPC$|IPC Service\nDisk|print$|Printer Drivers\nPrinter|hp|\n";
        assert_eq!(parse_smbclient(smb), vec!["Fotky"]);
        assert_eq!(size_from_path(Path::new("/usr/share/icons/hicolor/48x48/mimetypes/x.png")), Some(48));
        assert_eq!(size_from_path(Path::new("/usr/share/icons/breeze/mimetypes/32/x.svg")), Some(32));
        assert_eq!(file_uri("/home/jan/My file.txt"), "file:///home/jan/My%20file.txt");
    }
}
