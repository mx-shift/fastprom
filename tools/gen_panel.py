#!/usr/bin/env python3
"""Panelize the routed stack boards for JLCPCB's assembly line.

Builds hw/fastprom-panel.kicad_pcb from the two routed boards
(hw/fastprom-lower.kicad_pcb, hw/fastprom-upper.kicad_pcb), which
are read and never written. Run in KiCad's Python (scripts/kicad.sh py), by
`make panel`.

The panel, its reasons and the JLC rules behind it are in DESIGN_NOTES.md
("Panel"); in short, COLS matched pairs, tab-routed with mouse bites:

    +------------------------- rail, 5 mm, fiducials + tooling holes ------+
    | |  [U1] [U2] ... [U5]   upper row, turned 180: USB-C at the top,    | |
    |s|                        overhanging into the 2 mm routed gap       |s|
    |i|  [L1] [L2] ... [L5]   lower row, as drawn                         |i|
    |d|                                                                   |d|
    +------------------------- rail, 5 mm ---------------------------------+

Every gap is a 2 mm routed slot. The boards hang from 5 mm tabs on their
long sides only (their short ends have pads or the receptacle at the edge),
two (lower) or three (upper) per side, each tab perforated by a row of
0.6 mm mouse bites along every board edge it meets; none beside a ceramic
chip (capacitor or ferrite bead) at the edge (ceramic_clear). Each instance's nets are renamed with
an instance prefix (L1-, U1-, ...) so that DRC does not see the five
boards' grounds as one net left unconnected, and each reference gets the
instance number (C8_1): the panel's BOM and CPL name every placed part
once.

Everything is placed from the source boards and the constants here, with
KiCad's id generator seeded, so the panel is byte-identical run to run.
"""
import os
import sys

import pcbnew

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
import gen_layout as GL  # noqa: E402

NAME = "fastprom-panel"
COLS = 5                 # matched pairs per panel
RAIL = 5.0               # top and bottom process rails (JLC: 5 mm for SMT)
SIDE = 5.0               # left and right frame bars, which carry the end tabs
GAP = 2.0                # routed slot between boards (JLC: 1.6 or 2 mm)
TAB = 5.0                # tab width (JLC: 5 mm minimum with mouse bites)
MB_D = 0.6               # mouse-bite hole (JLC: 0.6 mm)
MB_PITCH = 0.95          # 0.35 mm between holes (JLC: 0.35-0.4 mm)
MB_N = 5                 # holes per row (JLC: 5-8)
# Hole centres sit this far out from the board edge, in the gap: a hole
# reaches 0.05 mm into the board, 0.25 mm from the boards' copper (0.3 mm
# inside their edges; the hole-to-copper rule is 0.2 mm), and the snapped
# tab leaves a nub of about 0.25 mm to sand off.
MB_OFF = 0.25
FID_IN = 3.85            # fiducial centres from the panel edges (JLC)
TOOL_X = 10.0            # tooling holes, this far in from the ends, on the rails

W_BOARD = 11.02
BOARDS = {
    # tag, row (0 top), turned 180?, tab centres along the board's own y
    # (gen_layout.py's frame, where the upper board starts at
    # -PROG_EXTRA). Each tab is on both long sides: a tab across a gap
    # joins one board's right edge to the next one's left.
    #
    # No tab or mouse bite comes within CERAMIC_ALONG of a ceramic chip
    # (an MLCC or a ferrite bead) whose courtyard is within CERAMIC_EDGE
    # of that long edge, measured along the edge from the tab's end to the
    # courtyard: a snapped tab bends the board there, and the chip cracks.
    # ceramic_clear() checks it against the routed boards and refuses to
    # build the panel otherwise.
    # Lower: between socket pins (1.70 + 2.54 k), whose pads are 0.9 mm
    # from the long edges; no ceramic chip is within 2.8 mm of either edge.
    # Upper, three tabs:
    #   - at the pin 1 end, 1.5 mm in from it (y -2.5 to 2.5), starting
    #     CHAMFER_ALONG past the end of the board's pin 1 chamfer (y -4.0
    #     to -3.0 on the left edge; chamfer_clear): the end carries J3,
    #     the SOICbite whose clip bites the end edge's middle, so the end
    #     itself stays a clean routed edge, but without this the
    #     U1-SW1-U5 end hangs 15.8 mm free past the mid-board tab and
    #     the squeegee bends it (DESIGN_NOTES.md, "Assembly panel").
    #     The regulator group stays clear of it: C2 1.9 mm in from the
    #     right edge, FB1 2.5 mm in from the left, and C1, the only one
    #     within 1.5 mm of an edge (1.0 mm), 1.7 mm along it from the tab;
    #   - mid-board, between the left edge's C14 (top, ends y 10.28) and
    #     C19 (underneath, from y 18.40), each 1.56 mm beyond the tab's
    #     ends, beside the USB clamp U6 and the OR-ing diodes D2/D3;
    #   - beside the receptacle's metal shell at the USB end (its shield
    #     legs 0.69 mm in), flush with that end.
    "fastprom-upper": ("U", 0, True, (0.0, 14.335, 25.25)),
    "fastprom-lower": ("L", 1, False, (5.51, 20.75)),
}
CERAMIC_EDGE = 1.5       # a ceramic chip this close to a long edge (courtyard)
CERAMIC_ALONG = 1.5      # ... keeps this far along it from every tab
CHAMFER_ALONG = 0.5      # a tab's end from the board's pin 1 chamfer
# the bites lie inside the tab's span, so the tab's ends bound them both
assert (MB_N - 1) / 2 * MB_PITCH + MB_D / 2 < TAB / 2


