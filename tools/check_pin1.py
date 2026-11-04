"""Pin 1 marks must show on the assembled board.

Every pin 1 mark is a filled polygon (the pin 1 triangle) on its
footprint's silkscreen layer (hw/lib/fastprom.pretty/README.md). A mark
under a part's body is hidden once the part is placed, so each one,
stroke included, must lie entirely outside every part's body on its side
of the board - its own and its neighbours'. A body is what the Fab layer
on that side draws: each rectangle, polygon and circle, and the convex
hull of each run of joined lines (a package outline drawn line by line,
its pin 1 chamfer included).

    scripts/kicad.sh py tools/check_pin1.py hw/<board>.kicad_pcb ...

Each footprint's mark must also point in toward its own package: its tip
(the vertex opposite the base, the longest side) must be nearer the
footprint's own body than the base's midpoint is. And it must point at
pin 1: taking its direction as base midpoint to tip, the tip lies on pad
1's axis (no further from the line through pad 1's centre in that
direction than the pad extends either side of it, measured across the
direction), pad 1's centre lies ahead of the tip, and the tip is nearer
pad 1's centre than the base's midpoint is. A mark beside pad 1, or
pointing into the package edge next to it, fails.

The module's own pin 1 is the board's shape, not a mark: each board
given must have its outline's corner nearest socket pin 1 (J1's pad 1,
on whichever board given carries J1: the boards share one frame) cut off
at 45 degrees, at least CHAMFER_MIN along each edge, and carry no
board-level filled silk triangle (a mark of the board's own would be
read as its neighbour's, as the upper board's once was by R24/R25).

Exits 1, naming each mark and what it fails, if any mark touches a body,
points away from its own or does not point at its pad 1, or if a board's
pin 1 corner is not chamfered. Run by `make layout-drc` with both boards
(DESIGN_NOTES.md, "Silkscreen marking").
"""
import math
import sys

import pcbnew

NM = 1e6
CHAMFER_MIN = 0.8   # mm along each edge: the pin 1 corner's cut, at least
SOCKET = "J1"


def pts_mm(seq):
    return [(p.x / NM, p.y / NM) for p in seq]


def hull(points):
    """Convex hull, counter-clockwise (monotone chain)."""
    p = sorted(set(points))
    if len(p) < 3:
        return p

    def cross(o, a, b):
        return (a[0] - o[0]) * (b[1] - o[1]) - (a[1] - o[1]) * (b[0] - o[0])
    lo, hi = [], []
    for q in p:
        while len(lo) >= 2 and cross(lo[-2], lo[-1], q) <= 0:
            lo.pop()
        lo.append(q)
    for q in reversed(p):
        while len(hi) >= 2 and cross(hi[-2], hi[-1], q) <= 0:
            hi.pop()
        hi.append(q)
    return lo[:-1] + hi[:-1]


def outline(poly_set):
    o = poly_set.Outline(0)
    return [(o.CPoint(i).x / NM, o.CPoint(i).y / NM)
            for i in range(o.PointCount())]


def bodies(fp, layer):
    """The convex regions the footprint's Fab drawing on `layer` encloses."""
    regions, lines = [], []
    for g in fp.GraphicalItems():
        if not isinstance(g, pcbnew.PCB_SHAPE) or g.GetLayer() != layer:
            continue
        s = g.GetShape()
        if s == pcbnew.SHAPE_T_SEGMENT:
            lines.append(((g.GetStart().x / NM, g.GetStart().y / NM),
                          (g.GetEnd().x / NM, g.GetEnd().y / NM)))
        elif s == pcbnew.SHAPE_T_RECTANGLE:
            regions.append(pts_mm(g.GetRectCorners()))
        elif s == pcbnew.SHAPE_T_POLY:
            regions.append(hull(outline(g.GetPolyShape())))
        elif s == pcbnew.SHAPE_T_CIRCLE:
            c, r = g.GetCenter(), g.GetRadius()
            regions.append([((c.x + r * math.cos(a * math.pi / 16)) / NM,
                             (c.y + r * math.sin(a * math.pi / 16)) / NM)
                            for a in range(32)])
    # runs of lines joined end to end
    groups = []
    for a, b in lines:
        hit = [g for g in groups if a in g["pts"] or b in g["pts"]]
        new = {"pts": {a, b}}
        for g in hit:
            new["pts"] |= g["pts"]
            groups.remove(g)
        groups.append(new)
    for g in groups:
        h = hull(g["pts"])
        if len(h) >= 3:
            regions.append(h)
    return regions


