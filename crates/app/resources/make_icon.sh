#!/usr/bin/env bash
# Regenerates icon.ico (exe and installer) and icon-64.rgba (window icon). Needs ImageMagick.
set -euo pipefail
cd "$(dirname "$0")"
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
convert -size 512x512 gradient:'#8F6BFF'-'#4A2BD1' "$work/gradient.png"
convert -size 512x512 xc:black -fill white -draw "roundrectangle 16,16 495,495 112,112" "$work/mask.png"
convert "$work/gradient.png" "$work/mask.png" -alpha off -compose CopyOpacity -composite "$work/tile.png"
convert "$work/tile.png" -fill white -draw "polygon 204,150 204,362 376,256" "$work/master.png"
convert "$work/master.png" -define icon:auto-resize=256,128,64,48,32,24,16 icon.ico
convert "$work/master.png" -resize 64x64 -depth 8 rgba:icon-64.rgba
convert "$work/master.png" -resize 128x128 "$work/preview.png"
cp "$work/preview.png" "${ICON_PREVIEW:-/dev/null}" 2>/dev/null || true
