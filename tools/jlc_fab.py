#!/usr/bin/env python3
"""Turn KiCad's exports into a JLCPCB order: gerber zip, BOM and CPL.

Run by `make jlc` after kicad-cli has written the gerbers, drill files and
position file into build/jlc/<board>/. Parts come from tools/jlc_parts.yaml,
matched on value and footprint; anything in the netlist without a matching
line stops the build, so a schematic change cannot silently order the wrong
part.

The CPL gives each part's position and rotation for JLC's library footprint
(the EasyEDA footprint linked to its LCSC number), not for ours: the `cpl`
table in jlc_parts.yaml holds each part's rotation and centroid correction,
and `placement` applies it, with JLC's bottom-side convention.
"""
import csv
import math
import os
import sys
import zipfile

import yaml

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(HERE)
sys.path.insert(0, os.path.join(HERE, "layout"))
import netlist as NL  # noqa: E402


def placement(x, y, rot, bottom, corr):
    """CPL position and rotation of a part whose footprint origin is at x, y
    (mm, Y up, seen from the top, as in KiCad's position file) at KiCad angle
    `rot`, for JLC's footprint: `corr` is its jlc_parts.yaml `cpl` entry.

    On the bottom JLC takes the angle as seen looking at the part, from
    underneath, with X still as seen from the top: 180 - KiCad's angle
    (kicad-jlcpcb-tools and JLC-Plugin-for-KiCad write the same). The offset
    is in our footprint's own coordinates (footprint editor, y down), so it
    goes through the same transform KiCad applies to the footprint's pads:
    rotate on top; flip then rotate on the bottom."""
    base = (180 - rot) if bottom else rot
    dx, dy = corr.get("offset", (0, 0))
    a = math.radians(base)
    ox = dx * math.cos(a) + dy * math.sin(a)       # (dx, -dy) rotated by base
    oy = dx * math.sin(a) - dy * math.cos(a)
    if bottom:
        ox = -ox
    r = (base + corr["rotation"]) % 360
    return x + ox, y + oy, 0.0 if abs(r - 360) < 1e-9 else r


def main():
    """jlc_fab.py <board>, or jlc_fab.py <panel> --from <board> <board>...
    for a panel from tools/gen_panel.py: its parts are the boards' parts,
    once per instance, named <ref>_<n> (the boards' references are
    disjoint, so the reference alone says which board a part is from)."""
    args = sys.argv[1:]
    pairs = None
    if "--pairs" in args:
        # assemble only instances 1..N of each board: a prototype order
        # assembles a handful of modules from the panel, the rest of its
        # boards stay bare
        i = args.index("--pairs")
        pairs = int(args[i + 1])
        del args[i:i + 2]
    board = args[0]
    sources = args[2:] if args[1:2] == ["--from"] else [board]
    out = os.path.join(ROOT, "build", "jlc", board)
    parts = yaml.safe_load(open(os.path.join(HERE, "jlc_parts.yaml")))
    table, cpl = parts["parts"], parts["cpl"]
    comps = {}
    for b in sources:
        c, _ = NL.read(os.path.join(ROOT, "hw", b + ".net"))
        if set(c) & set(comps):
            raise SystemExit(f"{b}: references shared with another board: "
                             f"{sorted(set(c) & set(comps))}")
        comps.update(c)
    rows = list(csv.DictReader(open(os.path.join(out, "pos.csv"))))
    if sources != [board]:
        # one entry per placed instance, each standing for its board's part
        inst = {}
        for r in rows:
            base, _, n = r["Ref"].rpartition("_")
            if base in comps and n.isdigit() and \
                    (pairs is None or int(n) <= pairs):
                inst[r["Ref"]] = comps[base]
        def fitted(c):
            # bare pads and other unfitted parts need no placement, and a
            # footprint excluded from the BOM is left out of the file
            if c.get("dnp"):
                return False
            return not any(p["value"] == c["value"] and
                           p["footprint"] == c["footprint"] and p.get("dnp")
                           for p in table)
        missing = sorted(r for r in set(comps) - {k.rpartition("_")[0] for k in inst}
                         if fitted(comps[r]))
        if missing:
            raise SystemExit(f"{board}: no instance of {', '.join(missing)} "
                             "in the position file")
        comps = inst

    def lookup(c):
        for p in table:
            if p["value"] == c["value"] and p["footprint"] == c["footprint"]:
                return p
        return None

    groups, problems, handwork, loose, unfitted = {}, [], [], [], []
    for ref, c in sorted(comps.items()):
        if c.get("dnp"):            # do-not-populate in the schematic itself
            unfitted.append(ref)
            continue
        p = lookup(c)
        if p is None:
            problems.append(f"{ref}: no jlc_parts.yaml line for "
                            f"{c['value']} / {c['footprint']}")
            continue
        if p.get("dnp"):
            continue
        if p.get("loose"):
            loose.append((ref, p))
            continue
        if not p.get("lcsc"):
            handwork.append((ref, p))
            continue
        groups.setdefault(p["lcsc"], (p, []))[1].append(ref)
    problems += [f"{lcsc}: no cpl correction in jlc_parts.yaml for "
                 f"{p['footprint']} ({', '.join(refs)}); derive one "
                 "(build/cpl-check) before ordering"
                 for lcsc, (p, refs) in sorted(groups.items()) if lcsc not in cpl]
    if problems:
        raise SystemExit("\n".join(problems))

    with open(os.path.join(out, f"{board}-bom.csv"), "w", newline="") as fh:
        w = csv.writer(fh)
        w.writerow(["Comment", "Designator", "Footprint", "LCSC Part #"])
        for lcsc, (p, refs) in sorted(groups.items(),
                                      key=lambda kv: kv[1][1][0]):
            w.writerow([p.get("mpn", p["value"]), ",".join(refs),
                        p["footprint"].split(":")[1], lcsc])

    placed = {r: lcsc for lcsc, (p, refs) in groups.items() for r in refs}
    with open(os.path.join(out, f"{board}-cpl.csv"), "w", newline="") as fh:
        w = csv.writer(fh)
        w.writerow(["Designator", "Mid X", "Mid Y", "Layer", "Rotation"])
        for r in rows:
            if r["Ref"] not in placed:
                continue
            bottom = r["Side"] != "top"
            x, y, rot = placement(float(r["PosX"]), float(r["PosY"]),
                                  float(r["Rot"]), bottom,
                                  cpl[placed[r["Ref"]]])
            w.writerow([r["Ref"], f"{x:.6f}mm", f"{y:.6f}mm",
                        "Bottom" if bottom else "Top", f"{rot:.6f}"])

    gdir = os.path.join(out, "gerbers")
    with zipfile.ZipFile(os.path.join(out, f"{board}-gerbers.zip"), "w",
                         zipfile.ZIP_DEFLATED) as z:
        for f in sorted(os.listdir(gdir)):
            z.write(os.path.join(gdir, f), f)

    print(f"{board}: {sum(len(r) for _, r in groups.values())} placed parts "
          f"in {len(groups)} lines")
    for ref, p in handwork:
        print(f"  not from LCSC: {ref} {p.get('mpn', p['value'])}")
    if unfitted:
        print(f"  not fitted (schematic DNP): {', '.join(unfitted)}")
    for ref, p in loose:
        print(f"  ships loose: {ref} {p.get('mpn', p['value'])}")


if __name__ == "__main__":
    main()