def extent(name):
    """The board's top edge and length in its own frame."""
    o = GL.BOARDS[name]["outline"]
    y0, y1 = min(r[1] for r in o), max(r[3] for r in o)
    return y0, y1 - y0


ROW_H = [extent(n)[1] for n in sorted(BOARDS, key=lambda n: BOARDS[n][1])]
X_B0 = GL.BOARD_X0                       # boards' left edge in their frame

PANEL_W = 2 * SIDE + COLS * W_BOARD + (COLS + 1) * GAP
PANEL_H = 2 * RAIL + sum(ROW_H) + (len(ROW_H) + 1) * GAP


def ceramic(fp):
    """A ceramic chip that cracks when the board bends under it: a
    capacitor (a C_ footprint) or a ferrite bead (an FB reference, on an
    inductor footprint). A cracked MLCC leaks or shorts its rail; a
    cracked ferrite opens it, or worse, only now and then."""
    return (str(fp.GetFPID().GetLibItemName()).startswith("C_")
            or fp.GetReference().startswith("FB"))


def ceramic_clear(name, src, tab_y):
    """The rule above, on the routed board: every ceramic chip (ceramic())
    whose courtyard comes within CERAMIC_EDGE of either long edge must
    keep CERAMIC_ALONG along that edge from every tab's span. Returns the
    breaches, one line each."""
    x0, x1 = X_B0, X_B0 + W_BOARD
    bad = []
    for fp in src.GetFootprints():
        if not ceramic(fp):
            continue
        fp.BuildCourtyardCaches()
        bb = fp.GetCourtyard(pcbnew.B_CrtYd if fp.IsFlipped()
                             else pcbnew.F_CrtYd).BBox()
        (a, b), (c, d) = GL.to_local(bb.GetOrigin()), GL.to_local(bb.GetEnd())
        edge = min(a - x0, x1 - c)
        if edge >= CERAMIC_EDGE:
            continue
        for ty in tab_y:
            along = max(ty - TAB / 2 - d, b - (ty + TAB / 2))
            if along < CERAMIC_ALONG:
                bad.append(f"{name}: {fp.GetReference()} ({edge:.2f} mm from "
                           f"a long edge) is {max(along, 0):.2f} mm along it "
                           f"from the tab at y {ty}; {CERAMIC_ALONG} wanted")
    return bad


def chamfer_clear(name, tab_y):
    """Every tab keeps CHAMFER_ALONG along the long edge from the board's
    pin 1 chamfer (GL.chamfer_cut): a tab, and its bites, meet a straight
    edge only. Returns the breaches, one line each."""
    cut = GL.chamfer_cut(GL.BOARDS[name])
    if cut is None:
        return []
    _, cy, _, sy, c = cut
    bad = []
    for ty in tab_y:
        # the cut runs along the long edge from the corner's end to y_end
        y_end = cy + sy * c
        along = (ty - TAB / 2 - y_end) if sy > 0 else (y_end - ty - TAB / 2)
        if along < CHAMFER_ALONG:
            bad.append(f"{name}: the tab at y {ty} is {along:.2f} mm along "
                       f"the edge from the pin 1 chamfer; {CHAMFER_ALONG} "
                       f"wanted")
    return bad


def col_x(i):
    return SIDE + GAP + i * (W_BOARD + GAP)


def row_y(r):
    return RAIL + GAP + sum(ROW_H[:r]) + r * GAP


def poly(pts):
    s = pcbnew.SHAPE_POLY_SET()
    s.NewOutline()
    for (x, y) in pts:
        s.Append(GL.nm(GL.OX + x), GL.nm(GL.OY + y))
    return s


