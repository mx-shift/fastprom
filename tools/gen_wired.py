#!/usr/bin/env python3
"""Draw the stack schematics the way they would be drawn by hand.

Every signal connection between parts is a drawn wire. Net names appear as
local labels sitting on a wire that is already connected. Rails - GND,
+3V3, +3V3_MCU, +5V_SKT - use power symbols with their names visible.

Connectors are the exception, because they are drawn as the part is: J1 as
the DIP it plugs into, J5 and J6 as the two rows their pads sit in, J3 and
J4 in their pad order. Their contacts then come in whatever order the board
needs, and where wiring them would tangle, each contact ends in a short
stub with a label (or a rail symbol) and the wiring it belongs to carries
the same label. tools/grade_schematic.py rebuilds connectivity from geometry
and fails a sheet on any label-made connection that is not such a connector
stub, and tools/diff_netlist.py proves the result is electrically identical
to the reference netlist.

Two layout ideas do the rest of the work:

  Pins are arranged so that the parts a bus connects line up row for row,
  so buses are straight wires rather than routed bundles.

  Wherever a resistor or capacitor has to tap a line, the rows either side
  are spaced far enough apart that the part, its rail symbol and its text
  fit between them. A tap never has to cross a neighbouring line.
"""
import argparse
import os
import re
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)

import yaml                                     # noqa: E402
import schlib as S                              # noqa: E402
from notice import HW_NOTICE                     # noqa: E402
from gen_kicad import (stack_contacts, AG32_PACKAGES, SW_DIP_FP,  # noqa: E402
                       USB_C_FP_STACK, skt_net, buf_net, socket_names,
                       pin_table, partdb_part, PROJECT_TEMPLATE)

ROOT = os.path.dirname(HERE)
HW = os.path.join(ROOT, "hw")
# The board revision, for the title blocks: hw/REVISION, the one place it
# is set (the layout's silkscreen and the release tag read it too).
REV = open(os.path.join(HW, "REVISION")).read().strip()

P = S.PITCH
R_FP = "fastprom:R_0402_1005Metric"
GAP = 8          # spacer rows before a tapped line: part, rail symbol and rail name fit between

# The AG32's dedicated and fixed-function pins, and the supply pins drawn on
# the top (supplies) and bottom (grounds) of its symbol.
AG32_FIXED = {
    "QFN-32": {"NRST": "4", "JTMS": "24", "JTCK": "25", "BOOT0": "30",
               "BOOT1": "15", "TX": "20", "RX": "21", "DP": "23", "DM": "22",
               "top": (("16", "VDD33"), ("32", "VDD33"), ("6", "VDDA33")),
               "bot": (("17", "GND"), ("33", "EP"))},
}
UART_GAP = 16    # rows between the UART lines: R24, the image-select switch and R25 between them,
                 # each on a pitch of wire


def spacer(side):
    return (None, "", "passive", side)


class Rails:
    """Hands out uniquely-referenced rail symbols."""

    def __init__(self, sh):
        self.sh = sh
        self.n = 0
        self.syms = {}

    def sym(self, name, up):
        if (name, up) not in self.syms:
            self.syms[(name, up)] = S.PowerSym(name, up=up)
        return self.syms[(name, up)]

    def at(self, name, pt, up=True):
        """An upright rail symbol at pt: a supply pointing up from the top
        end of its wire, ground pointing down from the bottom end. Never
        sideways (tools/grade_schematic.py: rail_sideways). On a part's pin
        it stands a pitch off on a short lead: pins meet through a visible
        wire, never pin on pin (pins_on_pins)."""
        pt = (round(pt[0], 2), round(pt[1], 2))
        if pt in self.sh.pin_points:
            end = (pt[0], round(pt[1] + (-P if up else P), 2))
            self.sh.wire(pt, end)
            pt = end
        self.n += 1
        self.sh.place(self.sym(name, up), f"#PWR{self.n:03d}", at=pt)


STUB = 20.32     # a connector contact's stub: room for its label over it,
                 # and a clear gap between the label and the rail ties at the pins


def pin_stub(sh, pt, side, net, length=STUB):
    """A short wire out of a pin that points to side ("L" or "R"), ending in
    a label named net, written over the stub. This is how a connector drawn
    in its physical contact order joins the wiring it belongs to
    (tools/grade_schematic.py accepts a label join only from such a stub).
    A connector's rail pins are not stubbed: they are tied, per rail, into
    one upright rail symbol. Returns the stub's far end."""
    d = -1 if side == "L" else 1
    end = (pt[0] + d * length, pt[1])
    sh.wire(pt, end)
    sh.name(net, end, rot=0 if side == "L" else 180)
    return end


def hang(sh, rails, sym, ref, value, line_pt, down, rail, fp=R_FP,
         dnp=False, junction=True):
    """A vertical part tapping a horizontal line, rail at its far end.

    The part sits one pitch off the line on a short wire, so its pin and the
    line never share a point and the tap is an unambiguous T with a dot. Pass
    junction=False when the line ends at the tap rather than passing through.
    Near and far pins are found from geometry, so an LED - whose pin 1 is the
    cathode and sits at the bottom - hangs the right way up.
    """
    x, y = line_pt
    centre = (x, y + P + 5.08) if down else (x, y - P - 5.08)
    pins = sh.place(sym, ref, value, centre, fp, dnp=dnp)
    ordered = sorted(pins.values(), key=lambda pt: pt[1])
    near, far = (ordered[0], ordered[-1]) if down else (ordered[-1], ordered[0])
    sh.wire((x, y), near)
    if junction:
        sh.junction((x, y))
    if rail:
        rails.at(rail, far, up=not down)
    return pins


def test_points(sh, rails, pads, at, pitch=20.32):
    """Bare test pads in a row, each under the rail it probes: (ref, rail)."""
    tp = S.test_point()
    for i, (ref, rail) in enumerate(pads):
        pins = sh.place(tp, ref, "TestPoint", (at[0] + i * pitch, at[1]),
                        "fastprom:TestPoint_Pad_D1.0mm")
        rails.at(rail, pins["1"], up=True)


def tie_top(sh, rails, ends, rail):
    """Tie supply pins along a symbol's top edge to one rail symbol."""
    ends = sorted(ends)
    if len(ends) == 1:
        rails.at(rail, ends[0], up=True)
        return
    y = ends[0][1] - P
    x0 = ends[0][0]
    sh.wire(ends[0], (x0, y - P))
    for e in ends[1:]:
        sh.wire(e, (e[0], y))
    sh.wire((x0, y), (ends[-1][0], y))
    sh.junction((x0, y))
    for e in ends[1:-1]:
        sh.junction((e[0], y))
    rails.at(rail, (x0, y - P), up=True)


def tie_bottom(sh, rails, ends, rail="GND"):
    """Tie ground pins along a symbol's bottom edge to one ground symbol."""
    ends = sorted(ends)
    if len(ends) == 1:
        rails.at(rail, ends[0], up=False)
        return
    y = ends[0][1] + P
    x0 = ends[0][0]
    sh.wire(ends[0], (x0, y + P))
    for e in ends[1:]:
        sh.wire(e, (e[0], y))
    sh.wire((x0, y), (ends[-1][0], y))
    sh.junction((x0, y))
    for e in ends[1:-1]:
        sh.junction((e[0], y))
    rails.at(rail, (x0, y + P), up=False)


def tie_column(sh, rails, pts, x_t, rail, down):
    """Stub a column of side pins out to a vertical wire with one rail symbol."""
    pts = sorted(pts, key=lambda pt: pt[1])
    for pt in pts:
        sh.wire(pt, (x_t, pt[1]))
    if down:
        end = (x_t, pts[-1][1] + P)
        sh.wire((x_t, pts[0][1]), end)
        for pt in pts[1:]:
            sh.junction((x_t, pt[1]))
        rails.at(rail, end, up=False)
    else:
        end = (x_t, pts[0][1] - P)
        sh.wire((x_t, pts[-1][1]), end)
        for pt in pts[:-1]:
            sh.junction((x_t, pt[1]))
        rails.at(rail, end, up=True)
    return end


