#!/usr/bin/env python3
"""Generate the KiCad project: symbol library, schematic, and library tables.

The schematic is generated rather than drawn so that it cannot drift from
tools/boardmap.yaml. Change the board map, regenerate, re-run ERC.

This is the REFERENCE generator. Its sheets are not meant for reading: the
two stack boards are drawn for people by tools/gen_wired.py, and this one's
job is to produce the reference netlists in docs/netlist/ that the drawn
sheets are proven equivalent to.

Connectivity is expressed with global labels on short stubs off every pin,
which is unambiguous to KiCad's netlister and keeps the generator from having
to solve a routing problem. Pins that are deliberately unconnected get an
explicit no-connect so ERC can tell intent from oversight.
"""
import argparse
import os
import re
import uuid

import yaml

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
HW = os.path.join(ROOT, "hw")

PITCH = 2.54
STUB = 5.08


# Deterministic UUIDs. Random ones would make every regeneration produce a
# diff against the committed files, which destroys the point of generating
# them: a real change to the board map should be visible in the diff, not
# buried in a few hundred changed identifiers.
_UUID_NS = uuid.UUID("6f6173cf-0000-4000-8000-000000000000")
_uid_seq = [0]


def uid(key=None):
    if key is None:
        _uid_seq[0] += 1
        key = f"seq-{_uid_seq[0]}"
    return str(uuid.uuid5(_UUID_NS, key))


def reset_uids():
    _uid_seq[0] = 0


# ---------------------------------------------------------------------------
# Symbol definitions. Pin entries are (number, name, etype, side).
# 'side' is L or R for IC-style symbols, T or B for two-pin parts.
# ---------------------------------------------------------------------------

# The pinout comes from AGM's pinout spreadsheet
# (AG32_pinout_100_64_48_32_2K.xlsx, cited in docs/00-ag32-reference.md),
# which is the authoritative source
# and covers more packages than the datasheet. The QFN-32 column used is
# AG32VFxxxK - the MCU variant, where pin 30 is BOOT0. On the AGRV2KQ32
# CPLD-branded SKU that same pin is GND; they are different SKUs of one die.
AG32_Q32 = {
    4: "NRST", 6: "VDDA33", 16: "VDD33", 17: "GND", 30: "BOOT0",
    32: "VDD33",
    # The exposed pad. The package has a single ground pin (17), so the pad
    # is tied to ground too; if it is in fact unconnected inside, tying it
    # to ground is harmless.
    33: "EP",
}
for _n in [1, 2, 3, 5] + list(range(7, 16)) + list(range(18, 30)) + [31]:
    AG32_Q32[_n] = f"PIN_{_n}"

AG32_PACKAGES = {
    "QFN-32": {
        "symbol": "AG32VF303KCU6",
        "pins": AG32_Q32,
        "footprint": "fastprom:QFN-32-1EP_4x4mm_P0.4mm_EP2.65x2.65mm",
        "split": 16,
    },
}

# Image-select switch: XKB DSHP02TS-S (LCSC C319052), a half-pitch 2-way slide
# switch on KingTek's DSHP02TS land pattern.
SW_DIP_FP = "fastprom:SW_DIP_SPSTx02_Slide_KingTek_DSHP02TS_W7.62mm_P1.27mm"

# SN74LVTH245A octal transceiver.
# SN74LVT245B / SN74LVTH245A share this pinout.
LVTH245_PINS = [
    (1, "DIR", "input", "L"),
    (2, "A1", "bidirectional", "L"),
    (3, "A2", "bidirectional", "L"),
    (4, "A3", "bidirectional", "L"),
    (5, "A4", "bidirectional", "L"),
    (6, "A5", "bidirectional", "L"),
    (7, "A6", "bidirectional", "L"),
    (8, "A7", "bidirectional", "L"),
    (9, "A8", "bidirectional", "L"),
    (10, "GND", "power_in", "L"),
    (19, "~{OE}", "input", "R"),
    (18, "B1", "bidirectional", "R"),
    (17, "B2", "bidirectional", "R"),
    (16, "B3", "bidirectional", "R"),
    (15, "B4", "bidirectional", "R"),
    (14, "B5", "bidirectional", "R"),
    (13, "B6", "bidirectional", "R"),
    (12, "B7", "bidirectional", "R"),
    (11, "B8", "bidirectional", "R"),
    (20, "VCC", "power_in", "R"),
    # The exposed pad. TI: connect to ground.
    (21, "EP", "passive", "L"),
]

LDO_PINS = [
    (1, "VIN", "power_in", "L"),
    (3, "EN", "input", "L"),
    (2, "GND", "power_in", "L"),
    (5, "VOUT", "power_out", "R"),
    (4, "NC", "no_connect", "R"),
]


