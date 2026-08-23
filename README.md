# ztorrent

A cross-platform BitTorrent client for macOS, Windows and Linux. Flat surfaces, system
type, a single accent colour, and a tabbed detail pane underneath the transfer list. The
Status column carries its own progress bar, µTorrent-style.

It speaks real BitTorrent: TCP and µTP peers, DHT, PEX, local peer discovery, HTTP and UDP
trackers, magnet links, and HTTP web seeds. Drop a `.torrent` on it and it downloads.

```
┌ native menu bar ───────────────────────────────────────────────┐
├ Add · Add-URL · Create │ Remove │ Start · Pause · Stop │ ▲ ▼ │ …│
├──────────┬─────────────────────────────────────────────────────┤
│ Torrents │ #  Name        Size  ▓▓▓▓ Downloading 62%  Seeds ETA│
│  ├ Down… │────────────────── splitter ───────────────────────  │
│  ├ Seed… │ General│Trackers│Peers│Pieces│Files│Speed│Logger     │
│  └ Label │ …detail pane…                                       │
├──────────┴─────────────────────────────────────────────────────┤
│ ⬤ DHT: 142 nodes │ Torrents: 3 │  D: 21.7 MB/s   U: 68 kB/s     │
└────────────────────────────────────────────────────────────────┘
```

## Running it

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

## What's in the UI

**Toolbar** — Add Torrent, Add from URL, Create New Torrent, Remove, Start, Pause, Stop,
move up/down the queue, alternate speed limits, Preferences, and a live filter box.

**Sidebar** — Torrents / Downloading / Seeding / Completed / Active / Inactive with live
counts, plus a Labels tree.

**Transfer list** — 18 available columns (`#`, Name, Size, Done, Status, Seeds, Peers, Down
Speed, Up Speed, ETA, Downloaded, Uploaded, Ratio, Availability, Label, Added On, Completed
On, Save Path). Click a header to sort, drag its right edge to resize, right-click the
header to choose which columns show. Widths and choices persist.

**Detail tabs**

- **General** — transfer stats on the left, torrent metadata (hash, pieces, comment, creator,
  private flag) on the right.
- **Trackers** — every announce URL plus the pseudo-entries µTorrent shows for DHT, PEX and
  LSD, with live seed/leech counts scraped from announce responses and the next update time.
  Right-click to add a tracker or force an announce.
- **Peers** — address, connection type (TCP/µTP/WebRTC/web seed), flags, client name,
  their completion, and per-peer transfer rates and totals.
- **Pieces** — the full piece map, one cell per piece: green have, amber in flight, grey missing.
- **Files** — per-file size, progress and priority. Right-click for High / Normal / Don't
  Download, or to open the file or reveal it in Finder.
- **Speed** — a 150-second download/upload graph for the selected torrent (or the whole
  session when nothing is selected).
- **Logger** — timestamped engine events.

**Status bar** — DHT node count, torrent count, the active rate caps, and session
down/up speeds with cumulative totals.

## Utilities

| | |
|---|---|
| Add torrent | File, magnet link, `http(s)://…/x.torrent`, bare 40-char info hash, or drag-and-drop |
| Add dialog | Choose save folder and label, tick individual files, start paused, sequential mode |
| Create torrent | Any file or folder; tracker tiers, web seeds, comment, piece size, private flag, and optional immediate seeding |
| Queue | Per-torrent order with move up/down; caps on active torrents and active downloads |
| Bandwidth | Global down/up caps plus a one-key alternate-limits profile (⇧⌘L) |
| Per-file priority | High / Normal / Don't Download, applied live and remembered across restarts |
| Force re-check | Re-hashes every piece against what is on disk |
| Labels | Free-form, shown in the sidebar and as a column |
| Sequential download | Per torrent, for streaming-ish access |
| Incomplete files | Written as `<name>.part` and renamed on completion, so a half-downloaded file is never mistaken for a finished one. An existing `.part` is always resumed, even with the preference off, so toggling it never strands data |
| Session | Torrents, progress, labels, priorities, column layout and window geometry all restore on launch |
| Desktop integration | Completion notifications, `.torrent` file association, `magnet:` URL scheme, native menus and context menus; Dock progress badge on macOS |
| Themes | Light and Dark, switchable in Preferences or View ▸ Appearance |

## Keyboard

| | |
|---|---|
| ⌘O / ⌘U / ⌘N | Add torrent / add from URL / create torrent |
| ⌘R / ⌘P / ⌘. | Start / Pause / Stop |
| ⌘E / ⌘T | Force re-check / update tracker |
| ⌘↑ / ⌘↓ | Move up / down the queue |
| ⌘⌫ / ⇧⌘⌫ | Remove / remove and delete data |
| ⇧⌘C / ⇧⌘O | Copy magnet URI / open containing folder |
| ⌘F | Focus the filter box |
| ⌘1 / ⌘2 / ⌘3 | Toggle sidebar / detail pane / status bar |
| ⇧⌘L | Toggle alternate speed limits |
| Space | Pause or resume the selected torrent |
| ↑ ↓ / ⇧↑ ⇧↓ | Move or extend the selection |

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
  make-icon.mjs generates build/icon.png with no image tooling
  make-icns.sh  turns that into build/icon.icns via sips + iconutil
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

## Licence

MIT. The Blender open-movie torrents are Creative Commons Attribution 3.0; the Debian
image is distributed under Debian's own terms.
