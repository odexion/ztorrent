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
#   NO_COLOR          set to anything to turn off colour and animation

set -eu

REPO="${ZTORRENT_REPO:-odexion/ztorrent}"
PREFIX="${ZTORRENT_PREFIX:-$HOME/.local/bin}"

WORK=""
MNT=""
DL=""
TAG=""
RELEASE=""
SPIN_PID=""
DL_PID=""

say()  { printf '%s\n' "$*"; }
warn() { printf '%s\n' "$*" >&2; }

cleanup() {
  spin_kill
  [ -n "$DL_PID" ] && kill "$DL_PID" 2>/dev/null
  show_cursor
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
  NO_COLOR          set to anything to turn off colour and animation
USAGE
  exit 0
}

# ------------------------------------------------------------------ terminal
#
# Every non-ASCII glyph here is assembled with printf octal escapes instead of
# being typed literally. bash 3.2 scans identifiers by the locale's character
# rules, so a multi-byte character sitting next to $VAR in the source can be
# read as part of its name -- keeping this file pure ASCII sidesteps that in
# every locale while still putting UTF-8 on the wire.

ui_init() {
  TTY=0
  [ -t 1 ] && TTY=1

  ESC=$(printf '\033')
  COLOR=0
  if [ "$TTY" = 1 ] && [ -z "${NO_COLOR:-}" ] && [ "${TERM:-dumb}" != dumb ]; then
    COLOR=1
  fi

  if [ "$COLOR" = 1 ]; then
    C_RESET="${ESC}[0m"; C_DIM="${ESC}[2m";   C_BOLD="${ESC}[1m"
    C_CYAN="${ESC}[36m"; C_GREEN="${ESC}[32m"; C_RED="${ESC}[31m"
    CLR_EOL="${ESC}[K";  CUR_HIDE="${ESC}[?25l"; CUR_SHOW="${ESC}[?25h"
  else
    C_RESET=""; C_DIM=""; C_BOLD=""
    C_CYAN="";  C_GREEN=""; C_RED=""
    CLR_EOL=""; CUR_HIDE=""; CUR_SHOW=""
  fi

  # UTF-8 only when the locale says the terminal can render it.
  UNI=0
  if [ "$TTY" = 1 ]; then
    case "${LC_ALL:-${LC_CTYPE:-${LANG:-}}}" in
      *UTF-8*|*utf-8*|*UTF8*|*utf8*) UNI=1 ;;
    esac
  fi

  if [ "$UNI" = 1 ]; then
    G_FILL=$(printf '\342\224\201')     # U+2501 heavy horizontal
    G_HEAD=$(printf '\342\225\270')     # U+2578 heavy left half, the bar's edge
    G_EMPTY="$G_FILL"                   # same rule, dimmed, as the unfilled track
    G_OK=$(printf '\342\234\223')       # U+2713 check
    G_BAD=$(printf '\342\234\227')      # U+2717 ballot x
    G_DOT=$(printf '\302\267')          # U+00B7 middle dot
    G_SPIN=$(printf '\342\240\213 \342\240\231 \342\240\271 \342\240\270 \342\240\274 \342\240\264 \342\240\246 \342\240\247 \342\240\207 \342\240\217')
  else
    G_FILL="="; G_HEAD=">"; G_EMPTY="-"
    G_OK="*";   G_BAD="x"; G_DOT="-"
    G_SPIN="| / - \\"
  fi

  # Redrawing over a line only makes sense on a terminal; piped into a file or
  # a CI log, a bare CR is just a stray byte in the middle of the transcript.
  CR=""
  [ "$TTY" = 1 ] && CR=$(printf '\r')

  COLS=$(term_cols)
  # What the bar sits next to: spinner, percentage, transferred/total, rate and
  # ETA. Give the rest of the line to the bar, within reason.
  BAR_W=$((COLS - 54))
  [ "$BAR_W" -gt 40 ] && BAR_W=40
  [ "$BAR_W" -lt 12 ] && BAR_W=12

  # A frame every 80ms where sleep takes a fraction, otherwise no animation --
  # a once-a-second spinner is worse than none.
  NAP=0.08
  if ! sleep "$NAP" 2>/dev/null; then NAP=""; fi
}

