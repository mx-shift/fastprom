# The two-board stack

Every awkward constraint in fitting this design to a DIP-20 traces back to
one number: after the two rows of socket pads, the footprint leaves a
**5.08 mm channel** to put things in. On one board that channel would have to
hold the AG32, three bus buffers, their terminators, two regulators and a
USB-C connector - and it cannot. A USB-C receptacle alone is wider than the
channel and deeper than the 1.7 mm past each end of the pin field, so on one
board it needs a tab hanging off the end of the module
(`08-usb-updates.md`).

The stack splits the design across two boards of the socket's own footprint,
3 mm apart, so each half gets a whole board - both sides of it - to itself.

## What goes where

**Lower board** (`fastprom-lower`, 11.02 x 26.26 mm, 4 layers): the
socket side of the design, everything that touches the host's bus.

* the 20 machined pins (two Preci-Dip strips, soldered on top)
* the three SN74LVT245B buffers: two receive the socket's ten inputs, one
  drives its eight data lines, enabled by `D_BUF_NOE`
* two 4 x 33 R arrays, series terminators on the data lines
* the socket-side pulls: pull-downs on the high address pins, a pull-up on
  /CE, and the R16/R20 pair that makes pin 16 an address bit (512x8) or a
  second enable (256x8)
* the buffers' own 3.3 V regulator (TLV74233), fed from socket pin 20 and
  nothing else
* the DF40 header

Putting the whole bus interface here does two things. The connector carries
only 3.3 V logic plus the socket's 5 V supply, so the upper board never sees
5 V signals. And because the
buffers are powered by the host alone, with the host switched off they are
unpowered and their ports high-impedance (Ioff) whatever the upper board is
doing - a module running from USB on the bench cannot drive, or be fed from,
a dead host's bus.

