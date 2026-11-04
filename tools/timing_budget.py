#!/usr/bin/env python3
"""Compute the socket timing budget for the as-built stack; emit docs/03-timing-budget.md.

Two sources of fabric delay, kept side by side so each can check the other:

  * a MODEL from datasheet primitives (AGRV2K Datasheet Rev 3.0 section 4) and
    the LUT depth measured by tools/synth_matrix.py (docs/synth-results.md);
  * PLACE AND ROUTE: AGM Supra's timing report for the real pin assignment on
    the QFN-32, from tools/ag32/run_matrix.sh, collected in
    tools/ag32/pnr-results.json.

Both are combined with the same external path - the stack's bus interface,
which is the part that is ours to compute: two SN74LVT245B crossings on the
lower board, the DF40 board-to-board connector, the traces of both boards and
the 33 R series terminators. Every figure is a maximum.

Things deliberately not hidden:
  * the AGRV2K primitives are shared boilerplate across AGM's CPLD line with
    no stated PVT corner, and Supra's model is AGM's too: neither is silicon
    characterisation;
  * the M9K clock-to-out comes only from Supra's model (it is unpublished).
"""
import argparse
import json
import math
import os
import re

import yaml

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
BOARD = "fastprom-upper"
PARTS = ("512x8", "256x8")

# ---------------------------------------------------------------------------
# AGRV2K fabric primitives, AGRV2K Datasheet Rev 3.0 section 4 (max values).
# ---------------------------------------------------------------------------
T_IN = 1.308      # input pad + buffer
T_DO = 2.140      # output buffer + pad
T_LUT = 1.111     # LUT4 combinational delay
T_LOCAL = 0.434   # local routing
T_ZX = 1.532      # output buffer enable (not on the stack's enable path)
T_XZ = 1.322      # output buffer disable (likewise)

ROUTE_AVG = 0.679   # T_C, column interconnect at average loading (datasheet)
ROUTE_WORST = 1.175 # RMUX worst case, from the recovered vendor mux tables

# ---------------------------------------------------------------------------
# Bus interface. Datasheet maxima at VCC = 3.3 V +/- 0.3 V over temperature,
# CL = 50 pF. LVT245B and LVTH245A have identical switching limits.
# ---------------------------------------------------------------------------
INTERFACES = {
    "lvt245": {
        "name": "SN74LVT245B",
        "tpd": 3.5,
        "ten": 5.5,    # max(tPZH, tPZL)
        "tdis": 5.9,   # max(tPHZ, tPLZ)
        "note": "I/O rated to 5.5 V, IOL 64 mA, IOH -32 mA, no bus hold",
    },
    "lvc245": {
        "name": "SN74LVC245A",
        "tpd": 6.3,
        "ten": 8.5,
        "tdis": 7.5,
        "note": "I/O 5.5 V tolerant but markedly slower than LVT",
    },
}

# ---------------------------------------------------------------------------
# The stack's wiring (docs/09-two-board-stack.md). Socket pin -> lower-board
# receive buffer -> DF40 -> upper board -> AG32, and back out through the
# drive buffer and a 33 R terminator to the socket pin.
# ---------------------------------------------------------------------------
# Longest routed socket-to-AG32 signal in hw/*.route.json on 2026-11-14 was
# 38.3 mm (socket pin 14, both boards, connector excluded); bounded here at
# 60 mm. Every signal is on an outer layer over a ground plane (microstrip,
# about 6 ps/mm on the JLC04081H-3313 stack-up); 7 ps/mm is the stripline
# figure, kept as the bound.
WIRE_MM = 60.0
PS_PER_MM = 7.0
T_DF40 = 0.05       # 3 mm mated stack; a few tens of ps, rounded up
R_TERM = 33.0       # ohms, RN1/RN2 between the drive buffer and the socket
C_HOST = 50e-12     # host load on a data line, the LVT datasheet's own CL
                    # (PROM datasheets test at 30 pF, so this is generous)

