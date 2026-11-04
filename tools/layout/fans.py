"""Hand-routed copper for the stack boards, laid before autorouting.

Both boards route on their outer layers only (In1 and In2 are ground
planes), so every crossing costs a via. Most of the copper has one right
answer that an autorouter does not find: the board-to-board connectors,
whose 0.4 mm pads can only be entered along their length, the buffers and
the AG32 that feed them in combs, the lines that have to change sides to
get round each other, and the supply runs. Those are routed here, as data,
and handed to Freerouting as fixed wiring; it routes the rest round them.
Ground is all here too (gen_layout.py prewire() adds a via at each
passive's ground pad); the ground vias that stitch the planes and sit
beside each via off ground come after routing (gen_layout.py stitch()).

Coordinates are mm in the stack's frame (the upper board's top-left corner
is the origin), taken from the placement in tools/gen_layout.py. Each
function names the placement it assumes; gen_layout.py checks every pad it
lands on (prewire(): the copper must clear every other pad) and KiCad's DRC
has the last word. If the parts move, this moves with them.

The corners here are where the geometry is easiest to state - square, or at
whatever angle joins two points - and are what Freerouting routes round.
After routing, route.smooth() redraws them as 45-degree chamfers wherever
the copper around allows (gen_layout.py --ses; DESIGN_NOTES.md, "Routing
quality"), so the finished boards follow these paths but not their corners.
What the docstrings say about order, nesting and which side a line enters
its pad from still holds; the connections never move.

Pure Python, no pcbnew: `python3 tools/layout/fans.py` checks the geometry
against itself.
"""
import math

W = 0.10            # signal track width (gen_layout.py W_SIG)
VIA_D = 0.45
CLR = 0.10          # gen_layout.py RULES["clearance"]
MARGIN = 0.01       # fixed copper is laid 0.11 mm apart at the least

# ---------------------------------------------------------------------------
# Lower board. J5 at (7.5, 16.55) turned 270: right row (odd contacts) pad
# centres x 8.855, left row (even) 6.145, both 0.66 long, so the outer ends
# are at 9.185 / 5.815 and the lane between the rows runs 6.475 - 8.525.
# Position k along it is y = 13.75 + 0.4 k. U3 at (7.5, 8.49) turned 180
# on top (B side left, pads x 5.85); U4 at (7.2, 23.1) turned 90 on top
# (B row y 21.45, A row y 24.75, pads 0.5 apart from x 5.45). U4 stands
# 0.2 mm right of the channel's centre so that its pin 1 triangle, past
# pin 1's outer end, clears J1 pin 10 (DESIGN_NOTES.md, "Silkscreen
# marking").
U4_X, U4_Y = 7.2, 23.1
U4_A1X = U4_X - 1.75          # A1 (pin 2) and B-row pin 19's x
# ---------------------------------------------------------------------------
J5_Y0, J5_P = 13.75, 0.4
L_PAD, R_PAD = 6.145, 8.855
OUT_L = 5.45                        # ground vias outside the left row
LANE_L, LANE_R = 6.84, 8.16         # and in the lane


def k_y(k):
    return round(J5_Y0 + J5_P * k, 4)


def u3_comb():
    """U3's five outputs into the left row from outside. They leave the
    buffer's B side to the left and turn down the 1.5 mm between it and the
    DIP's left pin column, each 0.24 mm outside the one below it, so the
    lowest pad (B1) runs innermost and takes the first contact; then each
    turns right into its contact's outer end. B1..B5 = BUF_nCE0, BUF_A5..A8
    at y 9.74 .. 7.74; contacts k 0, 1, 3, 4, 6 (the grounds between them
    take vias outside the row, between the comb's lines, see j5_vias)."""
    tracks = []
    for i, (net, k) in enumerate((("/BUF_nCE0", 0), ("/BUF_A5", 1),
                                  ("/BUF_A6", 3), ("/BUF_A7", 4),
                                  ("/BUF_A8", 6))):
        py = round(9.74 - 0.5 * i, 4)
        x = round(5.36 - 0.24 * i, 4)
        tracks.append(("F.Cu", net, [(5.85, py), (x, py), (x, k_y(k)),
                                     (L_PAD, k_y(k))]))
    return tracks, []


def u4_comb():
    """U4's nine B-side lines up into the connector's far end, all on top.
    U4 lies across the channel with its B row facing the header: OE and B1
    (its two leftmost) go up outside the left row to k 13 and 14, and OE's
    pull-up R21 (beside it, see lower_supply) joins it on the way; B2 to
    B6 go up the lane, B6 out to the right at k 14, B5 and B4 at k 10 and
    9, B2 and B3 out to the left at k 11 and 10; B7 and B8 (the right end)
    go up outside the right row to k 13 and 12. The mounting pads at the
    header's end leave the lane 2.05 mm wide."""
    y_pad, j = 21.45, 21.2      # pads, and the jog just off their ends
    t = [
        ("F.Cu", "/D_BUF_NOE", [(5.45, y_pad), (5.45, j), (5.25, 21.0),
                                (5.25, k_y(13)), (L_PAD, k_y(13))]),
        ("F.Cu", "/BUF_D0", [(5.95, y_pad), (5.95, j), (5.55, 20.8),
                            (5.55, k_y(14)), (L_PAD, k_y(14))]),
        ("F.Cu", "/BUF_D1", [(6.45, y_pad), (6.45, 21.0), (6.66, 20.79),
                            (6.66, k_y(11)), (L_PAD, k_y(11))]),
        ("F.Cu", "/BUF_D2", [(6.95, y_pad), (6.95, k_y(10)),
                            (L_PAD, k_y(10))]),
        ("F.Cu", "/BUF_D3", [(7.45, y_pad), (7.45, j), (7.25, 21.0),
                            (7.25, k_y(9)), (R_PAD, k_y(9))]),
        ("F.Cu", "/BUF_D4", [(7.95, y_pad), (7.95, j), (7.75, 21.0),
                            (7.75, k_y(10)), (R_PAD, k_y(10))]),
        ("F.Cu", "/BUF_D5", [(8.45, y_pad), (8.45, j), (8.25, 21.0),
                            (8.25, k_y(14)), (R_PAD, k_y(14))]),
        ("F.Cu", "/BUF_D6", [(8.95, y_pad), (8.95, 21.0), (9.4, 20.55),
                             (9.4, k_y(13)), (R_PAD, k_y(13))]),
        ("F.Cu", "/BUF_D7", [(9.35, 22.35), (9.65, 22.35), (9.65, k_y(8)),
                             (R_PAD, k_y(8))]),
    ]
    return t, []


# U2's five outputs reach the right row through vias in the lane, from
# underneath (corridor()); the grounds' vias are outside the left row
# (between the comb's lines) or in the lane, the right row's two socket
# power contacts share one outside the row.
J5_SIG = {  # net: (contact k, via)
    "/BUF_A0": (2, (6.9, 14.55)),
    "/BUF_A4": (3, (8.11, 14.95)),
    "/BUF_A3": (5, (8.11, 15.75)),
    "/BUF_A1": (6, (6.9, 16.15)),
    "/BUF_A2": (7, (8.11, 16.55)),
}
V5 = (9.65, 13.75)      # the socket-power via, outside k 0 and 1


def j5_vias():
    """A via for every contact the combs do not reach on top: U2's five
    lines (U2 being underneath), the socket power and every ground. Each
    joins its pad by a stub along the pad's length."""
    tracks, vias = [], []
    for net, (k, v) in J5_SIG.items():
        tracks.append(("F.Cu", net, [v, (R_PAD, k_y(k))]))
        vias.append((net,) + v)
    gnd = [  # (row x, k, via)
        (L_PAD, 2, (OUT_L, k_y(2))), (L_PAD, 5, (OUT_L, k_y(5))),
        (L_PAD, 7, (OUT_L, k_y(7))), (L_PAD, 8, (LANE_L, k_y(8))),
        (L_PAD, 9, (OUT_L, k_y(9))), (L_PAD, 12, (OUT_L, k_y(12) - 0.01)),
        (R_PAD, 4, (6.9, k_y(4))),
    ]
    for (px, k, v) in gnd:
        tracks.append(("F.Cu", "GND", [(px, k_y(k)), v]))
        vias.append(("GND",) + v)
    for k in (0, 1):
        tracks.append(("F.Cu", "+5V_SKT", [(R_PAD, k_y(k)), V5], 0.2))
    vias.append(("+5V_SKT",) + V5)
    # the right row's two grounds at k 11 and 12 share a via in the lane,
    # between them: outside the row is socket 14's line, up to k 8
    gv = (LANE_R, round((k_y(11) + k_y(12)) / 2, 4))
    for k in (11, 12):
        tracks.append(("F.Cu", "GND", [(R_PAD, k_y(k)), gv]))
    vias.append(("GND",) + gv)
    return tracks, vias


