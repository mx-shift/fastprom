# Project footprints

## Silkscreen

Every footprint here keeps JLC's legend rules, which the boards' DRC
enforces (`DESIGN_NOTES.md`, "Silkscreen rules"): every silkscreen
stroke at least 0.15 mm wide, and every silkscreen item at least 0.15 mm
from every pad's copper (the boards' pads have no mask expansion, so
that is the mask opening) and, placed, from the board edge and from
other legend. KiCad's stock footprints draw 0.12 mm lines, and its 0402
outlines stand 0.10 mm off their pads, so each footprint copied from the
stock library has its silk strokes at 0.15 mm, and strokes that would
come too near a pad at that width are trimmed or moved (below).

## The pin 1 triangle

Every footprint here for a part with a pin 1 carries the same pin 1 mark
on F.SilkS, and no other: a filled triangle, 0.5 mm across the base and
0.35 mm tip to base, with a 0.15 mm stroke (0.65 x 0.5 mm printed). It
lies entirely outside the part's body (the Fab outline), stroke included,
so it shows with the part on, and points in toward the package (tip
nearer the body than the base). It points at pin 1: on pad 1's centre
line (its axis), beyond the pad's outer end (away from the package),
tip toward the pad and 0.2 mm from its copper, stroke included - past
the inner end only where the outer end is the board edge (the
SOICbite). Never beside pad 1: a neighbour in the way moves on the
board, the mark stays. `tools/check_pin1.py` (run by `make
layout-drc`) fails the build if a mark on a board touches any part's
body, points away from its own, or is not on its pad 1's axis pointing
at that pad. `DESIGN_NOTES.md`
("Silkscreen marking") lists each part and its clearances. The side, in
each footprint's own frame:

| Footprint | Side of pad 1 | Triangle (tip; base corners), mm |
|---|---|---|
| `DIP-20_W7.62mm_PreciDip350_2xSIP` | -x (outer end) | (-1.075, 0); (-1.425, ±0.25) |
| `Hirose_DF40C-30DP-0.4V_2x15-1MP_P0.4mm` | -y (outer end) | (-2.8, -1.96); (-2.8 ±0.25, -2.31) |
| `Hirose_DF40HC(3.0)-30DS-0.4V_2x15_P0.4mm` | +y (outer end) | (-2.8, 2.165); (-2.8 ±0.25, 2.515) |
| `Texas_RGY_R-PVQFN-N20_EP2.05x3.05mm` | -y (outer end) | (-0.75, -2.725); (-0.75 ±0.25, -3.075) |
| `QFN-32-1EP_4x4mm_P0.4mm_EP2.65x2.65mm` | -x (outer end) | (-2.625, -1.4); (-2.975, -1.4 ±0.25) |
| `SOT-23-5`, `SOT-23-6` | -x (outer end) | (-2.075, -0.95); (-2.425, -0.95 ±0.25) |
| `Texas_X2SON-4_1x1mm_P0.65mm_NoEP` | -y (past the row's end) | (-0.43, -0.755); (-0.43 ±0.25, -1.105) |
| `Oscillator_SMD_SiT_PQFN-4Pin_2.0x1.6mm` | +y (outer edge) | (-0.75, 1.275); (-0.75 ±0.25, 1.625) |
| `SOICbite_2x04_P1.27mm_BoardEdge` | -y (inner end, past the clip's outline) | (1.905, -2.69); (1.905 ±0.25, -3.04) |
| `SW_DIP_SPSTx02_Slide_KingTek_DSHP02TS_W7.62mm_P1.27mm` | -x (outer end) | (-4.72, -0.635); (-5.07, -0.635 ±0.25) |

## Copies of KiCad's stock footprints, for the triangle

These are KiCad 10's stock footprints of the same names (CC-BY-SA-4.0,
`REUSE.toml`), with their pads, courtyards, fab layers and 3D models
unchanged; the stock pin 1 mark is replaced by the triangle, and their
other silk lines are 0.15 mm wide (stock 0.12 mm), in the same places:

| Footprint | Part | Stock library | Stock pin 1 mark removed |
|---|---|---|---|
| `QFN-32-1EP_4x4mm_P0.4mm_EP2.65x2.65mm` | U1 AG32 | `Package_DFN_QFN` | filled triangle at the corner |
| `Texas_RGY_R-PVQFN-N20_EP2.05x3.05mm` | U2-U4 transceivers | `Package_DFN_QFN` | filled triangle at the corner |
| `SOT-23-5` | U5 regulator | `Package_TO_SOT_SMD` | filled triangle |
| `SOT-23-6` | U6 USB clamp | `Package_TO_SOT_SMD` | filled triangle |
| `Oscillator_SMD_SiT_PQFN-4Pin_2.0x1.6mm` | X1 | `Oscillator` | the L-shaped outline along pin 1's two sides (its only silk) |
| `Hirose_DF40C-30DP-0.4V_2x15-1MP_P0.4mm` | J5 | `Connector_Hirose_DF40` | outline triangle (three 0.12 mm lines) |
| `Hirose_DF40HC(3.0)-30DS-0.4V_2x15_P0.4mm` | J6 | `Connector_Hirose_DF40` | outline triangle (three 0.12 mm lines) |
| `SW_DIP_SPSTx02_Slide_KingTek_DSHP02TS_W7.62mm_P1.27mm` | SW1 | `Button_Switch_SMD` | the lead-in line along pad 1 |

Each keeps its stock name, in this library, so the BOM and CPL name the
stock footprint; the schematic names them `fastprom:...`.

## Copies of KiCad's stock footprints, for the legend rules

KiCad 10's stock footprints of the same names (CC-BY-SA-4.0,
`REUSE.toml`), pads, courtyards, fab layers, reference fields and 3D
models unchanged; only the silkscreen differs. None has a pin 1 mark (no
pin 1 orientation; the diodes' cathode bracket is a polarity mark and
stays).

