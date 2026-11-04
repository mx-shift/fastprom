#!/usr/bin/env python3
"""Generate the PCBs for the two-board stack.

Runs inside KiCad's Python (pcbnew); `make layout` arranges that. Input is
the exported netlist for each board, so the copper is built from exactly the
connectivity the schematic grader and the netlist equivalence check signed
off - nothing about the circuit is restated here. What this file owns is
geometry: where each part sits, which side it is on, how the inner layers are
used, and the design rules.

The routing is not computed here. Each board's tracks and vias are kept in
hw/<board>.route.json (see tools/layout/route.py) and laid onto the placed
board, so a rebuild is deterministic and needs no autorouter. Routing is
redone only on purpose, with `make reroute`:

  --dsn   place, pour the planes, export build/fr/<board>.dsn
          (scripts/freerouting.sh then routes it)
  --ses   import build/fr/<board>.ses, tidy it, widen what Freerouting necked
          down, clean up its corners (clean_corners), and write
          hw/<board>.route.json
  --amend REF  re-lay the hand-routed ground copper (tools/layout/fans.py)
          at REF's ground pads in the stored routing, drop what then leads
          nowhere, and rewrite hw/<board>.route.json; no autorouter, and
          the rest of the routing is kept as it is

Both boards are built in one run, in one coordinate frame: the lower board's
pin 1 end is y 0 (the upper board's pin 1 end is PROG_EXTRA beyond it, at
negative y), and the lower board sits underneath the upper at the same X/Y. That is what lets the two halves of the board-to-board connector be
checked against each other contact by contact, rather than trusted.

Outputs: hw/<board>.kicad_pcb, plus build/layout/<board>.json, a plain dump
of the geometry for previews and checks that do not need KiCad.
"""
import argparse
import hashlib
import json
import math
import os
import re
import shutil
import subprocess
import sys
import uuid

import pcbnew

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(HERE)
sys.path.insert(0, os.path.join(HERE, "layout"))
import fans as FANS             # noqa: E402
from notice import HW_NOTICE    # noqa: E402
import netlist as NL            # noqa: E402
import route as RT              # noqa: E402

HW = os.path.join(ROOT, "hw")
# The board revision (hw/REVISION, which gen_wired.py's title blocks and
# the release tag also follow): silkscreen and the boards' title blocks.
REV = open(os.path.join(HW, "REVISION")).read().strip()
STOCK = os.environ.get("KICAD10_FOOTPRINT_DIR", "/usr/share/kicad/footprints")
LOCAL = os.path.join(HW, "lib", "fastprom.pretty")

# Where the frame's origin lands on the KiCad sheet. Arbitrary but fixed.
OX, OY = 100.0, 100.0

# ---------------------------------------------------------------------------
# Rules. JLCPCB 4-layer, 0.8 mm, inside the no-extra-cost envelope.
# ---------------------------------------------------------------------------
RULES = {
    # 0.1 mm track and space: inside JLC's 0.09/0.09 mm multilayer
    # minimum at no extra cost. At 0.15/0.127 the 1 mm lanes beside the
    # buffers and between the connector's via columns take two lines; at
    # 0.1/0.1, four and three.
    "clearance": 0.10,
    "min_track": 0.10,      # JLC's multilayer minimum is 0.09
    "edge": 0.30,           # copper to routed edge; JLC minimum is 0.2
    "via_d": 0.45,          # 0.2 mm hole with 0.45 mm pad: no surcharge
    "via_drill": 0.20,
    "hole_clearance": 0.20,     # JLC: via hole to track 0.2 mm
    # Silkscreen, JLC's legend rules (DESIGN_NOTES.md, "Silkscreen
    # rules"): 0.15 mm between legend items and from the board edge
    # (KiCad's silk clearance, which its edge check also takes), 0.15 mm
    # from any pad (its mask opening: the boards' pads have no mask
    # expansion), lines at least 0.15 mm wide, text at least 1.0 mm high
    # with 0.15 mm strokes. The pad and line rules are custom rules
    # (silk_rules below); KiCad has no board setting for either.
    "silk": 0.15,
    "silk_pad": 0.15,
    "silk_line": 0.15,
    "silk_text_h": 1.0,
    "silk_text_t": 0.15,
}
W_SIG, W_PWR = 0.10, 0.30
THICKNESS = 0.8

# Socket stack geometry, shared by both boards. The lower board's outline
# is BOARD_W x BOARD_L: the socket's pin field plus 1.70 mm past each pin
# row end, and no wider than its 11.02 mm (neighbouring parts on the host).
# The upper board has the same width and is USB_EXTRA + PROG_EXTRA longer
# (below). The frame's origin is BOARD_X0 left of the boards' left edge - where a wider
# upper board's corner once was - and stays there so that the coordinates
# in fans.py and the routing files mean what they always have.
BOARD_W, BOARD_L = 11.02, 26.26
CENTRE_X = 7.0                              # the socket's centre line
BOARD_X0 = CENTRE_X - BOARD_W / 2           # 1.49
DIP_X0 = CENTRE_X - 3.81                    # DIP pin 1 column
DIP_Y0 = 1.70                               # DIP pin 1 row
OUTLINE = [(BOARD_X0, 0.0, BOARD_X0 + BOARD_W, BOARD_L)]
# The upper board is the lower's width and USB_EXTRA longer, at its USB
# end (the socket's pin 11 end, y 26.26 -> 27.76).
# The receptacle sits at that end on top with J6 on the underside below
# it, and its two locating-peg holes (0.60 mm NPTH, 5.79 mm behind the
# board edge in HRO's land pattern) must stay out of J6's courtyard, which
# ends at y 21.405: that needs the board at least 1.24 mm longer than the
# lower (1.42 mm with KiCad's stock footprint, whose edge line sat 0.16 mm
# further forward; at 1.0 mm KiCad reported a peg hole inside J6's
# courtyard). 1.5 also leaves the receptacle's pad row and shell legs clear
# of the vias J6's last contacts need. Nothing overhangs sideways.
USB_EXTRA = 1.5
# And PROG_EXTRA longer at its pin 1 end (y 0 -> -4.0), for the SOICbite
# J3 at that edge: a test clip's jaws take the edge strip on both faces,
# its pads, the row of holes the clip's comb drops into and the jaw wall
# behind them (the footprint's courtyard, 2.92 mm), and no part may stand
# in them. Inside the lower board's length the underside's pin 1 end is
# the regulator's (U5, C1, C2, FB1) and the UART/switch resistors', with
# the AG32 right behind them, and the top's is the image-select switch's,
# with the AG32's decoupling right behind it: neither face has 3 mm to
# give up, nor anywhere else to put what is there. So the board grows
# instead. At 4.0 mm the courtyard ends 1.08 mm short of the lower board's
# end, 1.58 mm before the first part underneath (U5) and 3.10 mm before
# SW1 on top (1.10 mm before R24/R25, 0402s in the top's left corner,
# moved there to route J3; nothing else moved). The extra 4 mm overhangs
# the host past the socket's pin 1 end at about 10 mm, as the USB end
# does (DESIGN_NOTES.md, "J3 at the pin 1 end").
PROG_EXTRA = 4.0
UPPER_OUTLINE = [(BOARD_X0, -PROG_EXTRA, BOARD_X0 + BOARD_W,
                  BOARD_L + USB_EXTRA)]
# The module's pin 1 end is marked the way a DIP's is, by the board's
# shape: each board's corner at socket pin 1's column on its pin 1 end
# ("tl": the left edge and the low-y end, as drawn) is cut off at 45
# degrees, CHAMFER along each edge. On the lower board that is the
# corner beside J1 pin 1: its pad (1.6 mm, centred 1.70 mm from both
# edges) keeps 0.90 mm from the cut, as from the two edges. On the upper
# board it is the J3 end's left corner, outside the SOICbite clip's
# zone (the end's middle 7.8 mm, which leaves 1.61 mm at each corner).
# The cut is not part of the routing fingerprint (spec "outline"
# stays the rectangle); stitch() keeps ground vias clear of it, and
# --amend moves any stored stitch it cuts (DESIGN_NOTES.md, "Silkscreen
# marking").
CHAMFER = 1.0
# Board-to-board connector centre, shared by J5 (lower, top) and J6 (upper,
# underside). Along the socket, it is where the lower board's top side fits
# U3 above it and U4 below it with nothing to spare (see LOWER); across, it
# is 0.5 mm right of the channel's centre line, which makes the 1 mm gap
# beside its lower-left end that OE's pull-up R21 sits in.
CONN_X, CONN_Y = 7.5, 16.55



# ---------------------------------------------------------------------------
# Boards. place: ref -> (x, y, rotation_deg, side). Side "B" parts are flipped
# left-right about their own centre first and then rotated, which is what
# KiCad does when a part is flipped in the editor.
#
# planes: (net, layer, zone - None for the whole board, a rectangle, or
# {"poly": points} - and fill priority).
# route_layers: the layers ordinary nets are routed on; a plane net may also
# use its own plane's layer (see freerouting_layers).
# ---------------------------------------------------------------------------
# Parts marked FIXED are pinned for mechanical reasons; everything else is
# the layout's choice, as described here.
#
# LOWER: 4 layers - F and B for parts and routing, In1 and In2 solid
# ground planes. Everything lives in the channel between the DIP's two pin
# columns: 5.52 mm wide on top (between the pin tails' solder fillets) and
# 4.58 mm underneath (between the strips' insulators, which stand 2.79 mm
# proud of the board on the host socket; every part under here is 1 mm or
# less, and J1's courtyard underneath is the insulators). The pin field is
# x 3.19 / 10.81, pins 1-10 down the left, 20-11 down the right.
#
#   Top, from the pin 1 end: C18 at socket pin 20; the buffers' regulator
#   U7 with C12 (in), C13 (out, upright below U7) and its enable delay
#   R26/C17, over U2, their grounds dropping into U2's exposed pad; the
#   test pad TP1 (+3V3_BUF) below-left of U7, in the open with the upper
#   board lifted off; R14 under C17 in the column at the right; U3
#   (receiver for socket 15-19, turned 180: inputs face the right column,
#   in the socket's order, outputs the left); its capacitor C9 at its
#   supply end;
#   the header J5; and U4 (the drive buffer) turned 90 across the full
#   channel width, its B side facing J5's far end and its A side the
#   board's end, with OE's pull-up R21 beside its supply corner. U3's five
#   outputs comb into J5's left row and U4's nine go straight up into J5's
#   far end, all on top (fans.py).
#   Underneath: U2 (receiver for socket 1-5) at the pin 1 end, mirrored, so
#   its pins run in the reverse of the socket's order on both sides: two
#   inputs go round the outside of the pin column, two by via on top, and
#   its five outputs run under U3 to vias in J5's lane (fans.corridor);
#   C8 at its supply end; the pulls in a column beside the right pin
#   column, each on its pin's level; U4's capacitor C10 under its supply
#   end; the terminator packs RN2 (across, under U4's right half, its A row
#   over the A vias) and RN1 (upright, under J5's end).
#   Socket power runs underneath outside the right pin column to J5 and on
#   top along the board's end to U7; +3V3_BUF underneath down the left side
#   and outside the left pin column to C10.
LOWER = {
    "title": "FastPROM lower board - socket, bus buffers, stack connector",
    # FIXED: the outline is the socket's pin field, 1.70 mm past the pin
    # rows each end, and must not grow sideways (neighbouring parts on the
    # host); J1's pins are the DIP-20 the host socket takes.
    "outline": OUTLINE,
    "chamfer": ("tl", CHAMFER),             # the module's pin 1 corner
    "place": {
        "J1": (DIP_X0, DIP_Y0, 0, "F"),                       # FIXED
        # FIXED in that it must mate J6, at the same (x, y)
        "J5": (CONN_X, CONN_Y, 270, "F"),
        "U4": (FANS.U4_X, FANS.U4_Y, 90, "F"),
        "C10": (5.22, 23.3, 270, "B"),
        "C18": (8.8, 1.05, 180, "F"),
        "R21": (4.75, 19.5, 90, "F"),
        "U3": (7.5, 8.49, 180, "F"),
        "C9": (6.6, 11.75, 180, "F"),
        "R14": (FANS.COL_X, 5.1, 180, "F"),
        "U2": (7.0, 2.85, 0, "B"),
        "C8": (FANS.C8_X, FANS.C8_Y, 180, "B"),
        "U7": (6.5, 2.55, 90, "F"),
        "C12": (5.3, 1.05, 0, "F"),
        "C13": (FANS.C13_X, FANS.C13_Y, 90, "F"),
        "R26": (FANS.COL_X, 2.75, 180, "F"),
        "C17": (FANS.COL_X - 0.03, 4.05, 180, "F"),
        "R13": (8.77, 7.37, 270, "B"),
        "R12": (8.77, 9.35, 270, "B"),
        "R16": (8.77, 11.33, 90, "B"),
        "R20": (8.77, 13.31, 90, "B"),
        "R15": (8.77, 15.29, 270, "B"),
        "RN1": (5.755, 20.75, 0, "B"),
        "RN2": (7.98, 23.85, 270, "B"),
        # bare test pad on top, below-left of U7, probed in the host with
        # the upper board lifted off (fans.py)
        "TP1": (FANS.TP1_LO[0], FANS.TP1_LO[1], 0, "F"),
    },
    "prewire_routes": FANS.lower(),
    # Marking. Pin 1 marks are the footprints' own: one standard triangle
    # each (hw/lib/fastprom.pretty/README.md), J1's beside socket pin 1.
    # Up the top's left strip, between the pin 1-10 column's pads
    # and U3/J5 (both covered by the upper board when stacked): the test
    # pad's rail, just below the pad, then the name, 0.34 mm after the
    # label and 0.2 mm short of R21's pad; the copyright under J5 on the
    # underside, between the pin strips' insulators, 0.2 mm clear of the
    # pull-ups' column (DESIGN_NOTES.md, "Silkscreen rules").
    "silk": [
        ("text", "F", "3V3B", 4.82, 6.6, 90),
        ("text", "F", f"FastPROM rev {REV} LOWER", 4.82, 13.3, 90),
        ("text", "B", "\u00a9 2026", 6.05, 13.3, 90),
        ("text", "B", "mxshift", 7.6, 13.3, 90),
    ],
    "route_layers": ["F.Cu", "B.Cu"],
    "planes": [
        ("GND", "In1", None, 0),
        ("GND", "In2", None, 0),
    ],
}

