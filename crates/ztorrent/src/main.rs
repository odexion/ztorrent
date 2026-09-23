//! Wires the pieces together: where state lives, the secret seal, the engine
//! thread, and the window. The window never sees the engine itself -- only the
//! two channels handed over here.

// A release build on Windows is a window app: no console opens beside it.
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

use std::path::{Path, PathBuf};
use ztorrent_core::Store;
use ztorrent_core::store::STATE_FILE;
use ztorrent_engine::EngineOptions;

const APP_NAME: &str = "ztorrent";

/// Where this build keeps its state.
///
/// A release build uses Electron's userData directory itself, so upgrading
/// carries every torrent, label and preference across. A development build must
/// not touch the library of the Electron app that may be installed beside it:
/// it keeps its own directory and, the first time, imports a copy of that
/// library to have something real to show. ZTORRENT_DATA_DIR overrides both.
fn data_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("ZTORRENT_DATA_DIR") {
        return PathBuf::from(dir);
    }
    let config = dirs::config_dir().unwrap_or_else(|| PathBuf::from("."));
    let shared = config.join(APP_NAME);
    if !cfg!(debug_assertions) {
        return shared;
    }
    let dev = config.join("ztorrent-native-dev");
    import_copy(&shared, &dev);
    dev
}

fn import_copy(from: &Path, to: &Path) {
    let (src, dst) = (from.join(STATE_FILE), to.join(STATE_FILE));
    if dst.exists() || !src.exists() {
        return;
    }
    // The copy comes in with every torrent stopped: their data belongs to the
    // Electron app, and a development build must not start writing into it.
    let Ok(text) = std::fs::read_to_string(&src) else { return };
    let Ok(mut state) = serde_json::from_str::<serde_json::Value>(&text) else { return };
    if let Some(torrents) = state.get_mut("torrents").and_then(|t| t.as_array_mut()) {
        for t in torrents {
            t["state"] = serde_json::Value::String("stopped".into());
        }
    }
    let written = std::fs::create_dir_all(to).and_then(|_| std::fs::write(&dst, serde_json::to_string_pretty(&state).unwrap_or_default()));
    if written.is_ok() {
        eprintln!("[ztorrent] development build: imported a copy of {} with every torrent stopped", src.display());
    }
}

/// Only one instance may own the session directory. A second launch hands its
/// torrents to the first and exits; the first accepts nothing but magnet links
/// and paths to .torrent files that exist. The channel is a socket file in that
/// directory on Unix, and a named pipe named after it on Windows.
#[cfg(unix)]
fn single_instance(dir: &Path, args: &[String], opens: flume::Sender<String>) -> bool {
    use std::io::BufReader;
    use std::os::unix::net::{UnixListener, UnixStream};
    let socket = dir.join("ztorrent.sock");
    if let Ok(mut stream) = UnixStream::connect(&socket) {
        hand_over(&mut stream, args);
        return false;
    }
    let _ = std::fs::remove_file(&socket);
    let _ = std::fs::create_dir_all(dir);
    let Ok(listener) = UnixListener::bind(&socket) else { return true };
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&socket, std::fs::Permissions::from_mode(0o600));
    }
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            receive(BufReader::new(stream), &opens);
        }
    });
    true
}

#[cfg(windows)]
fn single_instance(dir: &Path, args: &[String], opens: flume::Sender<String>) -> bool {
    use interprocess::local_socket::{GenericNamespaced, ListenerOptions, Stream, prelude::*};
    use std::io::BufReader;
    // FNV-1a of the directory, case-folded as Windows paths are. Stable across
    // versions, so a freshly updated copy still finds the one it replaces.
    let key = dir.to_string_lossy().to_lowercase();
    let hash = key.bytes().fold(0xcbf2_9ce4_8422_2325u64, |h, b| (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3));
    let pipe = format!("ztorrent-{hash:016x}");
    if let Ok(mut stream) = pipe.as_str().to_ns_name::<GenericNamespaced>().and_then(Stream::connect) {
        hand_over(&mut stream, args);
        return false;
    }
    let listener = pipe.as_str().to_ns_name::<GenericNamespaced>().and_then(|name| ListenerOptions::new().name(name).create_sync());
    let Ok(listener) = listener else { return true };
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            receive(BufReader::new(stream), &opens);
        }
    });
    true
}

#[cfg(not(any(unix, windows)))]
fn single_instance(_dir: &Path, _args: &[String], _opens: flume::Sender<String>) -> bool {
    true
}

