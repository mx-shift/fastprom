#!/usr/bin/env python3
"""A small schematic layout engine that draws conventional sheets.

The difference from emitting a global label at every pin is that here
connectivity comes from *geometry* - real wires touching real pins, junctions
where three meet, and power symbols for rails. KiCad derives the netlist from
that geometry, so a wire that misses a pin by 0.01 mm silently breaks a net.
That is why anything built on this must diff its netlist against a known-good
one; see tools/diff_netlist.py.

What it provides:

  Sym      symbol definitions with known pin geometry, including
           single-column connectors and power symbols, which is what
           conventional sheets actually use
  Sheet    placement, orthogonal routing with track allocation, junctions
  bus()    route a group of nets as a parallel bundle, which is the shape
           almost every net on this design wants
"""
import math
import re
import uuid

PITCH = 2.54
TEXT_W = 0.9        # stroke-font advance per character, as a fraction of size
CAPTION_H = 3.81    # the band at the top of a block that holds its caption

_NS = uuid.UUID("6f6173cf-0002-4000-8000-000000000000")
_seq = [0]


def uid(key=None):
    if key is None:
        _seq[0] += 1
        key = f"s{_seq[0]}"
    return str(uuid.uuid5(_NS, key))


def reset_uids():
    _seq[0] = 0


def snap(v):
    return round(round(v / 1.27) * 1.27, 2)


# ---------------------------------------------------------------------------
# Symbols
# ---------------------------------------------------------------------------

