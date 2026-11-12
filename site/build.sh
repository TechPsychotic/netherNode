#!/bin/sh
# Assembles the GitHub Pages site into a folder (default: _site) from site/ plus the
# fonts, icons and screenshots already in the repository, so nothing is duplicated.
#   site/build.sh [out-dir]
set -eu
cd "$(dirname "$0")/.."
out=${1:-_site}

rm -rf "$out"
mkdir -p "$out/fonts" "$out/icons" "$out/screenshots"
cp site/index.html site/style.css site/main.js site/textures.js site/voxel.js "$out/"
cp assets/fonts/Minecraft-Regular.otf assets/fonts/Minecraft-Bold.otf assets/fonts/Minecraft-LICENSE.txt \
   assets/fonts/Inter-Regular.ttf assets/fonts/Inter-Bold.ttf assets/fonts/Inter-LICENSE.txt \
   assets/fonts/JetBrainsMono-Regular.ttf assets/fonts/JetBrainsMono-LICENSE.txt "$out/fonts/"
for size in 32 64 128 256; do cp "assets/icons/png/$size.png" "$out/icons/"; done
cp docs/screenshots/*.png "$out/screenshots/"
touch "$out/.nojekyll"
echo "Site built in $out/"
