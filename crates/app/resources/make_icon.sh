#!/usr/bin/env bash
# Regenerates the app icon for every color theme: icons/<theme>.ico (exe, shortcuts, installer)
# and icons/<theme>-64.rgba (window icon and welcome screen). Needs ImageMagick.
set -euo pipefail
cd "$(dirname "$0")"
mkdir -p icons
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

# Theme name, then the gradient's top and bottom colors.
themes=(
  "teal #2DD4BF #0E7490"
  "ocean #4F9DFF #1D4ED8"
  "mint #5EEAD4 #059669"
  "amber #FFB547 #E0671F"
  "coral #FF8A7A #E0405A"
  "purple #8F6BFF #4A2BD1"
  "graphite #5A5A66 #1E1E24"
)

convert -size 512x512 xc:black -fill white -draw "roundrectangle 16,16 495,495 112,112" "$work/mask.png"
for theme in "${themes[@]}"; do
  read -r name top bottom <<<"$theme"
  convert -size 512x512 gradient:"$top"-"$bottom" "$work/gradient.png"
  convert "$work/gradient.png" "$work/mask.png" -alpha off -compose CopyOpacity -composite "$work/tile.png"
  # A rounded play triangle over a seek bar.
  convert "$work/tile.png" \
    -fill white -stroke white -strokewidth 36 -draw "stroke-linejoin round polygon 196,140 196,308 330,224" \
    -stroke none -fill 'rgba(255,255,255,0.45)' -draw "roundrectangle 128,372 384,396 12,12" \
    -fill white -draw "roundrectangle 128,372 256,396 12,12" \
    "$work/master.png"
  convert "$work/master.png" -define icon:auto-resize=256,64,48,32,24,16 "icons/$name.ico"
  convert "$work/master.png" -resize 64x64 -depth 8 "rgba:icons/$name-64.rgba"
  convert "$work/master.png" -resize 96x96 "$work/$name-preview.png"
done
if [ -n "${ICON_PREVIEW:-}" ]; then
  convert "$work"/*-preview.png +append "$ICON_PREVIEW"
fi