class Sym:
    """A symbol definition. pins: list of (number, name, etype, side, index)."""

    # "stack" puts the reference above and the value below. "side" puts the
    # reference above-left and the value to the right, which is how a resistor
    # or capacitor is conventionally annotated and keeps the value clear of
    # whatever the part connects to below it.
    text_layout = "stack"

    def __init__(self, name, value, footprint, descr, pins,
                 width_units=6, ref_prefix="U", elements=None,
                 body_margin=PITCH, alternates=None):
        """elements: for a resistor network, the (left pin, right pin) of
        each resistor; each is drawn as a resistor body across the package
        between its two pins, so the part reads as resistors, not a box.

        body_margin: how far the body runs past the first and last side
        pins. A pitch leaves room for pin text; a resistor network, which
        has none, can be drawn tighter so a line passing just above or below
        it clears its body.

        alternates: {pin number: [(name, etype), ...]}, KiCad's alternate
        pin functions. An instance selects one (Sheet.place's alternates)
        where its wiring fixes what a multi-function pin does, so one symbol
        serves every use and ERC still checks each pin as it is used."""
        self.name = name
        self.value = value
        self.footprint = footprint
        self.descr = descr
        self.ref_prefix = ref_prefix
        self.geom = {}
        self.body_margin = body_margin
        self.alternates = alternates or {}
        self._build(pins, width_units)
        if elements:
            self._draw_elements(elements)

    def _draw_elements(self, elements):
        w = self.w_half
        rh, rw = 0.508, 1.27       # half height, half length of each body
        parts = [f'      (rectangle (start {-w} {self.h_half}) '
                 f'(end {w} {-self.h_half})\n'
                 f'        (stroke (width 0.254) (type default)) '
                 f'(fill (type none)))']
        for a, b in elements:
            ya, yb = self.geom[a][1], self.geom[b][1]
            assert abs(ya - yb) < 1e-9, "a resistor's two pins share a row"
            parts.append(f'      (rectangle (start {-rw} {ya + rh}) (end {rw} {ya - rh})\n'
                         f'        (stroke (width 0.254) (type default)) (fill (type none)))')
            for x0, x1 in ((-w, -rw), (rw, w)):
                parts.append(f'      (polyline (pts (xy {x0} {ya}) (xy {x1} {ya}))\n'
                             f'        (stroke (width 0) (type default)) (fill (type none)))')
        self.body = "\n".join(parts)

    def _build(self, pins, width_units):
        left = [p for p in pins if p[3] == "L"]
        right = [p for p in pins if p[3] == "R"]
        top = [p for p in pins if p[3] == "T"]
        bot = [p for p in pins if p[3] == "B"]

        rows = max(len(left), len(right), 1)
        # the side pins are centred on the origin; the body runs
        # body_margin past the first and last
        # (written so the default margin gives the same floats as before)
        half_h = (rows + 1) * PITCH / 2 - (PITCH - self.body_margin)
        top_y = half_h - self.body_margin
        # Wide enough for the left/right pin names and for however many
        # supply pins sit along the top or bottom at their wider pitch.
        # Body width is set by two constraints at once, which is why picking
        # a margin by hand kept trading one collision for another:
        #
        #   the left and right pin names occupy a zone of their own width
        #   inward from each edge, and
        #   the top and bottom pins are centred at a wider pitch, their names
        #   drawn rotated into the body.
        #
        # The outermost supply pin's name must land inside the gap those two
        # zones leave, so solve for the width rather than guess at it.
        n_tb = max(len(top), len(bot))
        tb_pitch = PITCH * 3
        tb_half_span = (n_tb - 1) * tb_pitch / 2 if n_tb else 0.0
        lr_names = [p[1] for p in left + right if p[1] not in ("~", "")]
        max_lr = max((len(n) for n in lr_names), default=0) * 1.27 * 0.72
        need = tb_half_span + max_lr + PITCH + 1.27
        # Round up to the grid. A computed width puts the pins on arbitrary
        # coordinates, and since wire() snaps its endpoints to the grid, every
        # wire then misses its pin by a fraction of a millimetre.
        half_w = max(width_units * PITCH / 2, need)
        half_w = math.ceil(half_w / 1.27 - 1e-9) * 1.27

        self.h_half = half_h
        self.rows = rows
        self.top_pin_y = top_y
        self.w_half = half_w
        # Symbols with supplies on top and bottom draw pin text above and
        # below the body, so their reference and value have to clear that
        # band as well as the body itself.
        self.has_tb = bool(top or bot)
        self.body = (f'      (rectangle (start {-half_w} {half_h}) '
                     f'(end {half_w} {-half_h})\n'
                     f'        (stroke (width 0.254) (type default)) '
                     f'(fill (type background)))')
        self.plines = []

        for i, (num, nm, et, _s) in enumerate(
                [(p[0], p[1], p[2], p[3]) for p in left]):
            if num is None:
                continue
            y = half_h - (i + 1) * PITCH + (PITCH - self.body_margin)
            x = -half_w - PITCH
            self.geom[num] = (x, y, "L")
            self.plines.append(_pin(et, x, y, 0, nm, num,
                                    alts=self.alternates.get(num)))

        for i, (num, nm, et, _s) in enumerate(
                [(p[0], p[1], p[2], p[3]) for p in right]):
            if num is None:
                continue
            y = half_h - (i + 1) * PITCH + (PITCH - self.body_margin)
            x = half_w + PITCH
            self.geom[num] = (x, y, "R")
            self.plines.append(_pin(et, x, y, 180, nm, num,
                                    alts=self.alternates.get(num)))

        # Top and bottom pin names are drawn rotated, so their text runs
        # along the pin rather than across it. At 2.54 mm centres a five-
        # character supply name overlaps its neighbours; 7.62 mm gives them
        # room.
        for i, (num, nm, et, _s) in enumerate(
                [(p[0], p[1], p[2], p[3]) for p in top]):
            x = -(len(top) - 1) * tb_pitch / 2 + i * tb_pitch
            self.geom[num] = (x, half_h + PITCH, "T")
            self.plines.append(_pin(et, x, half_h + PITCH, 270, nm, num))

        for i, (num, nm, et, _s) in enumerate(
                [(p[0], p[1], p[2], p[3]) for p in bot]):
            x = -(len(bot) - 1) * tb_pitch / 2 + i * tb_pitch
            self.geom[num] = (x, -half_h - PITCH, "B")
            self.plines.append(_pin(et, x, -half_h - PITCH, 90, nm, num))

    is_power = False

    def render(self, qualified=False):
        nm = f"fastprom:{self.name}" if qualified else self.name
        # The (power) token is what makes KiCad treat a one-pin symbol as a
        # global rail named after its pin. Without it every instance becomes
        # its own isolated net and ERC reports every rail as undriven.
        power = "    (power)\n" if self.is_power else ""
        hide = ("    (pin_numbers (hide yes)) (pin_names (offset 0) (hide yes))\n"
                if self.is_power or getattr(self, "hide_pins", False) else
                "    (pin_numbers (hide no)) (pin_names (offset 0.254))\n")
        return (
            f'  (symbol "{nm}"\n'
            + power + hide +
            f'    (exclude_from_sim no) (in_bom yes) (on_board yes)\n'
            f'    (property "Reference" "{self.ref_prefix}" (at 0 0 0) '
            f'(effects (font (size 1.27 1.27))))\n'
            f'    (property "Value" "{self.value}" (at 0 0 0) '
            f'(effects (font (size 1.27 1.27))))\n'
            f'    (property "Footprint" "{self.footprint}" (at 0 0 0) '
            f'(effects (font (size 1.27 1.27)) (hide yes)))\n'
            f'    (property "Datasheet" "" (at 0 0 0) '
            f'(effects (font (size 1.27 1.27)) (hide yes)))\n'
            f'    (property "Description" "{self.descr}" (at 0 0 0) '
            f'(effects (font (size 1.27 1.27)) (hide yes)))\n'
            f'    (symbol "{self.name}_0_1"\n{self.body}\n    )\n'
            f'    (symbol "{self.name}_1_1"\n' + "\n".join(self.plines) +
            '\n    )\n  )\n')


