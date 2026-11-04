#!/usr/bin/env python3
"""Score a generated KiCad schematic against hand-drafting conventions.

The standard is how a schematic is normally drawn by hand:

  * every signal connection is a drawn wire; a net label may *name* a wire
    but must never be what connects two pieces of copper
  * rails use power symbols, whose rail name is visible, at the end of the
    net they name: a symbol ends one wire, or sits on a pin, never partway
    along a wire or on a junction where it is hard to find
  * a rail symbol is upright: a supply's arrow points up, a ground's bars
    hang down (rail_sideways). Which way it points is read from its pin,
    the library angle with the instance's mirror and rotation applied, and
    the graphic must agree, straddling the pin's x and lying on the side
    the pin points to. A symbol pointing left or right, a supply pointing
    down or a ground pointing up is a defect: a connector's rail contacts
    are tied into one upright symbol per rail, never given sideways ones.
  * T-connections carry a junction dot, and four-way junctions are avoided
  * pins meet through a visible piece of wire, never pin on pin, a rail
    symbol's included (pins_on_pins)
  * no text collides with other text, with symbols, or with wires
  * a part's reference and value sit clearly nearer it than any other part,
    at most half as far, so they cannot be read as a neighbour's
    (text_ownership)
  * parts no wire joins keep 3.81 mm (a pitch and a half) apart, each with its
    reference and value, a rail symbol with its name, and a pitch from
    any wire not on its own net (crowded)
  * nothing dangles, nothing is off the page

Connectivity is rebuilt from geometry - wire segments, pin positions, label
anchors - the same way KiCad derives it, so "labels used as connections" is
measured directly: a label name that appears on more than one separate wire
island is doing the job a wire should.

The one exception is a connector (a symbol whose reference starts with J).
A connector is drawn the way its contacts physically sit, so its pins come
in whatever order the part has, and wiring each to where it goes would be a
tangle. A connector pin may instead end in a short stub carrying a label,
with the same label on the wiring it belongs to:

  * a connector stub is an island whose only pin is one connector pin
  * a label join is accepted when every island carrying the name but one
    is a connector stub (label_joins counts the rest)
  * every label on a net joined by labels sits on a stub's free end, a
    wire end touching nothing else, so it reads as "continues elsewhere";
    a label partway along a wire only names it (label_off_stub)
  * a connector stub's label must also appear on an island that is not a
    connector stub, or it connects to nothing drawn (dangling_labels)

Everywhere else the rule stands: labels name wires, they do not join them.

Each functional group is framed by a graphic rectangle with a caption, as
One ROM and the CM4 IO board draw theirs. A frame is not a body: wires may
cross its edges (a signal leaving one block for the next), but its edges
must not cut through a symbol, a table, a pin or any text (block_edges),
frames must not overlap or touch each other (1.27 mm apart at least) or
the title block (block_overlaps),
each must hold a caption in its top band (uncaptioned_blocks), and every
part and table must sit wholly inside one (unframed). The title block is a
body like a symbol's, so nothing is drawn over it.

At a connector, rail pins are tied close to the pins, one vertical tie and
one upright rail symbol per rail, and the lines leaving the connector -
signal stubs, other rails' pins - cross those ties. That is how such a
connector is drawn, so a crossing is not counted when both wires' islands
reach a pin of the same connector and one of them carries a rail symbol:
a connector's rail tie crossed by a line leaving that connector. Every
other crossing counts.

Hard defects are weighted so that any one of them outweighs every soft
metric combined. Soft metrics (crossings, length) only break ties between
otherwise-clean layouts.
"""
import argparse
import re
import sys
from collections import defaultdict

PAPER = {"A4": (297.0, 210.0), "A3": (420.0, 297.0),
         "A2": (594.0, 420.0), "A1": (841.0, 594.0)}

W = {
    # hard: any of these means the sheet is wrong
    "offsheet": 100.0,
    "global_labels": 200.0,
    "label_joins": 200.0,
    "dangling_labels": 100.0,
    "label_off_stub": 100.0,
    "pins_on_pins": 20.0,
    "dangling_ends": 100.0,
    "name_conflicts": 200.0,
    "wire_over_body": 50.0,
    "body_overlaps": 50.0,
    "text_overlaps": 25.0,
    "text_ownership": 15.0,
    "crowded": 15.0,
    "text_on_wires": 15.0,
    "missing_junctions": 20.0,
    "four_way": 20.0,
    "rail_mid_net": 20.0,
    "rail_sideways": 20.0,
    "block_edges": 25.0,
    "block_overlaps": 50.0,
    "uncaptioned_blocks": 50.0,
    "unframed": 25.0,
    # soft: tie-breakers between clean layouts
    "crossings": 3.0,
    "length_mm": 0.01,
}

