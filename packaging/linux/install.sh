#!/usr/bin/env bash
# Installs netherNode for the current user (no root needed):
#   binaries -> ~/.local/bin, icons -> ~/.local/share/icons, menu entry -> ~/.local/share/applications
# Usage: packaging/linux/install.sh [--uninstall]
# Build first with: cargo build --release
set -euo pipefail
root="$(cd "$(dirname "$0")/../.." && pwd)"
bin="$HOME/.local/bin"
share="${XDG_DATA_HOME:-$HOME/.local/share}"
sizes=(16 24 32 48 64 128 256 512)

if [ "${1:-}" = "--uninstall" ]; then
    rm -f "$bin/nethernode" "$bin/nethernode-gui" "$share/applications/nethernode.desktop"
    for s in "${sizes[@]}"; do rm -f "$share/icons/hicolor/${s}x${s}/apps/nethernode.png"; done
    echo "netherNode uninstalled (your instances and accounts in $share/nethernode were kept)"
    exit 0
fi

for b in nethernode nethernode-gui; do
    [ -x "$root/target/release/$b" ] || { echo "missing target/release/$b; run: cargo build --release" >&2; exit 1; }
    install -Dm755 "$root/target/release/$b" "$bin/$b"
done
for s in "${sizes[@]}"; do
    install -Dm644 "$root/assets/icons/png/$s.png" "$share/icons/hicolor/${s}x${s}/apps/nethernode.png"
done
install -Dm644 "$root/packaging/linux/nethernode.desktop" "$share/applications/nethernode.desktop"
command -v update-desktop-database >/dev/null && update-desktop-database -q "$share/applications" || true
command -v gtk-update-icon-cache >/dev/null && gtk-update-icon-cache -q -t "$share/icons/hicolor" || true
echo "installed: run 'nethernode-gui' or find netherNode in your app menu"
case ":$PATH:" in *":$bin:"*) ;; *) echo "note: $bin is not on your PATH" ;; esac