term_cols() {
  c=$(tput cols 2>/dev/null) || c=""
  case "$c" in ''|*[!0-9]*) c="" ;; esac
  if [ -z "$c" ]; then
    c=$(stty size 2>/dev/null | cut -d' ' -f2) || c=""
    case "$c" in ''|*[!0-9]*) c="" ;; esac
  fi
  printf '%s' "${c:-80}"
}

hide_cursor() { [ "$COLOR" = 1 ] && printf '%s' "$CUR_HIDE"; return 0; }
show_cursor() { [ "${COLOR:-0}" = 1 ] && printf '%s' "$CUR_SHOW"; return 0; }

die() {
  spin_kill
  show_cursor
  printf '%s%s  %s%s%s %s\n' "${CR:-}" "$CLR_EOL" "$C_RED" "$G_BAD" "$C_RESET" "$*" >&2
  exit 1
}

# A finished step: the check mark replaces whatever was on the line.
ok() { printf '%s%s  %s%s%s %s\n' "${CR:-}" "$CLR_EOL" "$C_GREEN" "$G_OK" "$C_RESET" "$*"; }

# ------------------------------------------------------------------- spinner

spin_start() { # <message>
  SPIN_MSG="$1"
  if [ "$TTY" != 1 ] || [ -z "$NAP" ]; then say "  $1"; return 0; fi
  hide_cursor
  (
    set -- $G_SPIN
    i=0
    while :; do
      eval f=\${$(( i % $# + 1 ))}
      printf '\r  %s%s%s %s%s' "$C_CYAN" "$f" "$C_RESET" "$SPIN_MSG" "$CLR_EOL"
      i=$((i + 1))
      sleep "$NAP"
    done
  ) &
  SPIN_PID=$!
}

# Reaping a killed job yields 128+SIGTERM, and `set -e` would take the whole
# script down over it, so both halves swallow their status deliberately.
spin_kill() {
  [ -z "${SPIN_PID:-}" ] && return 0
  kill "$SPIN_PID" 2>/dev/null || true
  wait "$SPIN_PID" 2>/dev/null || true
  SPIN_PID=""
  return 0
}

# Stop the spinner and leave a finished line in its place.
spin_ok() { spin_kill; ok "$*"; }

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

# The whole progress line, drawn by one awk rather than a handful of pipelines:
# at a dozen frames a second the process count is the cost that shows. LC_ALL=C
# keeps "9.0 MB" from becoming "9,0 MB" under a comma-decimal locale, and puts
# awk in byte mode, which is what repeating a multi-byte bar glyph wants.
progress_line() { # <got> <total> <elapsed> <tick>
  LC_ALL=C awk -v got="$1" -v total="$2" -v el="$3" -v tick="$4" -v w="$BAR_W" \
      -v fill="$G_FILL" -v head="$G_HEAD" -v empty="$G_EMPTY" -v spin="$G_SPIN" \
      -v cb="$C_CYAN" -v cd="$C_DIM" -v cr="$C_RESET" '
    function human(b,   u, i) {
      split("B KB MB GB", u, " ")
      i = 1
      while (b >= 1024 && i < 4) { b /= 1024; i++ }
      return (i == 1) ? sprintf("%d %s", b, u[i]) : sprintf("%.1f %s", b, u[i])
    }
    function rep(s, n,   out) { out = ""; while (n-- > 0) out = out s; return out }
    function clock(t,   m) { m = int(t / 60); return sprintf("%d:%02d", m, int(t % 60)) }
    BEGIN {
      n = split(spin, sp, " ")
      s = (n > 0) ? sp[(tick % n) + 1] : ""
      rate = (el > 0) ? got / el : 0

      if (total <= 0) {                      # length unknown: no bar to draw
        printf "  %s%s%s %s%s  %s/s%s", cb, s, cr, cd, human(got), human(rate), cr
        exit
      }

      frac = got / total
      if (frac > 1) frac = 1
      done = int(frac * w)
      bar = cb rep(fill, done)
      if (done < w) bar = bar head cd rep(empty, w - done - 1)
      bar = bar cr
      eta = (rate > 0 && total > got) ? clock((total - got) / rate) : "0:00"

      printf "  %s%s%s %s %3d%%  %s%s / %s  %s/s  %s%s%s",
        cb, s, cr, bar, int(frac * 100 + 0.5),
        cd, human(got), human(total), human(rate), eta, cr, ""
    }'
}

# The size is already in the release JSON, so ask for it there before spending a
# HEAD request on it. Splitting at commas puts every field on its own line,
# which holds whether the API pretty-prints its response or not.
asset_size() { # <download url> -> bytes (0 when unknown)
  n=$(printf '%s' "$RELEASE" | tr ',' '\n' | awk -v want="$1" '
    /"size"[[:space:]]*:/ {
      s = $0
      sub(/.*"size"[[:space:]]*:[[:space:]]*/, "", s)
      sub(/[^0-9].*/, "", s)
      if (s != "") last = s
    }
    /browser_download_url/ && index($0, want) { print last + 0; found = 1; exit }
    END { if (!found) print 0 }')
  case "$n" in ''|*[!0-9]*) n=0 ;; esac

  if [ "$n" = 0 ] && [ "$DL" = curl ]; then
    n=$(curl -fsIL "$1" 2>/dev/null | tr -d '\r' |
      awk '/^[Cc]ontent-[Ll]ength:/ { v = $2 } END { print v + 0 }')
    case "$n" in ''|*[!0-9]*) n=0 ;; esac
  fi
  printf '%s' "$n"
}

# download <url> <dest> <label>
#
# curl's own --progress-bar cannot be styled, so the transfer runs quietly in
# the background and we draw the bar from the size of the file as it lands. The
# child reports through a status file rather than through `kill -0`, which keeps
# returning true for an exited-but-unreaped child and would spin forever.
download() {
  url="$1"; dest="$2"; label="$3"
  total=$(asset_size "$url")

  if [ "$TTY" != 1 ] || [ -z "$NAP" ]; then
    say "  downloading $label"
    if [ "$DL" = curl ]; then curl -fL -sS -o "$dest" "$url"
    else wget -q -O "$dest" "$url"; fi || die "download failed: $url"
    ok "downloaded $label"
    return 0
  fi

  status_file="$WORK/dl.status"
  err_file="$WORK/dl.err"
  rm -f "$status_file" "$err_file"
  : > "$dest"

  # `set +e` first: the subshell inherits errexit, so a curl that exits non-zero
  # would take the subshell down before it could report -- and the poll below,
  # waiting on a status file that never arrives, would spin until killed.
  if [ "$DL" = curl ]; then
    ( set +e; curl -fL -sS -o "$dest" "$url" 2>"$err_file"; code=$?
      printf '%s' "$code" > "$WORK/dl.tmp"; mv -f "$WORK/dl.tmp" "$status_file" ) &
  else
    ( set +e; wget -q -O "$dest" "$url" 2>"$err_file"; code=$?
      printf '%s' "$code" > "$WORK/dl.tmp"; mv -f "$WORK/dl.tmp" "$status_file" ) &
  fi
  DL_PID=$!

  start=$(date +%s)
  tick=0
  hide_cursor
  while [ ! -f "$status_file" ]; do
    got=$(wc -c < "$dest" 2>/dev/null | tr -d ' ') || got=0
    case "$got" in ''|*[!0-9]*) got=0 ;; esac
    printf '\r%s%s' "$(progress_line "$got" "$total" "$(( $(date +%s) - start ))" "$tick")" "$CLR_EOL"
    tick=$((tick + 1))
    sleep "$NAP"
  done

  wait "$DL_PID" 2>/dev/null || true
  DL_PID=""
  code=$(cat "$status_file" 2>/dev/null || echo 1)
  if [ "$code" != 0 ]; then
    show_cursor
    printf '\n' >&2
    [ -s "$err_file" ] && cat "$err_file" >&2
    die "download failed: $url"
  fi

  # One last frame so the bar ends full rather than wherever the last poll left
  # it, then the finished line replaces it.
  got=$(wc -c < "$dest" 2>/dev/null | tr -d ' ') || got=0
  elapsed=$(( $(date +%s) - start ))
  printf '\r%s%s' "$(progress_line "$got" "${total:-$got}" "$elapsed" 0)" "$CLR_EOL"
  show_cursor
  ok "downloaded $label $C_DIM$G_DOT $(size_of "$got") in ${elapsed}s$C_RESET"
}

size_of() { # <bytes> -> human
  LC_ALL=C awk -v b="${1:-0}" 'BEGIN{
    split("B KB MB GB", u, " "); i = 1
    while (b >= 1024 && i < 4) { b /= 1024; i++ }
    if (i == 1) printf "%d %s", b, u[i]; else printf "%.1f %s", b, u[i]
  }'
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

  case "$OS" in
    mac)   OS_LABEL="macOS" ;;
    linux) OS_LABEL="Linux" ;;
  esac
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

  spin_start "looking up the latest release"
  RELEASE=$(fetch "$api") ||
    die "could not reach the GitHub API for ${REPO}${ZTORRENT_VERSION:+ at $ZTORRENT_VERSION} - is the release published?"

  TAG=$(printf '%s\n' "$RELEASE" |
    grep -o '"tag_name"[[:space:]]*:[[:space:]]*"[^"]*"' |
    sed 's/.*"\([^"]*\)"$/\1/' | head -n 1)
  [ -n "$TAG" ] || die "no release found for $REPO"

  spin_ok "ztorrent $C_BOLD$TAG$C_RESET $C_DIM$G_DOT $OS_LABEL $ARCH$C_RESET"
}

