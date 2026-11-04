"""Persisted routing for the stack boards.

A board's copper - every track and via - lives in hw/<board>.route.json, so
building the board needs only placement (gen_layout.py) and that file, not
an autorouter run. The file is written once per routing, by `make reroute`:
Freerouting routes, gen_layout.py --ses imports and tidies the result,
cleans up its corners (smooth(), below), and this module writes it out.

Coordinates are integer nanometres in the stack's shared frame (the upper
board's top-left corner is the origin), which is exactly KiCad's internal
unit: what is read back is what was written, with no rounding to break a
connection. Pure Python; no pcbnew.

The file also carries a fingerprint of what the routing was made against -
every pad's position, side and net, and the outline. Applying it to a board
whose placement or netlist has since changed is refused rather than producing
copper that no longer lands on its pads.
"""
import hashlib
import json
import math
import re

from netlist import parse_sexpr

VERSION = 1


def fingerprint(pads, outline):
    """pads: iterable of (ref, num, net, x_nm, y_nm, side)."""
    h = hashlib.sha256()
    for p in sorted(pads):
        h.update(repr(p).encode())
    h.update(repr(sorted(outline)).encode())
    return h.hexdigest()[:16]


def write(path, board, placement, source, tracks, vias):
    """tracks: (layer, net, width_nm, x1, y1, x2, y2); vias: (net, x, y,
    diameter_nm, drill_nm). One item per line, sorted, so that a reroute
    diffs as the copper that actually changed."""
    tracks = sorted(tracks, key=lambda t: (t[1], t[0], t[3:], t[2]))
    vias = sorted(vias, key=lambda v: (v[0], v[1:]))
    with open(path, "w") as fh:
        fh.write("{\n")
        fh.write(f'  "version": {VERSION},\n')
        fh.write(f'  "board": {json.dumps(board)},\n')
        fh.write(f'  "placement": {json.dumps(placement)},\n')
        fh.write(f'  "source": {json.dumps(source)},\n')
        fh.write('  "units": "nm",\n')
        fh.write('  "tracks": [\n')
        fh.write(",\n".join("    " + json.dumps(list(t)) for t in tracks))
        fh.write('\n  ],\n  "vias": [\n')
        fh.write(",\n".join("    " + json.dumps(list(v)) for v in vias))
        fh.write("\n  ]\n}\n")


def read(path):
    with open(path) as fh:
        r = json.load(fh)
    if r.get("version") != VERSION or r.get("units") != "nm":
        raise SystemExit(f"{path}: unsupported routing file")
    r["tracks"] = [tuple(t) for t in r["tracks"]]
    r["vias"] = [tuple(v) for v in r["vias"]]
    return r


def prune(tracks, vias, on_pad, keep_nets=(), stitched=()):
    """Drop copper that leads nowhere, repeatedly until nothing changes: a
    via of a net not in keep_nets (the plane nets, whose vias join a plane)
    with tracks ending on fewer than two layers, a via of a plane net with
    a single plane that meets no track and no pad (it reaches only its
    plane), and a track with an end that meets no other track, via or pad
    on its layer. A via of a net in stitched (one with a plane on two
    layers) always stays: it joins its planes. The pre-route fan-outs leave
    the first and last behind when the autorouter reaches a pad along its
    stub rather than through its via. Returns (tracks, vias, dropped)."""
    def on_seg(L, net, x, y, skip):
        for j, (M, n, w, ax, ay, bx, by) in enumerate(tracks):
            if j == skip or M != L or n != net:
                continue
            dx, dy = bx - ax, by - ay
            if (x - ax) * dy != (y - ay) * dx:
                continue
            if min(ax, bx) <= x <= max(ax, bx) and \
                    min(ay, by) <= y <= max(ay, by):
                return True
        return False

    dropped = 0
    changed = True
    while changed:
        changed = False
        layers_at = {}
        for (L, net, w, x1, y1, x2, y2) in tracks:
            for q in ((x1, y1), (x2, y2)):
                layers_at.setdefault((net, q), set()).add(L)
        kept = []
        for v in vias:
            net, x, y = v[0], v[1], v[2]
            if net in stitched:
                lost = False
            elif net not in keep_nets:
                lost = len(layers_at.get((net, (x, y)), ())) < 2
            else:
                lost = (net, (x, y)) not in layers_at and \
                    not any(on_pad(L, x, y) for L in ("F.Cu", "B.Cu")) and \
                    not any(on_seg(L, net, x, y, None)
                            for L in {t[0] for t in tracks})
            if lost:
                changed = True
                dropped += 1
                continue
            kept.append(v)
        vias = kept
        via_at = {(v[0], v[1], v[2]) for v in vias}
        kept = []
        for i, t in enumerate(tracks):
            L, net, w, x1, y1, x2, y2 = t
            free = False
            for (x, y) in ((x1, y1), (x2, y2)):
                if (net, x, y) in via_at or on_pad(L, x, y) or \
                        on_seg(L, net, x, y, i):
                    continue
                free = True
            if free:
                changed = True
                dropped += 1
                continue
            kept.append(t)
        tracks = kept
    return tracks, vias, dropped