def connector_rails(sh, rails, groups, side, top=None, bottom=None):
    """Tie a connector column's rail pins, one upright symbol per rail.

    groups: [(rail, [pin points])], every pin on the side given. Each rail
    gets a vertical tie close to the pins, grounds nearest (2.54 mm out),
    then supplies (5.08 mm, 7.62 mm, ...). A ground's tie runs from its top
    pin down past the column's last row (bottom) to its symbol; a supply's
    from its bottom pin up past the column's first row (top) to its symbol.
    Without top or bottom, a tie ends one pitch past its own last pin: for
    when nothing beyond that pin (only nearer ties) would meet the symbol.
    Signal stubs leaving the connector, and other rails' pins, simply cross
    the ties: the convention for a connector, which grade_schematic.py does
    not count as a crossing."""
    d = -1 if side == "L" else 1
    grounds = [g for g in groups if g[0].startswith("GND")]
    supplies = [g for g in groups if not g[0].startswith("GND")]
    for k, (rail, pts) in enumerate(grounds + supplies):
        x_t = pts[0][0] + d * 2.54 * (k + 1)
        pts = sorted(pts, key=lambda pt: pt[1])
        for pt in pts:
            sh.wire(pt, (x_t, pt[1]))
        if rail.startswith("GND"):
            end = (x_t, (pts[-1][1] if bottom is None else bottom) + P)
            corner, tees = pts[0], pts[1:]
        else:
            end = (x_t, (pts[0][1] if top is None else top) - P)
            corner, tees = pts[-1], pts[:-1]
        sh.wire((x_t, corner[1]), end)
        for pt in tees:
            sh.junction((x_t, pt[1]))
        rails.at(rail, end, up=not rail.startswith("GND"))


def df40(name, value, fp, descr, contacts):
    """A DF40 half, drawn as its pads sit. Both footprints, placed as the
    stack places them (J5 on the lower board's top, J6 on the upper board's
    underside, at the same x, y), put the odd contacts in one row and the
    even in the other, 1 and 2 at the end nearest the socket's pin 1; seen
    from the top of either board the even row is on the left. So both
    halves are drawn alike: 2-30 down the left, 1-29 down the right."""
    pins = ([(str(c), f"C{c}", "passive", "L") for c in range(2, 31, 2)] +
            [(str(c), f"C{c}", "passive", "R") for c in range(1, 30, 2)])
    assert sorted(int(p[0]) for p in pins) == sorted(contacts)
    return S.Sym(name, value, fp, descr, pins, width_units=6, ref_prefix="J")


def df40_stubs(sh, rails, cp, contacts):
    """Every DF40 signal contact ends in a stub and a label. Its grounds
    and supplies are tied per column, close to the contacts, into one rail
    symbol each - ground below the column, a supply above it - and the
    signal stubs cross the ties (connector_rails)."""
    cols = {"L": {}, "R": {}}
    for c, net in sorted(contacts.items()):
        side = "L" if c % 2 == 0 else "R"
        if net == "GND" or net.startswith("+5V"):
            cols[side].setdefault(net, []).append(cp[str(c)])
        else:
            pin_stub(sh, cp[str(c)], side, net=net)
    for side, groups in cols.items():
        if not groups:
            continue
        ys = [cp[str(c)][1] for c in contacts if (c % 2 == 0) == (side == "L")]
        connector_rails(sh, rails, sorted(groups.items()), side,
                        min(ys), max(ys))


def decoupling_row(sh, rails, rail, caps, start):
    """A row of decoupling capacitors between a supply wire and their own
    grounds, the rail symbol at its left end, as One ROM draws its bypass
    capacitors beside the part they serve. A single capacitor has no row:
    it is one vertical line, the rail symbol on top and ground below."""
    x0, y = start
    if len(caps) == 1:
        ref, val, fp = caps[0]
        x = x0 + 7.62
        rails.at(rail, (x, y), up=True)
        hang(sh, rails, S.two_pin("C_VW", "C", C_FP, "Capacitor", ref_prefix="C"),
             ref, val, (x, y), down=True, rail="GND", fp=fp, junction=False)
        return
    xs = [x0 + 7.62 + 12.7 * i for i in range(len(caps))]   # room for each one's text
    sh.wire((x0, y), (xs[-1], y))
    rails.at(rail, (x0, y), up=True)
    cap = S.two_pin("C_VW", "C", C_FP, "Capacitor", ref_prefix="C")
    for i, (x, (ref, val, fp)) in enumerate(zip(xs, caps)):
        hang(sh, rails, cap, ref, val, (x, y), down=True, rail="GND",
             fp=fp, junction=(i < len(caps) - 1))


