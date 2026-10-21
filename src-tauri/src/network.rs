//! "Network" place like Total Commander's Network Neighborhood: computers on
//! the local network → their shared folders → connecting a share.
//!
//! Windows: computers from the shell's Network folder (what Explorer shows),
//! shares via NetShareEnum, credentials via the system dialog (WNet).
//! macOS: computers via Bonjour (`_smb._tcp`), shares via `smbutil view`,
//! mounting via NetFS (the same dialogs as in Finder).
//!
//! Like Total Commander, the place opens at once with what is already known
//! (computers found last time, servers of connected drives / volumes and,
//! on Windows, the Network shortcuts); discovery runs in the background and
//! the panel refreshes when it finds something new.

use crate::error::{AppError, AppResult, ErrorCode};
use crate::location::{flags, Crumb, Entry, Listing, Location};
use parking_lot::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

/// Computers found by the last discovery (also kept on disk).
static KNOWN: Mutex<Option<Vec<String>>> = Mutex::new(None);
static LAST_SCAN: Mutex<Option<Instant>> = Mutex::new(None);
static SCANNING: AtomicBool = AtomicBool::new(false);
static NOTIFY: OnceLock<Box<dyn Fn() + Send + Sync>> = OnceLock::new();

/// Called when background discovery changed the list of computers.
pub fn set_notify(f: impl Fn() + Send + Sync + 'static) {
    let _ = NOTIFY.set(Box::new(f));
}

fn cache_file() -> std::path::PathBuf {
    crate::platform::data_dir().join("network-hosts.json")
}

fn known() -> Vec<String> {
    let mut k = KNOWN.lock();
    if k.is_none() {
        *k = Some(std::fs::read(cache_file()).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default());
    }
    k.clone().unwrap_or_default()
}

/// Looks for computers in the background (at most every 30 s).
fn scan_in_background() {
    if LAST_SCAN.lock().is_some_and(|t| t.elapsed() < Duration::from_secs(30)) || SCANNING.swap(true, Ordering::SeqCst) {
        return;
    }
    std::thread::Builder::new()
        .name("network-scan".into())
        .spawn(|| {
            let mut found = hosts();
            found.sort_by_key(|n| n.to_lowercase());
            found.dedup();
            let changed = known() != found;
            if changed {
                let _ = std::fs::write(cache_file(), serde_json::to_vec(&found).unwrap_or_default());
                *KNOWN.lock() = Some(found);
            }
            *LAST_SCAN.lock() = Some(Instant::now());
            SCANNING.store(false, Ordering::SeqCst);
            if changed {
                if let Some(n) = NOTIFY.get() {
                    n();
                }
            }
        })
        .ok();
}

pub fn list(host: Option<&str>) -> AppResult<Listing> {
    let root = Crumb { label: String::new(), icon: "network", location: Location::Network { host: None } };
    match host {
        None => {
            // Known at once: last discovery, servers in use, network shortcuts ("\\server\share").
            let mut names = known();
            names.extend(quick_places());
            names.sort_by_key(|n| (n.starts_with("\\\\"), n.to_lowercase()));
            names.dedup_by(|a, b| a.eq_ignore_ascii_case(b));
            scan_in_background();
            Ok(Listing {
                location: Location::Network { host: None },
                display_path: String::new(),
                parent: None,
                crumbs: vec![root],
                entries: names.into_iter().map(|n| Entry { name: n, flags: flags::DIR, size: 0, mtime: 0, attrs: 0 }).collect(),
            })
        }
        Some(h) => {
            let mut list = shares(h)?;
            list.sort_by_key(|n| n.to_lowercase());
            let loc = Location::Network { host: Some(h.to_string()) };
            Ok(Listing {
                location: loc.clone(),
                display_path: format!("\\\\{h}"),
                parent: Some(Location::Network { host: None }),
                crumbs: vec![root, Crumb { label: h.to_string(), icon: "server", location: loc }],
                entries: list.into_iter().map(|n| Entry { name: n, flags: flags::DIR, size: 0, mtime: 0, attrs: 0 }).collect(),
            })
        }
    }
}

fn auth_needed(host: &str) -> AppError {
    AppError::new(ErrorCode::PermissionDenied, "Sign in to see the shared folders").with_path(host)
}

