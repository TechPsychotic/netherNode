#!/usr/bin/env bash
# Builds target/release/netherNode.app from a release build (run on macOS).
# Usage: cargo build --release && packaging/macos/make-app.sh
set -euo pipefail
root="$(cd "$(dirname "$0")/../.." && pwd)"
version="$(sed -n 's/^version = "\(.*\)"/\1/p' "$root/crates/gui/Cargo.toml" | head -1)"
app="$root/target/release/netherNode.app"
rm -rf "$app"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"
cp "$root/target/release/nethernode-gui" "$app/Contents/MacOS/"
cp "$root/assets/icons/nethernode.icns" "$app/Contents/Resources/"
cp "$root/LICENSE" "$app/Contents/Resources/LICENSE"
sed "s/@VERSION@/$version/g" "$root/packaging/macos/Info.plist" > "$app/Contents/Info.plist"
# Ad-hoc signature: required to run on Apple Silicon. Real distribution needs a Developer ID.
command -v codesign >/dev/null && codesign --force --deep --sign - "$app"
echo "built $app"
