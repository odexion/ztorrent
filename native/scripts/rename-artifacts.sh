#!/bin/sh
# Renames cargo-packager's outputs to the names electron-builder used, which
# scripts/install.sh and the updater both match on. Change one, change all three.
#
#   ztorrent-<version>-mac-<x64|arm64>.dmg
#   ztorrent-<version>-win-<x64|arm64>.exe
#   ztorrent-<version>-linux-<x86_64|arm64>.AppImage
#   ztorrent-<version>-linux-<amd64|arm64>.deb
set -eu
dir="${1:?output directory}"
version="${2:?version}"
for f in "$dir"/*; do
  case "$f" in
    *aarch64*.dmg|*arm64*.dmg)   mv "$f" "$dir/ztorrent-$version-mac-arm64.dmg" ;;
    *.dmg)                       mv "$f" "$dir/ztorrent-$version-mac-x64.dmg" ;;
    *aarch64*setup.exe|*arm64*setup.exe) mv "$f" "$dir/ztorrent-$version-win-arm64.exe" ;;
    *setup.exe)                  mv "$f" "$dir/ztorrent-$version-win-x64.exe" ;;
    *aarch64*.AppImage)          mv "$f" "$dir/ztorrent-$version-linux-arm64.AppImage" ;;
    *.AppImage)                  mv "$f" "$dir/ztorrent-$version-linux-x86_64.AppImage" ;;
    *arm64*.deb|*aarch64*.deb)   mv "$f" "$dir/ztorrent-$version-linux-arm64.deb" ;;
    *.deb)                       mv "$f" "$dir/ztorrent-$version-linux-amd64.deb" ;;
  esac
done
ls -l "$dir"
