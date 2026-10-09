//! The few things the window does outside itself: the Dock badge, the
//! completion notification, and the system's file and link handlers.

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

/// Keeps Launch Services pointed at this copy of the app, and hands it magnet
/// links when it already opens .torrent files. Off the main thread: neither
/// needs the window, and registration can take a moment.
#[cfg(target_os = "macos")]
pub fn claim_handlers() {
    std::thread::spawn(handlers::run);
}

#[cfg(not(target_os = "macos"))]
pub fn claim_handlers() {}

#[cfg(target_os = "macos")]
mod handlers {
    use objc2::rc::Retained;
    use objc2::runtime::NSObjectProtocol;
    use objc2::sel;
    use objc2_app_kit::NSWorkspace;
    use objc2_foundation::{NSBundle, NSString, NSURL, NSUserDefaults};
    use objc2_uniform_type_identifiers::UTType;
    use std::ffi::{CStr, c_char, c_void};

    const BUNDLE_ID: &str = "dev.zaf4.ztorrent";
    const TORRENT_TYPE: &str = "org.bittorrent.torrent";
    /// Set once the user has been asked about magnet links, so a "no" is not
    /// met with the same question every launch.
    const ASKED: &str = "ZTMagnetHandlerAsked";

    #[link(name = "CoreServices", kind = "framework")]
    unsafe extern "C" {
        fn LSRegisterURL(url: *const c_void, update: u8) -> i32;
    }

    unsafe extern "C" {
        fn dlsym(handle: *mut c_void, symbol: *const c_char) -> *mut c_void;
    }

    /// A Launch Services function looked up at run time. macOS 12 deprecated
    /// the ones used on 11; linking them directly would stop the app from
    /// starting on whichever release finally drops them.
    fn ls_fn(name: &CStr) -> Option<*mut c_void> {
        const RTLD_DEFAULT: *mut c_void = -2isize as *mut c_void;
        // SAFETY: dlsym with RTLD_DEFAULT and a NUL-terminated name only reads.
        let f = unsafe { dlsym(RTLD_DEFAULT, name.as_ptr()) };
        (!f.is_null()).then_some(f)
    }

    fn cf(s: &NSString) -> *const c_void {
        s as *const NSString as *const c_void
    }

    fn bundle_id(app: &NSURL) -> Option<String> {
        NSBundle::bundleWithURL(app)?.bundleIdentifier().map(|s| s.to_string())
    }

    /// Which app opens .torrent files.
    fn torrent_handler(ws: &NSWorkspace) -> Option<String> {
        if ws.respondsToSelector(sel!(URLForApplicationToOpenContentType:)) {
            let ty = UTType::typeWithIdentifier(&NSString::from_str(TORRENT_TYPE))?;
            return ws.URLForApplicationToOpenContentType(&ty).and_then(|u| bundle_id(&u));
        }
        let f = ls_fn(c"LSCopyDefaultRoleHandlerForContentType")?;
        // SAFETY: LSCopyDefaultRoleHandlerForContentType(CFStringRef, LSRolesMask)
        // returns a +1 CFStringRef or NULL; NSString is toll-free bridged.
        unsafe {
            let f: extern "C" fn(*const c_void, u32) -> *mut NSString = std::mem::transmute(f);
            let id = f(cf(&NSString::from_str(TORRENT_TYPE)), 0xFFFF_FFFF);
            Retained::from_raw(id).map(|s| s.to_string())
        }
    }

    fn magnet_handler(ws: &NSWorkspace) -> Option<String> {
        let url = NSURL::URLWithString(&NSString::from_str("magnet:?xt=urn:btih:0"))?;
        ws.URLForApplicationToOpenURL(&url).and_then(|u| bundle_id(&u))
    }

    fn set_magnet_handler(ws: &NSWorkspace, app: &NSURL) {
        let scheme = NSString::from_str("magnet");
        if ws.respondsToSelector(sel!(setDefaultApplicationAtURL:toOpenURLsWithScheme:completionHandler:)) {
            ws.setDefaultApplicationAtURL_toOpenURLsWithScheme_completionHandler(app, &scheme, None);
        } else if let Some(f) = ls_fn(c"LSSetDefaultHandlerForURLScheme") {
            // SAFETY: LSSetDefaultHandlerForURLScheme(CFStringRef, CFStringRef) -> OSStatus.
            unsafe {
                let f: extern "C" fn(*const c_void, *const c_void) -> i32 = std::mem::transmute(f);
                f(cf(&scheme), cf(&NSString::from_str(BUNDLE_ID)));
            }
        }
    }

    pub fn run() {
        let bundle = NSBundle::mainBundle();
        // Only the installed app: a cargo-run build is a bare binary, and must
        // never register itself or take over magnet links.
        if bundle.bundleIdentifier().is_none_or(|id| id.to_string() != BUNDLE_ID) {
            return;
        }
        let app = bundle.bundleURL();

        // Re-register this copy, so its document icon and type claims are the
        // ones Launch Services serves -- not those of a staged update, a disk
        // image long since ejected, or the build an update just replaced.
        // SAFETY: NSURL is toll-free bridged to CFURLRef.
        unsafe { LSRegisterURL(&*app as *const NSURL as *const c_void, 1) };

        let ws = NSWorkspace::sharedWorkspace();
        let defaults = NSUserDefaults::standardUserDefaults();
        let asked = NSString::from_str(ASKED);
        if torrent_handler(&ws).as_deref() != Some(BUNDLE_ID) {
            // Not the .torrent app (any more): ask afresh if that changes.
            defaults.removeObjectForKey(&asked);
            return;
        }
        if magnet_handler(&ws).as_deref() == Some(BUNDLE_ID) || defaults.boolForKey(&asked) {
            return;
        }
        defaults.setBool_forKey(true, &asked);
        set_magnet_handler(&ws, &app);
    }
}