The parts fill both sides of the board: the channel between the pin columns
on top (5.52 mm between the pin tails' solder fillets) and the channel
between the strips' insulators underneath (4.58 mm, 2.79 mm deep). Nothing
underneath is taller than 1 mm.

**Upper board** (`fastprom-upper`, 11.02 x 31.76 mm, 4 layers): the
AG32 and everything that serves it.

* the AG32VF303KCU6 (QFN-32), on the underside
* its decoupling, the ferrite-isolated +3V3_MCU fabric rail AGM asks for, and
  the 12 MHz oscillator the PLL makes the USB clock from
* USB-C with its ESD clamp, at one end, opening outward
* the 5 V OR-ing diodes (socket or USB, whichever is present) and the
  AP2112K regulator
* the BOOT0, BOOT1 and NRST straps
* the two-way image-select DIP switch, on top, reachable with the module
  seated
* the programming connector `J3`, a SOICbite: eight pads on both faces of
  the pin 1 end edge (3V3, BOOT0, GND, RX, TX, NRST) for a SOIC-8 clip or a
  pogo clip that holds itself on

The upper board is the lower board's width, so nothing overhangs beside the
socket. Both boards have their pin 1 corner chamfered, 1.0 mm at 45
degrees, which marks the module's pin 1 end (`05-hardware.md`, "Form factor").
The upper board is 5.5 mm longer:
* 1.5 mm at the USB end: the receptacle's two locating-peg holes are
  directly over the DF40 socket on the underside, and need at least
  1.24 mm more length to clear it in HRO's own land pattern (at 1.0 mm a
  peg hole falls inside the socket's courtyard);
* 4 mm at the pin 1 end, for `J3`: a clip's jaws take the edge strip on
  both faces, and at that end the regulator's parts (underneath) and the
  image-select switch (on top) left no room for it inside the lower
  board's length (`../DESIGN_NOTES.md`, "J3 at the pin 1 end"). J3's strip
  ends 1.08 mm short of the lower board's end, so a clip goes on with the
  module stacked and seated: its lower jaw comes up under the overhang,
  not into the gap between the boards.

## Heights

| | Height above the host PCB |
|---|---|
| DIP socket top face | about 3.4 mm |
| Pin-strip insulator (0.110 in) | 3.4 to 6.2 mm |
| Lower board (0.8 mm) | 6.2 to 7.0 mm |
| DF40 3.0 mm stack | 7.0 to 10.0 mm |
| Upper board (0.8 mm) | 10.0 to 10.8 mm |
| Top of the USB-C receptacle | about 14 mm |
| A seated DIP package, for comparison | about 7.4 mm |

The 1.5 mm at the USB end and the 4 mm at the pin 1 end overhang the host
at about 10 mm, above a neighbouring DIP chip rather than beside it. Past
pin 1 the upper board reaches 5.7 mm beyond the socket's pins: a DIP
(about 7.4 mm seated) clears it, but a taller part beside the socket's
pin 1 end - an electrolytic, a crystal, a socketed part - may not; check
the host before fitting. A clip on `J3` needs room for its lower jaw under
that 4 mm, between the upper board and whatever the host has beyond the
socket's end.

The 3 mm between the boards is shared by the parts on the lower board's top,
the parts on the upper board's underside, and the pin tails. Between parts
the worst case is 2.0 mm (the upper board's SOT-23-5 regulator over the
lower board's regulator and its 0402s). Over the pin tails it is about
1.0 mm: J4's shell legs come through the upper board over pins 9-12, and
0603s sit beside pins 20 and 19. The pin tails must be trimmed to 1.0 mm
(1.5 mm at most) before stacking, and J4's legs soldered flush: see
`docs/07-bom.md`.

## Why the QFN-32

The QFN-32's 24 fabric I/O are exactly enough, with none spare:

| Use | I/O |
|---|---|
| Socket lines (10 in, 8 out) | 18 |
| Data buffer enable | 1 |
| USB clock (12 MHz into OSC_IN) | 1 |
| USB D+/D- | 2 |
| UART TX/RX, which also read the two image-select bits | 2 |

Two image-select bits give four images. BOOT1 is shared with a data line,
which its strap pulls down at reset; that has to be a data line, because the
AG32 drives those and nothing else does until it is configured, whereas the
receive buffers drive theirs from power-up. There is no status LED.

The LQFP-48 would give 32 I/O and is the only package the open AGaMEMnon
toolchain has silicon evidence for (`docs/01-part-selection.md`). But its
9 x 9 mm lead span would take most of an 11.02 mm board's width, where the
QFN-32 takes 4 mm, and nothing in the design needs the extra pins.

The cost of the QFN-32 is the toolchain: for it, the open flow supports only
builds that assign no pads, so it cannot build this design yet, and the
bitstream comes from AGM's Supra (see `docs/06-build-and-flash.md`).

## Choosing the connector

Hirose's board-to-board range, filtered to something that carries ~30 contacts:

| Series | Pitch | Stack height | Body depth | KiCad footprints |
|---|---|---|---|---|
| BM28 | 0.35 mm | **0.6 mm** | 1.7 mm | none |
| BM23FR | 0.35 mm | 0.6 / 0.8 mm | small | 16-position only |
| BM24 | 0.35 mm | - | small | 40-position only |
| DF30 | 0.4 mm | 0.9 mm | small | none |
| **DF40** | 0.4 mm | 1.5 to 4.0 mm | 2.94 mm | both halves, 70 variants |
| DF12 / DF23 | 0.5 mm | 3.0 to 5.0 mm | larger | partial |
| FX8 / FX10 | 0.5 / 0.6 mm | - | - | 60 to 168 positions only |

BM28 is the smallest: 0.35 mm pitch, 0.6 mm stack, 1.7 mm deep, under half
DF40's area. It is not used because of how the pins mount. The DIP pins
solder on the **top** of the lower board: the Preci-Dip strip's insulator
sits underneath and the tails come up through, so the lower board's top
surface carries 20 solder fillets, 0.3 to 0.5 mm tall, in exactly the two pad
rows the upper board has to overhang.

**Stack height therefore has a floor as well as a ceiling.** BM28's 0.6 mm
and DF30's 0.9 mm cannot clear those fillets.

So: **DF40C-30DP-0.4V** header on the lower board and
**DF40HC(3.0)-30DS-0.4V** socket on the upper, giving a **3.0 mm stack** -
about 2.5 mm of clearance over the fillets, and room for 0402 passives on the
lower board's top side if needed. The pin strips ship loose and are soldered
by whoever fits the module, so the fillets are not ours to control; the
margin is deliberate. It is also the height LCSC stocks (C531034), so JLC
can assemble both boards from stock - the 2.0 mm DF40C(2.0)-30DS is not on
LCSC and has the identical land pattern. Both halves have verified
KiCad footprints.

Body 8.6 x 2.94 mm, 0.3 A per contact, -55 to +85 C. The header sits on
top of the lower board in the channel between the pin columns, which is
5.52 mm wide between the pin tails' solder fillets.

The connector has 30 contacts rather than the 1.2 mm shorter DF40C-24's 24:
30 contacts buy proper ground interleaving, and 1.2 mm would not change
either board's placement.

**Not verified:** price and stock. Both halves have LCSC part numbers in
`tools/jlc_parts.yaml` (C429942 header, C531034 socket, captured
2026-11-01), but no price or stock level was recorded for them. Wider
availability is inferred: Mouser has a dedicated DF40 landing page, RS stocks
the line, and KiCad ships 70 DF40 footprints against one or two for the BM
families. Check before ordering.

## Contact allocation

30 contacts: **18 buffered socket signals, the data buffer's enable, two
5 V and nine grounds**, with a ground next to every pair of signals. The field
evidence on these parts is explicit that edge quality matters as much as
access time, and a short return path across the connector is what protects
it. The order of the signals along the rows was chosen for the copper, and
the AG32's pin assignment (`socket_to_mcu` in `tools/boardmap.yaml`) follows
it: the fabric can put any signal on any I/O.

Both halves are generated from one function, `stack_contacts()` in
`tools/gen_kicad.py`, so their pinouts agree. A mismatch
between the two boards' connector pinouts would be invisible in both
schematics and fatal in hardware. `make stack` checks it independently, by
extracting the contact map from each exported netlist and comparing them.

## Layout

Both boards are routed and DRC clean, with zero violations and zero
unconnected items, on JLC's 4-layer 0.8 mm stack-up JLC04081H-3313: F and B
carry parts and all the routing, In1 and In2 are both solid ground planes,
each signal layer 0.1 mm over its plane. Track and space are 0.1/0.1 mm
against JLC's 0.09 mm minimum; vias are 0.45/0.2 mm. A signal that changes
layer changes plane, so each via off ground has a ground via beside it
where the copper allows, and the planes are stitched together along the
edges and round the connector, the AG32 and the buffers
(`../DESIGN_NOTES.md`, "Four layers on both boards").

The layout is generated, not drawn: `tools/gen_layout.py` holds the
placement, `tools/layout/fans.py` the hand-routed copper as checked data, and
`hw/<board>.route.json` the routed copper, so `make layout` rebuilds both
boards without running an autorouter. `make reroute` routes them again with
Freerouting after a placement change, `make layout-drc` checks them, and
`make jlc` writes gerbers, drill, BOM and CPL.

## Costs

* **Two PCBs and a connector pair** instead of one board. More BOM lines, two
  designs on the assembly panel, an extra assembly step. Both boards share
  one stack-up.
* **Taller.** About 14 mm to the top of the USB connector against roughly
  7 mm for the original part in its socket. Fine in an arcade cabinet; check
  it against any lid or daughterboard above the socket. Most of the excess is
  the 2.79 mm pin-strip insulator, which loose machined pins would avoid at
  the cost of needing an assembly jig.
* **Sixty more solder joints** on a 0.4 mm pitch connector.
* **The pin strips are soldered by hand**, shipped loose with the assembled
  boards, and their tails trimmed before stacking.
* **Insertion force must not pass through the stack.** Pushing on the upper
  board to seat 20 pins puts that load on the board-to-board connector, which
  is designed for retention rather than axial abuse. Press on the lower board.

## Other socket families

The split puts everything specific to the socket - its pinout, which
pins are inputs and which outputs, the level shifting - on the lower
board, and the AG32 on the upper, so one upper board might serve other
parts. None of this is built or verified.

**20-pin PALs and GALs (PAL16L8/16R8, GAL16V8): the same upper board, a
new lower board.** They have 18 signal pins, as the PROMs do, so the
stack connector's 18 signals and one enable carry them unchanged.
What differs:

- Direction per pin, at run time. A 16L8's pins 13-18 are I/O whose
  output enable is a product term (all eight on a 16V8), so a pin can
  change direction mid-operation. A '245 turns a whole chip at once and
  the connector has one enable. A direction-free 5 V-tolerant FET bus
  switch (SN74CB3T3245 class) on the lower board would do it, the AG32
  tri-stating each pad itself and the connector's enable gating the
  switch.
- The switch has to stay off until the AG32 has booted: BOOT1 shares
  socket pin 8 (R11), a data output behind a disabled U4 on the PROM
  board but a host-driven input on a PAL.
- Timing, estimated only: a bus switch adds little against the '245s'
  8.1 ns, but the AG32 pads' own enable time joins the path, and a PAL
  equation is a few LUTs deep (one LUT is 7.3 ns pad to pad in P&R).
  Somewhere around 15-20 ns at the socket: the -25 and perhaps -15
  grades, not the -5 to -10 ones.
- Registered parts clock from socket pin 1, which reaches AG32 PIN_8;
  whether that can drive a fabric clock is unchecked.
- The real work is the fabric. A 16-input function cannot be a table in
  block RAM (64 Kbit per output against about 36 Kbit in all), and a
  generic programmable 16V8 array (about 2,048 fuses) would take most of
  the 2,112 LUTs if it fits at all. More likely each JEDEC fuse map is
  turned into a design and built with AGM's toolchain, so a PAL is not a
  file a user uploads the way a PROM image is.

**24-pin parts: a different upper board, not a wider connector.** A
24-pin part has 22 signal pins. The QFN-32's 24 usable fabric I/O are
all spoken for (18 socket lines, the enable, the USB clock, USB D+/D-,
the UART), so four more socket lines would cost the UART and the USB
clock and still fall two short. A 40-contact DF40 would cost about 2 mm
and buy nothing on this upper board. The next AG32, the LQFP-48 (32
fabric I/O), is 9 x 9 mm across its leads:

- 0.6" DIP-24 parts (most 24-pin bipolar PROMs) allow a module about
  18 mm wide, room for an LQFP-48 upper board. A 2Kx8 image is 16 Kbit
  and fits block RAM, so these reuse the PROM firmware and are the
  natural first 24-pin target.
- 0.3" DIP-24 parts (GAL22V10, PAL20L8/20R8) keep the module about 11 mm
  wide, where the LQFP-48 does not fit and the QFN-32 lacks the pins.
  Open.

A 24-pin stack would be a new upper and lower pair, free to choose its
own connector, so the DIP-20 stack keeps its 30 contacts.

## Boards

| Board | MCU | Images | USB | |
|---|---|---|---|---|
| `fastprom-lower` | - | - | - | lower half: pins and bus interface |
| `fastprom-upper` | QFN-32 | 4 | USB-C, WebUSB device (`11-usb-driver.md`) | upper half: the AG32 |

Both are generated from `tools/boardmap.yaml`; both schematics are ERC clean
with zero errors and zero warnings, and both boards are DRC clean.
