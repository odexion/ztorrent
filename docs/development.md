# Development

Developer notes for ztorrent. For installing and using the app, see the [README](../README.md).

## Running from source

```bash
npm install
npm start          # or: npm run dev   (opens devtools)
```

Then **File ▸ Add Torrent…** (⌘O on macOS, Ctrl+O elsewhere) and pick something from
`sample-torrents/`.

### Building

```bash
./scripts/make-icns.sh    # build/icon.icns from build/icon.png (macOS only)
npm run pack              # unpacked app in dist/
npm run dist              # macOS .dmg, arm64 + x64
npm run dist:win          # Windows NSIS installer, x64 + arm64
npm run dist:linux        # Linux AppImage + .deb, x64 + arm64
npm run dist:all          # all three
```

Windows and Linux icons are generated from `build/icon.png`, so `make-icns.sh` (which
needs `sips` and `iconutil`) is only required for the macOS build.

### Releases

CI builds all three platforms on every push to `main` and attaches the installers to the
run as artifacts. Pushing a `v*` tag additionally publishes them as a GitHub Release:

```bash
npm version patch     # or minor / major — writes package.json and tags
git push --follow-tags
```

That produces `.dmg` (arm64 + x64), `.exe` NSIS installers (x64 + arm64), and
`.AppImage` + `.deb` (x64 + arm64). Everything is unsigned.

The build sets `npmRebuild: false`. Every native dependency in the tree —
`bufferutil`, `utf-8-validate`, `utp-native`, `node-datachannel`,
`fs-native-extensions` — ships prebuilt binaries for each platform that import only
`napi_*` symbols. N-API is ABI-stable across Node and Electron, so those binaries
load in Electron unchanged and `@electron/rebuild` has nothing to do. Leaving it on
just makes node-gyp download Electron headers, which is the one step that needs the
network and the one step that fails behind a slow or filtered connection.

Builds are unsigned — there is no Developer ID here, so electron-builder skips code
signing. A locally built `.app` runs fine; one that has been downloaded will need
`xattr -dr com.apple.quarantine /Applications/ztorrent.app` or a right-click ▸ Open.
Windows SmartScreen will likewise warn about the unsigned installer.

## Sample torrents

`sample-torrents/` ships four well-seeded, freely distributable torrents so the app can be
exercised immediately:

| File | Size | Notes |
|---|---|---|
| `debian-13.6.0-amd64-netinst.iso.torrent` | 755 MB | Huge swarm (100+ seeds), HTTP tracker + two web seeds. Best for watching real throughput. |
| `sintel.torrent` | 123 MB | Blender open movie, 11 files. Good for exercising the Files tab and per-file priorities. |
| `big-buck-bunny.torrent` | 264 MB | Blender open movie. |
| `tears-of-steel.torrent` | 571 MB | Blender open movie. |

## Privacy

Preferences ▸ Connection has three controls worth understanding. All of them are
deliberately fail-closed: anything that cannot be routed under a policy is
switched off rather than sent around it, because a proxy that covers announces
but leaks peer connections costs speed and hides nothing — the address the swarm
records is the one the peers see.

**Protocol encryption** hides the peer handshake from traffic inspection.
*Enabled* negotiates it and falls back to plaintext; *Required* refuses peers that
will not encrypt, which is stricter but shrinks the usable swarm. It does nothing
about tracker or DHT traffic.

**Proxy** routes tracker announces, web-seed fetches and outgoing peer connections
through a SOCKS5 server, resolving hostnames at the proxy so no DNS leaks locally.
Turning it on also disables DHT, local peer discovery, µTP, UPnP/NAT-PMP port
mapping and `udp://` trackers, and closes the inbound listeners — with a proxy,
connections are outgoing only. The password is kept in the system keychain via
Electron's `safeStorage`, not in the settings file; where that is unavailable it
falls back to plaintext rather than losing it.

**Network interface** pins every outgoing connection to one interface's address —
a VPN's, typically. If that interface goes away the connections fail instead of
falling back to your normal one, and resume by themselves when it returns, which
is the kill switch. The address is re-read per connection, so a VPN that
reconnects on a different address is picked up without a restart. Local
discovery, µTP, port mapping and `udp://` trackers stop while this is set; DHT
keeps working, with its socket bound to the same interface.

Either policy needs a restart to take effect, and the log pane says so if you
change one and forget.

`npm test` covers both. `test:egress` exercises the routing against a local
SOCKS5 server and a real interface — asserting, among other things, that a bound
connection reaches the far end from the bound address, and that a dead proxy or a
vanished interface fails the connection rather than falling back to a direct one.
`test:secrets` covers the sealed password, including that a denied keychain does
not destroy it.

## Layout

```
electron/
  main.js       window, native menus, all IPC handlers, dock + notifications
  preload.cjs   the contextBridge surface — the only thing the renderer can see
  engine.js     WebTorrent wrapper: lifecycle, queue, priorities, snapshots
  store.js      atomic JSON persistence for settings and the resume session
renderer/
  index.html    static shell
  styles.css    the µTorrent theme, as CSS custom properties
  app.js        toolbar, sidebar, transfer grid, detail tabs, input handling
  dialogs.js    Add / Add-URL / Create / Preferences / Properties modals
  icons.js      the inline SVG icon set
  util.js       byte, speed, ETA and date formatting
scripts/
  make-icon.mjs generates build/icon.png and build/logo.svg, no image tooling
  make-icns.sh  turns the PNG into build/icon.icns via sips + iconutil
```

The renderer runs sandboxed with context isolation and no Node access; it talks to the
engine only over the named channels in `preload.cjs`. The main process polls the engine
once a second and pushes one combined snapshot, so the UI never blocks on IPC.

### Development flags

```bash
npx electron . --dev                     # detached devtools
npx electron . --add=path/to.torrent     # add a torrent at launch
npx electron . --shot=out.png \
               --shot-delay=20 \
               --shot-tabs --shot-quit   # capture the window (and each tab) headlessly
ZTORRENT_SHOT_EVAL="ztorrentUI.setTab('peers')" npx electron . --shot=out.png
```

`window.ztorrentUI` exposes `doAction`, `setTab`, `select`, `openAdd` and `state()` — the
same actions the toolbar triggers, useful for driving the app in tests.