class FlagSym(Sym):
    """A power flag: tells ERC a rail arriving through a passive part (a
    ferrite, a diode, a connector contact) is intentionally driven."""

    def __init__(self):
        self.name = "PWR_FLAG"
        # Pin text hidden, as on KiCad's own flag - but not a power symbol:
        # the (power) token would make it a rail named after its pin and
        # silently join every flag on the sheet into one net.
        self.hide_pins = True
        self.value = "PWR_FLAG"
        self.footprint = ""
        self.descr = "Power flag for ERC"
        self.ref_prefix = "#FLG"
        self.h_half = 0.0
        self.w_half = 0.0
        self.geom = {"1": (0.0, 0.0, "T")}
        self.body = ('      (polyline (pts (xy 0 1.27) (xy -1.27 2.54) '
                     '(xy 0 3.81) (xy 1.27 2.54) (xy 0 1.27))\n'
                     '        (stroke (width 0) (type default)) '
                     '(fill (type none)))\n'
                     '      (polyline (pts (xy 0 0) (xy 0 1.27))\n'
                     '        (stroke (width 0) (type default)) '
                     '(fill (type none)))')
        self.plines = [_pin("power_out", 0, 0, 90, "pwr", "1")]


class PowerSym(Sym):
    is_power = True
    h_half = 0.0
    w_half = 0.0

    """A rail symbol: one pin at the origin, graphic drawn away from it.

    Conventional sheets use these instead of naming every rail pin, which is
    most of what makes a sheet look like a schematic rather than a net list.
    """

    def __init__(self, name, up=True):
        """Always upright: a supply's arrow points up from its pin, a
        ground's bars hang down from it. There is no sideways variant: a
        connector's rail contacts are tied into one upright symbol per rail
        (docs/13-schematic-conventions.md; tools/grade_schematic.py grades
        it as rail_sideways)."""
        self.name = name
        self.value = name
        self.footprint = ""
        self.descr = f"{name} rail"
        self.ref_prefix = "#PWR"
        self.up = up
        self.geom = {"1": (0.0, 0.0, "T" if up else "B")}
        d = 1 if up else -1
        tip = 2.54 * d
        if up:
            g = (f'      (polyline (pts (xy 0 0) (xy 0 {tip}))\n'
                 f'        (stroke (width 0) (type default)) (fill (type none)))\n'
                 f'      (polyline (pts (xy -1.27 {tip}) (xy 0 {tip + 1.27}) '
                 f'(xy 1.27 {tip}))\n'
                 f'        (stroke (width 0) (type default)) (fill (type none)))')
        else:
            g = (f'      (polyline (pts (xy 0 0) (xy 0 {tip}))\n'
                 f'        (stroke (width 0) (type default)) (fill (type none)))\n'
                 f'      (polyline (pts (xy -1.905 {tip}) (xy 1.905 {tip}))\n'
                 f'        (stroke (width 0) (type default)) (fill (type none)))\n'
                 f'      (polyline (pts (xy -1.016 {tip - 0.762}) '
                 f'(xy 1.016 {tip - 0.762}))\n'
                 f'        (stroke (width 0) (type default)) (fill (type none)))')
        self.body = g
        self.plines = [_pin("power_in", 0, 0, 90 if up else 270, name, "1")]


def _pin(etype, x, y, angle, name, number, length=PITCH, alts=None):
    alt = "".join(f'\n        (alternate "{n}" {t} line)' for n, t in alts or ())
    return (f'      (pin {etype} line (at {x} {y} {angle}) (length {length})\n'
            f'        (name "{name}" (effects (font (size 1.27 1.27))))\n'
            f'        (number "{number}" (effects (font (size 1.27 1.27))))'
            + alt + ')')


def connector(name, value, footprint, descr, count, side="R",
              etypes=None, names=None):
    """A connector drawn the conventional way: one column of pins."""
    etypes = etypes or {}
    names = names or {}
    pins = [(n, names.get(n, f"{n}"), etypes.get(n, "passive"), side)
            for n in range(1, count + 1)]
    return Sym(name, value, footprint, descr, pins,
               width_units=5, ref_prefix="J")


def _line(pts, width=0.254, fill="none"):
    xy = " ".join(f"(xy {round(x, 4)} {round(y, 4)})" for x, y in pts)
    return (f'      (polyline (pts {xy})\n'
            f'        (stroke (width {width}) (type default)) (fill (type {fill})))')