/// "//user@nas._smb._tcp.local/Share" → "nas.local".
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn server_of_mount(from: &str) -> Option<String> {
    let rest = from.strip_prefix("//")?;
    let rest = rest.rsplit_once('@').map_or(rest, |(_, r)| r);
    let server = percent_encoding::percent_decode_str(rest.split('/').next()?).decode_utf8_lossy().into_owned();
    let server = server.replace("._smb._tcp", "").replace("._afpovertcp._tcp", "");
    (!server.is_empty()).then_some(server)
}

/// Lines of `smbutil view` → disk share names (no hidden `$` shares).
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn parse_smbutil_view(out: &str) -> Vec<String> {
    let mut lines = out.lines();
    let Some(header) = lines.by_ref().find(|l| l.trim_start().starts_with("Share")) else { return Vec::new() };
    let Some(type_col) = header.find("Type") else { return Vec::new() };
    let mut shares = Vec::new();
    for l in lines {
        if l.starts_with('-') || l.trim().is_empty() || l.ends_with("listed") {
            continue;
        }
        let (name, rest) = if l.len() > type_col && l.is_char_boundary(type_col) { l.split_at(type_col) } else { continue };
        let name = name.trim_end();
        if rest.trim_start().starts_with("Disk") && !name.is_empty() && !name.ends_with('$') {
            shares.push(name.to_string());
        }
    }
    shares
}

// ---------------------------------------------------------------------------
// macOS
// ---------------------------------------------------------------------------

#[cfg(target_os = "macos")]
mod imp {
    use super::*;
    use std::ffi::{c_char, c_void, CStr, CString};
    use std::time::{Duration, Instant};

    type DnsRef = *mut c_void;
    type BrowseReply = extern "C" fn(DnsRef, u32, u32, i32, *const c_char, *const c_char, *const c_char, *mut c_void);
    type ResolveReply = extern "C" fn(DnsRef, u32, u32, i32, *const c_char, *const c_char, u16, u16, *const u8, *mut c_void);

    extern "C" {
        fn DNSServiceBrowse(r: *mut DnsRef, flags: u32, iface: u32, regtype: *const c_char, domain: *const c_char, cb: BrowseReply, ctx: *mut c_void) -> i32;
        fn DNSServiceResolve(
            r: *mut DnsRef,
            flags: u32,
            iface: u32,
            name: *const c_char,
            regtype: *const c_char,
            domain: *const c_char,
            cb: ResolveReply,
            ctx: *mut c_void,
        ) -> i32;
        fn DNSServiceRefSockFD(r: DnsRef) -> i32;
        fn DNSServiceProcessResult(r: DnsRef) -> i32;
        fn DNSServiceRefDeallocate(r: DnsRef);
    }

    #[link(name = "NetFS", kind = "framework")]
    extern "C" {
        fn NetFSMountURLSync(
            url: *const c_void,
            mountpath: *const c_void,
            user: *const c_void,
            passwd: *const c_void,
            open_options: *const c_void,
            mount_options: *const c_void,
            mountpoints: *mut *const c_void,
        ) -> i32;
    }

    const FLAG_ADD: u32 = 0x2;

    /// Runs the DNS-SD event loop of `r` until `until` (or `done()`).
    fn pump(r: DnsRef, until: Instant, done: &dyn Fn() -> bool) {
        let fd = unsafe { DNSServiceRefSockFD(r) };
        if fd < 0 {
            return;
        }
        while !done() {
            let left = until.saturating_duration_since(Instant::now());
            if left.is_zero() {
                break;
            }
            let mut p = libc::pollfd { fd, events: libc::POLLIN, revents: 0 };
            let n = unsafe { libc::poll(&mut p, 1, left.as_millis().min(i32::MAX as u128) as i32) };
            if n <= 0 || unsafe { DNSServiceProcessResult(r) } != 0 {
                break;
            }
        }
    }

    struct Found {
        name: String,
        iface: u32,
        domain: String,
    }

    extern "C" fn on_browse(_r: DnsRef, flags: u32, iface: u32, err: i32, name: *const c_char, _t: *const c_char, domain: *const c_char, ctx: *mut c_void) {
        if err != 0 || flags & FLAG_ADD == 0 || name.is_null() {
            return;
        }
        let out = unsafe { &mut *(ctx as *mut Vec<Found>) };
        let name = unsafe { CStr::from_ptr(name) }.to_string_lossy().into_owned();
        let domain = if domain.is_null() { "local.".into() } else { unsafe { CStr::from_ptr(domain) }.to_string_lossy().into_owned() };
        if !out.iter().any(|f| f.name == name) {
            out.push(Found { name, iface, domain });
        }
    }

