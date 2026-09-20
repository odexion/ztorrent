# Development

Developer notes for ztorrent. For installing and using the app, see the
[README](../README.md).

ztorrent is a Rust workspace at the root of this repository: `cargo build` and
`cargo test` work from here with no subdirectory to step into. It was an
Electron app up to v0.4.1; that code is gone from the tree, and the v0.4.1 tag
is the last checkout that has it. How the rebuild was done is written up in
[rust-gpui-migration.md](rust-gpui-migration.md).

## Building

The app is standalone: libtorrent 2.1.1 and OpenSSL 3 are compiled from source
and linked in, so nothing needs installing where it runs. Building needs a C++
compiler, Perl and `make` (OpenSSL's build), `curl` and `tar`, and Boost's headers.

```bash
brew install boost                     # macOS; Debian/Ubuntu: apt install libboost-dev
cargo run -p ztorrent                  # the app
cargo test                             # the workspace
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
sh scripts/rename-artifacts.sh target/packages 0.5.3
```

The formats per platform are `dmg` (macOS), `nsis` (Windows) and `appimage,deb`
(Linux). The Mac app is signed ad hoc, as the Electron releases were not signed
either. The Windows installer removes an Electron install of ztorrent first,
never its data.

The app icon comes from `build/icon.png` and `build/icon.icns`, which
`scripts/make-icon.mjs` and `scripts/make-icns.sh` generate. Both are plain
Node and shell — there is no npm install here.

## Releasing

`.github/workflows/native.yml` tests every push on macOS, Linux and Windows. Run
it by hand to build all eight installers as artifacts (macOS arm64 and x64,
Windows x64 and arm64, and an AppImage and `.deb` each for Linux x86_64 and
arm64). Push a tag to publish them:

```bash
git tag v0.5.3 && git push origin v0.5.3
```

The installers keep the names electron-builder used, so an Electron install's
updater and `scripts/install.sh` move it across. A tag with a suffix
(`v0.6.0-beta.1`) publishes a prerelease, which both of them skip.

On Windows, the Electron updater runs the installer without `/R`, so after
that one update ztorrent has to be started by hand; later updates restart it.

### Signing

The Windows app and installer are signed through
[SignPath Foundation](https://signpath.org), which is free for open-source
projects, so that SmartScreen does not call them unknown. For a release tag,
the package job uploads `ztorrent.exe`, waits for SignPath to sign it,
packages it, then does the same for the installer. Until the settings below
exist, Windows builds are published unsigned, as before.

Once the project is accepted on signpath.io:

1. Add GitHub.com as a trusted build system, link it to the project, and
   install the SignPath GitHub App on this repository.
2. Paste `signing/artifact-configuration.xml` into the project's default
   artifact configuration.
3. In the repository's Actions settings, add the secret
   `SIGNPATH_API_TOKEN` (a submitter's API token) and the variables
   `SIGNPATH_ORGANIZATION_ID`, `SIGNPATH_PROJECT_SLUG` and
   `SIGNPATH_POLICY_SLUG` (`release-signing` for the foundation's certificate).

A release-signing request waits for an approver to accept it on signpath.io,
twice per Windows architecture; each wait times out after an hour. The policy
the foundation requires is at the end of the top-level README, and the
release notes link to it.

## Running

```bash
cargo run -p ztorrent                                    # your library (see below)
ZTORRENT_DATA_DIR=/tmp/zt cargo run -p ztorrent          # a scratch library
cargo run -p ztorrent -- path/to.torrent                 # opens the Add sheet
cargo run -p ztorrent -- --action=ztorrent::Preferences  # dispatches a named action
ZTORRENT_DEBUG_UI=1 cargo run -p ztorrent                # traces the window's flow
```

`--action=` takes the names in `crates/ztorrent-ui/src/actions.rs`, and
`--action-delay=N` waits N seconds first, so the first rows can arrive. That is
the scripting surface for driving the window without a mouse.

### An Electron library is still safe

A **release** build uses Electron's own data directory, so upgrading from
v0.4.1 or earlier carries every torrent, label and preference across. A
**development** build never does: it keeps `ztorrent-native-dev` beside it and,
the first time, imports a *copy* of the Electron library with **every torrent
stopped** — their data belongs to the Electron app, and nothing writes into it
until you start a torrent yourself. `ZTORRENT_DATA_DIR` overrides both.

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

CI enforces the one rule that matters here: `ztorrent-ui` must not be able to
name the engine. Its only way in is the command set in `ztorrent-core`.
