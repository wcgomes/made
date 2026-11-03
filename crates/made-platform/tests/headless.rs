//! Run separately with --no-default-features. Desktop defaults are unchanged.
#[cfg(not(feature = "desktop"))]
#[test]
fn headless_clipboard_and_png_are_explicitly_unsupported() {
    use made_platform::{ClipboardImage, PlatformError, SystemClipboard};
    assert!(matches!(
        SystemClipboard::new(),
        Err(PlatformError::Unsupported(_))
    ));
    let image = ClipboardImage::new(1, 1, vec![0; 4]).unwrap();
    assert!(matches!(image.to_png(), Err(PlatformError::Unsupported(_))));
}