def build_lower(board, project):
    """Socket, bus buffers and stack connector, left to right: J1 with the
    two tables below it, the buffers in two groups - address and enable
    (U2, U3 and the pulls on their socket lines) above data (U4, its
    RN1/RN2 terminators and R21) - and J5 at the right edge, level with
    their B sides. The buffers' supply (U7) is its own block in the top
    right corner and their decoupling a row beside them; the logic reaches
    both only through rail symbols. Each functional group is framed and
    captioned."""
    global Y0
    Y0 = 38.1
    sh = S.Sheet(project, paper="A3",
                 title="FastPROM lower board - socket, bus buffers, stack connector",
                 rev=REV, company="FastPROM", comment=HW_NOTICE)
    rails = Rails(sh)
    if board.get("rx_b_to_a"):
        sys.exit("the lower sheet draws its receive buffers A to B only")

    contacts = stack_contacts(board)
    ins = board["input_group"]
    outs = board["output_group"]
    grp_a, grp_b = ins[:5], ins[5:]
    vcc, gnd = board["power"]["vcc"], board["power"]["gnd"]
    BUS = "+3V3_BUF"

    # ---- rows ---------------------------------------------------------------
    # Two groups, one above the other, both reading host (left) to AG32
    # (right): address and enable on top, U2 (pins 1-5) above U3 (15-19)
    # with the pulls on U3's socket lines; data below, U4 with RN1/RN2 and
    # its enable's pull-up R21. All three buffers are the one SN74LVT245B
    # symbol. Each socket line starts at the left, at a label naming it, and
    # runs to its buffer: 1-5 level into U2, 15-19 level into U3, the data
    # lines level through RN1 and RN2 into U4. 15-19 stay a group: their
    # pull-ups rise above it, and their pull-downs sit in a row below it,
    # each dropping from its line across the lines beneath.
    buf = lvt245_sym()
    R2 = 0
    nb = buf.rows              # side-pin rows of each buffer
    R3 = R2 + nb + 13          # clear of U2's ground tie, U3's supply and R15
    PD_ROW = R3 + 4 + 2        # the pull-downs' drops end here, below A8
    ADDR_END = R3 + nb + 5     # below U3's ground and the pull-downs
    R4 = ADDR_END + 13         # the data group, its frame clear of this one
    NOE_RUN = R4 + nb + 4      # D_BUF_NOE's run, left from below U4's enable
    row_of = {}
    for i, p in enumerate(grp_a):
        row_of[p] = R2 + i
    for i, p in enumerate(outs):
        row_of[p] = R4 + i
    for i, p in enumerate(grp_b):
        row_of[p] = R3 + i
    assert sorted(row_of) == sorted(set(range(1, 21)) - {vcc, gnd}), sorted(row_of)

    # ---- J1, the plug, drawn as the DIP it is ------------------------------
    # As seen from the top of the board: pins 1-10 down the left, 20-11 down
    # the right, pin 20 opposite pin 1. Each signal pin ends in a stub and a
    # label naming its line; the line itself starts at the same label in
    # its group. Ground and the socket's 5 V are drawn at the pins.
    sh.group = "host"

    def plug_pin(p):
        et = "power_out" if p in (vcc, gnd) else "passive"
        nm = "VCC" if p == vcc else "GND" if p == gnd else f"P{p}"
        return (str(p), nm, et, "L" if p <= 10 else "R")
    plug = S.Sym("DIP20_PLUG_W", "DIP20_PLUG",
                 "fastprom:DIP-20_W7.62mm_PreciDip350_2xSIP",
                 "Machined round pins, plug into the host 20-pin DIP socket. "
                 "Drawn as seen from the top: 1-10 down the left, 20-11 down the right",
                 [plug_pin(p) for p in range(1, 11)] +
                 [plug_pin(p) for p in range(20, 10, -1)],
                 width_units=6, ref_prefix="J")
    J1R = 0
    X1 = 44.45
    pp = sh.place(plug, "J1", "DIP20_PLUG", (X1, centre_for_row0(plug, row(J1R))))
    for p in range(1, 21):
        if p not in (vcc, gnd):
            pin_stub(sh, pp[str(p)], "L" if p <= 10 else "R",
                     net=skt_net(board, p), length=12.7)
    # pin 10, the one ground, ties just beside the pins to its ground
    # symbol below the column
    connector_rails(sh, rails, [("GND", [pp[str(gnd)]])], "L",
                    pp["1"][1], pp["10"][1])
    v = pp[str(vcc)]
    # The rail symbol ends pin 20's wire, where a reader looks for it, and
    # the decoupling taps the wire on the way (grade_schematic.py checks that
    # every rail symbol ends its net), clear of the labels below it.
    sh.wire(v, (v[0] + 25.4, v[1]))
    rails.at("+5V_SKT", (v[0] + 25.4, v[1]), up=True)
    # 100 nF right at the pin: the data lines' return closes through the
    # host's own socket decoupling
    hang(sh, rails, S.two_pin("C_VW", "C", C_FP, "Capacitor", ref_prefix="C"),
         "C18", "100nF", (v[0] + 20.32, v[1]), down=True, rail="GND",
         fp=C_FP)
    # the socket lines start here, each at its label
    xj = 96.52

    # ---- buffers -------------------------------------------------------------
    AX = xj + 106.68           # the buffers' A-side pin ends
    placed = {}
    # the receivers' unused A6-A8, tied to ground, as inputs (lvt245_sym)
    tied = {str(2 + i): f"A{i + 1}_IN" for i in range(5, 8)}
    for ref, r0, alt in (("U2", R2, tied), ("U4", R4, None), ("U3", R3, tied)):
        sh.group = "data" if ref == "U4" else "addr"
        placed[ref] = sh.place(buf, ref, "SN74LVT245B",
                               (AX + P + buf.w_half, centre_for_row0(buf, row(r0))),
                               ref_pos=corner_text(buf, top=True),
                               val_pos=corner_text(buf, top=False, outside=True),
                               alternates=alt)
    u2, u4, u3 = placed["U2"], placed["U4"], placed["U3"]
    # The receivers: DIR high (A to B) straight to the rail; the enable and
    # the three unused A inputs tied to ground by one wire, clear of the
    # rail's name; the unused B outputs marked no-connect.
    sh.group = "addr"
    for u in (u2, u3):
        rails.at(BUS, u["20"], up=True)
        d = u["1"]
        sh.wire(d, (d[0] - P, d[1]))
        rails.at(BUS, (d[0] - P, d[1]), up=True)
        tie_column(sh, rails, [u["7"], u["8"], u["9"], u["19"]],
                   u["19"][0] - 12.7, "GND", down=True)   # clear of DIR's rail name
        tie_bottom(sh, rails, [u["10"], u["21"]], "GND")
        for pin in ("13", "12", "11"):
            sh.no_connect(u[pin])
    # The driver: DIR low (B to A) to ground; its enable runs to R21 below.
    sh.group = "data"
    rails.at(BUS, u4["20"], up=True)
    tie_bottom(sh, rails, [u4["10"], u4["21"]], "GND")
    d = u4["1"]
    sh.wire(d, (d[0] - 7.62, d[1]))
    rails.at("GND", (d[0] - 7.62, d[1]), up=False)

    # ---- socket lines in: 1-5 straight, 15-19 past their pulls ------------
    def line_start(p):
        a = (xj, row(row_of[p]))
        sh.name(skt_net(board, p), a)
        return a

    sh.group = "addr"
    for i, p in enumerate(grp_a):
        a, b = line_start(p), u2[str(2 + i)]
        assert abs(a[1] - b[1]) < 1e-6, (p, a, b)
        sh.wire(a, b)
    for i, p in enumerate(grp_b):
        a, b = line_start(p), u3[str(2 + i)]
        assert abs(a[1] - b[1]) < 1e-6, (p, a, b)
        sh.wire(a, b)

    # The pulls: pull-ups rising from their lines above the group (R20
    # crossing the nCE0 line), pull-downs in a row below it, each dropping
    # from its line and crossing the lines beneath. Each has a column of
    # its own, so no tap is a four-way.
    res = S.two_pin("R_VW", "R", R_FP, "Resistor")
    xr = xj + 17.78
    col = 12.7
    # (ref, socket pin, value, pull-down?, column, dnp)
    pulls = [("R15", grp_b[0], "100k", False, 0, False),
             ("R20", grp_b[1], "100k", False, 1, True),
             ("R16", grp_b[1], "100k", True, 2, False),
             ("R12", grp_b[2], "100k", True, 3, False),
             ("R13", grp_b[3], "100k", True, 4, False),
             ("R14", grp_b[4], "100k", True, 5, False)]
    y_up = row(R3 - 2)              # the pull-ups all start level, above nCE0
    y_dn = row(PD_ROW)              # the pull-downs all start level, below A8
    for ref, p, val, down, c, dnp in pulls:
        x = xr + c * col
        tap = (x, row(row_of[p]))
        end = (x, y_dn) if down else (x, y_up)
        if end != tap:
            sh.wire(tap, end)
        sh.junction(tap)
        hang(sh, rails, res, ref, val, end, down,
             "GND" if down else "+5V_SKT", dnp=dnp, junction=False)
    # R16 and R20 are alternatives on pin 16: say so beside them, and leave
    # which one to the Build options table.
    sh.text("R16 or R20: fit one (Build options)",
            (xr + 2 * col + 2.54, y_up - 8.89))

    # ---- data lines out: U4 -> series terminator packs -> socket ----------
    # Two 4 x 0402 arrays, element k pins k (socket side) and 9 - k (U4
    # side). Each is level with the four U4 channels it serves, so every
    # line from U4 to its pack is straight. They are stepped: RN1 (D0-D3)
    # beside U4, RN2 (D4-D7) further out, its outputs passing under RN1 and
    # RN1's inputs passing over it. The packs' bodies stop half a pitch past
    # their end elements so those lines clear them; their text goes above
    # RN1 and below RN2, out of the lines' way.
    sh.group = "data"
    pack = S.Sym("R_PACK04_W", "R_Pack04", "fastprom:R_Array_Convex_4x0402",
                 "4 x 0402 resistor array: element k is pins k and 9-k",
                 [(str(k), "~", "passive", "L") for k in range(1, 5)] +
                 [(str(9 - k), "~", "passive", "R") for k in range(1, 5)],
                 width_units=3, ref_prefix="RN", body_margin=P / 2,
                 elements=[(str(k), str(9 - k)) for k in range(1, 5)])
    span = 2 * (pack.w_half + P)        # pin end to pin end
    x_rn1 = AX - 7.62 - span / 2
    x_rn2 = x_rn1 - span - 7.62
    above = ((0, -(pack.h_half + 3.6), "center"),
             (0, -(pack.h_half + 1.4), "center"))
    below = ((0, pack.h_half + 1.6, "center"),
             (0, pack.h_half + 3.8, "center"))
    tpairs = []
    for ref, chans, x, (rpos, vpos) in (("RN1", range(0, 4), x_rn1, above),
                                        ("RN2", range(4, 8), x_rn2, below)):
        first = u4[str(2 + chans[0])]
        rn = sh.place(pack, ref, "4x33R", (x, centre_for_row0(pack, first[1])),
                      ref_pos=rpos, val_pos=vpos)
        for k, ch in enumerate(chans, start=1):
            a, b = rn[str(9 - k)], u4[str(2 + ch)]
            assert abs(a[1] - b[1]) < 1e-6
            sh.wire(a, b)
            tpairs.append((line_start(outs[ch]), rn[str(k)]))
    # all eight level with their packs
    sh.bus(tpairs, xj + 12.7, x_rn2 - span / 2 - 2.54, track_pitch=P)

    # D_BUF_NOE: from U4's enable down, and left past its pull-up below the
    # packs to a stub end carrying its label: the net goes on to J5.
    oe = u4["19"]
    y_oe = row(NOE_RUN)
    xv = oe[0] - P
    x21 = oe[0] - 20.32
    x_lab = x21 - 15.24
    sh.wire(oe, (xv, oe[1]))
    sh.wire((xv, oe[1]), (xv, y_oe))
    sh.wire((xv, y_oe), (x_lab, y_oe))
    sh.name("D_BUF_NOE", (x_lab, y_oe))
    # U4's enable pulled up to U4's own supply: TI's condition for its
    # outputs to stay high-impedance through power-up
    hang(sh, rails, S.two_pin("R_VW", "R", R_FP, "Resistor"), "R21", "10k",
         (x21, y_oe), down=False, rail=BUS)

    # ---- the buffers' B sides, each to a label that J5 carries ------------
    b_rows = {}
    for i, p in enumerate(grp_a):
        b_rows[p] = (u2, str(18 - i), "addr")
    for i, p in enumerate(outs):
        b_rows[p] = (u4, str(18 - i), "data")
    for i, p in enumerate(grp_b):
        b_rows[p] = (u3, str(18 - i), "addr")
    for p, (u, pin, g) in b_rows.items():
        sh.group = g
        pin_stub(sh, u[pin], "R", net=buf_net(board, p), length=12.7)

    # ---- stack connector ---------------------------------------------------
    # J5 drawn as its pads sit, seen from the top of the lower board (df40()),
    # at the right edge, level with the buffers' B sides. Its contacts come in
    # the order the board's copper needs, not the buffers', so every line
    # joins by label: a stub at each buffer's B pin and one at each contact.
    sh.group = "j5"
    b2b = df40("DF40C_30DP_W", "DF40C-30DP-0.4V",
               "fastprom:Hirose_DF40C-30DP-0.4V_2x15-1MP_P0.4mm",
               "0.4mm board-to-board header, mates with J6 on the upper board. "
               "Seen from the top of the lower board: even contacts 2-30 down the "
               "left, odd 1-29 down the right, 1 and 2 at the socket's pin 1 end. "
               "J6 above it, on the upper board's underside, is drawn from the "
               "same side: contact n meets contact n, row for row", contacts)
    J5R = R3 + 12
    X5 = 365.76
    cp = sh.place(b2b, "J5", "DF40C-30DP-0.4V", (X5, centre_for_row0(b2b, row(J5R))))
    df40_stubs(sh, rails, cp, contacts)

    # ---- the buffers' own supply, from the socket only --------------------
    # Fed from socket pin 20 and nothing else, so the buffers are unpowered
    # whenever the host is, and their ports high-impedance (Ioff): a module
    # running from USB cannot drive, or be fed from, a dead host's bus.
    # Its own block in the top right corner, apart from the logic.
    sh.group = "supply"
    cap = S.two_pin("C_VW", "C", C_FP, "Capacitor", ref_prefix="C")
    XP = 322.58
    y7 = row(R2 + 1)
    ldo7 = S.Sym("TLV74233_W", "TLV74233", "fastprom:Texas_X2SON-4_1x1mm_P0.65mm_NoEP",
                 "3.3V 200mA LDO, 1x1 mm",
                 [("4", "IN", "power_in", "L")] + [spacer("L")] * 4 +
                 [("3", "EN", "input", "L"),
                  ("1", "OUT", "power_out", "R")] + [spacer("R")] * 5 +
                 [("2", "GND", "power_in", "B")],
                 width_units=6)
    xin = XP + 2.54
    UX = xin + 35.56   # R26 clear of C12's value
    u7 = sh.place(ldo7, "U7", "TLV74233",
                  (UX + P + ldo7.w_half, centre_for_row0(ldo7, y7)),
                  ref_pos=(-ldo7.w_half, -ldo7.h_half - 4.2, "left"),
                  val_pos=(ldo7.w_half, -ldo7.h_half - 1.6, "right"))
    sh.wire((xin, y7), u7["4"])
    rails.at("+5V_SKT", (xin, y7), up=True)
    hang(sh, rails, cap, "C12", "1uF", (xin + 7.62, y7), down=True,
         rail="GND", fp=C_FP)
    # the enable delayed from the 5 V input, so the buffers power up after
    # the AG32's rail (tau 10 ms, EN threshold 0.9 V: about 2 ms)
    r26 = hang(sh, rails, S.two_pin("R_VW", "R", R_FP, "Resistor"), "R26",
               "100k", (xin + 20.32, y7), down=True, rail=None)
    en_l = max(r26.values(), key=lambda q: q[1])
    assert abs(en_l[1] - u7["3"][1]) < 1e-6, (en_l, u7["3"])
    sh.wire(en_l, u7["3"])
    sh.name("BUF_EN", (en_l[0] + 1.27, en_l[1]))
    hang(sh, rails, cap, "C17", "100nF", (xin + 27.94, en_l[1]), down=True,
         rail="GND", fp=C_FP)
    rails.at("GND", u7["2"], up=False)
    vo7 = u7["1"]
    end7 = (vo7[0] + 15.24, vo7[1])
    sh.wire(vo7, end7)
    hang(sh, rails, cap, "C13", "1uF", (vo7[0] + 7.62, vo7[1]), down=True,
         rail="GND", fp=C_FP)
    rails.at(BUS, end7, up=True)
    # a test pad for the buffers' supply, on top: reachable in the host with
    # the upper board lifted off (+5V_SKT is reachable at the socket's pin 20)
    test_points(sh, rails, [("TP1", BUS)],
                (xin + 7.62, y7 + 50.8))

    # ---- decoupling: one per buffer, in a row beside them -----------------
    sh.group = "decouple"
    decoupling_row(sh, rails, BUS,
                   [("C8", "100nF", C_FP), ("C9", "100nF", C_FP),
                    ("C10", "1uF", C_FP)], (266.7, y7))

    # ---- what to fit, and what the net names mean: tables -----------------
    # From tools/partdb.yaml. Pin 16 is an address line on some parts (R16,
    # a pull-down) and an enable on others (R20, a pull-up).
    sup = board["supports"]
    pin16 = {k: str(partdb_part(k)["socket_pins"][grp_b[1]]) for k in sup}
    fit = [["Part"] + list(sup),
           ["R16"] + ["fitted" if not pin16[k].startswith("nCE") else "not fitted" for k in sup],
           ["R20"] + ["fitted" if pin16[k].startswith("nCE") else "not fitted" for k in sup]]
    tx = 20.32
    sh.group = "fit"
    t = sh.table((tx, row(J1R + 18)), [12.7] + [17.78] * len(sup), fit)
    sh.group = "pins"
    sh.table((tx, t[3] + 15.24), [12.7, 17.78] + [17.78] * len(sup), pin_table(board))

    # ---- the frames ----------------------------------------------------------
    sh.group = None
    sh.frames({"host": "Host socket (DIP-20)",
               "fit": "Build options",
               "pins": "Pins that differ (nets follow the 512x8)",
               "addr": "Address and enable: host to AG32",
               "data": "Data: AG32 to host",
               "decouple": "Buffer decoupling (C8 U2, C9 U3, C10 U4)",
               "supply": "Buffer supply: socket 5 V only",
               "j5": "Stack connector (to upper board)"},
              align=[("left", ["host", "fit", "pins"]),
                     ("right", ["host", "fit", "pins"]),
                     ("left", ["addr", "data"]),
                     ("right", ["addr", "data"]),
                     ("top", ["host", "addr", "decouple", "supply"]),
                     ("right", ["supply", "j5"]),
                     ("left", ["supply", "j5"])])
    return sh


