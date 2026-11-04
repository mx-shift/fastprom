#!/bin/sh
# Autoroute one board with Freerouting, headless, in a Java container.
#
#   scripts/freerouting.sh <board.dsn> <board.ses> [plane nets, comma separated]
#
# The DSN comes from `tools/gen_layout.py --dsn`, which also rewrites its
# layer use (see freerouting_layers there). The jar is fetched once into
# ~/.cache/fastprom and checked against its release hash.
#
# Settings, and why each is set:
#   router.plane_nets               the nets with an inner plane, if any:
#                                   plane-routing mode and cheap vias for them
#                                   (the stack boards pass none: both their
#                                   planes are ground, which Freerouting
#                                   does not route - see gen_layout.py --dsn)
#   router.copper_to_edge_clearance_um=300
#                                   the board's edge rule (JLC routed edge)
#   router.fanout.*_via_diameter_mm=0.45
#                                   fanout vias default to 0.25 mm; ours are
#                                   0.45/0.2, JLC's no-surcharge size
# Automatic neckdown is left on: a 0.3 mm power track cannot reach a small
# regulator's 0.25 mm pads without it, and 2.5.0's fanout necks pad exits
# whatever the setting says (issue #973, fixed after 2.5.0). gen_layout.py
# --ses puts every necked track back to its class width wherever DRC allows.
#
# Pass criterion: nothing unrouted, and no violation Freerouting did not
# already report at load. Ground is not routed here: gen_layout.py --dsn
# hands it over as fixed copper with no pins (2.5.0 has no via-in-pad
# support - router.autorouter.via_in_pad arrived after it - and cannot see a
# ground connection made through an exposed-pad via). So at load it reports
# every place where that fixed ground copper meets what are now netless
# ground pads - the stubs, the exposed-pad vias, and pads that share a
# position (USB-C's paired ground pins) - as pre-existing, unfixable
# violations. They are intended, and KiCad's DRC checks them for real. A
# potentially fixable one at load, or any count above them at the end, is a
# failure.
# Not set: router.hole_clearance_um. In 2.5.0 it turns every DF40 contact
# pair of different net classes into a pin-to-pin violation at load, and the
# 0.127 mm via clearance already keeps a 0.2 mm hole 0.25 mm from copper.
#
# Corner quality: what 2.5.0 can do about it, and why none of it is set.
# (Its own sources at the v2.5.0 tag: DefaultSettings, RouterSettings,
# OptimizerSettings, MazeSearchEngine, Structure; the settings are applied
# by name, see --logging.console.level=DEBUG.)
#   trace angle    45 degrees already. The DSN's structure has no snap_angle
#                  (KiCad does not write one) and 2.5.0's default is
#                  FORTYFIVE_DEGREE. Its 45-degree mode is octilinear, not
#                  "45-degree corners": a trace may turn through 90 or 135
#                  degrees at a corner as freely as through 45, and it does.
#                  There is no setting that forbids a turn sharper than 45.
#   router.scoring.default_bend_cost (default 0)
#                  the maze search's cost per bend, clamped to 9.9 - in
#                  board units, which for these DSNs are 25 nm. Set to the
#                  maximum it changes nothing (the session came out byte
#                  for byte the same at 0, 2 and 5): a bend is worth a
#                  quarter of a micron of track.
#   router.optimizer.* (enabled, 100 passes, 2.5 % threshold, at most 12
#                  failures in pass 1)
#                  on both boards pass 1 stops after its 12 failures and is
#                  rejected (score not improved), so the optimizer leaves the
#                  routing as the autorouter made it. With the canary and
#                  the threshold opened up (max_consecutive_failures_pass1
#                  1000, improvement_threshold 0) every pass is rejected for
#                  breaking a connection, at 50 s a board.
#   router.trace_pull_tight_accuracy (default 500, 12.5 um)
#                  10 to 200 removes some bends on the lower board (262 ->
#                  243) by routing it differently, with 8 % more track, and
#                  barely touches the upper (273 -> 271); with four runs
#                  sharing the machine, every value from 1 to 200 left the
#                  upper board one connection unrouted (2.5.0's fanout has a
#                  per-pin time limit). Not worth a different routing.
# So the corners are cleaned up after routing instead, deterministically and
# checked by KiCad's DRC: route.py smooth(), run by gen_layout.py --ses (see
# DESIGN_NOTES.md, "Routing quality").
set -e
VER=2.5.0
SHA=f6f51bb02245e8e717f9359bd260cc9c5c0b1bc0acc8b7cb2cd5b8ffeb5de3c7
JRE=${FREEROUTING_JRE:-docker.io/library/eclipse-temurin:25-jre}
ENGINE=${KICAD_ENGINE:-podman}

