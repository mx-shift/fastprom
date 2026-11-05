#!/usr/bin/env bash
# The firmware's host tests: `cargo test` in fw/ against fixtures from
# independent producers (fw/tools/gen_fixtures.py: zlib, tools/mkimagetable.py).
# The tests also write the load traces sim/tb_fw_load.v replays on the HDL.
#
# Runs them wherever it can:
#   - cargo on PATH: here;
#   - podman on PATH: in the fastprom-rust image (`make fw-image`);
#   - neither, as inside the hdlc/sim container scripts/run_sim.sh runs in:
#     accept the result `make fw-test` recorded in build/fw/host-tests.stamp,
#     but only if nothing under fw/ nor the image tools changed since. Rust
#     cannot run in that container, so this is the honest alternative to
#     skipping the tests.
#
# Prints cargo's per-suite results and ends "fw host tests: PASS" or FAIL;
# exits non-zero on failure.
set -euo pipefail
cd "$(dirname "$0")/.."
OUT=build/fw
STAMP=$OUT/host-tests.stamp
IMAGE=${FW_IMAGE:-fastprom-rust}
mkdir -p "$OUT"

sources_hash() {
    {
        find fw -path fw/target -prune -o -path fw/.cargo-home -prune -o -type f -print \
            | LC_ALL=C sort | xargs sha256sum
        sha256sum tools/mkimage.py tools/mkimagetable.py
    } | sha256sum | cut -d' ' -f1
}

summarise() {  # cargo's output on stdin -> one line per suite, and a total
    awk '/^test result:/ { n++; p += $4; f += $6; print "  " $0 }
         END { printf "  %d suites, %d passed, %d failed\n", n, p, f }'
}

run_tests() {
    python3 fw/tools/gen_fixtures.py "$OUT/fixtures" > /dev/null
    rm -rf "$OUT/trace"
    local rc=0
    if command -v cargo > /dev/null; then
        (cd fw && cargo test --quiet) > "$OUT/host-tests.log" 2>&1 || rc=$?
    else
        podman run --rm --security-opt label=disable -v "$PWD:/w" -w /w/fw \
            -e CARGO_HOME=/w/fw/.cargo-home "$IMAGE" cargo test --quiet \
            > "$OUT/host-tests.log" 2>&1 || rc=$?
    fi
    summarise < "$OUT/host-tests.log"
    if [ "$rc" -ne 0 ]; then
        grep -E '^(error|failures:|---- |thread )' "$OUT/host-tests.log" | head -20
        echo "fw host tests: FAIL (log in $OUT/host-tests.log)"
        rm -f "$STAMP"
        return 1
    fi
    { echo "$(sources_hash)"; date -u +%Y-%m-%dT%H:%M:%SZ;
      summarise < "$OUT/host-tests.log"; } > "$STAMP"
    echo "fw host tests: PASS"
}

if command -v cargo > /dev/null || command -v podman > /dev/null; then
    run_tests
elif [ -f "$STAMP" ] && [ "$(head -1 "$STAMP")" = "$(sources_hash)" ]; then
    tail -n +3 "$STAMP"
    echo "fw host tests: PASS (recorded by make fw-test at $(sed -n 2p "$STAMP"); fw/ unchanged since)"
else
    echo "fw host tests: FAIL - no cargo or podman here, and no up-to-date result from"
    echo "  make fw-test (build/fw/host-tests.stamp missing, or fw/ changed since)."
    echo "  Run make fw-test on the host first."
    exit 1
fi