def build_symbols():
    """Return {name: (etype_map, sexp)} for every symbol we define."""
    syms = {}

    def ic(name, pins, value, footprint, desc):
        lefts = [p for p in pins if p[3] == "L"]
        rights = [p for p in pins if p[3] == "R"]
        rows = max(len(lefts), len(rights))
        half_h = (rows + 1) * PITCH / 2
        half_w = 6 * PITCH
        body = (f'      (rectangle (start {-half_w} {half_h}) (end {half_w} {-half_h})\n'
                f'        (stroke (width 0.254) (type default)) (fill (type background)))')
        geom = {}
        plines = []
        for i, (num, pname, etype, _s) in enumerate(lefts):
            y = half_h - (i + 1) * PITCH
            x = -half_w - PITCH
            geom[num] = (x, y, "L")
            plines.append(_pin(etype, x, y, 0, pname, num))
        for i, (num, pname, etype, _s) in enumerate(rights):
            y = half_h - (i + 1) * PITCH
            x = half_w + PITCH
            geom[num] = (x, y, "R")
            plines.append(_pin(etype, x, y, 180, pname, num))
        syms[name] = (geom, _symbol(name, value, footprint, desc, body, plines))

    def two_pin(name, value, footprint, desc, n1="1", n2="2",
                e1="passive", e2="passive", body=None):
        geom = {n1: (0, 5.08, "T"), n2: (0, -5.08, "B")}
        if body is None:
            body = ('      (rectangle (start -1.016 2.54) (end 1.016 -2.54)\n'
                    '        (stroke (width 0.254) (type default)) (fill (type none)))')
        plines = [_pin(e1, 0, 5.08, 270, "~", n1),
                  _pin(e2, 0, -5.08, 90, "~", n2)]
        syms[name] = (geom, _symbol(name, value, footprint, desc, body, plines))

    # One symbol per package; the board map selects which is placed.
    for pkg, info in AG32_PACKAGES.items():
        ag_pins = []
        for n in sorted(info["pins"]):
            nm = info["pins"][n]
            if nm.startswith(("VDD", "GND")):
                et = "power_in"
            elif nm == "NRST":
                et = "input"
            elif nm == "EP":
                et = "passive"
            else:
                et = "bidirectional"
            ag_pins.append((n, nm, et, "L" if n <= info["split"] else "R"))
        ic(info["symbol"], ag_pins, info["symbol"], info["footprint"],
           f"RISC-V MCU with AGRV2K programmable logic fabric, {pkg}")

    # SN74LVT245B rather than the LVTH part: identical switching
    # characteristics (verified from the datasheet), no bus hold - which is
    # what we want on pins that receive from an unknown host - and unlike
    # every TSSOP variant in this family it is actually in stock.
    #
    # The footprint is TI's RGY package, a 20-pin VQFN at 3.5 x 4.5 mm on
    # 0.5 mm pitch. KiCad's stock Texas_RGY_R-PVQFN-N20 matches TI's RGY0020A
    # land pattern, including the 2.05 x 3.05 mm exposed pad.
    # Two symbols for the one part: its I/O are bidirectional, but every use
    # here wires DIR, so each side is typed for the direction it really has -
    # inputs where the signal arrives, outputs where it leaves. ERC then sees
    # unused inputs tied to ground as inputs, which is what they are.
    for suffix, a_type, b_type, how in (("_AB", "input", "output", "A to B"),
                                        ("_BA", "output", "input", "B to A")):
        pins = [(n, nm, a_type if nm[0] == "A" and nm[1:].isdigit() else
                 b_type if nm[0] == "B" and nm[1:].isdigit() else et, s)
                for (n, nm, et, s) in LVTH245_PINS]
        ic("SN74LVT245B" + suffix, pins, "SN74LVT245B",
           "fastprom:Texas_RGY_R-PVQFN-N20_EP2.05x3.05mm",
           "Octal transceiver, 3.3V, I/O rated to 5.5V, 64mA sink, no bus "
           f"hold; wired {how}")

    # The drive buffer's own small regulator: 1 x 1 mm, 200 mA (TI TLV742P,
    # DQN). Its thermal pad is left off the land pattern: it sits too close
    # to the pins, and at ~30 mA it is not needed (see the footprint).
    ic("TLV74233", [(4, "IN", "power_in", "L"), (3, "EN", "input", "L"),
                    (2, "GND", "power_in", "L"), (1, "OUT", "power_out", "R")],
       "TLV74233", "fastprom:Texas_X2SON-4_1x1mm_P0.65mm_NoEP", "3.3V 200mA LDO regulator")

    ic("AP2112K-3.3", LDO_PINS, "AP2112K-3.3",
       "fastprom:SOT-23-5",
       "3.3V 600mA LDO regulator")

    # 4-pin CMOS clock oscillator, the common 2.0 x 1.6 mm pinout: 1 OE,
    # 2 GND, 3 OUT, 4 VDD.
    # Four isolated resistors in one 2 x 1 mm package: element k is pins k
    # and 9 - k.
    ic("R_Pack04", [(1, "R1.1", "passive", "L"), (2, "R2.1", "passive", "L"),
                    (3, "R3.1", "passive", "L"), (4, "R4.1", "passive", "L"),
                    (8, "R1.2", "passive", "R"), (7, "R2.2", "passive", "R"),
                    (6, "R3.2", "passive", "R"), (5, "R4.2", "passive", "R")],
       "R_Pack04", "fastprom:R_Array_Convex_4x0402",
       "4 x 0402 resistor array, convex terminals")
    ic("OSC_4PIN", [(1, "OE", "input", "L"), (2, "GND", "power_in", "L"),
                    (4, "VDD", "power_in", "R"), (3, "OUT", "output", "R")],
       "OSC", "fastprom:Oscillator_SMD_SiT_PQFN-4Pin_2.0x1.6mm",
       "CMOS clock oscillator")

    two_pin("R", "R", "fastprom:R_0402_1005Metric", "Resistor")
    # Pin 1 is the cathode and pin 2 the anode, following KiCad's Device:D.
    two_pin("D_Schottky", "D_Schottky",
            "fastprom:D_SOD-323", "Schottky diode, power OR-ing",
            n1="1", n2="2")
    two_pin("C", "C", "fastprom:C_0402_1005Metric", "Capacitor")
    two_pin("FerriteBead", "FerriteBead",
            "fastprom:L_0603_1608Metric", "Ferrite bead")
    # A bare pad for probing a rail.
    ic("TestPoint", [(1, "1", "passive", "L")], "TestPoint",
       "fastprom:TestPoint_Pad_D1.0mm", "Bare test pad")

    # The 20 downward pins that plug into the host's DIP socket.
    dip = []
    for n in range(1, 21):
        # Pin 10 is GND and pin 20 is VCC on every 20-pin bipolar PROM, so the
        # socket really is this board's power source - declaring those pins as
        # power outputs is both true and what ERC needs to see.
        etype = "power_out" if n in (10, 20) else "passive"
        dip.append((n, f"P{n}", etype, "L" if n <= 10 else "R"))
    ic("DIP20_PLUG", dip, "DIP20_PLUG",
       "Package_DIP:DIP-20_W7.62mm",
       "Machined round pins, plugs into the host 20-pin DIP socket")

    # Hirose DF40C-30, 0.4 mm pitch board-to-board, 30 contacts in two rows
    # of 15. 8.6 x 2.94 mm body, 0.3 A per contact, 1.5 mm mated height.
    # Chosen over the shorter DF40C-24 because 30 contacts leave enough spare
    # to interleave grounds between the signal lines, and over the 0.9 mm DF30
    # because the stack height has a floor set by the DIP pin solder fillets.
    # KiCad carries verified footprints for both halves, which the RGY
    # transceiver package notably does not.
    for role, suffix, fp in (
        ("header", "DP", "fastprom:Hirose_DF40C-30DP-0.4V_2x15-1MP_P0.4mm"),
        ("socket", "DS", "fastprom:Hirose_DF40HC(3.0)-30DS-0.4V_2x15_P0.4mm"),
    ):
        b2b = [(n, f"C{n}", "passive", "L" if n <= 15 else "R")
               for n in range(1, 31)]
        ic(f"DF40C-30{suffix}", b2b, f"DF40C-30{suffix}-0.4V", fp,
           f"0.4mm board-to-board {role}, 30 contacts")

    # USB 2.0 Type-C receptacle, 16 signal pads plus shell. Only the USB 2.0
    # subset is used: both D+ pairs and both D- pairs are tied together so the
    # cable works either way up, and CC1/CC2 each get a pull-down so the port
    # presents itself as a device.
    # VBUS is declared power_out on A4 only, because from the board's point of
    # view the cable is a power source - the same reasoning as the DIP plug's
    # pin 20. The other three VBUS pads are the same node inside the
    # connector, so they are passive; declaring all four as sources would be
    # four power outputs shorted together.
    usb_pins = [
        ("A1",  "GND",   "passive", "L"),
        ("A4",  "VBUS",  "power_out", "L"),
        ("A5",  "CC1",   "passive", "L"),
        ("A6",  "DP1",   "bidirectional", "L"),
        ("A7",  "DN1",   "bidirectional", "L"),
        ("A8",  "SBU1",  "passive", "L"),
        ("A9",  "VBUS",  "passive", "L"),
        ("A12", "GND",   "passive", "L"),
        ("B1",  "GND",   "passive", "R"),
        ("B4",  "VBUS",  "passive", "R"),
        ("B5",  "CC2",   "passive", "R"),
        ("B6",  "DP2",   "bidirectional", "R"),
        ("B7",  "DN2",   "bidirectional", "R"),
        ("B8",  "SBU2",  "passive", "R"),
        ("B9",  "VBUS",  "passive", "R"),
        ("B12", "GND",   "passive", "R"),
        ("SH",  "SHIELD", "passive", "R"),
    ]
    ic("USB_C_Receptacle", usb_pins, "USB_C_Receptacle_USB2.0",
       USB_C_FP_STACK,
       "USB 2.0 Type-C receptacle")

    # ESD clamp on the exposed data lines. The AG32's USB PHY is not 5 V
    # tolerant and this connector is the only thing on the board a person
    # touches, so it is not optional. I/O1 appears on pins 1 and 6 and I/O2 on
    # pins 3 and 4 - internally the same node each - so each line is routed in
    # one pin and out the other.
    esd_pins = [
        (1, "IO1_A", "bidirectional", "L"),
        (2, "GND",   "power_in", "L"),
        (3, "IO2_A", "bidirectional", "L"),
        (4, "IO2_B", "bidirectional", "R"),
        (5, "VBUS",  "power_in", "R"),
        (6, "IO1_B", "bidirectional", "R"),
    ]
    ic("USBLC6-2SC6", esd_pins, "USBLC6-2SC6",
       "fastprom:SOT-23-6",
       "Dual-line USB ESD protection")

    # A power flag, so ERC can see that a rail downstream of a passive part
    # (the ferrite bead) is intentionally powered.
    # Named apart from the drawn sheets' PWR_FLAG (tools/schlib.py), which
    # is drawn differently: both land in the same library, and one name for
    # two bodies would make ERC report every flag as a library mismatch.
    syms["PWR_FLAG_K"] = (
        {"1": (0, 2.54, "T")},
        _symbol("PWR_FLAG_K", "PWR_FLAG", "", "Power flag for ERC",
                '      (polyline (pts (xy 0 0) (xy 0 1.27))\n'
                '        (stroke (width 0) (type default)) (fill (type none)))',
                [_pin("power_out", 0, 2.54, 270, "~", "1")]))

    # Two-position DIP switch: switch 1 joins pins 1 and 4, switch 2 joins 2 and 3.
    ic("SW_DIP_x02", [(1, "A1", "passive", "L"), (2, "B1", "passive", "L"),
                      (4, "A2", "passive", "R"), (3, "B2", "passive", "R")],
       "SW_DIP_x02", SW_DIP_FP, "2-position DIP switch")

    # J3, the SOICbite programming connector: eight edge pads, 1-4 on top
    # and 8-5 under them
    ic("SOICbite", [(i, f"P{i}", "passive", "L") for i in range(1, 9)],
       "SOICbite", "fastprom:SOICbite_2x04_P1.27mm_BoardEdge",
       "SOICbite edge pads for a SOIC-8 clip or pogo fixture")

    return syms