dsn=$1; ses=$2; planes=$3
[ -n "$dsn" ] && [ -n "$ses" ] || { echo "usage: $0 board.dsn board.ses [plane,nets]" >&2; exit 2; }

cache=${XDG_CACHE_HOME:-$HOME/.cache}/fastprom
jar=$cache/freerouting-$VER.jar
if [ ! -f "$jar" ]; then
  mkdir -p "$cache"
  curl -sSLo "$jar.tmp" \
    "https://github.com/freerouting/freerouting/releases/download/v$VER/freerouting-$VER.jar"
  echo "$SHA  $jar.tmp" | sha256sum -c --quiet
  mv "$jar.tmp" "$jar"
fi

dir=$(cd "$(dirname "$dsn")" && pwd)
log=$dir/$(basename "$dsn" .dsn).freerouting.log
if [ "$ENGINE" = podman ]; then userns=--userns=keep-id; else userns="--user $(id -u):$(id -g)"; fi
"$ENGINE" run --rm $userns -v "$cache:/fr:z" -v "$dir:/w:z" -w /w -e HOME=/tmp "$JRE" \
  java -jar "/fr/freerouting-$VER.jar" --gui.enabled=false -da \
    -de "$(basename "$dsn")" -do "$(basename "$ses")" \
    ${planes:+"--router.plane_nets=$planes"} \
    --router.copper_to_edge_clearance_um=300 \
    --router.fanout.start_via_diameter_mm=0.45 \
    --router.fanout.end_via_diameter_mm=0.45 \
    > "$log" 2>&1

# Freerouting exits 0 whatever it manages; its summary line is the verdict.
summary=$(grep "Auto-routing stage completed" "$log" | sed 's/.*final score: //')
echo "$(basename "$dsn" .dsn): $summary"
pre=$(sed -n 's/.*Board has \([0-9]*\) pre-existing clearance violation.*/\1/p' "$log" | head -1)
unfixable=$(sed -n 's/.* - \([0-9]*\) unfixable violation.*/\1/p' "$log" | head -1)
fixable=$(sed -n 's/.* - \([0-9]*\) potentially fixable violation.*/\1/p' "$log" | head -1)
unrouted=$(echo "$summary" | sed -n 's/.*(\([0-9]*\) unrouted.*/\1/p')
viol=$(echo "$summary" | sed -n 's/.* and \([0-9]*\) violation.*/\1/p')
pre=${pre:-0}; unfixable=${unfixable:-0}; fixable=${fixable:-0}
if [ "$pre" != "$unfixable" ] || [ "$fixable" != 0 ]; then
  echo "freerouting: $pre pre-existing violations, $fixable of them fixable - see $log" >&2
  exit 1
fi
if [ "$unrouted" != 0 ] || [ "$viol" -gt "$pre" ]; then
  grep -A40 "could not be routed" "$log" | grep "^  Net\|^    -" >&2 || true
  echo "freerouting left work undone ($unrouted unrouted, $viol violations against $pre pre-existing), see $log" >&2
  exit 1
fi
[ "$pre" = 0 ] || echo "  ($pre pre-existing: fixed ground copper against its netless pads)"