# The artifact name carries the version (ztorrent-0.1.0-mac-arm64.dmg), so the
# download URL cannot be guessed from the tag alone - read it off the release.
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

  download "$url" "$WORK/ztorrent.dmg" "ztorrent $TAG"

  spin_start "installing to $app"

  MNT=$(mktemp -d "${TMPDIR:-/tmp}/ztorrent-mnt.XXXXXX")
  hdiutil attach -nobrowse -readonly -noautoopen -mountpoint "$MNT" \
    "$WORK/ztorrent.dmg" >/dev/null || die "could not mount the disk image"

  src=$(find "$MNT" -maxdepth 1 -name '*.app' -print 2>/dev/null | head -n 1)
  [ -n "$src" ] || die "no .app inside the disk image"

  [ -d "$app" ] && rm -rf "$app"
  cp -R "$src" "$app"

  # curl does not set the quarantine flag, but a re-run over a previously
  # quarantined copy might inherit one, and these builds are unsigned.
  # By full path: an xattr earlier on PATH need not be Apple's -- the PyPI
  # package of that name installs one that does not take these arguments.
  /usr/bin/xattr -dr com.apple.quarantine "$app" 2>/dev/null || true

  spin_ok "installed to $C_BOLD$app$C_RESET"
  say ""
  say "  ${C_DIM}Open it from Launchpad, or:${C_RESET} open -a ztorrent"
  say ""
}