T_WIRE = WIRE_MM * PS_PER_MM / 1000.0 + T_DF40
# The terminator and the host's load form an RC after the buffer's own edge.
# Adding the full R*C*ln2 on top of a tpd already specified into 50 pF counts
# that capacitance twice, which is the safe direction.
T_TERM = R_TERM * C_HOST * math.log(2) * 1e9

# Access-time grades across the 20-pin catalogue.
GRADES = [30, 35, 45, 55, 60, 70]


def load_yaml(name):
    with open(os.path.join(ROOT, "tools", name)) as fh:
        return yaml.safe_load(fh)


def synth_depths():
    """LOGIC-mode LUT depth per (organisation, content), from synth_matrix."""
    out = {}
    with open(os.path.join(ROOT, "docs", "synth-results.md")) as fh:
        for line in fh:
            m = re.match(r"\| (\S+(?: x\d img)?) \| \d+ \| (\w+) \| (\d+) \| (\d+) \|", line)
            if m:
                out[(m[1], m[2])] = (int(m[3]), int(m[4]))
    return out


def ext_in(i):
    return i["tpd"] + T_WIRE


def ext_out(i):
    return T_WIRE + i["tpd"] + T_TERM


def model_aa(depth, route, i):
    return ext_in(i) + T_IN + T_LOCAL + depth * (T_LUT + route) + T_DO + ext_out(i)


def model_ea(route, i):
    # One LUT of enable decode, out through buf_noe (an always-driven pad, so
    # T_DO, not T_ZX), across the connector to the drive buffer's /OE.
    return ext_in(i) + T_IN + T_LOCAL + T_LUT + route + T_DO + T_WIRE + i["ten"] + T_TERM


def model_dis(route, i):
    return ext_in(i) + T_IN + T_LOCAL + T_LUT + route + T_DO + T_WIRE + i["tdis"]


def verdict(delay, limit):
    if delay <= limit * 0.8:
        return "yes"
    if delay <= limit:
        return "tight"
    return "**NO**"


