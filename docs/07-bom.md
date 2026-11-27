# Bill of materials

The stack is assembled by JLCPCB from LCSC stock. The source of truth is
`tools/jlc_parts.yaml`, which maps each schematic value and footprint to a
manufacturer part number and an LCSC number; `make jlc` matches every part in
each board's exported netlist against it and writes JLC's BOM and CPL into
`build/jlc/<board>/`. A part with no matching line stops the build, so a
schematic change cannot silently order the wrong part.

**Provenance.** The LCSC numbers were captured on 2026-11-01. Stock moves:
check each line on jlcpcb.com/parts before ordering. No prices were recorded
with them, so this page carries none.

## Lower board, `fastprom-lower`

21 placed parts in 8 BOM lines, plus the pin strips (loose), one unfitted
pad pair and one bare test pad.

| Refs | Value | Manufacturer PN | LCSC | Package | Qty |
|---|---|---|---|---|---:|
| U2, U3, U4 | SN74LVT245B | TI SN74LVT245BRGYR | C2864103 | 20-VQFN (RGY) 3.5 x 4.5 mm | 3 |
| U7 | TLV74233 | TI TLV74233PDQNR | C469916 | X2SON-4 1 x 1 mm | 1 |
| J5 | DF40C-30DP-0.4V | Hirose DF40C-30DP-0.4V(51) | C429942 | 2 x 15, 0.4 mm header | 1 |
| RN1, RN2 | 4x33R | Uniroyal 4D02WGJ0330TCE | C25501 | 4 x 0402 convex array | 2 |
| R12-R16, R26 | 100k | Uniroyal 0402WGF1003TCE | C25741 | 0402 | 6 |
| R21 | 10k | Uniroyal 0402WGF1002TCE | C25744 | 0402 | 1 |
| C10, C12, C13 | 1uF | Samsung CL05A105KA5NQNC | C52923 | 0402 | 3 |
| C8, C9, C17, C18 | 100nF | Samsung CL05B104KO5NNNC | C1525 | 0402 | 4 |
| J1 | DIP20_PLUG | Preci-Dip 350-10-110-00-001101 | - (ships loose) | 10-way SIP, 0.47 mm round pin | 2 strips |
| R20 | 100k | - | - | 0402 | not fitted |
| TP1 | TestPoint | - | - | bare 1 mm SMD pad | nothing fitted |

**R20** is the 256x8 option: pin 16 is A5 on the 512x8 and wants R16's
pull-down, and a second enable on the 256x8 and wants R20's pull-up to the
socket's 5 V. Both pads are laid out and exactly one is fitted; the schematic
marks R20 DNP, so JLC fits R16. For a 256x8 build, swap them.

## Upper board, `fastprom-upper`

31 placed parts in 18 BOM lines, plus the programming pads and two bare test
pads.

| Refs | Value | Manufacturer PN | LCSC | Package | Qty |
|---|---|---|---|---|---:|
| U1 | AG32VF303KCU6 | AGM AG32VF303KCU6 | C41397171 | QFN-32-EP 4 x 4 mm | 1 |
| U5 | AP2112K-3.3 | Diodes AP2112K-3.3TRG1 | C51118 | SOT-23-5 | 1 |
| U6 | USBLC6-2SC6 | ST USBLC6-2SC6 | C7519 | SOT-23-6 | 1 |
| X1 | 12MHz | YXC OT201612MJBA4SL | C669076 | 2.0 x 1.6 mm, 4 pin | 1 |
| J4 | USB_C_USB2.0 | HRO TYPE-C-31-M-12 | C165948 | USB-C receptacle, at the board edge | 1 |
| J6 | DF40HC(3.0)-30DS-0.4V | Hirose DF40HC(3.0)-30DS-0.4V(51) | C531034 | 2 x 15, 0.4 mm socket, 3.0 mm stack | 1 |
| SW1 | Image_Select | XKB DSHP02TS-S | C319052 | 2-way slide DIP, 1.27 mm | 1 |
| D2, D3 | Schottky | B5819WS | C22624 | SOD-323 | 2 |
| FB1 | 220R | TDK MPZ1608S221ATA00 | C76815 | 0603 | 1 |
| C2, C14 | 10uF | Samsung CL10A106KP8NNNC | C19702 | 0603 | 2 |
| C1, C20 | 4.7uF | Samsung CL10A475KO8NNNC | C19666 | 0603 | 2 |
| C15 | 1uF | Samsung CL05A105KA5NQNC | C52923 | 0402 | 1 |
| C3, C4, C5, C11, C16, C19 | 100nF | Samsung CL05B104KO5NNNC | C1525 | 0402 | 6 |
| R9, R10, R24, R25 | 10k | Uniroyal 0402WGF1002TCE | C25744 | 0402 | 4 |
| R17, R18 | 100k | Uniroyal 0402WGF1003TCE | C25741 | 0402 | 2 |
| R11 | 4k7 | Uniroyal 0402WGF4701TCE | C25900 | 0402 | 1 |
| R22, R23 | 5k1 | Uniroyal 0402WGF5101TCE | C25905 | 0402 | 2 |
| R27 | 1R | Uniroyal 0402WGF100KTCE | C25086 | 0402 | 1 |
| J3 | SOICbite | - | - | eight bare edge pads, both faces | nothing fitted |
| TP2, TP3 | TestPoint | - | - | bare 1 mm SMD pad | nothing fitted |