# UPPER: 4 layers, the same stack as the lower board (In1 and In2 ground),
# 11.02 x 31.76 mm (y -4.0 to 27.76). Turned end for end against the
# lower board's pin 1 end: the programming connector J3, a SOICbite, at the
# pin 1 end's edge, on the 4 mm the board overhangs the lower board there
# (PROG_EXTRA), and the image-select switch SW1 on top behind it, where it
# stays reachable with the module seated; the USB-C receptacle J4 on top at
# the far end, opening outward.
#   Underneath: the AG32 U1 (turned 180) between the regulator U5 and the
# board-to-board socket J6, its bottom edge just above J6's pin 1 end;
# tools/boardmap.yaml gives each J6 contact the AG32 pin on the side
# facing it. Its left and bottom edges' I/O comb into J6's left row and its
# right edge's into the right row, and its far edge's five data lines wrap
# round the corner into the right row outside them (fans.py); two lines
# change sides on top to get past others (the drive buffer's enable, and
# BUF_D7 beside J6). U5 under SW1, its output capacitor C2 beside its
# output pin, the ferrite FB1 on its other side, its input capacitor C1
# below FB1 and the UART pull-downs R18 and R17 left of FB1, under SW1's
# left pads (DESIGN_NOTES.md, "Assembly panel"); the USB clamp's
# capacitor C19 beside J6's
# left row; under the receptacle the CC resistors R22 and R23 and the VBUS
# damper R27/C20.
#   On top, the switch's series resistors R24/R25 between J3's strip and
# SW1, at the left edge (fans.upper_prog). Over the AG32: its supply
# capacitors at the supply pins' vias
# (C5 at pin 32, C4 at pin 16 with a via in its ground pad, C3 at pin 6),
# the oscillator X1 over the middle with C16, C14 at the left edge, C15
# on the supply spine below X1 (fans.upper_mcu_supply), the BOOT0 resistor
# R10, and the NRST network R9/C11 down the right (C11 1.55 mm in from the
# edge, clear of the panel's mid-board tab). Over J6: the USB ESD clamp U6
# at the left (its courtyard inside the edge, for the tab's mouse bites),
# the 5 V OR-ing diodes D2 and D3 at the right, BOOT1's strap R11 over
# J6's lane, and the test pads TP3 (+3V3_MCU) between them and TP2
# (+5V_USB) behind the receptacle. The 5 V runs from the diodes up the
# right edge on top to U5.
#   Nothing on the underside is taller than the SOT-23-5 regulator (1.45
# mm, over the lower board's U7 and its capacitors: 2.0 mm of the 3.0 mm
# gap); the 0603s C2 and C1 at the pin 1 end, over the right pin column,
# leave 0.55 mm over a 1.5 mm pin tail (see docs/07-bom.md).
UPPER = {
    "title": "FastPROM upper board - AG32 fabric ROM, USB, power",
    # FIXED: the lower board's width; USB_EXTRA longer at the USB end and
    # PROG_EXTRA at the pin 1 end
    "outline": UPPER_OUTLINE,
    "chamfer": ("tl", CHAMFER),             # the module's pin 1 corner
    "copper": 4,
    "route_layers": ["F.Cu", "B.Cu"],
    "place": {
        # FIXED: USB-C at the end, opening outward, on top
        "J4": (CENTRE_X, BOARD_L + USB_EXTRA - 3.19, 0, "F"),
        # FIXED: mates J5 (gen_layout's mate() turns it to fit)
        "J6": (CONN_X, CONN_Y, 90, "B"),
        "SW1": (CENTRE_X, 4.45, 0, "F"),
        # FIXED: the SOICbite at the pin 1 end's edge, pin 1 on the left.
        # Turned 180, its Fab line at the edge: pads 0.381 mm in (the
        # board's copper-to-edge rule is 0.30) and 1.5 mm long, the clip's
        # five holes 2.29 mm in, courtyard to 2.92 mm.
        "J3": (CENTRE_X, -PROG_EXTRA + 0.381, 180, "F"),
        "U1": (7.0, 9.0, 180, "B"),
        "C5": (8.4, 7.4, 180, "F"),
        "X1": (8.0, 9.26, 0, "F"),
        "C3": (9.75, 11.25, 180, "F"),
        "C4": (4.45, 11.21, 90, "F"),
        "C16": (5.75, 9.26, 270, "F"),
        "R10": (6.4, 7.4, 180, "F"),
        "R9": (11.0, 9.75, 90, "F"),
        "C11": (10.0, 12.3, 180, "F"),
        # C14 and C19 at the left edge keep 1.5 mm along it from the
        # panel's mid-board tab, and C2 1.9 mm in from the right edge
        # beside its J3-end tab (gen_panel.py)
        "C14": (2.5, 8.75, 270, "F"),
        "C15": (7.89, 12.3, 0, "F"),
        "U5": (7.3, 2.6, 90, "B"),
        # the ferrite FB1 beside U5 (2.5 mm in from the left edge), its
        # input capacitor C1 below FB1 (3.2 mm along the edge from the
        # panel's J3-end tab) and the UART pull-downs R17/R18 left of FB1:
        # no ceramic chip beside a tab (gen_panel.py, ceramic_clear)
        "C2": (9.85, 2.6, 270, "B"),        # beside U5's output pin
        "FB1": (4.77, 2.6, 270, "B"),
        "C1": (4.02, 5.0, 180, "B"),
        "D2": (10.3, 13.85, 180, "F"),
        "D3": (10.3, 16.05, 180, "F"),
        "R23": (8.4, 24.0, 270, "B"),
        "U6": (FANS.U6_X, 16.0, 270, "F"),
        "C19": (2.6, 19.35, 270, "B"),
        "R22": (5.0, 23.4, 270, "B"),
        "R27": (9.45, 23.5, 270, "B"),
        "C20": (7.1, 24.2, 270, "B"),
        "R11": (7.45, 18.25, 90, "F"),
        # the switch's series resistors on top, beside J3's strip, their
        # UART pads up (fans.upper_prog)
        "R24": (2.75, 1.0, 270, "F"),
        "R25": (3.85, 1.0, 90, "F"),
        # the UART pull-downs left of the ferrite, sharing a ground via
        # between their ground pads (fans.upper_prog)
        "R18": (2.95, 1.6, 0, "B"),
        "R17": (2.95, 3.6, 0, "B"),
        # bare test pads on top, reachable with the module seated
        "TP2": (FANS.TP2_UP[0], FANS.TP2_UP[1], 0, "F"),
        "TP3": (FANS.TP3_UP[0], FANS.TP3_UP[1], 0, "F"),
    },
    # the AG32's supplies (a few tens of mA): 0.25 mm, which reaches
    # its 0.2 mm pads and leaves the crowded side of the board room
    "net_classes": {"Supply": (0.25, ("+3V3", "+3V3_MCU"))},
    "gnd_pins": ["J4", "J3"],
    # ground pads with a hand-routed via in them, besides the exposed pads:
    # C4's, the one place its ground via clears the AG32's bundle
    # underneath and the USB pair on top (fans.py). Vias are filled and
    # capped (hw/ORDER-JLCPCB.md), so the pad stays flat.
    "via_in_pad": [("C4", "2")],
    "prewire_routes": FANS.upper(),
    # Marking. On top, the only face that shows with the module seated.
    # The module's pin 1 is the chamfered corner (CHAMFER; plugging the
    # module in reversed puts 5 V on the wrong pins), not a silk mark,
    # which beside R24/R25 read as theirs. The parts' pin 1
    # triangles are their footprints' own (hw/lib/fastprom.pretty/
    # README.md). "PROG" between J3 and SW1; the name without "UPPER"
    # (the stacked module's only visible face is this board's top; the
    # full name, 10.5 mm of ink at JLC's 0.6 mm minimum character width,
    # is longer than the 10.0 mm the strip leaves between C16 and the
    # receptacle's pads), up the strip between C4/U6 and X1/TP3; "IMG"
    # below the image-select switch's pins 3-4 (the part prints ON and the
    # switch numbers, which are the image bits); and each test pad's rail
    # (3V3M between C15 and TP3, clear of the name and D2). Underneath,
    # the full name and the copyright along the two edge strips.
    "silk": [
        ("text", "F", "PROG", CENTRE_X, -0.45, 0),
        ("text", "F", f"FastPROM rev {REV}", 5.8, 15.175, 90),
        ("text", "F", "IMG", 10.9, 6.3, 0),
        ("text", "F", "3V3M", 7.64, 13.6, 0),
        ("text", "F", "5VU", 11.0, FANS.TP2_UP[1], 0),
        ("text", "B", f"FastPROM rev {REV} UPPER", 2.45, 11.3, 90),
        ("text", "B", "\u00a9 2026 mxshift", 11.5, 12.0, 90),
    ],
    "planes": [
        ("GND", "In1", None, 0),
        ("GND", "In2", None, 0),
    ],
}

BOARDS = {
    "fastprom-lower": LOWER,
    "fastprom-upper": UPPER,
}

LAYER_ID = {"In1": pcbnew.In1_Cu, "In2": pcbnew.In2_Cu,
            "In3": pcbnew.In3_Cu, "In4": pcbnew.In4_Cu}
PWR_NETS = ("+5V_SKT", "/+5V", "+5V_USB", "+3V3_BUF")
# Pins whose copper is tied to ground in layout though the symbol has no pin
# for it: the exposed pads. TI recommends connecting the thermal pad to
# ground.
EP_GND = {("U2", "21"), ("U3", "21"), ("U4", "21"), ("U1", "33")}


# ---------------------------------------------------------------------------
def mm(v):
    return pcbnew.FromMM(v)


def nm(v):
    """mm to nm, rounded. pcbnew.FromMM truncates (16.15 mm is 16149999
    nm), which leaves pre-route copper 1 nm off the end of a Freerouting
    track (on its 0.1 um grid) that meets it - KiCad joins them anyway, but
    route.prune compares ends exactly, so the pre-route copper uses this."""
    return int(round(v * 1e6))


def pt(x, y):
    return pcbnew.VECTOR2I(mm(OX + x), mm(OY + y))


def to_local(v):
    return pcbnew.ToMM(v.x) - OX, pcbnew.ToMM(v.y) - OY


# The routing file's frame: nanometres from the same origin
NX, NY = mm(OX), mm(OY)


def load_fp(fpid):
    lib, name = fpid.split(":", 1)
    path = LOCAL if lib == "fastprom" else os.path.join(STOCK, lib + ".pretty")
    fp = pcbnew.FootprintLoad(path, name)
    if fp is None:
        raise SystemExit(f"footprint not found: {fpid}")
    # FootprintLoad leaves the library nickname empty, which DRC's schematic
    # parity check reports as a different footprint
    fp.SetFPID(pcbnew.LIB_ID(lib, name))
    return fp


def place(board, ref, comp, where):
    fp = load_fp(comp["footprint"])
    fp.SetReference(ref)
    fp.SetValue(comp["value"])
    fp.SetDNP(bool(comp.get("dnp")))
    # the symbol's BOM attribute, as KiCad's update from the schematic sets
    # it (the stock test pad footprint excludes itself; its symbol doesn't)
    if "exclude_from_bom" in comp:
        fp.SetExcludedFromBOM(comp["exclude_from_bom"])
    x, y, rot, side = where
    fp.SetPosition(pt(x, y))
    # References are kept on the fab layers only: JLC's 1 mm minimum text
    # height does not fit next to 0402 parts on a board this size.
    fp.Reference().SetVisible(False)
    fp.Value().SetVisible(False)
    board.Add(fp)
    if side == "B":
        fp.Flip(fp.GetPosition(), pcbnew.FLIP_DIRECTION_LEFT_RIGHT)
    fp.SetOrientationDegrees(rot)
    return fp


