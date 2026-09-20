#!/bin/sh
# Fails when a packaged build still loads a library from outside the system --
# Homebrew's libtorrent or OpenSSL, say -- that the machines it is installed on
# will not have. Run from the repository root, after cargo packager.
#
#   sh scripts/check-standalone.sh <target triple> [package folder]
set -eu
target="${1:?target triple}"
packages="${2:-target/packages}"

case "$target" in
  *apple-darwin)
    # The app as people get it: the one inside the disk image.
    dmg=$(ls "$packages"/*.dmg | head -1)
    mnt=$(mktemp -d)
    hdiutil attach -readonly -nobrowse -mountpoint "$mnt" "$dmg" >/dev/null
    trap 'hdiutil detach "$mnt" -force >/dev/null 2>&1; rmdir "$mnt"' EXIT
    app="$mnt/ztorrent.app"
    exe="$app/Contents/MacOS/ztorrent"
    du -sh "$dmg" "$app"
    otool -L "$exe"
    if otool -L "$exe" | tail -n +2 | grep -vE '^[[:space:]]*(/System/Library/|/usr/lib/)'; then
      echo "error: links a library macOS does not provide"; exit 1
    fi
    case "$target" in aarch64*) want=arm64 ;; *) want=x86_64 ;; esac
    got=$(lipo -archs "$exe")
    [ "$got" = "$want" ] || { echo "error: built for $got, not $want"; exit 1; }
    min=$(otool -l "$exe" | awk '/LC_BUILD_VERSION/{f=1} f&&/minos/{print $2; exit}')
    echo "minimum macOS: $min"
    [ "$min" = "11.0" ] || { echo "error: needs macOS $min, not 11.0"; exit 1; }
    codesign --verify --deep --strict --verbose=2 "$app"
    ;;
  *linux*)
    exe="target/$target/release/ztorrent"
    ldd "$exe"
    if ldd "$exe" | grep -E 'not found|libtorrent|libssl|libcrypto|libboost'; then
      echo "error: links a library the package does not carry"; exit 1
    fi
    echo "newest glibc symbol: $(objdump -T "$exe" | grep -o 'GLIBC_[0-9.]*' | sort -uV | tail -1)"
    ;;
  *windows*)
    exe="target/$target/release/ztorrent.exe"
    # Imports are read from the PE header; every DLL must be one Windows ships.
    powershell -NoProfile -Command '
      $bytes = [IO.File]::ReadAllBytes($args[0])
      $text = [Text.Encoding]::ASCII.GetString($bytes)
      $dlls = [regex]::Matches($text, "[A-Za-z0-9_.-]+\.dll") | ForEach-Object { $_.Value.ToLower() } | Sort-Object -Unique
      $dlls
      $foreign = $dlls | Where-Object { $_ -match "torrent|ssl|crypto|boost" }
      if ($foreign) { Write-Output "error: links $foreign"; exit 1 }
    ' "$exe"
    ;;
  *)
    echo "error: no check for $target"; exit 1 ;;
esac
echo "standalone: $target"
