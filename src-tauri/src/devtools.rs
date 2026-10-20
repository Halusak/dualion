//! Development-only remote control (debug builds with `DUALION_DEBUG_DIR`
//! set): drop `*.js` files into that folder to run them in the webview, or
//! a `snapshot` file containing an output path to save a PNG of the page.
//! Used to verify the UI without screen-recording permissions.

use std::path::PathBuf;
use std::time::Duration;
use tauri::{AppHandle, Manager};

pub fn start(app: AppHandle) {
    let Some(dir) = std::env::var_os("DUALION_DEBUG_DIR").map(PathBuf::from) else { return };
    let _ = std::fs::create_dir_all(&dir);
    std::thread::spawn(move || loop {
        std::thread::sleep(Duration::from_millis(150));
        let Ok(rd) = std::fs::read_dir(&dir) else { continue };
        let mut files: Vec<PathBuf> = rd.flatten().map(|e| e.path()).collect();
        files.sort();
        for path in files {
            let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            if name.ends_with(".js") {
                if let Ok(js) = std::fs::read_to_string(&path) {
                    let _ = std::fs::remove_file(&path);
                    if let Some(w) = app.get_webview_window("main") {
                        let _ = w.eval(&js);
                    }
                }
            } else if name == "snapshot" {
                if let Ok(out) = std::fs::read_to_string(&path) {
                    let _ = std::fs::remove_file(&path);
                    snapshot(&app, out.trim().to_string());
                }
            }
        }
    });
}

#[cfg(target_os = "macos")]
fn snapshot(app: &AppHandle, out: String) {
    use block2::RcBlock;
    use objc2_app_kit::{NSBitmapImageFileType, NSBitmapImageRep, NSImage};
    use objc2_foundation::{NSDictionary, NSError};
    use objc2_web_kit::WKWebView;

    let Some(w) = app.get_webview_window("main") else { return };
    let r = w.with_webview(move |wv| unsafe {
        let web: &WKWebView = &*(wv.inner() as *const WKWebView);
        let out = out.clone();
        let block = RcBlock::new(move |img: *mut NSImage, err: *mut NSError| {
            if let Some(e) = err.as_ref() {
                eprintln!("[devtools] snapshot failed: {}", e.localizedDescription());
            }
            let Some(img) = img.as_ref() else { return };
            let Some(tiff) = img.TIFFRepresentation() else { return eprintln!("[devtools] no TIFF data") };
            let Some(rep) = NSBitmapImageRep::imageRepWithData(&tiff) else { return eprintln!("[devtools] no bitmap") };
            match rep.representationUsingType_properties(NSBitmapImageFileType::PNG, &NSDictionary::new()) {
                Some(png) => {
                    if let Err(e) = std::fs::write(&out, png.to_vec()) {
                        eprintln!("[devtools] write {out}: {e}");
                    }
                }
                None => eprintln!("[devtools] PNG encoding failed"),
            }
        });
        web.takeSnapshotWithConfiguration_completionHandler(None, &block);
    });
    if let Err(e) = r {
        eprintln!("[devtools] with_webview: {e}");
    }
}

#[cfg(not(target_os = "macos"))]
fn snapshot(_app: &AppHandle, _out: String) {}