# ---------------------------------------------------------------------------
# Upper board
# ---------------------------------------------------------------------------

FP_BUF = "fastprom:Texas_RGY_R-PVQFN-N20_EP2.05x3.05mm"
C_FP = "fastprom:C_0402_1005Metric"
C_FP6 = "fastprom:C_0603_1608Metric"
Y0 = 76.2


def row(k):
    return Y0 + k * P


def centre_for_row0(sym, y_row0):
    return y_row0 + getattr(sym, "top_pin_y", sym.h_half - P)


def centre_for_row(sym, y, r):
    """Symbol origin that puts side-pin row r at height y."""
    return y + getattr(sym, "top_pin_y", sym.h_half - P) - r * P


def lvt245_sym():
    """SN74LVT245B, one symbol for every use: A1-A8 down the left and B1-B8
    down the right, row for row, so each channel reads straight across; DIR
    and ~OE below A8, three rows clear so a rail symbol on DIR and its name
    fit between it and A8's tie with room to spare, and a row apart; VCC on top, GND and the thermal pad below.

    The ports are typed bidirectional, as the part is: the same symbol is
    wired A to B (U2, U3) and B to A (U4). Each port also has KiCad
    alternate functions, An_IN (input) and An_OUT (3-state output), for an
    instance whose wiring fixes what a port does: a receiver's unused A
    inputs are tied to ground, and selecting An_IN there is what lets ERC
    see a tied-off input rather than an I/O shorted to a supply.
    Pin numbers per TI (RGY): 1 DIR, 2-9 A1-A8, 10 GND, 11-18 B8-B1,
    19 ~OE, 20 VCC, 21 thermal pad."""
    L = [(str(2 + i), f"A{i + 1}", "bidirectional", "L") for i in range(8)]
    L += [spacer("L")] * 3
    # a row between DIR and ~OE too: adjacent, ~OE's overbar reads as
    # DIR underlined
    L += [("1", "DIR", "input", "L"), spacer("L"),
          ("19", "~{OE}", "input", "L")]
    R = [(str(18 - i), f"B{i + 1}", "bidirectional", "R") for i in range(8)]
    T = [("20", "VCC", "power_in", "T")]
    B = [("10", "GND", "power_in", "B"), ("21", "EP", "passive", "B")]
    alts = {num: [(f"{nm}_IN", "input"), (f"{nm}_OUT", "tri_state")]
            for num, nm, _et, _s in L[:8] + R}
    return S.Sym("SN74LVT245B", "SN74LVT245B", FP_BUF,
                 "Octal bus transceiver, 3-state, 3.3V, I/O to 5.5V",
                 L + R + T + B, width_units=8, alternates=alts)