def _pin(etype, x, y, angle, name, number):
    return (f'      (pin {etype} line (at {x} {y} {angle}) (length {PITCH})\n'
            f'        (name "{name}" (effects (font (size 1.27 1.27))))\n'
            f'        (number "{number}" (effects (font (size 1.27 1.27)))))')


def _symbol(name, value, footprint, desc, body, plines):
    ref = "U"
    if name == "R":
        ref = "R"
    elif name == "C":
        ref = "C"
    elif name == "FerriteBead":
        ref = "FB"
    elif name.startswith("CONN") or name == "DIP20_PLUG":
        ref = "J"
    return (
        f'  (symbol "{name}"\n'
        f'    (pin_numbers (hide no)) (pin_names (offset 0.254))\n'
        f'    (exclude_from_sim no) (in_bom yes) (on_board yes)\n'
        f'    (property "Reference" "{ref}" (at 0 0 0) (effects (font (size 1.27 1.27))))\n'
        f'    (property "Value" "{value}" (at 0 0 0) (effects (font (size 1.27 1.27))))\n'
        f'    (property "Footprint" "{footprint}" (at 0 0 0) (effects (font (size 1.27 1.27)) (hide yes)))\n'
        f'    (property "Datasheet" "" (at 0 0 0) (effects (font (size 1.27 1.27)) (hide yes)))\n'
        f'    (property "Description" "{desc}" (at 0 0 0) (effects (font (size 1.27 1.27)) (hide yes)))\n'
        f'    (symbol "{name}_0_1"\n{body}\n    )\n'
        f'    (symbol "{name}_1_1"\n' + "\n".join(plines) + '\n    )\n'
        f'  )\n'
    )


class Sch:
    def __init__(self, syms, sheet_uuid, project):
        self.syms = syms
        self.sheet = sheet_uuid
        self.project = project
        self.items = []
        self.used_refs = {}
        self.net_pins = {}
        # Every point a wire or label occupies, so a collision between two
        # different nets is caught here rather than by ERC (or worse, not at
        # all). Two stubs landing on the same coordinate silently shorts two
        # nets together.
        self.occupied = {}
        self.collisions = []

    def place(self, sym_name, ref, value, at, nets, footprint=None, dnp=False):
        """nets maps pin number -> net name, or None for an explicit no-connect."""
        geom, _ = self.syms[sym_name]
        # Snap placements to KiCad's 1.27 mm schematic grid. Symbol-local pin
        # coordinates are already grid multiples, so snapping the instance
        # origin puts every pin, wire end and label on grid.
        X, Y = (round(v / 1.27) * 1.27 for v in at)
        props = [
            f'    (property "Reference" "{ref}" (at {X + 8} {Y - 10} 0) '
            f'(effects (font (size 1.27 1.27)) (justify left)))',
            f'    (property "Value" "{value}" (at {X + 8} {Y - 7.5} 0) '
            f'(effects (font (size 1.27 1.27)) (justify left)))',
        ]
        if footprint:
            props.append(
                f'    (property "Footprint" "{footprint}" (at {X} {Y} 0) '
                f'(effects (font (size 1.27 1.27)) (hide yes)))')
        pin_uuids = "\n".join(f'    (pin "{p}" (uuid "{uid()}"))' for p in geom)

        self.items.append(
            f'  (symbol (lib_id "fastprom:{sym_name}") (at {X} {Y} 0) (unit 1)\n'
            f'    (exclude_from_sim no) (in_bom yes) (on_board yes) '
            f'(dnp {"yes" if dnp else "no"})\n'
            f'    (uuid "{uid()}")\n' + "\n".join(props) + "\n" + pin_uuids + "\n"
            f'    (instances (project "{self.project}" '
            f'(path "/{self.sheet}" (reference "{ref}") (unit 1))))\n'
            f'  )\n')

        for pnum, (px, py, side) in geom.items():
            gx, gy = X + px, Y - py
            net = nets.get(pnum, "UNSPEC")
            if net is None:
                self.items.append(
                    f'  (no_connect (at {gx} {gy}) (uuid "{uid()}"))\n')
                continue
            if net == "UNSPEC":
                self.items.append(
                    f'  (no_connect (at {gx} {gy}) (uuid "{uid()}"))\n')
                continue

            if side == "L":
                ex, ey, rot, just = gx - STUB, gy, 180, "right"
            elif side == "R":
                ex, ey, rot, just = gx + STUB, gy, 0, "left"
            elif side == "T":
                ex, ey, rot, just = gx, gy - STUB, 90, "left"
            else:
                ex, ey, rot, just = gx, gy + STUB, 270, "right"

            for pt in ((round(gx, 2), round(gy, 2)), (round(ex, 2), round(ey, 2))):
                prev = self.occupied.get(pt)
                if prev is not None and prev != net:
                    self.collisions.append((pt, prev, net, f"{ref}.{pnum}"))
                self.occupied[pt] = net

            self.items.append(
                f'  (wire (pts (xy {gx} {gy}) (xy {ex} {ey}))\n'
                f'    (stroke (width 0) (type default)) (uuid "{uid()}"))\n')
            self.items.append(
                f'  (global_label "{net}" (shape bidirectional) (at {ex} {ey} {rot})\n'
                f'    (effects (font (size 1.27 1.27)) (justify {just})) '
                f'(uuid "{uid()}"))\n')
            self.net_pins.setdefault(net, []).append(f"{ref}.{pnum}")

    def render(self):
        # In a schematic's embedded lib_symbols the outer symbol name must be
        # library-qualified ("fastprom:R"), while the .kicad_sym file itself
        # uses the bare name. Getting this wrong makes KiCad fail to resolve
        # the symbol, which shows up as every pin dangling rather than as a
        # missing-symbol error.
        libs = ""
        for name, (_g, body) in self.syms.items():
            libs += body.replace(f'(symbol "{name}"', f'(symbol "fastprom:{name}"', 1)
        return (
            "(kicad_sch\n"
            "  (version 20250114)\n"
            '  (generator "fastprom")\n'
            '  (generator_version "10.0")\n'
            f'  (uuid "{self.sheet}")\n'
            '  (paper "A2")\n'
            f"  (lib_symbols\n{libs}  )\n"
            + "".join(self.items) +
            ")\n")


