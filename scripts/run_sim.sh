#!/usr/bin/env bash
# Run the whole simulation suite. Requires iverilog on PATH.
set -euo pipefail
cd "$(dirname "$0")/.."
mkdir -p build

echo "== generating test images =="
python3 tools/mkimage.py --kind random  --depth 2048 --width 8 --out build/img_rand.hex
python3 tools/mkimage.py --kind palette --depth 2048 --width 8 --out build/img_pal.hex
python3 tools/mkimage.py --kind decode  --depth 2048 --width 8 --out build/img_dec.hex
python3 tools/mkimage.py --kind random  --depth 256  --width 8 --seed 7 --out build/img_oc.hex

fail=0
run() {
    local name=$1; shift
    echo "== $name =="
    if iverilog -g2012 -o "build/$name.vvp" "$@" hdl/*.v 2>&1 | grep -v '^$'; then :; fi
    if ! vvp "build/$name.vvp" | tail -6; then
        fail=1
    fi
}

echo "== tb_equiv: generated case ROM vs reference model =="
for k in rand pal dec; do
    python3 tools/gen_rom_logic.py --image "build/img_$k.hex" --aw 11 --dw 8 \
        --module fastprom_rom_logic_gen --out "build/romgen_$k.v" > /dev/null
    iverilog -g2012 -Ptb_equiv.IMAGE="\"build/img_$k.hex\"" \
        -o "build/tb_equiv_$k.vvp" \
        sim/tb_equiv.v hdl/fastprom_rom_logic.v "build/romgen_$k.v" 2>/dev/null
    printf '  %-5s ' "$k"
    if ! vvp "build/tb_equiv_$k.vvp" | grep -E 'PASS|FAIL'; then fail=1; fi
    vvp "build/tb_equiv_$k.vvp" | grep -q PASS || fail=1
done

echo "== tb_ahb =="
iverilog -g2012 -o build/tb_ahb.vvp sim/tb_ahb.v hdl/fastprom_ahb_slave.v 2>/dev/null
if ! vvp build/tb_ahb.vvp | tail -3; then fail=1; fi
vvp build/tb_ahb.vvp | grep -q PASS || fail=1

run tb_functional   sim/tb_functional.v
run tb_oc           sim/tb_oc.v
run tb_bram_timing  sim/tb_bram_timing.v

echo "== BRAM latency bound across clock rates and sync depths =="
for ss in 1 2; do
    for tclk in 5000 6667 10000; do
        iverilog -g2012 \
            -Ptb_bram_timing.SYNC_STAGES="$ss" \
            -Ptb_bram_timing.TCLK_PS="$tclk" \
            -Ptb_bram_timing.TRIALS=150 \
            -o build/tb_sweep.vvp sim/tb_bram_timing.v hdl/*.v 2>/dev/null
        out=$(vvp build/tb_sweep.vvp)
        printf '  SYNC=%s tclk=%sps  %s  %s\n' "$ss" "$tclk" \
            "$(echo "$out" | grep -o 'latency [0-9]* ps' | head -1)" \
            "$(echo "$out" | grep -oE 'PASS|FAIL' | head -1)"
        echo "$out" | grep -q PASS || fail=1
    done
done

# The ALTA_SYN branch of fastprom_rom_bram.v (what AGM's native flow builds:
# direct alta_bram9k instances, several blocks and a read mux) against AGM's
# own primitive model. That model is not in this repo: point ALTA_SIM_V at
# packages/tool-agrv_logic/map/share/agm/rodina/alta_sim.v in the AgRV_pio SDK.
if [ -n "${ALTA_SIM_V:-}" ]; then
    echo "== tb_bram_timing on AGM's alta_bram9k model =="
    printf '`define ALTA_SYN\n`include "hdl/fastprom_rom_bram.v"\n`undef ALTA_SYN\n' \
        > build/rom_bram_alta_syn.v
    for ss in 1 2; do
        iverilog -g2012 -I. -s tb_bram_timing \
            -Ptb_bram_timing.SYNC_STAGES="$ss" -Ptb_bram_timing.TRIALS=150 \
            -Ptb_bram_timing.MODEL_SLACK_PS=1000 \
            -o build/tb_alta.vvp "$ALTA_SIM_V" build/rom_bram_alta_syn.v \
            sim/tb_bram_timing.v hdl/fastprom_top.v hdl/fastprom_enable.v \
            hdl/fastprom_rom_logic.v 2>/dev/null
        out=$(vvp build/tb_alta.vvp)
        printf '  SYNC=%s  %s  %s\n' "$ss" \
            "$(echo "$out" | grep -o 'latency [0-9]* ps' | head -1)" \
            "$(echo "$out" | grep -oE 'PASS|FAIL' | head -1)"
        echo "$out" | grep -q PASS || fail=1
    done
fi

# The Rust firmware's host tests (fw/). Where neither cargo nor podman is
# available - inside the hdlc/sim container - this accepts the result
# `make fw-test` recorded, if fw/ is unchanged since; see scripts/fw_tests.sh.
echo "== firmware host tests (Rust, fw/) =="
if ! scripts/fw_tests.sh; then fail=1; fi

# Those tests write traces of the firmware's load sequence; replay them on
# the AHB slave itself, back to back (a pipelined master) and with idle
# cycles between transfers, checking every read and the loaded image.
echo "== tb_fw_load: the firmware's load sequence on fastprom_ahb_slave =="
traces=$(ls build/fw/trace/*.trace 2>/dev/null || true)
if [ -z "$traces" ]; then
    echo "  no traces in build/fw/trace (the host tests write them)"
    fail=1
fi
for t in $traces; do
    b=$(basename "$t" .trace)
    aw=${b#*_aw}; aw=${aw%%_*}; dw=${b##*_dw}
    case $b in
        hazard*) gaps=0; check=0 ;;  # the hazard only exists back to back
        *)       gaps="0 1"; check=1 ;;
    esac
    for g in $gaps; do
        iverilog -g2012 -Ptb_fw_load.AW="$aw" -Ptb_fw_load.DW="$dw" \
            -Ptb_fw_load.GAP="$g" -Ptb_fw_load.CHECK_IMAGE="$check" \
            -Ptb_fw_load.TRACE="\"$t\"" -Ptb_fw_load.IMAGE="\"build/fw/trace/$b.hex\"" \
            -o build/tb_fw_load.vvp sim/tb_fw_load.v hdl/fastprom_ahb_slave.v
        out=$(vvp -n build/tb_fw_load.vvp)
        printf '  %-16s %s\n' "$b" "$(echo "$out" | grep -E 'PASS|FAIL' | head -1)"
        echo "$out" | grep -q ': PASS' || fail=1
    done
done

if [ "$fail" -ne 0 ]; then
    echo "SUITE: FAIL"
    exit 1
fi
echo "SUITE: PASS"