/// What a second launch sends: its files and links, then a request to bring the
/// window forward.
#[cfg(any(unix, windows))]
fn hand_over(stream: &mut impl std::io::Write, args: &[String]) {
    for arg in args.iter().filter(|a| !a.starts_with('-')) {
        // Absolute, because the first instance runs in another directory.
        // Not canonicalised: on Windows that adds a \\?\ prefix.
        let arg = std::path::absolute(arg).map(|p| p.to_string_lossy().into_owned()).unwrap_or_else(|_| arg.clone());
        let _ = writeln!(stream, "{arg}");
    }
    let _ = writeln!(stream, "activate");
}

/// Reads one launch's hand-over. "activate" reaches the window as an empty
/// string; anything that is not a magnet link or an existing .torrent is dropped.
#[cfg(any(unix, windows))]
fn receive(reader: impl std::io::BufRead, opens: &flume::Sender<String>) {
    for line in reader.lines().map_while(Result::ok).take(64) {
        let accepted = line.starts_with("magnet:") || (line.to_ascii_lowercase().ends_with(".torrent") && Path::new(&line).is_file());
        if accepted || line == "activate" {
            let _ = opens.send(if line == "activate" { String::new() } else { line });
        }
    }
}

/// A build installed where the updater can swap it: inside an .app, from an
/// AppImage, or beside the uninstaller the Windows installer writes -- and never
/// a development build, or a release build run from the build directory.
fn is_packaged() -> bool {
    if cfg!(debug_assertions) {
        return false;
    }
    let exe = std::env::current_exe().unwrap_or_default();
    if cfg!(target_os = "macos") {
        exe.to_string_lossy().contains(".app/Contents/MacOS/")
    } else if cfg!(target_os = "linux") {
        std::env::var_os("APPIMAGE").is_some()
    } else if cfg!(windows) {
        exe.parent().is_some_and(|dir| dir.join("uninstall.exe").is_file())
    } else {
        false
    }
}

fn main() -> anyhow::Result<()> {
    // The OpenSSL inside libtorrent is built into the app and knows no
    // distribution's certificate paths: point it at this system's CA bundle.
    #[cfg(target_os = "linux")]
    // SAFETY: the first thing main does, before any thread exists to read the
    // environment concurrently.
    unsafe {
        openssl_probe::try_init_openssl_env_vars();
    }
    let dir = data_dir();
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (open_tx, open_rx) = flume::unbounded::<String>();
    if !single_instance(&dir, &args, open_tx.clone()) {
        return Ok(());
    }
    let codec = ztorrent_secrets::os_codec(APP_NAME, &dir);
    let store = Store::open(&dir, codec);
    let version = env!("CARGO_PKG_VERSION").to_string();
    // Update checks leave the way torrent traffic does: through the proxy, on
    // the bound interface, or not at all. The policy in force is the one read
    // at start-up, as it is for the engine.
    let policy = ztorrent_core::egress::EgressPolicy::from_settings(store.settings());
    let client_version = version.clone();
    let (update_tx, update_rx) = flume::unbounded();
    let updates = ztorrent_updater::spawn(ztorrent_updater::Options {
        dir: dir.join("updates"),
        current: version.clone(),
        events: update_tx,
        packaged: is_packaged(),
        client: Box::new(move || ztorrent_engine::http_client(policy.as_ref(), &client_version)),
    });
    let engine = ztorrent_engine::spawn(store, EngineOptions { data_dir: dir.clone(), version: version.clone() })?;

    // Terminated from outside -- a logout, `kill`, Ctrl+C in a terminal -- the
    // engine still writes its state and resume data before the process goes.
    let on_signal = engine.commands.clone();
    ctrlc::set_handler(move || {
        let (tx, mut rx) = futures_channel::oneshot::channel();
        let _ = on_signal.send(ztorrent_core::Command::Shutdown(tx));
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(8);
        while std::time::Instant::now() < deadline {
            match rx.try_recv() {
                Ok(Some(())) | Err(_) => break,
                Ok(None) => std::thread::sleep(std::time::Duration::from_millis(20)),
            }
        }
        std::process::exit(0);
    })?;

    ztorrent_ui::run(ztorrent_ui::RunOptions {
        commands: engine.commands.clone(),
        events: engine.events.clone(),
        updates,
        update_events: update_rx,
        opens: (open_tx, open_rx),
        version,
        args,
    });

    // The window has closed and the app is quitting: the engine has already
    // been told to shut down and has written its state.
    let _ = engine.thread.join();
    Ok(())
}