def _passive_body(kind, vertical, cathode_pos):
    """The graphic between a two-pin part's pins, which sit 2.54 mm beyond
    a 5.08 mm span: drawn horizontally, and turned for a vertical part.
    kind: "R" resistor (a box), "C" capacitor (two plates), "D_S" Schottky
    diode (triangle and hooked bar, cathode toward cathode_pos), "FB"
    ferrite bead (an angled box)."""
    if kind == "C":
        parts = [[(-2.54, 0), (-0.508, 0)], [(0.508, 0), (2.54, 0)]]
        plates = [[(-0.508, -1.016), (-0.508, 1.016)], [(0.508, -1.016), (0.508, 1.016)]]
        shapes = [(p, 0.254, "none") for p in parts] + [(p, 0.3048, "none") for p in plates]
    elif kind == "D_S":
        k = 1 if cathode_pos > 0 else -1       # the bar's side
        tri = [(-1.27 * k, 1.27), (-1.27 * k, -1.27), (1.27 * k, 0), (-1.27 * k, 1.27)]
        bar = [(1.27 * k + 0.635 * k, 1.016), (1.27 * k + 0.635 * k, 1.27),
               (1.27 * k, 1.27), (1.27 * k, -1.27),
               (1.27 * k - 0.635 * k, -1.27), (1.27 * k - 0.635 * k, -1.016)]
        leads = [[(-2.54, 0), (-1.27, 0)], [(1.27, 0), (2.54, 0)]]
        shapes = [(tri, 0.254, "none"), (bar, 0.254, "none")] + [(l, 0.254, "none") for l in leads]
    elif kind == "FB":
        box = [(-1.524, -0.508), (0.508, 1.524), (1.524, 0.508), (-0.508, -1.524), (-1.524, -0.508)]
        leads = [[(-2.54, 0), (-1.016, 0)], [(1.016, 0), (2.54, 0)]]
        shapes = [(box, 0.254, "none")] + [(l, 0.254, "none") for l in leads]
    else:
        return None
    if vertical:   # turn the horizontal drawing a quarter: x along -y
        shapes = [([(-y, x) for x, y in pts], w, f) for pts, w, f in shapes]
    return "\n".join(_line(pts, w, f) for pts, w, f in shapes)


def test_point(footprint="fastprom:TestPoint_Pad_D1.0mm"):
    """A bare test pad, drawn as the pad it is: a circle, its one pin on top
    so the rail it probes stands above it. Reference and value to the right,
    as a passive's are; no pin number, there is only one. The radius keeps
    the pin's end on the 1.27 mm grid that wires snap to."""
    r = 1.27
    s = Sym("TestPoint_W", "TestPoint", footprint, "Bare test pad",
            [("1", "1", "passive", "T")], width_units=1, ref_prefix="TP")
    s.geom = {"1": (0.0, r + PITCH, "T")}
    s.plines = [_pin("passive", 0.0, r + PITCH, 270, "1", "1")]
    s.body = (f'      (circle (center 0 0) (radius {r})\n'
              f'        (stroke (width 0.254) (type default)) (fill (type none)))')
    s.h_half = s.w_half = r
    s.has_tb = False
    s.text_layout = "side"
    s.hide_pins = True
    return s


def two_pin(name, value, footprint, descr, ref_prefix="R", vertical=True,
            swap=False, kind=None):
    """A passive, drawn vertically (the conventional orientation for a
    resistor or capacitor hanging off a signal line to a rail). kind picks
    the graphic ("R", "C", "D_S", "FB"); by default it follows ref_prefix
    (C a capacitor, FB a ferrite bead, D a Schottky diode, else a resistor)."""
    if kind is None:
        kind = {"C": "C", "FB": "FB", "D": "D_S"}.get(ref_prefix, "R")
    if vertical:
        pins = [("1", "~", "passive", "T"), ("2", "~", "passive", "B")]
    else:
        pins = [("1", "~", "passive", "L"), ("2", "~", "passive", "R")]
    s = Sym(name, value, footprint, descr, pins, width_units=1,
            ref_prefix=ref_prefix)
    # The body has to follow the orientation. Drawing a vertical rectangle for
    # a horizontally-mounted passive is why several of these overlapped their
    # neighbours and had wires run straight through them.
    if vertical:
        s.body = ('      (rectangle (start -1.016 2.54) (end 1.016 -2.54)\n'
                  '        (stroke (width 0.254) (type default)) '
                  '(fill (type none)))')
    else:
        s.body = ('      (rectangle (start -2.54 1.016) (end 2.54 -1.016)\n'
                  '        (stroke (width 0.254) (type default)) '
                  '(fill (type none)))')
    # A diode's cathode is pin 1 (KiCad's convention); which side that is
    # depends on swap, as below.
    if vertical:
        cath = 5.08 if not swap else -5.08
    else:
        cath = -5.08 if not swap else 5.08
    body = _passive_body(kind, vertical, cath)
    if body:
        s.body = body
    s.h_half = 2.54 if vertical else 1.016
    s.w_half = 1.016 if vertical else 2.54
    s.text_layout = "side" if vertical else "stack"
    s.has_tb = vertical
    # a two-pin part's numbers are noise: a resistor's, capacitor's or
    # bead's ends are interchangeable, and a diode's graphic shows which end
    # is the cathode (pin 1)
    s.hide_pins = True
    if vertical:
        # swap puts pin 2 at the top, which an LED wants: KiCad's convention
        # is pin 1 = cathode, so the anode has to be the upper pin for the
        # part to be drawn the right way up and still match the netlist.
        top, bot = ("2", "1") if swap else ("1", "2")
        s.geom = {top: (0.0, 5.08, "T"), bot: (0.0, -5.08, "B")}
        s.plines = [_pin("passive", 0, 5.08, 270, "~", top),
                    _pin("passive", 0, -5.08, 90, "~", bot)]
    else:
        # swap puts pin 2 on the left, which a diode wants: anode left,
        # cathode right, so current and the stub wires both run outward.
        lo, hi = ("2", "1") if swap else ("1", "2")
        s.geom = {lo: (-5.08, 0.0, "L"), hi: (5.08, 0.0, "R")}
        s.plines = [_pin("passive", -5.08, 0, 0, "~", lo),
                    _pin("passive", 5.08, 0, 180, "~", hi)]
    return s