def seg_dist(p, a, b):
    dx, dy = b[0] - a[0], b[1] - a[1]
    ll = dx * dx + dy * dy
    u = 0 if ll == 0 else max(0.0, min(1.0, ((p[0] - a[0]) * dx +
                                             (p[1] - a[1]) * dy) / ll))
    return math.hypot(p[0] - a[0] - u * dx, p[1] - a[1] - u * dy)


def inside(p, poly):
    """p inside or on the convex polygon poly (either winding)."""
    sign = 0
    for a, b in zip(poly, poly[1:] + poly[:1]):
        c = (b[0] - a[0]) * (p[1] - a[1]) - (b[1] - a[1]) * (p[0] - a[0])
        if abs(c) < 1e-12:
            continue
        if sign == 0:
            sign = 1 if c > 0 else -1
        elif (c > 0) != (sign > 0):
            return False
    return True


def crosses(a, b, c, d):
    def o(p, q, r):
        return (q[0] - p[0]) * (r[1] - p[1]) - (q[1] - p[1]) * (r[0] - p[0])
    return (o(a, b, c) * o(a, b, d) < 0) and (o(c, d, a) * o(c, d, b) < 0)


def gap(p, q):
    """Distance between two convex polygons, 0 if they overlap."""
    if any(inside(v, q) for v in p) or any(inside(v, p) for v in q):
        return 0.0
    ep = list(zip(p, p[1:] + p[:1]))
    eq = list(zip(q, q[1:] + q[:1]))
    if any(crosses(a, b, c, d) for a, b in ep for c, d in eq):
        return 0.0
    return min([seg_dist(v, c, d) for v in p for c, d in eq] +
               [seg_dist(v, a, b) for v in q for a, b in ep])


def point_gap(p, q):
    """Distance from point p to the convex polygon q, 0 if inside it."""
    if inside(p, q):
        return 0.0
    return min(seg_dist(p, c, d) for c, d in zip(q, q[1:] + q[:1]))


def pad1(fp, silk):
    """Pad 1's centre and outline on the copper layer of the mark's side
    (None if the footprint has no pad 1)."""
    cu = pcbnew.F_Cu if silk == pcbnew.F_SilkS else pcbnew.B_Cu
    for p in fp.Pads():
        if p.GetNumber() == "1":
            ps = pcbnew.SHAPE_POLY_SET()
            p.TransformShapeToPolygon(ps, cu, 0, 1000, pcbnew.ERROR_INSIDE)
            c = p.GetPosition()
            return (c.x / NM, c.y / NM), outline(ps)
    return None


def aims_at_pad1(tip, mid, pad):
    """Why the mark does not point at pad 1, or None if it does. Its
    direction is from the base's midpoint to the tip; the tip must lie on
    pad 1's axis along that direction (no further from the pad's centre
    line than the pad is wide, measured across the direction), the pad
    must lie ahead of the tip, and the tip must be nearer the pad than the
    base is."""
    if pad is None:
        return "has no pad 1 to point at"
    (cx, cy), shape = pad
    dx, dy = tip[0] - mid[0], tip[1] - mid[1]
    n = math.hypot(dx, dy)
    dx, dy = dx / n, dy / n
    half = max(abs((x - cx) * -dy + (y - cy) * dx) for x, y in shape)
    off = (tip[0] - cx) * -dy + (tip[1] - cy) * dx
    ahead = (cx - tip[0]) * dx + (cy - tip[1]) * dy
    if abs(off) > half + 1e-6:
        return (f"is off pad 1's axis (tip {abs(off):.3f} mm from its "
                f"centre line, pad {half:.3f} mm either side)")
    if ahead <= 0 or math.dist(tip, (cx, cy)) >= math.dist(mid, (cx, cy)):
        return "does not point at pad 1 (pad 1 is not ahead of its tip)"
    return None


def socket_pin1(paths):
    """J1 pad 1's centre, in mm, from the first board that has J1."""
    for path in paths:
        for fp in pcbnew.LoadBoard(path).GetFootprints():
            if fp.GetReference() == SOCKET:
                for p in fp.Pads():
                    if p.GetNumber() == "1":
                        c = p.GetPosition()
                        return c.x / NM, c.y / NM
    return None