def pad_centres(fp):
    return {p.GetNumber(): to_local(p.GetPosition()) for p in fp.Pads()
            if p.GetNumber()}


def mate(upper_fp, lower_fp):
    """Rotate the upper connector until every contact lands on the lower
    connector's contact of the same number. Raises if no rotation does."""
    want = pad_centres(lower_fp)
    best = None
    for rot in (0, 90, 180, 270):
        upper_fp.SetOrientationDegrees(rot)
        got = pad_centres(upper_fp)
        err = max(math.hypot(got[n][0] - want[n][0], got[n][1] - want[n][1])
                  for n in want if n in got and n != "MP")
        if best is None or err < best[0]:
            best = (err, rot)
    upper_fp.SetOrientationDegrees(best[1])
    # The two halves' pads differ in length (0.66 vs 0.70 mm) and row pitch
    # (2.71 vs 3.08 mm); contact centrelines must coincide along the
    # connector and sit within the row offset across it.
    if best[0] > 0.2:
        raise SystemExit(f"connector halves do not mate: worst {best[0]:.3f}")
    return best


def stitched_nets(spec):
    """Nets with a plane on two or more layers: a via of one joins its
    planes, whatever else it meets."""
    count = {}
    for (net, *_rest) in spec["planes"]:
        count[net] = count.get(net, 0) + 1
    return {n for n, c in count.items() if c > 1}


def plane_class(net):
    return "Plane_" + "".join(c if c.isalnum() else "_" for c in net)


def seed(name, stage):
    """Make the UUIDs KiCad hands out from here on a function of the board
    and the build stage, so a rebuild writes byte-identical files."""
    pcbnew.KIID.SeedGenerator(int.from_bytes(
        hashlib.sha256(f"{name}/{stage}".encode()).digest()[:4], "big"))


def fresh_project(name):
    """NewBoard loads the project file beside the board, and what it starts
    from shows in what it saves (the order and ids of what it makes). That
    file is written by gen_kicad.py (its template) and again by every save
    here (KiCad's full settings, the net classes below), so the board built
    would depend on which ran last. Every build starts from gen_kicad.py's
    template, its root sheet the schematic's own (as gen_wired.py writes
    it), never what an earlier build left. The panel has no schematic; its
    root sheet gets a fixed id from its name."""
    import gen_kicad                # noqa: E402 (tools/, no pcbnew needed)
    path = os.path.join(HW, name + ".kicad_pro")
    sch = os.path.join(HW, name + ".kicad_sch")
    if os.path.exists(sch):
        with open(sch) as fh:
            m = re.search(r'^\s*\(uuid "([0-9a-f-]+)"\)', fh.read(), re.M)
        if not m:
            raise SystemExit(f"hw/{name}.kicad_sch has no uuid; run make kicad")
        sheet = m.group(1)
    else:
        sheet = str(uuid.uuid5(uuid.NAMESPACE_URL, f"fastprom/{name}"))
    pro = json.loads(gen_kicad.PROJECT_TEMPLATE
                     .replace("SHEETUUID", sheet)
                     .replace("fastprom.kicad_pro", name + ".kicad_pro"))
    with open(path, "w") as fh:
        json.dump(pro, fh, indent=2)
        fh.write("\n")


def title_block(board, title):
    """The schematics' title block, on the board (the PDFs print it): no
    date, so a rebuild stays byte-identical."""
    tb = pcbnew.TITLE_BLOCK()
    tb.SetTitle(title)
    tb.SetRevision(REV)
    tb.SetCompany("FastPROM")
    tb.SetComment(0, HW_NOTICE)
    board.SetTitleBlock(tb)


def chamfer_cut(spec):
    """The pin 1 corner's cut (spec "chamfer"): the corner (x, y), the
    signs (sx, sy) pointing into the board from it, and the size; None
    for a plain rectangle."""
    if not spec.get("chamfer"):
        return None
    corner, c = spec["chamfer"]
    if len(spec["outline"]) != 1:
        raise SystemExit("a chamfer needs a one-rectangle outline")
    x0, y0, x1, y1 = spec["outline"][0]
    cx, sx = (x0, 1) if corner[1] == "l" else (x1, -1)
    cy, sy = (y0, 1) if corner[0] == "t" else (y1, -1)
    return cx, cy, sx, sy, c


def chamfer_in(spec, x, y):
    """How far (x, y) (mm) lies inside the chamfer's cut line; infinite
    with no chamfer. Inside the board's rectangle, a point is on the board
    when this is positive."""
    cut = chamfer_cut(spec)
    if cut is None:
        return math.inf
    cx, cy, sx, sy, c = cut
    return (sx * (x - cx) + sy * (y - cy) - c) / math.sqrt(2)


def board_poly(spec):
    """The board outline's corners (mm, the frame), chamfer included."""
    x0, y0, x1, y1 = (min(r[0] for r in spec["outline"]),
                      min(r[1] for r in spec["outline"]),
                      max(r[2] for r in spec["outline"]),
                      max(r[3] for r in spec["outline"]))
    pts = [(x0, y0), (x1, y0), (x1, y1), (x0, y1)]
    cut = chamfer_cut(spec)
    if cut is None:
        return pts
    cx, cy, sx, sy, c = cut
    i = pts.index((cx, cy))
    # the corner gives way to two points, in the outline's direction
    prev, nxt = pts[i - 1], pts[(i + 1) % 4]

    def towards(q):
        d = math.hypot(q[0] - cx, q[1] - cy)
        return (cx + (q[0] - cx) * c / d, cy + (q[1] - cy) * c / d)
    return pts[:i] + [towards(prev), towards(nxt)] + pts[i + 1:]


def build(name, spec, comps, nets, keep=True):
    """keep: this run saves the board to hw/. NewBoard writes an empty board
    at the path it is given at once, so a board built only for the connector
    check (the other half of an --only run) is made under build/ instead."""
    seed(name, "build")
    if keep:
        fresh_project(name)
        board = pcbnew.NewBoard(os.path.join(HW, name + ".kicad_pcb"))
    else:
        scratch = os.path.join(ROOT, "build", "layout", "scratch")
        os.makedirs(scratch, exist_ok=True)
        board = pcbnew.NewBoard(os.path.join(scratch, name + ".kicad_pcb"))
    title_block(board, spec["title"])
    board.SetCopperLayerCount(spec.get("copper", 4))
    for (_, layer, _, _) in spec["planes"]:
        board.SetLayerType(LAYER_ID[layer], pcbnew.LT_POWER)
    ds = board.GetDesignSettings()
    ds.SetBoardThickness(mm(THICKNESS))
    ds.m_MinClearance = mm(RULES["clearance"])
    ds.m_TrackMinWidth = mm(RULES["min_track"])
    ds.m_ViasMinSize = mm(RULES["via_d"])
    ds.m_MinThroughDrill = mm(RULES["via_drill"])
    ds.m_ViasMinAnnularWidth = mm(0.1)
    ds.m_CopperEdgeClearance = mm(RULES["edge"])
    ds.m_HoleClearance = mm(RULES["hole_clearance"])
    ds.m_HoleToHoleMin = mm(0.25)
    ds.m_SolderMaskMinWidth = mm(0.0)
    silk_settings(ds)

    # Net classes: the default for signals, Power for the 5 V nets, and one
    # per plane net (Freerouting needs those apart; see freerouting_layers)
    ns = ds.m_NetSettings
    # NewBoard loads the project file already beside the board; start from
    # no assignments, or a stale one from an earlier build survives
    ns.ClearNetclassPatternAssignments()

    # The schematic's wire, bus and line style live in the same net classes
    # (the project file is shared). A new NETCLASS carries KiCad's defaults
    # for them; the default net class from GetDefaultNetclass() carries none,
    # and saved that way its wires plot with no stroke: the schematic PDFs
    # and SVGs showed no signal wires at all. Give every class KiCad's own
    # defaults, taken from a fresh NETCLASS rather than restated here.
    sch = pcbnew.NETCLASS("defaults")

    def netclass(nc, width):
        nc.SetWireWidth(sch.GetWireWidth())
        nc.SetBusWidth(sch.GetBusWidth())
        nc.SetLineStyle(sch.GetLineStyle())
        nc.SetClearance(mm(RULES["clearance"]))
        nc.SetTrackWidth(mm(width))
        nc.SetViaDiameter(mm(RULES["via_d"]))
        nc.SetViaDrill(mm(RULES["via_drill"]))
        return nc
    netclass(ns.GetDefaultNetclass(), W_SIG)
    ns.SetNetclass("Power", netclass(pcbnew.NETCLASS("Power"), W_PWR))
    for n in PWR_NETS:
        if n in nets:
            ns.SetNetclassPatternAssignment(n, "Power")
    # other supply nets with a width of their own
    for cls, (width, members) in spec.get("net_classes", {}).items():
        ns.SetNetclass(cls, netclass(pcbnew.NETCLASS(cls), width))
        for n in members:
            if n in nets:
                ns.SetNetclassPatternAssignment(n, cls)
    for n in sorted({p[0] for p in spec["planes"]}):
        ns.SetNetclass(plane_class(n),
                       netclass(pcbnew.NETCLASS(plane_class(n)), W_SIG))
        ns.SetNetclassPatternAssignment(n, plane_class(n))

    if spec.get("chamfer"):
        pts = board_poly(spec)
        for a, b in zip(pts, pts[1:] + pts[:1]):
            g = pcbnew.PCB_SHAPE(board, pcbnew.SHAPE_T_SEGMENT)
            g.SetStart(pt(*a))
            g.SetEnd(pt(*b))
            g.SetLayer(pcbnew.Edge_Cuts)
            g.SetWidth(mm(0.1))
            board.Add(g)
    else:
        for (x0, y0, x1, y1) in spec["outline"]:
            r = pcbnew.PCB_SHAPE(board, pcbnew.SHAPE_T_RECT)
            r.SetStart(pt(x0, y0))
            r.SetEnd(pt(x1, y1))
            r.SetLayer(pcbnew.Edge_Cuts)
            r.SetWidth(mm(0.1))
            board.Add(r)

    netinfo = {}
    for n in nets:
        ni = pcbnew.NETINFO_ITEM(board, n)
        board.Add(ni)
        netinfo[n] = ni
    pinnet = {}
    for n, nodes in nets.items():
        for ref, pin in nodes:
            pinnet[(ref, pin)] = n
    for (ref, pin) in EP_GND:
        if ref in comps:
            pinnet[(ref, pin)] = "GND"

    fps = {}
    missing = [r for r in comps if r not in spec["place"]]
    if missing:
        raise SystemExit(f"{name}: no placement for {', '.join(missing)}")
    for ref, comp in sorted(comps.items()):
        fp = place(board, ref, comp, spec["place"][ref])
        for p in fp.Pads():
            n = pinnet.get((ref, p.GetNumber()))
            if n:
                p.SetNet(netinfo[n])
        fps[ref] = fp
    ns.RecomputeEffectiveNetclasses()
    board.SynchronizeNetsAndNetClasses(True)
    add_silk(board, spec.get("silk", []))
    return board, fps, netinfo


# Silkscreen marking, as data in each board's spec "silk":
#   ("text", side, text, x, y, rotation)  centred at (x, y)
#   ("poly", side, [(x, y), ...])         filled
#   ("line", side, (x0, y0), (x1, y1))    one stroke
#   ("arc", side, (cx, cy), (sx, sy), angle_deg)
#   ("pin1", side, (x, y), "up"|"down"|"left"|"right")  the standard pin 1
#                                         triangle, its tip at (x, y)
# JLC's legend rules (jlcpcb.com/capabilities/pcb-capabilities): text at
# least 1.0 mm high, strokes at least 0.15 mm (1:6 of the height), 0.15 mm
# from pads (RULES; DESIGN_NOTES.md, "Silkscreen rules"). Text here is
# 1.0 mm high, 0.6 mm wide (condensed to fit the narrow strips these boards
# have free), 0.17 mm strokes; underside text is mirrored, to read from
# below. 0.6 mm is as narrow as it gets: KiCad draws a character's strokes
# no wider than a quarter of its width (0.55 mm text printed 0.1375 mm).
SILK_H, SILK_W, SILK_T, SILK_LINE = 1.0, 0.6, 0.17, 0.17
# The pin 1 triangle every footprint in hw/lib/fastprom.pretty carries
# (README.md there): filled, 0.5 mm base, 0.35 mm tip to base, a 0.15 mm
# stroke round it (0.65 x 0.5 mm printed), its tip 0.2 mm off pad 1.
PIN1_BASE, PIN1_HEIGHT, PIN1_STROKE = 0.5, 0.35, 0.15


