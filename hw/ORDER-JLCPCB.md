# Ordering from JLCPCB

Every field of JLCPCB's PCB and PCBA order form, set for the FastPROM stack's
assembly panel. Upload the panel's own files, not the release's source
package: JLC parses only a gerber zip, and from it detects the layer count
and the size, nothing else. Everything below is set by hand.

The files, from the hardware release (or `make jlc-panel` into
`build/jlc/fastprom-panel/`):

| File | Where on the form |
|---|---|
| `fastprom-panel-gerbers.zip` | "Add gerber file", at the top |
| `fastprom-panel-bom.csv` | PCB Assembly, BOM |
| `fastprom-panel-cpl.csv` | PCB Assembly, CPL (pick-and-place) |

JLC's form changes its labels from time to time; the fields are named here
as the form names them on 2026-11-18, with the setting's reason so that a
renamed field can still be matched. Some fields appear only once an earlier
one is set (Panel Format, Gold Thickness, Layer Stackup); they are listed
where they appear. The reasons and calculations are
in `../DESIGN_NOTES.md`, "Fabrication options" and "Assembly panel".

## PCB

| Field | Set to | Why |
|---|---|---|
| Base Material | FR-4 | |
| Layers | 4 | detected from the gerbers; F.Cu, In1 (ground), In2 (ground), B.Cu |
| Dimensions | 77.10 × 74.02 mm | detected: the panel's outline |
| Panel Qty | 5 | panels (the field is "PCB Qty" until Delivery Format is set); JLC's minimum (prototypes: see below) |
| Product Type | Industrial/Consumer electronics | |
| Different Design | 2 | the panel carries two designs, the lower and upper boards |
| Delivery Format | Panel by Customer | the panel is in the files, edge rails, tooling holes and fiducials included |
| Panel Format | Column 5, Row 2 | five pairs side by side, the lower boards in one row and the upper boards in the other |
| PCB Thickness | 0.8 mm | the stack's height (`../docs/09-two-board-stack.md`) and J1's tail trim (`../DESIGN_NOTES.md`, "Pin-tail trim") assume it |
| PCB Color | Green | any colour works; green is the quickest |
| Silkscreen | White | |
| Material Type | FR4 TG135 | the default; nothing on the boards needs a higher Tg |
| Surface Finish | ENIG | the 0.4 mm-pitch QFN-32 and DF40 connectors want a flat finish, and J3's edge pads take a clip or pogo pins |
| Gold Thickness | 1 U" | the default |
| Outer Copper Weight | 1 oz | 0.1 mm track and space need 1 oz; 2 oz needs 0.15 mm |
| Inner Copper Weight | 0.5 oz | JLC's default |
| Specify Stackup | Yes | opens the stack-up dialog |
| Layer Stackup | JLC04081H-3313 | chosen in that dialog, then Confirm. The stack-up both board files record: 1 oz outer copper on 3313 prepreg (0.0994 mm) over the two 0.5 oz ground planes, 0.45 mm core (`../DESIGN_NOTES.md`, "Four layers on both boards") |
| Impedance Control | No requirement | no line is impedance-controlled; USB is full speed |
| Via Covering | Epoxy Filled & Capped | eleven vias sit in pads: ten in exposed pads (U1, U2, U3) and one in C4's ground pad; open vias there wick the solder away, and "plugged" is not offered for vias inside a mask opening |
| Via Plating Method | Not Specified | the default |
| Min via hole size/diameter | 0.2mm/(0.3/0.35mm) | every via is 0.2 mm drill, 0.45 mm diameter. This tier costs more than the 0.3 mm default (about $38 on five panels on 2026-11-18) |
| Board Outline Tolerance | ±0.1mm(Precision) | each board is exactly the socket footprint's 11.02 mm wide and must not grow. Each board's pin 1 corner is a 1 mm 45-degree chamfer (the module's pin 1 mark), routed with the rest of the outline: no special request |
| Confirm Production file | Yes | first order: check how JLC has read the panel, its tabs and mouse bites |
| Mark on PCB | Remove Mark | the default: no order number or code is added to the boards |
| Electrical Test | Flying Probe Fully Test | the only choice |
| Gold Fingers | No | |
| Castellated Holes | No | |
| Edge Plating | No | |

Leave every other field at its default.

## PCB Assembly

| Field | Set to | Why |
|---|---|---|
| PCBA Type | Standard | Economic places one side only; both boards have parts on both sides |
| Assembly Side | Both Sides | |
| PCBA Qty | 2 | panels; JLC's minimum, 10 modules |
| Tooling holes | Added by Customer | in the panel's rails |
| Confirm Parts Placement | Yes | check every part's pin 1 in the preview (below) |

Then upload the BOM and CPL. Not placed, by design: J1's two pin strips are
not in the BOM (JLC does not fit them; order them separately and solder
them by hand, `../docs/07-bom.md`), and R20 is a schematic DNP.

Soldering J1's strips by hand: pins 10-13 are within 0.4 mm of U4's
pads on the lower board's top. Use a fine tip and as little solder as
the joint takes on those four, then inspect U4's right-hand row for
bridges. Hot-air rework on U4 also reflows J1's pins 10-12.

## Before confirming

* In the placement preview: U1 sits on the upper board's underside; SW1's
  ON side, toward its pins 3-4, where the silkscreen says `ON`; the diode
  bands; the DF40 connectors' and resistor arrays' pin-1
  markers, which sit on pads 2 and 4 by design (`../docs/07-bom.md`). The upper
  boards are turned 180° on the panel.
* Prototypes: 5 bare panels and 2 assembled give 10 modules. For fewer,
  `make jlc-panel PAIRS=N` writes a BOM and CPL with only each panel's first
  N pairs; the unassembled instances come back tinned.

## On arrival

Break each board out at its tabs and sand off the nubs, about 0.25 mm. They
are on the long edges only, never on a short end:
- lower: two per side, 5 mm each, centred 5.51 mm and 20.75 mm from the
  pin 1 end (between socket pins); its long edges must stay within the
  socket footprint's 11.02 mm;
- upper: three per side: one near the J3 (pin 1) end, 1.5 mm to 6.5 mm
  from it along each long edge (clear of the pin 1 corner's chamfer);
  one mid-board, centred 18.3 mm from that end; one flush with the USB
  end. J3's end edge itself has none, and must
  stay clean for the clip: sand the long edges only. The J3-end tabs end
  0.5 mm short of the chamfer, which the router's radius may not clear:
  sand any nub left at the chamfer's start too. Afterwards the upper
  board's long edges, like the lower's, must measure within 11.02 mm.

Each lower and upper pair makes one module.

After breaking out, check each board's supplies against ground with an
ohmmeter (upper: +3V3, +3V3_MCU, +5V; lower: +3V3_BUF, +5V_SKT): a short
means a part cracked at a tab. The test pads `TP1`-`TP3` carry three of
them (`../DESIGN_NOTES.md`, "Test pads"). No tab sits beside a ceramic
capacitor or ferrite bead (`tools/gen_panel.py` checks it), but U6,
D2/D3 and the receptacle are beside the upper board's. Inspect C19 on the
upper board's underside: its ground pad is 0.36 mm from J4's front shell
slot, so if JLC solders J4's shell legs by hand from underneath, the iron
can reflow or lift it (ask JLC how J4's through-hole legs are soldered
when ordering). Then trim J1's
tails and check J4's legs before stacking (`../docs/07-bom.md`).
