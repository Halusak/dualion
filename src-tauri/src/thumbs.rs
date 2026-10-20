//! Thumbnails for the "thumbnails" view, served via `thumb://localhost/<path>?s=<px>`.
//!
//! macOS: ImageIO (downscaled decoding, HEIC/RAW/PSD…) and QuickLook for
//! videos, PDFs and documents. Windows: the Explorer thumbnail cache.
//! Everywhere: the `image` crate as fallback. Results are cached in memory
//! and on disk; a LIFO worker pool renders what the user looks at first.

use lru::LruCache;
use parking_lot::{Condvar, Mutex};
use std::io::Cursor;
use std::num::NonZeroUsize;
use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock, OnceLock};

pub struct Thumb {
    pub bytes: Vec<u8>,
    pub mime: &'static str,
}

type Job = Box<dyn FnOnce() + Send>;

struct Pool {
    stack: Mutex<Vec<Job>>,
    cv: Condvar,
}

static POOL: LazyLock<Arc<Pool>> = LazyLock::new(|| {
    let pool = Arc::new(Pool { stack: Mutex::new(Vec::new()), cv: Condvar::new() });
    let n = std::thread::available_parallelism().map(|n| (n.get() / 2).clamp(2, 6)).unwrap_or(3);
    for i in 0..n {
        let p = pool.clone();
        std::thread::Builder::new()
            .name(format!("thumbs-{i}"))
            .spawn(move || loop {
                let job = {
                    let mut s = p.stack.lock();
                    while s.is_empty() {
                        p.cv.wait(&mut s);
                    }
                    s.pop().expect("non-empty")
                };
                job();
            })
            .expect("thumb worker");
    }
    pool
});

/// Runs `f` on the thumbnail pool. Newest requests are served first, so
/// scrolling quickly does not leave the user waiting for off-screen items.
pub fn spawn(f: impl FnOnce() + Send + 'static) {
    let mut s = POOL.stack.lock();
    s.push(Box::new(f));
    // Drop the oldest requests when the user scrolled through thousands of files.
    if s.len() > 400 {
        s.drain(0..100);
    }
    drop(s);
    POOL.cv.notify_one();
}

static MEM: LazyLock<Mutex<LruCache<String, Arc<Thumb>>>> =
    LazyLock::new(|| Mutex::new(LruCache::new(NonZeroUsize::new(800).unwrap())));
static DISK_DIR: OnceLock<PathBuf> = OnceLock::new();

pub fn set_cache_dir(dir: PathBuf) {
    let _ = std::fs::create_dir_all(&dir);
    let _ = DISK_DIR.set(dir);
}

const IMAGE_CRATE_EXTS: &[&str] = &["jpg", "jpeg", "png", "gif", "webp", "bmp", "ico", "tif", "tiff"];

fn cache_key(path: &Path, px: u32) -> Option<String> {
    let m = std::fs::metadata(path).ok()?;
    let mtime = m.modified().map(crate::util::system_time_ms).unwrap_or(0);
    Some(format!("{}|{}|{}|{}", path.to_string_lossy(), mtime, m.len(), px))
}

pub fn thumbnail(path: &Path, px: u32) -> Option<Arc<Thumb>> {
    let px = px.clamp(32, 1024);
    let key = cache_key(path, px)?;
    if let Some(hit) = MEM.lock().get(&key) {
        return Some(hit.clone());
    }
    let disk = DISK_DIR.get().map(|d| d.join(format!("{:032x}", xxhash_rust::xxh3::xxh3_128(key.as_bytes()))));
    if let Some(d) = &disk {
        if let Ok(bytes) = std::fs::read(d) {
            let mime = if bytes.starts_with(&[0x89, b'P', b'N', b'G']) { "image/png" } else { "image/jpeg" };
            let t = Arc::new(Thumb { bytes, mime });
            MEM.lock().put(key, t.clone());
            return Some(t);
        }
    }
    let t = Arc::new(generate(path, px)?);
    if let Some(d) = &disk {
        let _ = std::fs::write(d, &t.bytes);
    }
    MEM.lock().put(key, t.clone());
    Some(t)
}

fn generate(path: &Path, px: u32) -> Option<Thumb> {
    #[cfg(target_os = "macos")]
    {
        if let Some(t) = macos::image_io(path, px) {
            return Some(t);
        }
        if let Some(t) = macos::quicklook(path, px) {
            return Some(t);
        }
    }
    #[cfg(windows)]
    {
        if let Some(png) = crate::icons::windows::thumbnail(&path.to_string_lossy(), px) {
            return Some(Thumb { bytes: png, mime: "image/png" });
        }
    }
    let ext = path.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
    if IMAGE_CRATE_EXTS.contains(&ext.as_str()) {
        return with_image_crate(path, px);
    }
    None
}

fn with_image_crate(path: &Path, px: u32) -> Option<Thumb> {
    use image::ImageDecoder;
    let reader = image::ImageReader::open(path).ok()?.with_guessed_format().ok()?;
    let mut decoder = reader.into_decoder().ok()?;
    let orientation = decoder.orientation().ok();
    let mut img = image::DynamicImage::from_decoder(decoder).ok()?;
    if let Some(o) = orientation {
        img.apply_orientation(o);
    }
    let img = if img.width() > px || img.height() > px { img.thumbnail(px, px) } else { img };
    encode(img)
}

fn encode(img: image::DynamicImage) -> Option<Thumb> {
    let mut out = Cursor::new(Vec::new());
    if img.color().has_alpha() {
        img.write_to(&mut out, image::ImageFormat::Png).ok()?;
        Some(Thumb { bytes: out.into_inner(), mime: "image/png" })
    } else {
        let rgb = img.to_rgb8();
        let enc = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, 82);
        rgb.write_with_encoder(enc).ok()?;
        Some(Thumb { bytes: out.into_inner(), mime: "image/jpeg" })
    }
}

