#!/bin/sh
# run_flow.sh - place and route FastPROM's fabric for the stack's AG32
# (AGRV2KQ32, QFN-32) with AGM's free native flow, and report its timing.
#
#   tools/gen_pins.py          boardmap.yaml -> <part>.ve / _params.vh
#   gen_vlog (AgRV_pio)        .ve -> AG32 top-level wrapper + .vex
#   pre_logic.tcl (af_cmd)     -> project files, af_map.tcl, af_run.tcl
#   patch_af_map.py            fix AGM's generated yosys script
#   af_map (AGM's yosys 0.47)  RTL -> top.vqm
#   af_cmd af_run.tcl, MODE NATIVE (Supra) -> map/place/route, timing, top.bin
#
# No Quartus: Web Edition drops AGM's CPU design partition and folds the bus.
# The recipe, the patches and the pitfalls: docs/06-build-and-flash.md.
#
# usage: tools/ag32/run_flow.sh <AgRV_pio dir> <part> <logic|bram> <work dir>
#        [image kind: random|palette|decode, default random] [bank bits, default 0]
#
# Environment: BOARD (default fastprom-upper), SYNC_STAGES (BRAM, 1),
# SYSCLK (MHz, default the .ve's 100; BUSCLK follows at half), VE_EDIT (a sed
# script applied to the .ve, for what-if pin experiments - not the board's map),
# SEED (Supra's placement seed, default 666 as AGM ships it), RDCLK (MHz for
# BRAM mode's read port on its own PLL output, default the .ve's 200; 0 puts it
# on sys_clock),
# FP_DEFINES (extra Verilog macros for opt-in HDL experiments, space separated),
# SUPRA_IMAGE (default ct-supra: Ubuntu 22.04 with the X/GL runtime libraries
# the AGM binaries link; tools/ag32/Containerfile builds it).
set -e
[ $# -ge 4 ] || { sed -n '2,30p' "$0"; exit 2; }
PIO=$(realpath "$1"); PART=$2; MODE=$3; WORK=$(realpath -m "$4")
KIND=${5:-random}; BANKW=${6:-0}
BOARD=${BOARD:-fastprom-upper}
SYNC_STAGES=${SYNC_STAGES:-1}
IMAGE=${SUPRA_IMAGE:-ct-supra}
DEVICE=AGRV2KQ32
REPO=$(cd "$(dirname "$0")/../.." && pwd)
HERE=$REPO/tools/ag32

case $MODE in
  logic) MODE_BRAM=0 ;;
  bram)  MODE_BRAM=1 ;;
  *) echo "mode must be logic or bram" >&2; exit 2 ;;
esac
[ -x "$PIO/packages/tool-agrv_logic/bin/af_cmd" ] || {
  echo "$PIO is not an AgRV_pio SDK (no packages/tool-agrv_logic/bin/af_cmd)" >&2; exit 2; }

rm -rf "$WORK"; mkdir -p "$WORK/logic" "$WORK/gen"

# ---- constraints and parameters from the board map -------------------------
python3 "$REPO/tools/gen_pins.py" --part "$PART" --board "$BOARD" \
    --banks "$BANKW" --out "$WORK/gen" >/dev/null
cp "$WORK/gen/$PART.ve" "$WORK/top.ve"
if [ -n "$VE_EDIT" ]; then  # what-if pin experiments: a sed script applied to the .ve
  sed -i "$VE_EDIT" "$WORK/top.ve"
fi
if [ -n "$SYSCLK" ]; then   # sys_clock: the AHB clock, and BRAM mode's sampling clock
  sed -i "s/^SYSCLK .*/SYSCLK $SYSCLK/; s/^BUSCLK .*/BUSCLK $((SYSCLK / 2))/" "$WORK/top.ve"
fi
# BRAM read clock: gen_pins.py puts it on PLL output 1 (PLLCLK1, 200 MHz by
# default). RDCLK=<MHz> changes it; RDCLK=0 clocks the read port from
# sys_clock instead, as the first builds did.
if [ -n "$RDCLK" ]; then
  if [ "$RDCLK" = 0 ]; then
    sed -i '/^PLLCLK1 /d; /^PLL_CLKOUT1 rd_clk/d' "$WORK/top.ve"
  elif grep -q '^PLLCLK1 ' "$WORK/top.ve"; then
    sed -i "s/^PLLCLK1 [^ ]*/PLLCLK1 $RDCLK/" "$WORK/top.ve"
  else
    printf 'PLLCLK1 %s\nPLL_CLKOUT1 rd_clk:OUTPUT\n' "$RDCLK" >> "$WORK/top.ve"
  fi
fi
if grep -q '^PLL_CLKOUT1 rd_clk' "$WORK/top.ve"; then
  FP_RDCLK=1; RDCLK_NAME=pll_inst\|clkout1
else
  FP_RDCLK=; RDCLK_NAME=pll_inst\|clkout0
