#!/usr/bin/env bash
# Build a hardware release: the stack's schematic and board PDFs and a board
# file package, for board revision REV (hw/REVISION). Run by
# .github/workflows/release-hw.yml for a tag hw-rev-<REV>; runs the same here.
#
#   scripts/release_hw.sh [REV]     REV defaults to hw/REVISION
#
# The schematics, boards, panel, netlists and BOMs are not tracked: this
# builds them all from the committed sources (tools/, hw/lib, the routing
# in hw/*.route.json, hw/REVISION), whatever is already in hw/. Refuses
# unless REV is hw/REVISION, the build leaves every tracked file as it was,
# and ERC and DRC are clean. Writes build/release/hw/:
#   fastprom-<board>-sch.pdf, fastprom-<board>-pcb.pdf   (each stack board)
#   fastprom-hw-rev-<REV>.zip     the KiCad projects, PDFs, BOMs and the JLCPCB
#                                 order files for each board and the panel
#   SHA256SUMS
# KiCad runs in its container (scripts/kicad.sh; KICAD_ENGINE=docker for
# docker), the rest needs python3 with PyYAML.
set -euo pipefail
cd "$(dirname "$0")/.."
REV_FILE=$(tr -d '[:space:]' < hw/REVISION)
REV=${1:-$REV_FILE}
STACK="fastprom-lower fastprom-upper"
PANEL=fastprom-panel
KCLI="scripts/kicad.sh cli"
export KICAD=$KCLI          # the Makefile's kicad-cli for erc, netlist, pdf
OUT=build/release/hw
NAME=fastprom-hw-rev-$REV
# what is checked out, taken before the checks regenerate anything
DESC=$(git describe --tags --always --dirty)

die() { echo "release_hw: $*" >&2; exit 1; }

# run <what> <command...>: the command's output (less KiCad's wx debug
# chatter) to build/release/<what>.log, all of it shown if it fails
mkdir -p build/release
run() {
    local what=$1; shift
    echo "== $what"
    if ! "$@" > "build/release/$what.log" 2>&1; then
        grep -v "Debug: Adding duplicate" "build/release/$what.log" | tail -60
        die "$what failed: $*"
    fi
}

[ "$REV" = "$REV_FILE" ] || die "tag says rev $REV, hw/REVISION says $REV_FILE"

# Everything from the sources, rebuilt even where hw/ holds an up-to-date
# copy (-B). The generators may not touch a tracked file: a source they
# rewrite (the routing, the symbol library) is one that was not committed
# as they make it.
before=$(git status --porcelain)
run build make -s -B kicad netlist layout panel
if [ "$(git status --porcelain)" != "$before" ]; then
    diff <(echo "$before") <(git status --porcelain) || true
    die "the generators changed tracked sources; rebuild and commit them"
fi
echo "schematics, boards and panel: built from the sources"

for b in $STACK; do
    grep -q "(rev \"$REV\")" hw/$b.kicad_sch || die "hw/$b.kicad_sch title block is not rev $REV"
    grep -q "(rev \"$REV\")" hw/$b.kicad_pcb || die "hw/$b.kicad_pcb title block is not rev $REV"
    grep -q "FastPROM rev $REV " hw/$b.kicad_pcb || die "hw/$b.kicad_pcb silkscreen is not rev $REV"
done

for b in $STACK; do
    echo "== erc-$b"
    $KCLI sch erc --severity-all --exit-code-violations \
        --output hw/erc-$b.rpt hw/$b.kicad_sch > build/release/erc-$b.log 2>&1 \
        || { cat build/release/erc-$b.log hw/erc-$b.rpt 2>/dev/null; die "$b: ERC not clean"; }
done
run grade make -s grade
run stack make -s stack
run jlc make -s jlc               # DRC each board (zero violations), then its order files
run jlc-panel make -s jlc-panel   # the panel's DRC and order files, every pair assembled
run pdf make -s pdf
for f in grade stack jlc jlc-panel; do
    grep -v "Debug: Adding duplicate" build/release/$f.log
done

rm -rf "$OUT"
pkg=$OUT/$NAME
mkdir -p "$pkg"/kicad "$pkg"/pdf "$pkg"/jlc
cp hw/REVISION hw/sym-lib-table hw/fp-lib-table "$pkg"/kicad/
cp -r hw/lib "$pkg"/kicad/
# The licences (LICENSE): the hardware is CERN-OHL-P-2.0, eighteen library
# footprints derived from KiCad's are CC-BY-SA-4.0, this README CC-BY-4.0.
cp LICENSE "$pkg"/
mkdir -p "$pkg"/LICENSES
cp LICENSES/CERN-OHL-P-2.0.txt LICENSES/CC-BY-SA-4.0.txt LICENSES/CC-BY-4.0.txt \
    "$pkg"/LICENSES/
for b in $STACK $PANEL; do
    for ext in kicad_pro kicad_dru kicad_sch kicad_pcb; do
        [ -f hw/$b.$ext ] && cp hw/$b.$ext "$pkg"/kicad/
    done
    mkdir -p "$pkg"/jlc/$b
    cp build/jlc/$b/$b-gerbers.zip build/jlc/$b/$b-bom.csv build/jlc/$b/$b-cpl.csv "$pkg"/jlc/$b/
done
for b in $STACK; do
    cp hw/$b.net hw/$b-bom.csv "$pkg"/kicad/
    cp build/pdf/$b-sch.pdf build/pdf/$b-pcb.pdf "$pkg"/pdf/
    cp build/pdf/$b-sch.pdf build/pdf/$b-pcb.pdf "$OUT"/
done
# What JLCPCB's order takes, beside the package: the panel's gerbers,
# BOM and CPL, and every field of the order form (hw/ORDER-JLCPCB.md).
cp build/jlc/$PANEL/$PANEL-gerbers.zip build/jlc/$PANEL/$PANEL-bom.csv \
    build/jlc/$PANEL/$PANEL-cpl.csv hw/ORDER-JLCPCB.md "$OUT"/
cp hw/ORDER-JLCPCB.md "$pkg"/
cat > "$pkg"/README.txt <<EOF
FastPROM hardware, board revision $REV ($DESC)

kicad/   KiCad 10 projects, built from the repository's sources: the two
         stack boards (fastprom-lower under fastprom-upper), the assembly
         panel, libraries, netlists and BOMs
pdf/     schematics and routed boards
jlc/     JLCPCB order files: gerbers (zip), BOM and CPL for each board, and
         for the panel of five matched pairs (docs/07-bom.md: ordering)
ORDER-JLCPCB.md  every field of JLCPCB's order form, set for this design
LICENSE  copyright 2026 mxshift; the hardware is CERN-OHL-P-2.0, eighteen
         footprints derived from KiCad's library CC-BY-SA-4.0 (LICENSES/)
EOF
(cd "$OUT" && python3 -m zipfile -c "$NAME.zip" "$NAME" && rm -rf "$NAME" \
    && sha256sum -- *.zip *.pdf *.csv *.md > SHA256SUMS)
echo "release_hw: rev $REV -> $OUT"
ls -1 "$OUT"
