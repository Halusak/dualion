//! Windows shell integration: Explorer's own context menu for files and
//! native drag & drop out of the window. Every shell call reports errors
//! instead of panicking (unusual paths such as RDP-redirected drives
//! `\\tsclient\C` can make the shell refuse an item).

use crate::error::{AppError, AppResult};
use std::cell::RefCell;
use std::ffi::c_void;
use windows::core::{Interface, HSTRING, PCSTR, PCWSTR, PSTR};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, WPARAM};
use windows::Win32::System::Com::{IBindCtx, IDataObject};
use windows::Win32::System::Ole::{IDropSource, OleInitialize, DROPEFFECT_COPY, DROPEFFECT_LINK, DROPEFFECT_MOVE};
use windows::Win32::UI::Shell::Common::ITEMIDLIST;
use windows::Win32::UI::Shell::{
    DefSubclassProc, ILFindLastID, ILFree, IContextMenu, IContextMenu2, IContextMenu3, IShellFolder, RemoveWindowSubclass,
    SHBindToParent, SHCreateShellItemArrayFromIDLists, SHDoDragDrop, SHParseDisplayName, SetWindowSubclass, BHID_DataObject,
    CMF_CANRENAME, CMF_EXTENDEDVERBS, CMF_NORMAL, CMIC_MASK_CONTROL_DOWN, CMIC_MASK_PTINVOKE, CMIC_MASK_SHIFT_DOWN,
    CMINVOKECOMMANDINFO, CMINVOKECOMMANDINFOEX, GCS_VERBW,
};
use windows::Win32::UI::Input::KeyboardAndMouse::{GetKeyState, VK_CONTROL, VK_SHIFT};
use windows::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CreatePopupMenu, DestroyMenu, GetCursorPos, InsertMenuW, SetForegroundWindow, TrackPopupMenuEx, HMENU,
    MF_BYPOSITION, MF_GRAYED, MF_POPUP, MF_SEPARATOR, MF_STRING, SW_SHOWNORMAL, TPM_RETURNCMD, TPM_RIGHTBUTTON, WM_DRAWITEM,
    WM_INITMENUPOPUP, WM_MEASUREITEM, WM_MENUCHAR,
};

const CMIC_MASK_UNICODE: u32 = 0x4000;
/// Shell commands use ids FIRST..=LAST, our own items start at APP_BASE.
const FIRST: u32 = 1;
const LAST: u32 = 0x7FFF;
const APP_BASE: u32 = 0x8000;
const SUBCLASS_ID: usize = 0xD0A1;

pub fn hwnd_of(window: &tauri::WebviewWindow) -> AppResult<HWND> {
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    match window.window_handle().map(|h| h.as_raw()) {
        Ok(RawWindowHandle::Win32(h)) => Ok(HWND(h.hwnd.get() as *mut c_void)),
        _ => Err(AppError::unsupported("No window handle")),
    }
}

/// Absolute item id lists, freed on drop.
struct Pidls(Vec<*mut ITEMIDLIST>);

impl Drop for Pidls {
    fn drop(&mut self) {
        for p in &self.0 {
            unsafe { ILFree(Some(*p)) };
        }
    }
}

impl Pidls {
    /// Parses every path; fails when the shell refuses any of them.
    fn parse(paths: &[String]) -> AppResult<Pidls> {
        let mut out = Pidls(Vec::with_capacity(paths.len()));
        for p in paths {
            let mut pidl: *mut ITEMIDLIST = std::ptr::null_mut();
            let r = unsafe { SHParseDisplayName(&HSTRING::from(p.as_str()), None::<&IBindCtx>, &mut pidl, 0, None) };
            if r.is_err() || pidl.is_null() {
                return Err(AppError::unsupported(format!("The shell cannot use this item: {p}")));
            }
            out.0.push(pidl);
        }
        Ok(out)
    }

    fn as_const(&self) -> Vec<*const ITEMIDLIST> {
        self.0.iter().map(|p| *p as *const ITEMIDLIST).collect()
    }
}