def tidy(tracks, vias, on_pad=lambda layer, x, y: False):
    """Remove an autorouter's leftovers without moving any copper:
    zero-length and duplicate segments go, and a run of collinear segments
    of one net, layer and width becomes one segment wherever nothing else
    joins at the point between them - no third segment, no via, and no pad
    (on_pad), since KiCad joins a track to a pad at the track's end."""
    seen = set()
    out = []
    for t in tracks:
        L, net, w, x1, y1, x2, y2 = t
        if (x1, y1) == (x2, y2):
            continue
        key = (L, net, w) + tuple(sorted(((x1, y1), (x2, y2))))
        if key in seen:
            continue
        seen.add(key)
        out.append(t)
    via_at = {(v[1], v[2]) for v in vias}

    merged = True
    while merged:
        merged = False
        ends = {}
        for i, (L, net, w, x1, y1, x2, y2) in enumerate(out):
            ends.setdefault((L, x1, y1), []).append(i)
            ends.setdefault((L, x2, y2), []).append(i)
        dead = set()
        touched = set()        # changed this pass: their ends index is stale
        for (L, x, y), idx in ends.items():
            if len(idx) != 2 or (x, y) in via_at or touched & set(idx) \
                    or on_pad(L, x, y):
                continue
            a, b = (out[i] for i in idx)
            if a[1] != b[1] or a[2] != b[2]:
                continue
            pa = (a[5], a[6]) if (a[3], a[4]) == (x, y) else (a[3], a[4])
            pb = (b[5], b[6]) if (b[3], b[4]) == (x, y) else (b[3], b[4])
            # collinear and on opposite sides of the joint
            ax, ay = pa[0] - x, pa[1] - y
            bx, by = pb[0] - x, pb[1] - y
            if ax * by - ay * bx != 0 or ax * bx + ay * by >= 0:
                continue
            out[idx[0]] = (L, a[1], a[2], pa[0], pa[1], pb[0], pb[1])
            dead.add(idx[1])
            touched.update(idx)
            merged = True
        out = [t for i, t in enumerate(out) if i not in dead]
    return out


# ---------------------------------------------------------------------------
# Corner cleanup. Freerouting 2.5.0 in its 45-degree mode turns through 90
# and 135 degrees as freely as through 45, leaves micron-sized jogs where its
# pull-tight stops, and its optimizer rejects every pass on these boards (see
# scripts/freerouting.sh); the hand-routed copper (fans.py) turns square
# where its geometry was easiest to write down. smooth() redraws each run of
# track between two fixed points with as few, and as gentle, bends as the
# copper around it allows. It never moves a fixed point: a via, a junction
# of three or more tracks, a change of width, the end of a track, or a pad
# where any of those is (a track passing through a pad keeps a corner in
# it). So every connection stays as it was, and only the path between
# connections changes. Its geometry check is conservative (pads as their
# bounding boxes); KiCad's DRC is the judge (gen_layout.py --ses).
#
#   python3 tools/layout/route.py build/layout/<board>.json
#
# counts a built board's bends the way the layout review does.
# ---------------------------------------------------------------------------
def _deg(ax, ay, px, py, bx, by):
    """How far a track turns at p, coming from a and going on to b:
    0 straight on, 90 a right angle, 180 straight back."""
    ux, uy = px - ax, py - ay
    vx, vy = bx - px, by - py
    lu, lv = math.hypot(ux, uy), math.hypot(vx, vy)
    if lu == 0 or lv == 0:
        return 180.0
    c = max(-1.0, min(1.0, (ux * vx + uy * vy) / (lu * lv)))
    return math.degrees(math.acos(c))


def bend_class(turn):
    """A joint's turn in degrees as the layout review counts it: under
    half a degree is straight on, within a degree of 45 or 90 is that, and
    anything else either sharper than a right angle or off the grid."""
    if turn < 0.5:
        return None
    if abs(turn - 45) < 1.0:
        return "45"
    if abs(turn - 90) < 1.0:
        return "90"
    if turn > 91.0:
        return "acute"
    return "off45"


# What each kind of joint costs smooth(): a 45-degree bend is the unit; a
# right angle is worth chamfering into two of them, and a turn sharper than
# a right angle, or one at an angle no 45-degree grid has, is worth a good
# deal more.
TURN_COST = {None: 0.0, "45": 1.0, "90": 4.0, "off45": 8.0, "acute": 20.0}
# A straight at an odd angle has no bend, but costs a little: of two
# routes with the same bends, the one on the 45-degree grid wins. smooth()
# draws a new one only as a stub of at most OFF_GRID_STUB (a short way into
# an off-grid pad or via), never as a long run across the board.
OFF_GRID_SEG = 0.5
OFF_GRID_STUB = 1000000     # 1 mm