# KiCad's stroke font advances about 0.9 of its size per character for the
# mostly-uppercase text on a schematic. 0.72 under-measured every string and
# let visibly colliding pin names pass.
CHAR_W = 0.9
EPS = 1e-4


def r2(v):
    return round(v, 2)


# --------------------------------------------------------------------------
# parsing
# --------------------------------------------------------------------------

def parse(path):
    with open(path) as fh:
        text = fh.read()

    m = re.search(r'\(paper "([^"]+)"\)', text)
    paper = PAPER.get(m.group(1) if m else "A4", PAPER["A4"])

    lib_at = text.find("(lib_symbols")
    inst_at = text.find("(symbol (lib_id")
    libtext = text[lib_at:inst_at] if lib_at >= 0 else ""

    libs = {}
    for lm in re.finditer(r'\(symbol "fastprom:([^"]+)"(.*?)\n  \)', libtext, re.S):
        name, body = lm.group(1), lm.group(2)
        xs, ys = [], []
        for r in re.finditer(r'\(rectangle\s*\(start ([-\d.]+) ([-\d.]+)\)\s*'
                             r'\(end ([-\d.]+) ([-\d.]+)\)', body):
            a, b, c, d = (float(g) for g in r.groups())
            xs += [a, c]
            ys += [b, d]
        is_power = "(power)" in body or name == "PWR_FLAG"
        # A symbol's body is everything it draws: rectangles above, and the
        # circles and polylines a test pad, capacitor, diode, bead or rail
        # symbol is made of.
        for c in re.finditer(r'\(circle \(center ([-\d.]+) ([-\d.]+)\) \(radius ([\d.]+)\)', body):
            cx, cy, cr = (float(g) for g in c.groups())
            xs += [cx - cr, cx + cr]
            ys += [cy - cr, cy + cr]
        for p in re.finditer(r'\(polyline \(pts ((?:\(xy [-\d.]+ [-\d.]+\)\s*)+)\)', body):
            for q in re.finditer(r'\(xy ([-\d.]+) ([-\d.]+)\)', p.group(1)):
                xs.append(float(q.group(1)))
                ys.append(float(q.group(2)))
        pins = []
        for pm in re.finditer(
                r'\(pin (\w+) line \(at ([-\d.]+) ([-\d.]+) (\d+)\)\s*'
                r'\(length [\d.]+\)\s*\(name "([^"]*)"[^\n]*\n\s*'
                r'\(number "([^"]*)"', body):
            pins.append((pm.group(6), float(pm.group(2)), float(pm.group(3)),
                         int(pm.group(4)), pm.group(5), pm.group(1)))
        libs[name] = {
            "box": (min(xs), min(ys), max(xs), max(ys)) if xs else None,
            "power": is_power,
            "pins": pins,
        }

    wires = []
    for wm in re.finditer(r'\(wire\s*\(pts\s*\(xy ([-\d.]+) ([-\d.]+)\)\s*'
                          r'\(xy ([-\d.]+) ([-\d.]+)\)\)', text):
        x1, y1, x2, y2 = (r2(float(g)) for g in wm.groups())
        wires.append((x1, y1, x2, y2))

    junctions = {(r2(float(a)), r2(float(b))) for a, b in
                 re.findall(r'\(junction\s*\(at ([-\d.]+) ([-\d.]+)\)', text)}
    noconn = {(r2(float(a)), r2(float(b))) for a, b in
              re.findall(r'\(no_connect\s*\(at ([-\d.]+) ([-\d.]+)\)', text)}

    labels = []
    for lm in re.finditer(r'\((global_label|label) "([^"]*)"\s*(?:\(shape \w+\)\s*)?'
                          r'\(at ([-\d.]+) ([-\d.]+) (\d+)\)(.*?)\(uuid', text, re.S):
        just = "left"
        jm = re.search(r'\(justify ([\w ]+)\)', lm.group(6))
        if jm and "right" in jm.group(1):
            just = "right"
        labels.append((lm.group(1), lm.group(2), r2(float(lm.group(3))),
                       r2(float(lm.group(4))), int(lm.group(5)), just))

    free = []
    for tm in re.finditer(r'\(text "((?:[^"\\]|\\.)*)"\s*\(at ([-\d.]+) ([-\d.]+) \d+\)'
                          r'\s*\(effects \(font \(size ([\d.]+)', text):
        free.append((tm.group(1), float(tm.group(2)), float(tm.group(3)),
                     float(tm.group(4))))

    insts = []
    chunks = text[inst_at:].split("(symbol (lib_id") if inst_at >= 0 else []
    for ch in chunks[1:]:
        hm = re.match(r' "fastprom:([^"]+)"\) \(at ([-\d.]+) ([-\d.]+) (\d+)\)'
                      r'(?:\s*\(mirror (\w)\))?', ch)
        if not hm:
            continue
        name, X, Y = hm.group(1), float(hm.group(2)), float(hm.group(3))
        rot, mirror = int(hm.group(4)), hm.group(5)
        props = []
        for pm in re.finditer(
                r'\(property "(Reference|Value)" "([^"]*)" \(at ([-\d.]+) ([-\d.]+) \d+\)'
                r' \(effects \(font \(size ([\d.]+) [\d.]+\)\)([^\n]*)', ch):
            rest = pm.group(6)
            if "hide yes" in rest:
                continue
            just = "center"
            if "justify left" in rest:
                just = "left"
            elif "justify right" in rest:
                just = "right"
            props.append((pm.group(1), pm.group(2), float(pm.group(3)),
                          float(pm.group(4)), float(pm.group(5)), just))
        ref = re.search(r'\(reference "([^"]+)"\)', ch)
        insts.append({"lib": name, "X": X, "Y": Y, "props": props,
                      "rot": rot, "mirror": mirror,
                      "ref": ref.group(1) if ref else "?"})

    # Tables: each one's extent, from its cells' corners and sizes.
    tables = []
    for tm in re.finditer(r'\(table \(column_count.*?\n  \)\n', text, re.S):
        xs, ys = [], []
        for cm in re.finditer(r'\(at ([-\d.]+) ([-\d.]+) 0\) \(size ([\d.]+) ([\d.]+)\)',
                              tm.group(0)):
            x, y, w, h = map(float, cm.groups())
            xs += [x, x + w]
            ys += [y, y + h]
        if xs:
            tables.append((min(xs), min(ys), max(xs), max(ys)))

    # Frames: graphic rectangles on the sheet itself (a symbol's own
    # rectangles are inside lib_symbols, indented further).
    rects = [tuple(float(g) for g in rm.groups()) for rm in re.finditer(
        r'\n  \(rectangle \(start ([-\d.]+) ([-\d.]+)\) \(end ([-\d.]+) ([-\d.]+)\)',
        text)]
    rects = [(min(a, c), min(b, d), max(a, c), max(b, d)) for a, b, c, d in rects]

    return paper, libs, wires, junctions, noconn, labels, free, insts, tables, rects


