#!/usr/bin/env bash
# Build a firmware release: the AG32 firmware at version VERSION (fw/
# Cargo.toml's workspace version, which the firmware reports over USB). Run
# by .github/workflows/release-fw.yml for a tag fw-v<VERSION>; runs the same
# here. Firmware is released apart from the boards (scripts/release_hw.sh,
# tags hw-rev-<REV>).
#
#   scripts/release_fw.sh [VERSION]    VERSION defaults to fw/Cargo.toml's
#
# Refuses unless VERSION is the crate version and the host tests and lints
# pass. Writes build/release/fw/:
#   fastprom-fw-<VERSION>.elf, .bin   the firmware, for flash at 0x80000000
#   fastprom-fw-<VERSION>-layout.txt  its sections and the flash map
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

rm -rf "$OUT"
mkdir -p "$OUT"
cp build/fw/fastprom-ag32.elf "$OUT/$NAME.elf"
cp build/fw/fastprom-ag32.bin "$OUT/$NAME.bin"
cp build/fw/layout.txt "$OUT/$NAME-layout.txt"
(cd "$OUT" && sha256sum -- "$NAME".* "$NAME"-layout.txt > SHA256SUMS)
echo "release_fw: $VERSION -> $OUT"
cat "$OUT/$NAME-layout.txt"
ls -1 "$OUT"