def corridor():
    """U2's five outputs, underneath, from its B side (pads x 8.65, B5 at
    the top: BUF_A4, A3, A2, A1, A0 at y 2.1 .. 4.1) to the lane vias.
    Four leave to the right and turn down beside the DIP's right column,
    the lowest pad innermost, and turn left under U2's far end; the fifth,
    BUF_A4, has no room there (pin 19's pad), so it passes out between
    pins 20 and 19, down outside the column and back in between pins 19 and
    18, to run outermost. Then all five go down under U3, between its
    exposed-pad vias and the pull resistors, 0.21 mm apart, and fan out to
    their vias in the order the lane takes them: BUF_A0 first, on the left,
    then from the right BUF_A4 and A3, then A1 on the left and A2 on the
    right."""
    P = 0.21
    t = []
    ys = [round(5.5 + P * i, 4) for i in range(5)]
    xd = [round(6.85 + P * i, 4) for i in range(5)]
    for i, (net, py) in enumerate((("/BUF_A0", 4.1), ("/BUF_A1", 3.6),
                                   ("/BUF_A2", 3.1), ("/BUF_A3", 2.6))):
        x, y = round(9.12 + P * i, 4), ys[i]
        t.append(("B.Cu", net, [(8.65, py), (x, py), (x, round(y - P, 4)),
                                (round(x - P, 4), y), (round(xd[i] + P, 4), y),
                                (xd[i], round(y + P, 4)), (xd[i], 12.0)]))
    x4, y4 = 9.96, ys[4]
    t.append(("B.Cu", "/BUF_A4", [
        (8.65, 2.1), (9.85, 2.1), (10.25, 2.5), (10.72, 2.97), (11.34, 2.97),
        (11.8, 3.43), (11.8, 5.05), (11.34, 5.51), (10.2, 5.51),
        (x4, 5.76), (x4, round(y4 - P, 4)), (round(x4 - P, 4), y4),
        (round(xd[4] + P, 4), y4), (xd[4], round(y4 + P, 4)), (xd[4], 12.0)]))
    # the fan to the vias, from y 12: each but BUF_A0 steps right far
    # enough to pass BUF_A0's via, the outermost first
    sig = {n: v for n, (k, v) in J5_SIG.items()}
    step = [None, 13.0, 12.8, 12.6, 12.4]
    xs = [None] + [round(xd[i] + 0.23, 4) for i in range(1, 5)]
    t += [
        ("B.Cu", "/BUF_A0", [(xd[0], 12.0), (xd[0], 14.5), sig["/BUF_A0"]]),
        ("B.Cu", "/BUF_A1", [(xd[1], 12.0), (xd[1], step[1]),
                             (xs[1], round(step[1] + 0.23, 4)), (xs[1], 15.75),
                             sig["/BUF_A1"]]),
        ("B.Cu", "/BUF_A2", [(xd[2], 12.0), (xd[2], step[2]),
                             (xs[2], round(step[2] + 0.23, 4)), (xs[2], 16.15),
                             (7.9, 16.55), sig["/BUF_A2"]]),
        ("B.Cu", "/BUF_A3", [(xd[3], 12.0), (xd[3], step[3]),
                             (xs[3], round(step[3] + 0.23, 4)), (xs[3], 15.33),
                             sig["/BUF_A3"]]),
        ("B.Cu", "/BUF_A4", [(xd[4], 12.0), (xd[4], step[4]),
                             (xs[4], round(step[4] + 0.23, 4)), (xs[4], 14.74),
                             sig["/BUF_A4"]]),
    ]
    return t, []


def u4_a_vias():
    """U4's A row (socket side, facing the board's end) drops straight to
    the underside, where the terminator packs are: a via under each pad, in
    two staggered rows since the pads are 0.5 mm apart, and a ground via
    at each end of the rows."""
    tracks, vias = [], []
    for i in range(8):
        x = round(U4_A1X + 0.5 * i, 4)
        vy = 25.375 if i % 2 == 0 else 25.725
        net = f"Net-(U4-A{i + 1})"
        tracks.append(("F.Cu", net, [(x, 24.75), (x, vy)]))
        vias.append((net, x, vy))
    vias += [("GND", 4.85, 25.55), ("GND", 9.55, 25.55)]
    return tracks, vias


def terminators():
    """The terminator packs underneath, fed from the A vias: RN2 (A5-A8)
    across the channel under U4's right half, its A row straight above
    their vias; RN1 (A1-A4) upright under the header's end, its A side
    facing the middle, A1 up into its end pad and A2-A4 up the 0.6 mm
    between the packs into theirs, the innermost into the lowest. Their
    socket sides (mirrored against the pins, being underneath) are the
    autorouter's."""
    t = [
        ("B.Cu", "Net-(U4-A1)", [(5.45, 25.375), (5.45, 24.9), (6.23, 24.12),
                                 (6.23, 21.5), (6.255, 21.5)]),
        ("B.Cu", "Net-(U4-A2)", [(5.95, 25.725), (5.95, 25.13), (6.44, 24.64),
                                 (6.44, 22.08), (6.67, 21.85), (6.67, 21.0),
                                 (6.255, 21.0)]),
        ("B.Cu", "Net-(U4-A3)", [(6.45, 25.375), (6.65, 25.175), (6.65, 22.29),
                                 (6.88, 22.06), (6.88, 20.5), (6.255, 20.5)]),
        ("B.Cu", "Net-(U4-A4)", [(6.95, 25.725), (6.86, 25.635), (6.86, 22.5),
                                 (7.09, 22.27), (7.09, 20.0), (6.255, 20.0)]),
    ]
    for i, x in enumerate((7.23, 7.73, 8.23, 8.73)):
        vx = round(U4_A1X + 2.0 + 0.5 * i, 4)
        vy = 25.375 if i % 2 == 0 else 25.725
        t.append(("B.Cu", f"Net-(U4-A{i + 5})", [(vx, vy), (x, 24.35)]))
    return t, []


# The buffers' exposed-pad vias (gen_layout.py adds a grid in every exposed
# pad; these are the ones the ground stubs below end on, so they are laid
# here and the grid skips them). U3's are along the pad's right edge, so
# that U2's lines pass under the pad's left half; U2's toward the side of
# the pad where the other side of the board has room for them. Freerouting
# 2.5.0 does not see a track ending inside an exposed pad as connected to
# it - nor the pad's own vias, which it takes for clearance violations - so
# each grounded pin is joined to a via, which it does see.
U3_EP = [(8.1, 7.3), (8.1, 8.5), (8.1, 9.7)]
U4_P10 = (9.6, 24.35)    # U4's pin 10 (ground) via
U2_EP = [(7.3, 1.55), (7.48, 3.5)]


def buffer_grounds():
    """The buffers' grounded pins, across the 0.3 mm between pad row and
    exposed pad, to an exposed-pad via: unused inputs A6-A8 and the
    enables of the two receivers, the drive buffer's DIR, and each pin 10;
    and the ground ends of the pull-downs R13 and R12, beside U3
    underneath. U2 is on the underside at (7.0, 2.85), turned 0 (A side
    left, pin 1 end down)."""
    W2 = 0.2
    t = [
        # U3, top: A6-A8 on a bar inside the pad, pin 10 onto it, OE across
        ("F.Cu", "GND", [(9.15, 6.74), (8.6, 6.74), (8.6, 7.3),
                         (8.6, 7.74), (9.15, 7.74)], W2),
        ("F.Cu", "GND", [(9.15, 7.24), (8.6, 7.3)], W2),
        ("F.Cu", "GND", [(8.6, 7.3), U3_EP[0]], W2),
        ("F.Cu", "GND", [(8.25, 6.34), U3_EP[0]], W2),
        ("F.Cu", "GND", [(5.85, 10.24), (6.2, 10.24), (6.5, 9.94),
                         (7.9, 9.94), U3_EP[2]], W2),
        # U4, top: DIR and pin 10 across into the exposed pad (which has
        # no vias: the A lines pass under it), and pin 10 to its via
        ("F.Cu", "GND", [(5.05, 23.85), (5.8, 23.85)], W2),
        ("F.Cu", "GND", [(9.35, 23.85), (8.6, 23.85)], W2),
        ("F.Cu", "GND", [(9.35, 23.85), U4_P10], 0.25),
        # U2, underside: A6-A8 on a bar, pin 10 onto it, OE across
        ("B.Cu", "GND", [(5.35, 1.1), (6.2, 1.1), (6.2, 1.6)], W2),
        ("B.Cu", "GND", [(5.35, 1.6), (6.2, 1.6), (6.2, 2.1)], W2),
        ("B.Cu", "GND", [(5.35, 2.1), (6.2, 2.1), U2_EP[0]], W2),
        ("B.Cu", "GND", [(6.25, 0.7), (6.2, 1.1)], W2),
        ("B.Cu", "GND", [(8.65, 4.6), (8.3, 4.6), (8.0, 4.3), U2_EP[1]], W2),
        # the pull-downs beside U3 into its exposed-pad vias
        ("B.Cu", "GND", [R13_GND, U3_EP[1]], W2),
        ("B.Cu", "GND", [R12_GND, U3_EP[2]], W2),
    ]
    v = [("GND", x, y) for (x, y) in U3_EP + U2_EP + [U4_P10]]
    return t, v