# ---------------------------------------------------------------------------
# The design
# ---------------------------------------------------------------------------

# The stack's upper board mounts its USB-C receptacle at the board end with
# the opening past the edge: the project's copy of the footprint has its
# silkscreen stop at that edge (hw/lib/fastprom.pretty/README.md).
USB_C_FP_STACK = "fastprom:USB_C_Receptacle_HRO_TYPE-C-31-M-12_BoardEdge"


# ---- socket signal names ---------------------------------------------------
# The socket's pins have fixed directions (boardmap.yaml's input and output
# groups), and the board's primary part fixes what each one is, so the nets
# are named by function rather than by pin number: each socket signal is
# named for what it carries in the first part the board supports (512x8),
# "A0" on the socket side and "BUF_A0" for its buffered 3.3 V line. The
# other supported organisations differ only on a few pins; pin_table()
# says which, on the sheets.

def _partdb():
    with open(os.path.join(ROOT, "tools", "partdb.yaml")) as fh:
        return yaml.safe_load(fh)["parts"]


def partdb_part(key):
    """One part's entry in tools/partdb.yaml."""
    return _partdb()[key]


def socket_names(board):
    """Socket pin -> its signal in the board's primary part, for every pin
    that is not power or ground."""
    pins = _partdb()[board["supports"][0]]["socket_pins"]
    skip = (board["power"]["vcc"], board["power"]["gnd"])
    return {int(p): s for p, s in pins.items() if int(p) not in skip}


def skt_net(board, pin):
    """The socket-side (5 V) net of a socket pin."""
    return socket_names(board)[pin]


def buf_net(board, pin):
    """The buffered (3.3 V) net of a socket pin, at the stack connector."""
    return "BUF_" + socket_names(board)[pin]


def dout_net(board, pin):
    """A data pin's net between the drive buffer and its series resistor."""
    return "DOUT_" + socket_names(board)[pin]


def pin_table(board, prefix=""):
    """Rows for the socket-pin table, from tools/partdb.yaml: a header, then
    each pin whose signal differs between the parts the board supports, with
    its net (prefix + the primary part's name) and its signal on each part.
    Pins that mean the same on every part need no row: their nets say it."""
    parts = _partdb()
    names = socket_names(board)
    sup = board["supports"]
    rows = [["Pin", "Net"] + list(sup)]
    for p in sorted(names):
        sigs = [str(parts[k]["socket_pins"][p]) for k in sup]
        if len(set(sigs)) > 1:
            rows.append([str(p), prefix + names[p]] + sigs)
    return rows


def stack_contacts(board):
    """Contact number -> net for the board-to-board connector.

    Both halves of the stack are generated from this one function, so their
    pinouts agree by construction. A mismatch here would be invisible in both
    schematics and fatal in hardware, which is exactly the kind of thing not
    to maintain by hand in two places.

    30 contacts: 18 buffered socket signals, the drive buffer's enable, two
    5 V and nine grounds, with both ends of each row taken by a signal or
    supply and every signal within three positions of a ground in its own
    row.
    """
    # The order is the lower board's copper (tools/gen_layout.py), which is
    # the tight one. The header lies along the socket channel, turned so
    # its odd row faces the DIP's right column (pins 11-20) and contacts 1
    # and 2 sit at the pin 1 end. Position k along the connector (0 at that
    # end) is contact 2k+1 in the right row and 2k+2 in the left.
    #
    # Pin 1 end, k 0-8, the receive lines:
    #   left row  - U3 (socket 15-19), on top just above the header, enters
    #               these pads from outside, in the order its outputs leave
    #               it (B1 first); the grounds between take vias beside
    #               the row.
    #   right row - socket power first (nearest socket pin 20), then U2
    #               (socket 1-5), which is underneath, through a via each
    #               in the lane (see fans.py).
    # Far end, k 9-14, the data lines: U4, the drive buffer, lies across
    # the channel just below the header with its B side facing it, and its
    # nine lines run straight up into this end - its first two and last
    # two outside the rows, the five between them up the lane.
    vcc, gnd = board["power"]["vcc"], board["power"]["gnd"]
    left = [p for p in range(1, 11) if p not in (vcc, gnd)]
    right = [p for p in range(20, 10, -1) if p not in (vcc, gnd)]
    if len(left) + len(right) != 18:
        raise SystemExit("expected 18 socket signals")
    # The buffers sit on the lower board, so the connector carries their
    # 3.3 V side: one line per socket signal, named for its function.
    S = lambda p: buf_net(board, p)
    rx_a, data_a = left[:5], left[5:]           # 1-5, 6-9
    data_b, rx_b = right[4:][::-1], right[:5][::-1]     # 11-14, 15-19
    G, V = "GND", "+5V_SKT"
    # The ground once at contact 17 (right row, k 8, between the left
    # row's grounds at 16, 18 and 20) is at 25 (k 12), beside the one at
    # 23: the data end had too few, and contact 29 (socket 12) was three
    # from a ground in its row; now two. Socket 14 takes contact 17: on the
    # lower board its line, already outermost on the right, runs on up to
    # k 8, and the two grounds share one via in the lane (fans.py).
    odd = [V, V, S(rx_a[0]), S(rx_a[4]), G, S(rx_a[3]), S(rx_a[1]),
           S(rx_a[2]), S(data_b[3]), S(data_a[3]), S(data_b[0]), G, G,
           S(data_b[2]), S(data_b[1])]
    even = [S(rx_b[0]), S(rx_b[1]), G, S(rx_b[2]), S(rx_b[3]), G,
            S(rx_b[4]), G, G, G, S(data_a[2]), S(data_a[1]), G,
            "D_BUF_NOE", S(data_a[0])]
    contacts = {}
    for t, (o, e) in enumerate(zip(odd, even)):
        contacts[2 * t + 1] = o
        contacts[2 * t + 2] = e

    assert len(contacts) == 30, len(contacts)
    assert sorted(n for n in contacts.values() if n.startswith("BUF_")) == \
        sorted(S(p) for p in left + right)
    return contacts