    extern "C" fn on_resolve(_r: DnsRef, _f: u32, _i: u32, err: i32, _full: *const c_char, host: *const c_char, _p: u16, _l: u16, _t: *const u8, ctx: *mut c_void) {
        if err == 0 && !host.is_null() {
            let out = unsafe { &mut *(ctx as *mut Option<String>) };
            *out = Some(unsafe { CStr::from_ptr(host) }.to_string_lossy().trim_end_matches('.').to_string());
        }
    }

    /// SMB servers announced via Bonjour, as host names (`nas.local`).
    pub fn hosts() -> Vec<String> {
        let mut found: Vec<Found> = Vec::new();
        let mut r: DnsRef = std::ptr::null_mut();
        let ty = CString::new("_smb._tcp").unwrap();
        let err = unsafe { DNSServiceBrowse(&mut r, 0, 0, ty.as_ptr(), std::ptr::null(), on_browse, &mut found as *mut _ as *mut c_void) };
        if err != 0 || r.is_null() {
            return Vec::new();
        }
        pump(r, Instant::now() + Duration::from_millis(1500), &|| false);
        unsafe { DNSServiceRefDeallocate(r) };

        let me = hostname();
        // Resolve all services at once (each may take up to a second).
        let jobs: Vec<_> = found.into_iter().map(|f| std::thread::spawn(move || resolve(f))).collect();
        let mut out = Vec::new();
        for h in jobs.into_iter().filter_map(|j| j.join().ok()) {
            // This Mac's own sharing is not a place to go to.
            if me.as_deref().is_some_and(|m| h.eq_ignore_ascii_case(m)) || out.contains(&h) {
                continue;
            }
            out.push(h);
        }
        out
    }

    fn resolve(f: Found) -> String {
        let ty = CString::new("_smb._tcp").unwrap();
        let mut host: Option<String> = None;
        let mut rr: DnsRef = std::ptr::null_mut();
        if let (Ok(name), Ok(domain)) = (CString::new(f.name.clone()), CString::new(f.domain.clone())) {
            let err = unsafe { DNSServiceResolve(&mut rr, 0, f.iface, name.as_ptr(), ty.as_ptr(), domain.as_ptr(), on_resolve, &mut host as *mut _ as *mut c_void) };
            if err == 0 && !rr.is_null() {
                let h = &host as *const Option<String>;
                pump(rr, Instant::now() + Duration::from_millis(1000), &|| unsafe { (*h).is_some() });
                unsafe { DNSServiceRefDeallocate(rr) };
            }
        }
        host.unwrap_or_else(|| format!("{}.local", f.name.replace(' ', "-")))
    }

    /// Servers of mounted SMB / AFP volumes.
    pub fn quick_places() -> Vec<String> {
        let mut buf: *mut libc::statfs = std::ptr::null_mut();
        let n = unsafe { libc::getmntinfo(&mut buf, libc::MNT_NOWAIT) };
        if n <= 0 || buf.is_null() {
            return Vec::new();
        }
        let all = unsafe { std::slice::from_raw_parts(buf, n as usize) };
        all.iter()
            .filter_map(|m| {
                let fs = unsafe { CStr::from_ptr(m.f_fstypename.as_ptr()) }.to_string_lossy().into_owned();
                if fs != "smbfs" && fs != "afpfs" {
                    return None;
                }
                let from = unsafe { CStr::from_ptr(m.f_mntfromname.as_ptr()) }.to_string_lossy().into_owned();
                server_of_mount(&from)
            })
            .collect()
    }

    fn hostname() -> Option<String> {
        let o = std::process::Command::new("scutil").args(["--get", "LocalHostName"]).output().ok()?;
        let n = String::from_utf8_lossy(&o.stdout).trim().to_string();
        (!n.is_empty()).then(|| format!("{n}.local"))
    }

    pub fn shares(host: &str) -> AppResult<Vec<String>> {
        let out = std::process::Command::new("smbutil")
            .args(["view", "-g", &format!("//{host}")])
            .output()
            .map_err(|e| AppError::io(e, host))?;
        if !out.status.success() {
            return Err(auth_needed(host));
        }
        Ok(parse_smbutil_view(&String::from_utf8_lossy(&out.stdout)))
    }