def u3_socket():
    """U3's inputs to socket pins 15-19, on top down the 0.56 mm between
    its A-side pads and the DIP's right pin column: B-side order is the
    socket's (A1 lowest, pin 15 lowest), so the two lines going up nest
    (A5 inside, past its pull-down R14, and into pin 19 from below; A4
    outside, into pin 18), A3 goes straight across to pin 17, and the two
    going down nest the other way (A2 outside, into pin 16; A1 inside,
    then between pins 16 and 15 and into 15 from above, clear of the
    header's vias). The pulls on the other four are underneath, at the
    pins (socket_pulls)."""
    t = [
        ("F.Cu", "/A8", [(9.15, 8.24), (9.64, 8.24), (9.64, 5.5),
                         (10.81, 5.5), (10.81, 4.24)]),
        ("F.Cu", "/A8", [R14_A8, (R14_A8[0], 5.5), (9.64, 5.5)]),
        ("F.Cu", "GND", [R14_GND, (8.25, 6.34)], 0.2),
        ("F.Cu", "/A7", [(9.15, 8.74), (9.88, 8.74), (9.88, 7.3),
                         (10.81, 6.78)]),
        ("F.Cu", "/A6", [(9.15, 9.24), (10.81, 9.32)]),
        ("F.Cu", "/A5", [(9.15, 9.74), (9.88, 9.74), (9.88, 11.0),
                         (10.81, 11.86)]),
        ("F.Cu", "/nCE0", [(9.15, 10.24), (9.64, 10.24), (9.64, 13.13),
                           (10.81, 13.13), (10.81, 14.4)]),
    ]
    return t, []


R14_A8 = (8.76 + 0.51, 5.1)    # R14 (on top, COL_X) pad 1
R14_GND = (8.76 - 0.51, 5.1)
R13_GND = (8.77, 7.88)  # the pull-downs underneath, pad 2
R12_GND = (8.77, 9.86)


def socket_pulls():
    """The pulls on socket pins 18-15, underneath beside the right pin
    column, each pad on its pin's level (R13 pin 18, R12 pin 17, R20 and
    R16 pin 16, R15 pin 15) joined straight across to it. The pull-ups'
    socket power comes down outside the column (lower_supply)."""
    t = [
        ("B.Cu", "/A7", [(8.77, 6.86), (10.81, 6.78)]),
        ("B.Cu", "/A6", [(8.77, 8.84), (9.25, 8.84), (9.73, 9.32),
                         (10.81, 9.32)]),
        ("B.Cu", "/A5", [(8.77, 11.84), (10.81, 11.86)]),
        ("B.Cu", "/A5", [(8.77, 12.8), (9.4, 12.8), (10.15, 12.3),
                         (10.81, 11.86)]),
        ("B.Cu", "/nCE0", [(8.77, 15.8), (9.4, 15.8), (10.3, 14.9),
                           (10.81, 14.4)]),
    ]
    return t, []


def u2_escape():
    """U2, on the underside at the pin 1 end, has its inputs (A side, pads
    x 5.35, A1 lowest at y 4.6) in the reverse of the socket's order, the
    price of being mirrored. A2 goes straight across to pin 2; A4 and A3
    leave between pins 1 and 2 and come down outside the pin column, into
    pins 5 and 4 from outside (going round the pins puts them back in
    order); A1 and A3's neighbour A2 drop through vias beside their pads and
    reach pins 1 and 3 on top."""
    t = [
        ("B.Cu", "/A1", [(5.35, 4.1), (4.4, 4.1), (3.19, 4.24)]),
        ("B.Cu", "/A0", [(5.35, 4.6), (4.95, 4.6), A0_VIA]),
        ("F.Cu", "/A0", [A0_VIA, (5.0, 4.7), (5.0, 2.3), (4.4, 1.7),
                         (3.19, 1.7)]),
        ("B.Cu", "/A2", [(5.35, 3.6), A2_VIA]),
        ("F.Cu", "/A2", [A2_VIA, (4.3, 3.8), (4.3, 5.9), (3.19, 6.78)]),
        ("B.Cu", "/A3", [(5.35, 3.1), (4.3, 3.1), (4.25, 3.15), (2.65, 3.15),
                         (2.19, 3.61), (2.19, 8.86), (2.65, 9.32),
                         (3.19, 9.32)]),
        ("B.Cu", "/A4", [(5.35, 2.6), (4.35, 2.6), (4.2, 2.75), (2.35, 2.75),
                         (1.89, 3.21), (1.89, 11.4), (2.35, 11.86),
                         (3.19, 11.86)]),
    ]
    return t, [("/A0",) + A0_VIA, ("/A2",) + A2_VIA]


A0_VIA = (4.8, 4.9)
A2_VIA = (4.5, 3.6)