# ---------------------------------------------------------------------------
# Sheet
# ---------------------------------------------------------------------------

class Sheet:
    def __init__(self, project, paper="A3", title="", rev="", company="",
                 comment=""):
        self.project = project
        self.comment = comment
        self.paper = paper
        self.title = title
        self.rev = rev
        self.company = company
        self.sheet_uuid = uid(f"sheet-{project}")
        self.syms = {}
        self.items = []
        self.pin_points = set()   # every placed part's pin ends (not rail symbols')
        self.refs = {}
        self.wires = []
        self.juncs = set()
        # Functional groups: everything drawn while self.group names one
        # records its extent there, and block() then draws a captioned
        # rectangle round the group.
        self.group = None
        self.extents = {}
        self.boxes = []

    # ---- extents, for the captioned blocks -------------------------------
    def _ext(self, box):
        if self.group is not None:
            self.extents.setdefault(self.group, []).append(box)

    def _text_ext(self, txt, x, y, size=1.27, just="center"):
        w = max(len(txt), 1) * size * TEXT_W
        h = size * 1.3
        x0 = x if just == "left" else x - w if just == "right" else x - w / 2
        self._ext((x0, y - h / 2, x0 + w, y + h / 2))

    def extent(self, group):
        es = self.extents.get(group, [])
        if not es:
            raise ValueError(f"group {group!r} has nothing in it")
        return (min(e[0] for e in es), min(e[1] for e in es),
                max(e[2] for e in es), max(e[3] for e in es))

    def block(self, group, caption, pad=2.54, grow=None):
        """A captioned rectangle round everything drawn in group, the way
        One ROM and the CM4 IO board frame each functional block. grow:
        (x0, y0, x1, y1), any of them None, to stretch the box to a line
        shared with a neighbouring block. Returns the rectangle."""
        x0, y0, x1, y1 = self.extent(group)
        cap = CAPTION_H
        box = [x0 - pad, y0 - pad - cap, x1 + pad, y1 + pad]
        # outward to the 1.27 mm grid, so edges line up between blocks
        box = [math.floor(box[0] / 1.27) * 1.27, math.floor(box[1] / 1.27) * 1.27,
               math.ceil(box[2] / 1.27) * 1.27, math.ceil(box[3] / 1.27) * 1.27]
        for i, v in enumerate(grow or ()):
            if v is not None:
                box[i] = min(box[i], v) if i < 2 else max(box[i], v)
        box = [round(v, 2) for v in box]
        self.rect(box)
        g, self.group = self.group, None
        self.text(caption, (box[0] + 1.27, box[1] + cap / 2 + 0.3), 1.27)
        self.group = g
        self.boxes.append((caption, tuple(box)))
        return tuple(box)

    def frame_box(self, group, pad=2.54):
        """The rectangle block() would draw round group, on the 1.27 mm grid."""
        x0, y0, x1, y1 = self.extent(group)
        box = [x0 - pad, y0 - pad - CAPTION_H, x1 + pad, y1 + pad]
        return [math.floor(box[0] / 1.27) * 1.27, math.floor(box[1] / 1.27) * 1.27,
                math.ceil(box[2] / 1.27) * 1.27, math.ceil(box[3] / 1.27) * 1.27]

    def frames(self, captions, align=()):
        """Frame every group in captions ({group: caption}). align: (edge,
        [groups]) pairs, edge one of left/top/right/bottom, each moving that
        edge of those groups' frames out to the outermost of them, so blocks
        in a row or a column share a line. Returns {group: box}."""
        boxes = {g: self.frame_box(g) for g in captions}
        for g, cap in captions.items():      # wide enough for the caption
            need = boxes[g][0] + 2.54 + len(cap) * 1.27 * TEXT_W
            boxes[g][2] = max(boxes[g][2], math.ceil(need / 1.27) * 1.27)
        for edge, groups in align:
            i = ("left", "top", "right", "bottom").index(edge)
            v = (min if i < 2 else max)(boxes[g][i] for g in groups)
            for g in groups:
                boxes[g][i] = v
        for g, cap in captions.items():
            self.block(g, cap, grow=boxes[g])
        return {g: tuple(round(v, 2) for v in boxes[g]) for g in captions}

    def rect(self, box):
        x0, y0, x1, y1 = box
        self.items.append(
            f'  (rectangle (start {x0} {y0}) (end {x1} {y1})\n'
            f'    (stroke (width 0) (type default)) (fill (type none)) '
            f'(uuid "{uid()}"))\n')

    def use(self, sym):
        self.syms[sym.name] = sym
        return sym

    def place(self, sym, ref, value=None, at=(0, 0), footprint=None,
              dnp=False, hide_value=False, ref_pos=None, val_pos=None,
              alternates=None):
        """Place a symbol. Returns {pin: (x, y)} in sheet coordinates.

        ref_pos / val_pos override where the reference and value text go:
        (dx, dy, justify) relative to the symbol origin, or "hide". Automatic
        placement cannot know what the generator is about to draw next to a
        part, so a sheet drawn to hand-drafting standards needs to say.
        """
        self.use(sym)
        X, Y = snap(at[0]), snap(at[1])
        value = value if value is not None else sym.value
        footprint = footprint if footprint is not None else sym.footprint

        hh = getattr(sym, "h_half", 2.54)
        wh = getattr(sym, "w_half", 2.54)
        hide_ref = " (hide yes)" if ref.startswith("#") else ""
        hide_val = (" (hide yes)"
                    if hide_value or ref.startswith("#") else "")
        if getattr(sym, "text_layout", "stack") == "side":
            # reference over value, both to the right of the body, as KiCad
            # places them: split either side, a part's value sits between it
            # and the next part's reference and reads as either's
            ref_at = f"{round(X + wh + 1.5, 2)} {round(Y - 1.27, 2)} 0"
            ref_just = " (justify left)"
            val_at = f"{round(X + wh + 1.5, 2)} {round(Y + 1.27, 2)} 0"
            val_just = " (justify left)"
        else:
            # 1.6 mm of clearance, not 2.54: at 2.54 the reference of a part
            # in a staggered column lands on the body of the one above it.
            # Parts with top/bottom pins need to clear that pin text too.
            clear = 6.0 if getattr(sym, "has_tb", False) else 1.6
            ref_at = f"{X} {Y - hh - clear} 0"
            ref_just = ""
            val_at = f"{X} {Y + hh + clear} 0"
            val_just = ""
        if getattr(sym, "is_power", False) and ref.startswith("#PWR"):
            # Show the rail name. With +3V3 and +3V3_MCU both on the sheet,
            # a rail symbol whose name is hidden is ambiguous.
            hide_val = ""
            dy = -5.08 if getattr(sym, "up", True) else 5.08
            val_at = f"{X} {round(Y + dy, 2)} 0"
            val_just = ""

        def _override(pos, at, just, hide):
            if pos is None:
                return at, just, hide
            if pos == "hide":
                return at, just, " (hide yes)"
            dx, dy, j = pos
            return (f"{round(X + dx, 2)} {round(Y + dy, 2)} 0",
                    "" if j == "center" else f" (justify {j})", hide)

        ref_at, ref_just, hide_ref = _override(ref_pos, ref_at, ref_just,
                                               hide_ref)
        val_at, val_just, hide_val = _override(val_pos, val_at, val_just,
                                               hide_val)

        props = [
            f'    (property "Reference" "{ref}" (at {ref_at}) '
            f'(effects (font (size 1.27 1.27)){ref_just}{hide_ref}))',
            f'    (property "Value" "{value}" (at {val_at}) '
            f'(effects (font (size 1.27 1.27)){val_just}{hide_val}))',
            f'    (property "Footprint" "{footprint}" (at {X} {Y} 0) '
            f'(effects (font (size 1.27 1.27)) (hide yes)))',
        ]
        alternates = alternates or {}
        for p, a in alternates.items():
            assert a in (n for n, _t in sym.alternates.get(p, ())), (ref, p, a)
        pin_uuids = "\n".join(
            f'    (pin "{p}" (uuid "{uid()}")'
            + (f' (alternate "{alternates[p]}")' if p in alternates else "")
            + ')' for p in sym.geom)

        self.items.append(
            f'  (symbol (lib_id "fastprom:{sym.name}") (at {X} {Y} 0) (unit 1)\n'
            f'    (exclude_from_sim no) (in_bom '
            f'{"no" if ref.startswith("#") else "yes"}) (on_board yes) '
            f'(dnp {"yes" if dnp else "no"})\n'
            f'    (uuid "{uid()}")\n' + "\n".join(props) + "\n" + pin_uuids +
            "\n    (instances (project \"" + self.project +
            f'" (path "/{self.sheet_uuid}" (reference "{ref}") (unit 1))))\n'
            f'  )\n')

        out = {}
        for p, (px, py, _side) in sym.geom.items():
            out[p] = (round(X + px, 2), round(Y - py, 2))
        if not ref.startswith("#"):
            self.pin_points.update(out.values())
        self._place_ext(sym, X, Y, out, value, props)
        return out

    def _place_ext(self, sym, X, Y, pins, value, props):
        if self.group is None:
            return
        if isinstance(sym, FlagSym):
            self._ext((X - 1.27, Y - 3.81, X + 1.27, Y))
            return
        if getattr(sym, "is_power", False):
            if sym.up:
                self._ext((X - 1.27, Y - 3.81, X + 1.27, Y))
                self._text_ext(value, X, Y - 5.08)
            else:
                self._ext((X - 1.905, Y, X + 1.905, Y + 3.3))
                self._text_ext(value, X, Y + 5.08)
            return
        wh, hh = sym.w_half, sym.h_half
        self._ext((X - wh, Y - hh, X + wh, Y + hh))
        for pt in pins.values():
            self._ext((pt[0], pt[1], pt[0], pt[1]))
        for pr in props[:2]:
            m = re.search(r'"(?:Reference|Value)" "([^"]*)" \(at ([-\d.]+) ([-\d.]+) 0\)', pr)
            if not m or "(hide yes)" in pr:
                continue
            j = re.search(r'\(justify (\w+)\)', pr)
            self._text_ext(m.group(1), float(m.group(2)), float(m.group(3)),
                           just=j.group(1) if j else "center")

    def wire(self, a, b):
        # Snap endpoints to the same grid placement uses. Every pin on this
        # sheet lands on that grid, so snapping here means a caller passing a
        # nominal coordinate still meets the pin exactly - which is the
        # difference between a connected net and a silently split one.
        ax, ay = snap(a[0]), snap(a[1])
        bx, by = snap(b[0]), snap(b[1])
        if (ax, ay) == (bx, by):
            return
        if abs(ax - bx) > 1e-6 and abs(ay - by) > 1e-6:
            raise ValueError(f"non-orthogonal wire {a} -> {b}")
        self.wires.append((ax, ay, bx, by))
        self._ext((min(ax, bx), min(ay, by), max(ax, bx), max(ay, by)))
        self.items.append(
            f'  (wire (pts (xy {ax} {ay}) (xy {bx} {by}))\n'
            f'    (stroke (width 0) (type default)) (uuid "{uid()}"))\n')

    def junction(self, pt):
        p = (snap(pt[0]), snap(pt[1]))
        if p in self.juncs:
            return
        self.juncs.add(p)
        self.items.append(
            f'  (junction (at {p[0]} {p[1]}) (diameter 0) '
            f'(color 0 0 0 0) (uuid "{uid()}"))\n')

    def no_connect(self, pt):
        self._ext((pt[0] - 0.7, pt[1] - 0.7, pt[0] + 0.7, pt[1] + 0.7))
        self.items.append(
            f'  (no_connect (at {round(pt[0], 2)} {round(pt[1], 2)}) '
            f'(uuid "{uid()}"))\n')

    def label(self, net, pt, rot=0, justify="left"):
        self.items.append(
            f'  (global_label "{net}" (shape bidirectional) '
            f'(at {round(pt[0], 2)} {round(pt[1], 2)} {rot})\n'
            f'    (effects (font (size 1.27 1.27)) (justify {justify})) '
            f'(uuid "{uid()}"))\n')

    def name(self, net, pt, rot=0):
        """Name the wire at pt. A local label, sitting just above the wire.

        This is naming, not connecting: the wire it sits on must already be
        drawn to everything on the net. tools/grade_schematic.py fails any
        sheet where a label name appears on two separate wire islands,
        unless all but one of them is a connector contact's stub, and a
        label joining islands must sit on a stub's free end.
        """
        just = "left bottom" if rot == 0 else "right bottom"
        x, y = snap(pt[0]), snap(pt[1])
        w = max(len(net), 1) * 1.27 * TEXT_W
        self._ext((x + 0.25, y - 1.9, x + w + 0.25, y) if rot == 0 else
                  (x - w - 0.25, y - 1.9, x - 0.25, y))
        self.items.append(
            f'  (label "{net}" (at {snap(pt[0])} {snap(pt[1])} {rot})\n'
            f'    (fields_autoplaced yes)\n'
            f'    (effects (font (size 1.27 1.27)) (justify {just})) '
            f'(uuid "{uid()}"))\n')

    def text(self, s, pt, size=1.27):
        """Free text, left-justified at pt, at KiCad's default size."""
        self._text_ext(s, pt[0], pt[1], size, "left")
        self.items.append(
            f'  (text "{s}" (at {round(pt[0], 2)} {round(pt[1], 2)} 0)\n'
            f'    (effects (font (size {size} {size})) '
            f'(justify left)) (uuid "{uid()}"))\n')

    def table(self, origin, col_widths, rows, row_h=3.81, size=1.27):
        """A KiCad table, its top-left corner at origin; rows[0] is the
        header. Cells are left-justified text at size mm."""
        x0, y0 = origin
        cells = []
        for r, row in enumerate(rows):
            x = x0
            for c, txt in enumerate(row):
                w = col_widths[c]
                cells.append(
                    f'      (table_cell "{txt}" (exclude_from_sim no)\n'
                    f'        (at {round(x, 3)} {round(y0 + r * row_h, 3)} 0) '
                    f'(size {w} {row_h}) (margins 0.635 0.635 0.635 0.635) '
                    f'(span 1 1) (fill (type none))\n'
                    f'        (effects (font (size {size} {size})) (justify left top)) '
                    f'(uuid "{uid()}"))\n')
                x += w
        stroke = "(stroke (width 0) (type solid))"
        self.items.append(
            f'  (table (column_count {len(col_widths)})\n'
            f'    (border (external yes) (header yes) {stroke})\n'
            f'    (separators (rows yes) (cols yes) {stroke})\n'
            f'    (column_widths {" ".join(str(w) for w in col_widths)})\n'
            f'    (row_heights {" ".join(str(row_h) for _ in rows)})\n'
            f'    (cells\n' + "".join(cells) + '    )\n  )\n')
        box = (x0, y0, x0 + sum(col_widths), y0 + len(rows) * row_h)
        self._ext(box)
        return box

    def route(self, a, b, via_x):
        """Orthogonal route: horizontal to via_x, vertical, horizontal in."""
        vx = snap(via_x)
        if abs(snap(a[1]) - snap(b[1])) < 1e-6:
            self.wire(a, b)
            return
        self.wire(a, (vx, a[1]))
        self.wire((vx, a[1]), (vx, b[1]))
        self.wire((vx, b[1]), b)

    def detour(self, a, b, y, stub=5.08, a_sign=None, b_sign=None):
        """Route a to b the long way round, via a clear horizontal corridor.

        For a signal that has to get past a bundle occupying the direct path:
        out of the pin, down (or up) to a corridor nothing else uses, across,
        and back in. Five segments instead of three, and it crosses nothing.
        """
        # Which way to step out of each pin. Defaults assume the two face
        # each other; pass explicit signs when a pin faces away, or the stub
        # is drawn back through its own symbol and across whatever that
        # symbol's other pins connect to.
        if a_sign is None:
            a_sign = 1 if b[0] > a[0] else -1
        if b_sign is None:
            b_sign = -1 if b[0] > a[0] else 1
        ax = a[0] + stub * a_sign
        bx = b[0] + stub * b_sign
        self.wire(a, (ax, a[1]))
        self.wire((ax, a[1]), (ax, y))
        self.wire((ax, y), (bx, y))
        self.wire((bx, y), (bx, b[1]))
        self.wire((bx, b[1]), b)

    def bus(self, pairs, x_start, x_end, track_pitch=1.27):
        """Route a bundle of nets as parallel staircases, crossing-free.

        The obvious scheme - one vertical per net, tracks left to right in
        source order - only works while each net shifts less than the pin
        pitch. Once a net has to travel further than that, its vertical spans
        the destination rows of its neighbours, and every one of those
        destination segments crosses it.

        The fix is that within a group of nets travelling the same direction,
        the verticals must be ordered *opposite* to the horizontals: the net
        that travels furthest gets the outermost track, so its long vertical
        sits to the left of every segment that would otherwise cross it.

        Nets going up and nets going down do not interact - one group's
        verticals live in the top rows and the other's in the bottom - so they
        are given separate bands and routed independently. Nets whose ends
        are level need no vertical at all.

        Assumes both ends of the bundle are in the same relative order, which
        is why the connector's contacts are ordered to match the socket's pins.
        """
        ups, downs, level = [], [], []
        for i, (a, b) in enumerate(pairs):
            if b[1] < a[1] - 1e-6:
                ups.append(i)
            elif b[1] > a[1] + 1e-6:
                downs.append(i)
            else:
                level.append(i)

        for i in level:
            self.wire(pairs[i][0], pairs[i][1])

        # Furthest-travelling first in each group.
        ups.sort(key=lambda i: pairs[i][0][1])
        downs.sort(key=lambda i: -pairs[i][0][1])

        n = len(ups) + len(downs)
        span = x_end - x_start
        pitch = min(track_pitch, span / (n + 1)) if n and span > 0 \
            else track_pitch

        slot = 0
        for group in (ups, downs):
            for i in group:
                self.route(pairs[i][0], pairs[i][1],
                           x_start + (slot + 1) * pitch)
                slot += 1

    def render(self):
        libs = "".join(s.render(qualified=True) for s in self.syms.values())
        return ("(kicad_sch\n"
                "  (version 20250114)\n"
                '  (generator "fastprom")\n'
                '  (generator_version "10.0")\n'
                f'  (uuid "{self.sheet_uuid}")\n'
                f'  (paper "{self.paper}")\n'
                + (f'  (title_block\n'
                   f'    (title "{self.title}")\n'
                   f'    (company "{self.company}")\n'
                   f'    (rev "{self.rev}")\n'
                   + (f'    (comment 1 "{self.comment}")\n' if self.comment else "")
                   + f'  )\n' if self.title else "")
                + f"  (lib_symbols\n{libs}  )\n"
                + "".join(self.items) + ")\n")

    def write(self, path):
        with open(path, "w") as fh:
            fh.write(self.render())