# ---------------------------------------------------------------- Linux

install_linux() {
  url=$(find_asset linux AppImage)
  [ -n "$url" ] || die "the $TAG release has no Linux $ARCH AppImage attached"

  mkdir -p "$PREFIX"
  bin="$PREFIX/ztorrent"

  download "$url" "$WORK/ztorrent.AppImage" "ztorrent $TAG"

  spin_start "installing to $bin"
  chmod +x "$WORK/ztorrent.AppImage"
  mv -f "$WORK/ztorrent.AppImage" "$bin"
  install_desktop_entry "$bin"
  spin_ok "installed to $C_BOLD$bin$C_RESET"

  say ""
  case ":$PATH:" in
    *":$PREFIX:"*) say "  ${C_DIM}Run it with:${C_RESET} ztorrent" ;;
    *) warn "  $PREFIX is not on your PATH. Add it to your shell profile:"
       warn "      export PATH=\"\$PATH:$PREFIX\"" ;;
  esac

  # electron-builder's AppImages are type 2; older runtimes on newer distros
  # need libfuse2. Say so here rather than letting the first launch fail.
  if ! ldconfig -p 2>/dev/null | grep -q 'libfuse\.so\.2'; then
    warn ""
    warn "  libfuse2 was not found - AppImages need it. On Debian/Ubuntu:"
    warn "      sudo apt install libfuse2"
    warn "  Or run it extracted: ztorrent --appimage-extract-and-run"
  fi
  say ""
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

  ui_init
  pick_downloader
  detect_platform
  WORK=$(mktemp -d "${TMPDIR:-/tmp}/ztorrent-install.XXXXXX")

  say ""
  load_release

  case "$OS" in
    mac)   install_mac ;;
    linux) install_linux ;;
  esac
}

main "$@"