def _octilinear(dx, dy):
    return dx == 0 or dy == 0 or abs(dx) == abs(dy)


def _on_grid(dx, dy):
    """Within a degree of a multiple of 45 degrees (Freerouting's own
    diagonals are rounded to its 0.1 um grid)."""
    a = math.degrees(math.atan2(dy, dx)) % 45.0
    return min(a, 45.0 - a) < 1.0


def _seg_pt(px, py, ax, ay, bx, by):
    dx, dy = bx - ax, by - ay
    ll = dx * dx + dy * dy
    u = 0.0 if ll == 0 else \
        max(0.0, min(1.0, ((px - ax) * dx + (py - ay) * dy) / ll))
    return math.hypot(px - ax - u * dx, py - ay - u * dy)


def _cross(ax, ay, bx, by, cx, cy):
    return (bx - ax) * (cy - ay) - (by - ay) * (cx - ax)


def _seg_seg(a, b):
    (ax, ay, bx, by), (cx, cy, dx, dy) = a, b
    d1, d2 = _cross(ax, ay, bx, by, cx, cy), _cross(ax, ay, bx, by, dx, dy)
    d3, d4 = _cross(cx, cy, dx, dy, ax, ay), _cross(cx, cy, dx, dy, bx, by)
    if ((d1 > 0 > d2) or (d1 < 0 < d2)) and ((d3 > 0 > d4) or (d3 < 0 < d4)):
        return 0.0
    return min(_seg_pt(ax, ay, cx, cy, dx, dy), _seg_pt(bx, by, cx, cy, dx, dy),
               _seg_pt(cx, cy, ax, ay, bx, by), _seg_pt(dx, dy, ax, ay, bx, by))


def _seg_rect(s, r):
    ax, ay, bx, by = s
    x0, y0, x1, y1 = r
    if x0 <= ax <= x1 and y0 <= ay <= y1 or x0 <= bx <= x1 and y0 <= by <= y1:
        return 0.0
    edges = ((x0, y0, x1, y0), (x1, y0, x1, y1), (x1, y1, x0, y1),
             (x0, y1, x0, y0))
    return min(_seg_seg(s, e) for e in edges)


