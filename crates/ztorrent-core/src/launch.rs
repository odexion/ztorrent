//! Which downloaded files run code when the system opens them. A torrent names
//! its own files, so "Open" on one is a program starting as often as a video
//! playing; the window asks first when it is the former.

use std::path::Path;

/// Extensions the platforms run, install or mount rather than view. Compared
/// case-insensitively, and on every platform: a Windows program in a torrent is
/// just as much a program when the torrent is opened on a Mac.
const RUNS_CODE: &[&str] = &[
    // Windows
    "exe", "com", "scr", "pif", "msi", "msp", "msix", "appx", "bat", "cmd", "ps1", "psm1", "vbs", "vbe", "js", "jse",
    "wsf", "wsh", "hta", "cpl", "msc", "lnk", "url", "reg", "inf", "scf", "chm", "jar", "application", "gadget",
    // macOS
    "app", "command", "tool", "pkg", "mpkg", "dmg", "workflow", "scpt", "scptd", "applescript", "terminal", "webloc",
    // Linux and Unix
    "sh", "bash", "zsh", "csh", "ksh", "run", "bin", "desktop", "appimage", "deb", "rpm", "flatpakref", "snap",
    // Interpreted scripts a file manager may hand to their interpreter
    "py", "pyw", "pl", "rb", "php",
];

/// True for a file (or bundle) that runs code when opened.
pub fn runs_code(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| RUNS_CODE.iter().any(|x| x.eq_ignore_ascii_case(e)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn programs_are_told_apart_from_media() {
        assert!(runs_code(Path::new("/d/Movie.2024.mp4.exe")));
        assert!(runs_code(Path::new("/d/Setup.MSI")));
        assert!(runs_code(Path::new("/d/Player.app")));
        assert!(runs_code(Path::new("/d/install.command")));
        assert!(runs_code(Path::new("/d/tool.AppImage")));
        assert!(!runs_code(Path::new("/d/Movie.2024.mp4")));
        assert!(!runs_code(Path::new("/d/readme.txt")));
        assert!(!runs_code(Path::new("/d/no-extension")));
    }
}