def pinno(name):
    return str(int(name.split("_")[1]))


def corner_text(sym, top=True, right=True, outside=False):
    """(dx, dy, justify) for text at a symbol's corner, clear of its edges."""
    dx = sym.w_half + (1.3 if outside else 0.0)
    if not right:
        dx = -dx
    dy = -(sym.h_half + 1.6) if top else (sym.h_half + 1.6)
    if outside:
        just = "left" if right else "right"
    else:
        just = "right" if right else "left"
    return (dx, dy, just)


def build_upper(board, project):
    """J6 on the left, the AG32 in the centre with its decoupling above it,
    and on its control side, top to bottom, USB-C, reset and boot, image
    select and the UART pads; the clock on the left below J6. The power
    path is its own block along the top right, reached by the logic only
    through rail symbols (and the one VBUS wire up from the USB clamp).
    Each functional group is framed and captioned."""
    global Y0
    Y0 = 99.06
    sh = S.Sheet(project, paper="A3",
                 title="FastPROM upper board - AG32 fabric ROM, USB, power",
                 rev=REV, company="FastPROM", comment=HW_NOTICE)
    rails = Rails(sh)
    contacts = stack_contacts(board)
    s2m = {int(k): v for k, v in board["socket_to_mcu"].items()}
    names = socket_names(board)
    ctrl = board["control"]
    grp_a = [1, 2, 3, 4, 5]
    grp_d = [6, 7, 8, 9, 11, 12, 13, 14]
    grp_b = [15, 16, 17, 18, 19]

    # Rows (in pitches from Y0) of the socket lines. The buffers are on the
    # lower board, so every line runs from J6 to its AG32 pin, by label.
    R2, R4 = 0, 8
    pk = AG32_FIXED[board["package"]]
    # The BOOT1 strap hangs off whichever data line shares BOOT1's pin, in
    # spare rows below that line; the enable's pull-up hangs above it.
    boot_k = next((i for i, p in enumerate(grp_d)
                   if pinno(s2m[p]) == pk["BOOT1"]), None)
    mid_gap = 9 if boot_k is not None and boot_k < 7 else 0
    d_rows = [R4 + i + (mid_gap if mid_gap and i > boot_k else 0)
              for i in range(8)]
    oe_row = d_rows[-1] + (GAP + 1 if mid_gap else GAP + 6)
    R3 = oe_row + 3

    # The AG32's left side: the socket lines in rows, then the clock input.
    nrows = R3 + 5
    hse_row = nrows + 6
    sig_rows = {p: R2 + i for i, p in enumerate(grp_a)}
    sig_rows.update(zip(grp_d, d_rows))
    sig_rows.update({p: R3 + i for i, p in enumerate(grp_b)})

    # ---- stack connector ---------------------------------------------------
    # J6 drawn as its pads sit (df40()). Its contacts are in the lower
    # board's copper order, not the AG32's, so every line joins by label:
    # a stub at each J6 contact and one at each AG32 pin.
    sh.group = "j6"
    j6s = df40("DF40C_30DS_W", "DF40HC(3.0)-30DS-0.4V",
               "fastprom:Hirose_DF40HC(3.0)-30DS-0.4V_2x15_P0.4mm",
               "0.4mm board-to-board socket, on the underside, mates with J5 on "
               "the lower board. Seen from the top of the upper board, through "
               "it: even contacts 2-30 down the left, odd 1-29 down the right, "
               "1 and 2 at the socket's pin 1 end - the same view as J5, which "
               "faces it from below: contact n meets contact n, row for row",
               contacts)
    # the value below the ground symbols that end the contact columns
    j6 = sh.place(j6s, "J6", "DF40HC(3.0)-30DS-0.4V",
                  (50.8, centre_for_row0(j6s, row(0))),
                  val_pos=(0, j6s.h_half + 8.89, "center"))
    df40_stubs(sh, rails, j6, contacts)
    # what the net names mean, from tools/partdb.yaml, below J6
    sh.group = "pins"
    y0 = max(pt[1] for pt in j6.values()) + 22.86
    sh.table((20.32, y0), [10.16, 17.78, 12.7, 12.7], pin_table(board, "BUF_"))

    rv = S.two_pin("R_VW", "R", R_FP, "Resistor")
    cap = S.two_pin("C_VW", "C", C_FP, "Capacitor", ref_prefix="C")

    # ---- the AG32 --------------------------------------------------------
    sh.group = "mcu"
    uleft = [spacer("L")] * (hse_row + 1)
    for p, r in sig_rows.items():
        kind = "OUT" if p in grp_d else "IN"
        uleft[r] = (pinno(s2m[p]), f"{kind}_{names[p]}", "bidirectional", "L")
    uleft[oe_row] = (pinno(ctrl["data_oe"]), "BUF_nOE", "bidirectional", "L")
    if "hse_in" in ctrl:
        uleft[hse_row] = (pinno(ctrl["hse_in"]), "OSC_IN", "bidirectional", "L")
    used = {pinno(v) for v in s2m.values()} | {pinno(v) for k, v in ctrl.items()
                                               if str(v).startswith("PIN_")
                                               and k != "hse_in"}
    boot1_data = pk["BOOT1"] in used
    if not boot1_data:
        sys.exit("BOOT1 on a pin of its own is not drawn")
    if ctrl.get("bank_select") != "uart_dip":
        sys.exit("image select: only bank_select: uart_dip is drawn")
    # The control side, top to bottom in the order its blocks sit: USB, the
    # image select between the UART lines, then reset and boot. TX, RX,
    # BOOT0 and NRST leave in J3's pad order, so the four lines reach J3
    # without crossing one another.
    dp_row = 3
    tx_row = 22
    rx_row = tx_row + UART_GAP
    boot_row = rx_row + 14
    nrst_row = boot_row + 10
    rt = {dp_row: (pk["DP"], "USBDP"), dp_row + 1: (pk["DM"], "USBDM"),
          tx_row: (pk["TX"], "UART0_TX"), rx_row: (pk["RX"], "UART0_RX"),
          rx_row + 4: (pk["JTMS"], "JTMS"), rx_row + 5: (pk["JTCK"], "JTCK"),
          boot_row: (pk["BOOT0"], "BOOT0"), nrst_row: (pk["NRST"], "NRST")}
    uright = [spacer("R")] * (nrst_row + 1)
    for r, (num, nm) in rt.items():
        uright[r] = (num, nm, "input" if nm in ("NRST", "BOOT0") else
                     "bidirectional", "R")
    utop = [(n, nm, "power_in", "T") for n, nm in pk["top"]]
    ubot = [(n, nm, "power_in" if nm != "EP" else "passive", "B")
            for n, nm in pk["bot"]]
    pinfo = AG32_PACKAGES[board["package"]]
    u1s = S.Sym(pinfo["symbol"] + "_W", pinfo["symbol"], pinfo["footprint"],
                f"RISC-V MCU with AGRV2K fabric, {board['package']}",
                uleft + uright + utop + ubot, width_units=14)
    UL = 111.76
    u1 = sh.place(u1s, "U1", pinfo["symbol"],
                  (UL + P + u1s.w_half, centre_for_row0(u1s, row(0))),
                  ref_pos=corner_text(u1s, top=True),
                  val_pos=corner_text(u1s, top=False, outside=True))
    tie_top(sh, rails, [u1[n] for n, _ in pk["top"]], "+3V3_MCU")
    tie_bottom(sh, rails, [u1[n] for n, _ in pk["bot"]], "GND")

    # Each socket line from its AG32 pin to its label.
    for p in sig_rows:
        b = u1[pinno(s2m[p])]
        pin_stub(sh, b, "L", net=buf_net(board, p), length=17.78)
        if pinno(s2m[p]) == pk["BOOT1"]:
            # BOOT1 shares this data line; its strap hangs off it.
            hang(sh, rails, rv, "R11", "4k7", (b[0] - 7.62, b[1]),
                 down=True, rail="GND")
    pin_stub(sh, u1[pinno(ctrl["data_oe"])], "L", net="D_BUF_NOE",
             length=17.78)
    # (its pull-up, R21, is on the lower board, at the buffer it enables)

    def name_at(pin, net):
        sh.group = "mcu"
        sh.name(net, (u1[pin][0] + P, u1[pin][1]))

    # ---- AG32 decoupling, in a row above it --------------------------------
    sh.group = "decouple"
    decoupling_row(sh, rails, "+3V3_MCU",
                   [("C3", "100nF", C_FP), ("C4", "100nF", C_FP),
                    ("C5", "100nF", C_FP), ("C15", "1uF", C_FP),
                    ("C14", "10uF", C_FP6)], (UL - 7.62, 45.72))

    # ---- the USB clock: 12 MHz oscillator into OSC_IN (HSE bypass) --------
    # On the AG32's left, below J6, with its own decoupling beside it.
    if "hse_in" in ctrl:
        sh.group = "clock"
        osc = S.Sym("OSC_4PIN_W", "OSC", "fastprom:Oscillator_SMD_SiT_PQFN-4Pin_2.0x1.6mm",
                    "12 MHz CMOS oscillator: the PLL makes the 48 MHz USB clock from it",
                    [("4", "VDD", "power_in", "L"), ("1", "OE", "input", "L"),
                     ("3", "OUT", "output", "R"), spacer("R"),
                     ("2", "GND", "power_in", "B")], width_units=5, ref_prefix="X")
        hp = pinno(ctrl["hse_in"])
        a = u1[hp]
        xo = UL - 38.1                      # X1's output pin
        x1 = sh.place(osc, "X1", "12MHz",
                      (xo - P - osc.w_half, centre_for_row0(osc, a[1])),
                      ref_pos=corner_text(osc, top=True, right=False),
                      val_pos=corner_text(osc, top=False, right=False, outside=True))
        assert abs(x1["3"][1] - a[1]) < 1e-6
        sh.name("HSE_IN", (x1["3"][0] + 1.27, a[1]))
        sh.wire(x1["3"], (x1["3"][0] + 12.7, a[1]))
        sh.group = None
        sh.wire((x1["3"][0] + 12.7, a[1]), a)
        sh.group = "clock"
        tie_column(sh, rails, [x1["4"], x1["1"]], x1["4"][0] - 5.08,
                   "+3V3_MCU", down=False)
        rails.at("GND", x1["2"], up=False)
        decoupling_row(sh, rails, "+3V3_MCU", [("C16", "100nF", C_FP)],
                       (x1["4"][0] - 27.94, a[1]))

    XR = u1[pk["NRST"]][0]
    xt = XR + 27.94
    tx, rx = pk["TX"], pk["RX"]
    xs = xt + 20.32                     # SW1's 3V3 clear of R17's ground
    # The four lines run on past the image select and reset blocks to a
    # stub end each, where a label carries them to J3: J3's pins come in
    # the clip's order, not these lines'.
    XEND = xs + 38.1                    # each label clear of the blocks' frames
    sh.group = "mcu"
    sh.no_connect(u1[pk["JTMS"]])
    sh.no_connect(u1[pk["JTCK"]])
    sh.group = None
    for pin, net in ((tx, "UART_TX"), (rx, "UART_RX"),
                     (pk["BOOT0"], "BOOT0"), (pk["NRST"], "NRST")):
        a = u1[pin]
        sh.wire(a, (XEND, a[1]))
        sh.name(net, (XEND, a[1]), rot=180)

    # ---- programming: J3, SOICbite ------------------------------------------
    # The one programming interface: firmware and fabric bitstream both go
    # through the UART0 mask-ROM loader. J3 is a SOICbite: eight pads at the
    # board edge, 1-4 on top and 8-5 under them, for a SOIC-8 clip or a
    # pogo fixture that holds itself on. Drawn as the clip sees it, 1-4 down
    # the left and 8-5 down the right; the pinout is SOICbite's ESP8266 one,
    # so the fixture holds BOOT0 (pin 2, the boot strap) high to select the
    # loader and resets through NRST (pin 8).
    sh.group = "uart"
    j3s = S.Sym("CONN_SOICBITE_W", "SOICbite",
                "fastprom:SOICbite_2x04_P1.27mm_BoardEdge",
                "SOICbite programming pads at the board edge: 1-4 on top, "
                "8-5 under them; SOIC-8 clip or pogo fixture",
                [("1", "3V3", "passive", "L"), ("2", "BOOT0", "passive", "L"),
                 ("3", "NC", "passive", "L"), ("4", "GND", "passive", "L"),
                 ("8", "NRST", "passive", "R"), ("7", "NC", "passive", "R"),
                 ("6", "TX", "passive", "R"), ("5", "RX", "passive", "R")],
                width_units=6, ref_prefix="J")
    j3 = sh.place(j3s, "J3", "SOICbite",
                  (XEND + 45.72 + j3s.w_half, centre_for_row0(j3s, u1[rx][1])))
    connector_rails(sh, rails, [("GND", [j3["4"]]), ("+3V3", [j3["1"]])],
                    "L", j3["1"][1], j3["4"][1])
    pin_stub(sh, j3["2"], "L", "BOOT0")
    for jp, net in (("8", "NRST"), ("6", "UART_TX"), ("5", "UART_RX")):
        pin_stub(sh, j3[jp], "R", net)
    for jp in ("3", "7"):
        sh.no_connect(j3[jp])
    yt, yr = u1[tx][1], u1[rx][1]

    # ---- image select, on the UART lines ------------------------------------
    # 100k down on each, and a switch that adds 10k to 3V3. UART idles
    # high, so the pull-up never disturbs it.
    sh.group = "imgsel"
    hang(sh, rails, rv, "R17", "100k", (xt, yt), down=True, rail="GND")
    hang(sh, rails, rv, "R18", "100k", (xt, yr), down=True, rail="GND")
    sws = S.Sym("SW_DIP_x02_W", "Image_Select", SW_DIP_FP,
                "2-position DIP switch: switch 1 = pins 1-4 (image bit 0), switch 2 = pins 2-3 (bit 1)",
                [("2", "B", "passive", "T"), ("3", "B", "passive", "R"),
                 ("4", "A", "passive", "L"), ("1", "A", "passive", "B")],
                width_units=4, ref_prefix="SW")
    r24 = hang(sh, rails, rv, "R24", "10k", (xs, yt), down=True, rail=None)
    r25 = hang(sh, rails, rv, "R25", "10k", (xs, yr), down=False, rail=None)
    lo24 = max(r24.values(), key=lambda q: q[1])
    hi25 = min(r25.values(), key=lambda q: q[1])
    sw = sh.place(sws, "SW1", "Image_Select",
                  (xs, (lo24[1] + hi25[1]) / 2),
                  ref_pos=(sws.w_half + 1.0, sws.h_half + 1.3, "left"),
                  val_pos=(sws.w_half + 1.0, sws.h_half + 3.2, "left"))
    sh.wire(lo24, sw["2"])
    sh.wire(sw["1"], hi25)
    sh.wire(sw["3"], (sw["3"][0] + P, sw["3"][1]))
    rails.at("+3V3", (sw["3"][0] + P, sw["3"][1]), up=True)
    sh.wire(sw["4"], (sw["4"][0] - P, sw["4"][1]))
    rails.at("+3V3", (sw["4"][0] - P, sw["4"][1]), up=True)

    # ---- reset and boot strap, on the BOOT0 and NRST lines ------------------
    # No JTAG: AGM's UART0 loader writes the fabric bitstream as well as the
    # firmware, so JTMS and JTCK are left on their internal pulls and the
    # programming strap (BOOT0) goes to J3.
    sh.group = "reset"
    b0, n = u1[pk["BOOT0"]], u1[pk["NRST"]]
    hang(sh, rails, rv, "R10", "10k", (xt + 15.24, b0[1]), down=True,
         rail="GND")
    hang(sh, rails, rv, "R9", "10k", (xt, n[1]), down=False,
         rail="+3V3_MCU")
    hang(sh, rails, cap, "C11", "100nF", (xt + 7.62, n[1]), down=True,
         rail="GND", fp=C_FP)

    # ---- USB: AG32 -> ESD clamp -> USB-C ---------------------------------
    sh.group = "usb"
    esd_s = S.Sym("USBLC6_W", "USBLC6-2SC6", "fastprom:SOT-23-6",
                  "Dual-line USB ESD protection",
                  [spacer("L"), ("3", "IO2", "passive", "L"),
                   ("1", "IO1", "passive", "L"), spacer("L"),
                   spacer("R"), ("4", "IO2", "passive", "R"),
                   ("6", "IO1", "passive", "R"), spacer("R"),
                   ("5", "VBUS", "power_in", "T"), ("2", "GND", "power_in", "B")],
                  width_units=7)
    UL6 = XR + 38.1
    u6 = sh.place(esd_s, "U6", "USBLC6-2SC6",
                  (UL6 + P + esd_s.w_half, centre_for_row(esd_s, u1[pk["DP"]][1], 1)),
                  ref_pos=corner_text(esd_s, top=True, right=False, outside=True),
                  val_pos=corner_text(esd_s, top=False, right=False, outside=True))
    # Each line runs through a pin pair (3-4 and 1-6), used so that on the
    # board the clamp's AG32 side faces the QFN.
    for pin, ep, net in ((pk["DP"], "3", "USB_DP"), (pk["DM"], "1", "USB_DM")):
        assert abs(u1[pin][1] - u6[ep][1]) < 1e-6
        sh.group = None
        sh.wire(u1[pin], u6[ep])
        name_at(pin, net)
    sh.group = "usb"
    rails.at("GND", u6["2"], up=False)

    # J4 drawn as its pads sit, top to bottom = left to right along the
    # footprint seen from the top of the board, the A row's and B row's
    # pads interleaved as HRO lays them out (each GND and VBUS pair shares
    # a pad). Everything on it is drawn: the clamp's D+ and D- run level
    # into A6 and A7, the flipped pair B6 and B7 tie onto them, and the CC
    # pads run to their pull-downs below.
    usb_pins = [("A1", "GND", "passive"), ("B12", "GND", "passive"),
                ("A4", "VBUS", "power_out"), ("B9", "VBUS", "passive"),
                ("B8", "SBU2", "passive"), ("A5", "CC1", "passive"),
                ("B7", "DN2", "passive"), ("A6", "DP1", "passive"),
                ("A7", "DN1", "passive"), ("B6", "DP2", "passive"),
                ("A8", "SBU1", "passive"), ("B5", "CC2", "passive"),
                ("A9", "VBUS", "passive"), ("B4", "VBUS", "passive"),
                ("A12", "GND", "passive"), ("B1", "GND", "passive")]
    usb_s = S.Sym("USB_C_W", "USB_C_Receptacle_USB2.0", USB_C_FP_STACK,
                  "USB 2.0 Type-C receptacle, pads in footprint order, left to "
                  "right seen from the top: A1/B12 A4/B9 B8 A5 B7 A6 A7 B6 A8 B5 "
                  "A9/B4 A12/B1, shell (SH) at both ends",
                  [(n, nm, et, "L") for n, nm, et in usb_pins] +
                  [("SH", "SHIELD", "passive", "L")],
                  width_units=8, ref_prefix="J")
    J4L = UL6 + 76.2
    # J4's A6 (D+) level with the clamp's IO2, so A6 and A7 meet the clamp
    # on straight wires
    j4 = sh.place(usb_s, "J4", "USB_C_USB2.0",
                  (J4L + P + usb_s.w_half, centre_for_row(usb_s, u6["4"][1], 7)))
    for pin in ("A8", "B8"):
        sh.no_connect(j4[pin])
    for ep, pin, net in (("4", "A6", "USB_DP_CON"), ("6", "A7", "USB_DM_CON")):
        assert abs(u6[ep][1] - j4[pin][1]) < 1e-6
        sh.wire(u6[ep], j4[pin])
        sh.name(net, (u6[ep][0] + P, u6[ep][1]))
    # The flipped plug's pair: B7 (D-) and B6 (D+), each by a short vertical
    # from its line, crossing the other line, out of the rail ties' way.
    xj4 = j4["A6"][0]
    for pin, line, x in (("B7", "A7", xj4 - 10.16), ("B6", "A6", xj4 - 12.7)):
        tap = (x, j4[line][1])
        sh.wire(tap, (x, j4[pin][1]))
        sh.wire((x, j4[pin][1]), j4[pin])
        sh.junction(tap)
    # The rails tie beside the pads, one upright symbol each: ground, the
    # shell with it, down to its symbol below J4, VBUS up to +5V_USB above
    # it. The signal stubs and the VBUS pads' wires cross the ground tie.
    connector_rails(sh, rails,
                    [("GND", [j4[p] for p in ("A1", "B12", "A12", "B1", "SH")]),
                     ("+5V_USB", [j4[p] for p in ("A4", "B9", "A9", "B4")])],
                    "L", top=j4["A1"][1])   # +5V_USB rises from J4's top edge, standing just above it
    # CC1 and CC2: 5k1 each to ground, presenting a device. Each pad's line
    # runs out to a drop down to its pull-down, the two in a row below the
    # pads, as the socket pulls are on the lower board: CC1's drop crosses
    # the D+ and D- lines, CC2's is the outer one.
    y_dn = j4["B5"][1] + P
    for pin, ref, net, x in (("A5", "R22", "USB_CC1", xj4 - 33.02),
                             ("B5", "R23", "USB_CC2", xj4 - 20.32)):
        pt = j4[pin]
        sh.wire(pt, (x, pt[1]))
        sh.wire((x, pt[1]), (x, y_dn))
        sh.name(net, (x + P, pt[1]))
        hang(sh, rails, rv, ref, "5k1", (x, y_dn), down=True, rail="GND",
             junction=False)

    build_power(sh, rails, u6["5"], cap)

    sh.group = None
    sh.frames({"j6": "Stack connector (from lower board, buffered 3.3 V)",
               "pins": "Pins that differ (nets follow the 512x8)",
               "clock": "Clock: 12 MHz for USB",
               "decouple": "AG32 decoupling",
               "mcu": "AG32: fabric ROM core",
               "usb": "USB-C",
               "reset": "Reset and boot",
               "imgsel": "Image select",
               "uart": "UART programming",
               "power": "Power path: socket or USB, regulator"},
              align=[("left", ["j6", "pins", "clock"]),
                     ("right", ["j6", "pins", "clock"]),
                     ("left", ["decouple", "mcu"]),
                     ("right", ["decouple", "mcu"]),
                     ("left", ["usb", "imgsel", "reset"]),
                     ("right", ["imgsel", "reset"]),
                     ("top", ["j6", "mcu", "usb"])])
    return sh


