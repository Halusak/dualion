//! Windows shell integration: system image list icons, shell thumbnails,
//! accent colour and build number.

use super::IconKey;
use std::ffi::c_void;
use std::mem::size_of;
use windows::core::{HSTRING, PCWSTR};
use windows::Win32::Foundation::SIZE;
use windows::Win32::Graphics::Gdi::{
    CreateCompatibleDC, DeleteDC, DeleteObject, GetDIBits, GetObjectW, BITMAP, BITMAPINFO, BITMAPINFOHEADER, BI_RGB,
    DIB_RGB_COLORS, HBITMAP, HGDIOBJ,
};
use windows::Win32::Storage::FileSystem::{FILE_ATTRIBUTE_DIRECTORY, FILE_ATTRIBUTE_NORMAL, FILE_FLAGS_AND_ATTRIBUTES};
use windows::Win32::System::Com::{CoInitializeEx, COINIT_APARTMENTTHREADED};
use windows::Win32::System::Registry::{RegGetValueW, HKEY, HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, RRF_RT_REG_DWORD, RRF_RT_REG_SZ};
use windows::Win32::UI::Controls::{IImageList, ILD_TRANSPARENT};
use windows::Win32::UI::Shell::{
    IShellItemImageFactory, SHCreateItemFromParsingName, SHGetFileInfoW, SHGetImageList, SHFILEINFOW, SHGFI_SYSICONINDEX,
    SHGFI_USEFILEATTRIBUTES, SHIL_EXTRALARGE, SHIL_JUMBO, SHIL_LARGE, SHIL_SMALL, SIIGBF_BIGGERSIZEOK,
    SIIGBF_THUMBNAILONLY,
};
use windows::Win32::UI::WindowsAndMessaging::{DestroyIcon, GetIconInfo, HICON, ICONINFO};

pub fn init_thread() {
    unsafe {
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
    }
}

/// Reads a 32-bit bitmap as straight RGBA (top-down).
fn bitmap_rgba(hbm: HBITMAP) -> Option<(u32, u32, Vec<u8>)> {
    unsafe {
        let mut bm = BITMAP::default();
        if GetObjectW(HGDIOBJ(hbm.0), size_of::<BITMAP>() as i32, Some(&mut bm as *mut _ as *mut c_void)) == 0 {
            return None;
        }
        let (w, h) = (bm.bmWidth, bm.bmHeight.abs());
        if w <= 0 || h <= 0 {
            return None;
        }
        let mut info = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: w,
                biHeight: -h,
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut buf = vec![0u8; (w * h * 4) as usize];
        let dc = CreateCompatibleDC(None);
        let lines = GetDIBits(dc, hbm, 0, h as u32, Some(buf.as_mut_ptr() as *mut c_void), &mut info, DIB_RGB_COLORS);
        let _ = DeleteDC(dc);
        if lines == 0 {
            return None;
        }
        for px in buf.chunks_exact_mut(4) {
            px.swap(0, 2); // BGRA -> RGBA
        }
        Some((w as u32, h as u32, buf))
    }
}

fn icon_rgba(icon: HICON) -> Option<(u32, u32, Vec<u8>)> {
    unsafe {
        let mut ii = ICONINFO::default();
        GetIconInfo(icon, &mut ii).ok()?;
        let color = bitmap_rgba(ii.hbmColor);
        let mask = bitmap_rgba(ii.hbmMask);
        let _ = DeleteObject(HGDIOBJ(ii.hbmColor.0));
        let _ = DeleteObject(HGDIOBJ(ii.hbmMask.0));
        let (w, h, mut px) = color?;
        // Legacy icons carry no alpha channel: derive it from the AND mask.
        if px.chunks_exact(4).all(|p| p[3] == 0) {
            if let Some((mw, _, m)) = mask {
                if mw == w {
                    for (p, mp) in px.chunks_exact_mut(4).zip(m.chunks_exact(4)) {
                        p[3] = if mp[0] == 0 { 255 } else { 0 };
                    }
                }
            } else {
                for p in px.chunks_exact_mut(4) {
                    p[3] = 255;
                }
            }
        }
        Some((w, h, px))
    }
}

fn content_extent(w: u32, h: u32, px: &[u8]) -> (u32, u32) {
    let (mut mx, mut my) = (0, 0);
    for y in 0..h {
        for x in 0..w {
            if px[((y * w + x) * 4 + 3) as usize] > 8 {
                mx = mx.max(x + 1);
                my = my.max(y + 1);
            }
        }
    }
    (mx, my)
}

fn system_icon(index: i32, list_kind: u32) -> Option<(u32, u32, Vec<u8>)> {
    unsafe {
        let list: IImageList = SHGetImageList(list_kind as i32).ok()?;
        let icon = list.GetIcon(index, ILD_TRANSPARENT.0).ok()?;
        let r = icon_rgba(icon);
        let _ = DestroyIcon(icon);
        r
    }
}

fn encode_png(w: u32, h: u32, rgba: Vec<u8>, px: u32) -> Option<Vec<u8>> {
    let img = image::RgbaImage::from_raw(w, h, rgba)?;
    let img = if w != px || h != px {
        image::imageops::resize(&img, px, px, image::imageops::FilterType::Lanczos3)
    } else {
        img
    };
    let mut out = std::io::Cursor::new(Vec::new());
    image::DynamicImage::ImageRgba8(img).write_to(&mut out, image::ImageFormat::Png).ok()?;
    Some(out.into_inner())
}