def add_silk(board, items):
    for it in items:
        kind, side = it[0], it[1]
        layer = pcbnew.F_SilkS if side == "F" else pcbnew.B_SilkS
        if kind == "text":
            _, _, text, x, y, rot = it
            t = pcbnew.PCB_TEXT(board)
            t.SetText(text)
            t.SetLayer(layer)
            t.SetTextSize(pcbnew.VECTOR2I(mm(SILK_W), mm(SILK_H)))
            t.SetTextThickness(mm(SILK_T))
            t.SetHorizJustify(pcbnew.GR_TEXT_H_ALIGN_CENTER)
            t.SetVertJustify(pcbnew.GR_TEXT_V_ALIGN_CENTER)
            t.SetPosition(pt(x, y))
            t.SetTextAngle(pcbnew.EDA_ANGLE(rot, pcbnew.DEGREES_T))
            t.SetMirrored(side == "B")
            board.Add(t)
        elif kind == "poly":
            g = pcbnew.PCB_SHAPE(board, pcbnew.SHAPE_T_POLY)
            g.SetPolyPoints([pt(x, y) for (x, y) in it[2]])
            g.SetFilled(True)
            g.SetWidth(mm(SILK_LINE))
            g.SetLayer(layer)
            board.Add(g)
        elif kind == "pin1":
            _, _, (x, y), d = it
            ux, uy = {"up": (0, -1), "down": (0, 1),
                      "left": (-1, 0), "right": (1, 0)}[d]
            h, w = PIN1_HEIGHT, PIN1_BASE / 2
            pts = [(x, y), (x - ux * h - uy * w, y - uy * h - ux * w),
                   (x - ux * h + uy * w, y - uy * h + ux * w)]
            g = pcbnew.PCB_SHAPE(board, pcbnew.SHAPE_T_POLY)
            g.SetPolyPoints([pt(px, py) for (px, py) in pts])
            g.SetFilled(True)
            g.SetWidth(mm(PIN1_STROKE))
            g.SetLayer(layer)
            board.Add(g)
        elif kind == "line":
            g = pcbnew.PCB_SHAPE(board, pcbnew.SHAPE_T_SEGMENT)
            g.SetStart(pt(*it[2]))
            g.SetEnd(pt(*it[3]))
            g.SetWidth(mm(SILK_LINE))
            g.SetLayer(layer)
            board.Add(g)
        elif kind == "arc":
            _, _, c, st, ang = it
            g = pcbnew.PCB_SHAPE(board, pcbnew.SHAPE_T_ARC)
            g.SetCenter(pt(*c))
            g.SetStart(pt(*st))
            g.SetArcAngleAndEnd(pcbnew.EDA_ANGLE(ang, pcbnew.DEGREES_T))
            g.SetWidth(mm(SILK_LINE))
            g.SetLayer(layer)
            board.Add(g)


def placement_fingerprint(board, spec, rename=None):
    """rename: {net: name} applied to pad nets before hashing (apply_route's
    proof that a net was only renamed)."""
    rename = rename or {}
    pads = []
    for fp in board.GetFootprints():
        for p in fp.Pads():
            q = p.GetPosition()
            n = p.GetNetname()
            pads.append((fp.GetReference(), p.GetNumber(), rename.get(n, n),
                         q.x - NX, q.y - NY, int(fp.IsFlipped())))
    return RT.fingerprint(pads, spec["outline"])


def dump(name, board, spec, path):
    """Geometry for previews: pads, courtyards, tracks, vias, outline."""
    out = {"pads": [], "fps": [], "tracks": [], "vias": [],
           "outline_rects": spec["outline"]}
    for fp in board.GetFootprints():
        ref = fp.GetReference()
        side = "B" if fp.IsFlipped() else "F"
        # the cached shapes are from wherever the part was last built, not
        # where it now stands
        fp.BuildCourtyardCaches()
        cy = fp.GetCourtyard(pcbnew.B_CrtYd if side == "B"
                             else pcbnew.F_CrtYd).BBox()
        x0, y0 = to_local(cy.GetOrigin())
        x1, y1 = to_local(cy.GetEnd())
        out["fps"].append({"ref": ref, "side": side,
                           "bbox": [x0, y0, x1, y1],
                           "at": to_local(fp.GetPosition())})
        for p in fp.Pads():
            bb = p.GetBoundingBox()
            a = to_local(bb.GetOrigin())
            b = to_local(bb.GetEnd())
            layers = [s for s, L in (("F", pcbnew.F_Cu), ("B", pcbnew.B_Cu))
                      if p.IsOnLayer(L)]
            out["pads"].append({
                "ref": ref, "num": p.GetNumber(), "net": p.GetNetname(),
                "bbox": [a[0], a[1], b[0], b[1]], "layers": layers,
                "round": p.GetShape(pcbnew.F_Cu) == pcbnew.PAD_SHAPE_CIRCLE,
                "drill": pcbnew.ToMM(p.GetDrillSize().x)})
    for t in board.GetTracks():
        if t.GetClass() == "PCB_VIA":
            out["vias"].append([*to_local(t.GetPosition()), t.GetNetname()])
        else:
            out["tracks"].append([*to_local(t.GetStart()), *to_local(t.GetEnd()),
                                  pcbnew.ToMM(t.GetWidth()),
                                  {pcbnew.F_Cu: "F", pcbnew.B_Cu: "B"}.get(
                                      t.GetLayer(), "I"),
                                  t.GetNetname()])
    out["keepouts"] = []
    for z in board.Zones():
        if z.GetIsRuleArea() and z.GetDoNotAllowTracks():
            bb = z.GetBoundingBox()
            a, b = to_local(bb.GetOrigin()), to_local(bb.GetEnd())
            for L in z.GetLayerSet().CuStack():
                out["keepouts"].append(
                    [{pcbnew.F_Cu: "F", pcbnew.B_Cu: "B"}.get(L, "I"),
                     a[0], a[1], b[0], b[1]])
    with open(path, "w") as fh:
        json.dump(out, fh, indent=0)


def overlaps(fps):
    """Courtyard collisions on the same side, as a list of strings: the real
    courtyard shapes, not their bounding boxes (J1's two strips have the
    channel between them). A quick check while placing; DRC has the final
    word."""
    bad = []
    items = []
    for ref, fp in fps.items():
        # the cached shapes are from wherever the part was last built, not
        # where it now stands
        fp.BuildCourtyardCaches()
        # both sides: J1 has a courtyard underneath too (the pin strips'
        # insulators), which the parts there must clear
        for side, layer in (("F", pcbnew.F_CrtYd), ("B", pcbnew.B_CrtYd)):
            cy = fp.GetCourtyard(layer)
            if cy.OutlineCount():
                items.append((ref, side, cy))
    for i, (ra, sa, a) in enumerate(items):
        for rb, sb, b in items[i + 1:]:
            if sa != sb or not a.BBox().Intersects(b.BBox()):
                continue
            # KiCad's DRC counts courtyards that only touch as overlapping
            x = pcbnew.SHAPE_POLY_SET(a)
            x.Inflate(1000, pcbnew.CORNER_STRATEGY_ROUND_ALL_CORNERS, 100)
            x.BooleanIntersection(b)
            if x.Area() > 0:
                bad.append(f"{ra}/{rb} ({sa})")
    return bad


def add_zones(board, netinfo, spec):
    """The inner planes (spec planes). The outer layers carry no pour: each
    ground pad has its own stub and via to the planes (prewire), and a pour
    there only broke up into islands between the 0.1 mm tracks
    (DESIGN_NOTES.md, "PCB process")."""
    def zone(net, layer, rect, prio):
        z = pcbnew.ZONE(board)
        z.SetLayer(layer)
        z.SetNet(netinfo[net])
        if isinstance(rect, dict):
            pts = rect["poly"]
        elif rect is None:
            pts = board_poly(spec)
        else:
            a, b, c, d = rect
            pts = ((a, b), (c, b), (c, d), (a, d))
        ol = z.Outline()
        ol.NewOutline()
        for (px, py) in pts:
            ol.Append(mm(OX + px), mm(OY + py))
        z.SetAssignedPriority(prio)
        z.SetMinThickness(mm(0.15))
        z.SetLocalClearance(mm(0.2))
        z.SetPadConnection(pcbnew.ZONE_CONNECTION_THERMAL)
        z.SetThermalReliefGap(mm(0.2))
        z.SetThermalReliefSpokeWidth(mm(0.25))
        board.Add(z)

    for (net, layer, rect, prio) in spec["planes"]:
        zone(net, LAYER_ID[layer], rect, prio)
    pcbnew.ZONE_FILLER(board).Fill(board.Zones())


def silk_settings(ds):
    """The silkscreen rules KiCad's board setup holds (RULES)."""
    ds.m_SilkClearance = mm(RULES["silk"])
    ds.m_MinSilkTextHeight = mm(RULES["silk_text_h"])
    ds.m_MinSilkTextThickness = mm(RULES["silk_text_t"])


def silk_rules():
    """The custom design rules (the board's .kicad_dru) for what KiCad's
    board setup has no setting for. "silk to pad": KiCad's silk-to-mask
    check (silk_over_copper) takes its clearance from the silk_clearance
    constraint, which the board setting leaves at zero against a pad; this
    asks RULES["silk_pad"] between any silkscreen item and any pad's mask
    opening, the part's own pads included. "silk line width": an assertion
    on every silkscreen line, arc, circle and polygon stroke (KiCad checks
    only text strokes)."""
    return (
        "(version 1)\n"
        "# Written by tools/gen_layout.py (silk_rules); DESIGN_NOTES.md,\n"
        "# \"Silkscreen rules\".\n"
        "(rule \"silk to pad\"\n"
        f"  (constraint silk_clearance (min {RULES['silk_pad']}mm))\n"
        "  (condition \"B.Type == 'Pad'\"))\n"
        "(rule \"silk line width\"\n"
        "  (condition \"A.Type == 'Graphic' && A.Layer == '?.Silkscreen'\")\n"
        f"  (constraint assertion \"A.Line_Width >= {RULES['silk_line']}mm\"))\n")


def project_rules(name):
    """Settings pcbnew's Python cannot reach, written into the project file
    it saves beside the board (kicad-cli DRC reads them from there), and the
    board's custom rules, in the .kicad_dru KiCad reads beside it."""
    path = os.path.join(HW, name + ".kicad_pro")
    with open(path) as fh:
        pro = json.load(fh)
    ds = pro.setdefault("board", {}).setdefault("design_settings", {})
    rules = ds.setdefault("rules", {})
    rules["min_track_width"] = RULES["min_track"]
    rules["min_silk_clearance"] = RULES["silk"]
    rules["min_text_height"] = RULES["silk_text_h"]
    rules["min_text_thickness"] = RULES["silk_text_t"]
    with open(path, "w") as fh:
        json.dump(pro, fh, indent=2)
        fh.write("\n")
    with open(os.path.join(HW, name + ".kicad_dru"), "w") as fh:
        fh.write(silk_rules())


# The fabricator's stack-up, JLCPCB's JLC04081H-3313 (0.8 mm, four layers):
# 1 oz outer copper, one 3313 prepreg each side, 0.5 oz inner copper on a
# 0.45 mm core. (layer, type, thickness mm, material, epsilon_r): the
# dielectric constants are JLC's for its 3313 prepreg and its core.
# DESIGN_NOTES.md, "Four layers on both boards".
STACKUP = [
    ("F.SilkS", "Top Silk Screen", None, None, None),
    ("F.Paste", "Top Solder Paste", None, None, None),
    ("F.Mask", "Top Solder Mask", 0.01, None, None),
    ("F.Cu", "copper", 0.035, None, None),
    ("dielectric 1", "prepreg", 0.0994, "3313", 4.1),
    ("In1.Cu", "copper", 0.0152, None, None),
    ("dielectric 2", "core", 0.45, "FR4", 4.6),
    ("In2.Cu", "copper", 0.0152, None, None),
    ("dielectric 3", "prepreg", 0.0994, "3313", 4.1),
    ("B.Cu", "copper", 0.035, None, None),
    ("B.Mask", "Bottom Solder Mask", 0.01, None, None),
    ("B.Paste", "Bottom Solder Paste", None, None, None),
    ("B.SilkS", "Bottom Silk Screen", None, None, None),
]
STACKUP_NAME = "JLC04081H-3313"


def stackup_sexpr():
    """The stack-up as the board file's (stackup ...) block. pcbnew's Python
    has no binding for BOARD_STACKUP, so save() writes it into the file;
    KiCad reads it back, and rewrites it in its own form, on the refill."""
    out = ["\t\t(stackup"]
    for (layer, typ, t, mat, er) in STACKUP:
        s = f'\t\t\t(layer "{layer}" (type "{typ}")'
        if t is not None:
            s += f" (thickness {t:g})"
        if mat is not None:
            s += f' (material "{mat}") (epsilon_r {er:g}) (loss_tangent 0.02)'
        out.append(s + ")")
    out += ['\t\t\t(copper_finish "ENIG")', "\t\t\t(dielectric_constraints no)",
            "\t\t)"]
    return "\n".join(out) + "\n"