    /// Mounts `smb://host/share` (or lets the user pick shares when `share`
    /// is None) with Finder's dialogs; returns the mount point.
    pub fn mount(host: &str, share: Option<&str>) -> AppResult<String> {
        use objc2::rc::Retained;
        use objc2_foundation::{NSArray, NSString, NSURL};
        const SEG: &percent_encoding::AsciiSet = &percent_encoding::NON_ALPHANUMERIC.remove(b'-').remove(b'.').remove(b'_').remove(b'~');
        let mut url = format!("smb://{host}");
        if let Some(s) = share {
            url.push('/');
            url.push_str(&percent_encoding::utf8_percent_encode(s, SEG).to_string());
        }
        if let Some(existing) = share.and_then(|s| mounted(host, s)) {
            return Ok(existing);
        }
        let ns = NSURL::URLWithString(&NSString::from_str(&url)).ok_or_else(|| AppError::invalid("Bad server address"))?;
        let mut points: *const c_void = std::ptr::null();
        let null = std::ptr::null();
        let rc = unsafe { NetFSMountURLSync(Retained::as_ptr(&ns) as *const c_void, null, null, null, null, null, &mut points) };
        let first = if points.is_null() {
            None
        } else {
            // The array is ours (Create rule); NSArray is toll-free bridged with CFArray.
            let arr: Retained<NSArray<NSString>> = unsafe { Retained::from_raw(points as *mut NSArray<NSString>) }.ok_or_else(|| AppError::other("No mount point"))?;
            arr.iter().next().map(|s| s.to_string())
        };
        match (rc, first) {
            (0, Some(p)) => Ok(p),
            (libc::EEXIST, _) => share.and_then(|s| mounted(host, s)).ok_or_else(|| AppError::other("Already connected")),
            (libc::ECANCELED, _) | (-128, _) => Err(AppError::cancelled()),
            (libc::EAUTH, _) | (libc::EACCES, _) | (libc::EPERM, _) => Err(auth_needed(host)),
            (rc, _) => Err(AppError::other(format!("Cannot connect to {url} (error {rc})"))),
        }
    }

    /// Mount point of an already connected share.
    fn mounted(host: &str, share: &str) -> Option<String> {
        let mut buf: *mut libc::statfs = std::ptr::null_mut();
        let n = unsafe { libc::getmntinfo(&mut buf, libc::MNT_NOWAIT) };
        if n <= 0 || buf.is_null() {
            return None;
        }
        let host = host.trim_end_matches(".local").to_lowercase();
        let all = unsafe { std::slice::from_raw_parts(buf, n as usize) };
        all.iter().find_map(|m| {
            let from = unsafe { CStr::from_ptr(m.f_mntfromname.as_ptr()) }.to_string_lossy().to_lowercase();
            let from = percent_encoding::percent_decode_str(&from).decode_utf8_lossy().into_owned();
            // "//user@nas._smb._tcp.local/Share" or "//nas.local/Share"
            let rest = from.strip_prefix("//")?;
            let rest = rest.rsplit_once('@').map_or(rest, |(_, r)| r);
            let (server, path) = rest.split_once('/')?;
            let server = server.split('.').next()?;
            (server == host && path.eq_ignore_ascii_case(share)).then(|| unsafe { CStr::from_ptr(m.f_mntonname.as_ptr()) }.to_string_lossy().into_owned())
        })
    }
}

// ---------------------------------------------------------------------------
// Windows
// ---------------------------------------------------------------------------

#[cfg(windows)]
mod imp {
    use super::*;
    use windows::core::{HSTRING, PCWSTR, PWSTR};
    use windows::Win32::Foundation::HWND;
    use windows::Win32::NetworkManagement::NetManagement::{NetApiBufferFree, MAX_PREFERRED_LENGTH};
    use windows::Win32::NetworkManagement::WNet::{WNetAddConnection3W, CONNECT_INTERACTIVE, NETRESOURCEW, RESOURCETYPE_DISK};
    use windows::Win32::Storage::FileSystem::{NetShareEnum, SHARE_INFO_1, STYPE_DISKTREE};
    use windows::Win32::System::Com::{CoInitializeEx, CoTaskMemFree, IBindCtx, COINIT_APARTMENTTHREADED};
    use windows::Win32::UI::Shell::Common::{ITEMIDLIST, STRRET};
    use windows::Win32::UI::Shell::{
        IEnumIDList, ILFree, IShellFolder, SHGetDesktopFolder, SHGetKnownFolderIDList, StrRetToStrW, FOLDERID_NetworkFolder, SHCONTF_FOLDERS,
        SHGDN_FORPARSING,
    };