def build_design(board, syms, project):
    sch = Sch(syms, uid(f"sheet-root-{project}"), project)

    # Two-pin parts and small connectors go into a regular grid so their
    # stubs cannot collide. Cells are wide enough that a 5.08 mm stub plus a
    # label never reaches a neighbour.
    cell_w, cell_h = 38.1, 43.18
    grid_x0, grid_y0, per_row = 40.0, 300.0, 14
    slot = [0]

    def next_slot():
        i = slot[0]
        slot[0] += 1
        return (grid_x0 + (i % per_row) * cell_w,
                grid_y0 + (i // per_row) * cell_h)

    # Socket-side nets are named for what the pin carries in the board's
    # primary part (socket_names(), above). Which part is emulated is decided
    # in the generated fabric constraints, not in copper; pin_table() puts
    # the other organisations' differences on the drawn sheets.
    s2m = {int(k): v for k, v in board["socket_to_mcu"].items()}
    ins = board["input_group"]
    outs = board["output_group"]
    vcc_pin = board["power"]["vcc"]
    gnd_pin = board["power"]["gnd"]

    # ---- the plug into the host socket ------------------------------------
    # Which half of the stack this board is.
    role = board.get("role")
    if role not in ("stack_lower", "stack_upper"):
        raise SystemExit(f"unknown board role {role!r}")
    lower = role == "stack_lower"

    # The socket's 5 V must carry the same name the connector's power
    # contacts use, or the lower board never delivers it. That was a real
    # defect: the lower board had J1 pin 20 on "+5V" while stack_contacts()
    # put "+5V_SKT" on the contacts, so socket power simply did not cross
    # the connector - and both halves passed ERC.
    skt_5v = "+5V_SKT"
    plug = {}
    for n in range(1, 21):
        if n == vcc_pin:
            plug[n] = skt_5v
        elif n == gnd_pin:
            plug[n] = "GND"
        else:
            plug[n] = skt_net(board, n)
    if lower:
        # two machined-pin strips with the DF40 header in the channel
        # between them: see hw/lib/fastprom.pretty/README.md
        sch.place("DIP20_PLUG", "J1", "DIP20_PLUG", (60, 90), plug,
                  "fastprom:DIP-20_W7.62mm_PreciDip350_2xSIP")

    # ---- the board-to-board connector -------------------------------------
    contacts = stack_contacts(board)
    if lower:
        sch.place("DF40C-30DP", "J5", "DF40C-30DP-0.4V", (200, 90),
                  contacts,
                  "fastprom:Hirose_DF40C-30DP-0.4V_2x15-1MP_P0.4mm")
    else:
        # 3.0 mm stack rather than the 1.5 mm default: the DIP pin tails
        # solder on the TOP of the lower board, so its top surface carries
        # 20 solder fillets in the pad rows - and the upper board overhangs
        # those rows, which is the whole point of stacking. Stack height
        # therefore has a floor, which is why the sub-millimetre parts
        # (BM28 at 0.6 mm, DF30 at 0.9 mm) are unusable here despite being
        # physically smaller. 3.0 rather than 2.0 because the pin strips
        # ship loose and are soldered by the customer, and because LCSC
        # stocks this height (the 2.0 mm socket is not) - same land pattern.
        sch.place("DF40C-30DS", "J6", "DF40HC(3.0)-30DS-0.4V", (60, 90),
                  contacts,
                  "fastprom:Hirose_DF40HC(3.0)-30DS-0.4V_2x15_P0.4mm")

    # On the upper half of a stack, ground and the socket's 5 V arrive through
    # the board-to-board connector, whose contacts are passive - so nothing on
    # this board declares them as sources and ERC would call them undriven.
    if role == "stack_upper":
        sch.place("PWR_FLAG_K", "#FLG3", "PWR_FLAG", next_slot(), {"1": "GND"})
        sch.place("PWR_FLAG_K", "#FLG4", "PWR_FLAG", next_slot(),
                  {"1": "+5V_SKT"})

    # ---- receive buffers, socket -> AG32 ----------------------------------
    # U2 takes the eight address-ish pins, U3 the two enable-ish pins. Both
    # are permanently enabled and permanently A->B; their outputs face the
    # AG32 only, so nothing on the host side ever sees them float.
    # Split in socket pin order, not by function. U2 and U3 are identical
    # receive buffers, so which one carries a given input is arbitrary -
    # and taking them in order makes the bundle from the connector monotonic,
    # which is what lets it route without crossings. Splitting 1-5,17-19 into
    # one buffer and 15,16 into the other forces three lines to travel the
    # full height of the connector and cross everything in between.
    # Five and five: the socket's inputs are two contiguous groups (1-5 and
    # 15-19) either side of its data pins, so one receive buffer per group
    # lets both input buses and the drive bus between them run as straight,
    # separate bundles. Any split that sends one group's lines to the other
    # group's buffer makes them cross the drive section. The buffers are
    # identical, so this is purely a drawing and routing choice.
    rx1 = ins[:5]                    # 1,2,3,4,5
    rx2 = ins[5:]                    # 15,16,17,18,19
    assert len(rx1) == 5 and len(rx2) == 5

    if lower:
        # Partial address decode is common in the field: high address pins get
        # tied low or left floating. Pull-downs give a floating pin a defined
        # value; the enable gets a pull-up so a floating enable reads as
        # deselected rather than accidentally enabling the part.
        for i, skt in enumerate(ins[7:]):
            sch.place("R", f"R{12 + i}", "100k", next_slot(),
                      {"1": skt_net(board, skt), "2": "GND"},
                      "fastprom:R_0402_1005Metric")
        # Pulls on the socket side reference the host's own 5 V rail, not the
        # module's internal one, which may be fed from USB instead.
        sch.place("R", "R15", "100k", next_slot(),
                  {"1": skt_5v, "2": skt_net(board, ins[5])},
                  "fastprom:R_0402_1005Metric")
        # Pin 16 is the one pin whose meaning changes between organisations: A5
        # on the 512x8, a second active-low enable on the 256x8. An address bit
        # wants a pull-down so a floating or grounded pin reads as zero; an enable
        # wants a pull-up so a floating pin reads as deselected. Both pads are
        # laid out and exactly one is populated.
        sch.place("R", "R16", "100k", next_slot(),
                  {"1": skt_net(board, ins[6]), "2": "GND"},
                  "fastprom:R_0402_1005Metric")
        sch.place("R", "R20", "100k_DNP_256x8", next_slot(),
                  {"1": skt_5v, "2": skt_net(board, ins[6])},
                  "fastprom:R_0402_1005Metric", dnp=True)

    # ---- the bus interface: buffers and terminators ----------------------
    # These sit on the lower board, next to the socket, and run from their own regulator fed by the socket's 5 V (U7): with the host
    # switched off and the module on USB they are unpowered and their ports
    # high-impedance (Ioff), so nothing back-feeds a dead host's bus. The
    # connector then carries only 3.3 V logic, and the upper board only the
    # AG32 side.
    if lower:
        bus = "+3V3_BUF"
        n_u2 = {21: "GND", 1: bus, 19: "GND", 20: bus, 10: "GND"}
        for i, skt in enumerate(rx1):
            n_u2[2 + i] = skt_net(board, skt)      # A1..A8 on pins 2..9
            n_u2[18 - i] = buf_net(board, skt)     # B1..B8 on pins 18..11
        for i in range(len(rx1), 8):
            n_u2[2 + i] = "GND"    # unused inputs must not float
            n_u2[18 - i] = None    # unused outputs: explicit no-connect
        sch.place("SN74LVT245B_AB", "U2", "SN74LVT245B", (140, 60), n_u2,
                  "fastprom:Texas_RGY_R-PVQFN-N20_EP2.05x3.05mm")

        n_u3 = {21: "GND", 1: bus, 19: "GND", 20: bus, 10: "GND"}
        for i, skt in enumerate(rx2):
            n_u3[2 + i] = skt_net(board, skt)
            n_u3[18 - i] = buf_net(board, skt)
        for i in range(len(rx2), 8):
            n_u3[2 + i] = "GND"    # unused inputs must not float
            n_u3[18 - i] = None    # unused outputs: explicit no-connect
        sch.place("SN74LVT245B_AB", "U3", "SN74LVT245B", (140, 130), n_u3,
                  "fastprom:Texas_RGY_R-PVQFN-N20_EP2.05x3.05mm")

        # ---- drive buffer, AG32 -> socket -------------------------------------
        # DIR low means B->A, so the AG32 drives B and the socket sees A. The
        # output enable comes from the fabric and is active low.
        n_u4 = {21: "GND", 1: "GND", 19: "D_BUF_NOE", 20: bus, 10: "GND"}
        for i, skt in enumerate(outs):
            n_u4[2 + i] = dout_net(board, skt)     # A1..A8 -> series resistor -> socket
            n_u4[18 - i] = buf_net(board, skt)     # B1..B8 <- AG32
        sch.place("SN74LVT245B_BA", "U4", "SN74LVT245B", (140, 200), n_u4,
                  "fastprom:Texas_RGY_R-PVQFN-N20_EP2.05x3.05mm")

        # ---- series termination on the data lines -----------------------------
        # Two 4 x 0402 arrays rather than eight resistors: under the
        # board, between the pin strips, is a 4.6 mm channel that also has
        # to hold U3, the pulls and the stack connector's via field.
        # RN1 takes socket pins 6-9 (U4 A1-A4), RN2 pins 11-14 (A5-A8);
        # element k joins pin k (socket side) to pin 9 - k (U4 side).
        packs = {"RN1": {}, "RN2": {}}
        for i, skt in enumerate(outs):
            k = i % 4 + 1
            pins = packs["RN1" if i < 4 else "RN2"]
            pins[k] = skt_net(board, skt)
            pins[9 - k] = dout_net(board, skt)
        for ref, pins in packs.items():
            sch.place("R_Pack04", ref, "4x33R", next_slot(), pins,
                      "fastprom:R_Array_Convex_4x0402")

        # U4's enable is active low and must be HIGH from power-on until the
        # fabric is configured and decides otherwise. It is pulled up to
        # U4's own supply, which TI requires for the outputs
        # to stay high-impedance through power-up (above 1.5 V the
        # power-up 3-state no longer holds and OE decides). With the host
        # off, the AG32 driving it high sources at most 0.33 mA into an
        # unpowered rail held near the dead host's 5 V by U7, which powers
        # nothing.
        sch.place("R", "R21", "10k_OE_PULLUP", next_slot(),
                  {"1": bus, "2": "D_BUF_NOE"},
                  "fastprom:R_0402_1005Metric")
        # one 100 nF per receiver; the driver (U4), which switches all eight
        # data lines into the host at once, gets 1 uF - about 1-1.5 nC per
        # edge, far more than 100 nF holds up across U4's long supply run
        for ref in ("C8", "C9", "C10"):
            sch.place("C", ref, "1uF/25V" if ref == "C10" else "100nF/16V",
                      next_slot(), {"1": bus, "2": "GND"},
                      "fastprom:C_0402_1005Metric")
        # 100 nF right at socket pin 20, so the data lines' return can
        # close through the host's own socket decoupling rather than
        # through 25 mm of track to the nearest capacitor (C12 at U7)
        sch.place("C", "C18", "100nF/16V", next_slot(),
                  {"1": "+5V_SKT", "2": "GND"},
                  "fastprom:C_0402_1005Metric")
        # Started about 2 ms after the socket's 5 V (R26/C17, tau 10 ms,
        # EN threshold 0.9 V), so the AG32's rail - behind D2 and with
        # some 20 uF to charge - is up before the buffers are. U4's
        # enable is pulled up to the buffers' own rail (R21, above), as
        # TI asks for its power-up high-impedance guarantee; the AG32
        # leading means its unpowered I/O never drag that line down.
        sch.place("TLV74233", "U7", "TLV74233", next_slot(),
                  {4: "+5V_SKT", 3: "BUF_EN", 2: "GND",
                   1: "+3V3_BUF"}, "fastprom:Texas_X2SON-4_1x1mm_P0.65mm_NoEP")
        sch.place("R", "R26", "100k", next_slot(),
                  {"1": "+5V_SKT", "2": "BUF_EN"},
                  "fastprom:R_0402_1005Metric")
        sch.place("C", "C17", "100nF/16V", next_slot(),
                  {"1": "BUF_EN", "2": "GND"},
                  "fastprom:C_0402_1005Metric")
        sch.place("C", "C12", "1uF/25V", next_slot(),
                  {"1": "+5V_SKT", "2": "GND"}, "fastprom:C_0402_1005Metric")
        sch.place("C", "C13", "1uF/25V", next_slot(),
                  {"1": "+3V3_BUF", "2": "GND"}, "fastprom:C_0402_1005Metric")
        # A test pad for the buffers' supply, reachable in the host with the
        # upper board lifted off.
        sch.place("TestPoint", "TP1", "TestPoint", next_slot(), {1: "+3V3_BUF"},
                  "fastprom:TestPoint_Pad_D1.0mm")

    if lower:
        return sch

    # ---- the AG32 ---------------------------------------------------------
    pkg = AG32_PACKAGES[board["package"]]
    mcu = {}
    for n, name in pkg["pins"].items():
        if name.startswith("VDD"):
            mcu[n] = "+3V3_MCU"
        elif name in ("GND", "EP"):
            mcu[n] = "GND"
        elif name == "NRST":
            mcu[n] = "NRST"
        elif name == "BOOT0":
            mcu[n] = "BOOT0"
        else:
            mcu[n] = None      # no-connect unless claimed below

    for skt, mcu_name in s2m.items():
        mcu[int(mcu_name.split("_")[1])] = buf_net(board, skt)

    ctrl = board["control"]
    for key, net in (("data_oe", "D_BUF_NOE"), ("hse_in", "HSE_IN")):
        if key in ctrl:
            mcu[int(ctrl[key].split("_")[1])] = net

    # Fixed-function pins on the QFN-32. Programmed over UART0 only: AGM's
    # serial loader writes the fabric bitstream as well as the firmware, so
    # the JTAG pair (PIN_24 JTMS, PIN_25 JTCK) is not brought out and the
    # pins are left to their internal pulls.
    fixed = {20: "UART_TX", 21: "UART_RX", 15: "BOOT1"}
    # BOOT1 is only sampled at reset. A board may put a data line on it (the
    # QFN-32 has no I/O to spare); the strap then pulls that line down, which
    # a buffer input does not mind, and BOOT1 still reads 0 for the loader.
    boot1_net = "BOOT1"
    for k, v in list(fixed.items()):
        if v == "BOOT1" and mcu.get(k):
            boot1_net = mcu[k]
            del fixed[k]
    mcu.update(fixed)

    # The USB PHY is a hard block and is not fabric-routable - it is the
    # one signal pair on this board that cannot be moved.
    mcu[22] = "USB_DM"
    mcu[23] = "USB_DP"

    sch.place(pkg["symbol"], "U1", pkg["symbol"], (300, 120), mcu,
              pkg["footprint"])

    # ---- USB --------------------------------------------------------------
    sch.place("USB_C_Receptacle", "J4", "USB_C_USB2.0", next_slot(),
              {"A1": "GND", "A12": "GND", "B1": "GND", "B12": "GND",
               "A4": "+5V_USB", "A9": "+5V_USB",
               "B4": "+5V_USB", "B9": "+5V_USB",
               "A5": "USB_CC1", "B5": "USB_CC2",
               "A6": "USB_DP_CON", "B6": "USB_DP_CON",
               "A7": "USB_DM_CON", "B7": "USB_DM_CON",
               "A8": None, "B8": None,
               "SH": "GND"},
              USB_C_FP_STACK)

    # 5.1k on each CC line is what tells a source this is a device.
    sch.place("R", "R22", "5k1", next_slot(),
              {"1": "USB_CC1", "2": "GND"},
              "fastprom:R_0402_1005Metric")
    sch.place("R", "R23", "5k1", next_slot(),
              {"1": "USB_CC2", "2": "GND"},
              "fastprom:R_0402_1005Metric")

    # I/O1 appears on pins 1 and 6 and I/O2 on pins 3 and 4, internally
    # the same node each, so each line routes in one pin and out the other,
    # in either direction. With the clamp under the connector, these pairs
    # put its AG32 side facing the QFN.
    esd = {1: "USB_DM", 6: "USB_DM_CON", 3: "USB_DP", 4: "USB_DP_CON"}
    sch.place("USBLC6-2SC6", "U6", "USBLC6-2SC6", next_slot(),
              {**esd, 2: "GND", 5: "+5V_USB"},
              "fastprom:SOT-23-6")
    # ST asks for 100 nF on the clamp's VBUS pin. And a cable's ~1 uH into
    # the ceramic behind D3 would ring a hot-plug to 7-8 V, past the
    # AP2112K's 6.5 V absolute maximum: R27 + C20 damp it (optimum about
    # sqrt(1 uH / 9.4 uF) = 0.33 R; 1 R errs on the overdamped side), and
    # keep VBUS within USB's 10 uF.
    sch.place("C", "C19", "100nF/16V", next_slot(),
              {"1": "+5V_USB", "2": "GND"},
              "fastprom:C_0402_1005Metric")
    sch.place("R", "R27", "1R", next_slot(),
              {"1": "+5V_USB", "2": "VBUS_DAMP"},
              "fastprom:R_0402_1005Metric")
    sch.place("C", "C20", "4.7uF/16V", next_slot(),
              {"1": "VBUS_DAMP", "2": "GND"},
              "fastprom:C_0603_1608Metric")

    # Two sources of 5 V, OR-ed with Schottkys so the module runs from the
    # socket when seated and from USB on the bench. The socket-side diode
    # is the one that matters: without it, plugging USB in while the module
    # is seated would push 5 V back out of socket pin 20 into the host's
    # rail.
    sch.place("D_Schottky", "D2", "Schottky", next_slot(),
              {"2": "+5V_SKT", "1": "+5V"}, "fastprom:D_SOD-323")
    sch.place("D_Schottky", "D3", "Schottky", next_slot(),
              {"2": "+5V_USB", "1": "+5V"}, "fastprom:D_SOD-323")
    sch.place("PWR_FLAG_K", "#FLG2", "PWR_FLAG", next_slot(), {"1": "+5V"})

    # ---- power ------------------------------------------------------------
    sch.place("AP2112K-3.3", "U5", "AP2112K-3.3", (60, 230),
              {1: "+5V", 2: "GND", 3: "+5V", 4: None, 5: "+3V3"},
              "fastprom:SOT-23-5")

    # 0603, not 0402: a 4.7 uF 0402 at 6.3 V delivers well under half its
    # marked value at 3.3 V bias, and the 16 V 0402 part is not in stock.
    sch.place("C", "C1", "4.7uF/16V", next_slot(), {"1": "+5V", "2": "GND"},
              "fastprom:C_0603_1608Metric")
    sch.place("C", "C2", "10uF/10V", next_slot(),
              {"1": "+3V3", "2": "GND"}, "fastprom:C_0603_1608Metric")

    # AGM asks for the fabric's 3.3 V rail on its own ferrite-isolated branch.
    sch.place("FerriteBead", "FB1", "220R@100MHz/2.2A", next_slot(),
              {"1": "+3V3", "2": "+3V3_MCU"},
              "fastprom:L_0603_1608Metric")
    sch.place("PWR_FLAG_K", "#FLG1", "PWR_FLAG", next_slot(), {"1": "+3V3_MCU"})
    # Test pads for USB's 5 V and the AG32's own rail (J3 already has 3V3).
    sch.place("TestPoint", "TP2", "TestPoint", next_slot(), {1: "+5V_USB"},
              "fastprom:TestPoint_Pad_D1.0mm")
    sch.place("TestPoint", "TP3", "TestPoint", next_slot(), {1: "+3V3_MCU"},
              "fastprom:TestPoint_Pad_D1.0mm")

    # Decoupling: one per AG32 supply pin (the buffers' are with them).
    # The QFN-32 has three supply pins (6 VDDA, 16, 32), one cap each
    # beside the pin, plus bulk.
    for ref in ("C3", "C4", "C5"):
        sch.place("C", ref, "100nF/16V", next_slot(),
                  {"1": "+3V3_MCU", "2": "GND"},
                  "fastprom:C_0402_1005Metric")
    # Bulk behind the ferrite, and 1 uF at VDDA, which also feeds the PLL.
    sch.place("C", "C14", "10uF/10V", next_slot(),
              {"1": "+3V3_MCU", "2": "GND"},
              "fastprom:C_0603_1608Metric")
    sch.place("C", "C15", "1uF/25V", next_slot(),
              {"1": "+3V3_MCU", "2": "GND"},
              "fastprom:C_0402_1005Metric")
    if "hse_in" in ctrl:
        # The USB clock: 12 MHz into OSC_IN, the PLL multiplies it to 48 MHz.
        sch.place("OSC_4PIN", "X1", "12MHz", next_slot(),
                  {1: "+3V3_MCU", 2: "GND", 3: "HSE_IN", 4: "+3V3_MCU"},
                  "fastprom:Oscillator_SMD_SiT_PQFN-4Pin_2.0x1.6mm")
        sch.place("C", "C16", "100nF/16V", next_slot(),
                  {"1": "+3V3_MCU", "2": "GND"},
                  "fastprom:C_0402_1005Metric")

    sch.place("C", "C11", "100nF/16V", next_slot(), {"1": "NRST", "2": "GND"},
              "fastprom:C_0402_1005Metric")

    # ---- straps and pulls -------------------------------------------------
    sch.place("R", "R9", "10k", next_slot(), {"1": "+3V3_MCU", "2": "NRST"},
              "fastprom:R_0402_1005Metric")

    # (The data buffer's enable pull-up, R21, is on the lower board, at the
    # buffer it enables.)
    sch.place("R", "R10", "10k", next_slot(), {"1": "BOOT0", "2": "GND"},
              "fastprom:R_0402_1005Metric")
    # 4.7k, not 10k: against the pin's internal 30-50k pull-up a 10k strap
    # sits at 0.55-0.83 V, on the 0.8 V VIL limit; 4.7k gives 0.28-0.45 V.
    # (DESIGN_NOTES.md)
    sch.place("R", "R11", "4k7", next_slot(), {"1": boot1_net, "2": "GND"},
              "fastprom:R_0402_1005Metric")

    # ---- image select ----------------------------------------------------
    if ctrl.get("bank_select") != "uart_dip":
        raise SystemExit("image select: only bank_select: uart_dip is drawn")
    # The two bank bits share the UART0 pins, which are ordinary fabric
    # I/O unless BOOT0 is held high for the mask-ROM loader. Each pin has
    # a 100k pull-down (bit = 0); closing a switch adds 10k to 3V3 (bit =
    # 1). UART idles high, so a pull-up is invisible to the loader and to
    # an adapter whichever way the switches are set. Switch 1 (pins 1-4,
    # RX) is image bit 0 and switch 2 (pins 2-3, TX) bit 1, so the numbers
    # printed on the switch name the bits (the firmware's decode); the TX
    # strap on pins 2-3 and RX on 1-4 keeps the two runs from crossing.
    sch.place("R", "R17", "100k", next_slot(), {"1": "UART_TX", "2": "GND"},
              "fastprom:R_0402_1005Metric")
    sch.place("R", "R18", "100k", next_slot(), {"1": "UART_RX", "2": "GND"},
              "fastprom:R_0402_1005Metric")
    sch.place("R", "R24", "10k", next_slot(),
              {"1": "UART_TX", "2": "BANK0_SW"},
              "fastprom:R_0402_1005Metric")
    sch.place("R", "R25", "10k", next_slot(),
              {"1": "BANK1_SW", "2": "UART_RX"},
              "fastprom:R_0402_1005Metric")
    sch.place("SW_DIP_x02", "SW1", "Image_Select", next_slot(),
              {2: "BANK0_SW", 3: "+3V3", 1: "BANK1_SW", 4: "+3V3"},
              SW_DIP_FP)

    # ---- programming and recovery ----------------------------------------
    # One strip does it all: AGM's UART0 mask-ROM loader writes both the
    # firmware and the fabric bitstream, and works even with blank or
    # corrupt flash. The fixture holds BOOT0 high to select the loader,
    # and NRST lets it reset into it without a power cycle. SOICbite's
    # ESP8266 pinout: 3V3, boot strap, GND, RX, TX, reset; 3 and 7 unused.
    sch.place("SOICbite", "J3", "SOICbite", next_slot(),
              {1: "+3V3", 2: "BOOT0", 3: None, 4: "GND", 5: "UART_RX",
               6: "UART_TX", 7: None, 8: "NRST"},
              "fastprom:SOICbite_2x04_P1.27mm_BoardEdge")

    return sch


PROJECT_TEMPLATE = """{
  "board": {"3dviewports": [], "layer_presets": [], "viewports": [],
    "design_settings": {"rules": {
      "min_clearance": 0.127, "min_track_width": 0.1,
      "min_via_diameter": 0.45, "min_through_hole_diameter": 0.2,
      "min_via_annular_width": 0.1, "min_copper_edge_clearance": 0.3,
      "min_hole_clearance": 0.25, "min_hole_to_hole": 0.25,
      "min_microvia_diameter": 0.2, "min_microvia_drill": 0.1,
      "min_silk_clearance": 0.0, "min_text_height": 0.8,
      "min_text_thickness": 0.08, "solder_mask_to_copper_clearance": 0.0}}},
  "boards": [],
  "cvpcb": {"equivalence_files": []},
  "libraries": {"pinned_footprint_libs": [], "pinned_symbol_libs": []},
  "meta": {"filename": "fastprom.kicad_pro", "version": 3},
  "net_settings": {"classes": [{"name": "Default", "clearance": 0.127,
    "track_width": 0.15, "via_diameter": 0.45, "via_drill": 0.2}]},
  "pcbnew": {"page_layout_descr_file": ""},
  "schematic": {"legacy_lib_dir": "", "legacy_lib_list": []},
  "sheets": [["SHEETUUID", "Root"]],
  "text_variables": {}
}
"""

SYM_LIB_TABLE = """(sym_lib_table
  (version 7)
  (lib (name "fastprom")(type "KiCad")(uri "${KIPRJMOD}/lib/fastprom.kicad_sym")(options "")(descr "FastPROM symbols"))
)
"""

# The stock libraries the stack's parts come from, plus the three that
# symbols in hw/lib/fastprom.kicad_sym name as their default footprint
# (Package_DIP, Connector_USB, Connector_PinHeader_2.54mm).
FP_LIBS = ["Package_DFN_QFN",
           "Package_TO_SOT_SMD", "Package_DIP", "Connector_USB", "Diode_SMD",
           "Connector_Hirose_DF40",
           "Resistor_SMD", "Capacitor_SMD", "Inductor_SMD",
           "Connector_PinHeader_2.54mm", "Button_Switch_SMD", "Oscillator",
           "TestPoint"]

# Project-local footprint library, for the packages with no stock
# equivalent or a modified stock one (hw/lib/fastprom.pretty/README.md).
LOCAL_FP_LIB = "fastprom"


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--board", required=True,
                    help="fastprom-lower or fastprom-upper")
    args = ap.parse_args()

    with open(os.path.join(ROOT, "tools", "boardmap.yaml")) as fh:
        bm = yaml.safe_load(fh)
    board = bm["boards"][args.board]
    if board.get("role") != "stack_lower" and not board.get("socket_to_mcu"):
        raise SystemExit(f"board {args.board} has no pin mapping yet")

    syms = build_symbols()
    reset_uids()
    os.makedirs(os.path.join(HW, "lib"), exist_ok=True)
    os.makedirs(os.path.join(HW, "lib", "fastprom.pretty"), exist_ok=True)

    with open(os.path.join(HW, "lib", "fastprom.kicad_sym"), "w") as fh:
        fh.write("(kicad_symbol_lib\n  (version 20241209)\n"
                 '  (generator "fastprom")\n'
                 '  (generator_version "10.0")\n')
        for _g, s in syms.values():
            fh.write(s)
        fh.write(")\n")

    # One project per board, so the two halves of the stack coexist instead
    # of overwriting each other.
    name = args.board
    sch = build_design(board, syms, name)
    with open(os.path.join(HW, f"{name}.kicad_sch"), "w") as fh:
        fh.write(sch.render())

    with open(os.path.join(HW, f"{name}.kicad_pro"), "w") as fh:
        fh.write(PROJECT_TEMPLATE
                 .replace("SHEETUUID", sch.sheet)
                 .replace("fastprom.kicad_pro", f"{name}.kicad_pro"))

    with open(os.path.join(HW, "sym-lib-table"), "w") as fh:
        fh.write(SYM_LIB_TABLE)

    with open(os.path.join(HW, "fp-lib-table"), "w") as fh:
        fh.write("(fp_lib_table\n  (version 7)\n")
        for lib in FP_LIBS:
            fh.write(f'  (lib (name "{lib}")(type "KiCad")'
                     f'(uri "${{KICAD10_FOOTPRINT_DIR}}/{lib}.pretty")'
                     f'(options "")(descr ""))\n')
        fh.write(f'  (lib (name "{LOCAL_FP_LIB}")(type "KiCad")'
                 f'(uri "${{KIPRJMOD}}/lib/{LOCAL_FP_LIB}.pretty")'
                 f'(options "")(descr "Project footprints"))\n')
        fh.write(")\n")

    if sch.collisions:
        for pt, a, b, who in sch.collisions:
            print(f"error: {pt} carries both {a!r} and {b!r} (at {who}) - "
                  f"these nets would be shorted", flush=True)
        raise SystemExit(f"{len(sch.collisions)} net collision(s); "
                         f"fix placement before generating")

    singles = sorted(n for n, p in sch.net_pins.items() if len(p) < 2)
    print(f"board {args.board}: {len(syms)} symbols defined, "
          f"{len(sch.net_pins)} nets")
    if singles:
        print(f"  nets with only one pin: {', '.join(singles)}")
    print(f"  wrote hw/{name}.kicad_sch, hw/{name}.kicad_pro, "
          f"hw/lib/fastprom.kicad_sym, hw/sym-lib-table, hw/fp-lib-table")


if __name__ == "__main__":
    main()
