# ztorrent, native

The Rust + GPUI rebuild of ztorrent, following
[docs/rust-gpui-migration.md](../docs/rust-gpui-migration.md). It lives beside the
Electron app, which is untouched: `electron/`, `renderer/`, `package.json` and
`.github/workflows/build.yml` build and ship exactly as before.

## Building

The app is standalone: libtorrent 2.1.1 and OpenSSL 3 are compiled from source
and linked in, so nothing needs installing where it runs. Building needs a C++
compiler, Perl and `make` (OpenSSL's build), `curl` and `tar`, and Boost's headers.

```bash
brew install boost                     # macOS; Debian/Ubuntu: apt install libboost-dev
cd native
cargo build -p ztorrent                # the app
cargo test -p ztorrent-core -p ztorrent-secrets -p ztorrent-lt-sys \
           -p ztorrent-engine -p ztorrent-updater -p ztorrent-ui
```

- The first build downloads libtorrent's release tarball and checks its SHA-256
  (`crates/ztorrent-lt-sys/build.rs`); `LIBTORRENT_SRC` points at an unpacked copy
  for offline builds. libtorrent is always built optimised, even in a debug build.
- Boost's headers are looked for under `/opt/homebrew/include`, `/usr/local/include`
  and `/usr/include`; set `BOOST_INCLUDE` if they are elsewhere.
- On Linux, GPUI also needs its window-system headers: see the package list in
  `.github/actions/native-setup/action.yml`.
- On Windows: Visual Studio's C++ tools, Strawberry Perl, and Boost's headers
  with `BOOST_INCLUDE` pointing at them.
- `.cargo/config.toml` builds for macOS 11 and later.

## Packaging

```bash
cargo install cargo-packager --locked --version 0.11.8
cargo build --release -p ztorrent --target aarch64-apple-darwin
cargo packager --release -p ztorrent --target aarch64-apple-darwin --formats dmg
sh scripts/check-standalone.sh aarch64-apple-darwin   # fails on any non-system library
sh scripts/rename-artifacts.sh target/packages 0.5.0
```

The formats per platform are `dmg` (macOS), `nsis` (Windows) and `appimage,deb`
(Linux). The Mac app is signed ad hoc, as the Electron releases were not signed
either. The Windows installer removes an Electron install of ztorrent first,
never its data.

## Releasing

`.github/workflows/native.yml` tests every push on macOS, Linux and Windows. Run
it by hand to build all eight installers as artifacts (macOS arm64 and x64,
Windows x64 and arm64, and an AppImage and `.deb` each for Linux x86_64 and
arm64). Push a tag to publish them:

```bash
git tag v0.5.0 && git push origin v0.5.0
```

From v0.5.0 the releases are this build; `build.yml` no longer runs for tags.
The installers carry the names electron-builder used, so the Electron app's
updater and `scripts/install.sh` move existing installs across. A tag with a
suffix (`v0.6.0-beta.1`) publishes a prerelease, which both of them skip.

On Windows, the Electron updater runs the installer without `/R`, so after
that one update ztorrent has to be started by hand; later updates restart it.

## Running

```bash
./target/debug/ztorrent                                  # your library (see below)
ZTORRENT_DATA_DIR=/tmp/zt ./target/debug/ztorrent        # a scratch library
./target/debug/ztorrent path/to.torrent                  # opens the Add sheet
./target/debug/ztorrent --action=ztorrent::Preferences   # dispatches a named action
ZTORRENT_DEBUG_UI=1 ./target/debug/ztorrent              # traces the window's flow
```

### Your Electron library is safe

A **release** build uses Electron's own data directory, so upgrading carries every
torrent, label and preference across. A **development** build never does: it
keeps `ztorrent-native-dev` beside it and, the first time, imports a *copy* of
the Electron library with **every torrent stopped** — their data belongs to the
Electron app, and nothing writes into it until you start a torrent yourself.
`ZTORRENT_DATA_DIR` overrides both.

The proxy password is sealed the way Electron's `safeStorage` sealed it (the
`ztorrent Safe Storage` Keychain item on macOS), so either build opens what the
other wrote.

## Layout

| Crate | What it is |
|---|---|
| `ztorrent-core` | Settings, the state file, formatting, columns, the egress policy, and the command set — the only thing the window can ask of the engine |
| `ztorrent-secrets` | The proxy password seal, compatible with Electron's |
| `ztorrent-lt-sys` | The cxx bridge to libtorrent-rasterbar; all the unsafe code |
| `ztorrent-engine` | The engine thread: session, queue, `.part` files, fail-closed egress, resume data |
| `ztorrent-updater` | Release check, download, staging and the swap script |
| `ztorrent-ui` | The window on GPUI; depends on `ztorrent-core`, never on the engine |
| `ztorrent` | The binary that wires them together |

## Where it stands

Working and tested:

- Engine: add (file, URL, magnet, bytes), inspect, start/pause/stop/recheck/remove,
  queue limits, seed goals, labels, file priorities, sequential, trackers, peers,
  `.part` naming and renaming on completion, resume data, state file compatible
  with Electron's in both directions.
- Egress: SOCKS5 with remote DNS for peers, trackers and fetches; a dead proxy fails
  instead of going direct; inbound TCP closed under a proxy; interface binding with
  a kill switch that holds every torrent while the interface is gone.
- A real download between two engines over loopback, directly and through a proxy.
- Updater: asset matching, staging, resume, the swap script's outcomes.
- Window: toolbar, sidebar with categories, labels and drop targets, the torrent
  grid (sort, resize, column chooser, Finder-style selection, drag to label,
  context menus), all seven detail tabs, status bar, alternate-speed wash, app
  menu with live check marks, keyboard, file drop, the update pill, Dock badge,
  completion notification, single instance, clean shutdown on SIGTERM.
- Sheets: Add New Torrent, Add from URL, Create, Preferences, Properties,
  Customize Label, and the generic prompt.

Not done yet:

- Checked by hand only in part: the Preferences pages other than General, the app
  menu and the context menus have not been looked at on screen.
- An offscreen `--shot` built on GPUI's headless renderer.
- Windows has not been built yet: its code (the named-pipe single instance, the
  installer hand-off in the updater, the icon, the Electron uninstall in the
  NSIS installer) waits for the first CI run on a Windows runner.
- Checked locally: the macOS DMGs (arm64 run, x64 inspected but not run, since
  this Mac has no Rosetta), and the Linux arm64 AppImage and `.deb`, installed
  and started in a clean Ubuntu 22.04 container. Linux x86_64 and Windows are
  CI's to check.
- Notification click-to-open, dropped magnet text, and the native CI workflow's
  first run.
