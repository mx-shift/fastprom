#!/bin/sh
# run_matrix.sh - place and route every configuration the timing budget
# quotes, then collect the results into tools/ag32/pnr-results.json, which
# tools/timing_budget.py reads. About 1.5 minutes per configuration; JOBS
# (default 4) run at once.
#
# BRAM mode is the product. Its default build clocks the block RAM's read port
# at 200 MHz from its own PLL output (Supra's restricted Fmax is 200.08 MHz),
# sys_clock staying at 100 MHz; the "sys100"/"sys150" builds read on sys_clock
# instead, for comparison. The four-image incompressible LOGIC build (89 % of the LUTs) did not finish
# routing in an hour on 2026-11-04, so it only runs with X4=1.
#
# usage: tools/ag32/run_matrix.sh <AgRV_pio dir>
set -e
[ $# -ge 1 ] || { sed -n '2,12p' "$0"; exit 2; }
PIO=$1
REPO=$(cd "$(dirname "$0")/../.." && pwd)
cd "$REPO"
OUT=build/ag32
JOBS=${JOBS:-4}
mkdir -p $OUT/logs

# name | env | part mode [kind bank-bits]
CONFIGS="
512x8-bram||512x8 bram
256x8-bram||256x8 bram
512x8-bram-sys100|RDCLK=0|512x8 bram
256x8-bram-sys100|RDCLK=0|256x8 bram
512x8-bram-sys150|RDCLK=0 SYSCLK=150|512x8 bram
256x8-bram-sys150|RDCLK=0 SYSCLK=150|256x8 bram
512x8-logic||512x8 logic random 0
256x8-logic||256x8 logic random 0
512x8-logic-pal||512x8 logic palette 0
"
[ -z "$X4" ] || CONFIGS="$CONFIGS
512x8-logic-x4||512x8 logic random 2
"

n=0
echo "$CONFIGS" | { while IFS='|' read -r name envs args; do
  [ -n "$name" ] || continue
  # shellcheck disable=SC2086
  ( env $envs sh tools/ag32/run_flow.sh "$PIO" ${args%% *} $(echo $args | cut -d' ' -f2) \
        $OUT/$name $(echo $args | cut -s -d' ' -f3-) > $OUT/logs/$name.log 2>&1 \
    && echo "ok    $name" || echo "FAIL  $name (see $OUT/logs/$name.log)" ) &
  n=$((n + 1))
  [ $((n % JOBS)) -ne 0 ] || wait
done; wait; }

python3 tools/ag32/collect.py $OUT tools/ag32/pnr-results.json \
    "$(sed -n 's/^Cmd : [^ ]* //p' $OUT/512x8-logic/logic/alta_logs/run.log | head -1)"
