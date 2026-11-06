#!/usr/bin/env bash
# Build a firmware release: the AG32 firmware at version VERSION (fw/
# Cargo.toml's workspace version, which the firmware reports over USB). Run
# by .github/workflows/release-fw.yml for a tag fw-v<VERSION>; runs the same
# here. Firmware is released apart from the boards (scripts/release_hw.sh,
# tags hw-rev-<REV>).
#
#   scripts/release_fw.sh [VERSION]    VERSION defaults to fw/Cargo.toml's
#
# Refuses unless VERSION is the crate version and the host tests, lints and
# the page's browser tests pass. Writes build/release/fw/:
#   fastprom-fw-<VERSION>.elf, .bin   the application, header stamped, for
#                                     flash at 0x80008000
#   fastprom-fw-<VERSION>-dfu.elf, .bin  the DFU stage, for 0x80000000
#   fastprom-fw-<VERSION>-recovery.elf, .bin  the recovery loader, which
#                                     the page uploads into SRAM at
#                                     0x20010000 through the mask ROM
#   fastprom-fw-<VERSION>-layout.txt  its sections and the flash map
#   fastprom-web-<VERSION>.zip        the update page (make web) with this
#                                     application, the DFU stage and the
#                                     recovery loader in its bundle; no
#                                     fabric (it needs AGM's SDK), so the
#                                     page offers firmware, and recovers a
#                                     board as far as its update mode
#   SHA256SUMS
# Not the combined flash image (make fw-batch): that needs the fabric
# bitstream and AGM's gen_batch, from the AgRV_pio SDK, which is not
# redistributable. Builds in the fastprom-rust image (make fw-image).
set -euo pipefail
cd "$(dirname "$0")/.."
CRATE=$(python3 - <<'EOF'
import re
s = open("fw/Cargo.toml").read()
sect = s.split("[workspace.package]", 1)[1].split("\n[", 1)[0]
print(re.search(r'^version\s*=\s*"([^"]+)"', sect, re.M).group(1))
EOF
)
VERSION=${1:-$CRATE}
OUT=build/release/fw
NAME=fastprom-fw-$VERSION

die() { echo "release_fw: $*" >&2; exit 1; }

[ "$VERSION" = "$CRATE" ] || die "tag says $VERSION, fw/Cargo.toml says $CRATE"

make -s fw-image > /dev/null
make -s fw-test
make -s fw-lint
rm -f build/fw/images.bin            # the layout report of the firmware alone
make -s fw-build FW_LOGIC=none > /dev/null    # nor a local fabric
# The update page with this firmware's bundle, tested in headless Chromium
# against the simulated board. No hosting: the site is a release asset.
make -s web > /dev/null
make -s web-test

rm -rf "$OUT"
mkdir -p "$OUT"
cp build/fw/fastprom-ag32.elf "$OUT/$NAME.elf"
cp build/fw/fastprom-ag32.bin "$OUT/$NAME.bin"
cp build/fw/fastprom-dfu.elf "$OUT/$NAME-dfu.elf"
cp build/fw/fastprom-dfu.bin "$OUT/$NAME-dfu.bin"
cp build/fw/fastprom-recovery.elf "$OUT/$NAME-recovery.elf"
cp build/fw/fastprom-recovery.bin "$OUT/$NAME-recovery.bin"
cp build/fw/layout.txt "$OUT/$NAME-layout.txt"
# The site, files in name order with fixed times, so the same tree zips to
# the same bytes.
python3 - "build/web" "$OUT/fastprom-web-$VERSION.zip" <<'EOF'
import os, sys, zipfile
src, dst = sys.argv[1], sys.argv[2]
with zipfile.ZipFile(dst, "w", zipfile.ZIP_DEFLATED) as z:
    for root, dirs, files in os.walk(src):
        dirs.sort()
        for f in sorted(files):
            p = os.path.join(root, f)
            info = zipfile.ZipInfo(os.path.join("fastprom-web", os.path.relpath(p, src)),
                                   (1980, 1, 1, 0, 0, 0))
            info.compress_type = zipfile.ZIP_DEFLATED
            info.external_attr = 0o644 << 16
            with open(p, "rb") as fh:
                z.writestr(info, fh.read())
EOF
(cd "$OUT" && sha256sum -- "$NAME".* "$NAME"-dfu.* "$NAME"-recovery.* "$NAME"-layout.txt \
    "fastprom-web-$VERSION.zip" > SHA256SUMS)
echo "release_fw: $VERSION -> $OUT"
cat "$OUT/$NAME-layout.txt"
ls -1 "$OUT"
