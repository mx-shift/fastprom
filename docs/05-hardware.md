# Boards

The hardware is a two-board stack: `fastprom-lower` plugs into the host
socket and carries the bus interface, and `fastprom-upper` sits
3 mm above it and carries the AG32. What goes on which board, the
board-to-board connector and the stack heights are in
`09-two-board-stack.md`; the reasons behind individual electrical and layout
choices are in `../DESIGN_NOTES.md`. This page covers the mechanical and
PCB-process side.

## Form factor

The module has to fit where the part it replaces fitted.

A 20-pin DIP is 0.3 inch (7.62 mm) between the two pin rows, 0.1 inch
(2.54 mm) pin pitch, so the pin field spans 7.62 x 22.86 mm between pin
centres. Both boards are **11.02 mm wide**: the pin field plus 1.70 mm past
each pin, and no wider, because these sockets are often on crowded boards
with neighbours a tenth of an inch away.

| Board | Outline | Why |
|---|---|---|
| `fastprom-lower` | 11.02 x 26.26 mm | the pin field plus 1.70 mm past each pin row end |
| `fastprom-upper` | 11.02 x 31.76 mm | the same, 1.5 mm longer at the USB end, so the USB-C receptacle's locating-peg holes clear the DF40 socket's courtyard underneath, and 4 mm longer at the pin 1 end, where the programming connector J3 sits at the edge with its clip's strip clear on both faces |

Nothing overhangs sideways. The upper board's extra length overhangs the
host about 10 mm up, 1.5 mm at the USB end and 4 mm at the pin 1 end,
above a neighbouring DIP rather than beside it. Each board's pin 1
corner is chamfered 1.0 mm at 45 degrees (the module's pin 1 mark,
"Pin 1" below); the lower board only loses that corner, so it stays
inside the socket footprint. The outlines are set in
`tools/gen_layout.py` (`BOARD_W`, `BOARD_L`, `USB_EXTRA`, `PROG_EXTRA`,
`CHAMFER`).
The reasons for the 4 mm are in `../DESIGN_NOTES.md`, "J3 at the pin 1
end".

Height is the stack's real cost: about 14 mm to the top of the USB-C
receptacle, against roughly 7.4 mm for a seated DIP package. The full height
table is in `09-two-board-stack.md`.

## Use round pins, not square header pins

The 20 downward pins are a real BOM line and a common way to get this wrong.

DIP sockets are made for round leads. Square-section header pins - the usual
0.1 inch breakaway strip - are wider across the diagonal than a DIP socket
contact expects, and they splay the contacts permanently. The socket then
makes poor contact with whatever is plugged into it next, including the
original PROM if someone puts it back.

So the pins are **round machined pins** on 0.1 inch pitch, in two rows 0.3
inch apart: two Preci-Dip 350-series 10-way strips (`J1`), whose tails come
up through the lower board and are soldered on top. They ship loose with the
assembled boards, and the tails must be trimmed before stacking.
`07-bom.md` carries the part number, the trim length and the searches that
offer the wrong part first.

## Layer stack

Both boards are **4 layers, 0.8 mm**, on JLCPCB's JLC04081H-3313
stack-up (recorded in both board files):

| Layer | Thickness | Use |
|---|---|---|
| F.Cu | 35 µm | parts and signals |
| prepreg 3313 | 0.1 mm | |
| In1.Cu | 15 µm | solid ground plane |
| core | 0.45 mm | |
| In2.Cu | 15 µm | solid ground plane |
| prepreg 3313 | 0.1 mm | |
| B.Cu | 35 µm | parts and signals |

All the routing is on the outer layers, each 0.1 mm over its own ground
plane: F over In1, B over In2. There are no supply planes; supplies are
0.2–0.3 mm tracks. A signal that changes layer changes plane, so each via
off ground has a ground via beside it wherever the copper allows (within
1 mm for most), and the two planes are stitched together along the edges
and round the connector, the AG32 and the buffers.

The outer layers carry no copper pour. Every ground pad has its own stub
and via to the planes; a pour breaks up into islands between the 0.1 mm bus
tracks and gives return currents a poorly stitched path beside the planes,
and F and B copper stays balanced without one.

Four layers even though the lower board's circuit is simple: the signals are
5 V TTL edges with a total delay budget in the twenties of nanoseconds, and
a solid ground plane under each signal layer is what keeps the return paths
short and the edges clean. The survey of real failures in these sockets is
explicit that **edge quality and enable turn-on matter as much as raw access
time**.

Design rules (`RULES` in `tools/gen_layout.py`): 0.1 mm track and space
against JLC's 0.09 mm multilayer minimum, 0.45 mm vias with 0.2 mm holes,
0.3 mm copper to the routed edge, 0.2 mm via hole to track. Why four layers
on both boards, why 0.1/0.1 and not something more relaxed, and why J1's
inner-layer pads are removed to keep the lower board's ground planes
whole, are in `../DESIGN_NOTES.md` ("PCB process").

## What the layout had to get right

In order, because they conflict with each other on boards this small:

1. **Ground plane continuity under the data path.** In1 and In2 are
   unbroken ground on both boards; J1's signal pins keep their copper rings
   off the inner layers so they cut small voids rather than a chain of
   large ones.
2. **Decoupling close to each supply pin.** On the upper board the AG32 is
   underneath and its capacitors are on top, one via away from the supply
   pins; on the lower board U4, which switches all eight data lines at
   once, has a 1 uF capacitor on a short loop underneath. Every capacitor's
   ground pad has a via to the planes within about 1 mm, C4's in the pad
   itself. The loop lengths are in `../DESIGN_NOTES.md`.
