//! Phones, cameras and other MTP devices on Windows through the shell (the
//! same items Explorer shows under "This PC"), no developer mode needed.
//! Items are addressed by their display names; absolute item ids are cached
//! per path so following operations need not walk the device again.
//! macOS has no system MTP support: Android devices go through ADB there.

use super::{entry, RemoteConfig, RemoteFs, Source};
use crate::error::{AppError, AppResult, ErrorCode};
use crate::jobs::vfs::{ReadStream, WriteStream};
use crate::location::Entry;
use parking_lot::Mutex;
use std::collections::HashMap;
use windows::core::{Interface, HSTRING, PCWSTR};
use windows::Win32::Foundation::FILETIME;
use windows::Win32::Storage::EnhancedStorage::{PKEY_DateModified, PKEY_Size};
use windows::Win32::Storage::FileSystem::FILE_ATTRIBUTE_DIRECTORY;
use windows::Win32::System::Com::{CoCreateInstance, CoInitializeEx, CoTaskMemFree, IBindCtx, IStream, CLSCTX_ALL, COINIT_APARTMENTTHREADED};
use windows::Win32::System::SystemServices::{SFGAO_FILESYSTEM, SFGAO_FOLDER};
use windows::Win32::UI::Shell::Common::ITEMIDLIST;
use windows::Win32::UI::Shell::{
    FileOperation, IEnumShellItems, IFileOperation, IFileOperationProgressSink, ILFree, ILGetSize, IShellItem, IShellItem2, SHCreateItemFromIDList,
    SHCreateItemFromParsingName, SHGetIDListFromObject, SHGetKnownFolderIDList, BHID_EnumItems, BHID_Stream, FOF_NOCONFIRMATION, FOF_NO_UI,
    FOLDERID_ComputerFolder, SIGDN_DESKTOPABSOLUTEPARSING, SIGDN_NORMALDISPLAY,
};

fn com() {
    // Shell items want a single-threaded apartment; a thread already in the
    // multithreaded one keeps working through the shell's proxies.
    let _ = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) };
}

fn err(e: windows::core::Error) -> AppError {
    // 0x80070002 / 0x80070003: not found; 0x80070005: access denied.
    let code = match e.code().0 as u32 {
        0x8007_0002 | 0x8007_0003 => ErrorCode::NotFound,
        0x8007_0005 => ErrorCode::PermissionDenied,
        0x8007_00B7 | 0x8007_0050 => ErrorCode::AlreadyExists,
        0x8007_0091 => ErrorCode::DirectoryNotEmpty,
        _ => ErrorCode::Io,
    };
    AppError::new(code, e.message())
}

fn display_name(item: &IShellItem, kind: windows::Win32::UI::Shell::SIGDN) -> String {
    unsafe {
        match item.GetDisplayName(kind) {
            Ok(p) => {
                let s = p.to_string().unwrap_or_default();
                CoTaskMemFree(Some(p.0 as *const _));
                s
            }
            Err(_) => String::new(),
        }
    }
}

/// Absolute item id list as bytes (safe to keep and share between threads).
fn id_of(item: &IShellItem) -> AppResult<Vec<u8>> {
    unsafe {
        let pidl = SHGetIDListFromObject(item).map_err(err)?;
        let size = ILGetSize(Some(pidl)) as usize;
        let bytes = std::slice::from_raw_parts(pidl as *const u8, size).to_vec();
        ILFree(Some(pidl));
        Ok(bytes)
    }
}

fn item_of(id: &[u8]) -> AppResult<IShellItem> {
    com();
    unsafe { SHCreateItemFromIDList(id.as_ptr() as *const ITEMIDLIST) }.map_err(err)
}

fn children(folder: &IShellItem) -> AppResult<Vec<IShellItem>> {
    let en: IEnumShellItems = unsafe { folder.BindToHandler(None::<&IBindCtx>, &BHID_EnumItems) }.map_err(err)?;
    let mut out = Vec::new();
    loop {
        let mut one = [None];
        let mut got = 0u32;
        if unsafe { en.Next(&mut one, Some(&mut got)) }.is_err() || got == 0 {
            break;
        }
        if let Some(i) = one[0].take() {
            out.push(i);
        }
    }
    Ok(out)
}

