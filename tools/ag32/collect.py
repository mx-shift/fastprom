#!/usr/bin/env python3
"""Collect build/ag32/*/timing.json into one tracked results file.

usage: collect.py <build/ag32> <out.json> [Supra version string]

The output records, with the results, exactly what they were computed
against: each board's socket_to_mcu and control map from tools/boardmap.yaml
at collection time, and the Supra version. tools/timing_budget.py refuses to
use results whose pin map no longer matches boardmap.yaml.

A run for a board no longer in boardmap.yaml (the r3 single board's
comparison runs, before it was dropped) keeps its timing but records no pin
map: there is nothing current to check it against.
"""
import datetime
import glob
import json
import os
import sys

import yaml

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))


def main():
    build, out = sys.argv[1], sys.argv[2]
    version = sys.argv[3] if len(sys.argv) > 3 else ""
    runs = {}
    for path in sorted(glob.glob(os.path.join(build, "*", "timing.json")) +
                       glob.glob(os.path.join(build, "whatif", "*", "timing.json"))):
        name = os.path.relpath(os.path.dirname(path), build)
        with open(path) as fh:
            full = json.load(fh)
        runs[name] = dict(
            board=full["board"], part=full["part"], mode=full["mode"],
            kind=full["kind"], bank_bits=full["bank_bits"], pins=full["pins"],
            pad_xy=full.get("pad_xy", {}),
            aa={k: round(v["delay"], 3) for k, v in full["aa"].items()},
            aa_worst=round(full["aa_worst"], 3),
            ea=round(full["ea"]["delay"], 3),
            sys_clock_mhz=full["sys_clock_mhz"], fmax_mhz=full["fmax_mhz"],
            rd_clock_mhz=full.get("rd_clock_mhz"), rd_fmax_mhz=full.get("rd_fmax_mhz"),
            worst_clocked_slack=full["worst_clocked_slack"],
            utilisation=full["utilisation"],
            period=full.get("period"),
            bram_capture=round(full["period"] - full["bram_capture"]["slack"], 3)
            if full.get("bram_capture") else None,
        )
    with open(os.path.join(ROOT, "tools", "boardmap.yaml")) as fh:
        bm = yaml.safe_load(fh)["boards"]
    boards = {}
    for name, r in runs.items():
        if name.startswith("whatif/") or r["board"] not in bm:
            continue
        b = bm[r["board"]]
        boards[r["board"]] = dict(socket_to_mcu=b["socket_to_mcu"], control=b["control"])
    res = dict(
        generated=datetime.date.today().isoformat(),
        tool=f"AGM Supra {version}".strip(),
        flow="tools/ag32/run_matrix.sh (AgRV_pio native flow, device AGRV2KQ32)",
        boards=boards, runs=runs)
    with open(out, "w") as fh:
        json.dump(res, fh, indent=1, sort_keys=True)
        fh.write("\n")
    print(f"collected {len(runs)} runs into {out}")


if __name__ == "__main__":
    main()
