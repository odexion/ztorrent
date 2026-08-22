#!/bin/bash
# Builds build/icon.icns from build/icon.png using only macOS built-ins.
set -euo pipefail
cd "$(dirname "$0")/.."
[ -f build/icon.png ] || node scripts/make-icon.mjs

SET=build/ztorrent.iconset
rm -rf "$SET"; mkdir -p "$SET"
for spec in "16 16x16" "32 16x16@2x" "32 32x32" "64 32x32@2x" \
            "128 128x128" "256 128x128@2x" "256 256x256" "512 256x256@2x" \
            "512 512x512" "1024 512x512@2x"; do
  px=${spec% *}; name=${spec#* }
  sips -z "$px" "$px" build/icon.png --out "$SET/icon_$name.png" >/dev/null
done
iconutil -c icns "$SET" -o build/icon.icns
rm -rf "$SET"
echo "wrote build/icon.icns ($(du -h build/icon.icns | cut -f1))"
