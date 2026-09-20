//! The few things the window does outside itself: the Dock badge and the
//! completion notification.

/// The Dock badge: the percentage of what is downloading, or nothing.
#[cfg(target_os = "macos")]
pub fn set_badge(text: Option<&str>) {
    use objc2::MainThreadMarker;
    use objc2_app_kit::NSApplication;
    use objc2_foundation::NSString;
    let Some(mtm) = MainThreadMarker::new() else { return };
    let tile = NSApplication::sharedApplication(mtm).dockTile();
    let label = text.map(NSString::from_str);
    tile.setBadgeLabel(label.as_deref());
}

#[cfg(not(target_os = "macos"))]
pub fn set_badge(_text: Option<&str>) {}

/// Keeps the window's own chrome -- the title bar, native menus and sheets -- in
/// the theme the user chose rather than the system's, as Electron's
/// nativeTheme.themeSource did.
#[cfg(target_os = "macos")]
pub fn set_appearance(dark: bool) {
    use objc2::MainThreadMarker;
    use objc2_app_kit::{NSAppearance, NSAppearanceNameAqua, NSAppearanceNameDarkAqua, NSApplication};
    let Some(mtm) = MainThreadMarker::new() else { return };
    // SAFETY: the appearance names are constant NSStrings exported by AppKit.
    let name = unsafe { if dark { NSAppearanceNameDarkAqua } else { NSAppearanceNameAqua } };
    let appearance = NSAppearance::appearanceNamed(name);
    NSApplication::sharedApplication(mtm).setAppearance(appearance.as_deref());
}

#[cfg(not(target_os = "macos"))]
pub fn set_appearance(_dark: bool) {}

/// "Download complete", with the torrent's name. Shown off the main thread:
/// the system call can take a moment and the window should not wait on it.
pub fn notify_complete(name: String) {
    std::thread::spawn(move || {
        let _ = notify_rust::Notification::new().summary("Download complete").body(&name).appname("ztorrent").show();
    });
}