def lower_supply():
    """Socket power and the buffers' 3.3 V.

    Socket power leaves pin 20 two ways. Underneath, a 0.2 mm track runs
    down outside the right pin column (outside BUF_A4's detour) and comes
    in between pins 16 and 15 to the connector's via V5 and the pull-ups
    R20 and R15 beside it. On top, it passes C18 and runs along the
    board's end to the buffers' regulator U7, on top over U2 with its
    input and output capacitors C12 and C13 (C13 upright below U7's
    ground pin) and its enable delay R26/C17 in the column at the right
    with R14, whose grounds drop into U2's exposed pad underneath (C12's
    through a via of its own beside its pad).

    The regulator's output, +3V3_BUF, runs straight down from U7's output
    pin, past the test pad TP1 (below-left of U7, on a 0.5 mm stub), to
    C13; from C13 down through a via beside U2's supply pins,
    where C8 is, and from there down the channel's left side underneath,
    with a via up to U3's capacitor C9; then out between pins 5 and 6,
    down outside the left pin column and back in between pins 9 and 10 to
    C10's via, which U4's pin 20 shares; and on top from pin 20 up to
    OE's pull-up R21. TP1 is on top, the side that faces the upper board:
    with the upper board lifted off, it is in the open between the pin 1
    end's small parts, well clear of J5."""
    W5 = 0.2
    t = [
        # socket power, underneath
        ("B.Cu", "+5V_SKT", [(10.81, 1.7), (11.4, 1.7), (12.07, 2.37),
                             (12.07, 12.67), (11.61, 13.13), (10.2, 13.13),
                             V5], W5),
        ("B.Cu", "+5V_SKT", [V5, (8.77, 13.82)], W5),
        ("B.Cu", "+5V_SKT", [V5, (8.77, 14.78)], 0.15),
        # and on top, to the regulator
        ("F.Cu", "+5V_SKT", [(10.81, 1.7), (9.28, 1.05)], 0.3),
        ("F.Cu", "+5V_SKT", [(9.28, 1.05), (8.88, 0.45), (5.0, 0.45),
                             (4.82, 0.63), (4.82, 1.05)], 0.25),
        ("F.Cu", "+5V_SKT", [(4.82, 1.05), (4.82, 1.6), (5.34, 2.12),
                             (6.175, 2.12)], 0.2),
        ("F.Cu", "+5V_SKT", [(9.28, 1.05), R26_5V], 0.2),
        ("F.Cu", "/BUF_EN", [(6.825, 2.12), (7.455, 2.75), R26_EN], 0.15),
        ("F.Cu", "/BUF_EN", [R26_EN, (C17_EN[0], R26_EN[1] + C17_EN[0]
                                      - R26_EN[0]), C17_EN], 0.15),
        # the regulator's grounds, into U2's exposed-pad vias
        ("F.Cu", "GND", [(5.78, 1.05), C12_GND], 0.25),
        ("F.Cu", "GND", [(6.825, 2.98), U2_EP[1]], 0.25),
        ("F.Cu", "GND", [C13_GND, U2_EP[1]], 0.25),
        ("F.Cu", "GND", [C17_GND, U2_EP[1]], 0.25),
        # +3V3_BUF: out, straight down past the test pad to C13, down
        # beside U2's supply end, and along the left
        ("F.Cu", "+3V3_BUF", [(6.175, 2.98), (6.175, TP1_LO[1]),
                              (6.175, 4.88), (6.47, C13_VDD[1]), C13_VDD],
         0.25),
        ("F.Cu", "+3V3_BUF", [TP1_LO, (6.175, TP1_LO[1])], 0.25),
        ("F.Cu", "+3V3_BUF", [(6.47, C13_VDD[1]), VDD_VIA], 0.25),
        ("B.Cu", "+3V3_BUF", [(6.25, 5.0), (6.25, 5.25), VDD_VIA], 0.2),
        ("B.Cu", "+3V3_BUF", [(7.75, 5.0), (6.25, 5.0)], 0.2),
        # pin 1 straight down to C8, under the pin 1 mark (past which C8
        # stands), and on down the left
        ("B.Cu", "+3V3_BUF", [(6.25, 5.25), (C8_VDD[0], 5.33), C8_VDD],
         0.2),
        ("B.Cu", "+3V3_BUF", [C8_VDD, (6.33, 10.67), (6.1, 10.9)], W5),
        ("B.Cu", "+3V3_BUF", [(6.1, 10.9), (5.0, 12.0), (5.0, 12.73),
                              (4.6, 13.13), (2.5, 13.13), (1.9, 13.73),
                              (1.9, 22.83), (2.36, 23.29), (3.75, 23.29),
                              (4.19, 22.85), C10_VDD], W5),
        ("B.Cu", "+3V3_BUF", [(6.1, 10.9), C9_VIA], W5),
        ("F.Cu", "+3V3_BUF", [C9_VIA, (6.75, 10.64)], W5),
        ("F.Cu", "+3V3_BUF", [(6.75, 10.64), (7.08, 10.64), (8.25, 10.64)],
         W5),
        ("F.Cu", "+3V3_BUF", [(7.08, 10.64), (7.08, 11.75)], W5),
        ("F.Cu", "GND", [(6.12, 11.75), C9_GND], 0.25),
        ("B.Cu", "GND", [C8_VSS, C8_GND], 0.25),
        # OE's pull-up, from U4's pin 20
        ("F.Cu", "+3V3_BUF", [R21_VDD, (4.9, 20.16), (4.9, 22.3),
                              (5.05, 22.35)], 0.2),
        ("F.Cu", "/D_BUF_NOE", [R21_NOE, (5.25, k_y(13))]),
    ]
    return t, [("+3V3_BUF",) + VDD_VIA, ("+3V3_BUF",) + C9_VIA,
               ("GND",) + C8_GND, ("GND",) + C18_GND, ("GND",) + C9_GND,
               ("GND",) + C12_GND]


# +3V3_BUF down from the regulator to U2's pin 1 stub, under the left
# corner of U2's pin 1 mark (the triangle past pin 1's outer end, on its
# axis), between pin 1 and C8
VDD_VIA = (6.0, 5.5)
C12_GND = (6.75, 1.05)      # C12's ground, beside its pad, clear of U2's pads
# The test pad TP1 on top, below-left of U7, on +3V3_BUF's run from U7's
# output pin down to C13: in the open with the upper board lifted off
TP1_LO = (5.67, 4.6)
# C13 upright below U7's ground pin (ground up, +3V3_BUF down), and the
# column at the right: the enable delay R26 and C17, then R14
C13_X, C13_Y = 7.24, 4.7
C13_GND = (C13_X, C13_Y - 0.48)
C13_VDD = (C13_X, C13_Y + 0.48)
COL_X = 8.76
R26_EN = (COL_X - 0.51, 2.75)
R26_5V = (COL_X + 0.51, 2.75)
C17_EN = (COL_X + 0.48 - 0.03, 4.05)
C17_GND = (COL_X - 0.48 - 0.03, 4.05)
C9_VIA = (6.3, 11.1)        # and up again to U3 and its capacitor C9
C9_GND = (5.9, 12.45)
# C8 underneath, below U2's pin 1 past its pin 1 mark: the mark's base,
# stroke included, is at y 6.0 and C8's pads start at 6.17 (0.17 mm on)
C8_X, C8_Y = 5.85, 6.48
C8_VDD = (C8_X + 0.48, C8_Y)
C8_VSS = (C8_X - 0.48, C8_Y)
C8_GND = (4.72, C8_Y)       # its own ground via, beside the ground pad
C10_VDD = (4.55, 22.85)
C18_GND = (7.85, 1.75)
R21_VDD = (4.75, 20.01)     # R21 (on top, beside U4's pin 20) pad 1
R21_NOE = (4.75, 18.99)


def lower_decoupling():
    """U4's capacitor C10 under its supply end, and C18 at socket pin 20.

    C10 is underneath, below RN1, its supply pad up by U4's pin 20 and its
    ground pad by pin 1. Each of those two pins drops through a via of its
    own, just outside U4's end (under the DIP strip's insulator, which
    vias may sit under), into the capacitor's pad on the underside: the
    loop is the two pins, two vias, and under 1 mm of track each side.
    C10's ground also reaches U4's exposed-pad vias through the planes
    from the pin 1 via.

    C18 is on top at the board's pin 1 end, its supply pad 1.8 mm from
    socket pin 20 and its ground pad 0.8 mm from a via into U2's exposed
    pad underneath (and the planes)."""
    W3 = 0.3
    vss = (4.55, 24.35)
    t = [
        ("F.Cu", "+3V3_BUF", [(5.05, 22.35), C10_VDD], W3),
        ("B.Cu", "+3V3_BUF", [C10_VDD, (5.22, 22.82)], W3),
        ("F.Cu", "GND", [(5.05, 23.85), vss], 0.25),
        ("B.Cu", "GND", [vss, (5.22, 23.78)], W3),
        ("F.Cu", "GND", [(8.32, 1.05), C18_GND], W3),
    ]
    v = [("+3V3_BUF",) + C10_VDD, ("GND",) + vss]
    return t, v


def lower():
    tracks, vias = [], []
    for t, v in (u3_comb(), u4_comb(), j5_vias(), corridor(), u4_a_vias(),
                 terminators(), buffer_grounds(), u3_socket(), socket_pulls(), u2_escape(),
                 lower_supply(), lower_decoupling()):
        tracks += t
        vias += v
    return tracks, vias


# Upper board (placement in gen_layout.py UPPER: J3 at the pin 1 end's
# edge, SW1 behind it on top, U5 under SW1, J4 at the far end). U1 (AG32,
# QFN-32) on the underside at (7.0, 9.0), turned 180: its bottom edge
# (pins 16, 15 .. 9 at x 5.6 .. 8.4, pad centres y 10.95) faces J6's
# pin 1 end; its left edge (pins 24 .. 17 at y 7.6 .. 10.4, pad centres
# x 5.05). J6 (underside, same place as J5) has its left
# row's pad centres at x 5.96, outer ends 5.61, position k at
# y = 13.75 + 0.4 k.
UJ6_L = 5.96