3. **The ferrite bead between the regulator and the AG32 rail.** AGM asks
   for the fabric's 3.3 V on its own branch. FB1 puts every AG32 supply pin
   and the oscillator behind the bead. The bus buffers are not on this rail
   at all: they have their own regulator on the lower board.
4. **Series termination at the drive buffer**, not at the socket pins, so
   the resistor damps the driver rather than the trace: the two 4 x 33 R
   arrays sit under U4 and the header's end, fed from vias under U4's A
   side.
5. **Short data runs.** The data path crosses the connector twice - AG32 to
   U4, U4 to socket pin - and every millimetre is delay the budget has
   already spent elsewhere. The timing budget carries the longest routed
   signal (`03-timing-budget.md`).

## How the layout is made

The layout is generated, not drawn by hand in the editor:

* `tools/gen_layout.py` holds the placement - each part's position, side
  and rotation, with the reasons in its comments - the design rules and the
  planes, and builds both boards from the exported netlists that
  `make grade` signs off;
* `tools/layout/fans.py` holds the hand-routed copper as checked data:
  the fan-outs into and out of the connectors, the buffers and the AG32,
  the lines that change sides, the supply runs and the ground;
* `hw/<board>.route.json` holds the finished copper: that, what
  Freerouting adds, and the stitching vias.

`make layout` rebuilds both boards byte for byte from those, with no
autorouter. `make reroute` routes them again with Freerouting, then
chamfers and straightens what it drew where DRC allows (DESIGN_NOTES.md,
"Routing quality") and adds the ground stitching (`gen_layout.py
stitch()`), and is needed only after a placement or netlist change.
`gen_layout.py --amend` changes the stored ground copper (fans.py's, at
named parts) without routing again, and moves any ground stitch the pin 1
chamfer cuts to the nearest place clear of it. `make layout-drc` runs KiCad's DRC
on both with schematic parity, and must report zero violations and zero
unconnected items; `make jlc` writes gerbers, drill files, BOM and CPL for
JLCPCB into `build/jlc/`. Both boards are DRC clean.

## Assembly

Both boards are double-sided. On the lower board U3, U4, the header J5 and
a few passives are on top in the channel between the pin columns; U2, the
regulator U7, the terminators and the pulls are underneath, between the
strips' insulators. On the upper board the AG32, its regulator and the DF40
socket are underneath; the switch, the programming pads, USB-C and most
passives are on top. Every fitted part is placed by JLC from LCSC stock
except the pin strips, which ship loose (`07-bom.md`).

**Pin 1.** One mark means pin 1 everywhere: a small filled triangle
(0.65 x 0.5 mm printed) just outside pad 1, pointing at it. Every part
with a pin 1 carries it, on whichever face the part sits (the list is in
`DESIGN_NOTES.md`, "Silkscreen marking"); on the lower board J1's
triangle marks the socket's pin 1, beside it, where the pin strips go
in. The module's own pin 1 is marked the way a DIP's is, by its shape:
both boards have the corner at socket pin 1 cut off at 45 degrees,
1.0 mm along each edge (the upper board's at its J3 end, left as drawn,
over socket pin 1's column; the lower board's beside J1 pin 1).

With the module stacked, the upper board's cut corner is the one you
see: put it at the socket's pin 1, the end with the socket's own notch.
The module plugs in either way round, and reversed it puts 5 V and ground on
the wrong pins. Each board also carries its name (`FastPROM rev A UPPER` /
`LOWER`) and `© 2026 mxshift`; the upper board's name is on top as well,
the only face that shows with the module seated. The upper board's top
also labels the image-select switch SW1 `IMG` (the switch prints its own `ON` and its
numbers: switch 1 is image bit 0, switch 2 bit 1), and labels the
programming connector J3 `PROG` (its pin 1, marked by its triangle, is
the left pad on top, at the board's end; pinout in
`06-build-and-flash.md`).

**Test pads.** Three bare 1 mm pads, `TP1` on the lower board and `TP2`,
`TP3` on the upper, for measuring the supplies at bring-up. They are not
assembled: JLC places nothing on them (`07-bom.md`). Each is labelled with
its rail.

| Board | Pad | Rail | Label | Where | Reach it |
|---|---|---|---|---|---|
| upper | TP2 | `+5V_USB` | `5VU` | top, between the OR-ing diode D3 and the USB-C receptacle, on the VBUS run | seated or not, from above |
| upper | TP3 | `+3V3_MCU` | `3V3M` | top, mid-board over the DF40 socket, between U6 and D2/D3, at the end of the AG32 rail's spine | seated or not, from above |
| lower | TP1 | `+3V3_BUF` | `3V3B` | top, at the pin 1 end, below-left of the regulator U7, level with socket pin 2, off the run from U7 to C13 | in the host, with the upper board lifted off |

The lower board's underside faces the host socket and its top the upper
board, so its pad is on top: lift the upper board off J5 and the lower
board stays in the host, powered, with TP1 in the open at the pin 1 end,
clear of J5. The socket's own 5 V, `+5V_SKT`, has no pad: it is J1 pin 20
itself (the socket's pin 20 in the host, the supply lead on the bench).
For ground, use J3's `GND` pad or the receptacle's shell on the upper
board, or J1 pin 10.

The AGM parts are LCSC-only and MSL 3, so they need a bake if they have been
sitting in humidity. At JLCPCB, AGM parts are "Extended" and stay in the
JLCPCB library for assembly only - they cannot be shipped loose, so
prototypes assembled elsewhere need a separate LCSC order.

## What is not done

Nothing has been fabricated. The open measurements for the first boards -
ground bounce at the enable event, the buffer regulator's ramp, fabric
configuration time, and the socket timings against the model - are listed
at the end of `../DESIGN_NOTES.md`.
