#!/bin/sh
# Install ztorrent on macOS or Linux from the latest GitHub release.
#
#   curl -fsSL https://raw.githubusercontent.com/odexion/ztorrent/main/scripts/install.sh | sh
#
# macOS gets the .dmg copied into /Applications; Linux gets the AppImage dropped
# into ~/.local/bin with a desktop entry. Nothing here needs sudo, and nothing is
# installed outside your home directory unless /Applications is already writable.
#
# Environment:
#   ZTORRENT_VERSION  tag to install, e.g. v0.1.0   (default: the latest release)
#   ZTORRENT_REPO     owner/name to install from    (default: odexion/ztorrent)
#   ZTORRENT_PREFIX   Linux: directory for the executable (default: ~/.local/bin)
#   GITHUB_TOKEN      optional, only to lift the anonymous API rate limit

set -eu

REPO="${ZTORRENT_REPO:-odexion/ztorrent}"
PREFIX="${ZTORRENT_PREFIX:-$HOME/.local/bin}"

WORK=""
MNT=""
DL=""
TAG=""
RELEASE=""

say()  { printf '%s\n' "$*"; }
warn() { printf '%s\n' "$*" >&2; }
die()  { printf 'install.sh: %s\n' "$*" >&2; exit 1; }

cleanup() {
  if [ -n "$MNT" ] && [ -d "$MNT" ]; then
    hdiutil detach "$MNT" >/dev/null 2>&1 ||
      hdiutil detach "$MNT" -force >/dev/null 2>&1 || true
    rmdir "$MNT" 2>/dev/null || true
  fi
  [ -n "$WORK" ] && rm -rf "$WORK"
  return 0
}
trap cleanup EXIT INT TERM

usage() {
  cat <<'USAGE'
Install ztorrent on macOS or Linux from the latest GitHub release.

  curl -fsSL https://raw.githubusercontent.com/odexion/ztorrent/main/scripts/install.sh | sh

macOS installs ztorrent.app into /Applications (or ~/Applications when that is
not writable); Linux installs the AppImage into ~/.local/bin with a desktop
entry. No sudo, nothing written outside your home directory.

Environment:
  ZTORRENT_VERSION  tag to install, e.g. v0.1.0   (default: the latest release)
  ZTORRENT_REPO     owner/name to install from    (default: odexion/ztorrent)
  ZTORRENT_PREFIX   Linux: directory for the executable (default: ~/.local/bin)
  GITHUB_TOKEN      optional, only to lift the anonymous API rate limit
USAGE
  exit 0
}

# ---------------------------------------------------------------- fetching

pick_downloader() {
  if command -v curl >/dev/null 2>&1; then
    DL=curl
  elif command -v wget >/dev/null 2>&1; then
    DL=wget
  else
    die "need curl or wget on PATH"
  fi
}

# fetch <url> -> stdout
fetch() {
  if [ "$DL" = curl ]; then
    if [ -n "${GITHUB_TOKEN:-}" ]; then
      curl -fsSL -H "Authorization: Bearer $GITHUB_TOKEN" "$1"
    else
      curl -fsSL "$1"
    fi
  elif [ -n "${GITHUB_TOKEN:-}" ]; then
    wget -qO- --header="Authorization: Bearer $GITHUB_TOKEN" "$1"
  else
    wget -qO- "$1"
  fi
}

# download <url> <dest>
download() {
  if [ "$DL" = curl ]; then
    curl -fL --progress-bar -o "$2" "$1"
  else
    wget -q --show-progress -O "$2" "$1" 2>/dev/null || wget -q -O "$2" "$1"
  fi
}

# ---------------------------------------------------------------- platform

detect_platform() {
  OS=$(uname -s)
  ARCH=$(uname -m)

  case "$OS" in
    Darwin) OS=mac ;;
    Linux)  OS=linux ;;
    *)      die "unsupported operating system: $OS (macOS and Linux only)" ;;
  esac

  case "$ARCH" in
    x86_64|amd64)  ARCH=x64 ;;
    arm64|aarch64) ARCH=arm64 ;;
    *)             die "unsupported architecture: $ARCH (x64 and arm64 only)" ;;
  esac

  # A shell running under Rosetta reports x86_64 on Apple silicon; install the
  # native build rather than the translated one.
  if [ "$OS" = mac ] && [ "$ARCH" = x64 ] &&
     [ "$(sysctl -n sysctl.proc_translated 2>/dev/null || echo 0)" = 1 ]; then
    ARCH=arm64
  fi
}

# ---------------------------------------------------------------- release

# Fetch the release once, into globals. Doing this in a command substitution
# would put TAG in a subshell, where every later reference loses it.
load_release() {
  if [ -n "${ZTORRENT_VERSION:-}" ]; then
    api="https://api.github.com/repos/$REPO/releases/tags/$ZTORRENT_VERSION"
  else
    api="https://api.github.com/repos/$REPO/releases/latest"
  fi

  RELEASE=$(fetch "$api") ||
    die "could not reach the GitHub API for ${REPO}${ZTORRENT_VERSION:+ at $ZTORRENT_VERSION} — is the release published?"

  TAG=$(printf '%s\n' "$RELEASE" |
    grep -o '"tag_name"[[:space:]]*:[[:space:]]*"[^"]*"' |
    sed 's/.*"\([^"]*\)"$/\1/' | head -n 1)
  [ -n "$TAG" ] || die "no release found for $REPO"
}