def upper_left_bundle():
    """The lines to J6's left row, on the underside, entering their pads
    from outside. Six of the bottom edge's pins fan down and to the left -
    each turning a little deeper and running a little further in than the
    one to its left, so the rightmost (pin 9) is innermost and takes the
    first contact - and the left edge's two run outside them, pin 18 (the
    lower) inside pin 19. 0.235 mm apart.

    The seventh, pin 14, is the drive buffer's enable, and its contact
    (k 13) lies beyond pin 15's and pin 18's (k 10 and k 11): on the
    underside it would have to cross them. It drops instead through a via
    just below its pad, between pin 15's and pin 13's lines (0.125 mm from
    each), runs on top over the bundle, and comes back down beside k 13's
    outer end, where the only line still running is pin 19's to k 14,
    below it. The fabric wants the enable on pin 14 (near the chip enable's
    pin 9; see tools/boardmap.yaml)."""
    order = [  # (pin x, k, net): innermost first
        (8.4, 0, "/BUF_nCE0"), (8.0, 1, "/BUF_A5"), (7.6, 3, "/BUF_A6"),
        (7.2, 4, "/BUF_A7"), (6.8, 6, "/BUF_A8"), (6.0, 10, "/BUF_D2")]
    t = []
    for i, (px, k, net) in enumerate(order):
        x = round(5.45 - 0.235 * i, 4)
        yt = round(13.01 - 0.235 * i, 4)
        t.append(("B.Cu", net, [(px, 10.95), (px, yt), (x, yt), (x, k_y(k)),
                                (UJ6_L, k_y(k))]))
    # The left edge's two, pin 18 (socket 7, k 11) inside pin 19 (k 14).
    # The strap R11 on pin 15's line (BOOT1) is the autorouter's, from the
    # contact pad: it joins a pin, never the middle of a fixed track.
    for i, (py, k, net) in ((6, (10.0, 11, "/BUF_D1")),
                            (7, (9.6, 14, "/BUF_D0"))):
        x = round(5.45 - 0.235 * i, 4)
        t.append(("B.Cu", net, [(5.05, py), (x, py), (x, k_y(k)),
                                (UJ6_L, k_y(k))]))
    # the enable, pin 14, by way of the top
    n = "/D_BUF_NOE"
    t += [("B.Cu", n, [(6.4, 10.95), NOE_DOWN]),
          ("F.Cu", n, [NOE_DOWN, (5.4, 12.6), (5.4, 18.65), NOE_UP]),
          ("B.Cu", n, [NOE_UP, (UJ6_L, k_y(13))])]
    return t, [(n,) + NOE_DOWN, (n,) + NOE_UP]


NOE_DOWN, NOE_UP = (6.4, 11.6), (5.1, k_y(13))
UJ6_R = 9.04


def upper_right_bundle():
    """U1's right edge (pins 8, 7, 5, 3, 1 at y 10.4 .. 7.6, pad centres
    x 8.95) to J6's right row, on the underside, entering from outside:
    out to the right and down the strip beside J6, the lowest pin
    innermost. Pins 6 (VDD), 4 (NRST) and 2 (OSC_IN) sit between them and
    leave through dog-bone vias in a column outside the pads, with the
    I/O passing between at 0.125 mm - legal at the board's 0.1 mm rule.
    VDD's via feeds its capacitor C3 on top; OSC_IN's comes up beside
    the oscillator."""
    t, v = [], []
    for i, (py, k, net) in enumerate(((10.4, 2, "/BUF_A0"), (10.0, 3, "/BUF_A4"),
                                      (9.2, 5, "/BUF_A3"), (8.4, 6, "/BUF_A1"),
                                      (7.6, 7, "/BUF_A2"))):
        x = (9.6, 9.835, 10.155, 10.39, 10.625)[i]
        t.append(("B.Cu", net, [(8.95, py), (x, py), (x, k_y(k)),
                                (UJ6_R, k_y(k))]))
    for (py, vx, net) in ((9.6, 9.7, "+3V3_MCU"), (8.8, 9.75, "/NRST"),
                          (8.0, 9.75, "/HSE_IN")):
        t.append(("B.Cu", net, [(8.95, py), (vx, py)]))
        v.append((net, vx, py))
    return t, v


PIN32_VIA = (9.2, 6.75)
BOOT0_VIA = (7.6, 6.2)
D7_HOP = (11.28, 18.15)     # BUF_D7 up, beyond D3's and D4's contacts
D7_IN = (9.76, k_y(8))      # and down again beside its own


def upper_far_edge():
    """U1's far edge (pins 25-32 at x 5.6 .. 8.4, pad centres y 7.05)
    faces the pin 1 end. Its five data lines, all for J6's right row, wrap
    round the corner underneath, the rightmost innermost, and run down
    outside the right edge's bundle, 0.21 mm apart: BUF_D3 (pin 31) to
    k 9, BUF_D4 (29) to k 10, BUF_D7 (28), BUF_D6 (27) to k 13 and BUF_D5
    (26) to k 14. BUF_D7's contact, k 8, comes before D3's and D4's: it
    runs on past them, rises through a via once they have turned in, and
    comes back on top to a via beside k 8's outer end (D6 and D5 step
    outward round its via). BOOT0 (pin 30) and the supply pin 32 leave
    through vias of their own, BOOT0's on top up to J3, pin 32's
    into its capacitor C5 on top."""
    P = 0.21
    t, v = [], []
    strip = {"/BUF_D3": 10.86, "/BUF_D4": 11.07, "/BUF_D7": 11.28,
             "/BUF_D6": 11.49, "/BUF_D5": 11.7}
    k = {"/BUF_D7": 8, "/BUF_D3": 9, "/BUF_D4": 10, "/BUF_D6": 13,
         "/BUF_D5": 14}
    turn = {"/BUF_D3": 6.3, "/BUF_D4": 5.75, "/BUF_D7": 5.54,
            "/BUF_D6": 5.33, "/BUF_D5": 5.12}
    pin = {"/BUF_D3": 8.0, "/BUF_D4": 7.2, "/BUF_D7": 6.8, "/BUF_D6": 6.4,
           "/BUF_D5": 6.0}
    for net in ("/BUF_D3", "/BUF_D4", "/BUF_D7", "/BUF_D6", "/BUF_D5"):
        px, ty, x = pin[net], turn[net], strip[net]
        pts = [(px, 7.05), (px, round(ty + P, 4)), (round(px + P, 4), ty),
               (round(x - P, 4), ty), (x, round(ty + P, 4))]
        if net == "/BUF_D7":
            pts += [(x, round(D7_HOP[1] - 0.01, 4)), D7_HOP]
        elif net in ("/BUF_D6", "/BUF_D5"):
            # step outward round D7's via, D5 first
            dx, y0 = 0.2, (17.5 if net == "/BUF_D6" else 17.29)
            ky = k_y(k[net])
            pts += [(x, y0), (round(x + dx, 4), round(y0 + dx, 4)),
                    (round(x + dx, 4), round(ky - 0.2, 4)),
                    (round(x + dx - 0.2, 4), ky), (UJ6_R, ky)]
        else:
            pts += [(x, k_y(k[net])), (UJ6_R, k_y(k[net]))]
        t.append(("B.Cu", net, pts))
    n = "/BUF_D7"
    t += [("F.Cu", n, [D7_HOP, (10.96, 18.15), D7_IN]),
          ("B.Cu", n, [D7_IN, (UJ6_R, k_y(8))])]
    v += [(n,) + D7_HOP, (n,) + D7_IN]
    t += [("B.Cu", "/BOOT0", [(7.6, 7.05), BOOT0_VIA])]
    v.append(("/BOOT0",) + BOOT0_VIA)
    # pin 32 (VDD) to a via beside the corner, inside D3's corner, and up
    # into C5 on top
    t.append(("B.Cu", "+3V3_MCU", [(8.4, 7.05), PIN32_VIA], 0.2))
    t.append(("F.Cu", "+3V3_MCU", [PIN32_VIA, (9.2, 7.08), (8.88, 7.4)],
              0.25))
    v.append(("+3V3_MCU",) + PIN32_VIA)
    return t, v


# U1's exposed-pad vias: clear of the oscillator's pads on top
U1_EP = [(6.25, 8.1), (8.0, 7.95), (8.0, 9.26), (6.3, 10.1)]
C3_GND = (8.95, 11.85)
U5_GND = (7.3, 2.6)