fn shell_err(e: windows::core::Error) -> AppError {
    AppError::other(e.message())
}

// ---------------------------------------------------------------------------
// Clipboard: cut vs. copy ("Preferred DropEffect", as Explorer uses it)
// ---------------------------------------------------------------------------

fn drop_effect_format() -> u32 {
    use windows::Win32::System::DataExchange::RegisterClipboardFormatW;
    unsafe { RegisterClipboardFormatW(&HSTRING::from("Preferred DropEffect")) }
}

/// True when the files on the clipboard were cut (Ctrl+X in Explorer).
pub fn clipboard_is_cut() -> bool {
    use windows::Win32::Foundation::HGLOBAL;
    use windows::Win32::System::DataExchange::{CloseClipboard, GetClipboardData, OpenClipboard};
    use windows::Win32::System::Memory::{GlobalLock, GlobalUnlock};
    unsafe {
        if OpenClipboard(None).is_err() {
            return false;
        }
        let mut cut = false;
        if let Ok(h) = GetClipboardData(drop_effect_format()) {
            let g = HGLOBAL(h.0);
            let p = GlobalLock(g) as *const u32;
            if !p.is_null() {
                cut = *p & DROPEFFECT_MOVE.0 != 0;
                let _ = GlobalUnlock(g);
            }
        }
        let _ = CloseClipboard();
        cut
    }
}

/// Marks the files on the clipboard as cut, so pasting in Explorer moves them.
pub fn clipboard_mark_cut() {
    use windows::Win32::Foundation::HANDLE;
    use windows::Win32::System::DataExchange::{CloseClipboard, OpenClipboard, SetClipboardData};
    use windows::Win32::System::Memory::{GlobalAlloc, GlobalLock, GlobalUnlock, GMEM_MOVEABLE};
    unsafe {
        let Ok(g) = GlobalAlloc(GMEM_MOVEABLE, 4) else { return };
        let p = GlobalLock(g) as *mut u32;
        if p.is_null() {
            return;
        }
        *p = DROPEFFECT_MOVE.0;
        let _ = GlobalUnlock(g);
        if OpenClipboard(None).is_ok() {
            // The clipboard owns the memory from here on.
            let _ = SetClipboardData(drop_effect_format(), Some(HANDLE(g.0)));
            let _ = CloseClipboard();
        }
    }
}

// ---------------------------------------------------------------------------
// Opening files
// ---------------------------------------------------------------------------

/// Opens a file like Explorer does: the association's default verb (which
/// is not always "open", e.g. Photoshop's TIFF association).
pub fn open_default(path: &str) -> AppResult<()> {
    use windows::Win32::UI::Shell::{ShellExecuteExW, SEE_MASK_INVOKEIDLIST, SEE_MASK_NOASYNC, SHELLEXECUTEINFOW};
    use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
    let _ = unsafe { windows::Win32::System::Com::CoInitializeEx(None, windows::Win32::System::Com::COINIT_APARTMENTTHREADED) };
    let file = HSTRING::from(path);
    let mut info = SHELLEXECUTEINFOW {
        cbSize: std::mem::size_of::<SHELLEXECUTEINFOW>() as u32,
        fMask: SEE_MASK_INVOKEIDLIST | SEE_MASK_NOASYNC,
        lpFile: PCWSTR(file.as_ptr()),
        nShow: SW_SHOWNORMAL.0,
        ..Default::default()
    };
    unsafe { ShellExecuteExW(&mut info) }.map_err(|e| AppError::other(e.message()).with_path(path))
}

// ---------------------------------------------------------------------------
// Drag out
// ---------------------------------------------------------------------------

