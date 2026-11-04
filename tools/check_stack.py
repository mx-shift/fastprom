#!/usr/bin/env python3
"""Check that both halves of a stack agree on the connector pinout.

The two boards' board-to-board pinouts must match exactly. A mismatch is
invisible in both schematics - each is internally consistent - and fatal in
hardware. They are generated from one function so they agree by construction,
but this verifies it independently from the exported netlists, which is the
only way to catch a regression in that function or a hand edit to one board.
"""
import argparse
import re
import subprocess
import sys

LOWER_REF = "J5"
UPPER_REF = "J6"


def nets_from(path):
    with open(path) as fh:
        text = fh.read()
    out, cur = {}, None
    for line in text[text.index("(nets"):].splitlines():
        ls = line.strip()
        m = re.match(r'\(name "([^"]+)"\)', ls)
        if m:
            cur = m.group(1)
            out[cur] = []
        m = re.match(r'\(ref "([^"]+)"\)', ls)
        if m:
            out.setdefault(cur, []).append([m.group(1)])
        m = re.match(r'\(pin "([^"]+)"\)', ls)
        if m and out.get(cur):
            out[cur][-1].append(m.group(1))
    return {k: [".".join(v) for v in vs] for k, vs in out.items()}


def contact_map(nets, ref):
    out = {}
    for net, pins in nets.items():
        # Nets named by a local label carry the sheet path ("/BUF_A0"); nets
        # named by a global label or a power symbol do not. Compare the bare
        # name, or a drawing that names wires locally looks like a mismatch.
        bare = net.rsplit("/", 1)[-1]
        for pin in pins:
            if pin.startswith(ref + "."):
                out[pin.split(".", 1)[1]] = bare
    return out


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--lower", default="hw/fastprom-lower.net")
    ap.add_argument("--upper", default="hw/fastprom-upper.net")
    args = ap.parse_args()

    lower = contact_map(nets_from(args.lower), LOWER_REF)
    upper = contact_map(nets_from(args.upper), UPPER_REF)

    if not lower or not upper:
        sys.exit("could not find the connector in one of the netlists")

    problems = []
    if len(lower) != len(upper):
        problems.append(f"contact counts differ: {len(lower)} vs {len(upper)}")

    for c in sorted(set(lower) | set(upper), key=lambda x: int(x)):
        a, b = lower.get(c), upper.get(c)
        if a != b:
            problems.append(f"contact {c}: lower={a} upper={b}")

    signals = sum(1 for v in lower.values() if v.startswith("BUF_"))
    enable = sum(1 for v in lower.values() if v == "D_BUF_NOE")
    grounds = sum(1 for v in lower.values() if v == "GND")
    power = sum(1 for v in lower.values() if v.startswith("+5V"))

    print(f"connector: {len(lower)} contacts "
          f"({signals} signal, {enable} enable, {power} power, "
          f"{grounds} ground)")

    if signals != 18:
        problems.append(f"expected 18 socket signals, found {signals}")
    if enable != 1:
        problems.append(f"expected the drive enable once, found {enable}")
    if grounds < signals // 2:
        problems.append(f"only {grounds} grounds for {signals} signals - "
                        f"too few for a clean return path")

    if problems:
        for p in problems:
            print(f"FAIL {p}")
        sys.exit(1)

    print("check_stack: PASS (both halves agree on every contact)")


if __name__ == "__main__":
    main()