class _Grid:
    """Obstacles by 0.5 mm cell, for the clearance checks."""
    CELL = 500000

    def __init__(self):
        self.cells = {}
        self.items = {}
        self.next = 0

    def _span(self, x0, y0, x1, y1):
        c = self.CELL
        for i in range(int(x0 // c), int(x1 // c) + 1):
            for j in range(int(y0 // c), int(y1 // c) + 1):
                yield (i, j)

    def add(self, item, bbox):
        k = self.next
        self.next += 1
        self.items[k] = (item, bbox)
        for cell in self._span(*bbox):
            self.cells.setdefault(cell, set()).add(k)
        return k

    def remove(self, k):
        _, bbox = self.items.pop(k)
        for cell in self._span(*bbox):
            self.cells[cell].discard(k)

    def near(self, x0, y0, x1, y1):
        ks = set()
        for cell in self._span(x0, y0, x1, y1):
            ks |= self.cells.get(cell, set())
        for k in sorted(ks):
            item, (a, b, c, d) = self.items[k]
            if a <= x1 and c >= x0 and b <= y1 and d >= y0:
                yield item


def _split_at_joins(tracks, vias):
    """Cut each track at every point where another track of its net and
    layer ends on it, or one of its net's vias stands on it, so that every
    connection is at a track end (KiCad also joins an end to the middle of
    a track; smooth() must see that point as fixed)."""
    pts = {}
    for (L, net, w, x1, y1, x2, y2) in tracks:
        pts.setdefault((L, net), set()).update(((x1, y1), (x2, y2)))
    vat = {}
    for v in vias:
        vat.setdefault(v[0], set()).add((v[1], v[2]))
    out = []
    for t in tracks:
        L, net, w, x1, y1, x2, y2 = t
        cuts = []
        for (x, y) in pts[(L, net)] | vat.get(net, set()):
            if (x, y) in ((x1, y1), (x2, y2)):
                continue
            if _cross(x1, y1, x2, y2, x, y) != 0:
                continue
            if min(x1, x2) <= x <= max(x1, x2) and \
                    min(y1, y2) <= y <= max(y1, y2):
                cuts.append((x, y))
        if not cuts:
            out.append(t)
            continue
        cuts.sort(key=lambda q: math.hypot(q[0] - x1, q[1] - y1))
        chain = [(x1, y1)] + cuts + [(x2, y2)]
        for a, b in zip(chain, chain[1:]):
            out.append((L, net, w, *a, *b))
    return out


def _trim_overlaps(tracks):
    """Two tracks of one net and layer leaving the same point in the same
    direction (to within a micron) lie on top of each other: the longer
    one is cut back to start where the shorter ends, which leaves a
    junction there instead of a track doubled back on itself."""
    changed = True
    while changed:
        changed = False
        ends = {}
        for i, (L, net, w, x1, y1, x2, y2) in enumerate(tracks):
            ends.setdefault((L, net, x1, y1), []).append((i, (x2, y2)))
            ends.setdefault((L, net, x2, y2), []).append((i, (x1, y1)))
        for (L, net, px, py), lst in sorted(ends.items()):
            pairs = [(p, q) for p in lst for q in lst if p[0] < q[0]]
            for (i, a), (j, b) in pairs:
                ax, ay, bx, by = a[0] - px, a[1] - py, b[0] - px, b[1] - py
                la, lb = math.hypot(ax, ay), math.hypot(bx, by)
                # in line to within a micron at the nearer end
                if ax * bx + ay * by <= 0 or \
                        abs(ax * by - ay * bx) > 1000 * max(la, lb):
                    continue
                if la == lb:
                    # the same track twice: keep the wider
                    del tracks[i if tracks[i][2] <= tracks[j][2] else j]
                    changed = True
                    break
                k, far, near = (i, a, b) if la > lb else (j, b, a)
                t = tracks[k]
                tracks[k] = (t[0], t[1], t[2], *near, *far)
                changed = True
                break
            if changed:
                break
    return tracks


def smooth(tracks, vias, pads, holes, keepouts, outline, clearance, edge,
           on_pad=lambda layer, x, y: False, forbidden=frozenset(),
           margin=0):
    """Redraw each run of track between fixed points with fewer and gentler
    bends, where the copper around it leaves room.

    tracks/vias: routing-file items (nm). pads: (layers, net, x0, y0, x1,
    y1, round) copper shapes, as their bounding boxes, a round pad as its
    circle; layers is the set of layer names the pad has copper on. holes:
    (net, x1, y1, x2, y2, r), every drilled hole on every layer, from one
    end of its slot to the other (both the same for a round hole), r its
    radius plus the hole clearance less `clearance` (net None for an
    unplated one).
    keepouts: (layer, x0, y0, x1, y1) no-track areas. outline: (x0, y0,
    x1, y1). forbidden: track tuples (normalised: see _norm) that must not
    be drawn - the ones DRC rejected on an earlier try. on_pad(layer, x,
    y): whether a point is on any pad. Returns (tracks, runs redrawn).

    A run may pass through a pad of its own net: where exactly two tracks
    of one width meet on a pad, with no via, the run goes on through it,
    and whatever it is redrawn as must still have a corner (or a point on
    a straight) well inside that pad, or end on it, which is all KiCad
    needs to join them. So two tracks meeting square on a pad can become
    one chamfered track crossing it.

    A run's cost is its joints' TURN_COST plus OFF_GRID_SEG for each
    straight at an odd angle, and a joint at a fixed point where exactly
    two tracks meet counts too (at a via, where the two are branches, only
    if they meet at less than a right angle). For each run, every stretch
    between two of its points is tried as each of the shapes _shapes()
    offers between them - a straight, one 45-degree bend either way round,
    or a 45-degree jog or chamfer between two straights - and the cheapest
    that clears everything (then the shortest) replaces it, if it is
    cheaper than what is there and at most 10 % + 0.2 mm longer, or as
    cheap and shorter. Runs are taken in a fixed order and the whole board
    is swept again until a sweep changes nothing.

    Last, a right angle where two widths meet (a power track that could
    only be widened part of the way) is chamfered: both tracks are cut
    back and joined by a diagonal at the narrower width."""
    tracks = _split_at_joins(_trim_overlaps(list(tracks)), vias)
    via_at = {(v[0], v[1], v[2]) for v in vias}
    grid = _Grid()
    for (layers, net, x0, y0, x1, y1, rnd) in pads:
        grid.add(("pad", frozenset(layers), net, (x0, y0, x1, y1), rnd),
                 (x0, y0, x1, y1))
    for (net, ax, ay, bx, by, r) in holes:
        grid.add(("hole", None, net, (ax, ay, bx, by), r),
                 (min(ax, bx) - r, min(ay, by) - r, max(ax, bx) + r,
                  max(ay, by) + r))
    for (net, x, y, d, _) in vias:
        r = d / 2
        grid.add(("via", None, net, (x, y), r), (x - r, y - r, x + r, y + r))
    for (L, x0, y0, x1, y1) in keepouts:
        grid.add(("keepout", frozenset((L,)), None, (x0, y0, x1, y1), False),
                 (x0, y0, x1, y1))

    def seg_box(L, net, w, ax, ay, bx, by):
        h = w / 2
        return (min(ax, bx) - h, min(ay, by) - h, max(ax, bx) + h,
                max(ay, by) + h)

    # every run of track: a list of points between two fixed points
    ends = {}
    for (L, net, w, x1, y1, x2, y2) in tracks:
        for q in ((x1, y1), (x2, y2)):
            ends.setdefault((L, net, q), []).append(w)

    def inside(L, net, q):
        """The pad of this net that q is on, as (where a corner surely
        joins it, its whole box): the box less a quarter of its smaller
        side all round, or for a round pad a square well inside its
        circle."""
        for (layers, pnet, a, b, c, d, rnd) in pads:
            if pnet == net and L in layers and a <= q[0] <= c and \
                    b <= q[1] <= d:
                k = min(c - a, d - b) * (0.2 if rnd else 0.25)
                return (a + k, b + k, c - k, d - k), (a, b, c, d)
        return None

    def fixed(L, net, q):
        ws = ends[(L, net, q)]
        if len(ws) != 2 or ws[0] != ws[1] or (net, *q) in via_at:
            return True
        return on_pad(L, q[0], q[1]) and inside(L, net, q) is None
    adj = {}
    for i, (L, net, w, x1, y1, x2, y2) in enumerate(tracks):
        adj.setdefault((L, net, (x1, y1)), []).append(i)
        adj.setdefault((L, net, (x2, y2)), []).append(i)
    used = set()
    runs = []
    loops = []
    for i in range(len(tracks)):
        if i in used:
            continue
        L, net, w = tracks[i][:3]
        a, b = tracks[i][3:5], tracks[i][5:7]
        # walk back to a fixed point, then forward to the next
        start, j = a, i
        seen = {i}
        while not fixed(L, net, start):
            k = [m for m in adj[(L, net, start)] if m != j][0]
            if k in seen:
                break                       # a closed loop: leave it be
            seen.add(k)
            t = tracks[k]
            start = t[5:7] if t[3:5] == start else t[3:5]
            j = k
        if not fixed(L, net, start):
            used |= seen
            loops += [tracks[k] for k in sorted(seen)]
            continue
        pts = [start]
        pins = []           # (point, pad areas) for each pad passed through
        cur, j = start, j
        while True:
            used.add(j)
            t = tracks[j]
            cur = t[5:7] if t[3:5] == cur else t[3:5]
            pts.append(cur)
            if fixed(L, net, cur):
                break
            if on_pad(L, cur[0], cur[1]):
                pins.append((cur, *inside(L, net, cur)))
            j = [m for m in adj[(L, net, cur)] if m != j][0]
        runs.append([L, net, w, pts, [], pins])
    runs.sort(key=lambda r: (r[1], r[0], r[3][0], r[3][-1], r[2]))

    for r in runs:
        L, net, w, pts, keys, _ = r
        for a, b in zip(pts, pts[1:]):
            keys.append(grid.add(("seg", frozenset((L,)), net, (*a, *b), w / 2),
                                 seg_box(L, net, w, *a, *b)))
    # the runs that meet at each fixed point, for the joints there
    at = {}
    for n, r in enumerate(runs):
        at.setdefault((r[0], r[1], r[3][0]), []).append((n, 0))
        at.setdefault((r[0], r[1], r[3][-1]), []).append((n, -1))

    def end_dir(n, which):
        p = runs[n][3]
        return (p[0], p[1]) if which == 0 else (p[-1], p[-2])

    def cost(n, pts):
        L, net = runs[n][0], runs[n][1]
        c = 0.0
        for a, p, b in zip(pts, pts[1:], pts[2:]):
            c += TURN_COST[bend_class(_deg(*a, *p, *b))]
        for a, b in zip(pts, pts[1:]):
            dx, dy = b[0] - a[0], b[1] - a[1]
            if not _on_grid(dx, dy):
                c += OFF_GRID_SEG
        for which, q, nb in ((0, pts[0], pts[1]), (-1, pts[-1], pts[-2])):
            meet = at[(L, net, q)]
            if len(meet) != 2 or meet[0][0] == meet[1][0]:
                continue
            m, mw = meet[0] if meet[1] == (n, which) else meet[1]
            _, other = end_dir(m, mw)
            k = bend_class(_deg(*nb, *q, *other))
            # two tracks meeting at a via are two branches, not a bend:
            # only a sharp V between them counts
            if (net, *q) in via_at and k != "acute":
                continue
            c += TURN_COST[k]
        return c

    def length(pts):
        return sum(math.hypot(b[0] - a[0], b[1] - a[1])
                   for a, b in zip(pts, pts[1:]))

    x0, y0, x1, y1 = outline

    def clear(L, net, w, a, b):
        h = w / 2
        lo, hi = edge + h, edge + h
        for (x, y) in (a, b):
            if not (x0 + lo <= x <= x1 - hi and y0 + lo <= y <= y1 - hi):
                return False
        s = (*a, *b)
        reach = h + clearance + margin
        bx0, by0 = min(a[0], b[0]) - reach, min(a[1], b[1]) - reach
        bx1, by1 = max(a[0], b[0]) + reach, max(a[1], b[1]) + reach
        for (kind, layers, onet, geo, extra) in grid.near(bx0, by0, bx1, by1):
            if layers is not None and L not in layers:
                continue
            if kind == "keepout":
                if _seg_rect(s, geo) == 0.0:
                    return False
                continue
            if onet == net:
                continue
            if kind in ("seg", "hole"):
                if _seg_seg(s, geo) < h + extra + clearance + margin:
                    return False
            elif kind == "via":
                if _seg_pt(*geo, *s) < h + extra + clearance + margin:
                    return False
            elif extra:         # a round pad
                cx, cy = (geo[0] + geo[2]) / 2, (geo[1] + geo[3]) / 2
                if _seg_pt(cx, cy, *s) < (geo[2] - geo[0]) / 2 + h + \
                        clearance + margin:
                    return False
            elif _seg_rect(s, geo) < h + clearance + margin:
                return False
        return True

    def simplify(pts):
        out = [pts[0]]
        for p in pts[1:]:
            if p == out[-1]:
                continue
            if len(out) >= 2 and _cross(*out[-2], *out[-1], *p) == 0 and \
                    (out[-1][0] - out[-2][0]) * (p[0] - out[-1][0]) + \
                    (out[-1][1] - out[-2][1]) * (p[1] - out[-1][1]) > 0:
                out[-1] = p
                continue
            out.append(p)
        return out

    def pin(pts, pins):
        """pts with a corner in each pad the run passes through: one it
        already has, an end of the run on the pad, or a point added on a
        straight that crosses the pad's inner area (exactly on the
        straight: a 45-degree one steps in whole nanometres both ways).
        None if a pad is missed."""
        for (orig, (a, b, c, d), (fa, fb, fc, fd)) in pins:
            if any(q == orig or a <= q[0] <= c and b <= q[1] <= d
                   for q in pts) or any(
                    fa <= q[0] <= fc and fb <= q[1] <= fd
                    for q in (pts[0], pts[-1])):
                continue
            for k in range(len(pts) - 1):
                (px, py), (qx, qy) = pts[k], pts[k + 1]
                dx, dy = qx - px, qy - py
                if not _octilinear(dx, dy):
                    continue
                ux, uy = (dx > 0) - (dx < 0), (dy > 0) - (dy < 0)
                lo, hi = 0, max(abs(dx), abs(dy))
                for (p0, u, m0, m1) in ((px, ux, a, c), (py, uy, b, d)):
                    if u == 0:
                        if not m0 <= p0 <= m1:
                            lo, hi = 1, 0
                        continue
                    t0, t1 = (m0 - p0) / u, (m1 - p0) / u
                    lo = max(lo, math.ceil(min(t0, t1)))
                    hi = min(hi, math.floor(max(t0, t1)))
                if lo <= hi:
                    t = (lo + hi) // 2
                    pts = pts[:k + 1] + [(px + ux * t, py + uy * t)] + \
                        pts[k + 1:]
                    break
            else:
                return None
        return pts

    redrawn = set()
    changed = True
    while changed:
        changed = False
        for n, r in enumerate(runs):
            while True:
                L, net, w, pts, keys, pins = r
                old_c, old_l = cost(n, pts), length(pts)
                if old_c == 0 and len(pts) == 2:
                    break
                cands = []
                for i in range(len(pts) - 1):
                    for j in range(i + 1, len(pts)):
                        for shape in _shapes(pts[i], pts[j]):
                            new = simplify(pts[:i] + shape + pts[j + 1:])
                            if pins:
                                new = pin(new, pins)
                            if new is None or new == pts:
                                continue
                            c, ln = cost(n, new), length(new)
                            if c < old_c - 1e-9 and \
                                    ln <= old_l * 1.1 + 200000 or \
                                    abs(c - old_c) < 1e-9 and ln < old_l - 1000:
                                cands.append((c, ln, i, j, new))
                cands.sort(key=lambda t: (t[0], t[1], t[2], t[3]))
                best = None
                old_segs = set(_norm((L, net, w, *a, *b))
                               for a, b in zip(pts, pts[1:]))
                for c, ln, i, j, new in cands:
                    segs = [(a, b) for a, b in zip(new, new[1:])]
                    fresh = [(a, b) for (a, b) in segs
                             if _norm((L, net, w, *a, *b)) not in old_segs]
                    if any(_norm((L, net, w, *a, *b)) in forbidden
                           for (a, b) in fresh):
                        continue
                    if all(clear(L, net, w, a, b) for (a, b) in fresh):
                        best = new
                        break
                if best is None:
                    break
                for k in keys:
                    grid.remove(k)
                keys = [grid.add(("seg", frozenset((L,)), net, (*a, *b), w / 2),
                                 seg_box(L, net, w, *a, *b))
                        for a, b in zip(best, best[1:])]
                r[3], r[4] = best, keys
                redrawn.add(n)
                changed = True
    # right angles where two widths meet
    for (L, net, q), lst in sorted(at.items()):
        ws = ends[(L, net, q)]
        if len(lst) != 2 or len(ws) != 2 or ws[0] == ws[1] or \
                (net, *q) in via_at or on_pad(L, q[0], q[1]) or \
                lst[0][0] == lst[1][0]:
            continue
        (n1, e1), (n2, e2) = lst
        p1, p2 = runs[n1][3], runs[n2][3]
        a = p1[1] if e1 == 0 else p1[-2]
        b = p2[1] if e2 == 0 else p2[-2]
        if bend_class(_deg(*a, *q, *b)) != "90":
            continue
        steps = []
        for o in (a, b):
            dx, dy = o[0] - q[0], o[1] - q[1]
            if not _octilinear(dx, dy):
                break
            steps.append(((dx > 0) - (dx < 0), (dy > 0) - (dy < 0),
                          max(abs(dx), abs(dy))))
        if len(steps) != 2:
            continue
        wd = min(runs[n1][2], runs[n2][2])
        room = min(steps[0][2], steps[1][2]) - 1
        for c in [s for s in _STEPS if s < room]:
            qa = (q[0] + steps[0][0] * c, q[1] + steps[0][1] * c)
            qb = (q[0] + steps[1][0] * c, q[1] + steps[1][1] * c)
            if _norm((L, net, wd, *qa, *qb)) in forbidden or \
                    not clear(L, net, wd, qa, qb):
                continue
            for (n, e, nq) in ((n1, e1, qa), (n2, e2, qb)):
                r = runs[n]
                if e == 0:
                    r[3] = [nq] + r[3][1:]
                else:
                    r[3] = r[3][:-1] + [nq]
                redrawn.add(n)
            runs.append([L, net, wd, [qa, qb],
                         [grid.add(("seg", frozenset((L,)), net, (*qa, *qb),
                                    wd / 2), seg_box(L, net, wd, *qa, *qb))],
                         []])
            break
    out = [(r[0], r[1], r[2], *a, *b) for r in runs
           for a, b in zip(r[3], r[3][1:])]
    # tracks in closed loops (none expected) pass through as they were
    return out + loops, len(redrawn)


def _norm(t):
    """A track with its ends in a fixed order, for comparing geometry."""
    L, net, w, x1, y1, x2, y2 = t
    a, b = sorted(((x1, y1), (x2, y2)))
    return (L, net, w, *a, *b)


# jog and chamfer sizes tried, nm, after the fractions of the run
_STEPS = (600000, 400000, 300000, 200000, 150000, 100000, 50000)


def _shapes(p, q):
    """Paths from p to q (lists of points, both ends included), simplest
    first: the straight (at an odd angle only as a stub of at most
    OFF_GRID_STUB); one 45-degree bend, either way round; a diagonal
    between two straights along the same axis (a jog); and a diagonal
    between a horizontal and a vertical straight (a chamfered corner),
    either way round, at several sizes. All on a 45-degree grid but the
    straight."""
    (px, py), (qx, qy) = p, q
    dx, dy = qx - px, qy - py
    if _octilinear(dx, dy):
        return [[p, q]]
    out = [[p, q]] if math.hypot(dx, dy) <= OFF_GRID_STUB else []
    sx = 1 if dx > 0 else -1
    sy = 1 if dy > 0 else -1
    ax, ay = abs(dx), abs(dy)
    m = min(ax, ay)
    major_x = ax > ay
    rest = abs(ax - ay)

    def along(d):           # d along the major axis, then the diagonal
        return (px + sx * d, py) if major_x else (px, py + sy * d)
    # one bend: straight then diagonal, diagonal then straight
    out.append([p, along(rest), q])
    out.append([p, (px + sx * m, py + sy * m), q])
    # a jog: straight, diagonal, straight on the same axis
    for f in (2, 4):
        for k in range(1, f):
            a = rest * k // f
            d0 = along(a)
            out.append([p, d0, (d0[0] + sx * m, d0[1] + sy * m), q])
    # a chamfered corner: horizontal, diagonal, vertical (or the other way
    # round), the diagonal c long in each axis
    sizes = []
    for f in (4, 2):
        for k in range(f - 1, 0, -1):
            sizes.append(m * k // f)
    sizes += [s for s in _STEPS if s < m]
    sizes.append(0)
    for c in sorted(set(sizes), reverse=True):
        h, v = ax - c, ay - c
        out.append([p, (px + sx * h, py), (px + sx * (h + c), py + sy * c), q])
        out.append([p, (px, py + sy * v), (px + sx * c, py + sy * (v + c)), q])
    return out


def _kids(node, key):
    return [n for n in node[1:] if isinstance(n, list) and n and n[0] == key]


def parse_ses(text, origin_nm):
    """A Specctra session file (Freerouting's output) as routing-file
    tracks and vias. origin_nm: where the frame's origin sits on the KiCad
    sheet, (x, y) in nm. Specctra's y axis points up, KiCad's down."""
    root = parse_sexpr(text)
    routes = _kids(root, "routes")[0]
    unit, per = _kids(routes, "resolution")[0][1:3]
    scale = {"um": 1000, "mm": 1000000, "mil": 25400}[unit] // int(per)
    ox, oy = origin_nm

    def xy(sx, sy):
        return round(float(sx) * scale) - ox, -round(float(sy) * scale) - oy
    tracks, vias = [], []
    for net in _kids(_kids(routes, "network_out")[0], "net"):
        name = net[1]
        for wire in _kids(net, "wire"):
            for path in _kids(wire, "path"):
                layer, width = path[1], round(float(path[2]) * scale)
                pts = [xy(path[i], path[i + 1]) for i in range(3, len(path), 2)]
                for a, b in zip(pts, pts[1:]):
                    tracks.append((layer, name, width, *a, *b))
            if len(wire) > 1 and not _kids(wire, "path"):
                raise SystemExit(f"session wire on {name} is not a path")
        for via in _kids(net, "via"):
            m = re.search(r"_(\d+):(\d+)_um$", via[1])
            if not m:
                raise SystemExit(f"session via padstack {via[1]!r} has no size")
            vias.append((name, *xy(via[2], via[3]),
                         int(m.group(1)) * 1000, int(m.group(2)) * 1000))
    return tracks, vias


def bends(tracks):
    """Every joint where exactly two tracks of one layer meet, and turn:
    [(net, layer, x, y, bend_class)]."""
    ends = {}
    for (L, net, w, x1, y1, x2, y2) in tracks:
        ends.setdefault((L, x1, y1), []).append((x2, y2, net))
        ends.setdefault((L, x2, y2), []).append((x1, y1, net))
    out = []
    for (L, x, y), lst in sorted(ends.items()):
        if len(lst) != 2:
            continue
        k = bend_class(_deg(lst[0][0], lst[0][1], x, y, lst[1][0], lst[1][1]))
        if k:
            out.append((lst[0][2], L, x, y, k))
    return out


# A via off ground wants a ground via this close (mm), for its return
# current to change planes beside it (DESIGN_NOTES.md, "Four layers on both
# boards")
RETURN_VIA = 1.0


def via_returns(vias, gnd_holes):
    """Each via not on ground with the distance to the nearest ground via or
    plated ground hole (a return current's way between In1 and In2, the two
    ground planes): [(net, x, y, distance)], in the dump's mm, farthest
    first. vias: [(x, y, net)]; gnd_holes: [(x, y)]."""
    gnd = [(x, y) for (x, y, n) in vias if n == "GND"] + list(gnd_holes)
    out = []
    for (x, y, n) in vias:
        if n == "GND":
            continue
        d = min((math.hypot(x - gx, y - gy) for (gx, gy) in gnd),
                default=float("inf"))
        out.append((n, x, y, d))
    return sorted(out, key=lambda r: -r[3])


def main():
    import collections
    import sys
    for path in sys.argv[1:]:
        with open(path) as fh:
            d = json.load(fh)
        lay = {"F": "F.Cu", "B": "B.Cu", "I": "In2.Cu"}
        tracks = [(lay[t[5]], t[6], t[4], round(t[0] * 1e6), round(t[1] * 1e6),
                   round(t[2] * 1e6), round(t[3] * 1e6)) for t in d["tracks"]]
        b = bends(tracks)
        kinds = collections.Counter(k for *_, k in b)
        print(f"{path}: {len(tracks)} segments, {len(b)} bends: "
              + ", ".join(f"{kinds[k]} {k}" for k in ("45", "90", "acute",
                                                       "off45")))
        nets = collections.Counter(n for n, *_ in b)
        print("  most bent: " + ", ".join(f"{n} {c}"
                                          for n, c in nets.most_common(6)))
        for (n, L, x, y, k) in b:
            if k != "45":
                print(f"  {k:5} {n} {L} ({x / 1e6:.3f}, {y / 1e6:.3f})")
        holes = [((p["bbox"][0] + p["bbox"][2]) / 2,
                  (p["bbox"][1] + p["bbox"][3]) / 2) for p in d["pads"]
                 if p["net"] == "GND" and p["drill"] > 0]
        r = via_returns(d["vias"], holes)
        far = [v for v in r if v[3] > RETURN_VIA]
        print(f"  vias: {len(d['vias']) - len(r)} ground, {len(r)} other; "
              f"{len(far)} with no ground via within {RETURN_VIA} mm")
        for (n, x, y, dist) in far:
            print(f"    {n} ({x:.3f}, {y:.3f}): {dist:.2f} mm")


if __name__ == "__main__":
    main()
