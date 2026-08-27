# ztorrent

A free BitTorrent client for macOS, Windows and Linux. Open a torrent, watch it
download, find your files in your Downloads folder.

## Install

**macOS** — download for [Apple silicon][mac-arm] or [Intel][mac-x64].

Open the downloaded file and drag ztorrent into Applications. (Not sure which one?
Apple menu ▸ About This Mac. "Apple M1/M2/M3" means Apple silicon.)

**Windows** — [download the installer][win].

Run it and follow the prompts.

**Linux** — download the [AppImage][linux-appimage], or a [.deb][linux-deb] for Debian
and Ubuntu.

Make the AppImage executable (right-click ▸ Properties ▸ Permissions) and double-click it.

Prefer the terminal? On macOS and Linux this does the whole thing for you:

```bash
curl -fsSL https://raw.githubusercontent.com/odexion/ztorrent/main/scripts/install.sh | sh
```

All downloads are on the [releases page](https://github.com/odexion/ztorrent/releases).

### First time you open it

ztorrent isn't signed with a paid developer certificate, so your computer will warn
you that it's from an unidentified developer. This is expected.

- **macOS** — right-click the app and choose **Open**, then **Open** again. Only needed once.
- **Windows** — click **More info**, then **Run anyway**.

## Using it

**Add a torrent** any of these ways:

- Drag a `.torrent` file onto the window
- **File ▸ Add Torrent…** and pick a file
- Copy a magnet link, then **File ▸ Add from URL…**
- Double-click a `.torrent` file anywhere on your computer

Then pick where to save it and click OK. That's it — the download starts, and the
bar in the Status column fills up as it goes.

Files land in your Downloads folder unless you choose somewhere else. Downloads
resume by themselves when you reopen the app, so it's safe to quit partway through.

**Handy to know**

| | |
|---|---|
| Space | Pause or resume whatever is selected |
| ⌘F / Ctrl+F | Search your list |
| ⌘L / Ctrl+L | Switch between light and dark |
| Right-click a torrent | Open the folder, copy the magnet link, remove it |
| Bottom of the window | Your current download and upload speeds |

There are more shortcuts under each menu, and everything has a right-click menu.

## What it can do

- Downloads from `.torrent` files and magnet links
- Pause, resume, and queue up as many as you like
- Pick which files inside a torrent you actually want
- Speed limits, so it doesn't hog your connection
- Labels and filters to keep a long list tidy
- Light and dark themes
- Optional privacy routing through a proxy or VPN connection
- Make your own torrents to share

## Something not working?

**The app won't open on macOS** — right-click it and choose Open (see above).

**The AppImage won't start on Linux** — it needs a system library called libfuse2.
On Debian or Ubuntu: `sudo apt install libfuse2`.

**Nothing is downloading** — a torrent needs other people online sharing it. Try one
of the well-shared samples in `sample-torrents/` to check the app itself is fine.

## Anything else

Built with [WebTorrent](https://webtorrent.io) and [Electron](https://electronjs.org).
Developer notes — building, architecture, privacy internals — are in
[docs/development.md](docs/development.md).

MIT licensed. Please only share files you have the right to share.

[mac-arm]: https://github.com/odexion/ztorrent/releases/download/v0.1.0/ztorrent-0.1.0-mac-arm64.dmg
[mac-x64]: https://github.com/odexion/ztorrent/releases/download/v0.1.0/ztorrent-0.1.0-mac-x64.dmg
[win]: https://github.com/odexion/ztorrent/releases/download/v0.1.0/ztorrent-0.1.0-win-x64.exe
[linux-appimage]: https://github.com/odexion/ztorrent/releases/download/v0.1.0/ztorrent-0.1.0-linux-x86_64.AppImage
[linux-deb]: https://github.com/odexion/ztorrent/releases/download/v0.1.0/ztorrent-0.1.0-linux-amd64.deb