    /// Servers of connected network drives and the user's Network shortcuts
    /// (as `\\\\server\\share`).
    pub fn quick_places() -> Vec<String> {
        use windows::Win32::NetworkManagement::WNet::WNetGetConnectionW;
        let mut out = Vec::new();
        for v in crate::local::volumes().into_iter().filter(|v| v.kind == "network") {
            let letter = HSTRING::from(v.path.trim_end_matches('\\'));
            let mut buf = [0u16; 512];
            let mut len = buf.len() as u32;
            if unsafe { WNetGetConnectionW(&letter, Some(PWSTR(buf.as_mut_ptr())), &mut len) }.0 == 0 {
                let unc = String::from_utf16_lossy(&buf[..buf.iter().position(|&c| c == 0).unwrap_or(0)]);
                if let Some(host) = unc.strip_prefix("\\\\").and_then(|r| r.split('\\').next()) {
                    out.push(host.to_string());
                }
            }
        }
        out.extend(network_shortcuts());
        out
    }

    /// Targets of the shortcuts in "Network shortcuts" (This PC › Network locations).
    fn network_shortcuts() -> Vec<String> {
        use windows::Win32::System::SystemServices::SFGAO_LINK;
        use windows::Win32::UI::Shell::{IEnumShellItems, IShellItem, SHCreateItemFromIDList, BHID_EnumItems, BHID_LinkTargetItem, FOLDERID_NetHood, SIGDN_DESKTOPABSOLUTEPARSING};
        let _ = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) };
        let mut out = Vec::new();
        unsafe {
            let Ok(pidl) = SHGetKnownFolderIDList(&FOLDERID_NetHood, 0, None) else { return out };
            let folder: windows::core::Result<IShellItem> = SHCreateItemFromIDList(pidl);
            ILFree(Some(pidl));
            let Ok(folder) = folder else { return out };
            let Ok(en) = folder.BindToHandler::<_, IEnumShellItems>(None::<&IBindCtx>, &BHID_EnumItems) else { return out };
            loop {
                let mut one = [None];
                let mut got = 0u32;
                if en.Next(&mut one, Some(&mut got)).is_err() || got == 0 {
                    break;
                }
                let Some(item) = one[0].take() else { continue };
                let link = item.GetAttributes(SFGAO_LINK).map(|a| a.0 & SFGAO_LINK.0 != 0).unwrap_or(false);
                let target = if link { item.BindToHandler::<_, IShellItem>(None::<&IBindCtx>, &BHID_LinkTargetItem).unwrap_or(item) } else { item };
                if let Ok(p) = target.GetDisplayName(SIGDN_DESKTOPABSOLUTEPARSING) {
                    let s = p.to_string().unwrap_or_default();
                    CoTaskMemFree(Some(p.0 as *const _));
                    if s.starts_with("\\\\") {
                        out.push(s.trim_end_matches('\\').to_string());
                    }
                }
            }
        }
        out
    }

    /// Computers in Explorer's Network folder (WS-Discovery + NetBIOS).
    pub fn hosts() -> Vec<String> {
        let _ = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) };
        let mut out = Vec::new();
        unsafe {
            let Ok(pidl) = SHGetKnownFolderIDList(&FOLDERID_NetworkFolder, 0, None) else { return out };
            let folder: windows::core::Result<IShellFolder> = SHGetDesktopFolder().and_then(|d| d.BindToObject(pidl, None::<&IBindCtx>));
            ILFree(Some(pidl));
            let Ok(folder) = folder else { return out };
            let mut en: Option<IEnumIDList> = None;
            if folder.EnumObjects(HWND::default(), SHCONTF_FOLDERS.0 as u32, &mut en).is_err() {
                return out;
            }
            let Some(en) = en else { return out };
            loop {
                let mut item = [std::ptr::null_mut::<ITEMIDLIST>()];
                let mut got = 0u32;
                if en.Next(&mut item, Some(&mut got)).is_err() || got == 0 {
                    break;
                }
                let mut sr = STRRET::default();
                if folder.GetDisplayNameOf(item[0], SHGDN_FORPARSING, &mut sr).is_ok() {
                    let mut s = PWSTR::null();
                    if StrRetToStrW(&mut sr, Some(item[0]), &mut s).is_ok() && !s.is_null() {
                        let name = s.to_string().unwrap_or_default();
                        CoTaskMemFree(Some(s.0 as *const _));
                        // Computers parse as "\\NAME"; devices (media servers, printers) do not.
                        if let Some(h) = name.strip_prefix("\\\\") {
                            if !h.is_empty() && !h.contains('\\') {
                                out.push(h.to_string());
                            }
                        }
                    }
                }
                ILFree(Some(item[0]));
            }
        }
        out
    }

    pub fn shares(host: &str) -> AppResult<Vec<String>> {
        let server = HSTRING::from(format!("\\\\{host}"));
        let mut buf: *mut u8 = std::ptr::null_mut();
        let (mut read, mut total) = (0u32, 0u32);
        let rc = unsafe { NetShareEnum(&server, 1, &mut buf, MAX_PREFERRED_LENGTH, &mut read, &mut total, None) };
        if rc != 0 {
            if !buf.is_null() {
                unsafe { NetApiBufferFree(Some(buf as *const _)) };
            }
            // 5 = access denied, 1326 = bad credentials, 86 = invalid password
            return Err(if matches!(rc, 5 | 1326 | 86 | 1244) { auth_needed(host) } else { AppError::other(format!("Cannot list shares of {host} (error {rc})")) });
        }
        let mut out = Vec::new();
        if !buf.is_null() {
            let items = unsafe { std::slice::from_raw_parts(buf as *const SHARE_INFO_1, read as usize) };
            for i in items {
                if i.shi1_type.0 & 0xFF != STYPE_DISKTREE.0 || i.shi1_type.0 & 0x8000_0000 != 0 {
                    continue;
                }
                let name = unsafe { i.shi1_netname.to_string() }.unwrap_or_default();
                if !name.is_empty() && !name.ends_with('$') {
                    out.push(name);
                }
            }
            unsafe { NetApiBufferFree(Some(buf as *const _)) };
        }
        Ok(out)
    }

    /// Connects `\\host\share` (or the host's IPC$ to sign in) with the
    /// system credential dialog when needed; returns the UNC path.
    pub fn connect(hwnd: HWND, host: &str, share: Option<&str>) -> AppResult<String> {
        let unc = match share {
            Some(s) => format!("\\\\{host}\\{s}"),
            None => format!("\\\\{host}\\IPC$"),
        };
        if share.is_some() && std::fs::read_dir(&unc).is_ok() {
            return Ok(unc);
        }
        let mut remote: Vec<u16> = unc.encode_utf16().chain(std::iter::once(0)).collect();
        let nr = NETRESOURCEW { dwType: RESOURCETYPE_DISK, lpRemoteName: PWSTR(remote.as_mut_ptr()), ..Default::default() };
        let nr = if share.is_none() { NETRESOURCEW { dwType: Default::default(), ..nr } } else { nr };
        let rc = unsafe { WNetAddConnection3W(Some(hwnd), &nr, PCWSTR::null(), PCWSTR::null(), CONNECT_INTERACTIVE) };
        match rc.0 {
            0 | 1219 => Ok(unc), // 1219: already connected with other credentials
            1223 => Err(AppError::cancelled()),
            e => Err(AppError::other(format!("Cannot connect to {unc} (error {e})"))),
        }
    }
}

