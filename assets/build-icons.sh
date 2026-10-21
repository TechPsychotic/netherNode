#!/usr/bin/env bash
# Regenerates every icon file from icon.svg (48 px and up) and icon-small.svg (16-32 px).
# Needs ImageMagick 7 (`magick`) with librsvg, plus python3. The outputs are committed, so
# normal builds do not need these tools.
set -euo pipefail
cd "$(dirname "$0")"
out=icons
rm -rf "$out" && mkdir -p "$out/png"

render() { # render <size>
    local src=icon.svg
    [ "$1" -le 32 ] && src=icon-small.svg
    magick -background none -density 384 "$src" -resize "$1x$1" -strip "PNG32:$out/png/$1.png"
}
for size in 16 24 32 48 64 128 256 512 1024; do render "$size"; done

# Windows
magick $(for s in 16 24 32 48 64 128 256; do echo "$out/png/$s.png"; done) "$out/nethernode.ico"

# macOS .icns: a list of (type, PNG) entries. Modern macOS accepts PNG data for all of these.
python3 -I - "$out" <<'PY'
import struct, sys, pathlib
out = pathlib.Path(sys.argv[1])
entries = [("icp4", 16), ("icp5", 32), ("ic11", 32), ("icp6", 64), ("ic12", 64), ("ic07", 128),
           ("ic08", 256), ("ic13", 256), ("ic09", 512), ("ic14", 512), ("ic10", 1024)]
body = b"".join(
    kind.encode() + struct.pack(">I", 8 + len(data)) + data
    for kind, size in entries
    for data in [(out / "png" / f"{size}.png").read_bytes()]
)
(out / "nethernode.icns").write_bytes(b"icns" + struct.pack(">I", 8 + len(body)) + body)
PY
echo "icons written to $(pwd)/$out"
