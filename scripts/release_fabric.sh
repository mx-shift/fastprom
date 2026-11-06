#!/usr/bin/env bash
# Add the fabric bitstreams to a published firmware release. CI cannot build
# them (Supra comes with AGM's AgRV_pio SDK, which is not redistributable),
# so after release-fw.yml has published fw-v<VERSION>, run this on a machine
# with the SDK, at that tag:
#
#   git checkout fw-v0.1.0
#   scripts/release_fabric.sh fw-v0.1.0            # build and check
#   scripts/release_fabric.sh fw-v0.1.0 --upload   # ... and upload (gh)
#
# It downloads the release's firmware, DFU stage and recovery loader and
# checks them against its SHA256SUMS, so the bundle carries exactly what was
# released, not a local rebuild. It builds the BRAM-mode fabric for every
# part the upper board supports (tools/ag32/run_flow.sh), rebuilds the update
# page with the fabrics in its bundle, runs the browser tests, and writes
# build/release/fabric/:
#   fastprom-fabric-<VERSION>.zip   fabric-<part>.bin and its manifest page,
#                                   for each part
#   fastprom-web-<VERSION>.zip      the page, now with the fabrics
#   SHA256SUMS                      the release's, with both zips updated
# --upload replaces the release's fastprom-web zip and SHA256SUMS and adds
# the fabric zip (gh release upload --clobber).
#
# Environment: AGRV_PIO (default /opt/agm/AgRV_pio).
set -euo pipefail
cd "$(dirname "$0")/.."
TAG=${1:?usage: scripts/release_fabric.sh fw-v<VERSION> [--upload]}
UPLOAD=${2:-}
VERSION=${TAG#fw-v}
PIO=${AGRV_PIO:-/opt/agm/AgRV_pio}
OUT=build/release/fabric
DL=$OUT/download
AG32=$OUT/ag32
NAME=fastprom-fw-$VERSION

die() { echo "release_fabric: $*" >&2; exit 1; }

[ "$TAG" != "$VERSION" ] || die "$TAG is not a firmware tag (fw-v<VERSION>)"
[ -z "$UPLOAD" ] || [ "$UPLOAD" = --upload ] || die "unknown option $UPLOAD"
[ "$(git describe --tags --exact-match HEAD 2>/dev/null)" = "$TAG" ] \
    || die "HEAD is not $TAG: git checkout $TAG first"
[ -z "$(git status --porcelain --untracked-files=no)" ] \
    || die "the working tree has changes; the fabric must be built from $TAG as tagged"
[ -x "$PIO/packages/tool-agrv_logic/bin/af_cmd" ] || die "no AgRV_pio SDK at $PIO (AGRV_PIO)"
command -v gh > /dev/null || die "needs gh, logged in, to download the release"

rm -rf "$OUT"
mkdir -p "$DL" "$AG32"

# ---- the release's own binaries, checked against its SHA256SUMS -------------
gh release download "$TAG" --dir "$DL" --pattern SHA256SUMS \
    --pattern "$NAME.bin" --pattern "$NAME-dfu.bin" --pattern "$NAME-recovery.bin"
(cd "$DL" && grep -E " $NAME(-dfu|-recovery)?\.bin$" SHA256SUMS | sha256sum -c --quiet -) \
    || die "the downloaded binaries do not match the release's SHA256SUMS"
echo "release_fabric: $TAG's firmware, DFU stage and recovery loader, checked"

# ---- the fabrics -----------------------------------------------------------
PARTS=$(python3 -c "
import yaml
b = yaml.safe_load(open('tools/boardmap.yaml'))['boards']['fastprom-upper']
print(' '.join(b['supports']))")
for part in $PARTS; do
    echo "release_fabric: fabric for $part"
    sh tools/ag32/run_flow.sh "$PIO" "$part" bram "$AG32/$part-bram" > "$AG32/$part.log" 2>&1 \
        || { tail -30 "$AG32/$part.log"; die "fabric for $part failed (log: $AG32/$part.log)"; }
    grep -E "address->data|sys_clock" "$AG32/$part.log" | tail -2
done

# ---- the page, with the release's binaries and these fabrics ----------------
# make web bundles whatever is in build/fw, so put the release's binaries
# there under the names the build gives them.
mkdir -p build/fw
cp "$DL/$NAME.bin" build/fw/fastprom-ag32.bin
cp "$DL/$NAME-dfu.bin" build/fw/fastprom-dfu.bin
cp "$DL/$NAME-recovery.bin" build/fw/fastprom-recovery.bin
make -s web WEB_FABRICS="$AG32" > /dev/null
make -s web-test
python3 - "$PARTS" build/web/bundle <<'EOF'
import json, os, sys
parts, bundle = sys.argv[1].split(), sys.argv[2]
m = json.load(open(os.path.join(bundle, "manifest.json")))
have = sorted(f["part"] for f in m["fabrics"])
if have != sorted(parts):
    sys.exit(f"release_fabric: the bundle has fabrics for {have}, expected {sorted(parts)}")
EOF

zipdir() {  # zipdir <dir> <zip> <prefix> [glob]: name order, fixed times
    python3 - "$@" <<'EOF'
import fnmatch, os, sys, zipfile
src, dst, prefix = sys.argv[1:4]
pat = sys.argv[4] if len(sys.argv) > 4 else "*"
with zipfile.ZipFile(dst, "w", zipfile.ZIP_DEFLATED) as z:
    for root, dirs, files in os.walk(src):
        dirs.sort()
        for f in sorted(files):
            if not fnmatch.fnmatch(f, pat):
                continue
            p = os.path.join(root, f)
            info = zipfile.ZipInfo(os.path.join(prefix, os.path.relpath(p, src)),
                                   (1980, 1, 1, 0, 0, 0))
            info.compress_type = zipfile.ZIP_DEFLATED
            info.external_attr = 0o644 << 16
            with open(p, "rb") as fh:
                z.writestr(info, fh.read())
EOF
}
zipdir build/web "$OUT/fastprom-web-$VERSION.zip" fastprom-web
zipdir build/web/bundle "$OUT/fastprom-fabric-$VERSION.zip" "fastprom-fabric-$VERSION" "fabric-*"

# The release's SHA256SUMS with the two zips' new hashes.
{ grep -v -E " fastprom-(web|fabric)-$VERSION\.zip$" "$DL/SHA256SUMS"
  (cd "$OUT" && sha256sum -- "fastprom-web-$VERSION.zip" "fastprom-fabric-$VERSION.zip"); } \
    > "$OUT/SHA256SUMS"
echo "release_fabric: $TAG -> $OUT"
ls -1 "$OUT" | grep -v -E "^(download|ag32)$"

if [ "$UPLOAD" = --upload ]; then
    gh release upload "$TAG" --clobber \
        "$OUT/fastprom-fabric-$VERSION.zip" "$OUT/fastprom-web-$VERSION.zip" "$OUT/SHA256SUMS"
    echo "release_fabric: uploaded to $TAG"
else
    echo "release_fabric: not uploaded; rerun with --upload to add them to $TAG"
fi