def build_power(sh, rails, vbus, cap):
    """Socket 5 V or USB 5 V, OR-ed, regulated, and isolated for the fabric:
    its own block along the top right. Everything enters and leaves it as a
    rail: +5V_USB (the clamp's VBUS pin and J4's VBUS pads carry the same
    rail symbol) and +5V_SKT in, +3V3 and +3V3_MCU out."""
    rails.at("+5V_USB", vbus, up=True)
    sh.group = "power"
    xv = vbus[0]
    y_skt = 33.02                   # D2's row; D3's is three pitches down
    ybus = y_skt + 3 * P
    dio = S.two_pin("D_HSW", "D_Schottky", "fastprom:D_SOD-323",
                    "Schottky, power OR-ing", ref_prefix="D", vertical=False,
                    swap=True)
    XD = xv + 48.26                 # D3's text clear of the damper
    # the OR-ing pair's text goes outside the pair: D2's above it, D3's below
    d3 = sh.place(dio, "D3", "Schottky", (XD, ybus), "fastprom:D_SOD-323",
                  ref_pos=(0, 2.8, "center"), val_pos=(0, 4.8, "center"))
    sh.wire((xv, ybus), d3["2"])
    rails.at("+5V_USB", (xv, ybus), up=True)
    # USB VBUS: the clamp's 100 nF, and a hot-plug damper (R27 + C20) so a
    # cable's inductance cannot ring the regulator's input past its rating
    hang(sh, rails, cap, "C19", "100nF", (xv + 15.24, ybus), down=True,
         rail="GND", fp=C_FP)
    # The damper is drawn one part deep, not two: R27 across, just below its
    # tap, and C20 hanging from its far end. That keeps this block shallow
    # enough for J4's +5V_USB to stand above J4 in the USB-C block below.
    tap = (xv + 25.4, ybus)
    jog = (tap[0], ybus + 3 * P)       # clear of D3's text
    sh.wire(tap, jog)
    sh.junction(tap)
    rh = S.two_pin("R_H", "R", R_FP, "Resistor", vertical=False)
    r27 = sh.place(rh, "R27", "1R", (jog[0] + 5.08, jog[1]), R_FP)
    left, right = sorted(r27.values())
    assert abs(left[0] - jog[0]) < 1e-6 and abs(left[1] - jog[1]) < 1e-6
    c20 = (right[0] + 5.08, right[1])     # clear of R27's value text
    sh.wire(right, c20)
    hang(sh, rails, cap, "C20", "4.7uF", c20, down=True, rail="GND",
         fp=C_FP6, junction=False)
    d2 = sh.place(dio, "D2", "Schottky", (XD, y_skt), "fastprom:D_SOD-323",
                  ref_pos=(0, -4.8, "center"), val_pos=(0, -2.8, "center"))
    sh.wire(d2["2"], (d2["2"][0] - 10.16, d2["2"][1]))
    rails.at("+5V_SKT", (d2["2"][0] - 10.16, d2["2"][1]), up=True)
    f5 = (d2["2"][0] - P, d2["2"][1])
    sh.wire(f5, (f5[0], f5[1] - 2 * P))
    sh.junction(f5)
    sh.place(S.FlagSym(), "#FLG01", at=(f5[0], f5[1] - 2 * P))
    xc = d2["1"][0] + 5.08
    ynode = d2["1"][1]
    sh.wire(d2["1"], (xc, ynode))
    sh.wire(d3["1"], (xc, d3["1"][1]))
    sh.wire((xc, ynode), (xc, d3["1"][1]))

    ldo = S.Sym("AP2112K_W", "AP2112K-3.3", "fastprom:SOT-23-5",
                "3.3V 600mA LDO",
                [("1", "VIN", "power_in", "L"), ("3", "EN", "input", "L"),
                 spacer("L"),
                 ("5", "VOUT", "power_out", "R"), ("4", "NC", "no_connect", "R"),
                 spacer("R"),
                 ("2", "GND", "power_in", "B")], width_units=6)
    UX = XD + 35.56
    u5 = sh.place(ldo, "U5", "AP2112K-3.3",
                  (UX + P + ldo.w_half, centre_for_row0(ldo, ynode)),
                  ref_pos=(-ldo.w_half, -ldo.h_half - 4.2, "left"),
                  val_pos=(ldo.w_half, -ldo.h_half - 1.6, "right"))
    sh.wire((xc, ynode), u5["1"])
    sh.junction((xc, ynode))
    sh.name("+5V", (xc + P, ynode))
    xe = u5["3"][0] - 5.08
    sh.wire(u5["3"], (xe, u5["3"][1]))
    sh.wire((xe, u5["3"][1]), (xe, ynode))
    sh.junction((xe, ynode))
    hang(sh, rails, cap, "C1", "4.7uF", (xc + 10.16, ynode), down=True,
         rail="GND", fp=C_FP6)
    fx = xc + 15.24
    sh.wire((fx, ynode), (fx, ynode - 2 * P))
    sh.junction((fx, ynode))
    sh.place(S.FlagSym(), "#FLG03", at=(fx, ynode - 2 * P))
    rails.at("GND", u5["2"], up=False)
    sh.no_connect(u5["4"])

    vo = u5["5"]
    hang(sh, rails, cap, "C2", "10uF", (vo[0] + 7.62, vo[1]), down=True,
         rail="GND", fp=C_FP6)
    xr3 = vo[0] + 12.7
    sh.wire((xr3, vo[1]), (xr3, vo[1] - 2 * P))
    sh.junction((xr3, vo[1]))
    rails.at("+3V3", (xr3, vo[1] - 2 * P), up=True)
    fb = S.two_pin("FB_HW", "FerriteBead", "fastprom:L_0603_1608Metric",
                   "Ferrite bead", ref_prefix="FB", vertical=False)
    fbp = sh.place(fb, "FB1", "220R", (vo[0] + 25.4, vo[1]),
                   "fastprom:L_0603_1608Metric")
    sh.wire(vo, fbp["1"])
    tail = (fbp["2"][0] + 10.16, vo[1])
    sh.wire(fbp["2"], tail)
    rails.at("+3V3_MCU", tail, up=True)
    fx2 = fbp["2"][0] + P
    sh.wire((fx2, vo[1]), (fx2, vo[1] - 2 * P))
    sh.junction((fx2, vo[1]))
    sh.place(S.FlagSym(), "#FLG04", at=(fx2, vo[1] - 2 * P))
    # Ground arrives only through connector contacts, so it needs its own flag.
    gf = (tail[0] + 15.24, vo[1] - 2 * P)
    sh.place(S.FlagSym(), "#FLG02", at=gf)
    sh.wire(gf, (gf[0], gf[1] + 2 * P))
    rails.at("GND", (gf[0], gf[1] + 2 * P), up=False)
    # test pads for USB's 5 V and the AG32's own rail (J3 already has 3V3)
    test_points(sh, rails, [("TP2", "+5V_USB"), ("TP3", "+3V3_MCU")],
                (fbp["2"][0], vo[1] + 22.86))


