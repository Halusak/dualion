use super::IconKey;
use objc2::rc::autoreleasepool;
use objc2::AllocAnyThread;
use objc2_app_kit::{NSBitmapImageFileType, NSBitmapImageRep, NSColor, NSColorSpace, NSImage, NSWorkspace};
use objc2_foundation::{NSDictionary, NSPoint, NSRect, NSSize, NSString};

fn image_for(key: &IconKey) -> objc2::rc::Retained<NSImage> {
    let ws = NSWorkspace::sharedWorkspace();
    #[allow(deprecated)]
    match key {
        IconKey::Ext(ext) => ws.iconForFileType(&NSString::from_str(ext)),
        IconKey::Dir => ws.iconForFileType(&NSString::from_str("public.folder")),
        IconKey::File => ws.iconForFileType(&NSString::from_str("public.data")),
        IconKey::Path(p) => ws.iconForFile(&NSString::from_str(p)),
    }
}

pub fn image_png(image: &NSImage, px: u32) -> Option<Vec<u8>> {
    let mut rect = NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(px as f64, px as f64));
    let cg = unsafe { image.CGImageForProposedRect_context_hints(&mut rect, None, None) }?;
    let rep = NSBitmapImageRep::initWithCGImage(NSBitmapImageRep::alloc(), &cg);
    let data = unsafe { rep.representationUsingType_properties(NSBitmapImageFileType::PNG, &NSDictionary::new()) }?;
    Some(data.to_vec())
}

pub fn render(key: &IconKey, px: u32) -> Option<Vec<u8>> {
    autoreleasepool(|_| {
        let image = image_for(key);
        image_png(&image, px)
    })
}

/// System accent colour as `#rrggbb`.
pub fn accent_color() -> Option<String> {
    autoreleasepool(|_| {
        let c = NSColor::controlAccentColor().colorUsingColorSpace(&NSColorSpace::sRGBColorSpace())?;
        let to = |v: f64| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
        Some(format!("#{:02x}{:02x}{:02x}", to(c.redComponent()), to(c.greenComponent()), to(c.blueComponent())))
    })
}