fn is_folder(item: &IShellItem) -> bool {
    unsafe { item.GetAttributes(SFGAO_FOLDER) }.map(|a| a.0 & SFGAO_FOLDER.0 != 0).unwrap_or(false)
}

fn filetime_ms(ft: FILETIME) -> i64 {
    let t = ((ft.dwHighDateTime as u64) << 32) | ft.dwLowDateTime as u64;
    if t == 0 {
        return 0;
    }
    (t / 10_000) as i64 - 11_644_473_600_000
}

/// Portable devices listed under "This PC".
pub fn devices() -> AppResult<Vec<Source>> {
    com();
    let pc: IShellItem = unsafe {
        let pidl = SHGetKnownFolderIDList(&FOLDERID_ComputerFolder, 0, None).map_err(err)?;
        let item = SHCreateItemFromIDList(pidl);
        ILFree(Some(pidl));
        item.map_err(err)?
    };
    let mut out = Vec::new();
    for c in children(&pc)? {
        let attrs = unsafe { c.GetAttributes(SFGAO_FILESYSTEM | SFGAO_FOLDER) }.map(|a| a.0).unwrap_or(0);
        // Devices are folders outside the file system with a device path.
        if attrs & SFGAO_FOLDER.0 == 0 || attrs & SFGAO_FILESYSTEM.0 != 0 {
            continue;
        }
        let parsing = display_name(&c, SIGDN_DESKTOPABSOLUTEPARSING);
        if !parsing.contains("\\\\?\\") && !parsing.to_ascii_lowercase().contains("#vid_") {
            continue;
        }
        let name = display_name(&c, SIGDN_NORMALDISPLAY);
        out.push(Source { protocol: super::Protocol::Mtp, id: parsing, name, detail: "MTP".into() });
    }
    Ok(out)
}

pub struct Mtp {
    /// Panel path → absolute item id.
    ids: Mutex<HashMap<String, Vec<u8>>>,
}

impl Mtp {
    pub fn connect(cfg: &RemoteConfig) -> AppResult<Self> {
        com();
        let root: IShellItem = unsafe { SHCreateItemFromParsingName(&HSTRING::from(cfg.host.trim()), None::<&IBindCtx>) }.map_err(err)?;
        let mut ids = HashMap::new();
        ids.insert("/".to_string(), id_of(&root)?);
        Ok(Self { ids: Mutex::new(ids) })
    }

    /// The shell item of a path, walking down from the nearest known folder.
    fn item(&self, path: &str) -> AppResult<IShellItem> {
        let path = super::norm(path);
        if let Some(id) = self.ids.lock().get(&path).cloned() {
            if let Ok(i) = item_of(&id) {
                return Ok(i);
            }
        }
        let parent = super::parent(&path).ok_or_else(|| AppError::new(ErrorCode::NotFound, "Device disconnected"))?;
        let folder = self.item(&parent)?;
        let name = super::file_name(&path);
        for c in children(&folder)? {
            if display_name(&c, SIGDN_NORMALDISPLAY) == name {
                self.ids.lock().insert(path.clone(), id_of(&c)?);
                return Ok(c);
            }
        }
        Err(AppError::new(ErrorCode::NotFound, "Not found").with_path(&path))
    }

    fn forget(&self, path: &str) {
        let path = super::norm(path);
        self.ids.lock().retain(|k, _| k != &path && !k.starts_with(&format!("{path}/")));
    }

    fn operation(&self) -> AppResult<IFileOperation> {
        com();
        let op: IFileOperation = unsafe { CoCreateInstance(&FileOperation, None, CLSCTX_ALL) }.map_err(err)?;
        unsafe { op.SetOperationFlags(FOF_NO_UI | FOF_NOCONFIRMATION) }.map_err(err)?;
        Ok(op)
    }
}

