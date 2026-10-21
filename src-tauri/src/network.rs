//! "Network" place like Total Commander's Network Neighborhood: computers on
//! the local network → their shared folders → connecting a share.
//!
//! Windows: computers from the shell's Network folder (what Explorer shows),
//! shares via NetShareEnum, credentials via the system dialog (WNet).
//! macOS: computers via Bonjour (`_smb._tcp`), shares via `smbutil view`,
//! mounting via NetFS (the same dialogs as in Finder).

use crate::error::{AppError, AppResult, ErrorCode};
use crate::location::{flags, Crumb, Entry, Listing, Location};

pub fn list(host: Option<&str>) -> AppResult<Listing> {
    let root = Crumb { label: String::new(), icon: "network", location: Location::Network { host: None } };
    match host {
        None => {
            let mut names = hosts();
            names.sort_by_key(|n| n.to_lowercase());
            names.dedup();
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
        let mut out = Vec::new();
        for f in found {
            let mut host: Option<String> = None;
            let mut rr: DnsRef = std::ptr::null_mut();
            let (Ok(name), Ok(domain)) = (CString::new(f.name.clone()), CString::new(f.domain)) else { continue };
            let err = unsafe {
                DNSServiceResolve(&mut rr, 0, f.iface, name.as_ptr(), ty.as_ptr(), domain.as_ptr(), on_resolve, &mut host as *mut _ as *mut c_void)
            };
            if err == 0 && !rr.is_null() {
                let h = &host as *const Option<String>;
                pump(rr, Instant::now() + Duration::from_millis(1000), &|| unsafe { (*h).is_some() });
                unsafe { DNSServiceRefDeallocate(rr) };
            }
            let h = host.unwrap_or_else(|| format!("{}.local", f.name.replace(' ', "-")));
            // This Mac's own sharing is not a place to go to.
            if me.as_deref().is_some_and(|m| h.eq_ignore_ascii_case(m)) {
                continue;
            }
            if !out.contains(&h) {
                out.push(h);
            }
        }
        out
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
    pub fn shares(host: &str) -> AppResult<Vec<String>> {
        Err(auth_needed(host))
    }
}

pub use imp::*;

#[cfg(test)]
mod tests {
    use super::*;

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