def write_stackup(path):
    with open(path) as fh:
        text = fh.read()
    if "(stackup" in text:
        return
    head = "\t(setup\n"
    if text.count(head) != 1:
        raise SystemExit(f"{path}: no setup section for the stack-up")
    with open(path, "w") as fh:
        fh.write(text.replace(head, head + stackup_sexpr()))


def save(name, board, refill=False):
    path = os.path.join(HW, name + ".kicad_pcb")
    board.Save(path)
    write_stackup(path)
    project_rules(name)
    if refill:
        # A board made with NewBoard fills its zones without the copper-to-
        # edge rule (its rule engine has no project behind it), so the planes
        # would run to the board edge. Loaded from disk with its project
        # beside it, the fill honours every rule.
        b = pcbnew.LoadBoard(path)
        pcbnew.ZONE_FILLER(b).Fill(b.Zones())
        b.Save(path)


# ---------------------------------------------------------------------------
# The routing file
# ---------------------------------------------------------------------------
def route_path(name):
    return os.path.join(HW, name + ".route.json")


CU = {"F.Cu": pcbnew.F_Cu, "In1.Cu": pcbnew.In1_Cu, "In2.Cu": pcbnew.In2_Cu,
      "In3.Cu": pcbnew.In3_Cu, "In4.Cu": pcbnew.In4_Cu, "B.Cu": pcbnew.B_Cu}
CU_NAME = {v: k for k, v in CU.items()}


def extract(board):
    """The board's tracks and vias in the routing file's form."""
    tracks, vias = [], []
    for t in board.GetTracks():
        if t.GetClass() == "PCB_VIA":
            p = t.GetPosition()
            vias.append((t.GetNetname(), p.x - NX, p.y - NY, t.GetWidth(pcbnew.F_Cu),
                         t.GetDrillValue()))
        elif t.GetClass() == "PCB_TRACK":
            a, b = t.GetStart(), t.GetEnd()
            tracks.append((CU_NAME[t.GetLayer()], t.GetNetname(),
                           t.GetWidth(), a.x - NX, a.y - NY, b.x - NX, b.y - NY))
        else:
            raise SystemExit(f"unexpected {t.GetClass()} in the routing")
    return tracks, vias


def lay(board, netinfo, tracks, vias):
    """Add these tracks and vias to a board that has none."""
    made = {}
    for (L, net, w, x1, y1, x2, y2) in tracks:
        t = pcbnew.PCB_TRACK(board)
        t.SetStart(pcbnew.VECTOR2I(x1 + NX, y1 + NY))
        t.SetEnd(pcbnew.VECTOR2I(x2 + NX, y2 + NY))
        t.SetWidth(w)
        t.SetLayer(CU[L])
        t.SetNet(netinfo[net])
        # a fresh id, kept as text: reading m_Uuid back from a track after
        # the board has had tracks removed returns an untyped SWIG object
        kid = pcbnew.KIID()
        t.SetUuidDirect(kid)
        board.Add(t)
        made[(L, net, w, x1, y1, x2, y2)] = (t, kid.AsString())
    for (net, x, y, d, drill) in vias:
        v = pcbnew.PCB_VIA(board)
        v.SetPosition(pcbnew.VECTOR2I(x + NX, y + NY))
        v.SetViaType(pcbnew.VIATYPE_THROUGH)
        v.SetWidth(d)
        v.SetDrill(drill)
        v.SetNet(netinfo[net])
        board.Add(v)
    return made


def apply_route(name, board, netinfo, spec, renames=None):
    """Lay the board's stored routing. renames ({old: new} net names): when a
    net has only been renamed in the schematic, the stored routing is renamed
    with it - but only after proving that is the sole change: the board's
    pads, with each new name mapped back to the old, must give exactly the
    fingerprint the routing was made against. Anything else still refuses."""
    path = route_path(name)
    if not os.path.exists(path):
        raise SystemExit(f"{name}: no {os.path.relpath(path, ROOT)}; "
                         "run `make reroute` to route the board")
    r = RT.read(path)
    fp = placement_fingerprint(board, spec)
    if r["placement"] != fp and renames:
        back = {new: old for old, new in renames.items()}
        if placement_fingerprint(board, spec, back) == r["placement"]:
            r["tracks"] = [(t[0], renames.get(t[1], t[1])) + tuple(t[2:])
                           for t in r["tracks"]]
            r["vias"] = [(renames.get(v[0], v[0]),) + tuple(v[1:]) for v in r["vias"]]
            RT.write(path, name, fp, r["source"], r["tracks"], r["vias"])
            r["placement"] = fp
            print(f"  {os.path.relpath(path, ROOT)}: renamed "
                  f"{', '.join(f'{a} -> {b}' for a, b in renames.items())}; "
                  "copper unchanged")
    if r["placement"] != fp:
        raise SystemExit(f"{name}: placement or netlist changed since "
                         f"{os.path.relpath(path, ROOT)} was routed "
                         f"({r['placement']} -> {fp}); run `make reroute`")
    lay(board, netinfo, r["tracks"], r["vias"])
    return r


def amend(name, board, spec, fps, refs):
    """Change the stored routing without routing again: the ground copper
    at the ground pads of each part in refs is replaced by what the hand
    routing (spec prewire_routes) now lays there - its tracks with an end on
    one of those pads and the vias they end at - and RT.prune() then drops
    whatever leads nowhere (a track end that meets nothing; a ground via
    stays, joining the two ground planes). Ground is all pre-route copper
    (freerouting_layers), so nothing
    the autorouter laid depends on it; KiCad's DRC judges the result. A
    ground stitch the board's pin 1 chamfer cuts moves to the nearest place
    stitch() allows. Refuses unless the placement is the one the routing was made against."""
    path = route_path(name)
    r = RT.read(path)
    fp = placement_fingerprint(board, spec)
    if r["placement"] != fp:
        raise SystemExit(f"{name}: placement or netlist changed since "
                         f"{os.path.relpath(path, ROOT)} was routed; "
                         "--amend cannot help, run `make reroute`")
    tracks, vias = list(r["tracks"]), list(r["vias"])
    gp = [p for ref in refs if ref in fps for p in fps[ref].Pads()
          if p.GetNetname() == "GND"]

    def on(pads, L, x, y):
        q = pcbnew.VECTOR2I(x + NX, y + NY)
        return any(p.IsOnLayer(CU[L]) and p.HitTest(q) for p in pads)
    if gp:
        tracks = [t for t in tracks if not (t[1] == "GND" and (
            on(gp, t[0], t[3], t[4]) or on(gp, t[0], t[5], t[6])))]
        rt, rv = spec.get("prewire_routes", ([], []))
        ends = set()
        for rr in rt:
            L, net, pts = rr[0], rr[1], rr[2]
            if net != "GND":
                continue
            segs = [(L, net, nm(FANS.width(rr, W_SIG)), nm(a[0]), nm(a[1]),
                     nm(b[0]), nm(b[1])) for a, b in zip(pts, pts[1:])]
            if any(on(gp, L, t[3], t[4]) or on(gp, L, t[5], t[6])
                   for t in segs):
                tracks += segs
                ends |= {q for t in segs for q in ((t[3], t[4]), (t[5], t[6]))}
        if not ends:
            raise SystemExit(f"{name}: no hand-routed ground copper at "
                             f"{', '.join(refs)}; --amend would only remove")
        have = {(v[0], v[1], v[2]) for v in vias}
        for (net, x, y) in rv:
            if net == "GND" and (nm(x), nm(y)) in ends and \
                    (net, nm(x), nm(y)) not in have:
                vias.append((net, nm(x), nm(y), nm(RULES["via_d"]),
                             nm(RULES["via_drill"])))
    # a ground stitch the board's chamfer cuts (no track ends on it) moves
    # to the nearest place clear of the cut, by stitch()'s rules
    inset = RULES["edge"] + RULES["via_d"] / 2
    ends = {q for t in tracks for q in ((t[3], t[4]), (t[5], t[6]))}
    cut = [v for v in vias if chamfer_in(spec, v[1] / 1e6, v[2] / 1e6) < inset]
    if any(v[0] != "GND" or (v[1], v[2]) in ends for v in cut):
        raise SystemExit(f"{name}: the chamfer cuts routed copper; "
                         "run `make reroute`")
    if cut:
        vias = [v for v in vias if v not in cut]
        added, missed = stitch(spec, tracks, vias, corner_obstacles(board),
                               {}, pin1_marks(board),
                               seeds=[(v[1], v[2]) for v in cut])
        if missed:
            raise SystemExit(f"{name}: no room clear of the chamfer for the "
                             f"ground stitch at {missed}")
        vias += added
        for a, b in zip(cut, added):
            print(f"  amend: ground stitch ({a[1] / 1e6:.3f}, "
                  f"{a[2] / 1e6:.3f}) -> ({b[1] / 1e6:.3f}, "
                  f"{b[2] / 1e6:.3f}), clear of the chamfer")
    pads = [p for f in fps.values() for p in f.Pads()]
    tracks, vias, gone = RT.prune(tracks, vias,
                                  lambda L, x, y: on(pads, L, x, y),
                                  {p[0] for p in spec["planes"]},
                                  stitched_nets(spec))
    old_t, old_v = set(r["tracks"]), set(r["vias"])
    print(f"  amend: tracks -{len(old_t - set(tracks))} "
          f"+{len(set(tracks) - old_t)}, vias -{len(old_v - set(vias))} "
          f"+{len(set(vias) - old_v)} ({gone} pruned)")
    src = r["source"]
    if "amended" not in src:
        src += "; amended by gen_layout.py --amend"
    RT.write(path, name, fp, src, tracks, vias)


# ---------------------------------------------------------------------------
# Freerouting. The board goes out as a Specctra DSN with its parts placed and
# its inner planes poured. Its session file comes back with the tracks and
# vias; --ses cleans those up and writes the routing file.
# ---------------------------------------------------------------------------
def freerouting_layers(dsn, spec):
    """Rewrite the exported DSN's layer use for Freerouting 2.5.

    Freerouting forces a `power` layer inactive, then rejects any route whose
    end lies on an inactive layer - and a pin's connection to a plane ends on
    the plane's layer, so no plane pin can ever be joined to its plane (see
    AutorouteEngine: "because some of their layers are disabled"). So every
    layer goes out as `signal`, and each net class says which layers it may
    use: ordinary nets the routing layers only, a plane net the routing
    layers plus its own plane's layer. Nothing foreign can be routed on a
    plane layer, and a plane net's vias can land on its plane."""
    route = list(spec["route_layers"])
    own = {}
    for (net, layer, _, _) in spec["planes"]:
        own.setdefault(plane_class(net), []).append(layer + ".Cu")
    with open(dsn) as fh:
        text = fh.read()
    text = text.replace("(type power)", "(type signal)")
    # Ground is not Freerouting's to route. Every ground pin is joined to
    # the planes by the pre-route copper (prewire: a via beside each
    # passive's ground pad, the exposed-pad vias and the stubs into them,
    # the connector's ground vias), and 2.5.0 cannot see a connection made
    # through a via in an exposed pad - it reports each such via as a
    # clearance violation against its pad and leaves everything that
    # reaches the plane only through one cut off, then fails trying to
    # route round it. So the ground net goes out with no pins: its copper
    # stays in the DSN as fixed wiring that everything else avoids, and the
    # pins are obstacles. KiCad's DRC checks every ground connection.
    text, n = re.subn(r"\(net GND\n\s*\(pins[^)]*\)\n\s*\)", "(net GND)", text)
    if n != 1:
        raise SystemExit(f"{dsn}: expected one ground net, found {n}")
    out = []
    cls = None
    for line in text.split("\n"):
        s = line.strip()
        if s.startswith("(class "):
            cls = s.split()[1]
        if s.startswith("(use_via ") and cls is not None:
            layers = route + [L for L in own.get(cls, []) if L not in route]
            line = line + "\n" + line[:len(line) - len(line.lstrip())] + \
                "(use_layer " + " ".join(layers) + ")"
            cls = None
        out.append(line)
    with open(dsn, "w") as fh:
        fh.write("\n".join(out))


# ---------------------------------------------------------------------------
# Copper placed before autorouting, locked, so Freerouting routes round it:
# the hand-routed copper from tools/layout/fans.py (spec prewire_routes),
# the connectors' fan-outs among it; via arrays in the exposed pads; and a
# ground via beside every passive's ground pad (and the small ICs' ground
# pins), joined by a short stub. This is all of each
# board's ground: Freerouting is not given ground to route (see
# freerouting_layers), so every ground pin must reach the planes here, and
# each decoupling loop gets its own short path to them before anything else
# claims the space. KiCad's DRC checks that none is missed.
# ---------------------------------------------------------------------------
# A 2 x 2 grid in the QFN's pad and one row of three along each
# transceiver's (unless the hand routing lays a pad's vias itself).
EP_VIA_PITCH = 1.2
EP_ONE_ROW = 2.5        # a pad narrower than this gets a single row
STUB_W = 0.3
GND_PINS = {"U5": "2", "U6": "2", "X1": "2", "U7": "2"}