BUILDERS = {"fastprom-lower": build_lower,
            "fastprom-upper": build_upper}


def write_library(sh):
    libdir = os.path.join(HW, "lib")
    os.makedirs(libdir, exist_ok=True)
    path = os.path.join(libdir, "fastprom.kicad_sym")
    existing = {}
    if os.path.exists(path):
        with open(path) as fh:
            body = fh.read()
        for m in re.finditer(r'(  \(symbol "([^"]+)".*?\n  \)\n)', body, re.S):
            existing[m.group(2)] = m.group(1)
    for name, sym in sh.syms.items():
        existing[name] = sym.render(qualified=False)
    with open(path, "w") as fh:
        fh.write('(kicad_symbol_lib\n  (version 20241209)\n'
                 '  (generator "fastprom")\n  (generator_version "10.0")\n')
        for name in sorted(existing):
            fh.write(existing[name])
        fh.write(")\n")


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--board", required=True)
    args = ap.parse_args()
    with open(os.path.join(ROOT, "tools", "boardmap.yaml")) as fh:
        board = yaml.safe_load(fh)["boards"][args.board]
    if args.board not in BUILDERS:
        sys.exit(f"no wired builder for {args.board}")
    S.reset_uids()
    sh = BUILDERS[args.board](board, args.board)
    sh.write(os.path.join(HW, f"{args.board}.kicad_sch"))
    # The project beside it, naming this sheet as its root, so a clean
    # checkout builds the same project, netlist and board as any other.
    with open(os.path.join(HW, f"{args.board}.kicad_pro"), "w") as fh:
        fh.write(PROJECT_TEMPLATE
                 .replace("SHEETUUID", sh.sheet_uuid)
                 .replace("fastprom.kicad_pro", f"{args.board}.kicad_pro"))
    write_library(sh)
    print(f"wrote hw/{args.board}.kicad_sch, .kicad_pro ({len(sh.wires)} "
          f"wires, {len(sh.juncs)} junctions)")


if __name__ == "__main__":
    main()
