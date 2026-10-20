//! File tags. On macOS these are the Finder tags (shared with Finder and
//! Spotlight); elsewhere there is no system-wide equivalent yet.

/// Tag names of a file ("Red", "Projekt X" …).
#[cfg(target_os = "macos")]
pub fn tags_of(path: &str) -> Vec<String> {
    use std::ffi::CString;
    let Ok(p) = CString::new(path) else { return Vec::new() };
    let name = c"com.apple.metadata:_kMDItemUserTags";
    let len = unsafe { libc::getxattr(p.as_ptr(), name.as_ptr(), std::ptr::null_mut(), 0, 0, 0) };
    if len <= 0 || len > 1 << 20 {
        return Vec::new();
    }
    let mut buf = vec![0u8; len as usize];
    let n = unsafe { libc::getxattr(p.as_ptr(), name.as_ptr(), buf.as_mut_ptr().cast(), buf.len(), 0, 0) };
    if n <= 0 {
        return Vec::new();
    }
    buf.truncate(n as usize);
    let Ok(plist::Value::Array(items)) = plist::from_bytes::<plist::Value>(&buf) else { return Vec::new() };
    // Entries look like "Red\n6" (name, newline, colour index).
    items.into_iter().filter_map(|v| v.into_string()).map(|s| s.split('\n').next().unwrap_or("").to_string()).filter(|s| !s.is_empty()).collect()
}

#[cfg(not(target_os = "macos"))]
pub fn tags_of(_path: &str) -> Vec<String> {
    Vec::new()
}