def _obstacles(board):
    """Every pad as (x0, y0, x1, y1, net, layers, is_hole) in local mm."""
    obs = []
    for fp in board.GetFootprints():
        for p in fp.Pads():
            bb = p.GetBoundingBox()
            a, b = to_local(bb.GetOrigin()), to_local(bb.GetEnd())
            layers = {L for L in (pcbnew.F_Cu, pcbnew.B_Cu) if p.IsOnLayer(L)}
            hole = p.GetAttribute() == pcbnew.PAD_ATTRIB_NPTH
            if p.GetDrillSize().x > 0:
                layers = {pcbnew.F_Cu, pcbnew.B_Cu}
            if not layers:
                continue            # paste or mask aperture: no copper
            rnd = p.GetShape(pcbnew.F_Cu) == pcbnew.PAD_SHAPE_CIRCLE and \
                abs((b[0] - a[0]) - (b[1] - a[1])) < 1e-6
            obs.append((a[0], a[1], b[0], b[1], p.GetNetname(), layers, hole,
                        f"{fp.GetReference()}.{p.GetNumber()}", rnd))
    return obs


def _rect_dist(x, y, r):
    """Distance from a point to a pad: its bounding box, or its circle for
    a round pad (the DIP pins)."""
    if len(r) > 8 and r[8]:
        return max(0.0, math.hypot(x - (r[0] + r[2]) / 2,
                                   y - (r[1] + r[3]) / 2) - (r[2] - r[0]) / 2)
    dx = max(r[0] - x, 0, x - r[2])
    dy = max(r[1] - y, 0, y - r[3])
    return math.hypot(dx, dy)


