#!/usr/bin/env python3
"""Summarise a Supra run of tools/ag32/run_flow.sh into the socket-path delays.

Reads <work>/logic/alta_db/setup.rpt.gz (Supra's worst path per endpoint,
verbose), <work>/logic/alta_db/io.asf (where Supra actually put each port) and
<work>/top.ve (where tools/gen_pins.py asked for them), and writes
<work>/timing.json:

  pins       port -> package pin, as placed (checked against the .ve)
  aa         LOGIC: worst a_pin -> d_pin pad-to-pad delay, per data pin
             BRAM:  per data pin, 2T - slack(a_pin -> M9K) + arrival(M9K -> d_pin)
             (see tools/ag32/fastprom_bram.sdc for why that is the fabric tAA)
  ea         worst ce_pin -> buf_noe pad-to-pad delay
  fmax, worst_clocked_slack, utilisation

It refuses to report (exit 1) if any port is not where the .ve put it, or if a
socket path is missing from the report: an unconstrained path is not a pass.
"""
import gzip
import json
import os
import re
import sys

HDR = re.compile(r"^Setup from (\S+) to (\S+), clock (\S+), constraint ([-\d.]+), "
                 r"skew ([-\d.]+), data ([-\d.]+)")


def paths(rpt):
    out = []
    cur = None
    with gzip.open(rpt, "rt") as fh:
        for line in fh:
            m = HDR.match(line)
            if m:
                cur = dict(src=m[1], dst=m[2], clock=m[3], constraint=float(m[4]),
                           skew=float(m[5]), data=float(m[6]))
                out.append(cur)
                continue
            if cur is not None:
                s = line.strip()
                # The reference the launch side is timed from: the PLL output
                # edge (input delays and register launches both hang off it).
                if "launch_ref" not in cur and (s.endswith("Input Delay") or
                                                re.search(r"=> +pll_inst\|clkout\d( D)?$", s)):
                    cur["launch_ref"] = float(s.split()[0])
                if s.startswith("Slack:"):
                    cur["slack"] = float(s.split()[1])
                elif s.startswith("Arrival Time:"):
                    cur["arrival"] = float(s.split()[2])
                elif s.startswith("Required Time:"):
                    cur["required"] = float(s.split()[2])
    return out


def base(port):
    return port.split("[")[0]


