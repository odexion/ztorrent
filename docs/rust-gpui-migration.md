# Moving ztorrent to Rust and GPUI

A plan to rebuild ztorrent as a native Rust application on GPUI, one-to-one with the
Electron app as of 0.4.1 (`47b4b23`): every feature, every screen and interaction,
every utility, and every privacy and security guarantee — plus the preferences
system and the path existing users take from one to the other.

"One-to-one" is meant literally. Each row in the tables below is a behaviour the
current code has, with the file it lives in. Where the new stack cannot match a
behaviour, the row says so and says what happens instead. Where the current
behaviour is a bug, it is listed under [Quirks](#15-quirks-keep-or-fix) with a
recommendation, never silently changed.

Ecosystem facts were checked on 2026-09-14 against crates.io and the upstream
sources; they are marked **verified**. Anything that still needs proving on real
hardware is marked **spike** and has a gate in [Phases](#13-phases-and-gates).

---

## Contents

1. [Decisions](#1-decisions)
2. [The stack](#2-the-stack)
3. [Architecture](#3-architecture)
4. [Workspace layout and file-by-file map](#4-workspace-layout-and-file-by-file-map)
5. [Engine parity](#5-engine-parity)
6. [Options: how preferences work](#6-options-how-preferences-work)
7. [Persistence and migrating existing users](#7-persistence-and-migrating-existing-users)
8. [Privacy and egress: fail closed](#8-privacy-and-egress-fail-closed)
9. [Security model](#9-security-model)
10. [UI and UX, region by region](#10-ui-and-ux-region-by-region)
11. [Platform integration](#11-platform-integration)
12. [Updater, packaging, installer, CI](#12-updater-packaging-installer-ci)
13. [Phases and gates](#13-phases-and-gates)
14. [Testing and the parity checklist](#14-testing-and-the-parity-checklist)
15. [Quirks: keep or fix](#15-quirks-keep-or-fix)
16. [Risks and open questions](#16-risks-and-open-questions)

---

## 1. Decisions

| # | Decision | Recommendation | Why |
|---|---|---|---|
| D1 | BitTorrent engine | **libtorrent-rasterbar 2.0.x behind our own thin `cxx` bridge** | The only engine that covers every current feature: MSE encryption with a *required* mode, SOCKS5 for peers **and** trackers with remote DNS, interface binding, web seeds, per-file priorities, sequential/rarest-first, torrent creation. librqbit 9.0.1 (**verified**) lacks MSE (rqbit #633 open), web seeds (#500 open), priorities beyond on/off, rarest-first (sequential only) and `.part` naming. |
| D2 | UI framework | **`gpui-component =0.6.1`**, which brings **`gpui-pre 0.3.x`** (Zed's published GPUI snapshot) | **Verified:** the `gpui` crate on crates.io is stuck at 0.2.2 (Oct 2025). `gpui-pre` is published from zed-industries/zed (0.3.5 is a snapshot of `zed@d89e9c2`, 2026-09-14), and `gpui-component` 0.6.1 (Apache-2.0, 2026-09-09) depends on it. Pin both exactly; upgrade in dedicated commits. |
| D3 | Where the code lives | Same repo. A Cargo workspace at the root next to `electron/` until cutover, then one commit deletes the Electron tree | Both CI pipelines run side by side; history and lessons stay in one place. |
| D4 | Process model | One process. The GPUI main thread, one engine thread that owns the libtorrent session, and a small task for the updater | Mirrors `main.js` + renderer: the UI never blocks on the engine, and it only reaches the engine through one typed command set. |
| D5 | Compatibility promise | Rust reads the Electron state file in place and writes a **superset** of it; Electron 0.4.x can still open a file the Rust build wrote | This makes downgrading safe for the first Rust release. |
| D6 | Release path | Betas as GitHub **prereleases** (the Electron updater reads `/releases/latest`, so it skips them). The first stable Rust release is an ordinary release that the existing 0.4.x updater installs by itself | No manual reinstall for anyone. |

Open decisions for the maintainer are collected in [§16](#16-risks-and-open-questions).

---

## 2. The stack

| Concern | Electron today | Rust | Notes |
|---|---|---|---|
| Window, rendering, input | Electron 43 + Chromium | `gpui-pre` (Metal / DirectX 11 / Vulkan) | Linux now needs a Vulkan driver; see [§16](#16-risks-and-open-questions). |
| Widgets | Hand-written DOM + CSS | `gpui-component` for inputs, select, checkbox, switch, tooltip, scrollbar, dialog shell, native menus. **Custom** grid, splitters, canvases | **Verified** in the source: `table/` (sortable/resizable/movable columns), `resizable/`, `dialog/`, `select`, `input/`, `tab/`, `native_menu/` (macOS and Windows native popups, drawn fallback on Linux), `chart/`, `selectable_text`. |
| BitTorrent | WebTorrent 3 (+ utp-native, node-datachannel) | libtorrent-rasterbar 2.0.x, statically linked, through `ztorrent-lt-sys` (cxx) | See the gap list in [§5](#5-engine-parity): WebRTC peers. |
| .torrent parse / create | parse-torrent, create-torrent | libtorrent `load_torrent_*`, `parse_magnet_uri`, `create_torrent` (**v1-only** flag) | v1-only keeps infohashes and output identical to create-torrent. |
| Chunk store / `.part` | `part-store.js` over fs-chunk-store | libtorrent storage + `renamed_files` / `rename_file` | [§5.4](#54-part-files). |
| Egress | Monkey patches on `net.connect`, `Torrent._drain`, undici dispatcher, `DHT.listen` | libtorrent `settings_pack` (proxy, `outgoing_interfaces`, `listen_interfaces`) + a policy-aware `reqwest` client | No monkey patching: routing becomes configuration. |
| HTTP (torrent URL, add-inspect) | global `fetch` (patched) | `reqwest` + rustls, built from the egress policy | These fetches **must** follow the policy, as they do today. |
| HTTP (updater) | `electron.net.fetch` (Chromium stack, not patched) | Separate plain `reqwest` + rustls client | Direct connection, as today. |
| Secrets | `safeStorage` (Keychain / DPAPI / libsecret) | GPUI `read/write/delete_credentials` (**verified** on all three platforms) | [§7.2](#72-the-proxy-password). |
| Settings / state | `store.js` JSON | `serde` + `serde_json`, same file | [§7](#7-persistence-and-migrating-existing-users). |
| Interfaces list | `os.networkInterfaces()` | `if-addrs` | |
| File dialogs | `dialog.showOpenDialog/SaveDialog` | `rfd` (title, button label, filters, starting directory, create-directory) | GPUI's `prompt_for_paths` has no filters or starting directory; `rfd` matches every option used today. |
| Message boxes | `dialog.showMessageBox` | GPUI `window.prompt(level, message, detail, buttons)` | Native sheet on macOS. |
| App menu | `Menu` | GPUI `set_menus` with `MenuItem::Action { checked }` (**verified**) | |
| Context menus | `Menu.popup` | `gpui_component::NativeMenu` (native on macOS and Windows, drawn on Linux) | |
| Notifications | `Notification` | `mac-notification-sys` / `tauri-winrt-notification` / `notify-rust` behind one trait | **Spike:** click-to-open on an unsigned macOS bundle. |
| Dock badge / progress | `app.dock.setBadge`, `win.setProgressBar` | `objc2-app-kit` `NSDockTile` | |
| Single instance | `requestSingleInstanceLock` | `interprocess` local socket (per-user name) | [§11](#11-platform-integration). |
| Clipboard | `clipboard` / `navigator.clipboard` | GPUI `write_to_clipboard` / `read_from_clipboard` | |
| Reveal / open | `shell.showItemInFolder/openPath` | GPUI `reveal_path` / `open_with_system` | |
| Packaging | electron-builder | `cargo-packager` (dmg, nsis, appimage, deb) | Artifact names kept byte-for-byte; [§12](#12-updater-packaging-installer-ci). |
| Icon generation | `scripts/make-icon.mjs`, `make-icns.sh` | `cargo xtask icons` (same geometry constants) | Removes Node from the toolchain. |
| Logging | in-memory 800 lines | same ring buffer + `tracing` to stderr in dev | |

---

## 3. Architecture

```
┌──────────────────────────── ztorrent (one process) ────────────────────────────┐
│                                                                                 │
│  GPUI main thread                              engine thread                    │
│  ┌───────────────────────────┐   Command  ┌──────────────────────────────────┐ │
│  │ ztorrent-ui               │ ─────────▶ │ ztorrent-engine                  │ │
│  │  Workspace, Toolbar,      │  (flume)   │  Engine { records, queue,        │ │
│  │  Sidebar, TorrentGrid,    │            │           store, egress, log }   │ │
│  │  DetailPane, StatusBar,   │ ◀───────── │  owns lt::session (via lt-sys)   │ │
│  │  Dialogs, Menus           │   Event    │  1 Hz: Snapshot + Globals        │ │
│  │                           │ (async-    │        + Details(selected id)    │ │
│  │  Model entities:          │  channel)  │  alerts → record updates         │ │
│  │   AppState, Settings,     │            └──────────────────────────────────┘ │
│  │   Selection, UpdateState  │                                                  │
│  └───────────┬───────────────┘            updater task (reqwest, fs, spawn)     │
│              │ platform glue: dock, notifications, single-instance, open-url    │
└──────────────┴──────────────────────────────────────────────────────────────────┘
```

- **The command set is the allow-list.** Every channel in `preload.cjs` becomes one
  variant of `ztorrent_core::Command` (queries answer through a oneshot inside the
  command). Every pushed event (`tick`, `log`, `changed`, `menu`, `open-torrent`,
  `settings-changed`, `update`) becomes one variant of `Event`. `ztorrent-ui` depends
  on `ztorrent-core` only, never on `ztorrent-engine` or `ztorrent-lt-sys`, and a CI
  check enforces that.
- **The ticker stays in the engine.** Once a second it runs `enforce_seed_goals()` and
  sends `Event::Tick { rows, globals, details }`, with details only for the torrent
  the UI last named through `Command::Details(id)` (today's `selectedId`). The UI
  keeps the rule from `ca31eb6`: drop a details payload whose id is not the current
  first selection.
- **libtorrent is driven, not waited on.** The engine thread loops on
  `wait_for_alert(250 ms)` and drains the command channel between waits. Every second
  it calls `post_torrent_updates()` and `post_session_stats()`, and builds the tick
  from the replies.
- **Models are GPUI entities.** `Settings`, `AppState` (rows, globals, labels, label
  styles, log, histories), `Selection` and `UpdateState` are `Entity<T>`, and views
  `cx.observe` them. Lesson 4 ("every path that changes state must refresh every
  view") stops being a discipline and becomes structural: the toolbar button,
  status-bar pill, Options menu checkbox and Preferences all observe the one
  `Settings` entity. The menu is rebuilt from it because `set_menus` takes a value.

---

## 4. Workspace layout and file-by-file map

```
Cargo.toml                    workspace, pinned versions, [workspace.lints]
crates/
  ztorrent-core/              no I/O dependencies on GPUI or libtorrent
    command.rs  event.rs      the allow-list (was preload.cjs)
    settings.rs               Settings + defaults + effective-value helpers
    store.rs                  atomic debounced JSON, legacy Murmur adoption
    secrets.rs                SecretCodec trait (credentials / plaintext fallback)
    record.rs  state.rs       TorrentRecord, State, Row, Globals, Details
    fmt.rs                    bytes, speed, eta, pct, ratio, datetime, clock, duration, compare_text
    columns.rs                column model + migration
    version.rs                compare_versions
  ztorrent-lt-sys/            cxx bridge + build.rs (cmake, static libtorrent)
    include/bridge.h  src/bridge.cpp  src/lib.rs
  ztorrent-engine/
    engine.rs                 lifecycle, add, commands, persist, snapshot (was engine.js)
    queue.rs                  pump_queue, move_queue, enforce_seed_goals
    part_files.rs             (was part-store.js)
    egress.rs                 policy → settings_pack + reqwest client (was egress.js)
    peers.rs                  flags, connection labels, client names
    create.rs  inspect.rs     create torrent, inspect source
  ztorrent-updater/           (was updater.js) check, download, stage, apply script
  ztorrent-platform/          dock, notifications, single instance, file dialogs, open-urls
  ztorrent-ui/
    app.rs  workspace.rs  theme.rs  icons.rs  actions.rs  keymap.rs  menus.rs
    toolbar.rs  sidebar.rs  grid/{mod,columns,cells,progress_bar}.rs  status_bar.rs
    detail/{general,trackers,peers,pieces,files,speed,logger}.rs
    dialogs/{modal,prompt,confirm,add_torrent,add_url,label_style,create_torrent,preferences,properties}.rs
    drag.rs  splitter.rs  shot.rs
  ztorrent/                   the binary: argv, bootstrap, wiring (was main.js bootstrap)
assets/icons/*.svg            emitted by xtask from the icon table
xtask/                        icons, theme-check, golden-fmt
tests/fixtures/               Electron state files, sample torrents
```

| Electron file | Goes to | Notes |
|---|---|---|
| `electron/main.js` window, ticker, bootstrap | `ztorrent/src/main.rs`, `ztorrent-ui/src/app.rs`, `ztorrent-engine` ticker | |
| `electron/main.js` IPC handlers | `Command` handlers in `ztorrent-engine/engine.rs`; UI-only ones (dialogs, clipboard, reveal) in `ztorrent-ui` / `ztorrent-platform` | |
| `electron/main.js` menus, context menus | `ztorrent-ui/menus.rs` | [§10.9](#109-menus) |
| `electron/main.js` `--add/--shot` hooks | `ztorrent/src/main.rs` + `ztorrent-ui/shot.rs` | [§14](#14-testing-and-the-parity-checklist) |
| `electron/preload.cjs` | `ztorrent-core/command.rs`, `event.rs` | |
| `electron/engine.js` | `ztorrent-engine/{engine,queue,peers}.rs` | |
| `electron/egress.js` | `ztorrent-engine/egress.rs` | |
| `electron/part-store.js` | `ztorrent-engine/part_files.rs` | |
| `electron/store.js` | `ztorrent-core/{store,settings,secrets}.rs` | |
| `electron/updater.js` | `ztorrent-updater` | |
| `renderer/index.html` | `ztorrent-ui/workspace.rs` | |
| `renderer/app.js` | `ztorrent-ui/{toolbar,sidebar,grid,status_bar,detail/*,drag,splitter,actions,keymap}.rs` | |
| `renderer/dialogs.js` | `ztorrent-ui/dialogs/*` | |
| `renderer/styles.css` | `ztorrent-ui/theme.rs` (tokens) + per-view styling | [§10.1](#101-theme-tokens) |
| `renderer/icons.js` | `assets/icons/*.svg` + `ztorrent-ui/icons.rs` (`TAG_SYMBOLS`, `TAG_COLORS`, `tag_style`) | |
| `renderer/util.js` | `ztorrent-core/fmt.rs` | Golden tests against the JS output. |
| `scripts/test-egress.mjs`, `socks-server.mjs` | `crates/ztorrent-engine/tests/egress.rs` + a test SOCKS5 server | |
| `scripts/test-secrets.mjs` | `crates/ztorrent-core/tests/secrets.rs` | |
| `scripts/test-inbound.mjs` | `crates/ztorrent-engine/tests/inbound.rs` | |
| `scripts/make-icon.mjs`, `make-icns.sh` | `xtask icons` | |
| `scripts/install.sh` | unchanged (asset names are kept) | |
| `package.json` `build` | `Packager.toml` / `[package.metadata.packager]` | |
| `.github/workflows/build.yml` | rewritten, same shape | [§12.4](#124-ci) |

---

## 5. Engine parity

### 5.1 Records, states, lifecycle

| Behaviour (engine.js) | Rust |
|---|---|
| States `downloading seeding paused stopped queued checking metadata finished error` | `enum State`, serialised as the same lowercase strings |
| Record fields (id UUID, infoHash, name, magnetURI, torrentFile b64, savePath, label, order, addedOn, completedOn, uploadedBase, downloadedBase, length, wanted, priorities, sequential, pieceLength, pieceCount, bitfield, files, announce, webSeeds, meta, state, progress) | `TorrentRecord` with `#[serde(rename_all = "camelCase")]`, same names and encodings (base64 kept), unknown fields preserved |
| `restoreSession`: `wantStart = state ∉ {stopped, paused}`; spawn those, pump the queue; log `Restored N torrent(s) from the previous session.` | Same. The first start of a migrated torrent rechecks ([§7.3](#73-resume-data)) |
| `persist()` snapshots live data (bitfield, files, announce, url-list, meta) onto the record, so the tabs keep working when stopped (`de7243e`) | `snapshot_live(record, &torrent_status, &torrent_info)` before every persist, stop and quit |
| `add(source)`: magnet, http(s) URL, path, raw bytes; parse; duplicate by infoHash (`"X" is already in the list.` → `{duplicate}`); record `QUEUED`; `wantStart = !paused && startTorrentsAutomatically`; `_rememberLabel` | Same; the URL fetch uses the **policy-aware** reqwest client |
| `_spawn`: the source is torrentFile, otherwise the magnet; strategy; upload slots; part store; `so` = wanted | `add_torrent_params`: `ti` or magnet, `sequential_download` flag, `max_uploads`, `file_priorities`, `renamed_files` (part plan), `save_path`, not auto-managed, resume data if present |
| `metadata` event → name, length, torrentFile, apply priorities, persist | `metadata_received_alert` → same; torrent bytes from `write_torrent_file_buf` |
| `ready` → state from done/paused | `torrent_checked_alert` / state update |
| `done` → completedOn, SEEDING, commit part files, log `"X" finished downloading.`, emit complete, pump | `torrent_finished_alert` → same order |
| `error` → ERROR + message + log | `torrent_error_alert`, `file_error_alert` |
| Tracker stats per announce URL | `announce_entry.endpoints[].info_hashes[v1]` scrape_complete / scrape_incomplete |
| Warning filter (drops tracker/socket chatter) | Same regex over alert messages |
| `start`: resume + reselect when live, otherwise QUEUED + pump; log `Started "X".` | `resume()`, else queue |
| `pause`: pause + `_haltTransfer` hack; PAUSED; `wantStart=false` | `pause()`. libtorrent's pause really stops transfer, so the hack is deleted. **Visible difference:** peers disconnect rather than idling |
| `stop`: snapshot, fold counters into bases, release the swarm, STOPPED, pump | `save_resume_data` → store the blob → `remove_torrent(h)` (keep files) → STOPPED |
| `remove(deleteData)`: release with destroyStore; if not released, `rm -rf savePath/name` | `remove_torrent(h, delete_files)`; fallback delete **with a containment check** ([§9](#9-security-model)) |
| `recheck`: live → CHECKING, rescan, log result; stopped → start | `force_recheck()`; stopped → add, then recheck |
| `setLabel`, `_rememberLabel` | Same |
| `setSequential` | `set_flags / unset_flags(sequential_download)` |
| `setFilePriority` 0/1/2 | `file_priority(i, 0 / 4 / 7)`; stored as 0/1/2 |
| `addTracker` (live only), `addPeer` (log accepted/rejected), `reannounce` | `add_tracker`, `connect_peer` (parse `ip:port`, `[v6]:port`), `force_reannounce` + `force_dht_announce` |
| `moveQueue` swaps `order` | Same |
| `enforceSeedGoals` (ratio % / 100, minutes, `shareRatio` uses length when nothing was downloaded) | Same function, called from the tick |
| `pumpQueue` (maxActiveTorrents, maxActiveDownloads, order) | Ported as is. Torrents are **not** auto-managed, so libtorrent's queue never fights ours |
| `_applyConnBudget` shares `globalMaxConnections` out between torrents | **Deleted.** libtorrent's `connections_limit` is already global, which is what the setting always meant |
| Cache budget in bytes (24 MiB) | libtorrent 2.0 uses memory-mapped I/O; there is no equivalent knob. Measure RSS instead ([§14](#14-testing-and-the-parity-checklist)) |
| `destroy()`: persist, flush, destroy client | On quit: `save_resume_data` for all (5 s cap), persist, flush, `session.abort()` |
| Log ring of 800 lines with level | Same, `Event::Log(line)` |

### 5.2 Snapshot row (per torrent, 1 Hz)

| Field | Source |
|---|---|
| `state` derivation (metadata / paused / seeding / downloading unless checking) | `torrent_status.state`, `flags & paused`, `has_metadata` |
| `size`, `wantedSize` (skips priority-0 files) | `ti.total_size()`, sum over `file_priorities` |
| `done` | `status.progress` (live) / `record.progress` |
| `downloaded`, `uploaded` | `all_time_download`, `uploadedBase + all_time_upload` (same base arithmetic) |
| `downloadSpeed`, `uploadSpeed` | `download_payload_rate`, `upload_payload_rate` |
| `numPeers`, `seeds`, `peers` | `num_peers`, `num_seeds`, `num_peers − num_seeds` |
| `eta` | `(total_wanted − total_wanted_done) / download_payload_rate`, in **ms** (see quirk Q1) |
| `ratio` | `share_ratio(downloaded, uploaded, length)` |
| `availability` | Sum of peer-bitfield ratios, from `get_peer_info` + `piece_availability`. `distributed_copies` is close but not identical; keep the sum for parity |
| `magnetURI` | `make_magnet_uri(h)` / record |

### 5.3 Details (selected torrent only)

| Pane data | Source |
|---|---|
| Comment, created by/on, piece length/count, private, sequential | `torrent_info` / record `meta` when stopped |
| Pieces map (2 have / 1 partial / 0 missing), `have` | `status.pieces` + `get_download_queue()` for partial; saved bitfield when stopped |
| Trackers: `[Local Peer Discovery]`, `[Peer Exchange]`, `[DHT]` (Working/Bootstrapping, node count), announce URLs with status `Not contacted yet / Not working / Working / Announcing…`, seeds, peers, interval; web seeds `Web Seed` | `trackers()` endpoints: `last_error` → Not working, `updating` → Announcing…, `scrape_*`; `min_announce`/`next_announce` for the interval column; `url_seeds()`; DHT from session stats |
| Peers: address, port, client, flags `DUCIWP`, progress, rates, totals, connection label `TCP in/out`, `µTP in/out`, `Web Seed` | `peer_info`: `ip`, `client` (sanitised to printable ASCII, peer-id table fallback), flags from `choked/interesting/remote_*`, `connection_type`, `flags & utp_socket`, `local_connection` |
| Files: index, name, path, length, downloaded, progress, priority | `file_progress(piece_granularity)`, record priorities; record `files` when stopped |

**Gap — WebRTC peers.** WebTorrent in Node talks to browser peers over WebRTC via
`wss://` trackers (`CONN_LABEL.webrtc`). libtorrent 2.0.x does not do WebTorrent.
libtorrent's development line carries an experimental WebTorrent build option,
not in a stable release. **Spike S5** decides between building it and documenting
the drop. `wss://` trackers stay listed, shown as `Not working`.

### 5.4 Part files

The per-file plan from `part-store.js` is kept exactly:

| On disk at add | Use |
|---|---|
| final name exists | final |
| `<name>.part` exists | `.part` (even with the setting off) |
| neither | `partFiles` setting decides |

The plan is passed as `add_torrent_params.renamed_files`. On
`torrent_finished_alert`, each `.part` file gets `rename_file(i, final)`, and the
engine waits for `file_renamed_alert` / `file_rename_failed_alert`. It logs
`Finalised N file(s) for "X".` or `Could not rename .part files for "X": …`, and
seeding continues. `ENOENT` (a zero-length file never written) counts as done.

**Visible difference:** for files set to *Don't Download*, libtorrent keeps
boundary pieces in a hidden `.<infohash>.parts` file in the save folder.

### 5.5 Create torrent

`create_torrent(file_storage, piece_size, flags = v1_only)`, trackers as tiers,
url seeds, comment, private, creator, `set_piece_hashes` with a progress callback.
The dialog can then show real progress instead of the static "Hashing…". The file is
written to the chosen path, and **Start seeding** adds it with
`savePath = parent(input)`, not paused. Log: `Created torrent "X" -> path`.

### 5.6 Globals

`downloadSpeed/uploadSpeed` (session payload rates), cumulative
`downloaded/uploaded` summed from records (as today), `ratio`, `dhtReady`,
`dhtNodes` (`dht.dht_nodes` counter), `listenPort` (`session.listen_port()`),
`torrentCount`, `altSpeed`.

---

## 6. Options: how preferences work

### 6.1 Principles carried over (lessons §5)

- **Defaults live in one place:** `impl Default for Settings`, field for field with
  `DEFAULT_SETTINGS`.
- **Every field is `#[serde(default)]`,** so a pre-feature file and a corrupt value
  both load (a corrupt *file* loads as empty, as `_read` does).
- **Unknown keys are preserved** with `#[serde(flatten)] extra: Map<String, Value>`,
  so a downgrade or a newer build loses nothing.
- **Session-only state is not written:** `alt_speed_enabled` is
  `#[serde(skip_serializing)]` and forced to `false` on load.
- **One helper reads the effective value:** `Settings::throttle_rates()` returns the
  pair in force, with `0 ⇒ unlimited` everywhere. Nothing else reads the four rate
  fields.
- **Sentinels stay consistent:** 0 = unlimited for every rate, 0 = forever for the
  seed goals, 0 = random for the port.
- **Changing `downloadPath` clears `lastSavePath`,** detected as an actual change of
  value in the patch, exactly as `settings:set` does.
- **Writes are atomic and debounced:** 400 ms debounce, temp file + `fsync` + rename,
  `flush()` on quit.
- **Every change is one `Settings` update,** and every view observes it.

### 6.2 Every setting

Legend for **Apply**: *live* means applied at once through `apply_settings`;
*restart* means it takes effect at the next launch, and the log says so, with the
same wording as today.

| Key | Default | Preferences page / control | libtorrent / effect | Apply |
|---|---|---|---|---|
| `downloadPath` | `~/Downloads` | Directories · path + Browse… | default `save_path` | live |
| `lastSavePath` | `''` | (internal) | offered by the Add / Add-URL sheets | live |
| `askWhereToSave` | `true` | General · "Show the Add Torrent dialog" | **currently read by nothing** — quirk Q9 | — |
| `startTorrentsAutomatically` | `true` | General | `wantStart` on add | live |
| `sequentialDownload` | `false` | General | default for new torrents | live |
| `partFiles` | `true` | General + hint | part plan for new files | live |
| `maxDownloadRate` / `maxUploadRate` | 0 | Bandwidth · kB/s | `download_rate_limit` / `upload_rate_limit` = kB × 1024 via `throttle_rates` | live |
| `altDownloadRate` / `altUploadRate` | 100 / 20 | Bandwidth | same, when alternate is on | live |
| `altSpeedEnabled` | false, session only | Bandwidth checkbox, toolbar, status pill, Options menu, ⌘⇧L | `throttle_rates` | live |
| `globalMaxConnections` | 200 (min 10) | Bandwidth | `connections_limit` | live |
| `maxUploadSlots` | 4 (min 1) | Bandwidth | `set_max_uploads` per torrent | live |
| `listenPort` / `randomizePort` | 0 / true | Connection | `listen_interfaces = "0.0.0.0:P,[::]:P"` (P = 0 when random) | restart |
| `enableUPnP` | true | Connection | `enable_upnp`, `enable_natpmp` | restart |
| `enableDHT` | true | Connection | `enable_dht` | restart |
| `enablePEX` | true | Connection | ut_pex plugin; `disable_pex` torrent flag when off | restart |
| `enableLSD` | **false** | Connection + hint | `enable_lsd` | restart |
| `enableUTP` | true | Connection | `enable_outgoing_utp`, `enable_incoming_utp` | restart |
| `encryption` | 1 | Connection · select + hint | `out_enc_policy` / `in_enc_policy` = `pe_disabled / pe_enabled / pe_forced`, `allowed_enc_level = pe_both` | live (the current code reads `client.secure` per dial) |
| `proxyEnabled`, `proxyHost`, `proxyPort` (1080), `proxyUsername`, `proxyPassword` | off | Connection · Proxy + hint | [§8](#8-privacy-and-egress-fail-closed) | restart |
| `bindInterface` | `''` | Connection · select (`name — address`, `— not present`) + hint | [§8](#8-privacy-and-egress-fail-closed) | restart |
| `maxActiveTorrents` / `maxActiveDownloads` | 8 / 5 | Queueing | `pump_queue` | live |
| `seedRatioLimit` (%) / `seedTimeLimit` (min) | 0 / 0 | Queueing | `enforce_seed_goals` | live |
| `autoUpdate` | true | General · Updates + hint | updater start/stop (packaged builds only) | live |
| `theme` | `classic` | Appearance · Light/Dark (live preview, reverted on Cancel); View menu radios; ⌘L | theme + native appearance | live |
| `confirmOnDelete` | true | General | remove confirmation | live |
| `notifyOnComplete` | true | General | notification | live |
| `showSpeedInDock` | true | General | dock badge (macOS) | live |

Other persisted UI state: `labels`, `labelStyles { name: { symbol, color } }`
(deleted, not stored, when reset to default), `columns { order, widths }` (with the
`done` removal and Seeds/Peers migration), `window { x, y, width, height }`.

### 6.3 Preferences sheet behaviour

- Six pages in a left nav (General, Directories, Connection, Bandwidth, Queueing,
  Appearance) with the same icons. Apply / Cancel; Enter applies; Escape and a click
  outside cancel.
- **Suppressed options are greyed** while the boxes above them say they cannot apply:
  DHT under a proxy; LSD, µTP and UPnP under a proxy **or** a bind. The rule lives in
  `EgressPolicy::suppressed(&draft)`, shared with the engine, so the sheet and the
  engine cannot disagree.
- Apply sends the whole patch. `settings:set` semantics are kept: detect a policy
  change → log the restart notice, `apply_settings`, sync the alt-speed menu, apply
  the theme, start/stop the updater.
- Number fields clamp to their `min` and turn `NaN` into 0, as `Number(v) || 0` does.

---

## 7. Persistence and migrating existing users

### 7.1 The state file

| | |
|---|---|
| Location | `dirs::config_dir()/ztorrent/ztorrent-state.json`. That is Electron's `userData` exactly: `~/Library/Application Support/ztorrent`, `%APPDATA%\ztorrent`, `$XDG_CONFIG_HOME/ztorrent` |
| Legacy | If the file is absent, adopt `murmur-state.json` in the same directory or in `../Murmur/`, and leave the old file untouched |
| Compatibility | Read everything Electron writes; write the same keys with the same encodings; keep unknown keys. **Golden test:** a real 0.4.1 state file round-trips byte-equivalent (modulo key order) |
| Extras | libtorrent resume blobs go to `userData/resume/<infohash>.resume` (atomic write). Electron ignores them, so a downgrade just rechecks |
| Permissions | Written `0600` on Unix (hardening H3) |
| Chromium leftovers | `Cache/`, `GPUCache/`, `Local Storage/` and so on in `userData` are left alone in the first Rust release (downgrade stays possible) and removed one release later |

### 7.2 The proxy password

1. **New storage:** GPUI `write_credentials("ztorrent://proxy", username, password)`
   → Keychain / Windows Credential Manager / Secret Service. The settings file records
   `proxyPasswordInCredentials: true`, never the value.
2. **Migration from `proxyPasswordEnc`** (sealed by Electron `safeStorage`). The
   Chromium `os_crypt` format is documented:
   - macOS: `v10` + AES-128-CBC. The key is PBKDF2-SHA1 of the "`ztorrent Safe Storage`"
     Keychain password (salt `saltysalt`, 1003 iterations).
   - Windows: `v10` + AES-256-GCM, with the key from `Local State`
     `os_crypt.encrypted_key` (DPAPI).
   - Linux: `v11` with the libsecret key, or `v10` with the fixed key.

   **Spike S6** verifies it against real sealed values on all three platforms. On
   success the password moves to the credential store. **`proxyPasswordEnc` is left in
   the file for one release**, so Electron 0.4.x can still open it after a downgrade.
3. **A denied or unavailable store must not destroy a secret** (the lesson from
   `test-secrets.mjs`). An unreadable value is empty in memory, and the sealed or
   stored value is preserved on the next save. Only an explicit new value (including
   `''`) replaces it. With no credential store at all (a Linux desktop without Secret
   Service), fall back to plaintext as today and log that once.
4. If migration fails, log `Could not read the saved proxy password; enter it again in
   Preferences ▸ Connection.` and keep the sealed value.

### 7.3 Resume data

WebTorrent verifies pieces on add, and there is no resume file to import. So the
first Rust launch shows **Checking** for each non-stopped torrent while libtorrent
rehashes. That takes seconds for the samples and minutes for very large libraries.
From then on, libtorrent resume data (saved on stop, on quit, and every 5 minutes
when anything changed) makes starts instant. Stopped torrents keep their saved
bitfield for the Pieces tab and recheck when started.

`.part` files written by Electron resume in place, thanks to the part plan in
[§5.4](#54-part-files).

### 7.4 Columns and window

The column migration (`migrateColumns`, `done` filtered out) moves to
`ztorrent-core/columns.rs` with the same conditional, idempotent rule and its tests.
Window bounds are restored; **new:** clamped to a visible display (Electron did not
do this).

---

## 8. Privacy and egress: fail closed

The rule does not change: **a partial guarantee is worse than none.** Anything that
cannot be routed under a policy is switched off, never sent around it.

### 8.1 Policy

```rust
pub struct EgressPolicy { pub proxy: Option<ProxyConfig>, pub bind: Option<String> }
// proxy_config(): None unless enabled, host non-blank, port > 0 (same as egress.js)
// PartialEq replaces samePolicy()
```

### 8.2 Every channel that carries the address outward

| Channel | No policy | Proxy | Bind (e.g. `utun4`) |
|---|---|---|---|
| Outgoing peer TCP | direct | `proxy_peer_connections = true`, SOCKS5 (`socks5` / `socks5_pw`) | `outgoing_interfaces = "utun4"` |
| HTTP(S) tracker announce | direct | `proxy_tracker_connections = true`, `proxy_hostnames = true` (DNS at the proxy) | bound |
| `udp://` trackers | direct | **stripped** from every announce list (add, magnet `tr=`, Add Tracker…) | **stripped** |
| Web seeds | direct | proxied (they are peer connections) | bound |
| DHT | per setting | **off** | **on**, on sockets bound through `listen_interfaces = "utun4:P"` |
| LSD | per setting | **off** | **off** |
| µTP | per setting | **off** | **off** |
| UPnP / NAT-PMP | per setting | **off** | **off** |
| Inbound listeners | open | **closed**: `enable_incoming_tcp = false`, `enable_incoming_utp = false` (replaces `closeInbound`; no race with `listening`) | open, bound |
| .torrent URL fetch, Add-sheet inspect | direct | `reqwest` `Proxy::all("socks5h://…")` | `reqwest` `local_address(addr)`, re-read each request; missing → `EEGRESSDOWN "bound interface X has no address -- refusing to connect"` |
| Updater | direct | direct (as today) | direct (as today) |

The first Rust release keeps these semantics as they are: the same switches off,
`udp://` stripped rather than tried through SOCKS5 UDP ASSOCIATE. Loosening them
later is a separate, deliberate change.

### 8.3 The kill switch

- **Linux:** libtorrent binds outgoing sockets with `SO_BINDTODEVICE`. When the device
  goes, connections fail.
- **macOS / Windows:** libtorrent binds to the interface's *address*. A watcher polls
  `if-addrs` every second (same cadence as `bindAddress`'s cache). On an address
  change it calls `session.reopen_network_sockets()`, so a VPN that comes back on a
  new address resumes by itself.

**Spike S3** must show on macOS (utun) and Windows (a Wintun/WireGuard adapter) that a
vanished interface fails connections **without falling back** to the physical NIC.
If libtorrent falls back on either platform, the engine pauses every torrent while
the interface has no address, and resumes them when it returns. That keeps the
guarantee even if it is enforced one layer up.

### 8.4 Log lines kept verbatim

- `Requiring protocol encryption: peers that will not encrypt are refused. This shrinks the pool of usable peers.`
- `Trackers, web seeds and peer connections go out via SOCKS5 host:port (authenticated), bound to utun4 (10.x.x.x).`
- `DHT, local discovery, uTP, port mapping, udp:// trackers are off while this is on -- none of them can be routed.`
- `Inbound listeners closed: with a proxy, connections are outgoing only.`
- The two restart notices from `settings:set`.

---

## 9. Security model

| Electron guarantee | Rust equivalent |
|---|---|
| Renderer sandbox, `contextIsolation`, no Node | No web renderer, no JavaScript, no remote content: the whole XSS/CSP class is gone. The boundary becomes **compile-time**: `ztorrent-ui` cannot name the engine or libtorrent, only `Command` / `Event`. CI asserts it (`cargo metadata` check). |
| Preload allow-list for invokes **and events** | The `Command` and `Event` enums *are* the list. Adding a capability means adding a variant, which is visible in review. |
| CSP `default-src 'none'` | Not applicable; no resources are ever loaded from outside the binary. |
| `esc()` on every user string | GPUI text is never parsed as markup, so labels with quotes cannot break layout (lesson 5). |
| Secrets in the OS store; a denied store keeps them | [§7.2](#72-the-proxy-password). Held in `secrecy::SecretString`, never in `Debug`, never logged, zeroised on drop. |
| Egress fail-closed, with tests | [§8](#8-privacy-and-egress-fail-closed) + the ported test suite. |
| Single instance owns the session directory | Per-user local socket or named pipe (ACL: current user). The second instance forwards argv; the receiver **accepts only** `magnet:` URIs and paths to existing `.torrent` files. |
| Updater refuses unpackaged runs | Refuses unless the executable sits inside an installed bundle/AppImage/install dir **and** it is a release build. |
| Updater shell script quoting | Same POSIX single-quote escaping, pid validation (`case "$pid" in ''\|0\|*[!0-9]*`), move-aside rollback. |

New code paths introduce new risks. These are the mitigations:

- **FFI:** all `unsafe` sits in `ztorrent-lt-sys`. Every C++ call is wrapped in
  `try/catch` → `Result`. No panics cross the boundary (`catch_unwind` at callback
  edges). The engine thread runs under a supervisor that logs and shows an error
  state rather than exiting silently.
- **Untrusted inputs parsed in C++** (.torrent, magnet, peer messages): libtorrent at
  a pinned, current 2.0.x release. Explicit `load_torrent_limits`: max buffer 10 MiB,
  max pieces, decode depth and tokens. Fuzz our own inspect/add path with
  `cargo-fuzz` over the bridge.
- **Paths built from torrent data** (reveal, open, delete fallback): every path is
  `save_path.join(sanitised)`, canonicalised and checked with
  `starts_with(save_path)`. Names containing `..` or separators are refused.
  **This fixes a real hole:** `removeTorrent`'s fallback `rm -rf path.join(savePath, name)` trusts `name` (quirk Q8).
- **Supply chain:** committed `Cargo.lock`; `cargo-deny` (advisories, licences,
  duplicate and unknown sources) and `cargo-audit` in CI. libtorrent is fetched at a
  tag with a pinned SHA-256. GitHub Actions are pinned by commit SHA. `gpui-pre` and
  `gpui-component` versions are exact.
- **Hardening beyond parity (recommended, each a separate decision):**
  - H1: verify a `SHA256SUMS` + minisign signature before staging an update. The
    updater runs downloaded code, and today only TLS protects it.
  - H2: set `com.apple.quarantine` on files a torrent completes (macOS), as browsers do.
  - H3: state file mode `0600`.
  - H4: libtorrent `anonymous_mode` offered as an option under a proxy.

---

## 10. UI and UX, region by region

Visual parity means the same tokens, metrics, copy, states and interactions. The
reference is the Electron app captured with `--shot --shot-tabs` in both themes
([§14](#14-testing-and-the-parity-checklist)).

### 10.1 Theme tokens

- `theme.rs` carries **every** custom property from `styles.css`, `:root` (classic)
  and `[data-theme="graphite"]`, with the same names in snake_case: `bg`, `chrome`,
  `sidebar`, `sunken`, `raised`, `line`, `line_soft`, `line_hard`, `divider`, `ink`,
  `ink_dim`, `ink_faint`, `accent`, `accent_hover`, `accent_soft`, `hover`, `pressed`,
  `sel`, `sel_blur`, `sel_quiet`, `bar_track`, `bar`, `bar_seed`, `bar_fin`,
  `bar_idle`, `bar_err`, `bar_label`, `bar_label_on`, `piece_partial`, `ok`, `fin`,
  `warn`, `warn_soft`, `alt_wash`, `alt_edge`, `err`, `down`, `up`, the `ic_*` aliases,
  the eight `tag_*`, `chart_down_fill`, `chart_up_fill`, `shadow_modal`.
- **Aliases stay aliases** (`ic_downloading = down`, and so on): computed from their
  targets, never duplicated hex (lesson 4).
- `xtask theme-check` parses the CSS from git history (`47b4b23:renderer/styles.css`)
  and fails if a token value drifts.
- **Fonts:** system UI font (`.SystemUIFont` on macOS, Segoe UI on Windows, the
  system sans on Linux), 13 px base. Mono: SF Mono / Cascadia Mono / DejaVu Sans Mono.
  Numeric columns use the `tnum` font feature.
- `gpui-component`'s `Theme` is populated from these tokens, so its inputs, selects
  and scrollbars wear the same palette.

### 10.2 Window

1180×760 default, minimum 820×480, title `ztorrent`. The background colour is set
from the theme before first paint (no white flash in dark). Bounds are saved on
resize and move, not while minimised. The throttled-window wash is the last child of
the root, above dialogs: a full-window `alt_wash` fill with a 1 px `alt_edge` inset
ring, no mouse handlers (so every click lands underneath), and a 160 ms opacity
transition.

### 10.3 Toolbar

48 px tall, 12 px side padding. Buttons are 32×32 with radius 6 and 18 px icons, in
this order: `add-file add-url create | remove | start pause stop | queue-up
queue-down | alt-speed preferences`, then a spacer and the search box.

- Hover: `hover`, ink. Pressed: `pressed`. Disabled: opacity 0.3, no events.
  `remove/start/pause/stop` need a selection; the queue buttons need exactly one row.
- Alt-speed active: `warn_soft` background with `warn` ink, never the blue.
- Tooltip `Name  (accelerator)`, spelled per platform (`⇧⌘L` / `Ctrl+Shift+L`,
  `⌘⌫` / `Del`). Alt-speed has the stateful tooltip text from `altSpeedHint`.
- Search: 200×30, `sunken` background, 30 px left padding, 15 px icon with stroke
  2.1, placeholder `Filter torrents`. On focus: `bg`, `accent` border and a 3 px
  `accent_soft` ring. Filtering is debounced by 120 ms. Escape clears and blurs.
  It matches name or label.

### 10.4 Sidebar

- Width 216 by default; the splitter clamps it to 120–400 and highlights `accent` on
  hover. Background `sidebar` with a right hairline, padding 10/8.
- "Torrents" is the root (weight 500), followed by children with category colours:
  Downloading (`ic_downloading`), Seeding, Completed, Active, Inactive.
- A "LABELS" group title (11 px, 600, 0.04 em tracking, `ink_faint`) holds
  No Label, the labels (symbol and colour from `tag_style`, falling back to
  `label`/`slate`) and the quiet **New Label…** item.
- Rows are 30 px, radius 6, gap 8, 16 px icons, children indented 16, counts shown as
  `(N)` in 12 px `ink_faint` tabular figures. Selection uses `sel_quiet` and weight
  500 **without recolouring the icon**.
- Category filters, `isActive` and the counts match `categoryFilter()` exactly.
- Right-click a label → native menu `Customize "name"…` → Customize Label sheet.
- A drop target gets `accent_soft` plus a 2 px inset `accent` ring (no layout shift),
  and its count turns accent. Accepts: No Label, labels, New Label… (prompts).

### 10.5 Torrent grid

- A custom virtualised grid on GPUI `uniform_list`, not `gpui-component`'s Table:
  Finder-exact multi-selection and dragging rows out are not what that component
  models.
- Columns (key, label, default width): `# 44`, `Name 360`, `Size 78`, `Status 220`,
  `Down Speed 110`, `Up Speed 88`, `ETA 72`, `Seeds 68`, `Peers 68`,
  `Downloaded 86`, `Uploaded 86`, `Ratio 68`, `Avail. 68`, `Label 88`,
  `Added On 116`, `Completed On 116`, `Save Path 200`.
  - Default visible: `# name size status downloadSpeed uploadSpeed eta seeds peers ratio addedOn`.
  - Numeric columns are right aligned with tabular figures.
- Header: 32 px, 12 px weight 500 `ink_dim`, hover ink.
  - Sort: click sorts; clicking again reverses. The ▲/▼ arrow is 8 px accent, placed
    on the left for numeric columns. Text columns use natural ordering
    (`compare_text`: numeric-aware, case-insensitive, via `icu_collator` with numeric
    ordering).
  - Resize: a grip 9 px wide with a 1×16 divider that becomes a 2 px full-height
    accent on hover. Minimum width 28. Widths persist on mouse-up.
  - Right-click the header → a native checkbox menu of all columns (`Order (#)`;
    `#` and Name disabled). Toggling re-inserts a column in canonical order and
    persists.
- Rows are 26 px with 4 px vertical padding on the list. Hover uses `hover`.
  Selection uses `sel`, or `sel_blur` when the list is not focused.
- Cells:
  - `#`: row index + 1.
  - Name: state icon (`downloading`, `seeding`, `completed`, `error`, `inactive`,
    coloured `down/ok/fin/err/ink_faint`) + ellipsised name + tooltip.
  - Size: `wantedSize || size`.
  - Seeds and Peers: blank when stopped.
  - ETA: blank when done or not downloading.
  - Downloaded/Uploaded: blank zero.
  - Avail.: 3 decimal places.
  - Dates: `dd/mm/yyyy HH:MM`.
  - Save Path: with a tooltip.
- **Status progress bar:** a 22 px `bar_track` with a square fill coloured by
  `barClass` (`bar`, `seed`, `fin`, `idle`, `err`). The label is drawn **twice**:
  once in `bar_label` over the track, and once in `bar_label_on` inside a container
  clipped to the fill width, holding a full-width copy so the glyphs line up. The
  percentage is appended only while incomplete and not in error. The fill width
  animates over 250 ms.
- Empty state: 40 px logo at 40 % opacity, `No torrents yet.` / `No torrents match
  this view.`, `Drop a .torrent file here, or press ⌘O to add one.` with a `kbd` chip.
- The first time any rows arrive, select the first visible one.

### 10.6 Detail pane

The horizontal splitter sets the pane height: 260 by default, clamped to
90…(window height − 190). A tab strip 40 px tall holds 28 px tabs with 15 px icons:
General, Trackers, Peers, Pieces, Files, Speed, Logger. The active tab uses
`sel_quiet`.

- **General:** a 28 px bar (max width 520) + percentage to 2 decimal places, then two
  blocks.
  - **Transfer:** Time Elapsed, Remaining, Downloaded, Uploaded, Download Speed,
    Upload Speed, Share Ratio, Seeds `N connected`, Peers, Availability, Status.
  - **Torrent:** Name, Save As, Total Size `(X selected)`, Pieces `N × size (have H)`,
    Hash (selectable mono), Comment, Created By, Created On, Added On, Completed On,
    Private `Yes (DHT/PEX off)`, Order `Sequential / Rarest first`.
  - Grid: 128 px label column, 7/12 gaps, `—` for missing values.
- **Trackers:** Tracker 52 %, Status 18 % (`ok` / `err` / `ink_faint`), Seeds, Peers,
  Update In (`Ns` / `—`). `No trackers.` when empty.
  - Right-click → Add Tracker… (prompt pre-filled
    `udp://tracker.opentrackr.org:1337/announce`), Update Tracker, Copy Tracker URL.
- **Peers:** IP, Conn, Flags (mono 11 px), Client, %, Down Speed, Up Speed,
  Downloaded, Uploaded, sorted by down + up descending. `No peers connected.` when
  empty. Right-click → Add Peer… (`ip:port`), Copy Peer Address.
- **Pieces:** a legend (Have `bar_seed`, Downloading `piece_partial`, Missing
  `bar_track`) and `H of N pieces · size each`.
  - Drawn with a GPUI `canvas` using the same cell algorithm: cell = √(area/n) clamped
    to 2–14, shrunk until it fits, 1 px gap when cell > 4, 6 px padding.
  - Blanked when no map; `Waiting for metadata…` while the torrent has no metadata.
- **Files:** #, Name (path tooltip), Size, 12 px minibar, %, Priority (`prio-0` faint,
  `prio-2` ok 500).
  - `File list appears once the torrent metadata arrives.` until then.
  - Right-click → Open, Show in Finder / Explorer / file manager (quirk Q5), and the
    priority radios. Double-click → open.
- **Speed:** a legend and `Showing: name` / `All torrents`, plus `D x · U y`.
  - `canvas` with a 150-sample history per torrent plus a global one (purged when a
    torrent is removed).
  - Plot padding 58/8/8/16, four gridlines labelled with speed, peak =
    max(1024, samples) × 1.15.
  - Area fills with 1.4 px strokes, axis in `line_hard`, `150s ago` / `now`.
- **Logger:** mono 12/1.7, `[HH:MM:SS]`, `warn` / `err` colours. Auto-scrolls only if
  within 24 px of the bottom; keeps 800 lines.
- Every pane without a selection shows `Select a torrent to see its details.` A
  details payload for a different id is ignored.

### 10.7 Status bar

The bar is 30 px, 12 px `ink_dim`, tabular figures. Segments are 22 px pills with
radius 5, from left to right:

- `DHT: N nodes` (green dot) / `DHT: starting` (yellow) / `DHT: disabled` (red).
- `Torrents: N`.
- A spacer.
- The **update pill** (hidden unless there is something to say):
  - `Checking for updates…` (manual checks only, pulsing icon)
  - `Update available · X` (click downloads, or opens the release page when not installable)
  - `Downloading update · N%` (busy)
  - `Preparing update…` (busy)
  - `Restart to update · X` (accent + `accent_soft`, 500; click → confirm sheet)
  - `Update download failed` (click retries)
- The **alt pill:** `No limit` / `Limit: ↓100 kB/s ↑∞` / `Alt: no limit` /
  `Alt: ↓… ↑…`, amber when on. Clicking it toggles.
- Down: blue arrow, `D: speed`, `T: total`.
- Up: green arrow, `U: speed`, `T: total`.
- Brand: 14 px logo + version at 75 % opacity (100 % on hover), tooltip
  `ztorrent X -- About ztorrent`. Click opens About.
- Pulse and wash animations are disabled when the OS asks for reduced motion
  (`NSWorkspace.accessibilityDisplayShouldReduceMotion`,
  `SPI_GETCLIENTAREAANIMATION`, the GNOME `enable-animations` setting).

### 10.8 Dialogs

**Shared modal plumbing:**

- Overlay `rgba(0,0,0,.32)`, card radius 10, `shadow_modal`, max 760 px / 92 % wide
  and 86 % tall.
- Title 15/600. Body padding 16/20, scrolling. Footer with a top hairline:
  `[extra] … Cancel [Primary]`.
- **Enter** confirms, except in a textarea or with Shift. **Escape** and a click on
  the overlay cancel.
- The first text field is focused and selected.
- While an async OK runs, OK is disabled. A thrown error goes into the error slot
  (`err` colour). Returning `false` keeps the sheet open.
- One modal at a time. Global shortcuts are suppressed while it is open (a key
  context).

| Sheet | Width | Contents and behaviour |
|---|---|---|
| Prompt | 400 | label + text field; blank = stay open |
| Confirm | 420 | message (`ink_dim`, 1.55 line height); resolves true only via the primary button |
| **Add New Torrent** | 560 | *Save In*: path + Browse… (opens at the offered folder, walking up to an existing directory). *Label*: select of existing labels + `New label…` (last entry, matched by **position**) that reveals a text field. *Torrent Contents*: bold name + size summary (`X of Y` when partial); file table with check-all and per-file checkboxes (max height 240), or `This is a magnet link — the file list arrives once metadata is fetched from the swarm.` / `Single-file torrent.`; Info Hash (mono 10 px, selectable); Comment; `N announce URL(s)`. *Start torrent*, *Download sequentially*. Errors: `Choose a download folder.`, `Select at least one file.`, `This torrent is already in the list.`. Remembers `lastSavePath`. Unreadable source → `Could not read this torrent.` + error, Close only |
| **Add Torrent from URL** | 480 | text field (magnet / 40-hex infohash → `magnet:?xt=urn:btih:` lowercased / http(s)); pre-filled from the clipboard when it holds a magnet or a `.torrent` URL; Save In + Browse…; Continue → Add New Torrent with that folder. Error `Enter a link.` |
| **Customize Label** | 400 | live preview row; *Color*: 8 swatches, 26 px circles, 10 px gap, hover scale 1.08, selected ring 2 px `raised` + 4 px `accent`; *Symbol*: 8-column grid, 36 px tiles, `sunken`, selected `accent_soft` + accent border. Resolves `{symbol, color}` |
| **Create New Torrent** | 560 | *Select Source*: path + Add File… + Add Folder…; *Properties*: trackers textarea (three default UDP trackers), web seeds textarea, comment, piece size (Auto, 16 kB … 16 MB); *Private torrent (disable DHT and PEX)*, *Start seeding* (on). OK label `Create and Save As…` → save dialog `name.torrent` → status `Hashing… this can take a moment for large folders.` (plus real progress, [§5.5](#55-create-torrent)). Error `Choose a file or folder first.` |
| **Preferences** | 620 | [§6.3](#63-preferences-sheet-behaviour); nav 184 px; every field, hint text verbatim |
| **Properties — name** | 520 | read-only, selectable 130 px / 1fr grid: Name, Info Hash, Save Path, Total Size, Selected Size, Pieces, Private, Comment, Created By, Created On, Added On, Completed On, Downloaded, Uploaded, Ratio, Label, Magnet URI; Close only |
| Remove confirm | native | `Remove "X" from the list?` / `Remove N torrents from the list?`, detail `The downloaded files will be left on disk.`. With data: `…and delete the downloaded data?`, detail about deletion, buttons Cancel / Remove or Delete Data. Only when `confirmOnDelete` |
| Restart to update | Confirm | `Restart to update` — `ztorrent X is ready to install. ztorrent will close, update itself and open again. Downloads resume where they left off.` — Restart |
| About | native | `ztorrent X`; detail `A BitTorrent client for macOS, Windows and Linux.` + the engine line, updated to name libtorrent |
| Manual update check | native | `ztorrent X is up to date.` / `Could not check for updates.` + detail |

Form metrics: rows use a 168 px label column with 8/12 gaps and 10 px bottom margin.
Inputs: padding 6/9, radius 6, `line_hard` border, focus ring 3 px `accent_soft`.
Buttons: min 84, height 30, weight 500; primary is white on `accent`; small is 26
tall. Checkboxes are 15 px accent. Fieldset legends are uppercase 11/600. Hints are
12 px `ink_faint`.

### 10.9 Menus

GPUI `set_menus`. The menus are rebuilt from `Settings` whenever the theme or
alternate speed changes, so check marks are always right. Accelerators use GPUI's
`secondary-` modifier (⌘ on macOS, Ctrl elsewhere).

| Menu | Items |
|---|---|
| ztorrent (macOS) | About ztorrent · Check for Updates… · — · Preferences… `⌘,` · — · Services · — · Hide / Hide Others / Show All · — · Quit |
| File | Add Torrent… `O` · Add Torrent from URL… `U` · — · Create New Torrent… `N` · — · Close Window · (non-macOS: — · Preferences… `,` · — · Exit) |
| Edit | Undo · Redo · — · Cut · Copy · Paste · Select All (bound to gpui-component input actions) · — · Find `F` |
| Torrent | Start `R` · Pause `P` · Stop `.` · — · Force Re-Check `E` · Update Tracker `T` · — · Move Up Queue `↑` · Move Down Queue `↓` · — · Remove `⌫` · Remove And Delete Data… `⇧⌫` · — · Copy Magnet URI `⇧C` · Open Containing Folder `⇧O` |
| Options | ✓ Alternate Speed Limits `⇧L` · — · Preferences… `⌥,` |
| View | Appearance ▸ (● Light, ● Dark, —, Toggle Light/Dark `L`) · — · Enter Full Screen. *Reload* and *Toggle Developer Tools* have no meaning without a web view: dropped, with the GPUI inspector behind `--dev` in debug builds (deviation V1) |
| Window | macOS: Minimize · Zoom · — · Bring All to Front; others: Minimize · Close |
| Help | ztorrent Help (About) · Sample Torrents Folder · (non-macOS: — · Check for Updates…) |

**Context menus** (`NativeMenu`, same order and enablement):

- **torrent:**
  - Start · Pause · Stop · — · Force Re-Check · Update Tracker¹ · — · Remove ·
    Remove And Delete Data… · —
  - Bandwidth Allocation ▸ (Move Up Queue¹, Move Down Queue¹, —, ✓ Download Sequentially¹)
  - Labels ▸ (Remove Label, —, ● each label, —, New Label…)
  - — · Open Containing Folder¹ · Copy Magnet URI¹ · Save .torrent As…¹ · — · Properties…¹
- **file:** Open · Show in Finder · — · ● High Priority · ● Normal Priority · ● Don't Download
- **tracker**, **peer**, **label**, **columns**: as in [§10.4](#104-sidebar)–[§10.6](#106-detail-pane).

¹ enabled only for a single selection.

### 10.10 Keyboard

These bindings are active in the `TorrentList` key context: not in text fields, not
while a modal is open.

| Key | Action |
|---|---|
| ↑ / ↓ (⇧ extends the range) | move the selection; scroll into view |
| ⌘A / Ctrl+A | select every visible row |
| Esc | clear the selection |
| Delete / Backspace (⇧ = with data) | Remove |
| Space | pause if running, start if paused or stopped |
| Enter | Open Containing Folder |
| ⌘L / Ctrl+L | toggle light/dark (global) |

GPUI actions are named (`ztorrent::Start`, `ztorrent::SetTab { tab }`,
`ztorrent::Select { id }` and so on). These names *are* the scripting surface that
replaces `window.ztorrentUI`.

### 10.11 Mouse and drag

- **Selection** follows `app.js` exactly:
  - A plain mousedown on one row of a multi-selection defers the collapse to mouseup.
  - ⌘/Ctrl click toggles a row; Shift click selects a range from the anchor.
  - Right-click on a selected row keeps the selection.
  - A click on empty space clears the selection.
  - Double-click opens the containing folder.
  - The grid gets focus when clicked; `sel_blur` shows when focus leaves.
- **Rows onto labels:** a GPUI drag carrying the selected ids. A drag started outside
  the selection first selects that row. It cancels the pending collapse. The drag view
  is the accent chip `N torrents` (for a single row: the torrent's name, deviation V2).
  Label drop targets light up as in [§10.4](#104-sidebar).
- **External files:** `ExternalPaths` (**verified**, `FileDropEvent`) filtered to
  `.torrent`. One file → Add sheet; several → added straight away to `lastSavePath`.
  A 2 px dashed accent outline inset 8 px shows while hovering. Internal drags never
  show it.
- **Dropped text** (magnet or `.torrent` URL from a browser): GPUI exposes only
  dropped paths. **Spike S7** adds a macOS `NSPasteboard` string type and a Windows
  `CF_UNICODETEXT` drop target in `ztorrent-platform`; otherwise it is a documented
  gap. Pasting still works through Add from URL.
- **Splitters** show a resize cursor during the drag; the detail pane re-lays out live.

### 10.12 Icons

`xtask icons` emits `assets/icons/<name>.svg` from the single table that was
`icons.js`: 24-grid, `stroke="currentColor"`, 1.75 stroke, round caps. The logo keeps
its per-path stroke widths. The same run produces `build/icon.png`, `build/icon.icns`
and `build/logo.svg` from the same constants, so the dock, About, empty state and
status bar cannot drift. GPUI `svg().path(..).text_color(token)` tints them, which is
what `currentColor` did. `TAG_SYMBOLS` (16) and `TAG_COLORS` (8) keep their order;
`tag_style()` falls back to `label`/`slate`.

### 10.13 Formatting

`ztorrent-core/fmt.rs` ports `util.js` verbatim:

- `bytes` uses 1024 steps with units `B kB MB GB TB PB`: 0 dp for bytes, 1 dp at
  ≥100, else 2; `blank_zero`.
- `speed` is `bytes/s`, blank below 1 when `blank_zero`.
- `eta` takes **milliseconds**: `<1s`, `∞` beyond a year, `d h` / `h m` / `m s` / `s`.
- `pct` clamps; `ratio` uses 3 dp, `∞` when not finite.
- `datetime` is `dd/mm/yyyy HH:MM` local; `clock` is `HH:MM:SS`; `duration` is
  `d h` / `h m` / `m`.
- `compare_text` is natural and case-insensitive.

`xtask golden-fmt` runs the JS functions under Node over about 500 inputs once, and
commits the table as a Rust test fixture.

---

## 11. Platform integration

| Behaviour | Electron | Rust |
|---|---|---|
| Only one instance | `requestSingleInstanceLock`; `second-instance` restores + focuses and handles argv | Local socket lock. The second process sends argv and exits; the first restores/activates its window (`activate_window`) and handles it |
| macOS `.torrent` double-click / `magnet:` link | `open-file`, `open-url` | GPUI `on_open_urls` (**verified**; the `application:openURLs:` delegate receives `file://` and `magnet:`). Queued until the window exists, as `pendingOpen` is |
| argv on Windows/Linux | skip `-` flags; accept `magnet:` or `*.torrent` | Same |
| File associations | electron-builder `fileAssociations` + hand-written `CFBundleDocumentTypes` | macOS Info.plist with **one** document type entry (UTI `org.bittorrent.torrent`, `LSHandlerRank Owner`, **with** the icon; lesson 7) + `CFBundleURLTypes` for `magnet`; NSIS: ProgID + `magnet` URL protocol (per-user HKCU); Linux `.desktop` `MimeType=application/x-bittorrent;x-scheme-handler/magnet;` |
| Dock progress badge | badge `N%` of active downloads + progress; cleared otherwise (macOS, `showSpeedInDock`) | `NSDockTile.setBadgeLabel` every tick; the progress overlay via a dock tile content view (**spike**; badge alone is the fallback) |
| Completion notification | `Download complete` / name; click opens the save folder | Per-platform notifier; click → `open_with_system(save_path)` (**spike S4** on unsigned macOS) |
| Native theme | `nativeTheme.themeSource`, window background | `NSApp.appearance` (macOS), `DWMWA_USE_IMMERSIVE_DARK_MODE` (Windows title bar), window background |
| Closing the last window | macOS keeps running; others quit | Same; `on_reopen` makes a window |
| Quit | before-quit awaits engine destroy, then exits | `on_app_quit` future: resume data (≤5 s) + flush |
| Sample torrents | `resources/sample-torrents`; Help ▸ Sample Torrents Folder | Bundled in `Contents/Resources`, next to the exe on Windows, `$APPDIR/usr/share/ztorrent` in the AppImage |
| Development dock icon | set from `build/icon.png` when unpackaged | Same in debug builds |
| Clipboard | write magnet / tracker / peer; read for the URL sheet | GPUI clipboard |
| Reveal / open | `showItemInFolder` if it exists, else open the save path | `reveal_path` / `open_with_system`, same fallback |

---

## 12. Updater, packaging, installer, CI

### 12.1 Updater (one-to-one with `updater.js`)

- **States:** `idle checking available downloading staging ready error`, plus
  `version url size received file error errorFrom manual installable current`.
- **Timing:** first check 20 s after start, then every 6 h. Each check has a 15 s
  timeout (`GitHub did not answer`). Only packaged builds, only with `autoUpdate`.
- **`check({manual})` guards:**
  - Skipped while downloading or staging.
  - An automatic check is skipped if one is already running or a build is ready.
  - A staged build is remembered across the check, and kept on a network failure
    (with `error`, `errorFrom: "check"`).
  - The same version re-offers READY without a new download. A newer release discards
    the staged build.
- **Asset matching:** `endsWith("-{os}-{arch}.{ext}")` over every architecture
  spelling (`arm64/aarch64`, `x64/x86_64/amd64`). `mac`/`dmg`, `win`/`exe`,
  `linux`/`AppImage` only when `$APPIMAGE` is set. **Must stay in agreement with
  `scripts/install.sh`**, said in both files.
- **Download:** stream to `.part`, report progress every 256 KiB, verify the size,
  rename.
- **Stage:**
  - macOS: detach stale mounts of the image (`hdiutil info` parse), attach
    `-nobrowse -readonly -noautoopen` at a temp mount, `ditto` the `.app` out, detach
    in `finally`, delete the dmg.
  - Linux: copy + chmod 755 the AppImage.
  - Then write `staged.json`.
- **`resume()`** on start: offers READY if `staged.json` names a newer version whose
  payload exists, otherwise resets.
- **Apply:** refuses unpackaged runs. Windows: spawn the installer `/S --force-run`
  detached. macOS/Linux: write `apply.sh` (same script: pid wait ≤60 s, move-aside
  swap with rollback, `/usr/bin/xattr -dr com.apple.quarantine`, relaunch, clean up)
  and spawn it detached, then quit.
- **`openReleasePage()`** → `tag/vX` or `latest`.
- **Environment:** `ZTORRENT_UPDATE_FEED`, `ZTORRENT_UPDATE_PRETEND_VERSION`,
  `ZTORRENT_UPDATE_REPO`.
- `compare_versions` is ported with its prerelease rule and a table test.

### 12.2 Packaging

| Target | Artifact name (unchanged) | Tool |
|---|---|---|
| macOS arm64 / x64 | `ztorrent-X-mac-arm64.dmg`, `ztorrent-X-mac-x64.dmg` | cargo-packager dmg; the `.app` at the image root; bundle id `dev.zaf4.ztorrent`; category utilities; ad-hoc signed (arm64 requires a signature to run), otherwise unsigned as today |
| Windows x64 / arm64 | `ztorrent-X-win-x64.exe`, `ztorrent-X-win-arm64.exe` | cargo-packager NSIS: not one-click, per-user, changeable directory (as today). Honours `/S`; relaunches on `--force-run` |
| Linux x64 / arm64 | `ztorrent-X-linux-x86_64.AppImage`, `…-linux-arm64.AppImage`, `ztorrent-X-linux-amd64.deb`, `…-linux-arm64.deb` | cargo-packager; deb maintainer from `authors` (lesson: the email is mandatory); category Network; keywords |

### 12.3 The transition release

1. **Asset names and layout match**, so the 0.4.x updater and `install.sh` pick the
   Rust artifacts unchanged.
2. **Windows:** the Rust NSIS installer must, when run `/S` by 0.4.x:
   - install to the same per-user directory;
   - silently run the electron-builder uninstaller found under HKCU `Uninstall`, so
     no stale Electron files or Start-menu entries remain;
   - re-register file and protocol associations;
   - relaunch on `--force-run`.
3. **macOS:** the `.app` keeps the same name and bundle id, so the swap, Launch
   Services and Keychain entries continue.
4. **Gate:** with `ZTORRENT_UPDATE_FEED` pointing at a local release JSON, a
   **packaged 0.4.1** on each OS updates itself to the Rust build. The user's torrents,
   labels, columns, window, theme and proxy password must survive, and a second update
   (Rust → Rust) must work.
5. **Version number:** see open question O4.

### 12.4 CI

Same shape as `build.yml`, including every lesson from §3:

- A matrix with `fail-fast: false`: macOS (arm64 native + x86_64 cross), Windows
  (x64; arm64 via `aarch64-pc-windows-msvc`), Linux (ubuntu x64, ubuntu arm64 runner).
- Cache `~/.cargo` and the libtorrent build keyed by the libtorrent tag.
- **Jobs:**
  - `fmt`, `clippy -D warnings`
  - `test` (unit + egress + inbound + secrets + fmt golden + state-file golden)
  - `cargo deny`, `cargo audit`, the UI-crate dependency boundary check
  - `package`
  - `upload-artifact` with `if-no-files-found: error`
- **Release job:** unchanged logic (`gh` CLI, upload into an existing release, prepend
  the install header, `test -n "$(ls -A release)"`).
- Electron jobs keep running until cutover.

---

## 13. Phases and gates

Effort figures are rough, for one experienced Rust developer.

| Phase | Work | Exit criteria |
|---|---|---|
| **0. Spikes** (1–2 wk) | S1 libtorrent static build via cxx on all six targets in CI. S2 GPUI + gpui-component window: 10k-row `uniform_list` at 60 fps, native menus with check marks, `NativeMenu`, credentials round-trip, `on_open_urls`. S3 bind kill switch on macOS utun and Windows. S4 notification click on an unsigned macOS bundle. S5 WebTorrent/WebRTC decision. S6 `safeStorage` decryption on three OSes. S7 dropped text. S8 `--shot` capture of our own window | Written go/no-go per spike; D1 and D2 confirmed or revised |
| **1. Core** (1 wk) | `ztorrent-core`: Settings, Store (Murmur adoption, atomic writes, unknown keys), secrets codec, records, `fmt`, columns migration, `compare_versions` | Golden state-file round-trip; fmt golden table; secrets tests ported and passing |
| **2. Engine** (3–4 wk) | `lt-sys` bridge; add/inspect/create; lifecycle; queue; seed goals; part files; snapshot/details/globals; egress; log | Ported `test-egress`, `test-inbound` pass; headless integration test downloads the Sintel sample to 100 % with `.part` → final rename, stop/start without recheck, remove + delete data |
| **3. Shell UI** (2–3 wk) | theme, icons, window, toolbar, sidebar, grid (sort, resize, chooser, selection, keyboard), status bar, splitters, alt wash | Screens match reference shots in both themes (reviewed side by side); selection and keyboard tests in `TestAppContext` |
| **4. Detail tabs** (1–2 wk) | the seven panes incl. canvases | Stale-selection test (no previous torrent's pieces); tab shots match |
| **5. Dialogs** (2 wk) | modal plumbing + eight sheets + native prompts | Every error string reachable; Preferences grey-out matches the engine; theme preview revert |
| **6. Platform** (1–2 wk) | menus, context menus, single instance, open-urls, associations, dock, notifications, drop, clipboard, reveal | Double-click a `.torrent` and open a `magnet:` link with the app closed and open, on all OSes |
| **7. Updater + packaging** (1–2 wk) | updater, cargo-packager, NSIS script, CI, install.sh check | Transition gate ([§12.3](#123-the-transition-release)) passes on all OSes |
| **8. Parity beta** (2+ wk) | prereleases; checklist run; memory/CPU comparison | Checklist fully green or each red row signed off as a deviation |
| **9. Cutover** | stable release; one release later, delete `electron/`, `renderer/`, `package.json`, `node_modules`, Node scripts; rewrite README/development.md; add lessons | Net deletion commit; docs say how it was verified |

---

## 14. Testing and the parity checklist

**Ported suites:**

- `egress` (with an in-process SOCKS5 test server that counts CONNECTs, auths and
  source addresses) keeps every existing assertion, rewritten against libtorrent and
  reqwest:
  - peer dial via proxy round-trips data
  - fetch via proxy
  - hostname resolved at the proxy
  - `udp://` stripped, and untouched with no policy
  - turning the proxy off restores direct traffic
  - a dead proxy fails dials and fetches instead of going direct
  - auth on every connection
  - a bound dial and fetch leave from the bound address
  - proxy + bind binds the hop to the proxy
  - a vanished interface stops dials and fetches (`EEGRESSDOWN`)
  - DHT bound only under bind
  - inbound closed under a proxy while torrents still start
- `secrets`: every assertion in `test-secrets.mjs`, against a fake credential store
  (including denied and absent stores).

**New tests:**

- State-file goldens: 0.4.1, a pre-labels file, a Murmur file, a corrupt file.
- fmt goldens; `compare_versions` table; asset-matching table (including the
  `.dmg.blockmap` trap).
- The `apply.sh` three outcomes (success, failed copy → rollback, pid 0/garbage).
- UI logic in GPUI `TestAppContext`:
  - Finder selection semantics
  - Alt-speed shown identically in toolbar, pill, menu and prefs after each of the
    four ways to toggle it
  - Theme radios after ⌘L
  - Details dropped on a selection change
  - Label typed in the Add sheet appears in the sidebar and the next sheet
  - Column migration idempotence
- Bridge fuzzing: `cargo fuzz` over inspect and add.

**Scriptability:**

- `--add=path` (repeatable), `--shot=out.png`, `--shot-delay=N`, `--shot-tabs`,
  `--shot-quit`, `--dev`.
- `ZTORRENT_SHOT_EVAL` becomes `ZTORRENT_SHOT_ACTIONS='[{"action":"ztorrent::SetTab","args":{"tab":"peers"}}]'`,
  dispatched through GPUI's named actions.
- Capture renders our own window to PNG (**spike S8**: on macOS
  `CGWindowListCreateImage` for our own window id needs no Screen Recording
  permission; Windows `PrintWindow`; Linux via the renderer's readback).

**Parity checklist.** Every row in §5, §6.2, §8.2, §10 and §11 gets an ID (E-, O-,
P-, U-, X-) in a `docs/parity-checklist.md`. Each ID names its test or manual step
and records pass, or a deviation with a reason. Reference screenshots come from the
Electron app on a fixed fixture state (the four samples at mixed progress, two labels
with styles, one error) in both themes. Screenshots are a review aid, not a pixel
gate: font rasterisation differs.

**Performance acceptance:**

- Cold start to first paint ≤ Electron's.
- Idle RSS with four torrents at most half of Electron's (expected far less, without
  Chromium).
- 1 Hz tick costs < 2 % of one core with 200 torrents.
- Grid scrolls smoothly at 5,000 rows.

---

## 15. Quirks: keep or fix

Found while mapping. Each needs a yes/no before Phase 1; the recommendation is in the last column.

| ID | Current behaviour | Where | Recommendation |
|---|---|---|---|
| Q1 | `eta()` is documented as seconds but divides by 1000 (WebTorrent's `timeRemaining` is ms) | `util.js` | **Keep output**, name the parameter `ms` |
| Q2 | Created torrents say `createdBy: 'ztorrent 1.0.0'` whatever the version | `main.js` | **Fix**: `ztorrent X.Y.Z` (the About-box lesson) |
| Q3 | Create sheet says "blank line separates tiers" but every tracker becomes its own tier | `main.js` / `dialogs.js` | **Fix**: implement tiers as the label says |
| Q4 | Torrent name for a new torrent is split on `/`, wrong on Windows | `dialogs.js` | **Fix**: `Path::file_name` |
| Q5 | "Show in Finder" on every platform | `main.js` | **Fix**: "Show in Explorer" / "Show in File Manager" |
| Q6 | Peer flag `P` never set (`wire.type === 'utp'` never matches) | `engine.js` | **Fix**: set it for µTP connections |
| Q7 | `[Local Peer Discovery]` and `[Peer Exchange]` always say Working, even when disabled | `engine.js` | **Fix**: `Disabled` when off or suppressed |
| Q8 | Delete-data fallback `rm -rf join(savePath, name)` trusts `name` | `engine.js` | **Fix** (security): containment check |
| Q9 | `askWhereToSave` ("Show the Add Torrent dialog") is shown but read by nothing | `store.js` / `dialogs.js` | **Decide**: wire it (single add skips the sheet) or remove the checkbox |
| Q10 | `seedsTotal` reads `t._seedsTotal`, which is never set | `engine.js` | **Drop** the field (nothing displays it) |
| Q11 | Remove-with-data detail text talks about "the trash of no return" | `main.js` | **Keep** (it is honest: nothing goes to the trash) |
| Q12 | Taskbar/launcher progress only on macOS | `main.js` | **Keep** for parity; Windows `ITaskbarList3` is a cheap later addition |

Deviations forced by the platform:

- **V1:** no Reload / Toggle DevTools menu items.
- **V2:** the drag image for a single row is a chip, not a row snapshot.
- **V3:** a paused torrent disconnects its peers.
- **V4:** a hidden `.parts` file for unselected pieces.
- **V5:** WebRTC peers, unless S5 finds a supported route.
- **V6:** a one-time recheck on the first launch after migration.

---

## 16. Risks and open questions

### Risks

| Risk | Impact | Mitigation |
|---|---|---|
| **GPUI is pre-1.0**; `gpui-pre` snapshots break APIs | Upgrade churn | Exact pins, committed lockfile, upgrades in isolated commits, UI tests in `TestAppContext` |
| **Linux needs Vulkan** (GPUI's renderer); VMs and old GPUs without it cannot start the app | A new class of "won't start", like `libfuse2` | Detect at startup and print a clear message; README troubleshooting entry; spike on a VM without GPU |
| **Windows arm64 and x86_64-macOS support in GPUI/gpui-component** | A target might not build | S1/S2 build all six targets before Phase 1 |
| **Accessibility regresses**: Chromium gave screen readers the DOM; GPUI's accessibility is limited (gpui-base has only initial macOS accessibility work) | VoiceOver/Narrator users lose the app | Decide the bar (O3); at minimum label every control and keep all actions reachable from menus and keys |
| **libtorrent C++ build matrix** (Boost headers, OpenSSL for HTTPS trackers, cross-compiling) | CI time, binary size | Static build via cmake with a pinned tag; cache; vcpkg static triplets for OpenSSL; measure size |
| **Unsafe FFI and C++ parsing untrusted data** | Memory-safety exposure the JS engine did not have | [§9](#9-security-model): limits, catch-all bridge, fuzzing, prompt libtorrent updates via `cargo deny`-style advisory watch |
| **Kill switch semantics differ per OS** | Leak on VPN drop | S3 + the pause-all fallback in [§8.3](#83-the-kill-switch) |
| **safeStorage migration fails** somewhere | Users re-enter the proxy password once | Sealed value kept; a clear log line |
| **First-launch recheck** of large libraries | Slow first start | Recheck with queue limits applied; progress visible in Status |
| **Unsigned binaries change signature every update** | macOS may ask again for Keychain access after updates | Same situation as Electron today; document it; a Developer ID removes it |

### Open questions (maintainer decisions)

| ID | Question | Recommendation |
|---|---|---|
| O1 | Engine: libtorrent via cxx (full parity, C++ in the build) or librqbit (pure Rust, drops MSE, web seeds, priorities, rarest-first and `.part` until upstream catches up)? | libtorrent |
| O2 | Adopt the hardening items H1–H4, which go beyond parity? | H1 and H3 yes; H2 and H4 later |
| O3 | Accessibility bar for the first stable Rust release? | Keyboard and menu completeness + labelled controls; full screen-reader support as a tracked follow-up |
| O4 | Version for the first Rust release: `0.5.0` or `1.0.0`? | `0.5.0` betas → `0.5.0` stable; `1.0.0` once the parity checklist has no open deviations |
| O5 | Quirks Q1–Q12 | As recommended in [§15](#15-quirks-keep-or-fix) |