def margin(delay, limit):
    return f"{limit - delay:+.1f} ns"


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--iface", default="lvt245", choices=sorted(INTERFACES))
    ap.add_argument("--fclk", type=float, default=100.0,
                    help="fabric clock in MHz for the BRAM-mode model")
    ap.add_argument("--pnr", default="tools/ag32/pnr-results.json")
    ap.add_argument("--out", default="docs/03-timing-budget.md")
    args = ap.parse_args()

    i = INTERFACES[args.iface]
    db = load_yaml("partdb.yaml")["parts"]
    board = load_yaml("boardmap.yaml")["boards"][BOARD]
    depths = synth_depths()

    pnr = None
    pnr_path = os.path.join(ROOT, args.pnr)
    stale = []
    if os.path.exists(pnr_path):
        with open(pnr_path) as fh:
            pnr = json.load(fh)
        got = pnr["boards"].get(BOARD, {})
        want = {str(k): v for k, v in board["socket_to_mcu"].items()}
        if got.get("socket_to_mcu") != want or got.get("control") != board["control"]:
            stale.append(BOARD)

    L = [
        "# Timing budget",
        "",
        "Computed by `tools/timing_budget.py` for the as-built two-board stack",
        f"(`fastprom-lower` + `{BOARD}`). Re-run it after changing any",
        "assumption rather than editing this file; re-run",
        "`tools/ag32/run_matrix.sh` (`make pnr`) after any pin or HDL change.",
        "",
        "All figures are **maxima**. There are two sources for the fabric's",
        "share, side by side:",
        "",
        "* **model**: datasheet primitives (AGRV2K Datasheet Rev 3.0 section 4,",
        "  see `docs/00-ag32-reference.md`) times the LUT depth that",
        "  `tools/synth_matrix.py` measures;",
        "* **P&R**: AGM Supra's static timing after placing and routing the real",
        "  design on the QFN-32 with the board's actual pin assignment",
        "  (`tools/ag32/run_flow.sh`, recipe in `docs/06-build-and-flash.md`).",
        "",
        "The P&R figures supersede the model wherever both exist. Neither is",
        "silicon: the primitives are AGM boilerplate with no stated PVT corner,",
        "and Supra's delay model is AGM's too. Both want confirming on hardware.",
        "",
        "## The path through the stack",
        "",
        "```",
        "socket pin -> [lower] U2/U3 LVT245 -> DF40 -> [upper] AG32 pad",
        "   ... fabric ...",
        "AG32 pad -> DF40 -> [lower] U4 LVT245 -> 33 R (RN1/RN2) -> socket pin",
        f"AG32 {board['control']['data_oe']} (buf_noe) -> DF40 -> [lower] U4 /OE (R21 pull-up)",
        "```",
        "",
        "| External term | Value | Basis |",
        "|---|---:|---|",
        f"| Receive buffer tpd ({i['name']}) | {i['tpd']:.1f} ns | datasheet max, 3.3 V +/- 0.3 V, CL 50 pF |",
        f"| Drive buffer tpd | {i['tpd']:.1f} ns | same part |",
        f"| Drive buffer enable, max(tPZH, tPZL) | {i['ten']:.1f} ns | |",
        f"| Drive buffer disable, max(tPHZ, tPLZ) | {i['tdis']:.1f} ns | |",
        f"| Wiring, each way | {T_WIRE:.2f} ns | {WIRE_MM:.0f} mm at {PS_PER_MM} ps/mm + DF40 {T_DF40} ns; longest routed signal is 38.3 mm |",
        f"| 33 R terminator into the host | {T_TERM:.2f} ns | R x {C_HOST*1e12:.0f} pF x ln 2, on top of a tpd already rated into 50 pF |",
        f"| **Socket to AG32 pad** | **{ext_in(i):.2f} ns** | |",
        f"| **AG32 pad to socket** | **{ext_out(i):.2f} ns** | |",
        "",
        f"{i['note']}. The two buffer crossings and the terminator are",
        f"{2 * i['tpd'] + T_TERM:.1f} ns of every access, and on the enable path",
        f"the drive buffer's {i['ten']} ns enable replaces its tpd.",
        "",
        "The enable path has one LUT of decode and leaves the AG32 through",
        "`buf_noe`, an ordinary always-driven pad, so the pad's own",
        "output-enable time (T_ZX) is not in it: on the stack the AG32's data",
        "pads drive unconditionally and the drive buffer alone gates the bus.",
        "",
    ]

    # ---------------------------------------------------------------- P&R
    if pnr:
        runs = pnr["runs"]
        cur = {k: v for k, v in runs.items()
               if v["board"] == BOARD and not k.startswith("whatif/")}
        L += [
            "## Place and route",
            "",
            f"Source: `{args.pnr}`, generated {pnr['generated']} with {pnr['tool']}",
            f"by {pnr['flow']}. Pin map checked against:",
            "",
        ]
        if stale:
            L += ["> **STALE:** `tools/boardmap.yaml` no longer matches the pin map these",
                  "> results were computed with. Re-run `make pnr` before trusting them.",
                  ""]
        m = pnr["boards"][BOARD]
        sm = m["socket_to_mcu"]
        L += ["| Socket pin | " + " | ".join(str(k) for k in sorted(sm, key=int)) + " | /OE |",
              "|---|" + "---|" * (len(sm) + 1),
              "| AG32 | " + " | ".join(sm[k].replace("PIN_", "") for k in sorted(sm, key=int))
              + f" | {m['control']['data_oe'].replace('PIN_', '')} |",
              ""]

        def r(name):
            return cur.get(name)

        # Per-part, per-mode summary against the part's own grades.
        L += ["### Against each part's grades",
              "",
              "Fabric delay is pad to pad as Supra reports it (worst data pin);",
              "socket delay adds the external path above. `yes` is 20 % or more",
              "margin, `tight` under that, **NO** does not fit.",
              ""]
        for part in PARTS:
            tn = db[part]["timing_ns"]
            L += [f"#### {part}", "",
                  "| Build | Parameter | Fabric (P&R) | At the socket | "
                  + " | ".join(f"{g} ({tn[g]['t_aa']}/{tn[g]['t_ea']})" for g in ("fastest", "common", "slowest"))
                  + " |",
                  "|---|---|---:|---:|---|---|---|"]
            rows = [(f"{part}-bram", "**BRAM, read port 200 MHz**"),
                    (f"{part}-bram-sys150", "BRAM, read on sys_clock 150 MHz"),
                    (f"{part}-bram-sys100", "BRAM, read on sys_clock 100 MHz"),
                    (f"{part}-logic", "LOGIC (fallback), incompressible"),
                    (f"{part}-logic-pal", "LOGIC, palette"),
                    (f"{part}-logic-x4", "LOGIC, 4 images, incompressible")]
            for name, label in rows:
                x = r(name)
                if not x:
                    continue
                aa = ext_in(i) + x["aa_worst"] + ext_out(i)
                ea = ext_in(i) + x["ea"] + T_WIRE + i["ten"] + T_TERM
                er = ext_in(i) + x["ea"] + T_WIRE + i["tdis"]
                L.append(f"| {label} | tAA | {x['aa_worst']:.1f} ns | {aa:.1f} ns | "
                         + " | ".join(f"{verdict(aa, tn[g]['t_aa'])} {margin(aa, tn[g]['t_aa'])}"
                                      for g in ("fastest", "common", "slowest")) + " |")
                L.append(f"| | tEA | {x['ea']:.1f} ns | {ea:.1f} ns | "
                         + " | ".join(f"{verdict(ea, tn[g]['t_ea'])} {margin(ea, tn[g]['t_ea'])}"
                                      for g in ("fastest", "common", "slowest")) + " |")
                L.append(f"| | tER | {x['ea']:.1f} ns | {er:.1f} ns | "
                         + " | ".join(f"{verdict(er, tn[g]['t_er'])} {margin(er, tn[g]['t_er'])}"
                                      for g in ("fastest", "common", "slowest")) + " |")
            L.append("")

        if not r("512x8-logic-x4"):
            L += ["**Not placed and routed: a four-image incompressible 512x8 LOGIC",
                  "build.** It is 1883 of 2112 LUTs (89 %), and on 2026-11-04 Supra had",
                  "not converged after an hour of routing (about 800 overflowing nodes",
                  "throughout), with a post-placement estimate of 20.1 ns of fabric",
                  "address delay - about 29 ns at the socket before routing congestion.",
                  "Treat multi-image incompressible LOGIC builds as unverified; real",
                  "multi-image contents are usually structured and far smaller, and",
                  "the stack's image switch is read by firmware, which has no path",
                  "into the fabric's bank bits (`fastprom_ahb_slave` has no bank",
                  "register). `X4=1 make pnr` retries it.",
                  ""]

        # Per-pin view of the address path.
        x = r("512x8-logic")
        if x:
            inv = {v: k for k, v in sm.items()}
            L += ["### Per data pin, 512x8 LOGIC incompressible",
                  "",
                  "| Data bit | AG32 pin | Socket pin | Fabric tAA |",
                  "|---|---|---:|---:|"]
            for d in sorted(x["aa"], key=lambda k: int(k.split("[")[1][:-1])):
                pin = x["pins"][d]
                L.append(f"| {d} | {pin} | {inv.get(pin, '-')} | {x['aa'][d]:.2f} ns |")
            spread = max(x["aa"].values()) - min(x["aa"].values())
            slow = sorted(x["aa"], key=lambda k: -x["aa"][k])[:2]
            L += ["", f"Spread across the eight outputs: {spread:.2f} ns. The two slowest,",
                  f"{slow[0]} and {slow[1]}, are on {x['pins'][slow[0]]} and {x['pins'][slow[1]]};"
                  " the rest of the spread is",
                  "the logic cone each bit happens to get. See the pad positions below.",
                  ""]

        # Where the pads sit on the die.
        x = cur.get("512x8-logic")
        if x and x.get("pad_xy"):
            L += ["### Does the pin assignment matter?",
                  "",
                  "On the enable path, yes. Supra's routed netlist gives each pad's tile",
                  "on the die (23 x 14 tiles). The QFN-32 bonds its pads to tiles on",
                  "three sides, and PIN_18 and PIN_19 (with PIN_26 near them) sit at the",
                  "opposite edge from the rest:",
                  "",
                  "| Signal | AG32 pin | Die tile (x, y) |",
                  "|---|---|---|"]
            order = sorted(x["pad_xy"], key=lambda p: (x["pad_xy"][p][0], x["pad_xy"][p][1]))
            for port in order:
                L.append(f"| {port} | {x['pins'][port]} | {x['pad_xy'][port][0]}, {x['pad_xy'][port][1]} |")
            ce = x["pad_xy"].get("ce_pin") or x["pad_xy"].get("ce_pin[0]")
            oe = x["pad_xy"]["buf_noe"]
            L += ["",
                  f"The chip enable comes in at tile {ce[0]}, {ce[1]} and `buf_noe` leaves at",
                  f"{oe[0]}, {oe[1]}. The one-LUT enable path is mostly the two pads and the",
                  "route into the output pad, so its length is set by that distance.",
                  "With `buf_noe` on PIN_18, across the die, the fabric enable is",
                  "9.5-9.9 ns and tEA misses 20 ns by 0.6-0.9 ns; on PIN_14 runs on",
                  "2026-11-04 measured 7.1-7.9 ns (three seeds, three designs). Socket 8",
                  "is on PIN_15 and socket 7 on PIN_18 to make room; the data bit on",
                  "PIN_18 is up to about 1.5 ns slower than it would be on PIN_15, which",
                  "tAA absorbs.",
                  ""]

        # Clocked paths and resources.
        eas = [v["ea"] for v in cur.values()]
        L += ["### Run-to-run variation",
              "",
              f"The enable path is the same logic in every build, so its spread across",
              f"the {len(eas)} as-built runs is placement noise: {min(eas):.2f} to",
              f"{max(eas):.2f} ns of fabric. Margins under about",
              f"{max(eas) - min(eas):.1f} ns are within it; a real build should be",
              "checked from its own report (`timing.json`), not from this table.",
              ""]
        L += ["### Clocked paths and fit",
              "",
              "| Build | LUTs | M9K | sys_clock (Fmax) | BRAM read clock (Fmax) | Worst clocked slack |",
              "|---|---:|---:|---:|---:|---:|"]
        for name in sorted(cur):
            x = cur[name]
            u = x["utilisation"]
            rd = (f"{x['rd_clock_mhz']:.0f} MHz ({x['rd_fmax_mhz']:.0f})" if x.get("rd_clock_mhz")
                  else ("sys_clock" if x["mode"] == "bram" else "-"))
            L.append(f"| {name} | {u['luts'][0]}/{u['luts'][1]} | {u['brams'][0]}/{u['brams'][1]} | "
                     f"{x['sys_clock_mhz']:.0f} MHz ({x['fmax_mhz']:.0f}) | {rd} | {x['worst_clocked_slack']:.2f} ns |")
        L += ["",
              "The clocked paths are the AHB slave (and, in BRAM mode, the load",
              "port and the M9K): all meet their clocks with margin. Fmax is",
              "Supra's path-based figure; its restricted Fmax, the clock-network",
              "limit, is 200.08 MHz throughout.",
              ""]

        b = cur.get("512x8-bram")
        if b:
            launch = b["aa_worst"] - b["period"] - b["bram_capture"]
            L += ["### BRAM mode, decomposed",
                  "",
                  "```",
                  "tAA(fabric) = T + capture + launch",
                  "  T        sampling uncertainty, one sys_clock period",
                  "           (measured in sim/tb_bram_timing.v)",
                  "  capture  pad -> M9K address register, setup, less clock latency",
                  "  launch   clock latency + M9K clock-to-out + route + output pad",
                  "```",
                  "",
                  f"From the {1000 / b['period']:.0f} MHz read-clock run: capture {b['bram_capture']:.2f} ns, launch",
                  f"{launch:.2f} ns. Item by item, from Supra's worst paths:",
                  "",
                  "| Stage | Delay | Kind |",
                  "|---|---:|---|",
                  "| Sampling: wait for the next read-clock edge | T = 1 period | quantised (SYNC_STAGES = 1) |",
                  "| Input pad | 1.31 ns | combinational |",
                  "| Route, pad to the M9K's address pins | 2.48 ns | combinational |",
                  "| M9K address-register setup | -0.09 ns | combinational |",
                  "| Clock insertion, launch less capture | 0.05 ns | cancels except this |",
                  "| M9K clock to DataOut (unregistered output) | 3.24 ns | combinational |",
                  "| Route, M9K to the data pad | 2.9-3.1 ns | combinational |",
                  "| Output pad | 3.48 ns | combinational |",
                  "",
                  "There is no fabric synchroniser: the address pads feed the M9K's own",
                  "address register, the only sampling stage, and there is no output",
                  "mux for a single-block image. So the only quantised term is one",
                  "period, which `sim/tb_bram_timing.v` measures (its behavioural RAM has",
                  f"tRAM = 0); everything else, about {b['bram_capture'] + launch:.1f} ns, is fixed.",
                  "",
                  "The read port takes its own PLL output (`PLLCLK2` in the `.ve` that",
                  "`tools/gen_pins.py` writes; `RDCLK` overrides it in",
                  "`tools/ag32/run_flow.sh`), so it can run faster than sys_clock, which",
                  "carries the CPU's AHB. Supra reports a restricted Fmax of 200.08 MHz",
                  "for any clock in this design (it times 250 and 300 MHz read clocks at",
                  "path level but flags them over that limit), so 200 MHz - 400 MHz",
                  "VCO / 2 from the 12 MHz HSE - is the ceiling.",
                  ""]

    # --------------------------------------------------------------- model
    L += ["## Model from datasheet primitives",
          "",
          "```",
          "tAA = ext_in + T_IN + T_LOCAL + N x (T_LUT + route) + T_DO + ext_out",
          "tEA = ext_in + T_IN + T_LOCAL + T_LUT + route + T_DO + wire + tEN + tTERM",
          "```",
          "",
          "Routing per level two ways, because the datasheet quotes interconnect",
          "at \"average loading\" while the recovered vendor mux tables show a",
          f"worst case nearly twice as large: {ROUTE_AVG} ns and {ROUTE_WORST} ns.",
          "N is the depth `tools/synth_matrix.py` measures (docs/synth-results.md).",
          "",
          "| Case | LUT depth | tAA avg routing | tAA worst routing |",
          "|---|---:|---:|---:|"]
    cases = [("structured contents, any organisation", ("512x8", "palette")),
             ("256x8 incompressible", ("256x8", "random")),
             ("512x8 incompressible", ("512x8", "random")),
             ("512x8 x2 images, incompressible", ("512x8 x2 img", "random")),
             ("512x8 x4 images, incompressible", ("512x8 x4 img", "random")),
             ("2048x4 incompressible", ("2048x4", "random")),
             ("4096x4 incompressible", ("4096x4", "random"))]
    for label, key in cases:
        _luts, dep = depths[key]
        L.append(f"| {label} | {dep} | {model_aa(dep, ROUTE_AVG, i):.1f} ns | "
                 f"{model_aa(dep, ROUTE_WORST, i):.1f} ns |")
    L += ["",
          "| Parameter | avg routing | worst routing |",
          "|---|---:|---:|",
          f"| tEA, enable to outputs driving | {model_ea(ROUTE_AVG, i):.1f} ns | {model_ea(ROUTE_WORST, i):.1f} ns |",
          f"| tER, enable to outputs released | {model_dis(ROUTE_AVG, i):.1f} ns | {model_dis(ROUTE_WORST, i):.1f} ns |",
          ""]
    if pnr and "512x8-logic" in pnr["runs"]:
        x = pnr["runs"]["512x8-logic"]
        _l, dep = depths[("512x8", "random")]
        fab_model = model_aa(dep, ROUTE_WORST, i) - ext_in(i) - ext_out(i)
        L += ["**Where the model is wrong.** Against Supra for 512x8 incompressible,",
              f"the worst-routing model gives {fab_model:.1f} ns of fabric against",
              f"{x['aa_worst']:.1f} ns placed and routed - close, but for opposite",
              "reasons: LUT levels are cheaper than the model (0.1-0.6 ns",
              "in Supra against 1.111 ns T_LUT), and pads are dearer. The output pad",
              "is 3.48 ns against T_DO 2.14 ns, and the route from the last LUT into",
              "any output pad is 3.1-3.4 ns. On the one-LUT enable path that fixed",
              f"pad overhead is almost everything: {x['ea']:.1f} ns placed and routed",
              f"against {model_ea(ROUTE_WORST, i) - ext_in(i) - T_WIRE - i['ten'] - T_TERM:.1f} ns modelled.",
              ""]

    L += ["### BRAM mode, model",
          "",
          "```",
          "tAA = ext_in + SYNC_STAGES x tCLK + capture + launch + ext_out",
          "```",
          "",
          "The sampling term is measured, not estimated: `sim/tb_bram_timing.v`",
          "moves the address at phases unrelated to the clock and finds the worst",
          "case is one full clock period per sync stage, within a few",
          "picoseconds, across every rate and depth tested.",
          ""]

    # --------------------------------------------------------- conclusions
    if pnr:
        def sock(x):
            return (ext_in(i) + x["aa_worst"] + ext_out(i),
                    ext_in(i) + x["ea"] + T_WIRE + i["ten"] + T_TERM,
                    ext_in(i) + x["ea"] + T_WIRE + i["tdis"])

        def fits(v, lim):
            return f"meets it by {lim - v:.1f} ns" if v <= lim else f"**misses it by {v - lim:.1f} ns**"

        runs = pnr["runs"]
        L += ["## Conclusions", "",
              "Socket figures from the P&R fabric delays plus the external path;",
              "each against the part's fastest characterised grade.", ""]
        n = 1
        for part in PARTS:
            tn = db[part]["timing_ns"]["fastest"]
            br, b100, lg = (runs.get(f"{part}-bram"), runs.get(f"{part}-bram-sys100"),
                            runs.get(f"{part}-logic"))
            if br:
                a, e, d = sock(br)
                L += [f"{n}. **{part}, BRAM.** tAA {a:.1f} ns against {tn['t_aa']} ns:",
                      f"   {fits(a, tn['t_aa'])}. tEA {e:.1f} ns against {tn['t_ea']} ns: {fits(e, tn['t_ea'])}.",
                      f"   tER {d:.1f} ns against {tn['t_er']} ns: {fits(d, tn['t_er'])}."
                      + (f" (Read on sys_clock at 100 MHz it would be {sock(b100)[0]:.1f} ns.)" if b100 else "")]
                n += 1
            if lg:
                a, e, d = sock(lg)
                L += [f"{n}. **{part}, LOGIC (fallback, incompressible).** tAA {a:.1f} ns:",
                      f"   {fits(a, tn['t_aa'])}; tEA {e:.1f} ns: {fits(e, tn['t_ea'])}; tER {d:.1f} ns:"
                      f" {fits(d, tn['t_er'])}."]
                n += 1
        b = runs.get("512x8-bram")
        if b:
            L += [f"{n}. **What limits BRAM mode.** One read-clock period (5 ns at",
                  "   200 MHz, the most Supra allows) plus about "
                  f"{b['aa_worst'] - b['period']:.1f} ns of fixed fabric delay - pads,",
                  "   routing and the M9K's 3.24 ns access, which no pad setting or",
                  "   placement constraint changed - plus the 9.1 ns of bus interface.",
                  "   Measured on 2026-11-04: the M9K sits at the site nearest the",
                  "   pads (X13 Y4); a BRAM-optimised pin map gains about 0.2 ns; pad",
                  "   standard, drive and slew settings do not change Supra's 3.48 ns",
                  "   output pad; the RAM's output register would add a period. Backing",
                  "   off the clock limit costs little: 192 MHz (VCO 384 MHz, sys_clock",
                  "   96 MHz) is 0.2 ns slower. The 256x8's 28 ns grade is the thin one;",
                  f"   against the PROMs' own 30 pF test load the terminator term is",
                  f"   {R_TERM * 30e-12 * math.log(2) * 1e9:.2f} ns rather than {T_TERM:.2f} ns, which is",
                  "   worth about 0.45 ns but is not assumed here. LOGIC mode is the",
                  "   fallback where it matters.",
                  ]
            n += 1
        lg = runs.get("512x8-logic")
        if lg:
            e = sock(lg)[1]
            L += [f"{n}. **The enable path is the tightest.** One LUT of decode",
                  f"   costs {lg['ea']:.1f} ns of fabric, almost all of it two pads and the route",
                  f"   into the output pad, and the stack adds {ext_in(i):.1f} ns in and",
                  f"   {T_WIRE + i['ten'] + T_TERM:.1f} ns out (the drive buffer's {i['ten']} ns enable is the",
                  f"   largest single item). tEA {e:.1f} ns is {e / sock(lg)[0]:.2f} of tAA, not the 1/2-1/3",
                  "   the period parts offer; a host relying on that ratio rather than on",
                  "   the datasheet limit is at risk.",
                  ]
            n += 1
            L += [f"{n}. **Pins and clocks.** The enable leaves on {lg['pins']['buf_noe']},",
                  "   next to the chip enable; the address path is insensitive to the pin",
                  "   map; nothing needs a global-clock pin (LOGIC mode has no clock,",
                  "   BRAM mode clocks from the PLL fed by the HSE on PIN_2).",
                  ]
            n += 1
        L += [""]

    L += ["## Unresolved risk: the configuration window",
          "",
          "The fabric is SRAM-based and configured from flash at power-on, and",
          "AGM publishes no configuration time. Until the fabric is live the",
          "socket is dead: a host that starts fetching before configuration",
          "completes reads nothing meaningful. The design fails safe (R21 holds",
          "the drive buffer disabled), but the time needs measuring against the",
          "host's own reset delay."]

    with open(os.path.join(ROOT, args.out), "w") as fh:
        fh.write("\n".join(L) + "\n")
    print(f"wrote {args.out}")
    print(f"  external: in {ext_in(i):.2f} ns, out {ext_out(i):.2f} ns, terminator {T_TERM:.2f} ns")
    if stale:
        print(f"  WARNING: {args.pnr} is stale for {', '.join(stale)}")
    if pnr:
        for name, x in sorted(pnr["runs"].items()):
            aa = ext_in(i) + x["aa_worst"] + ext_out(i)
            ea = ext_in(i) + x["ea"] + T_WIRE + i["ten"] + T_TERM
            print(f"  {name:<18} fabric aa {x['aa_worst']:5.1f} ea {x['ea']:4.1f} -> socket tAA {aa:5.1f} tEA {ea:5.1f}")


if __name__ == "__main__":
    main()