#[cfg(not(any(target_os = "macos", windows)))]
mod imp {
    use super::*;
    pub fn hosts() -> Vec<String> {
        Vec::new()
    }
    pub fn quick_places() -> Vec<String> {
        Vec::new()
    }
    pub fn shares(host: &str) -> AppResult<Vec<String>> {
        Err(auth_needed(host))
    }
}

pub use imp::*;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn servers_of_mounts() {
        assert_eq!(server_of_mount("//jan@NAS._smb._tcp.local/Fotky").as_deref(), Some("NAS.local"));
        assert_eq!(server_of_mount("//nas.lan/Share").as_deref(), Some("nas.lan"));
        assert_eq!(server_of_mount("/dev/disk3s1"), None);
    }

    #[test]
    fn parses_smbutil_view() {
        let out = "Share                                           Type    Comments\n\
                   -------------------------------\n\
                   IPC$                                            Pipe    Remote IPC\n\
                   Public                                          Disk    \n\
                   Photo Archive                                   Disk    Family photos\n\
                   ADMIN$                                          Disk    Remote Admin\n\
                   printer                                         Printer \n\
                   \n\
                   5 shares listed\n";
        assert_eq!(parse_smbutil_view(out), vec!["Public", "Photo Archive"]);
        assert!(parse_smbutil_view("smbutil: server connection failed").is_empty());
    }

    /// Browses for a service registered just for the test:
    /// `dns-sd -R DualionTest _smb._tcp local. 445 &` then
    /// `cargo test bonjour_live -- --ignored --nocapture`
    #[cfg(target_os = "macos")]
    #[test]
    #[ignore]
    fn bonjour_live() {
        let h = hosts();
        println!("{h:?}");
    }
}