| Footprint | Parts | Stock library | Silkscreen |
|---|---|---|---|
| `R_0402_1005Metric` | the 0402 resistors | `Resistor_SMD` | none: the stock outline is two 0.3 mm dashes between the pads, 0.10 mm off them; at 0.15 mm off they would be 0.13 mm long (resistor) or nothing (capacitor) |
| `C_0402_1005Metric` | the 0402 capacitors | `Capacitor_SMD` | none, as the resistor's |
| `C_0603_1608Metric` | the 0603 capacitors | `Capacitor_SMD` | none: no passive carries an outline (below) |
| `L_0603_1608Metric` | FB1 | `Inductor_SMD` | none |
| `D_SOD-323` | D2, D3 | `Diode_SMD` | stock cathode bracket at 0.15 mm (0.19 mm off the pads) |
| `R_Array_Convex_4x0402` | RN1, RN2 | `Resistor_SMD` | none (both ends are alike: four equal elements) |
| `TestPoint_Pad_D1.0mm` | TP1-TP3 | `TestPoint` | ring 0.75 mm in radius at 0.15 mm (stock 0.70 mm at 0.12 mm, 0.14 mm off the pad; here 0.175 mm off) |

Each keeps its stock name, so the BOM, the CPL and `tools/jlc_parts.yaml`
match on the same name; the schematic names them `fastprom:...`.