def rect(x0, y0, x1, y1):
    return poly(((x0, y0), (x1, y0), (x1, y1), (x0, y1)))


def board_shape(name, turned, at):
    """The board's outline (its pin 1 chamfer included) where an instance
    lands on the panel: turned 180 if asked, its top-left at `at`."""
    top, height = extent(name)
    out = []
    for (x, y) in GL.board_poly(GL.BOARDS[name]):
        if turned:
            x, y = 2 * X_B0 + W_BOARD - x, 2 * top + height - y
        out.append((at[0] + x - X_B0, at[1] + y - top))
    return out


def copy_board(panel, src, tag, n, turned, top, height, at):
    """Every item of src but its outline (top edge at y `top` in its
    frame), turned 180 about its centre if asked, moved so its outline's
    top-left corner lands at `at`, with nets and references made the
    instance's own."""
    nets = {}

    def net(item_net):
        name = item_net.GetNetname()
        if not name:
            return None
        if name not in nets:
            ni = pcbnew.NETINFO_ITEM(panel, f"{tag}{n}-{name}")
            panel.Add(ni)
            nets[name] = ni
        return nets[name]

    centre = pcbnew.VECTOR2I(GL.nm(GL.OX + X_B0 + W_BOARD / 2),
                             GL.nm(GL.OY + top + height / 2))
    delta = pcbnew.VECTOR2I(GL.nm(at[0] - X_B0), GL.nm(at[1] - top))
    half = pcbnew.EDA_ANGLE(180.0, pcbnew.DEGREES_T)

    def place(item):
        if turned:
            item.Rotate(centre, half)
        item.Move(delta)

    for fp in src.GetFootprints():
        d = fp.Duplicate(False).Cast()
        d.SetReference(f"{fp.GetReference()}_{n}")
        for p in d.Pads():
            nn = net(p.GetNet())
            if nn is not None:
                p.SetNet(nn)
        place(d)
        panel.Add(d)
    for t in src.GetTracks():
        d = t.Duplicate().Cast()
        nn = net(t.GetNet())
        if nn is not None:
            d.SetNet(nn)
        place(d)
        panel.Add(d)
    for z in src.Zones():
        d = z.Duplicate(False).Cast()
        nn = net(z.GetNet())
        if nn is not None:
            d.SetNet(nn)
        place(d)
        panel.Add(d)
    for g in src.GetDrawings():
        if g.GetLayer() == pcbnew.Edge_Cuts:
            continue
        d = g.Duplicate().Cast()
        place(d)
        panel.Add(d)


def outline(panel, shapes, tabs):
    """Edge.Cuts: the panel's substrate is the frame, the boards (shapes:
    board_shape()'s) and the tabs; its outline and its holes (the routed
    slots) are drawn as segments."""
    sub = rect(0, 0, PANEL_W, RAIL)
    for s in (rect(0, PANEL_H - RAIL, PANEL_W, PANEL_H),
              rect(0, 0, SIDE, PANEL_H),
              rect(PANEL_W - SIDE, 0, PANEL_W, PANEL_H)):
        sub.BooleanAdd(s)
    for b in shapes:
        sub.BooleanAdd(poly(b))
    for t in tabs:
        sub.BooleanAdd(rect(*t))
    sub.Simplify()

    def chain(ln):
        pts = [ln.CPoint(k) for k in range(ln.PointCount())]
        for a, b in zip(pts, pts[1:] + pts[:1]):
            s = pcbnew.PCB_SHAPE(panel, pcbnew.SHAPE_T_SEGMENT)
            s.SetStart(a)
            s.SetEnd(b)
            s.SetLayer(pcbnew.Edge_Cuts)
            s.SetWidth(GL.mm(0.1))
            panel.Add(s)
    for o in range(sub.OutlineCount()):
        chain(sub.Outline(o))
        for h in range(sub.HoleCount(o)):
            chain(sub.Hole(o, h))


