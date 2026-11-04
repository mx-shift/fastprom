#!/usr/bin/env python3
"""Compare two exported netlists by connectivity, ignoring net names.

Redrawing a schematic conventionally means connectivity now comes from
geometry - wires touching pins - rather than from a label at every pin. That
is a much easier thing to get subtly wrong: a wire that misses a pin by a
fraction of a millimetre silently splits a net, and both the old and new
sheets will pass ERC.

So the redraw is only trustworthy if the netlist is provably unchanged. This
compares the *partition of pins into nets*, not the net names, because
geometry-derived nets get auto-generated names that will not match the old
label-derived ones. Two netlists are equivalent when every pin is grouped with
exactly the same set of other pins.
"""
import argparse
import re
import sys


def nets(path):
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
    return {k: frozenset(".".join(v) for v in vs) for k, vs in out.items()}


def partition(n):
    """Set of pin-groups, dropping single-pin nets (no-connects)."""
    return {g for g in n.values() if len(g) > 1}


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("before")
    ap.add_argument("after")
    args = ap.parse_args()

    a, b = nets(args.before), nets(args.after)
    pa, pb = partition(a), partition(b)

    def name_of(d, group):
        for k, v in d.items():
            if v == group:
                return k
        return "?"

    only_a = pa - pb
    only_b = pb - pa

    print(f"before: {len(pa)} multi-pin nets   after: {len(pb)}")

    if not only_a and not only_b:
        print("diff_netlist: PASS (connectivity identical)")
        return 0

    for g in sorted(only_a, key=lambda s: sorted(s)):
        print(f"  LOST  {name_of(a, g)}: {' '.join(sorted(g))}")
    for g in sorted(only_b, key=lambda s: sorted(s)):
        print(f"  NEW   {name_of(b, g)}: {' '.join(sorted(g))}")
    print(f"diff_netlist: FAIL ({len(only_a)} lost, {len(only_b)} new)")
    return 1


if __name__ == "__main__":
    sys.exit(main())