fi
VH=$WORK/gen/${PART}_params.vh
p() { sed -n "s/.*localparam [^ ]* *$1 *= *\([^;]*\);.*/\1/p" "$VH"; }
PROM_AW=$(p PROM_AW); DW=$(p DW); NCE=$(p NCE); CEMASK=$(p CE_ACTIVE_LOW)
AW=$((PROM_AW + BANKW))

# ---- the IP source: one file, as pre_logic.tcl expects ---------------------
IP=$WORK/logic/fastprom_ip.v
{
  echo "// Assembled by tools/ag32/run_flow.sh: $PART $MODE ($KIND, $BANKW bank bits) on $BOARD"
  echo "\`define FP_PROM_AW $PROM_AW"
  echo "\`define FP_DW $DW"
  echo "\`define FP_NCE $NCE"
  echo "\`define FP_CE_ACTIVE_LOW $CEMASK"
  echo "\`define FP_MODE_BRAM $MODE_BRAM"
  echo "\`define FP_SYNC_STAGES $SYNC_STAGES"
  echo "\`define FP_BANKW $BANKW"
  [ -z "$FP_RDCLK" ] || echo "\`define FP_RDCLK"
  for d in $FP_DEFINES; do echo "\`define $d"; done   # opt-in HDL experiments
  cat "$HERE/fastprom_ip.v" "$REPO/hdl/fastprom_top.v" "$REPO/hdl/fastprom_enable.v" \
      "$REPO/hdl/fastprom_ahb_slave.v" "$REPO/hdl/fastprom_rom_bram.v"
} > "$IP"
if [ $MODE_BRAM = 0 ]; then
  # LOGIC mode: the contents as an explicit case table, not an initialised
  # array, so nothing can be read as a RAM.
  python3 "$REPO/tools/mkimage.py" --kind "$KIND" --depth $((1 << AW)) \
      --width "$DW" --out "$WORK/gen/image.hex" >/dev/null
  python3 "$REPO/tools/gen_rom_logic.py" --image "$WORK/gen/image.hex" \
      --aw "$AW" --dw "$DW" --module fastprom_rom_logic --out "$WORK/gen/rom.v" >/dev/null
  cat "$WORK/gen/rom.v" >> "$IP"
else
  # BRAM mode never reaches the LOGIC ROM, but fastprom_top names it.
  cat "$REPO/hdl/fastprom_rom_logic.v" >> "$IP"
fi

cd "$WORK"
"$PIO/python_dist/python3" "$PIO/platforms/AgRV/etc/gen_vlog" -c logic/top.hx -s \
    -d $DEVICE -x logic/top.vex top.ve logic/top.v -m logic/fastprom_ip.v -F \
    > logic/gen_vlog.log 2>&1 || { cat logic/gen_vlog.log; exit 1; }

SUPRA="podman run --rm --security-opt label=disable -v $PIO:/pio -v $WORK:/work -w /work $IMAGE"
BIN=/pio/packages/tool-agrv_logic/bin
$SUPRA $BIN/af_cmd -L logic/logic_log.txt \
    -X "set LOGIC_DEVICE {$DEVICE}" -X "set LOGIC_DESIGN {top}" -X "set LOGIC_TOPPIN {false}" \
    -X "set LOGIC_DIR {logic}" -X "set LOGIC_VV {top.v}" -X "set LOGIC_ASF {}" -X "set LOGIC_PRE {}" \
    -X "set LOGIC_POST {}" -X "set DESIGN_ASF {}" -X "set DESIGN_PRE {}" -X "set DESIGN_POST {}" \
    -X "set LOGIC_FORCE {true}" -X "set VEX_FILE {top.vex}" -X "set AGF_FILE {}" -X "set SDC_FILE {}" \
    -X "set IP_VV {fastprom_ip.v}" -X "set ORIGINAL_PIN {-}" -F /pio/platforms/AgRV/etc/pre_logic.tcl \
    > logic/pre_logic.log 2>&1 || { tail -20 logic/pre_logic.log; exit 1; }

cat "$HERE/fastprom.sdc" >> logic/top.sdc
[ $MODE_BRAM = 1 ] && sed "s/pll_inst|clkout0/$RDCLK_NAME/g" "$HERE/fastprom_bram.sdc" >> logic/top.sdc

python3 "$HERE/patch_af_map.py" "$PIO" logic/af_map.tcl
$SUPRA sh -c "cd /work/logic && $BIN/af_map -c af_map.tcl > map.log 2>&1" || {
  tail -30 logic/map.log; exit 1; }
$SUPRA sh -c "cd /work/logic && $BIN/af_cmd -L supra_run.log -X 'set MODE NATIVE' \
    -X 'set SEED ${SEED:-666}' -F af_run.tcl \
    > supra_console.log 2>&1" || true
[ -f logic/top.bin ] || { tail -40 logic/supra_console.log; echo "no top.bin" >&2; exit 1; }

python3 "$HERE/timing_report.py" "$WORK" "$PART" "$MODE" "$KIND" "$BANKW" "$BOARD"