def upper_usb():
    """USB, on top all the way from U1 to the receptacle. The lines leave
    U1's left edge through dog-bone vias, D+ (the upper pin) outside D-,
    run down the left side as a pair to the clamp U6's top row (D+ left,
    D- right), and leave its bottom row the same way round, D- passing
    above D+ to the receptacle. Its pads interleave D-, D+, D-, D+ (B7,
    A6, A7, B6): D- comes down into A7, D+ into A6, D+'s two pads are
    joined under the row on top, and D-'s two through a via each and a
    link underneath, a ground via just below each."""
    t = [("B.Cu", "/UART_TX", [(5.05, 9.2), (3.35, 9.2), (2.94, 8.79),
                               (2.94, 6.2)]),
         ("B.Cu", "/UART_RX", [(5.05, 8.8), (3.55, 8.8), (3.15, 8.4),
                               (3.15, 6.2)]),
         ("B.Cu", "/USB_DP", [(5.05, 8.0), (4.45, 8.0), DP_VIA]),
         ("B.Cu", "/USB_DM", [(5.05, 8.4), DM_VIA]),
         ("F.Cu", "/USB_DP", [DP_VIA, (3.2, 8.15), (3.2, 13.4), (2.4, 14.2),
                              (2.4, 14.863)]),
         ("F.Cu", "/USB_DM", [DM_VIA, (3.45, 9.25), (3.45, 13.4), (4.3, 14.25),
                              (4.3, 14.863)]),
         ("F.Cu", "/USB_DM_CON", [(4.3, 17.137), (4.3, 19.0), (4.7, 19.4),
                                  (6.85, 19.4), (7.25, 19.8), (7.25, 20.9)]),
         ("F.Cu", "/USB_DP_CON", [(2.4, 17.137), (2.4, 17.7), (1.95, 18.15),
                                  (1.95, 19.4), (2.35, 19.8), (6.35, 19.8),
                                  (6.75, 20.2), (6.75, 20.9)]),
         ("F.Cu", "/USB_DP_CON", [(6.75, 20.9), (6.75, 22.82), (7.75, 22.82),
                                  (7.75, 20.9)]),
         ("F.Cu", "/USB_DM_CON", [(6.25, 20.9), DM_LINK[0]]),
         ("F.Cu", "/USB_DM_CON", [(7.25, 20.9), DM_LINK[1]]),
         ("B.Cu", "/USB_DM_CON", list(DM_LINK)),
         # the clamp's VBUS capacitor C19 underneath, through a via
         # under its pin 5
         ("F.Cu", "+5V_USB", [(U6_X, 17.137), VBUS_U6], 0.25),
         ("B.Cu", "+5V_USB", [VBUS_U6, (2.6, 18.87)], 0.25),
         # and its ground straight into the receptacle's shell slot, a
         # plated hole into both planes (no room for a via of its own)
         ("B.Cu", "GND", [(2.6, 19.83), (2.675, 20.55)], 0.3),
         ("F.Cu", "GND", [(U6_X, 14.863), U6_GND], 0.3),
         ("F.Cu", "GND", [(U6_X, 14.863), U6_GND2], 0.4),
         # the receptacle's ground pads into its shell's plated slots
         ("F.Cu", "GND", [(3.8, 20.9), (3.3, 21.4), (2.675, 21.4)], 0.3),
         ("F.Cu", "GND", [(10.2, 20.9), (10.7, 21.4), (11.325, 21.4)], 0.3),
         # the CC lines to their resistors underneath
         ("F.Cu", "/USB_CC1", [(5.75, 20.9), CC1_VIA]),
         ("B.Cu", "/USB_CC1", [CC1_VIA, (5.0, 22.89)]),
         ("F.Cu", "/USB_CC2", [(8.75, 20.9), CC2_VIA]),
         ("B.Cu", "/USB_CC2", [CC2_VIA, (8.4, 23.49)]),
         ]
    v = [("/USB_DP",) + DP_VIA, ("/USB_DM",) + DM_VIA,
         ("/USB_DM_CON",) + DM_LINK[0], ("/USB_DM_CON",) + DM_LINK[1],
         # a ground via under each of D-'s, for its return between planes
         ("GND", 6.25, 22.42), ("GND", 7.25, 22.42),
         ("+5V_USB",) + VBUS_U6, ("GND",) + U6_GND, ("GND",) + U6_GND2,
         ("/USB_CC1",) + CC1_VIA,
         ("/USB_CC2",) + CC2_VIA]
    return t, v


DM_LINK = ((6.25, 21.85), (7.25, 21.85))
# U6 (the USB clamp) at x 3.35, its pins 2 and 5 on that line: 0.15 mm in
# from where it was, so its courtyard ends inside the board edge and clears
# the assembly panel's mouse bites beside it (tools/gen_panel.py)
U6_X = 3.35
VBUS_U6 = (U6_X, 18.25)
U6_GND = (U6_X, 13.95)      # U6's ground, above pin 2 between the pair
U6_GND2 = (U6_X, 16.0)      # and under its body, between its pad rows
CC1_VIA = (5.55, 22.25)
CC2_VIA = (8.4, 22.7)
DP_VIA = (3.75, 7.6)
DM_VIA = (4.3, 8.4)


def j6_vias():
    """A via for each of J6's ground and socket-power contacts, in the lane
    between its rows (every signal enters its pad from outside, so the lane
    is otherwise empty), joined by a stub along the pad; adjacent ground
    contacts share one. And one more, for BUF_D2's BOOT1 strap R11: the
    strap sits on top over the lane, on a via from k 10's inner end."""
    L, R = 6.7, 8.24
    t, v = [], []
    plan = [  # (row x, contacts, via)
        (UJ6_R, (0, 1), (R, 13.95)),
        (UJ6_L, (2,), (L, k_y(2))), (UJ6_R, (4,), (R, k_y(4))),
        (UJ6_L, (5,), (L, k_y(5))), (UJ6_L, (7,), (L, k_y(7))),
        (UJ6_L, (8, 9), (7.4, 17.15)), (UJ6_R, (11, 12), (R, 18.35)),
        (UJ6_L, (12,), (L, k_y(12))),
    ]
    for (px, ks, via) in plan:
        net = "+5V_SKT" if ks == (0, 1) else "GND"
        for k in ks:
            t.append(("B.Cu", net, [(px, k_y(k)), via],
                      0.2 if net == "+5V_SKT" else W))
        v.append((net,) + via)
    t += [("B.Cu", "/BUF_D2", [(UJ6_L, k_y(10)), D2_STRAP]),
          ("F.Cu", "/BUF_D2", [D2_STRAP, (7.45, 18.76)]),
          ("F.Cu", "GND", [(7.45, 17.74), (7.4, 17.15)], 0.2),
          ("F.Cu", "+5V_SKT", [(R, 13.95), (9.25, 13.85)], 0.3)]
    v.append(("/BUF_D2",) + D2_STRAP)
    return t, v


D2_STRAP = (6.7, 17.75)


# J3, the SOICbite, at the pin 1 end's edge (gen_layout.py UPPER, y -4.0):
# its pads' centres on y J3_Y, 1.27 mm apart, and the clip's five 0.7 mm
# holes on J3_HOLES_Y between them, so every line leaves its pad straight
# in, between two holes.
J3_Y = -2.869
J3_HOLES_Y = -1.714
J3_X = {1: 5.095, 2: 6.365, 4: 8.905, 5: 8.905, 6: 7.635, 8: 5.095}


def upper_prog():
    """J3's lines out to the board. Seen from above, the pads run 3V3,
    BOOT0, -, GND on top and NRST, -, TX, RX underneath, left to right,
    while 3V3 and NRST are wanted at the right (U5 and the NRST network)
    and TX and RX at the left (the AG32's UART edge, the switch's
    resistors): each pair crosses the other and BOOT0, which runs straight
    down the middle on top to its via at the AG32. So, in the strip
    between the holes and the parts:
      - TX and RX, underneath, turn left just past the holes, the one
        nearer the holes (TX) outside, and come up through vias into the
        switch's series resistors R24 and R25 on top, and on down
        underneath into their pull-downs R18 and R17;
      - NRST, underneath at the far left, leaves its pad sideways (outside
        the hole row) through a via to the top, comes down outside 3V3's
        pad and back through a via below TX and RX, and runs right
        underneath to a via that takes it up to its network;
      - 3V3, on top, necks to 0.15 mm between the holes, comes down past
        TX and RX and through a via below NRST, and runs right underneath
        into U5's output pin.
    """
    W3, NECK = 0.25, 0.15
    yt, yr, yn, y3 = -1.11, -0.9, -0.5, -0.05
    t = [
        ("B.Cu", "/UART_TX", [(J3_X[6], J3_Y), (J3_X[6], yt),
                              (TX_J3[0], yt), TX_J3]),
        ("B.Cu", "/UART_RX", [(J3_X[5], J3_Y), (J3_X[5], yr),
                              (RX_J3[0], yr), RX_J3]),
        # up to the switch's series resistors on top
        ("F.Cu", "/UART_TX", [TX_J3, (R24_X, TX_J3[1] + R24_X - TX_J3[0]),
                              (R24_X, 0.49)]),
        ("F.Cu", "/UART_RX", [RX_J3, (R25_X, RX_J3[1] + R25_X - RX_J3[0]),
                              (R25_X, 0.49)]),
        # and on underneath to their pull-downs R18 (RX, above) and R17
        # (TX), which share a ground via between their ground pads, and
        # past the ferrite and C1 to the AG32's UART edge (upper_usb): TX
        # down the board's left edge, RX between the pull-downs and the
        # ferrite and between C1's pads; C1's ground via between the two
        # below it
        ("B.Cu", "/UART_TX", [TX_J3, (TX_J3[0], 0.6), (1.95, 1.0),
                              (1.95, 3.11), R17_TX]),
        ("B.Cu", "/UART_TX", [R17_TX, (2.5, 3.66), (2.5, 5.76),
                              (2.94, 6.2)]),
        ("B.Cu", "/UART_RX", [RX_J3, (RX_J3[0], 0.6), R18_RX]),
        ("B.Cu", "/UART_RX", [(RX_J3[0], 0.6), (4.05, 1.2), (4.05, 5.6),
                              (3.45, 6.2), (3.15, 6.2)]),
        ("B.Cu", "GND", [(3.46, 1.6), PULL_GND, (3.46, 3.6)], 0.3),
        ("B.Cu", "GND", [(3.245, 5.0), C1_GND], 0.3),
        ("B.Cu", "/NRST", [(J3_X[8], J3_Y), NRST_J3[0]]),
        ("F.Cu", "/NRST", [NRST_J3[0], (NRST_J3[0][0], -1.1), NRST_J3[1]]),
        ("B.Cu", "/NRST", [NRST_J3[1], NRST_J3[2]]),
        ("F.Cu", "+3V3", [(J3_X[1], J3_Y), (J3_X[1], -1.05)], NECK),
        ("F.Cu", "+3V3", [(J3_X[1], -1.05), V33_J3], W3),
        ("B.Cu", "+3V3", [V33_J3, (8.35, y3), (8.35, 1.462)], W3),
        ("F.Cu", "/BOOT0", [(J3_X[2], J3_Y), (J3_X[2], 3.3), (7.6, 4.535),
                            BOOT0_VIA]),
    ]
    assert NRST_J3[1][1] == yn and NRST_J3[2][1] == yn and V33_J3[1] == y3
    v = [("GND",) + PULL_GND, ("GND",) + C1_GND]
    v += [("/NRST",) + p for p in NRST_J3] + [("+3V3",) + V33_J3,
                                             ("/UART_TX",) + TX_J3,
                                             ("/UART_RX",) + RX_J3]
    return t, v