pub fn render(key: &IconKey, px: u32) -> Option<Vec<u8>> {
    let (path, attrs, use_attrs): (String, FILE_FLAGS_AND_ATTRIBUTES, bool) = match key {
        IconKey::Ext(e) => (format!("file.{e}"), FILE_ATTRIBUTE_NORMAL, true),
        IconKey::Dir => ("folder".into(), FILE_ATTRIBUTE_DIRECTORY, true),
        IconKey::File => ("file".into(), FILE_ATTRIBUTE_NORMAL, true),
        IconKey::Path(p) => (p.clone(), FILE_FLAGS_AND_ATTRIBUTES(0), false),
    };
    let mut info = SHFILEINFOW::default();
    let mut flags = SHGFI_SYSICONINDEX;
    if use_attrs {
        flags |= SHGFI_USEFILEATTRIBUTES;
    }
    let ok = unsafe {
        SHGetFileInfoW(&HSTRING::from(path.as_str()), attrs, Some(&mut info), size_of::<SHFILEINFOW>() as u32, flags)
    };
    if ok == 0 {
        return None;
    }
    let kind = match px {
        0..=16 => SHIL_SMALL,
        17..=32 => SHIL_LARGE,
        33..=48 => SHIL_EXTRALARGE,
        _ => SHIL_JUMBO,
    };
    let (mut w, mut h, mut rgba) = system_icon(info.iIcon, kind)?;
    if kind == SHIL_JUMBO {
        // Types without 256px artwork are drawn small in the corner of the jumbo bitmap.
        let (cx, cy) = content_extent(w, h, &rgba);
        if cx <= w / 4 && cy <= h / 4 {
            if let Some(r) = system_icon(info.iIcon, SHIL_EXTRALARGE) {
                (w, h, rgba) = r;
            }
        }
    }
    encode_png(w, h, rgba, px)
}

/// Explorer thumbnail (images, videos, PDFs… whatever the shell can preview).
pub fn thumbnail(path: &str, px: u32) -> Option<Vec<u8>> {
    unsafe {
        let factory: IShellItemImageFactory = SHCreateItemFromParsingName(&HSTRING::from(path), None).ok()?;
        let size = SIZE { cx: px as i32, cy: px as i32 };
        let hbm = factory.GetImage(size, SIIGBF_THUMBNAILONLY | SIIGBF_BIGGERSIZEOK).ok()?;
        let r = bitmap_rgba(hbm);
        let _ = DeleteObject(HGDIOBJ(hbm.0));
        let (w, h, rgba) = r?;
        let img = image::RgbaImage::from_raw(w, h, rgba)?;
        let img = image::DynamicImage::ImageRgba8(img);
        let img = if w > px || h > px { img.thumbnail(px, px) } else { img };
        let mut out = std::io::Cursor::new(Vec::new());
        img.write_to(&mut out, image::ImageFormat::Png).ok()?;
        Some(out.into_inner())
    }
}

fn reg_dword(root: HKEY, key: &str, value: &str) -> Option<u32> {
    let mut data = 0u32;
    let mut len = size_of::<u32>() as u32;
    let r = unsafe {
        RegGetValueW(
            root,
            &HSTRING::from(key),
            &HSTRING::from(value),
            RRF_RT_REG_DWORD,
            None,
            Some(&mut data as *mut u32 as *mut c_void),
            Some(&mut len),
        )
    };
    r.is_ok().then_some(data)
}

fn reg_string(root: HKEY, key: &str, value: &str) -> Option<String> {
    let mut buf = [0u16; 256];
    let mut len = (buf.len() * 2) as u32;
    let r = unsafe {
        RegGetValueW(
            root,
            &HSTRING::from(key),
            &HSTRING::from(value),
            RRF_RT_REG_SZ,
            None,
            Some(buf.as_mut_ptr() as *mut c_void),
            Some(&mut len),
        )
    };
    if r.is_err() {
        return None;
    }
    let n = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    Some(String::from_utf16_lossy(&buf[..n]))
}

/// DWM accent colour as `#rrggbb` (stored as 0xAABBGGRR).
pub fn accent_color() -> Option<String> {
    let v = reg_dword(HKEY_CURRENT_USER, r"Software\Microsoft\Windows\DWM", "AccentColor")?;
    let (r, g, b) = (v & 0xff, (v >> 8) & 0xff, (v >> 16) & 0xff);
    Some(format!("#{r:02x}{g:02x}{b:02x}"))
}

pub fn build_number() -> u32 {
    reg_string(HKEY_LOCAL_MACHINE, r"SOFTWARE\Microsoft\Windows NT\CurrentVersion", "CurrentBuildNumber")
        .and_then(|s| s.trim().parse().ok())
        .unwrap_or(0)
}

/// Opens the native "Properties" dialog of a file.
pub fn show_properties(path: &str) -> bool {
    use windows::Win32::UI::Shell::{SHObjectProperties, SHOP_FILEPATH};
    init_thread();
    unsafe { SHObjectProperties(None, SHOP_FILEPATH, &HSTRING::from(path), PCWSTR::null()).as_bool() }
}