The designators and quantities above are the ones `make jlc` writes; if they
disagree with `build/jlc/<board>/<board>-bom.csv`, the CSV is right and this
page is stale.

## What ships loose, what is optional, what is unverified

* **Ships loose: the pin strips (J1).** JLC does not fit them. Two Preci-Dip
  10-way strips per module, soldered by whoever fits the module, tails
  trimmed before stacking (below).
* **Not assembled: the test pads (TP1 on the lower board, TP2 and TP3
  on the upper).** Bare 1 mm copper pads for probing the supplies at
  bring-up (`../DESIGN_NOTES.md`, "Test pads"); `tools/jlc_parts.yaml` marks
  them DNP, so they are in neither BOM nor CPL and JLC places nothing on
  them. ENIG leaves them flat and gold for a probe tip.
* **Optional: 4 x 47 R terminators** (Uniroyal 4D02WGJ0470TCE, LCSC C55183,
  same footprint) in place of RN1/RN2's 4 x 33 R, for builds targeting 45 ns
  and slower parts only. They damp ground bounce through the single socket
  ground pin harder, at about 0.5 ns per data path - most of the fastest
  512x8 grade's tEA margin (`../DESIGN_NOTES.md`, "33 Ω series terminators").
  Set RN1/RN2's value to `4x47R` and `make jlc` picks the line up.