NRST_J3 = ((3.7, J3_Y), (4.3, -0.5), (10.0, -0.5))
TX_J3 = (2.35, -0.45)       # under R24's TX pad, on top
RX_J3 = (3.45, -0.3)        # under R25's RX pad
R24_X, R25_X = 2.75, 3.85   # gen_layout.py
R18_RX = (2.44, 1.6)        # R18 at (2.95, 1.6), R17 at (2.95, 3.6)
R17_TX = (2.44, 3.6)
PULL_GND = (3.46, 2.6)      # between their ground pads, clear of SW1's on top
C1_GND = (3.245, 5.82)      # below C1's ground pad, clear of SW1's pad 2
V33_J3 = (J3_X[1], -0.05)


def upper():
    """Upper board hand copper: the bundles; U1's ground pin 17 (left
    edge, y 10.4, next to the corner) across into the exposed pad along
    the 0.45 mm between pin 18 and the corner pin 16; and the corner
    supply pin 16, boxed in by the bundle, out through a via of its own
    in the pocket the bundle leaves (its capacitor C4 is on top, over the
    same pocket, with a via in its ground pad: the only place a ground
    via clears the bundle underneath and the USB pair on top)."""
    t, v = upper_left_bundle()
    for rt, rv in (upper_right_bundle(), upper_far_edge(), upper_usb(),
                   j6_vias(), upper_mcu_supply(), upper_test_pads(),
                   upper_prog()):
        t += rt
        v += rv
    t += [("B.Cu", "GND", [(5.05, 10.35), (5.55, 10.35), (5.8, 10.2)]),
          ("B.Cu", "+3V3_MCU", [(5.6, 10.95), (5.6, 11.25), VDD16_VIA]),
          # C4's ground via, in its pad, also takes pin 17 underneath:
          # the capacitor sits across the two adjacent pins 16 and 17
          ("B.Cu", "GND", [C4_GND, (4.8, 10.4)], 0.25),
          ("F.Cu", "GND", [(9.27, 11.25), C3_GND], 0.25),
          # C11 (NRST) and C15 (1 uF) share C3's ground via, either side
          ("F.Cu", "GND", [(9.52, 12.3), (9.07, 11.85), C3_GND], 0.25),
          ("F.Cu", "GND", [(8.37, 12.3), (8.82, 11.85), C3_GND], 0.25),
          ("F.Cu", "GND", [(7.92, 7.4), U1_EP[1]], 0.25),
          ("F.Cu", "GND", [(8.75, 9.86), (8.3, 9.46), U1_EP[2]], 0.25),
          ("B.Cu", "GND", [(7.3, 3.7375), U5_GND], 0.3)]
    v += [("GND",) + C3_GND, ("GND",) + U5_GND, ("GND",) + C4_GND]
    v += [("GND", x, y) for (x, y) in U1_EP]
    v += [("+3V3_MCU",) + VDD16_VIA]
    s, sv = upper_supply()
    return t + s, v + sv


VDD16_VIA = (5.05, 11.25)
C4_GND = (4.45, 10.73)      # C4 at (4.45, 11.21) turned 90: ground pad up
SPINE_X = 7.41              # X1's supply column, between R10 and C5


def upper_mcu_supply():
    """+3V3_MCU on top, as one tree from the ferrite. FB1's output comes up
    under SW1's left half and down into the branch to the 10 uF C14
    (upper_supply), so the ferrite's current reaches the bulk capacitor
    first. From there the branch runs right, round the top of the USB
    pair's vias, past X1's capacitor C16 to X1's VDD pad (pin 4), where
    the spine runs straight down X1's left pin column, through its OE pad
    (pin 1), to C15 (1 uF) and on to the test pad TP3. Off the spine:
      - at pin 4, up the 0.46 mm between R10 and C5 and right above C5,
        0.2 mm wide, to pin 32's via and C5;
      - under X1, one straight bar: right to pin 6's via and C3 (and R9,
        NRST's pull-up), left to pin 16's via and C4.
    Every supply pin's via reaches its own capacitor before the bar or the
    spine does; FB1's input side (+3V3) never comes near."""
    W2, W = 0.2, 0.25
    x = SPINE_X
    t = [
        ("F.Cu", "+3V3_MCU", [PIN32_VIA, (7.66, 6.75), (x, 7.0), (x, 8.78)],
         W2),
        ("F.Cu", "+3V3_MCU", [(x, 8.78), (x, 9.86)], W),
        ("F.Cu", "+3V3_MCU", [(x, 9.86), (x, 10.6)], W),
        # into X1's two pad centres from the column beside them (the
        # autorouter sees a pin joined only where a track ends in it)
        ("F.Cu", "+3V3_MCU", [(x, 8.78), (7.25, 8.66)], W),
        ("F.Cu", "+3V3_MCU", [(x, 9.86), (7.25, 9.86)], W),
        ("F.Cu", "+3V3_MCU", [(x, 10.6), (x, 12.3)], W),
        ("F.Cu", "+3V3_MCU", [(x, 12.3), TP3_UP], W),
        ("F.Cu", "+3V3_MCU", [(x, 8.78), (5.75, 8.78), (4.07, 7.1),
                              (2.9, 7.1), (2.5, 7.5), (2.5, 7.975)], W),
        ("F.Cu", "+3V3_MCU", [(x, 10.6), (10.23, 10.6)], W),
        ("F.Cu", "+3V3_MCU", [(x, 10.6), (5.7, 10.6), VDD16_VIA], W),
        ("F.Cu", "+3V3_MCU", [VDD16_VIA, (4.61, 11.69), (4.45, 11.69)], W),
        ("F.Cu", "+3V3_MCU", [(9.7, 9.6), (10.23, 10.13), (10.23, 10.6)],
         W),
        ("F.Cu", "+3V3_MCU", [(10.23, 10.6), (10.23, 11.25)], W),
        ("F.Cu", "+3V3_MCU", [(10.23, 10.13), (11.0, 10.13)], W),
    ]
    return t, []


TP3_UP = (SPINE_X, 15.15)   # +3V3_MCU, between J6's lane vias
TP2_UP = (9.25, 18.15)      # +5V_USB, behind the receptacle