# --------------------------------------------------------------------------
# geometry helpers
# --------------------------------------------------------------------------

def on_segment(pt, w, interior=False):
    x, y = pt
    x1, y1, x2, y2 = w
    if abs(x1 - x2) < EPS:
        if abs(x - x1) > EPS:
            return False
        lo, hi = sorted((y1, y2))
        return (lo + EPS < y < hi - EPS) if interior else (lo - EPS <= y <= hi + EPS)
    if abs(y1 - y2) < EPS:
        if abs(y - y1) > EPS:
            return False
        lo, hi = sorted((x1, x2))
        return (lo + EPS < x < hi - EPS) if interior else (lo - EPS <= x <= hi + EPS)
    return False


def boxes_overlap(a, b):
    return a[0] < b[2] - EPS and b[0] < a[2] - EPS and a[1] < b[3] - EPS and b[1] < a[3] - EPS


def seg_hits_box(w, b):
    x1, y1, x2, y2 = w
    if abs(y1 - y2) < EPS:
        lo, hi = sorted((x1, x2))
        return b[1] + EPS < y1 < b[3] - EPS and lo < b[2] - EPS and hi > b[0] + EPS
    if abs(x1 - x2) < EPS:
        lo, hi = sorted((y1, y2))
        return b[0] + EPS < x1 < b[2] - EPS and lo < b[3] - EPS and hi > b[1] + EPS
    return False


def text_box(txt, x, y, size, just="center"):
    w = max(len(txt), 1) * size * CHAR_W
    h = size * 1.3
    if just == "left":
        return (x, y - h / 2, x + w, y + h / 2)
    if just == "right":
        return (x - w, y - h / 2, x, y + h / 2)
    return (x - w / 2, y - h / 2, x + w / 2, y + h / 2)


