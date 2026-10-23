//! Native file icons (Finder / Explorer look) served to the webview through
//! the `icon://` URI scheme, rendered on a dedicated thread and cached.
//!
//! URL forms: `icon://localhost/ext/<ext>?s=<px>`, `/dir`, `/file`,
//! `/path/<percent-encoded absolute path>`.

#[cfg(target_os = "macos")]
pub mod macos;
#[cfg(windows)]
pub mod windows;

use crossbeam_channel::{unbounded, Sender};
use lru::LruCache;
use parking_lot::Mutex;
use std::num::NonZeroUsize;
use std::sync::{Arc, LazyLock};

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum IconKey {
    Ext(String),
    Dir,
    File,
    Path(String),
}

impl IconKey {
    pub fn parse(url_path: &str) -> Option<IconKey> {
        let p = url_path.trim_start_matches('/');
        let (kind, rest) = p.split_once('/').unwrap_or((p, ""));
        let rest = percent_encoding::percent_decode_str(rest).decode_utf8_lossy().into_owned();
        match kind {
            "ext" if !rest.is_empty() => Some(IconKey::Ext(rest.to_lowercase())),
            "dir" => Some(IconKey::Dir),
            "file" => Some(IconKey::File),
            "path" if !rest.is_empty() => Some(IconKey::Path(rest)),
            _ => None,
        }
    }
}

type Png = Arc<Vec<u8>>;
type Reply = Sender<Option<Png>>;
type IconCache = LruCache<(IconKey, u32), Option<Png>>;

static CACHE: LazyLock<Mutex<IconCache>> =
    LazyLock::new(|| Mutex::new(LruCache::new(NonZeroUsize::new(4096).unwrap())));

static WORKER: LazyLock<Sender<(IconKey, u32, Reply)>> = LazyLock::new(|| {
    let (tx, rx) = unbounded::<(IconKey, u32, Reply)>();
    std::thread::Builder::new()
        .name("icons".into())
        .spawn(move || {
            #[cfg(windows)]
            windows::init_thread();
            for (key, px, reply) in rx {
                let cached = CACHE.lock().get(&(key.clone(), px)).cloned();
                let result = match cached {
                    Some(hit) => hit,
                    None => {
                        let png = render(&key, px).map(Arc::new);
                        CACHE.lock().put((key, px), png.clone());
                        png
                    }
                };
                let _ = reply.send(result);
            }
        })
        .expect("icon thread");
    tx
});

fn render(key: &IconKey, px: u32) -> Option<Vec<u8>> {
    #[cfg(target_os = "macos")]
    {
        macos::render(key, px)
    }
    #[cfg(windows)]
    {
        windows::render(key, px)
    }
    #[cfg(target_os = "linux")]
    {
        crate::linux::render_icon(key, px)
    }
    #[cfg(not(any(target_os = "macos", windows, target_os = "linux")))]
    {
        let _ = (key, px);
        None
    }
}

/// Returns a PNG for the icon, blocking the calling (protocol) thread.
pub fn icon_png(key: IconKey, px: u32) -> Option<Png> {
    let px = px.clamp(16, 512);
    if let Some(hit) = CACHE.lock().get(&(key.clone(), px)) {
        return hit.clone();
    }
    let (tx, rx) = crossbeam_channel::bounded(1);
    WORKER.send((key, px, tx)).ok()?;
    rx.recv().ok().flatten()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_urls() {
        assert_eq!(IconKey::parse("/ext/PDF"), Some(IconKey::Ext("pdf".into())));
        assert_eq!(IconKey::parse("/dir"), Some(IconKey::Dir));
        assert_eq!(IconKey::parse("/path/%2FApplications%2FSafari.app"), Some(IconKey::Path("/Applications/Safari.app".into())));
        assert_eq!(IconKey::parse("/nope"), None);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn renders_native_icons() {
        let png = icon_png(IconKey::Ext("txt".into()), 32).expect("icon");
        assert_eq!(&png[1..4], b"PNG");
        assert!(icon_png(IconKey::Dir, 64).is_some());
    }
}