def main():
    work, part, mode, kind, bankw = sys.argv[1:6]
    board = sys.argv[6] if len(sys.argv) > 6 else "fastprom-upper"
    logic = os.path.join(work, "logic")
    db = os.path.join(logic, "alta_db")

    want = {}
    with open(os.path.join(work, "top.ve")) as fh:
        for line in fh:
            line = line.split("#")[0].split()
            if len(line) == 2 and ":" in line[1] and line[1].startswith("PIN_"):
                want[line[0]] = line[1].split(":")[0]
    placed = {}
    with open(os.path.join(db, "io.asf")) as fh:
        for line in fh:
            m = re.match(r"set_location_assignment -to (\S+) (PIN_\d+)$", line.strip())
            if m:
                placed[m[1]] = m[2]
    bad = [f"{p}: asked {w}, placed {placed.get(p)}" for p, w in want.items()
           if placed.get(p) != w]

    console = open(os.path.join(logic, "supra_console.log"), errors="replace").read()
    for w in re.findall(r"Warn: IO (\S+) is not assigned, placed at pin (\S+)", console):
        bad.append(f"{w[0]}: not assigned in the .ve, Supra chose {w[1]}")
    if bad:
        print("pin placement does not match the constraints:", file=sys.stderr)
        for b in bad:
            print("  " + b, file=sys.stderr)
        sys.exit(1)

    # Where on the die each pad cell landed (tile x, y), from the routed netlist.
    coords = {}
    routed = os.path.join(logic, "top_routed.v")
    if os.path.exists(routed):
        for m in re.finditer(r"defparam (\w+)\.coord_([xy]) = (\d+);", open(routed).read()):
            coords.setdefault(m[1], {})[m[2]] = int(m[3])
    pad_xy = {}
    for port in placed:
        b, _, idx = port.partition("[")
        for cell in (f"{b}_o_{idx[:-1]}", f"{b}_i_{idx[:-1]}", f"{b}_o", f"{b}_i"):
            if cell in coords and "x" in coords[cell]:
                pad_xy[port] = [coords[cell]["x"], coords[cell]["y"]]
                break

    ps = paths(os.path.join(db, "setup.rpt.gz"))
    res = dict(part=part, mode=mode, kind=kind, bank_bits=int(bankw), board=board,
               pins=placed, pad_xy=pad_xy)

    def util(name):
        m = re.findall(rf"Total +{name} *: *(\d+)/ *(\d+)", console)
        return [int(m[-1][0]), int(m[-1][1])] if m else None
    res["utilisation"] = dict(luts=util("LUTs"), registers=util("Registers"),
                              brams=util("Block Rams"))
    fm = re.findall(r"User constraint: *([\d.]+)MHz, Fmax: *([\d.]+)MHz, Clock: pll_inst\|clkout0",
                    console)
    rk = re.findall(r"User constraint: *([\d.]+)MHz, Fmax: *([\d.]+)MHz, Clock: pll_inst\|clkout1",
                    console)
    if rk:
        res["rd_clock_mhz"], res["rd_fmax_mhz"] = float(rk[-1][0]), float(rk[-1][1])
    res["sys_clock_mhz"], res["fmax_mhz"] = (float(fm[-1][0]), float(fm[-1][1])) if fm else (None, None)
    clocked = [p for p in ps if p["clock"] != "n/a" and "slack" in p]
    res["worst_clocked_slack"] = min(p["slack"] for p in clocked) if clocked else None

    ea = [p for p in ps if base(p["src"]) == "ce_pin" and p["dst"] == "buf_noe"]
    if not ea:
        sys.exit("no ce_pin -> buf_noe path in the report")
    res["ea"] = dict(delay=max(p["data"] for p in ea),
                     path=max(ea, key=lambda p: p["data"]))

    aa = {}
    if mode == "logic":
        for p in ps:
            if base(p["src"]) == "a_pin" and base(p["dst"]) == "d_pin":
                aa[p["dst"]] = dict(delay=p["data"], src=p["src"])
    else:
        # Worst capture leg over all address bits (one shared register clock),
        # and per data pin the launch leg out of the block RAM.
        cap = [p for p in ps if base(p["src"]) == "a_pin" and p["clock"] != "n/a"]
        if not cap:
            sys.exit("no a_pin -> register path in the report")
        # An address change at the pad is captured by the first edge it
        # meets with setup to spare; the worst case just misses one and waits
        # a whole period T. Measured from the PLL output edge R:
        #   capture: (pad -> M9K pin) + setup - clock latency = T - slack_in
        #   launch:  clock latency + M9K clock-to-out + route + pad = arrival - R
        #   tAA(fabric) = T + (T - slack_in) + (arrival_out - R_out)
        worst_cap = min(cap, key=lambda p: p["slack"])
        period = worst_cap["constraint"]
        res["bram_capture"] = worst_cap
        for p in ps:
            if base(p["dst"]) == "d_pin" and p["clock"] != "n/a":
                launch = p["arrival"] - p["launch_ref"]
                aa[p["dst"]] = dict(
                    delay=period + (period - worst_cap["slack"]) + launch,
                    src=p["src"], launch=launch,
                    capture=period - worst_cap["slack"])
        res["period"] = period
        # The launch leg's own slack is relaxed by a multicycle constraint and
        # says nothing; leave it out of the clocked summary.
        clocked = [p for p in clocked if base(p["dst"]) != "d_pin"]
        res["worst_clocked_slack"] = min(p["slack"] for p in clocked) if clocked else None
    if len(aa) != 8:
        sys.exit(f"found address->data paths to {len(aa)} of 8 data pins")
    res["aa"] = aa
    res["aa_worst"] = max(v["delay"] for v in aa.values())

    with open(os.path.join(work, "timing.json"), "w") as fh:
        json.dump(res, fh, indent=1, sort_keys=True)

    print(f"{part} {mode} ({kind}, {bankw} bank bits): "
          f"LUTs {res['utilisation']['luts']}, BRAM {res['utilisation']['brams']}")
    for d in sorted(aa, key=lambda k: int(k.split('[')[1][:-1])):
        print(f"  {d:<9} {placed[d]:<7} address->data {aa[d]['delay']:6.3f} ns  (from {aa[d]['src']})")
    print(f"  enable -> buf_noe ({placed['buf_noe']}) {res['ea']['delay']:.3f} ns")
    print(f"  sys_clock {res['sys_clock_mhz']} MHz, Fmax {res['fmax_mhz']} MHz, "
          f"worst clocked slack {res['worst_clocked_slack']} ns")
    print(f"  wrote {os.path.join(work, 'timing.json')}")


if __name__ == "__main__":
    main()