def prewire(board, netinfo, spec, fps):
    """Place the locked pre-route copper; returns (tracks, vias) in the
    routing file's form."""
    via_r = RULES["via_d"] / 2
    clr = RULES["clearance"]
    obs = _obstacles(board)
    x0 = min(r[0] for r in spec["outline"]) + RULES["edge"] + via_r
    y0 = min(r[1] for r in spec["outline"]) + RULES["edge"] + via_r
    x1 = max(r[2] for r in spec["outline"]) - RULES["edge"] - via_r
    y1 = max(r[3] for r in spec["outline"]) - RULES["edge"] - via_r
    vias, tracks = [], []
    segs = []            # (layer, net, ax, ay, bx, by): the routes laid here
    rt, rv = spec.get("prewire_routes", ([], []))
    seg_w = []           # each hand-routed segment's width, index for index
    for r in rt:
        L, net, pts, w = r[0], r[1], r[2], FANS.width(r, W_SIG)
        for (a, b) in zip(pts, pts[1:]):
            segs.append((CU[L], net, *a, *b))
            seg_w.append(w)
            tracks.append((L, net, nm(w), nm(a[0]), nm(a[1]),
                           nm(b[0]), nm(b[1])))
    for (net, x, y) in rv:
        vias.append((x, y, net))

    def seg_d(px, py, ax, ay, bx, by):
        dx, dy = bx - ax, by - ay
        ll = dx * dx + dy * dy
        u = 0 if ll == 0 else max(0, min(1, ((px - ax) * dx + (py - ay) * dy) / ll))
        return math.hypot(px - ax - u * dx, py - ay - u * dy)

    # the hand-routed copper must clear every pad too
    bad = []
    for (x, y, net) in vias:
        for o in obs:
            if o[4] != net and _rect_dist(x, y, o) < via_r + clr + FANS.MARGIN:
                bad.append(f"via {net} ({x}, {y}) {_rect_dist(x, y, o):.3f} "
                           f"from pad {o[7]}")
    for (L, net, a, b, c, d), w in zip(segs, seg_w):
        n = max(2, int(math.hypot(c - a, d - b) / 0.02))
        for o in obs:
            if o[4] == net or L not in o[5]:
                continue
            g = min(_rect_dist(a + (c - a) * k / n, b + (d - b) * k / n, o)
                    for k in range(n + 1))
            if g < w / 2 + clr + FANS.MARGIN:
                bad.append(f"{net} on {CU_NAME[L]} {g:.3f} from pad {o[7]}")
    # and a via must not land on a pad of its own net either (other than an
    # exposed pad, whose vias are meant to be there): solder would wick
    # into it, and on the far side it would sit under another part's pad
    ep = {f"{r}.{n}" for (r, n) in EP_GND} | \
        {f"{r}.{n}" for (r, n) in spec.get("via_in_pad", ())}
    for (x, y, net) in vias:
        for o in obs:
            if o[4] == net and o[7] not in ep and _rect_dist(x, y, o) < via_r:
                bad.append(f"via {net} ({x}, {y}) lands on pad {o[7]}")
    if bad:
        raise SystemExit("hand-routed copper does not clear the pads:\n  " +
                         "\n  ".join(sorted(set(bad))))
    # pads the hand routing already takes to ground: no via of their own
    served = set()
    for (L, net, a, b, c, d) in segs:
        if net != "GND":
            continue
        for o in obs:
            if o[4] == "GND" and L in o[5] and not o[6] and any(
                    o[0] <= px <= o[2] and o[1] <= py <= o[3]
                    for (px, py) in ((a, b), (c, d))):
                served.add(o[7])
    # and the pads with a hand-routed via in them
    vip = {f"{r}.{n}" for (r, n) in spec.get("via_in_pad", ())}
    for o in obs:
        if o[7] in vip and any(n == o[4] and o[0] <= x <= o[2] and
                               o[1] <= y <= o[3] for (x, y, n) in vias):
            served.add(o[7])

    def via_ok(x, y, net, own=None):
        if any(n != net and seg_d(x, y, a, b, c, d) < via_r + clr + w / 2
               for (_, n, a, b, c, d), w in zip(segs, seg_w)):
            return False
        if not (x0 <= x <= x1 and y0 <= y <= y1):
            return False
        for (vx, vy, vn) in vias:
            # with a margin: Freerouting reports fixed vias closer than
            # about 0.15 mm edge to edge, though they are legal
            if math.hypot(vx - x, vy - y) < RULES["via_d"] + 0.15:
                return False
        for o in obs:
            # full clearance from every pad, its own and other ground pads
            # included: Freerouting counts a via that nearly touches any pad
            # as a violation, connected or not
            d = _rect_dist(x, y, o)
            need = RULES["hole_clearance"] + 0.1 if o[6] else \
                via_r + clr + 0.03
            if d < need:
                return False
        return True

    def stub_ok(ax, ay, bx, by, layer, net, own):
        n = max(2, int(math.hypot(bx - ax, by - ay) / 0.05))
        for k in range(n + 1):
            x = ax + (bx - ax) * k / n
            y = ay + (by - ay) * k / n
            if any(L == layer and sn != net and
                   seg_d(x, y, a, b, c, d) < STUB_W / 2 + clr + w / 2
                   for (L, sn, a, b, c, d), w in zip(segs, seg_w)):
                return False
            # and every other net's via, which goes through every layer
            if any(vn != net and math.hypot(vx - x, vy - y) <
                   via_r + STUB_W / 2 + clr for (vx, vy, vn) in vias):
                return False
            for o in obs:
                if o[7] == own or layer not in o[5]:
                    continue
                if o[4] == net and not o[6]:
                    continue
                if _rect_dist(x, y, o) < STUB_W / 2 + clr:
                    return False
        return True

    # exposed pads: a grid of vias inside each, less any too close to the
    # package's own pins
    for ref, num in sorted(EP_GND):
        if ref not in fps:
            continue
        pad = [p for p in fps[ref].Pads() if p.GetNumber() == num][0]
        bb = pad.GetBoundingBox()
        (a, b), (c, d) = to_local(bb.GetOrigin()), to_local(bb.GetEnd())
        nx = int((c - a - RULES["via_d"]) // EP_VIA_PITCH) + 1
        ny = int((d - b - RULES["via_d"]) // EP_VIA_PITCH) + 1
        if min(c - a, d - b) < EP_ONE_ROW:
            if c - a < d - b:
                nx = 1
            else:
                ny = 1
        cx, cy = (a + c) / 2, (b + d) / 2
        own = f"{ref}.{num}"
        if any(a <= vx <= c and b <= vy <= d for (vx, vy, _) in vias):
            continue        # the hand routing has laid this pad's vias
        for i in range(nx):
            for j in range(ny):
                x = round(cx + (i - (nx - 1) / 2) * EP_VIA_PITCH, 4)
                y = round(cy + (j - (ny - 1) / 2) * EP_VIA_PITCH, 4)
                # clear of every pad but its own, and of the hand-routed
                # copper (U4's has its A lines running under it)
                if all(_rect_dist(x, y, o) >= via_r + clr for o in obs
                       if o[7] != own) and \
                        not any(n != "GND" and seg_d(x, y, a, b, c, d) <
                                via_r + clr + FANS.MARGIN + w / 2
                                for (_, n, a, b, c, d), w in zip(segs, seg_w)):
                    vias.append((x, y, "GND"))

    # a ground via beside each ground pad of a passive or small IC
    for ref, fp in sorted(fps.items()):
        pads = list(fp.Pads())
        if ref in GND_PINS:
            gp = [p for p in pads if p.GetNumber() == GND_PINS[ref]]
        elif ref in spec.get("gnd_pins", ()):
            # a leadless IC's ground pins, other than its exposed pad
            gp = [p for p in pads if p.GetNetname() == "GND"
                  and (ref, p.GetNumber()) not in EP_GND]
        elif ref[0] in "CRDL" or ref.startswith("FB"):
            gp = [p for p in pads if p.GetNetname() == "GND"] \
                if len(pads) == 2 else []
        else:
            continue
        for p in gp:
            if f"{ref}.{p.GetNumber()}" in served:
                continue
            px, py = to_local(p.GetPosition())
            bb = p.GetBoundingBox()
            (a, b), (c, d) = to_local(bb.GetOrigin()), to_local(bb.GetEnd())
            cx, cy = to_local(fp.GetPosition())
            ux, uy = px - cx, py - cy
            L = math.hypot(ux, uy) or 1.0
            ux, uy = ux / L, uy / L
            half = max(c - a, d - b) / 2
            layer = pcbnew.F_Cu if p.IsOnLayer(pcbnew.F_Cu) else pcbnew.B_Cu
            own = f"{ref}.{p.GetNumber()}"
            found = None
            # straight out first, then out at 45 degrees, then sideways
            dirs = [(ux, uy)]
            for s in (1, -1):
                dirs.append(((ux - s * uy) / math.sqrt(2),
                             (uy + s * ux) / math.sqrt(2)))
            for s in (1, -1):
                dirs.append((-s * uy, s * ux))
            for s in (1, -1):           # back past the part, as a last resort
                dirs.append(((-ux - s * uy) / math.sqrt(2),
                             (-uy + s * ux) / math.sqrt(2)))
            for (dx, dy) in dirs:
                for k in range(0, 26):
                    # a full clearance from its own pad too: Freerouting
                    # counts a via that nearly touches its pad as a violation
                    dist = half + via_r + clr + 0.05 * k
                    x, y = round(px + dx * dist, 4), round(py + dy * dist, 4)
                    if via_ok(x, y, "GND", own) and \
                            stub_ok(px, py, x, y, layer, "GND", own):
                        found = (x, y, layer)
                        break
                if found:
                    break
            if not found:
                # no room for its own: share the nearest ground via in reach
                for (vx, vy, vn) in sorted(
                        (v for v in vias if v[2] == "GND"),
                        key=lambda v: math.hypot(v[0] - px, v[1] - py)):
                    if math.hypot(vx - px, vy - py) > 2.0:
                        break
                    if stub_ok(px, py, vx, vy, layer, "GND", own):
                        tracks.append((CU_NAME[layer], "GND", nm(STUB_W),
                                       nm(px), nm(py), nm(vx), nm(vy)))
                        found = "shared"
                        break
            if found == "shared":
                continue
            if not found:
                # say what is in the way straight out from the pad
                x, y = px + ux * (half + 0.4), py + uy * (half + 0.4)
                near = sorted((round(_rect_dist(x, y, o), 2), o[7], o[4])
                              for o in obs if o[7] != own and o[4] != "GND")[:3]
                print(f"  prewire: no room for a ground via at {own} "
                      f"({px:.2f}, {py:.2f}); nearest: {near}")
                continue
            x, y, layer = found
            vias.append((x, y, "GND"))
            tracks.append((CU_NAME[layer], "GND", nm(STUB_W),
                           nm(px), nm(py), nm(x), nm(y)))
    out_v = [(n, nm(x), nm(y), nm(RULES["via_d"]), nm(RULES["via_drill"]))
             for (x, y, n) in vias]
    return tracks, out_v


def snap_wire_ends(dsn, board):
    """Put the fixed wires' ends back on the pad centres they were drawn to.

    KiCad 10's Specctra exporter writes pin offsets to 0.1 um (the DSN's
    `resolution um 10`) but wire points to six significant digits, so a
    track drawn to a pad centre with a half-micron in it (a 0603's 0.7875 mm
    offset, a SOT-23's 1.1375 mm) goes out 0.5 um beside it. Freerouting
    2.5.0 counts a fixed wire as joined to an SMD pin only when it ends on
    the pin's centre exactly, so it reports every such connection unrouted.
    Each wire point within 1 um of a pad centre is moved onto it."""
    centres = []
    for fp in board.GetFootprints():
        for pad in fp.Pads():
            q = pad.GetPosition()
            centres.append((q.x / 1000.0, -q.y / 1000.0))

    def fmt(v):
        return f"{v:.1f}".rstrip("0").rstrip(".")

    def snap(m):
        nums = [float(v) for v in m.group(2).split()]
        pts = []
        for x, y in zip(nums[0::2], nums[1::2]):
            for cx, cy in centres:
                if abs(x - cx) <= 1.0 and abs(y - cy) <= 1.0:
                    x, y = cx, cy
                    break
            pts.append(f"{fmt(x)} {fmt(y)}")
        return m.group(1) + "  " + "  ".join(pts) + ")"

    with open(dsn) as fh:
        text = fh.read()
    text = re.sub(r"(\(wire \(path \S+ [\d.]+)\s+([-\d.\s]+?)\)", snap, text)
    with open(dsn, "w") as fh:
        fh.write(text)


def export_dsn(name, board, dsn, spec):
    if not pcbnew.ExportSpecctraDSN(board, dsn):
        raise SystemExit(f"DSN export failed: {dsn}")
    freerouting_layers(dsn, spec)
    snap_wire_ends(dsn, board)


def drc(name):
    """KiCad's DRC on the saved board: {(type, frozenset(uuids))}."""
    rpt = os.path.join(ROOT, "build", "fr", name + ".drc.json")
    subprocess.run(["kicad-cli", "pcb", "drc", "--format", "json",
                    "--refill-zones", "--severity-error", "-o", rpt,
                    os.path.join(HW, name + ".kicad_pcb")],
                   check=True, capture_output=True)
    with open(rpt) as fh:
        d = json.load(fh)
    return {(v["type"], frozenset(i["uuid"] for i in v["items"]))
            for v in d["violations"]}


def widen(name, board, ns, tracks, objs, uids):
    """Put back the widths Freerouting 2.5.0 necked down (its fanout ignores
    automatic_neckdown=false; issue #973). Every track narrower than its net
    class is tried at the class width, then at each narrower step; a track
    moves down a step whenever DRC reports something new against it, and
    keeps its routed width if no step fits. objs/uids: the laid track
    objects and their ids, index for index. Returns the new widths."""
    def steps(t):
        w, cw = t[2], ns.GetEffectiveNetClass(t[1]).GetTrackWidth()
        return [s for s in (cw, mm(0.25), mm(0.2), mm(0.15), mm(0.127))
                if w < s <= cw] + [w]
    todo = {i: steps(t) for i, t in enumerate(tracks)}
    todo = {i: s for i, s in todo.items() if len(s) > 1}
    if not todo:
        return tracks
    # what DRC reports at the routed widths is not the widening's doing
    save(name, board)
    base = drc(name)
    uid = {u: i for i, u in enumerate(uids)}
    level = dict.fromkeys(todo, 0)
    for _ in range(8):
        cur = [(t[0], t[1], todo[i][level[i]], *t[3:]) if i in todo else t
               for i, t in enumerate(tracks)]
        for i in todo:
            objs[i].SetWidth(cur[i][2])
        save(name, board)
        bad = {uid[u] for (_, items) in drc(name) - base for u in items
               if uid.get(u) in todo and level[uid[u]] < len(todo[uid[u]]) - 1}
        if not bad:
            return cur
        for i in bad:
            level[i] += 1
    raise SystemExit(f"{name}: widening did not settle")


def corner_obstacles(board):
    """What RT.smooth() must keep clear of, from the placed board: every
    pad's copper on each layer it is flashed on (bounding box, or circle),
    every hole at its hole clearance, and the no-track rule areas."""
    pads, holes, keepouts = [], [], []
    hc = RULES["hole_clearance"] - RULES["clearance"]
    for fp in board.GetFootprints():
        for p in fp.Pads():
            layers = {n for n, L in CU.items() if L in
                      (pcbnew.F_Cu, pcbnew.In1_Cu, pcbnew.In2_Cu, pcbnew.B_Cu)
                      and p.FlashLayer(L)}
            bb = p.GetBoundingBox()
            x0, y0 = bb.GetOrigin().x - NX, bb.GetOrigin().y - NY
            x1, y1 = bb.GetEnd().x - NX, bb.GetEnd().y - NY
            rnd = p.GetShape(pcbnew.F_Cu) == pcbnew.PAD_SHAPE_CIRCLE and \
                x1 - x0 == y1 - y0
            if layers:
                pads.append((layers, p.GetNetname(), x0, y0, x1, y1, rnd))
            if p.GetDrillSize().x > 0:
                hs = p.GetEffectiveHoleShape()      # a slot, or a point
                sa, sb = hs.GetSeg().A, hs.GetSeg().B
                pth = p.GetAttribute() != pcbnew.PAD_ATTRIB_NPTH
                holes.append((p.GetNetname() if pth else None,
                              sa.x - NX, sa.y - NY, sb.x - NX, sb.y - NY,
                              hs.GetWidth() / 2 + mm(hc)))
    for z in board.Zones():
        if z.GetIsRuleArea() and z.GetDoNotAllowTracks():
            bb = z.GetBoundingBox()
            for L in z.GetLayerSet().CuStack():
                keepouts.append((CU_NAME[L], bb.GetOrigin().x - NX,
                                 bb.GetOrigin().y - NY, bb.GetEnd().x - NX,
                                 bb.GetEnd().y - NY))
    return pads, holes, keepouts


def drc_items(name, placed, tracks, vias, tag):
    """Lay tracks and vias on a fresh copy of the placed board, run KiCad's
    DRC on it, and return what
    it reports, errors, warnings and unconnected items alike: [(type, item
    descriptions, the tracks it names, its positions)]."""
    b = pcbnew.LoadBoard(placed)
    nets = {p.GetNetname(): p.GetNet() for fp in b.GetFootprints()
            for p in fp.Pads() if p.GetNetname()}
    made = lay(b, nets, tracks, vias)
    by_uid = {u: t for t, (_, u) in made.items()}
    tmp = os.path.join(ROOT, "build", "fr", f"{name}.{tag}.kicad_pcb")
    b.Save(tmp)
    shutil.copyfile(os.path.join(HW, name + ".kicad_pro"),
                    os.path.join(ROOT, "build", "fr", f"{name}.{tag}.kicad_pro"))
    rpt = os.path.join(ROOT, "build", "fr", f"{name}.{tag}.json")
    subprocess.run(["kicad-cli", "pcb", "drc", "--format", "json",
                    "--refill-zones", "--severity-error", "--severity-warning",
                    "-o", rpt, tmp], check=True, capture_output=True)
    with open(rpt) as fh:
        d = json.load(fh)
    out = []
    for v in d["violations"] + d.get("unconnected_items", []):
        named = [by_uid[i["uuid"]] for i in v["items"] if i["uuid"] in by_uid]
        pos = [(round(i["pos"]["x"] * 1e6) - NX, round(i["pos"]["y"] * 1e6) - NY)
               for i in v["items"] if "pos" in i]
        desc = [i.get("description", "") for i in v["items"]]
        out.append((v["type"], tuple(desc), named, pos))
    return out


def clean_corners(name, spec, placed, tracks, vias, obstacles, on_pad):
    """RT.smooth() on the widened routing, judged by KiCad's DRC: a redrawn
    track DRC names - or, for something new it reports without naming a
    track, a redrawn track within 1.5 mm of it - is forbidden to smooth()
    (that exact track), and smooth() runs again, until DRC has nothing new
    to say. What DRC reports against the routing as it came (the library
    check on a board saved outside hw/) is not the cleanup's doing."""
    pads, holes, keepouts = obstacles
    outline = [nm(v) for v in (min(r[0] for r in spec["outline"]),
                               min(r[1] for r in spec["outline"]),
                               max(r[2] for r in spec["outline"]),
                               max(r[3] for r in spec["outline"]))]
    base = drc_items(name, placed, tracks, vias, "corners")
    known = {(t, desc) for (t, desc, named, pos) in base}
    if base:
        print(f"  corners: DRC already reports {len(base)} before cleanup")
    forbidden = set()
    orig = {RT._norm(t) for t in tracks}
    for attempt in range(12):
        out, n = RT.smooth(tracks, vias, pads, holes, keepouts, outline,
                           nm(RULES["clearance"]), nm(RULES["edge"]),
                           on_pad, frozenset(forbidden))
        out = RT.tidy(out, vias, on_pad)
        new = [t for t in out if RT._norm(t) not in orig]
        rep = drc_items(name, placed, out, vias, "corners")
        bad = set()
        for (typ, desc, named, pos) in rep:
            mine = [t for t in named if RT._norm(t) not in orig]
            if mine:
                bad.update(RT._norm(t) for t in mine)
            elif (typ, desc) not in known:
                # nothing redrawn is named: blame what was redrawn nearby
                for t in new:
                    if any(math.hypot(px - (t[3] + t[5]) / 2,
                                      py - (t[4] + t[6]) / 2) < 1500000
                           for (px, py) in pos):
                        bad.add(RT._norm(t))
                if not pos or not bad:
                    raise SystemExit(f"{name}: corner cleanup: DRC reports "
                                     f"{typ} {desc} and nothing to blame")
        print(f"  corners: try {attempt + 1}: {n} runs redrawn, "
              f"{len(new)} new segments, DRC objects to {len(bad)}")
        if not bad:
            return out
        forbidden |= bad
    raise SystemExit(f"{name}: corner cleanup did not settle")


# ---------------------------------------------------------------------------
# Ground stitching. Both inner layers are ground planes, so a signal that
# changes layer through a via changes its reference plane too; its return
# current crosses between the planes at the nearest ground via. stitch()
# gives every via off ground one within RETURN_VIA, and keeps the two
# planes tied together at most STITCH_PITCH apart along the board's edges
# and round the connector, the AG32 and the buffers (DESIGN_NOTES.md, "Four
# layers on both boards"). Vias go only where they clear every pad, hole,
# track and the edge; what cannot be met is reported, not forced.
# ---------------------------------------------------------------------------
RETURN_VIA = RT.RETURN_VIA       # mm
STITCH_PITCH = 4.0               # mm, the most between stitches
STITCH_ROUND = ("J5", "J6", "U1", "U2", "U3", "U4")


def pin1_marks(board):
    """Each footprint's pin 1 mark (its filled silk triangle) as a box
    (x0, y0, x1, y1) in nm, stroke included: stitch() keeps its vias off
    them, so no mark is printed over a via."""
    out = []
    for fp in board.GetFootprints():
        for g in fp.GraphicalItems():
            if (isinstance(g, pcbnew.PCB_SHAPE)
                    and g.GetLayer() in (pcbnew.F_SilkS, pcbnew.B_SilkS)
                    and g.GetShape() == pcbnew.SHAPE_T_POLY
                    and g.IsSolidFill()):
                bb = g.GetBoundingBox()
                out.append((bb.GetLeft() - NX, bb.GetTop() - NY,
                            bb.GetRight() - NX, bb.GetBottom() - NY))
    return out


def stitch(spec, tracks, vias, obstacles, fp_boxes, marks=(), seeds=None):
    """Ground vias to add, in the routing file's form. obstacles: from
    corner_obstacles(); fp_boxes: {ref: (x0, y0, x1, y1)} in nm, the parts
    to stitch round; marks: pin1_marks(), boxes to keep vias off. seeds:
    instead of the usual stitching, one via as near each of these points
    (nm) as it will go (amend's replacements)."""
    pads, holes, _ = obstacles
    vd, dr = nm(RULES["via_d"]), nm(RULES["via_drill"])
    r = vd // 2
    clr = nm(RULES["clearance"] + FANS.MARGIN)
    x0 = nm(min(o[0] for o in spec["outline"]))
    y0 = nm(min(o[1] for o in spec["outline"]))
    x1 = nm(max(o[2] for o in spec["outline"]))
    y1 = nm(max(o[3] for o in spec["outline"]))
    inset = nm(RULES["edge"]) + r + nm(0.01)
    out = []

    def seg_d(px, py, ax, ay, bx, by):
        dx, dy = bx - ax, by - ay
        ll = dx * dx + dy * dy
        u = 0 if ll == 0 else max(0.0, min(1.0, ((px - ax) * dx +
                                                 (py - ay) * dy) / ll))
        return math.hypot(px - ax - u * dx, py - ay - u * dy)

    def rect_d(px, py, p):
        _, _, a, b, c, d, rnd = p
        if rnd:
            return max(0.0, math.hypot(px - (a + c) / 2, py - (b + d) / 2)
                       - (c - a) / 2)
        return math.hypot(max(a - px, 0, px - c), max(b - py, 0, py - d))

    def ok(x, y):
        if not (x0 + inset <= x <= x1 - inset and y0 + inset <= y <= y1 - inset):
            return False
        if chamfer_in(spec, x / 1e6, y / 1e6) * 1e6 < inset:
            return False
        for v in vias + out:
            if math.hypot(v[1] - x, v[2] - y) < vd + nm(0.1):
                return False
        for (L, n, w, ax, ay, bx, by) in tracks:
            if n != "GND" and seg_d(x, y, ax, ay, bx, by) < r + w / 2 + clr:
                return False
        for p in pads:
            # a ground pad is not landed on either (solder would wick
            # into the via)
            if rect_d(x, y, p) < r + (0 if p[1] == "GND" else clr):
                return False
        for (n, ax, ay, bx, by, rad) in holes:
            if seg_d(x, y, ax, ay, bx, by) < rad + dr / 2 + nm(0.15):
                return False
        for (a, b, c, d) in marks:
            if math.hypot(max(a - x, 0, x - c),
                          max(b - y, 0, y - d)) < r + nm(0.05):
                return False
        return True

    def ground():
        g = [(v[1], v[2]) for v in vias + out if v[0] == "GND"]
        g += [((a + c) / 2, (b + d) / 2) for (ly, n, a, b, c, d, rnd) in pads
              if n == "GND" and rnd]
        return g

    def near(x, y, dist):
        return any(math.hypot(gx - x, gy - y) <= dist for gx, gy in ground())

    def place(cands):
        for (x, y) in cands:
            x, y = int(round(x)), int(round(y))
            if ok(x, y):
                out.append(("GND", x, y, vd, dr))
                return True
        return False

    missed = []
    if seeds is not None:
        for (sx, sy) in seeds:
            cands = [(sx + rr * math.cos(math.radians(a)),
                      sy + rr * math.sin(math.radians(a)))
                     for rr in [nm(0.05 * i) for i in range(1, 41)]
                     for a in range(0, 360, 5)]
            if not place(cands):
                missed.append(("GND", sx, sy))
        return out, missed

    # beside each via off ground, as close as it will go
    for v in sorted(vias, key=lambda v: (v[1], v[2])):
        if v[0] == "GND" or near(v[1], v[2], nm(RETURN_VIA)):
            continue
        rings = [nm(0.56 + 0.04 * i) for i in range(12)]
        cands = [(v[1] + rr * math.cos(math.radians(a)),
                  v[2] + rr * math.sin(math.radians(a)))
                 for rr in rings for a in range(0, 360, 5)]
        if not place(cands):
            missed.append(v)

    # along the edges and round the parts that carry the fast lines
    def walk(pts):
        """pts: a closed path of corners; place stitches so that no point
        on it is farther than half the pitch from one."""
        half = nm(STITCH_PITCH / 2)
        for (ax, ay), (bx, by) in zip(pts, pts[1:] + pts[:1]):
            n = max(1, int(math.hypot(bx - ax, by - ay) / nm(0.25)))
            for k in range(n + 1):
                x = ax + (bx - ax) * k / n
                y = ay + (by - ay) * k / n
                if near(x, y, half):
                    continue
                cands = [(x + du * (bx - ax) / max(1, math.hypot(bx - ax,
                                                                 by - ay))
                          + dv * (ay - by) / max(1, math.hypot(bx - ax, by - ay)),
                          y + du * (by - ay) / max(1, math.hypot(bx - ax,
                                                                 by - ay))
                          + dv * (bx - ax) / max(1, math.hypot(bx - ax, by - ay)))
                         for dv in (nm(0.05 * j) for j in range(0, 21))
                         for du in (nm(0.1 * i) * s for i in range(0, 13)
                                    for s in (1, -1))]
                place(cands)

    e = inset
    walk([(x0 + e, y0 + e), (x1 - e, y0 + e), (x1 - e, y1 - e),
          (x0 + e, y1 - e)])
    for ref in STITCH_ROUND:
        if ref in fp_boxes:
            a, b, c, d = fp_boxes[ref]
            m = nm(0.3)
            walk([(a - m, b - m), (c + m, b - m), (c + m, d + m),
                  (a - m, d + m)])
    return out, missed


def import_ses(name, spec, ses):
    """Freerouting's session -> tidied, widened, corner-cleaned
    hw/<board>.route.json. Reads the board --dsn saved (placed, inner planes
    poured, no tracks)."""
    path = os.path.join(HW, name + ".kicad_pcb")
    board = pcbnew.LoadBoard(path)
    # Everything read from the board comes first, and tracks are only ever
    # added to it: once items are removed or bulk-imported, KiCad 10's
    # bindings start handing back untyped objects (which is also why the
    # session is parsed here rather than by pcbnew.ImportSpecctraSES).
    placement = placement_fingerprint(board, spec)
    pads = [p for fp in board.GetFootprints() for p in fp.Pads()]
    netinfo = {p.GetNetname(): p.GetNet() for p in pads if p.GetNetname()}
    ns = board.GetDesignSettings().m_NetSettings
    if len(board.GetTracks()):
        raise SystemExit(f"{name}: {path} already has tracks; run --dsn first")
    obstacles = corner_obstacles(board)
    # the placed board without copper, for the corner cleanup's DRC runs
    placed = os.path.join(ROOT, "build", "fr", name + ".placed.kicad_pcb")
    shutil.copyfile(path, placed)
    shutil.copyfile(os.path.join(HW, name + ".kicad_pro"),
                    os.path.join(ROOT, "build", "fr", name + ".placed.kicad_pro"))

    with open(ses) as fh:
        tracks, vias = RT.parse_ses(fh.read(), (NX, NY))
    n0 = len(tracks)
    # the pre-route copper, whether or not the session repeats it
    pre = os.path.join(os.path.dirname(ses), name + ".prewire.json")
    if os.path.exists(pre):
        with open(pre) as fh:
            pw = json.load(fh)
        tracks = list(dict.fromkeys(tracks + [tuple(x) for x in pw["tracks"]]))
        vias = list(dict.fromkeys(vias + [tuple(x) for x in pw["vias"]]))

    def on_pad(L, x, y):
        q = pcbnew.VECTOR2I(x + NX, y + NY)
        return any(p.IsOnLayer(CU[L]) and p.HitTest(q) for p in pads)
    tracks = RT.tidy(tracks, vias, on_pad)
    tracks, vias, gone = RT.prune(tracks, vias, on_pad,
                                  {p[0] for p in spec["planes"]},
                                  stitched_nets(spec))
    if gone:
        print(f"  pruned {gone} items that led nowhere")
    made = lay(board, netinfo, tracks, vias)
    objs = [made[t][0] for t in tracks]
    uids = [made[t][1] for t in tracks]
    tracks = widen(name, board, ns, tracks, objs, uids)
    tracks = clean_corners(name, spec, placed, tracks, vias, obstacles, on_pad)

    if stitched_nets(spec):
        boxes = {}
        for fp in board.GetFootprints():
            if fp.GetReference() in STITCH_ROUND:
                fp.BuildCourtyardCaches()
                bb = fp.GetCourtyard(pcbnew.B_CrtYd if fp.IsFlipped()
                                     else pcbnew.F_CrtYd).BBox()
                boxes[fp.GetReference()] = (bb.GetOrigin().x - NX,
                                            bb.GetOrigin().y - NY,
                                            bb.GetEnd().x - NX,
                                            bb.GetEnd().y - NY)
        added, missed = stitch(spec, tracks, vias, obstacles, boxes,
                               pin1_marks(board))
        # KiCad's DRC judges them: any it names is dropped
        base = {(t, d) for (t, d, _, _) in
                drc_items(name, placed, tracks, vias, "stitch0")}
        for _ in range(5):
            rep = drc_items(name, placed, tracks, vias + added, "stitch")
            bad = {p for (t, d, _, pos) in rep if (t, d) not in base
                   for p in pos}
            drop = [v for v in added if (v[1], v[2]) in bad]
            if not drop:
                break
            added = [v for v in added if v not in drop]
        vias = vias + added
        print(f"  stitching: {len(added)} ground vias; "
              f"{len(missed)} vias with no room for one within "
              f"{RETURN_VIA} mm")

    RT.write(route_path(name), name, placement,
             "Freerouting 2.5.0 (scripts/freerouting.sh), tidied, widened, "
             "corner-cleaned and stitched by gen_layout.py --ses",
             tracks, vias)
    narrow = sum(1 for t in tracks
                 if t[2] < ns.GetEffectiveNetClass(t[1]).GetTrackWidth())
    print(f"{name}: {n0} segments from Freerouting, {len(tracks)} after "
          f"tidying, {len(vias)} vias; {narrow} still below their class "
          f"width -> {os.path.relpath(route_path(name), ROOT)}")


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--only", help="build just this board")
    ap.add_argument("--out", default=os.path.join(ROOT, "build", "layout"))
    ap.add_argument("--fr", default=os.path.join(ROOT, "build", "fr"),
                    help="directory for the DSN and SES files. Kept apart from "
                    "--out: Freerouting reads any <board>.json beside the DSN "
                    "as a KiCad board of its own")
    mode = ap.add_mutually_exclusive_group()
    mode.add_argument("--place-only", action="store_true",
                      help="place and pour, without the routing file")
    mode.add_argument("--dsn", action="store_true",
                      help="place, pour the planes, save, and export "
                      "<fr>/<board>.dsn for Freerouting")
    ap.add_argument("--amend", nargs="*", metavar="REF",
                    help="re-lay the hand-routed ground copper at each REF's "
                    "ground pads in the stored routing (with --only) and drop "
                    "what leads nowhere, and move any ground stitch the "
                    "pin 1 chamfer cuts, then build as usual (see amend)")
    ap.add_argument("--rename", action="append", default=[], metavar="OLD=NEW",
                    help="a net renamed in the schematic: rename it in the "
                    "stored routing, after proving the rename is the only "
                    "change (see apply_route)")
    mode.add_argument("--ses", action="store_true",
                      help="import <fr>/<board>.ses into the board --dsn "
                      "saved, tidy it, and write hw/<board>.route.json")
    args = ap.parse_args()
    os.makedirs(args.out, exist_ok=True)
    os.makedirs(args.fr, exist_ok=True)

    if args.ses:
        for name, spec in BOARDS.items():
            if args.only and args.only != name:
                continue
            import_ses(name, spec, os.path.join(args.fr, name + ".ses"))
        return

    built = {}
    for name, spec in BOARDS.items():
        comps, nets = NL.read(os.path.join(HW, name + ".net"))
        built[name] = build(name, spec, comps, nets,
                            keep=not args.only or args.only == name) + (spec,)

    lower = built["fastprom-lower"][1]["J5"]
    upper = built["fastprom-upper"][1]["J6"]
    err, rot = mate(upper, lower)
    print(f"connector: J6 at {rot} deg mates J5, worst contact offset "
          f"{err:.3f} mm")

    for name, (board, fps, netinfo, spec) in built.items():
        if args.only and args.only != name:
            continue
        bad = overlaps(fps)
        print(f"{name}: {len(fps)} parts, courtyard overlaps: "
              f"{', '.join(bad) if bad else 'none'}")
        if args.dsn:
            # hw/ gets the placed board without copper (--ses builds on it);
            # the DSN gets the pre-route copper as fixed wiring
            add_zones(board, netinfo, spec)
            save(name, board)
            pt_, pv_ = prewire(board, netinfo, spec, fps)
            lay(board, netinfo, pt_, pv_)
            for v in board.GetTracks():
                v.SetLocked(True)
            with open(os.path.join(args.fr, name + ".prewire.json"), "w") as fh:
                json.dump({"tracks": pt_, "vias": pv_}, fh)
            print(f"  prewire: {len(pv_)} vias, {len(pt_)} stubs")
            export_dsn(name, board, os.path.join(args.fr, name + ".dsn"),
                       spec)
        elif args.place_only:
            add_zones(board, netinfo, spec)
            save(name, board, refill=True)
        else:
            seed(name, "route")
            renames = dict(a.split("=", 1) for a in args.rename)
            if args.amend is not None:
                if args.amend and not args.only:
                    raise SystemExit("--amend REF needs --only <board>")
                amend(name, board, spec, fps, args.amend)
            r = apply_route(name, board, netinfo, spec, renames)
            add_zones(board, netinfo, spec)
            save(name, board, refill=True)
            print(f"  routing {os.path.relpath(route_path(name), ROOT)}: "
                  f"{len(r['tracks'])} segments, {len(r['vias'])} vias")
        dump(name, board, spec, os.path.join(args.out, name + ".json"))


if __name__ == "__main__":
    main()