#[cfg(target_os = "macos")]
mod macos {
    use super::Thumb;
    use block2::RcBlock;
    use objc2::rc::autoreleasepool;
    use objc2::AllocAnyThread;
    use objc2_app_kit::{NSBitmapImageFileType, NSBitmapImageRep};
    use objc2_core_foundation::{CFBoolean, CFDictionary, CFNumber, CFString, CFType, CFURL, CGSize};
    use objc2_core_graphics::CGImage;
    use objc2_foundation::{NSDictionary, NSError, NSURL};
    use objc2_image_io::{
        kCGImageSourceCreateThumbnailFromImageAlways, kCGImageSourceCreateThumbnailWithTransform,
        kCGImageSourceShouldCacheImmediately, kCGImageSourceThumbnailMaxPixelSize, CGImageSource,
    };
    use objc2_quick_look_thumbnailing::{
        QLThumbnailGenerationRequest, QLThumbnailGenerationRequestRepresentationTypes, QLThumbnailGenerator,
        QLThumbnailRepresentation,
    };
    use std::path::Path;
    use std::time::Duration;

    const IMAGE_EXTS: &[&str] = &[
        "jpg", "jpeg", "png", "gif", "webp", "bmp", "ico", "icns", "tif", "tiff", "heic", "heif", "avif", "psd",
        "tga", "dng", "cr2", "cr3", "nef", "arw", "orf", "raf", "rw2", "pef", "srw", "jp2", "exr", "svg",
    ];

    fn encode_cg(cg: &CGImage) -> Option<Thumb> {
        let rep = NSBitmapImageRep::initWithCGImage(NSBitmapImageRep::alloc(), cg);
        let has_alpha = rep.hasAlpha();
        let ty = if has_alpha { NSBitmapImageFileType::PNG } else { NSBitmapImageFileType::JPEG };
        let data = unsafe { rep.representationUsingType_properties(ty, &NSDictionary::new()) }?;
        Some(Thumb { bytes: data.to_vec(), mime: if has_alpha { "image/png" } else { "image/jpeg" } })
    }

    pub fn image_io(path: &Path, px: u32) -> Option<Thumb> {
        let ext = path.extension()?.to_string_lossy().to_lowercase();
        if !IMAGE_EXTS.contains(&ext.as_str()) || ext == "svg" {
            return None;
        }
        autoreleasepool(|_| {
            let url = CFURL::from_file_path(path)?;
            let src = unsafe { CGImageSource::with_url(&url, None) }?;
            let max = CFNumber::new_i32(px as i32);
            let keys: [&CFString; 4] = unsafe {
                [
                    kCGImageSourceCreateThumbnailFromImageAlways,
                    kCGImageSourceCreateThumbnailWithTransform,
                    kCGImageSourceShouldCacheImmediately,
                    kCGImageSourceThumbnailMaxPixelSize,
                ]
            };
            let t: &CFType = CFBoolean::new(true);
            let values: [&CFType; 4] = [t, t, t, &max];
            let opts = CFDictionary::<CFString, CFType>::from_slices(&keys, &values);
            let cg = unsafe { src.thumbnail_at_index(0, Some(opts.as_opaque())) }?;
            encode_cg(&cg)
        })
    }

    /// QuickLook thumbnails (videos, PDFs, Office documents, fonts…).
    pub fn quicklook(path: &Path, px: u32) -> Option<Thumb> {
        let (tx, rx) = std::sync::mpsc::channel::<Option<Thumb>>();
        autoreleasepool(|_| {
            let url = NSURL::from_file_path(path)?;
            let req = unsafe {
                QLThumbnailGenerationRequest::initWithFileAtURL_size_scale_representationTypes(
                    QLThumbnailGenerationRequest::alloc(),
                    &url,
                    CGSize { width: px as f64, height: px as f64 },
                    1.0,
                    QLThumbnailGenerationRequestRepresentationTypes::Thumbnail,
                )
            };
            let tx = std::sync::Mutex::new(Some(tx));
            let block = RcBlock::new(move |rep: *mut QLThumbnailRepresentation, _err: *mut NSError| {
                let result = unsafe { rep.as_ref() }.and_then(|r| {
                    let cg = unsafe { r.CGImage() };
                    encode_cg(&cg)
                });
                if let Some(tx) = tx.lock().ok().and_then(|mut g| g.take()) {
                    let _ = tx.send(result);
                }
            });
            unsafe {
                QLThumbnailGenerator::sharedGenerator().generateBestRepresentationForRequest_completionHandler(&req, &block)
            };
            Some(())
        })?;
        rx.recv_timeout(Duration::from_secs(10)).ok().flatten()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn image_crate_thumbnail() {
        let dir = std::env::temp_dir().join(format!("dualion-thumb-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("a.png");
        image::RgbImage::from_pixel(400, 200, image::Rgb([200, 30, 30])).save(&p).unwrap();
        let t = with_image_crate(&p, 100).unwrap();
        assert_eq!(t.mime, "image/jpeg");
        let back = image::load_from_memory(&t.bytes).unwrap();
        assert_eq!((back.width(), back.height()), (100, 50));
        #[cfg(target_os = "macos")]
        {
            let t = macos::image_io(&p, 100).unwrap();
            let back = image::load_from_memory(&t.bytes).unwrap();
            assert_eq!(back.width().max(back.height()), 100);
        }
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