def chamfer(board, pin1):
    """Why the board's pin 1 corner is not chamfered, or None if it is:
    an Edge.Cuts segment at 45 degrees joining the two edges that meet at
    the outline's corner nearest socket pin 1, CHAMFER_MIN or more along
    each."""
    if pin1 is None:
        return f"has no socket pin 1 ({SOCKET}) to find its pin 1 corner by"
    segs = []
    for g in board.GetDrawings():
        if (isinstance(g, pcbnew.PCB_SHAPE) and g.GetLayer() == pcbnew.Edge_Cuts
                and g.GetShape() == pcbnew.SHAPE_T_SEGMENT):
            segs.append(((g.GetStart().x / NM, g.GetStart().y / NM),
                         (g.GetEnd().x / NM, g.GetEnd().y / NM)))
    if not segs:
        return "has no outline drawn as segments (a plain rectangle?)"
    xs = [p[0] for s in segs for p in s]
    ys = [p[1] for s in segs for p in s]
    cx = min(xs) if abs(pin1[0] - min(xs)) < abs(pin1[0] - max(xs)) else max(xs)
    cy = min(ys) if abs(pin1[1] - min(ys)) < abs(pin1[1] - max(ys)) else max(ys)
    eps = 1e-3
    for a, b in segs:
        dx, dy = abs(b[0] - a[0]), abs(b[1] - a[1])
        if abs(dx - dy) > eps or dx < CHAMFER_MIN:
            continue
        on_x = [p for p in (a, b) if abs(p[0] - cx) < eps]
        on_y = [p for p in (a, b) if abs(p[1] - cy) < eps]
        if on_x and on_y and on_x[0] != on_y[0]:
            return None
    return (f"is not chamfered at its pin 1 corner ({cx:.2f}, {cy:.2f}), "
            f"at least {CHAMFER_MIN} mm at 45 degrees")


def check(path, pin1=None):
    board = pcbnew.LoadBoard(path)
    fps = list(board.GetFootprints())
    fp_of = {fp.GetReference(): fp for fp in fps}
    fab = {pcbnew.F_SilkS: pcbnew.F_Fab, pcbnew.B_SilkS: pcbnew.B_Fab}
    body = {(fp.GetReference(), L): bodies(fp, L)
            for fp in fps for L in fab.values()}
    bad, marks = [], 0
    # the module's pin 1: the board's chamfered corner
    why = chamfer(board, pin1)
    if why:
        bad.append(f"board: {why}")
    items = [(fp.GetReference(), g) for fp in fps
             for g in fp.GraphicalItems()]
    items += [("board", g) for g in board.GetDrawings()]
    for ref, g in items:
        if not (isinstance(g, pcbnew.PCB_SHAPE)
                and g.GetLayer() in fab
                and g.GetShape() == pcbnew.SHAPE_T_POLY
                and g.IsSolidFill()):
            continue
        marks += 1
        tri = hull(outline(g.GetPolyShape()))
        half = g.GetWidth() / NM / 2        # the stroke round the fill
        L = fab[g.GetLayer()]
        if ref == "board":
            bad.append(f"board: a pin 1 mark of its own on "
                       f"{board.GetLayerName(g.GetLayer())}; the chamfered "
                       f"corner marks the module's pin 1")
            continue
        if ref != "board" and len(tri) == 3 and body[(ref, L)]:
            def to_body(p):
                return min(point_gap(p, r) for r in body[(ref, L)])
            # the base is the longest side; the tip the vertex opposite it
            i = max(range(3), key=lambda k: math.dist(tri[k - 1],
                                                      tri[k - 2]))
            tip, a, b = tri[i], tri[i - 1], tri[i - 2]
            mid = ((a[0] + b[0]) / 2, (a[1] + b[1]) / 2)
            if to_body(tip) >= to_body(mid):
                bad.append(f"{ref}: pin 1 mark on "
                           f"{board.GetLayerName(g.GetLayer())} points away "
                           f"from its own body (tip {to_body(tip):.2f} mm, "
                           f"base {to_body(mid):.2f} mm from it)")
            why = aims_at_pad1(tip, mid, pad1(fp_of[ref], g.GetLayer()))
            if why:
                bad.append(f"{ref}: pin 1 mark on "
                           f"{board.GetLayerName(g.GetLayer())} {why}")
        for other in fps:
            o = other.GetReference()
            for region in body[(o, L)]:
                if gap(tri, region) <= half:
                    whose = "its own body" if o == ref else f"{o}'s body"
                    bad.append(f"{ref}: pin 1 mark on "
                               f"{board.GetLayerName(g.GetLayer())} "
                               f"overlaps {whose}")
    return marks, bad


def main():
    fail = False
    pin1 = socket_pin1(sys.argv[1:])
    for path in sys.argv[1:]:
        marks, bad = check(path, pin1)
        for b in bad:
            print(f"{path}: {b}")
        fail |= bool(bad)
        print(f"{path}: {marks} pin 1 marks, "
              f"{'all clear of every body, each pointing at its pin 1; pin 1 corner chamfered' if not bad else 'NOT CLEAR'}")
    sys.exit(1 if fail else 0)


if __name__ == "__main__":
    main()