def upper_test_pads():
    """TP2 on +5V_USB: on the straight run from the receptacle's VBUS
    pads to the OR-ing diode D3, between D3 and the receptacle. TP3 is on
    the +3V3_MCU spine (upper_mcu_supply). Both on top, which stays
    reachable with the module seated. With that run fixed, VBUS's way
    underneath is fixed too: from a via beside the receptacle's pad row,
    left to the clamp's via (and its capacitor C19, 1.56 mm along the edge
    from the panel's mid-board tab) and down to the damper R27. The receptacle's two VBUS pairs join on top, round
    under its pad row, as the autorouter once had them."""
    W = 0.3
    t = [("F.Cu", "+5V_USB", [(9.25, 16.05), TP2_UP, (9.25, VBUS_J4[1])], W),
         ("F.Cu", "+5V_USB", [(9.25, VBUS_J4[1]), (9.25, 20.9)], W),
         ("F.Cu", "+5V_USB", [(9.25, VBUS_J4[1]), VBUS_J4], W),
         ("B.Cu", "+5V_USB", [VBUS_J4, (3.98, VBUS_J4[1]), (3.35, 19.36),
                              VBUS_U6], W),
         ("B.Cu", "+5V_USB", [VBUS_J4, (8.13, 21.66), (9.45, 22.98)], W),
         # the receptacle's other VBUS pair (A4/B9) round under its pad
         # row on top, below the D- link and the D+ loop, to A9/B4
         ("F.Cu", "+5V_USB", [(4.6, 20.9), (4.6, 21.5208), (4.8602, 21.781),
                              (4.8602, 22.4018), (5.6756, 23.2172),
                              (8.6133, 23.2172), (9.0595, 22.771),
                              (9.0595, 21.7776), (9.3063, 21.5308),
                              (9.3063, 20.9), (9.4, 20.9)], W)]
    return t, [("+5V_USB",) + VBUS_J4]


VBUS_J4 = (8.13, 19.99)


def upper_supply():
    """The 5 V from the OR-ing diodes D2 and D3 (on top over J6's right
    row) to the regulator U5 (underneath at the pin 1 end): up the board's
    right edge on top, outside everything there, and through a via beside
    SW1 into a track along the top of the far edge's lines underneath, to
    U5's input (1) and enable (3) pins and on to its input capacitor C1,
    below the ferrite. U5's output capacitor C2 sits right beside its
    output pin, its ground via between it and U5's input pin, and the
    +3V3 goes on from it through a via up to SW1's two commons. The
    ferrite FB1 is on U5's other side, 2.48 mm in from the left edge (the
    panel's J3-end tab: gen_panel.py): its input off the +3V3 that runs
    above U5 to J3's pin 1 (upper_prog), its output through a via between
    it and U5's enable pin and down on top, in the clear strip between
    SW1's left pads and BOOT0, into the +3V3_MCU tree at the branch to
    the 10 uF C14 (upper_mcu_supply). FB1's two sides never meet: +3V3
    above it, +3V3_MCU below and on top."""
    W = 0.3
    t = [
        ("F.Cu", "/+5V", [(11.35, 16.05), (11.95, 15.45), (11.95, 13.85),
                          (11.35, 13.85)], W),
        ("F.Cu", "/+5V", [(11.95, 13.85), (11.95, 4.9), V5_DOWN], W),
        ("B.Cu", "/+5V", [V5_DOWN, (11.6, 4.75), (8.55, 4.75), (8.35, 4.55),
                          (8.35, 3.738)], W),
        ("B.Cu", "/+5V", [(8.35, 4.55), (8.15, 4.75), (6.65, 4.75),
                          (6.45, 4.55), (6.45, 3.738)], W),
        ("B.Cu", "/+5V", [(6.65, 4.75), (C1_5V[0] + 0.25, 4.75), C1_5V], W),
        # +3V3: U5's output straight into C2 beside it, and on through a
        # via in line with them up to SW1's two commons
        ("B.Cu", "+3V3", [(8.35, 1.462), (9.0, 1.462), (9.363, 1.825),
                          (9.85, 1.825)], 0.25),
        ("B.Cu", "+3V3", [(9.85, 1.825), V33_SW], 0.25),
        # C2's ground into a via beside it, 1.9 mm from U5's
        ("B.Cu", "GND", [(9.85, 3.375), C2_GND], 0.3),
        ("F.Cu", "+3V3", [V33_SW, (10.81, 3.815)], 0.25),
        ("F.Cu", "+3V3", [(10.81, 3.815), (10.81, 5.085)], 0.2),
        # the ferrite's input, off J3's +3V3 where it comes down
        ("B.Cu", "+3V3", [V33_J3, (V33_J3[0], 1.4875), FB1_IN], 0.25),
        # and its output, up beside it and down on top into the tree
        ("B.Cu", "+3V3_MCU", [FB1_OUT, MCU_UP], 0.25),
        ("F.Cu", "+3V3_MCU", [MCU_UP, (C14_JOIN[0], 5.13), C14_JOIN], 0.25),
    ]
    return t, [("/+5V",) + V5_DOWN, ("+3V3_MCU",) + MCU_UP,
               ("+3V3",) + V33_SW, ("GND",) + C2_GND]


V5_DOWN = (11.95, 4.4)
FB1_IN = (4.77, 1.8125)     # FB1 at (4.77, 2.6) turned 270: input pad up
FB1_OUT = (4.77, 3.3875)
C1_5V = (4.795, 5.0)        # C1 at (4.02, 5.0) turned 180: 5 V pad right
MCU_UP = (5.6, 3.6)         # between FB1's output pad and U5's pin 3
C14_JOIN = (4.07, 7.1)      # upper_mcu_supply's bend toward C14
V33_SW = (10.81, 1.825)     # in line with C2's pad and SW1's commons
C2_GND = (8.92, 3.55)       # between U5's input pin and C2's ground pad


# ---------------------------------------------------------------------------
def _seg_d(px, py, ax, ay, bx, by):
    dx, dy = bx - ax, by - ay
    ll = dx * dx + dy * dy
    u = 0 if ll == 0 else max(0, min(1, ((px - ax) * dx + (py - ay) * dy) / ll))
    return math.hypot(px - ax - u * dx, py - ay - u * dy)


def _seg_seg(a, b):
    (ax, ay, bx, by), (cx, cy, dx, dy) = a, b
    # segments that cross have distance 0
    def side(px, py, qx, qy, rx, ry):
        return (qx - px) * (ry - py) - (qy - py) * (rx - px)
    d1, d2 = side(ax, ay, bx, by, cx, cy), side(ax, ay, bx, by, dx, dy)
    d3, d4 = side(cx, cy, dx, dy, ax, ay), side(cx, cy, dx, dy, bx, by)
    if d1 * d2 < 0 and d3 * d4 < 0:
        return 0.0
    return min(_seg_d(ax, ay, cx, cy, dx, dy), _seg_d(bx, by, cx, cy, dx, dy),
               _seg_d(cx, cy, ax, ay, bx, by), _seg_d(dx, dy, ax, ay, bx, by))


def width(route, default=W):
    """A route is (layer, net, points) at the signal width, or (layer, net,
    points, width)."""
    return route[3] if len(route) > 3 else default


def check(tracks, vias):
    """Every via against every other net's track (vias go through every
    layer), and every pair of other-net tracks on one layer."""
    segs = [(r[0], r[1], (*a, *b), width(r)) for r in tracks
            for a, b in zip(r[2], r[2][1:])]
    out = []
    for (vn, vx, vy) in vias:
        for (L, n, s, w) in segs:
            if n != vn:
                gap = _seg_d(vx, vy, *s) - VIA_D / 2 - w / 2
                if gap < CLR + MARGIN - 1e-6:
                    out.append(f"via {vn} ({vx}, {vy}) is {gap:.3f} mm from "
                               f"{n} on {L}")
    for i, (vn, vx, vy) in enumerate(vias):
        for (wn, wx, wy) in vias[i + 1:]:
            if wn != vn and math.hypot(vx - wx, vy - wy) - VIA_D < CLR + MARGIN - 1e-6:
                out.append(f"vias {vn} and {wn} too close")
    for i, (L, n, s, w) in enumerate(segs):
        for (M, m, t, v) in segs[i + 1:]:
            if L == M and n != m:
                gap = _seg_seg(s, t) - (w + v) / 2
                if gap < CLR + MARGIN - 1e-6:
                    out.append(f"{n} and {m} on {L}: {gap:.3f} mm")
    return out


if __name__ == "__main__":
    for name, (t, v) in (("upper", upper()), ("lower", lower())):
        p = check(t, v)
        print(f"{name}: " + ("\n".join(p) if p else
                             f"clear: {len(t)} tracks, {len(v)} vias"))