def main():
    path = os.path.join(GL.HW, NAME + ".kicad_pcb")
    GL.seed(NAME, "panel")
    GL.fresh_project(NAME)
    panel = pcbnew.NewBoard(path)
    GL.title_block(panel, "FastPROM assembly panel - lower and upper boards")
    panel.SetCopperLayerCount(4)
    for layer in sorted({p[1] for spec in GL.BOARDS.values()
                         for p in spec["planes"]}):
        panel.SetLayerType(GL.LAYER_ID[layer], pcbnew.LT_POWER)
    ds = panel.GetDesignSettings()
    ds.SetBoardThickness(GL.mm(GL.THICKNESS))
    ds.m_MinClearance = GL.mm(GL.RULES["clearance"])
    ds.m_TrackMinWidth = GL.mm(GL.RULES["min_track"])
    ds.m_ViasMinSize = GL.mm(GL.RULES["via_d"])
    ds.m_MinThroughDrill = GL.mm(GL.RULES["via_drill"])
    ds.m_ViasMinAnnularWidth = GL.mm(0.1)
    ds.m_CopperEdgeClearance = GL.mm(GL.RULES["edge"])
    ds.m_HoleClearance = GL.mm(GL.RULES["hole_clearance"])
    ds.m_HoleToHoleMin = GL.mm(0.25)
    ds.m_SolderMaskMinWidth = GL.mm(0.0)
    GL.silk_settings(ds)
    nc = ds.m_NetSettings.GetDefaultNetclass()
    nc.SetClearance(GL.mm(GL.RULES["clearance"]))
    nc.SetTrackWidth(GL.mm(GL.W_SIG))
    nc.SetViaDiameter(GL.mm(GL.RULES["via_d"]))
    nc.SetViaDrill(GL.mm(GL.RULES["via_drill"]))

    tabs, bites, breaches, shapes = [], [], [], []
    for name, (tag, r, turned, tab_y) in BOARDS.items():
        src = pcbnew.LoadBoard(os.path.join(GL.HW, name + ".kicad_pcb"))
        breaches += ceramic_clear(name, src, tab_y)
        top, height = extent(name)
        y0 = row_y(r)
        for i in range(COLS):
            copy_board(panel, src, tag, i + 1, turned, top, height,
                       (col_x(i), y0))
            shapes.append(board_shape(name, turned, (col_x(i), y0)))
        breaches += chamfer_clear(name, tab_y)
        for ty in tab_y:
            yc = y0 + (height - (ty - top) if turned else ty - top)
            for i in range(COLS + 1):
                # the tab across the gap left of column i (or the right
                # side bar's gap, for i == COLS), 0.5 mm into what it joins
                xl = col_x(i - 1) + W_BOARD if i else SIDE
                xr = col_x(i) if i < COLS else PANEL_W - SIDE
                tabs.append((xl - 0.5, yc - TAB / 2, xr + 0.5, yc + TAB / 2))
                for k in range(MB_N):
                    yy = yc + (k - (MB_N - 1) / 2) * MB_PITCH
                    if i:                    # a board on the left
                        bites.append((xl + MB_OFF, yy))
                    if i < COLS:             # a board on the right
                        bites.append((xr - MB_OFF, yy))
    if breaches:
        raise SystemExit("tabs beside ceramic chips at the edge:\n  "
                         + "\n  ".join(breaches))
    outline(panel, shapes, tabs)

    def part(ref, fpid, x, y, side="F"):
        fp = GL.place(panel, ref, {"footprint": fpid,
                                   "value": fpid.split(":")[1]},
                      (x, y, 0, side))
        return fp

    for k, (x, y) in enumerate(sorted(bites, key=lambda p: (p[1], p[0]))):
        part(f"MB{k + 1}", "fastprom:Panel_MouseBite_NPTH_0.6mm", x, y)
    fids = [(FID_IN, FID_IN), (PANEL_W - FID_IN, FID_IN),
            (FID_IN, PANEL_H - FID_IN)]
    k = 0
    for side in ("F", "B"):
        for (x, y) in fids:
            k += 1
            part(f"FID{k}", "fastprom:Panel_Fiducial_1mm_Mask2mm", x, y, side)
    for k, (x, y) in enumerate(((TOOL_X, RAIL / 2),
                                (PANEL_W - TOOL_X, RAIL / 2),
                                (TOOL_X, PANEL_H - RAIL / 2),
                                (PANEL_W - TOOL_X, PANEL_H - RAIL / 2))):
        part(f"TH{k + 1}", "fastprom:Panel_ToolingHole_NPTH_2mm", x, y)

    # JLC prints its order number wherever this text is: on the rail, not
    # on a board with no room for it
    t = pcbnew.PCB_TEXT(panel)
    t.SetText("JLCJLCJLCJLC")
    t.SetLayer(pcbnew.F_SilkS)
    t.SetPosition(GL.pt(PANEL_W / 2, RAIL / 2))
    t.SetTextSize(pcbnew.VECTOR2I(GL.mm(1.0), GL.mm(1.0)))
    t.SetTextThickness(GL.mm(0.15))
    panel.Add(t)

    panel.BuildListOfNets()
    GL.save(NAME, panel)
    print(f"{NAME}: {COLS} pairs, {PANEL_W:.2f} x {PANEL_H:.2f} mm, "
          f"{len(tabs)} tabs, {len(bites)} mouse bites "
          f"-> {os.path.relpath(path, GL.ROOT)}")


if __name__ == "__main__":
    main()
