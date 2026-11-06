#!/bin/sh
# build_dfu_fabric.sh - the DFU stage's own fabric configuration: the clocks
# and USB0, no other fabric I/O (docs/10-updates.md, "DFU mode"; the recipe
# in docs/06-build-and-flash.md, "The DFU stage's fabric").
#
#   tools/gen_pins.py --dfu-ve   boardmap.yaml -> dfu_usb.ve (clocks, USB0, no pins)
#   gen_vlog (AgRV_pio)          .ve -> top.vx (AGM's netlist of the CPU block), top.vex
#   af_cmd gen_logic.tcl         Supra: place, route, bitgen -> logic/top.bin,
#                                compressed, and logic/top_raw.bin, the same plain
#   tools/ag32/check_dfu_fabric.py   the pin evidence from Supra's own reports
#                                    and the decompression check -> <work>/pins.txt
#
# This is AGM's flow for a design without custom logic, the one its DFU
# example is built with: no yosys and no IP, so none of run_flow.sh's patches
# apply. The .ve has no pin line, so there is no port Supra could place.
#
# usage: tools/ag32/build_dfu_fabric.sh <AgRV_pio dir> [--install] [work dir]
#   default work dir build/ag32/dfu-usb. --install copies the result into
#   fw/fastprom-dfu/fabric/: dfu_usb.bin (compressed), dfu_usb.ve,
#   dfu_usb.manifest and dfu_usb.evidence.txt. Without it, the build is checked against what is
#   committed there: the same .ve and inputs (the manifest's inputs_crc32).
#   Not the same bytes: Supra's router does not always choose the same clock
#   routing for the same inputs (a few routing bits differ between runs), so
#   a rebuild is a valid bitstream of the same design, not a copy.
#
# Environment: BOARD (default fastprom-upper), SUPRA_IMAGE (default ct-supra,
# tools/ag32/Containerfile).
set -e
[ $# -ge 1 ] || { sed -n '2,27p' "$0"; exit 2; }
PIO=$(realpath "$1"); shift
WORK=build/ag32/dfu-usb
INSTALL=
for a in "$@"; do
  case $a in
    --install) INSTALL=1 ;;
    *) WORK=$a ;;
  esac
done
BOARD=${BOARD:-fastprom-upper}
IMAGE=${SUPRA_IMAGE:-ct-supra}
DEVICE=AGRV2KQ32
REPO=$(cd "$(dirname "$0")/../.." && pwd)
HERE=$REPO/tools/ag32
DEST=$REPO/fw/fastprom-dfu/fabric
WORK=$(realpath -m "$WORK")
L=$WORK/logic
[ -x "$PIO/packages/tool-agrv_logic/bin/af_cmd" ] || {
  echo "$PIO is not an AgRV_pio SDK (no packages/tool-agrv_logic/bin/af_cmd)" >&2; exit 2; }

rm -rf "$WORK"; mkdir -p "$L"
python3 "$REPO/tools/gen_pins.py" --dfu-ve --board "$BOARD" --out "$WORK" >/dev/null
cp "$WORK/dfu_usb.ve" "$L/top.ve"
cp "$HERE/usb0.asf" "$HERE/dfu_usb.sdc" "$HERE/dfu_usb_pre.tcl" "$HERE/dfu_usb_post.tcl" "$L/"

(cd "$L" &&
 "$PIO/python_dist/python3" "$PIO/platforms/AgRV/etc/gen_vlog" -c top.hx \
     -d $DEVICE top.ve top.vx -x top.vex -F > gen_vlog.log 2>&1) || {
  cat "$L/gen_vlog.log"; exit 1; }

# As AgRV_pio's builder runs gen_logic.tcl for a board without custom logic
# (platforms/AgRV/builder/main.py, GENERFLAGS). LOGIC_COMPRESS is what
# board_logic.compress = true passes: gen_logic.tcl then sets
# ON_CHIP_BITSTREAM_DECOMPRESSION ON and bitgen writes the LZW-coded bitstream
# FCB_AutoDecompress reads. IP_SDC makes gen_logic.tcl read our constraints
# (with ::HSI_PERIOD/::HSE_PERIOD from top.hx) instead of its bare
# derive_pll_clocks, which leaves PIN_HSI to an auto constraint (a warning).
# LOGIC_PRE turns the coverage limit off (dfu_usb_pre.tcl says why);
# LOGIC_POST writes the plain bitstream of the same routing.
podman run --rm --security-opt label=disable -v "$PIO:/pio" -v "$L:/work" -w /work "$IMAGE" \
    /pio/packages/tool-agrv_logic/bin/af_cmd -L logic_log.txt \
    -X "set LOGIC_TYPE {VX}" -X "set LOGIC_DB {logic_db}" \
    -X "set LOGIC_DEVICE {$DEVICE}" -X "set LOGIC_DESIGN {top}" -X "set LOGIC_TOPPIN {false}" \
    -X "set LOGIC_DIR {.}" -X "set LOGIC_VX {top.vx}" -X "set VEX_FILE {top.vex}" \
    -X "set LOGIC_PRE {dfu_usb_pre.tcl}" -X "set LOGIC_ASF {usb0.asf}" \
    -X "set LOGIC_POST {dfu_usb_post.tcl}" -X "set IP_SDC {dfu_usb.sdc}" \
    -X "set LOGIC_BIN {top.bin}" -X "set LOGIC_COMPRESS true" \
    -F /pio/platforms/AgRV/etc/gen_logic.tcl > "$L/supra_console.log" 2>&1 || {
  tail -40 "$L/supra_console.log"; exit 1; }
[ -f "$L/top.bin" ] && [ -f "$L/top_raw.bin" ] || {
  tail -40 "$L/supra_console.log"; echo "no top.bin and top_raw.bin" >&2; exit 1; }

python3 "$HERE/check_dfu_fabric.py" "$WORK" --sdk "$PIO" > "$WORK/pins.txt" || {
  cat "$WORK/pins.txt"; exit 1; }
cat "$WORK/pins.txt"

if [ -n "$INSTALL" ]; then
  mkdir -p "$DEST"
  cp "$L/top.bin" "$DEST/dfu_usb.bin"
  cp "$WORK/dfu_usb.ve" "$DEST/dfu_usb.ve"
  cp "$WORK/manifest" "$DEST/dfu_usb.manifest"
  cp "$WORK/pins.txt" "$DEST/dfu_usb.evidence.txt"
  echo "installed in fw/fastprom-dfu/fabric/"
else
  inputs() { grep '^inputs_crc32 ' "$1"; }
  if cmp -s "$WORK/dfu_usb.ve" "$DEST/dfu_usb.ve" &&
     [ "$(inputs "$WORK/manifest")" = "$(inputs "$DEST/dfu_usb.manifest")" ]; then
    echo "same inputs as the committed fw/fastprom-dfu/fabric/"
  else
    echo "the committed fw/fastprom-dfu/fabric/ was built from other inputs:" >&2
    echo "  rerun with --install, and commit" >&2
    exit 1
  fi
fi
