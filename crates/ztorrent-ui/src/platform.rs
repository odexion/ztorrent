//! The few things the window does outside itself: the Dock badge and the
//! completion notification.

use gpui::{App, SystemNotification};

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

/// "Download complete", with the torrent's name, through the notification
/// centre GPUI already talks to -- UNUserNotificationCenter on macOS, the
/// portal on Linux, a toast on Windows. notify-rust drove NSUserNotification
/// here, a class current macOS no longer ships at all; worse, reaching for it
/// made the process claim Finder's bundle identifier, so the system stopped
/// recognising ztorrent and asked again for every drive it had already been
/// allowed.
///
/// The torrent's id is the tag, so a torrent that finishes twice replaces its
/// own notification instead of stacking another one.
pub fn notify_complete(id: &str, name: &str, cx: &App) {
    cx.show_system_notification(SystemNotification {
        tag: format!("complete:{id}").into(),
        title: "Download complete".into(),
        body: name.into(),
        actions: Vec::new(),
    });
}
