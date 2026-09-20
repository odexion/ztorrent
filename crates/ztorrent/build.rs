//! Windows only: embeds the app icon and version details in ztorrent.exe, so
//! Explorer, the taskbar and the Start menu show them.

fn main() {
    println!("cargo:rerun-if-changed=icons/icon.ico");
    println!("cargo:rerun-if-env-changed=ZTORRENT_SKIP_NATIVE");
    // ZTORRENT_SKIP_NATIVE=1: a `cargo check` from another platform, which has no
    // resource compiler for this.
    let skip = std::env::var("ZTORRENT_SKIP_NATIVE").as_deref() == Ok("1");
    if skip || std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    let mut res = winresource::WindowsResource::new();
    res.set_icon("icons/icon.ico")
        .set("ProductName", "ztorrent")
        .set("FileDescription", "ztorrent")
        .set("InternalName", "ztorrent")
        .set("OriginalFilename", "ztorrent.exe");
    res.compile().expect("embedding the Windows resources");
}