# The artifact name carries the version (ztorrent-0.1.0-mac-arm64.dmg), so the
# download URL cannot be guessed from the tag alone — read it off the release.
# Match on the end of the name, or a .dmg suffix also matches its .dmg.blockmap.
asset_url() { # <suffix> -> url on stdout
  printf '%s\n' "$RELEASE" |
    grep -o '"browser_download_url"[[:space:]]*:[[:space:]]*"[^"]*"' |
    sed 's/.*"\(https[^"]*\)"$/\1/' |
    while IFS= read -r u; do
      case "$u" in *"$1") printf '%s\n' "$u"; break ;; esac
    done
}

# electron-builder names each artifact with its target's own arch convention,
# not one canonical spelling: the dmg is x64, the AppImage x86_64, the deb
# amd64. Try every spelling rather than pinning the one that happens to be
# right today.
arch_aliases() {
  case "$ARCH" in
    x64)   printf '%s\n' x64 x86_64 amd64 ;;
    arm64) printf '%s\n' arm64 aarch64 ;;
  esac
}

find_asset() { # <os> <ext> -> url on stdout
  arch_aliases | while IFS= read -r a; do
    u=$(asset_url "-$1-$a.$2")
    if [ -n "$u" ]; then printf '%s\n' "$u"; break; fi
  done
}

# ---------------------------------------------------------------- macOS

install_mac() {
  url=$(find_asset mac dmg)
  [ -n "$url" ] || die "the $TAG release has no macOS $ARCH build (.dmg) attached"

  dest=/Applications
  [ -w "$dest" ] || dest="$HOME/Applications"
  mkdir -p "$dest"
  app="$dest/ztorrent.app"

  say "Downloading ztorrent $TAG for macOS ${ARCH}…"
  download "$url" "$WORK/ztorrent.dmg"

  MNT=$(mktemp -d "${TMPDIR:-/tmp}/ztorrent-mnt.XXXXXX")
  hdiutil attach -nobrowse -readonly -noautoopen -mountpoint "$MNT" \
    "$WORK/ztorrent.dmg" >/dev/null || die "could not mount the disk image"

  src=$(find "$MNT" -maxdepth 1 -name '*.app' -print 2>/dev/null | head -n 1)
  [ -n "$src" ] || die "no .app inside the disk image"

  if [ -d "$app" ]; then
    say "Replacing the existing $app"
    rm -rf "$app"
  fi

  cp -R "$src" "$app"

  # curl does not set the quarantine flag, but a re-run over a previously
  # quarantined copy might inherit one, and these builds are unsigned.
  xattr -dr com.apple.quarantine "$app" 2>/dev/null || true

  say ""
  say "ztorrent $TAG installed to $app"
  say "Open it from Launchpad, or: open -a ztorrent"
}

# ---------------------------------------------------------------- Linux

install_linux() {
  url=$(find_asset linux AppImage)
  [ -n "$url" ] || die "the $TAG release has no Linux $ARCH AppImage attached"

  mkdir -p "$PREFIX"
  bin="$PREFIX/ztorrent"

  say "Downloading ztorrent $TAG for Linux ${ARCH}…"
  download "$url" "$WORK/ztorrent.AppImage"
  chmod +x "$WORK/ztorrent.AppImage"
  mv -f "$WORK/ztorrent.AppImage" "$bin"

  install_desktop_entry "$bin"

  say ""
  say "ztorrent $TAG installed to $bin"

  case ":$PATH:" in
    *":$PREFIX:"*) say "Run it with: ztorrent" ;;
    *) warn ""
       warn "$PREFIX is not on your PATH. Add it to your shell profile:"
       warn "    export PATH=\"\$PATH:$PREFIX\"" ;;
  esac

  # electron-builder's AppImages are type 2; older runtimes on newer distros
  # need libfuse2. Say so here rather than letting the first launch fail.
  if ! ldconfig -p 2>/dev/null | grep -q 'libfuse\.so\.2'; then
    warn ""
    warn "libfuse2 was not found — AppImages need it. On Debian/Ubuntu:"
    warn "    sudo apt install libfuse2"
    warn "Or run it extracted: ztorrent --appimage-extract-and-run"
  fi
}

install_desktop_entry() { # <bin>
  exe="$1"
  apps="$HOME/.local/share/applications"
  share="$HOME/.local/share/ztorrent"
  mkdir -p "$apps" "$share"

  # Best effort: pull the icon out of the AppImage so the launcher entry is not
  # blank. Failure here is not worth aborting an otherwise good install over.
  icon=ztorrent
  if (cd "$WORK" && "$exe" --appimage-extract 'ztorrent.png' >/dev/null 2>&1) &&
     [ -f "$WORK/squashfs-root/ztorrent.png" ]; then
    cp -f "$WORK/squashfs-root/ztorrent.png" "$share/ztorrent.png"
    icon="$share/ztorrent.png"
  fi

  cat > "$apps/ztorrent.desktop" <<DESKTOP
[Desktop Entry]
Type=Application
Name=ztorrent
Comment=A BitTorrent client
Exec=$exe %U
Icon=$icon
Terminal=false
Categories=Network;FileTransfer;P2P;
Keywords=torrent;bittorrent;p2p;download;
MimeType=application/x-bittorrent;x-scheme-handler/magnet;
StartupWMClass=ztorrent
DESKTOP

  update-desktop-database "$apps" >/dev/null 2>&1 || true
}

# ---------------------------------------------------------------- main

main() {
  case "${1:-}" in -h|--help) usage ;; esac

  pick_downloader
  detect_platform
  WORK=$(mktemp -d "${TMPDIR:-/tmp}/ztorrent-install.XXXXXX")

  load_release

  case "$OS" in
    mac)   install_mac ;;
    linux) install_linux ;;
  esac
}

main "$@"