* **Checked against the makers' drawings (2026-11-05):**
  * **SW1**, XKB DSHP02TS-S (drawing X-DSHP02TS-S rev A1, LCSC C319052) on
    KiCad's KingTek DSHP02TS footprint: XKB's recommended PCB layout is
    0.76 x 1.27 mm pads at 1.27 mm pitch, rows 6.35 mm apart inside and
    8.89 mm outside (7.62 mm between pad centres). The footprint is exactly
    that. Pins are numbered as a DIP, so switch 1 is pins 1-4 and switch 2
    pins 2-3, as XKB's circuit shows and the schematic uses. XKB's top view
    prints the switch numbers along one pin row and `ON` along the other;
    with DIP numbering that puts ON toward pins 3-4 (the board's silkscreen
    labels the switch `IMG` and leaves `ON` and the numbers to the part).
    The switch's orientation in the CPL still decides which way is ON and
    which is switch 1 (image bit 0) and switch 2 (bit 1): check its pin 1
    in JLC's assembly preview.
  * **X1**, YXC OT201612MJBA4SL (YSO110TR series, 2.0 x 1.6 mm, LCSC
    C669076): pins 1 tri-state (OE), 2 GND, 3 OUT, 4 VDD, matching the
    schematic. KiCad's SiT PQFN-4 2.0 x 1.6 footprint has YXC's recommended
    0.9 x 0.8 mm pads, numbered the same way round (1 bottom-left,
    counter-clockwise, top view), with their centres 0.05 mm further out per
    side (gaps 0.6 x 0.4 mm against YXC's 0.5 x 0.3 mm); the package's
    terminals still land wholly inside every pad. YXC asks for 0.01-0.1 uF
    from VDD to GND, which is C16 (100 nF). X1 is the 12 MHz part because
    LCSC has no 8 MHz oscillator in this size; 12 MHz serves as well (the
    PLL's 1200 MHz VCO is x100, giving USB's 60 MHz exactly).
* **Unverified:**
  * **SN74LVT245BRGYR stock** is thin: about 225 at LCSC on 2026-11-01,
    against three per lower board.
  * **Price and stock of every line**, the DF40 pair included: none was
    recorded (`09-two-board-stack.md`).
  * **Component rotations in the CPL.** `make jlc` corrects KiCad's angles
    and centroids for JLC's library footprints (the `cpl` table in
    `tools/jlc_parts.yaml`, derived 2026-11-05 against the EasyEDA
    footprints; JLC's bottom-side convention is in `tools/jlc_fab.py`).
    JLC's preview is still the last word: check every part's pin 1 there
    before confirming an order. The DF40s' and resistor arrays' pin-1
    markers sit on pad 2 and pad 4 of ours (their EasyEDA numbering is a
    mirror image of KiCad's), and U7's exposed pad shows on bare board.

## Part choices

### The transceiver package

The transceiver is `SN74LVT245B` in 20-VQFN, the one package of it that
fits the 5.08 mm channel between the socket pad rows
(`01-part-selection.md`): TSSOP-20 is 6.4 mm across its leads and SSOP-20
7.8 mm. The RGY package is **3.5 x 4.5 mm** (from the datasheet); KiCad's stock
`Texas_RGY_R-PVQFN-N20_EP2.05x3.05mm` matches TI's land pattern
(`hw/lib/fastprom.pretty/README.md`).

`SN74LVT245B` has no bus hold, which suits pins that receive from an unknown
host. Its switching characteristics, from the datasheet, are identical to
`SN74LVTH245A`'s: tpd 3.5 ns, enable 5.5 ns, disable 5.9 ns max at 3.3 V
over temperature, as `03-timing-budget.md` assumes.

`SN74LVT245BDBR` in SSOP-20 is the fallback **if the board grows** - it
cannot be used in the DIP-20 channel.

onsemi is dead in this family: `74LVTH245MTCX` and `74LVT245WMX` are both
obsolete at zero stock. Nexperia's DHVQFN20 at 2.5 x 4.5 mm is the smallest
part in the family, but had zero stock and a 16 week lead on 2026-09-12.

### Open-collector buffers

There is no open-collector lower board: the stack's drive buffer U4 is a
'245 with one enable for all eight bits (`02-electrical.md`). If one is ever
made, these are the findings to start from.

`SN74AC7541` is an octal buffer with open-drain outputs in a 4.5 x 2.5 mm
VQFN, in stock, cheap, and exactly eight channels. **Do not use it.** Its
absolute maximum on any pin is VCC + 0.5 V, so at VCC = 3.3 V a 5 V pull-up
on the host bus exceeds it. Running it at 5 V does not help either, because
AC-family VIH is 0.7 x VCC = 3.5 V, which 3.3 V logic cannot drive. It is the
only true 3.3 V octal open-drain buffer in existence and it is wrong for this
application. A parametric search will offer it first.

`74LVC07A` is right, and unusually clearly so: its absolute maximum on both
inputs and open-drain outputs is 6.5 V **independent of VCC**, with IOL 24 mA.
That is exactly open-collector behaviour on a 5 V bus. (Two `74LVC07ABQ,115`,
14-DHVQFN 2.5 x 3.0 mm, would replace U4.)

Also rejected: `74LV07A` (I/O not rated above VCC + 0.5 the way LVC07A is),
`74HC07` (5 V part, wrong supply domain), and `TXS0108E` (internal pull-ups
both sides and auto-direction sensing - it cannot present a true float and
will fight a host that drives the bus).

### Round pins

`Mill-Max 800 series` dominates searches for machined pin headers and is
not used: its pin head is 0.030 inch, against a DIP socket acceptance
window of 0.015 to 0.025 inch. It will splay the socket's contacts
permanently, which breaks the socket for the original part as well.

`Mill-Max 310 series` also appears in those searches and is a **female
receptacle**, discontinued at DigiKey.

The chosen part is a **Preci-Dip 350-series 10-way strip**, two per module,
at 0.300 inch row spacing: 0.47 mm round pin, 3.96 mm mating length against a
2.7 mm minimum insertion, 4.17 mm solder tail. The tails go up through the
lower board and are soldered on top, so the insulator ends up underneath and
doubles as an alignment jig.

**Trim the tails to 1.0 mm above the lower board before stacking, set with
a gauge or shim; 1.5 mm is the limit.** A 4.17 mm tail stands 3.37 mm proud
of the 0.8 mm board, more than the 3.0 mm stack gap. Over the pin rows the
upper board's underside carries an 0603 (C1, beside pin 2's tail) and
J4's through-hole shell legs (over pins 9-12), which come through about
0.24 mm. The shell is grounded: a tail left long under it shorts D3
(pin 9) or D5 (pin 12) to ground.

**Before stacking, check J4's legs on the upper board's underside:** soldered
flush, with no solder spikes. Reasons and margins: `../DESIGN_NOTES.md`,
"Pin-tail trim". If the tallest part or leg over a pin row changes, redo
this.

`350-80-110-00-001101` (DK `1212-1145-ND`) is the same geometry in tin at
roughly half the price. Vintage sockets are usually tin, so tin-on-tin is the
textbook match; gold is better for repeated insertion. Either is fine for a
part that gets plugged in once.

On 2026-09-12 these carried a 17 week factory lead time against 453 and 431
pieces on hand at DigiKey, so order pins with the boards rather than after
them.

### Passives: DC bias

Class-2 ceramics lose capacitance under DC bias, and worse as the case
shrinks at fixed value. That is why the 4.7 uF (C1, C20) and 10 uF (C2, C14)
capacitors are **0603, not 0402**: at 3.3 V a 4.7 uF 6.3 V X5R in 0402
typically delivers 1.2 to 1.9 uF - with a 20 % tolerance on top, worst case
under 1 uF, which is the regulator's minimum - and the 16 V 0402 part
(`GRM155R61C475ME01D`) was at zero stock. The 100 nF decoupling stays in
0402, where the small body buys lower loop inductance at no derating cost.

C10, U4's decoupling, is 1 uF rather than 100 nF: U4 switches all eight data
lines into the host at once (`../DESIGN_NOTES.md`).

On 2026-09-12 Samsung's **0603** CL-series MLCCs showed 39 to 49 week lead
times and widespread zero stock; the 0402 ones did not. The stack's 0603
Samsung lines (CL10A106KP8NNNC, CL10A475KO8NNNC) have LCSC numbers, but
their stock was not recorded; check them first.

## Ordering: the assembly panel

`hw/ORDER-JLCPCB.md` sets every field of JLC's order form; the hardware
release attaches it beside the panel's gerber zip, BOM and CPL. Upload
those, not the release's source package.

JLC assembles the stack from one panel of five matched pairs, built by
`make panel` (`tools/gen_panel.py`) from the routed boards. `make jlc-panel`
writes the order files into `build/jlc/fastprom-panel/`:
- the gerber zip;
- the BOM;
- the CPL, with every instance named `<ref>_<n>`.

The reasons and JLC's rules behind the panel are in `../DESIGN_NOTES.md`,
"Assembly panel".

Every field of JLC's PCB and assembly order form is set in
`../hw/ORDER-JLCPCB.md`. Assembly is Standard PCBA on both sides, since
Economic places one side only and both boards carry parts on both sides.
JLC counts the panel as two designs, so it adds one panel fee to the setup
fee.

**Prototypes.** JLC's minimums are 5 bare panels and 2 assembled. Order 5
panels with PCBA quantity 2. That gives 10 modules, or 2N if the BOM and
CPL come from `make jlc-panel PAIRS=N`, which lists only each panel's first
N pairs. Unassembled instances on an assembled panel still get paste from
the stencil, so they come back tinned. The three unassembled panels come
back clean.

**Before you confirm,** check every part's pin 1 in JLC's placement preview
(see "Component rotations in the CPL" above). The U1 QFN and the U2–U4
buffers are underneath, and the upper boards are turned 180° on the panel.

**On arrival,** break each board out at its tabs and sand off the nubs,
about 0.25 mm, on the long edges. The lower board's long edges must stay
within the socket footprint's 11.02 mm. Each lower and upper pair makes one
module.

## Sourcing risks

| Risk | Why it matters |
|---|---|
| **AGM is single-sourced to LCSC**, with no Western distributor. | No second source exists for the one irreplaceable part. |
| **LCSC's parametric data on AGM parts is wrong** - it advertises 248 MHz and 1 MB flash for a part AGM specs at 208 MHz and 256 K. | Do not design from LCSC parametrics; use AGM's documents. The QFN-32 pinout comes from AGM's pinout spreadsheet (`00-ag32-reference.md`). |
| **LVT 5 V tolerance does not extend to driving high.** | See `02-electrical.md`. Fine into 5 V TTL, not 5 V CMOS. |
| **SN74LVT245BRGYR stock is thin** (17 at JLC on 2026-11-18; TI out of stock). | Three per module, and no other package fits the channel. The LVTH part in the same package cannot replace it (`02-electrical.md`), and the in-stock 74LVC245A in VQFN is too slow for the fastest grades. |
| **AGM parts at JLC are "Extended"**, assembly only. | They cannot be shipped loose; boards assembled elsewhere need a separate LCSC order (`05-hardware.md`). |

## Cost

Not computed: no prices were recorded with the LCSC numbers. The machined
pins are the largest line among the parts and cost more than the
microcontroller.

For comparison, the part being replaced draws 140 to 170 mA continuously at
5 V. The replacement draws a few tens of milliamps, which frees roughly
0.7 W per socket.