impl RemoteFs for Mtp {
    fn list(&self, path: &str) -> AppResult<Vec<Entry>> {
        let folder = self.item(path)?;
        let base = super::norm(path);
        let mut out = Vec::new();
        for c in children(&folder)? {
            let name = display_name(&c, SIGDN_NORMALDISPLAY);
            if name.is_empty() {
                continue;
            }
            let dir = is_folder(&c);
            let (size, mtime) = match c.cast::<IShellItem2>() {
                Ok(i2) => (unsafe { i2.GetUInt64(&PKEY_Size) }.unwrap_or(0), unsafe { i2.GetFileTime(&PKEY_DateModified) }.map(filetime_ms).unwrap_or(0)),
                Err(_) => (0, 0),
            };
            if let Ok(id) = id_of(&c) {
                self.ids.lock().insert(super::join(&base, &name), id);
            }
            out.push(entry(name, dir, size, mtime, 0, false));
        }
        Ok(out)
    }
    fn mkdir(&self, path: &str) -> AppResult<()> {
        let parent = self.item(&super::parent(path).unwrap_or_else(|| "/".into()))?;
        let op = self.operation()?;
        let name = HSTRING::from(super::file_name(path));
        unsafe { op.NewItem(&parent, FILE_ATTRIBUTE_DIRECTORY.0, &name, PCWSTR::null(), None::<&IFileOperationProgressSink>) }.map_err(err)?;
        unsafe { op.PerformOperations() }.map_err(err)
    }
    fn remove_file(&self, path: &str) -> AppResult<()> {
        let item = self.item(path)?;
        let op = self.operation()?;
        unsafe { op.DeleteItem(&item, None::<&IFileOperationProgressSink>) }.map_err(err)?;
        unsafe { op.PerformOperations() }.map_err(err)?;
        self.forget(path);
        Ok(())
    }
    fn remove_dir(&self, path: &str) -> AppResult<()> {
        self.remove_file(path)
    }
    fn rename(&self, from: &str, to: &str) -> AppResult<()> {
        let item = self.item(from)?;
        let op = self.operation()?;
        let new_name = HSTRING::from(super::file_name(to));
        if super::parent(from) == super::parent(to) {
            unsafe { op.RenameItem(&item, &new_name, None::<&IFileOperationProgressSink>) }.map_err(err)?;
        } else {
            let dest = self.item(&super::parent(to).unwrap_or_else(|| "/".into()))?;
            unsafe { op.MoveItem(&item, &dest, &new_name, None::<&IFileOperationProgressSink>) }.map_err(err)?;
        }
        unsafe { op.PerformOperations() }.map_err(err)?;
        self.forget(from);
        Ok(())
    }
    fn reader(&self, path: &str) -> AppResult<Box<dyn ReadStream + '_>> {
        let item = self.item(path)?;
        let stream: IStream = unsafe { item.BindToHandler(None::<&IBindCtx>, &BHID_Stream) }.map_err(err)?;
        Ok(Box::new(StreamReader(stream)))
    }
    fn writer(&self, path: &str) -> AppResult<Box<dyn WriteStream + '_>> {
        // The shell copies a local file onto the device under the wanted name.
        let parent = super::parent(path).unwrap_or_else(|| "/".into());
        let name = super::file_name(path).to_string();
        let target = super::norm(path);
        Ok(Box::new(super::SpoolWriter::new_path(move |local| {
            let dest = self.item(&parent)?;
            com();
            let src: IShellItem = unsafe { SHCreateItemFromParsingName(&HSTRING::from(local.as_os_str()), None::<&IBindCtx>) }.map_err(err)?;
            let op = self.operation()?;
            unsafe { op.CopyItem(&src, &dest, &HSTRING::from(name.as_str()), None::<&IFileOperationProgressSink>) }.map_err(err)?;
            unsafe { op.PerformOperations() }.map_err(err)?;
            self.forget(&target);
            Ok(())
        })?))
    }
}

/// Reads an `IStream` (used on the thread that opened it).
struct StreamReader(IStream);

// The copy engine reads a file on the thread that opened it.
unsafe impl Send for StreamReader {}

impl std::io::Read for StreamReader {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let mut n = 0u32;
        let len = buf.len().min(u32::MAX as usize) as u32;
        let hr = unsafe { self.0.Read(buf.as_mut_ptr() as *mut _, len, Some(&mut n)) };
        if hr.is_err() {
            return Err(std::io::Error::other(windows::core::Error::from(hr).message()));
        }
        Ok(n as usize)
    }
}

impl ReadStream for StreamReader {
    fn finish(self: Box<Self>) -> AppResult<()> {
        Ok(())
    }
}