/// Starts a system drag of `paths` (blocks until the drop; main thread only).
pub fn drag_out(hwnd: HWND, paths: &[String]) -> AppResult<()> {
    // Already initialised by the webview on this thread; S_FALSE is fine.
    let _ = unsafe { OleInitialize(None) };
    let pidls = Pidls::parse(paths)?;
    let array = unsafe { SHCreateShellItemArrayFromIDLists(&pidls.as_const()) }.map_err(shell_err)?;
    let data: IDataObject = unsafe { array.BindToHandler(None::<&IBindCtx>, &BHID_DataObject) }.map_err(shell_err)?;
    // SHDoDragDrop supplies the drop source and the drag image (like Explorer).
    unsafe { SHDoDragDrop(Some(hwnd), &data, None::<&IDropSource>, DROPEFFECT_COPY | DROPEFFECT_MOVE | DROPEFFECT_LINK) }
        .map_err(shell_err)?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Context menu
// ---------------------------------------------------------------------------

/// One of our own commands shown in the "Dualion" submenu (`id` empty = separator).
#[derive(Debug, Clone, serde::Deserialize)]
pub struct AppItem {
    pub id: String,
    #[serde(default)]
    pub label: String,
    #[serde(default = "yes")]
    pub enabled: bool,
}

fn yes() -> bool {
    true
}

thread_local! {
    /// The menu being shown, for owner-drawn items and submenus ("Send to", "Open with").
    static ACTIVE: RefCell<Option<IContextMenu>> = const { RefCell::new(None) };
}

unsafe extern "system" fn subclass(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM, _id: usize, _data: usize) -> LRESULT {
    if matches!(msg, WM_INITMENUPOPUP | WM_DRAWITEM | WM_MEASUREITEM | WM_MENUCHAR) {
        let handled = ACTIVE.with(|a| {
            let a = a.borrow();
            let cm = a.as_ref()?;
            if let Ok(cm3) = cm.cast::<IContextMenu3>() {
                let mut res = LRESULT(0);
                if unsafe { cm3.HandleMenuMsg2(msg, wp, lp, Some(&mut res)) }.is_ok() {
                    return Some(res);
                }
            }
            if msg != WM_MENUCHAR {
                if let Ok(cm2) = cm.cast::<IContextMenu2>() {
                    if unsafe { cm2.HandleMenuMsg(msg, wp, lp) }.is_ok() {
                        return Some(LRESULT(0));
                    }
                }
            }
            None
        });
        if let Some(r) = handled {
            return r;
        }
    }
    unsafe { DefSubclassProc(hwnd, msg, wp, lp) }
}

/// Destroys the popup menu (and its submenus) when done.
struct Menu(HMENU);

impl Drop for Menu {
    fn drop(&mut self) {
        let _ = unsafe { DestroyMenu(self.0) };
    }
}

fn verb_of(cm: &IContextMenu, id: u32) -> String {
    let mut buf = [0u16; 128];
    let ok = unsafe { cm.GetCommandString((id - FIRST) as usize, GCS_VERBW, None, PSTR(buf.as_mut_ptr() as *mut u8), buf.len() as u32) };
    if ok.is_err() {
        return String::new();
    }
    let len = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    String::from_utf16_lossy(&buf[..len]).to_ascii_lowercase()
}

/// Shows Explorer's context menu for `paths` (all in one folder) at the
/// cursor, with our commands in a submenu. Returns the id of one of our
/// commands to run, or None when nothing (or a shell command) was chosen.
/// Main thread only (the menu is modal).
pub fn context_menu(hwnd: HWND, paths: &[String], all_dirs: bool, title: &str, items: &[AppItem]) -> AppResult<Option<String>> {
    let pidls = Pidls::parse(paths)?;
    let first = *pidls.0.first().ok_or_else(|| AppError::invalid("Nothing selected"))?;
    let folder: IShellFolder = unsafe { SHBindToParent(first, None) }.map_err(shell_err)?;
    let children: Vec<*const ITEMIDLIST> = pidls.0.iter().map(|p| unsafe { ILFindLastID(*p) } as *const ITEMIDLIST).collect();
    let cm: IContextMenu = unsafe { folder.GetUIObjectOf(hwnd, &children, None) }.map_err(shell_err)?;

    let menu = Menu(unsafe { CreatePopupMenu() }.map_err(shell_err)?);
    let shift = unsafe { GetKeyState(VK_SHIFT.0 as i32) } < 0;
    let flags = CMF_NORMAL | CMF_CANRENAME | if shift { CMF_EXTENDEDVERBS } else { 0 };
    unsafe { cm.QueryContextMenu(menu.0, 0, FIRST, LAST, flags) }.ok().map_err(shell_err)?;

    // Our commands: a submenu at the top, like other file managers do.
    if !items.is_empty() {
        let sub = unsafe { CreatePopupMenu() }.map_err(shell_err)?;
        for (i, it) in items.iter().enumerate() {
            if it.id.is_empty() {
                let _ = unsafe { AppendMenuW(sub, MF_SEPARATOR, 0, PCWSTR::null()) };
            } else {
                let f = if it.enabled { MF_STRING } else { MF_STRING | MF_GRAYED };
                let _ = unsafe { AppendMenuW(sub, f, APP_BASE as usize + i, &HSTRING::from(it.label.as_str())) };
            }
        }
        // The parent menu owns `sub` from here on (destroyed with it).
        unsafe { InsertMenuW(menu.0, 0, MF_BYPOSITION | MF_POPUP, sub.0 as usize, &HSTRING::from(title)) }.map_err(shell_err)?;
        let _ = unsafe { InsertMenuW(menu.0, 1, MF_BYPOSITION | MF_SEPARATOR, 0, PCWSTR::null()) };
    }

    let mut pt = POINT::default();
    let _ = unsafe { GetCursorPos(&mut pt) };
    ACTIVE.with(|a| *a.borrow_mut() = Some(cm.clone()));
    let subclassed = unsafe { SetWindowSubclass(hwnd, Some(subclass), SUBCLASS_ID, 0) }.as_bool();
    let _ = unsafe { SetForegroundWindow(hwnd) };
    let cmd = unsafe { TrackPopupMenuEx(menu.0, TPM_RETURNCMD.0 | TPM_RIGHTBUTTON.0, pt.x, pt.y, hwnd, None) }.0 as u32;
    if subclassed {
        let _ = unsafe { RemoveWindowSubclass(hwnd, Some(subclass), SUBCLASS_ID) };
    }
    ACTIVE.with(|a| *a.borrow_mut() = None);

    if cmd == 0 {
        return Ok(None);
    }
    if cmd >= APP_BASE {
        return Ok(items.get((cmd - APP_BASE) as usize).map(|i| i.id.clone()));
    }

    // Commands with a Dualion equivalent run in Dualion (undo, inline rename,
    // folders open in the panel); the rest is executed by the shell.
    let verb = verb_of(&cm, cmd);
    let ours = match verb.as_str() {
        "rename" => Some("rename"),
        "delete" => Some("delete"),
        "open" if all_dirs => Some("open"),
        _ => None,
    };
    if let Some(id) = ours {
        return Ok(Some(id.to_string()));
    }
    let mut mask = CMIC_MASK_UNICODE | CMIC_MASK_PTINVOKE;
    if unsafe { GetKeyState(VK_SHIFT.0 as i32) } < 0 {
        mask |= CMIC_MASK_SHIFT_DOWN;
    }
    if unsafe { GetKeyState(VK_CONTROL.0 as i32) } < 0 {
        mask |= CMIC_MASK_CONTROL_DOWN;
    }
    let offset = (cmd - FIRST) as usize;
    let info = CMINVOKECOMMANDINFOEX {
        cbSize: std::mem::size_of::<CMINVOKECOMMANDINFOEX>() as u32,
        fMask: mask,
        hwnd,
        lpVerb: PCSTR(offset as *const u8),
        lpVerbW: PCWSTR(offset as *const u16),
        nShow: SW_SHOWNORMAL.0,
        ptInvoke: pt,
        ..Default::default()
    };
    unsafe { cm.InvokeCommand(&info as *const CMINVOKECOMMANDINFOEX as *const CMINVOKECOMMANDINFO) }.map_err(shell_err)?;
    // Copy / Cut also fill Dualion's own clipboard so Paste works in the panels.
    Ok(match verb.as_str() {
        "copy" => Some("clipCopy".into()),
        "cut" => Some("clipCut".into()),
        _ => None,
    })
}