def label_box(txt, x, y, rot, just):
    """Local labels sit just above the wire they name."""
    w = max(len(txt), 1) * 1.27 * CHAR_W
    if rot in (0, 180):
        if just == "right" or rot == 180:
            return (x - w - 0.25, y - 1.9, x - 0.25, y - 0.3)
        return (x + 0.25, y - 1.9, x + w + 0.25, y - 0.3)
    return (x - 1.9, y - w - 0.25, x - 0.3, y - 0.25)


# --------------------------------------------------------------------------
# grading
# --------------------------------------------------------------------------

def grade(path, detail):
    paper, libs, wires, junctions, noconn, labels, free, insts, tables, rects = parse(path)
    pw, ph = paper
    # KiCad's title block: 110 x 34 mm in the bottom right corner, inside
    # the 10 mm frame.
    title = (pw - 120.0, ph - 44.0, pw - 10.0, ph - 10.0)

    # ---- pins in sheet coordinates -----------------------------------------
    pins = []          # (x, y, inst_index, number, rail_name_or_None)
    bodies = []        # (desc, box, is_power)
    for i, inst in enumerate(insts):
        lib = libs.get(inst["lib"])
        if not lib:
            continue
        X, Y = inst["X"], inst["Y"]
        for num, px, py, _a, nm, _t in lib["pins"]:
            rail = nm if lib["power"] and inst["lib"] != "PWR_FLAG" else None
            pins.append((r2(X + px), r2(Y - py), i, num, rail))
        if lib["box"]:
            x0, y0, x1, y1 = lib["box"]
            box = (X + x0, Y - y1, X + x1, Y - y0)
            if lib["power"]:
                # The graphic starts at the pin; only the part away from the
                # pin can collide with anything.
                if x1 <= EPS:          # drawn left of the pin (sideways)
                    box = (box[0], box[1], X - 1.0, box[3])
                elif x0 >= -EPS:       # drawn right of the pin
                    box = (X + 1.0, box[1], box[2], box[3])
                elif y1 <= EPS:        # drawn below the pin (ground)
                    box = (box[0], Y + 1.0, box[2], box[3])
                else:                  # drawn above the pin (supply)
                    box = (box[0], box[1], box[2], Y - 1.0)
            bodies.append((f'{inst["ref"]}', box, lib["power"]))

    # A table is a body like a symbol's: nothing may run through or over it.
    for k, box in enumerate(tables):
        bodies.append((f"table {k + 1}", box, False))

    bodies.append(("title block", title, False))

    m = defaultdict(float)
    d = defaultdict(list)

    # ---- off sheet --------------------------------------------------------
    # 13 mm: the page frame's coordinate strip sits inside a 10 mm border, so
    # anything closer than that is drawn over the frame.
    M = 13.0
    for w in wires:
        if min(w[0], w[2]) < M or max(w[0], w[2]) > pw - M or \
           min(w[1], w[3]) < M or max(w[1], w[3]) > ph - M:
            m["offsheet"] += 1
    for desc, b, _p in bodies:
        if desc == "title block":
            continue
        if b[0] < M or b[2] > pw - M or b[1] < M or b[3] > ph - M:
            m["offsheet"] += 1
            d["offsheet"].append(desc)

    # ---- connectivity from geometry --------------------------------------
    n = len(wires)
    parent = list(range(n + len(pins)))

    def find(a):
        while parent[a] != a:
            parent[a] = parent[parent[a]]
            a = parent[a]
        return a

    def union(a, b):
        ra, rb = find(a), find(b)
        if ra != rb:
            parent[ra] = rb

    for i in range(n):
        for j in range(i + 1, n):
            wi, wj = wires[i], wires[j]
            for pt in ((wi[0], wi[1]), (wi[2], wi[3])):
                if on_segment(pt, wj):
                    union(i, j)
            for pt in ((wj[0], wj[1]), (wj[2], wj[3])):
                if on_segment(pt, wi):
                    union(i, j)

    at_point = defaultdict(list)
    for k, (x, y, _i, _num, _rail) in enumerate(pins):
        at_point[(x, y)].append(n + k)
        for i, w in enumerate(wires):
            if on_segment((x, y), w):
                union(n + k, i)
    for pt, ids in at_point.items():
        for a in ids[1:]:
            union(ids[0], a)
        # pins meet through a visible wire, never end to end: a symbol
        # sitting on another's pin hides where one ends and the other starts
        if len(ids) > 1:
            m["pins_on_pins"] += len(ids) - 1
            refs = sorted(f"{insts[pins[k - n][2]]['ref']}.{pins[k - n][3]}" for k in ids)
            d["pins_on_pins"].append(f"{' on '.join(refs)} at {pt}")

    # ---- labels ------------------------------------------------------------
    island_names = defaultdict(set)
    name_islands = defaultdict(set)
    for kind, name, x, y, rot, just in labels:
        if kind == "global_label":
            m["global_labels"] += 1
            d["global_labels"].append(name)
        hit = None
        for i, w in enumerate(wires):
            if on_segment((x, y), w):
                hit = i
                break
        if hit is None:
            for k, (px, py, *_rest) in enumerate(pins):
                if (px, py) == (x, y):
                    hit = n + k
                    break
        if hit is None:
            m["dangling_labels"] += 1
            d["dangling_labels"].append(name)
            continue
        root = find(hit)
        island_names[root].add(name)
        name_islands[name].add(root)

    # Connector stubs: islands whose only pin is one pin of a J symbol.
    island_pins = defaultdict(list)
    for k, (_x, _y, i, num, _rail) in enumerate(pins):
        island_pins[find(n + k)].append((i, num))

    def connector_stub(root):
        ps = island_pins.get(root, [])
        return len(ps) == 1 and insts[ps[0][0]]["ref"].startswith("J")

    for name, roots in name_islands.items():
        stubs = [r for r in roots if connector_stub(r)]
        wiring = len(roots) - len(stubs)
        if stubs and wiring == 0:
            m["dangling_labels"] += len(stubs)
            d["dangling_labels"].append(
                f"{name!r} on connector stubs only: no wiring carries it")
        if wiring > 1:
            m["label_joins"] += wiring - 1
            d["label_joins"].append(f"{name!r} joins {wiring} separate wire islands")
    # A net joined by labels carries its name on stubs: each of its labels
    # sits on a wire's free end, touching nothing else, so the label reads
    # as "this goes on elsewhere". One partway along a wire, among the parts
    # it joins, reads as a name and hides the connection.
    ends = defaultdict(int)
    for w in wires:
        ends[(w[0], w[1])] += 1
        ends[(w[2], w[3])] += 1
    pin_at = {(p[0], p[1]) for p in pins}

    def free_end(x, y):
        if ends.get((x, y), 0) != 1 or (x, y) in pin_at:
            return False
        return not any(on_segment((x, y), w, interior=True) for w in wires)

    for kind, name, x, y, rot, just in labels:
        if len(name_islands.get(name, ())) > 1 and not free_end(x, y):
            m["label_off_stub"] += 1
            d["label_off_stub"].append(
                f"{name!r} at ({x}, {y}) joins by label but is not on a stub's free end")
    for root, names in island_names.items():
        if len(names) > 1:
            m["name_conflicts"] += 1
            d["name_conflicts"].append(f"one wire carries {sorted(names)}")

    # ---- dangling wire ends, junctions -----------------------------------
    pin_pts = {(p[0], p[1]) for p in pins}
    label_pts = {(l[2], l[3]) for l in labels}
    for i, w in enumerate(wires):
        for pt in ((w[0], w[1]), (w[2], w[3])):
            touches = 0
            for j, w2 in enumerate(wires):
                if j != i and on_segment(pt, w2):
                    touches += 1
            if touches == 0 and pt not in pin_pts and pt not in noconn \
                    and pt not in label_pts:
                m["dangling_ends"] += 1
                d["dangling_ends"].append(f"wire end at {pt}")

    # Points where three or more conductors meet need a dot.
    meet = defaultdict(int)
    for w in wires:
        for pt in ((w[0], w[1]), (w[2], w[3])):
            meet[pt] += 1
    for x, y, *_rest in pins:
        meet[(x, y)] += 1
    candidates = set(meet)
    for w in wires:
        for pt in list(candidates):
            if on_segment(pt, w, interior=True):
                meet[pt] += 2
    for pt, k in meet.items():
        if k >= 3 and pt not in junctions:
            # A power symbol sitting on a pin is two items at a point, not a
            # branch, and must not demand a dot.
            m["missing_junctions"] += 1
            d["missing_junctions"].append(f"branch at {pt}")
        if k >= 4:
            m["four_way"] += 1
            d["four_way"].append(f"four-way at {pt}")

    # Power symbols end their net, as drawn by hand: one wire ends at the
    # symbol, or the symbol sits on a pin with no wire. A symbol partway
    # along a wire, or on a junction of several, is hard to find.
    for x, y, i, num, rail in pins:
        if not rail:
            continue
        pt = (x, y)
        ends = sum(1 for w in wires if (w[0], w[1]) == pt or (w[2], w[3]) == pt)
        inside = any(on_segment(pt, w, interior=True) for w in wires)
        on_pins = sum(1 for p in pins if (p[0], p[1]) == pt and p[2] != i)
        if inside or not ((ends == 1 and on_pins == 0) or (ends == 0 and on_pins >= 1)):
            m["rail_mid_net"] += 1
            d["rail_mid_net"].append(f"{insts[i]['ref']} ({rail}) at {pt}")

    # Rail symbols are upright: a supply points up, a ground down, never
    # sideways. The pin's direction (library angle, then the instance's
    # mirror and rotation) says which way the symbol points; the graphic
    # must agree, straddling the pin and lying on that side of it.
    names = {0: "right", 90: "up", 180: "left", 270: "down"}
    for inst in insts:
        lib = libs.get(inst["lib"])
        if not lib or not lib["power"] or inst["lib"] == "PWR_FLAG":
            continue
        for _num, _px, _py, ang, nm, _t in lib["pins"]:
            if inst["mirror"] == "x":        # flips y
                ang = (360 - ang) % 360
            elif inst["mirror"] == "y":      # flips x
                ang = (180 - ang) % 360
            ang = (ang + inst["rot"]) % 360
            want = 270 if nm.upper().startswith("GND") else 90
            drawn = True
            if lib["box"] and not inst["rot"] and not inst["mirror"]:
                x0, y0, x1, y1 = lib["box"]
                drawn = (x0 < -EPS and x1 > EPS and
                         (y0 >= -EPS if want == 90 else y1 <= EPS))
            if ang != want or not drawn:
                m["rail_sideways"] += 1
                d["rail_sideways"].append(
                    f"{inst['ref']} ({nm}) at ({inst['X']}, {inst['Y']}) "
                    f"points {names.get(ang, ang)}"
                    + ("" if drawn else ", graphic beside its pin"))

    # ---- crossings ---------------------------------------------------------
    # A connector's rail tie crossed by a line leaving the same connector is
    # the convention, not a crossing: both islands reach a pin of one
    # connector, and one of them carries a rail symbol.
    island_conns = defaultdict(set)
    island_rail = set()
    for k, (_x, _y, i, _num, rail) in enumerate(pins):
        root = find(n + k)
        if insts[i]["ref"].startswith("J"):
            island_conns[root].add(i)
        if rail:
            island_rail.add(root)

    def connector_tie(a, b):
        ra, rb = find(wires.index(a)), find(wires.index(b))
        return (bool(island_conns[ra] & island_conns[rb]) and
                (ra in island_rail or rb in island_rail))

    horiz = [w for w in wires if abs(w[1] - w[3]) < EPS]
    vert = [w for w in wires if abs(w[0] - w[2]) < EPS]
    for h in horiz:
        for v in vert:
            pt = (v[0], h[1])
            if on_segment(pt, h, True) and on_segment(pt, v, True):
                if pt in junctions:
                    m["four_way"] += 1
                    d["four_way"].append(f"junction on a crossing at {pt}")
                elif connector_tie(h, v):
                    pass             # the connector convention
                else:
                    m["crossings"] += 1
                    d["crossings"].append(f"crossing at {pt}")

    # ---- wires through bodies, bodies on bodies --------------------------
    for desc, b, is_power in bodies:
        for w in wires:
            if seg_hits_box(w, b):
                m["wire_over_body"] += 1
                d["wire_over_body"].append(f"wire through {desc}")
    for i in range(len(bodies)):
        for j in range(i + 1, len(bodies)):
            if boxes_overlap(bodies[i][1], bodies[j][1]):
                m["body_overlaps"] += 1
                d["body_overlaps"].append(f"{bodies[i][0]} on {bodies[j][0]}")

    # ---- text ----------------------------------------------------------------
    tboxes = []
    for inst in insts:
        for kind, txt, x, y, sz, just in inst["props"]:
            tboxes.append((f"{inst['ref']}.{kind}={txt!r}", text_box(txt, x, y, sz, just)))
    for kind, name, x, y, rot, just in labels:
        tboxes.append((f"label {name!r}", label_box(name, x, y, rot, just)))
    for txt, x, y, sz in free:
        tboxes.append((f"text {txt[:24]!r}", text_box(txt, x, y, sz, "left")))

    # Pin names inside symbols.
    for inst in insts:
        lib = libs.get(inst["lib"])
        if not lib or lib["power"]:
            continue
        X, Y = inst["X"], inst["Y"]
        for num, px, py, a, nm, _t in lib["pins"]:
            if nm in ("~", ""):
                continue
            wlen = len(nm) * 1.27 * CHAR_W
            sx, sy = X + px, Y - py
            if a == 0:
                b = (sx + 2.8, sy - 0.8, sx + 2.8 + wlen, sy + 0.8)
            elif a == 180:
                b = (sx - 2.8 - wlen, sy - 0.8, sx - 2.8, sy + 0.8)
            elif a == 270:   # top pin, name reads down into the body
                b = (sx - 0.8, sy + 2.8, sx + 0.8, sy + 2.8 + wlen)
            else:            # bottom pin
                b = (sx - 0.8, sy - 2.8 - wlen, sx + 0.8, sy - 2.8)
            tboxes.append((f"{inst['ref']} pin {nm}", b))

    for i in range(len(tboxes)):
        for j in range(i + 1, len(tboxes)):
            if boxes_overlap(tboxes[i][1], tboxes[j][1]):
                m["text_overlaps"] += 1
                d["text_overlaps"].append(f"{tboxes[i][0]} / {tboxes[j][0]}")

    # A part's reference and value read as its own: each sits clearly nearer
    # its own body than any other part's, at most half as far and 1.27 mm
    # nearer at least. Text midway between two parts reads as either's.
    def gap(a, b):
        dx = max(b[0] - a[2], a[0] - b[2], 0.0)
        dy = max(b[1] - a[3], a[1] - b[3], 0.0)
        return (dx * dx + dy * dy) ** 0.5
    parts = {desc: b for desc, b, is_power in bodies
             if not is_power and not desc.startswith(("table", "title"))}
    for desc, tb in tboxes:
        if "." not in desc or " " in desc.split("=")[0]:
            continue
        owner = desc.split(".")[0]
        if owner not in parts:
            continue
        own = gap(tb, parts[owner])
        near = min(((gap(tb, b), o) for o, b in parts.items() if o != owner),
                   default=(1e9, None))
        if near[0] < max(2 * own, own + 1.27):
            m["text_ownership"] += 1
            d["text_ownership"].append(
                f"{desc} is {own:.1f} mm from {owner} but {near[0]:.1f} mm from {near[1]}")

    # Text against other symbols' bodies (a symbol's own pin names are meant
    # to sit inside it) and against wires.
    for desc, tb in tboxes:
        owner = desc.split(" ")[0].split(".")[0]
        for bdesc, b, is_power in bodies:
            if bdesc == owner and " pin " in desc:
                continue
            if boxes_overlap(tb, b):
                m["text_overlaps"] += 1
                d["text_overlaps"].append(f"{desc} on {bdesc}")
        for w in wires:
            if seg_hits_box(w, tb):
                m["text_on_wires"] += 1
                d["text_on_wires"].append(f"{desc} over a wire")
                break

    # ---- frames ----------------------------------------------------------------
    def edges(r):
        x0, y0, x1, y1 = r
        return [(x0, y0, x1, y0), (x1, y0, x1, y1), (x1, y1, x0, y1), (x0, y1, x0, y0)]

    def inside(b, r):
        return r[0] - EPS <= b[0] and b[2] <= r[2] + EPS and \
            r[1] - EPS <= b[1] and b[3] <= r[3] + EPS

    for k, r in enumerate(rects):
        where = f"frame {k + 1} at ({r[0]}, {r[1]})"
        if r[0] < M or r[2] > pw - M or r[1] < M or r[3] > ph - M:
            m["offsheet"] += 1
            d["offsheet"].append(where)
        for e in edges(r):
            for desc, b, _p in bodies:
                if seg_hits_box(e, b):
                    m["block_edges"] += 1
                    d["block_edges"].append(f"{where} cuts {desc}")
            for desc, tb in tboxes:
                if seg_hits_box(e, tb):
                    m["block_edges"] += 1
                    d["block_edges"].append(f"{where} cuts {desc}")
            for x, y, i, num, _rail in pins:
                if on_segment((x, y), e):
                    m["block_edges"] += 1
                    d["block_edges"].append(f"{where} on {insts[i]['ref']} pin {num}")
        if boxes_overlap(r, title):
            m["block_overlaps"] += 1
            d["block_overlaps"].append(f"{where} on the title block")
        # frames keep apart: two sharing an edge read as one
        grown = (r[0] - 1.27, r[1] - 1.27, r[2] + 1.27, r[3] + 1.27)
        for j in range(k + 1, len(rects)):
            if boxes_overlap(grown, rects[j]):
                m["block_overlaps"] += 1
                d["block_overlaps"].append(f"{where} on frame {j + 1}")
        band = (r[0], r[1], r[2], r[1] + 6.0)
        if not any(inside(text_box(t, x, y, sz, "left"), r) and boxes_overlap(
                text_box(t, x, y, sz, "left"), band) for t, x, y, sz in free):
            m["uncaptioned_blocks"] += 1
            d["uncaptioned_blocks"].append(where)
    for desc, b, is_power in bodies:
        if is_power or desc == "title block":
            continue
        if not any(inside(b, r) for r in rects):
            m["unframed"] += 1
            d["unframed"].append(desc)

    # ---- clearance between unrelated parts ---------------------------------
    # A part with its reference and value (a rail symbol with its name) is
    # one envelope. Two parts that no wire joins keep a pitch and a half
    # (3.81 mm) apart, or one reads as belonging to the other.
    env = {}
    for desc, b, _p in bodies:
        if not desc.startswith(("table", "title")):
            env[desc] = list(b)
    for desc, tb in tboxes:
        if "=" not in desc or " " in desc.split("=")[0]:
            continue
        owner = desc.split(".")[0]
        if owner in env:
            e = env[owner]
            env[owner] = [min(e[0], tb[0]), min(e[1], tb[1]),
                          max(e[2], tb[2]), max(e[3], tb[3])]
    ref_inst = {inst["ref"]: i for i, inst in enumerate(insts)}
    roots_of = defaultdict(set)
    for k, (_x, _y, i, _num, _rail) in enumerate(pins):
        roots_of[i].add(find(n + k))
    names = sorted(env)
    for a_i, a in enumerate(names):
        for b in names[a_i + 1:]:
            if roots_of[ref_inst[a]] & roots_of[ref_inst[b]]:
                continue                     # wired together: related
            g = gap(env[a], env[b])
            if g < 3.81 - EPS:
                m["crowded"] += 1
                d["crowded"].append(f"{a} and {b}, not wired together, {g:.1f} mm apart")

    # and its text, and a rail symbol, keep clear of wires they are not on:
    # a pitch from any wire none of the part's own pins reaches. (A part's
    # body against wires is wire_over_body's.)
    labelled = defaultdict(list)
    for desc, tb in tboxes:
        if "=" in desc and " " not in desc.split("=")[0]:
            labelled[desc.split(".")[0]].append(tb)
    for desc, b, is_power in bodies:
        if is_power:
            labelled[desc].append(b)
    for name in names:
        mine = roots_of[ref_inst[name]]
        if not labelled.get(name):
            continue
        e = labelled[name][0]
        for tb in labelled[name][1:]:
            e = (min(e[0], tb[0]), min(e[1], tb[1]), max(e[2], tb[2]), max(e[3], tb[3]))
        for k, w in enumerate(wires):
            if find(k) in mine:
                continue
            wb = (min(w[0], w[2]), min(w[1], w[3]), max(w[0], w[2]), max(w[1], w[3]))
            g = gap(e, wb)
            if g < 2.54 - EPS:
                m["crowded"] += 1
                d["crowded"].append(f"{name} {g:.1f} mm from a wire it is not on, at "
                                    f"({w[0]}, {w[1]})-({w[2]}, {w[3]})")
                break

    m["length_mm"] = sum(abs(w[0] - w[2]) + abs(w[1] - w[3]) for w in wires)
    return m, d


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("schematic")
    ap.add_argument("--quiet", action="store_true")
    ap.add_argument("--where", action="store_true",
                    help="say where each defect is")
    args = ap.parse_args()

    m, d = grade(args.schematic, True)
    score = sum(W[k] * m.get(k, 0) for k in W)
    hard = sum(m.get(k, 0) for k in W if W[k] >= 15)

    if not args.quiet:
        print(args.schematic)
        for k in W:
            print(f"  {k:<18} {m.get(k, 0):>9.1f}   x{W[k]:<6} = {W[k] * m.get(k, 0):>9.1f}")
    if args.where:
        for k in W:
            for line in d.get(k, [])[:25]:
                print(f"  ! {k}: {line}")
    print(f"  SCORE {score:.1f}   hard defects {int(hard)}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