No passive carries a body outline: the 0402s' cannot stand clear of their
pads, and the boards draw none rather than some. Placement is JLC's, from
the CPL, checked part by part in their preview; the marks that matter
for orientation stay (pin 1 triangles, the diodes' cathode brackets).

## SOICbite_2x04_P1.27mm_BoardEdge

`J3`, the programming connector on the stack's upper board: Simon Merrett's
SOICbite (github.com/SimonMerrett/SOICbite, BSD-2-Clause), renamed with its
pads and holes unchanged. Eight pads at the board edge, 1-4 on top and 8-5 under
them on the bottom, 1.27 mm pitch, 0.8 x 1.5 mm, and five 0.7 mm NPTH a
row 1.905 mm in from the edge. A SOIC-8 test clip, closed further than it
leaves the factory, bites the edge and holds itself; a printed clip with
pogo pins top and bottom fits the same pads, located by the holes. Pin out
as SOICbite's ESP8266 column: 1 3V3, 2 BOOT0, 4 GND, 5 RX, 6 TX, 8 NRST
(RX and TX the AG32's); 3 and 7 unused. The board is programmed over the
AG32's UART0 mask-ROM loader only (docs/06-build-and-flash.md): the fixture
holds BOOT0 high to select it, and NRST resets into it. Placed with its Fab
line (y = +0.381) on the board edge, so the pads start 0.381 mm in; its pin
1 mark, a 0.1 mm cup round pad 1, is replaced by the pin 1 triangle,
at pad 1's inner end past the Fab outline (the area the clip covers; the
pad's outer end is the board edge).

## USB_C_Receptacle_HRO_TYPE-C-31-M-12_BoardEdge

`J4` on the stack's upper board (no pin 1 mark: the receptacle is
symmetric): HRO's own recommended land pattern for the
TYPE-C-31-M-12 (drawing rev A, 2020-12-08, from LCSC C165948's datasheet),
with KiCad's stock courtyard, fab outline and 3D model, for the receptacle
mounted at the board end. HRO's drawing puts the PCB edge 5.79 mm in front
of the locating-peg holes, at the footprint's y = +3.19.

KiCad's stock land pattern is not used; it differs from HRO's:
- the stock signal pads are 1.45 mm long, against HRO's 1.14 mm, and sit
  0.375 mm further from the peg holes, centre to centre;
- the stock GND and VBUS pads are 0.05 mm further out (x ±3.25 and ±2.45,
  against ±3.20 and ±2.40);
- the stock peg holes are 0.65 mm, against HRO's 0.60 mm, for 0.50 mm pegs;
- the stock shell slots are 0.03 mm off;
- the stock rear slot is shorter (drill 1.2 mm, against 1.4 mm);
- the stock edge line is 0.16 mm further forward.

The silkscreen side lines run from y = +2.08, their ink 0.17 mm clear of
the rear shell pads, to y = +2.94, 0.25 mm inside the edge (their 0.15 mm
ink 0.175 mm inside it), and the front line is left off (the edge is the
front).

## DIP-20_W7.62mm_PreciDip350_2xSIP

`J1` on the stack's lower board: the 20 pins that plug into the host's DIP
socket, made of two 10-way machined-pin strips rather than a DIP body. The
pads are the stock DIP-20's (1.6 mm, 0.8 mm drill, 7.62 x 2.54 mm grid). The
courtyards follow what is really there: on top, only the solder fillets of
each row; underneath, each strip's 2.54 mm insulator. The top courtyards
stand 0.10 mm off the pads (IPC's least margin), not KiCad's usual 0.25:
on top there is no body, only the pin tails' solder fillets inside each
1.6 mm pad, and a 0.25 mm margin would leave U4 (the RGY drive buffer,
across the channel at the far end) no room to clear pin 10 with its pin
1 triangle on pad 1's axis (DESIGN_NOTES.md, "Silkscreen marking"). The channel between the
rows is empty, which is where the board-to-board header sits - the stock DIP
footprint's single courtyard round both rows reports that as a collision.
Every pin but pad 10 (ground) keeps its copper ring only on the outer layers
and wherever a track meets it (`remove_unused_layers`, `keep_end_layers`):
full 1.6 mm rings on the inner layers would make each signal pin cut a
2.0 mm hole in the lower board's In1 ground plane, leaving 0.54 mm necks
between the pins and slivers outside the columns. Pad 10 keeps its ring on
every layer, where it joins the plane. Its pin 1 mark is the pin 1
triangle, outside the pin 1 column.

## The RGY transceiver package

KiCad's stock `Package_DFN_QFN:Texas_RGY_R-PVQFN-N20_EP2.05x3.05mm` is TI's
RGY (20-VQFN, 3.5 x 4.5 mm): 0.5 mm pitch, pad rows at +/-1.65 mm,
0.6 x 0.24 mm pads, 2.05 x 3.05 mm exposed pad, matching TI's RGY0020A land
pattern. The boards use it, through this library's copy (above).

## Panel_*

`Panel_Fiducial_1mm_Mask2mm` (KiCad's stock fiducial, kept out of the BOM
and position files), `Panel_MouseBite_NPTH_0.6mm` and
`Panel_ToolingHole_NPTH_2mm` are used only on the assembly panel that
`tools/gen_panel.py` builds, never on a board.

## Texas_X2SON-4_1x1mm_P0.65mm_NoEP

`U7`, the lower board's TLV74233 regulator: KiCad's stock
`Texas_X2SON-4_1x1mm_P0.65mm` (TI DQN0004A, CC-BY-SA-4.0) with plain
rectangular pads in place of its chamfered custom ones (Freerouting 2.5.0
cannot reach those) and no thermal pad (under 0.127 mm from the pins in
that land pattern, and not needed at the 30 mA it carries here). Its pin 1 dot is replaced by the pin 1
triangle, past the end of pad 1's row; the body's top silk line, which
the triangle would cross, starts at the package's centre instead of its
corner.
