# Design notes

Why the FastPROM stack is built the way it is: where it departs from a
reference design or datasheet application circuit, and the electrical
choices that needed a reason or a number. Each entry gives the decision,
what it departs from (if anything), and the calculation or evidence behind
it.

These are notes, not specifications. Figures marked *model* come from AGM's
Supra timing model or from datasheet maxima, not from silicon. Keep this file
current: when a choice changes, change its entry in the same commit.

Boards: `fastprom-lower` (socket side) and `fastprom-upper`
(AG32 side). The signal and power paths are described in
`docs/02-electrical.md`, the timing in `docs/03-timing-budget.md`, and the
stack in `docs/09-two-board-stack.md`.

---

## Power and power-up

### The bus buffers have their own regulator, fed from the socket only

**Decision.** U2, U3 and U4 (SN74LVT245B) run from +3V3_BUF, made by U7
(TLV74233) from socket pin 20 (+5V_SKT). They don't run from the AG32's
regulator.

**Why.** The module can run from USB with the host switched off. With the
buffers on the host's supply alone, a dead host means unpowered buffers, and
the SN74LVT245B's Ioff and power-up 3-state keep every port high-impedance
(Ioff ≤ ±100 µA at VCC = 0, VI/VO 0–4.5 V). Nothing the module does can drive
a dead host's bus or feed its rail. The board-to-board connector then carries
only 3.3 V logic plus +5V_SKT.

### D_BUF_NOE pull-up goes to the buffer's own rail (R21 → +3V3_BUF)

**Reference followed.** TI, SN74LVT245B: "To ensure the high-impedance state
during power up or power down, OE should be tied to VCC through a pullup
resistor." The power-up 3-state only holds for VCC 0–1.5 V (IOZPU, "OE = don't
care"); above 1.5 V, OE decides.

**Why not the AG32's +3V3.** With the buffers' rail coming up first (next
entry), a pull-up to the AG32's +3V3 could leave U4 fully powered with OE
still below VIH (2.0 V), driving the host's data lines from floating inputs.

**Host-off back-feed check.** With the host off and the module on USB, the
AG32 driving NOE high pushes current through R21 into the unpowered +3V3_BUF:

    I = 3.3 V / 10 kΩ = 0.33 mA

That rail is held near the dead host's 5 V by U7's body diode, a few hundred
mV at most. It powers nothing, and keeps the buffers below the 1.5 V where
they are high-impedance anyway. While the fabric drives NOE low, it sinks the
same 0.33 mA, which is nothing.

### U7 starts about 2 ms after the socket's 5 V (R26/C17 on EN)

**Deviation.** The TLV742's typical application ties EN to IN. Here EN is on
BUF_EN, R26 100 kΩ from +5V_SKT and C17 100 nF to ground.

**Why.** Both regulators start from socket 5 V, but U7 would win:

| | U7 (buffers, TLV74233) | U5 (AG32, AP2112K) |
|---|---|---|
| Input | +5V_SKT directly | behind Schottky D2 (≈0.3–0.4 V) |
| Output capacitance | ≈1.3 µF (C13, C8–C10) | ≈21 µF (C2, C14, C3–C5, C15, C16) |
| Start-up | tSTR 100 µs | 20 µs unloaded, but foldback to 50 mA into 0 V |

U5 limited to 50 mA into about 21 µF ramps at about 0.42 ms/V at first:

    dV/dt = I/C = 50 mA / 21 µF ≈ 2.4 V/ms

The AG32's I/O may clamp toward its own supply while that supply is low,
dragging NOE down. So the AG32's rail must lead the buffers'. The delay is
set by:

    τ = 100 kΩ × 100 nF = 10 ms
    t(EN = 0.9 V) = τ · ln(5 / (5 − 0.9)) = 10 ms × 0.198 ≈ 2.0 ms   (5 V step)

On a slow host ramp, EN lags further behind the input.

**Not settled.** The SN74LVT245B recommends a power-up ramp no faster than
200 µs/V, which is ≥ 0.66 ms to 3.3 V. U7 ramps in about 100 µs once
enabled. With OE held at the buffer's own VCC this should not matter, but TI
does not say so below 1.5 V. Measure on the first boards.

### FB1 isolates the whole AG32 rail, not VDDA alone

**Reference followed.** AGM recommends feeding the AGRV2K's 3.3 V rail
through its own ferrite-isolated branch rather than sharing it with other
loads.

**On the stack.** The buffers are not on this rail. The other +3V3 loads
are SW1, its resistors, and J3's 3V3 pad, where an external adapter
connects. FB1 (220 Ω at 100 MHz, MPZ1608S221A) puts VDD33 (pins 16 and 32),
VDDA33 (pin 6) and X1 behind the bead on +3V3_MCU. FB1 does not feed VDDA
alone, because AGM's guidance names the whole rail.

The bead (≈0.4 µH low-frequency) with about 6–11 µF effective gives an LC near
100 kHz with Q ≈ 3–4. That is harmless at tens of mA.

### Regulator temperatures (U5 and U7)

**Check.** Both LDOs drop 5 V to 3.3 V in small packages, and the module
sits inside the host. With no pours on the outer layers, they spread heat
only through their pins, stubs and vias.

| | U7 (TLV74233, X2SON 1x1 mm) | U5 (AP2112K, SOT-23-5) |
|---|---|---|
| Input, worst case | +5V_SKT, 5.25 V (TTL 5 V +5 %) | 5.5 V USB, less D3's ≈0.3 V: 5.2 V |
| Drop to 3.3 V | 1.95 V | 1.9 V |
| Load, worst case | ≈25 mA, taken as 50 mA | AG32 current unpublished; taken as 100 mA |
| Dissipation | 0.10 W | 0.19 W |
| RθJA, datasheet (JEDEC board) | 180.4 °C/W | 184 °C/W |
| RθJA, taken here (×1.5 for this board) | 271 °C/W | 276 °C/W |
| Rise | 26 °C | 52 °C |
| Junction at 60 °C inside the host | ≈86 °C | ≈112 °C |
| Limit | 125 °C | 125 °C (operating) |

U7's load: three SN74LVT245B at ICC ≤ 5 mA each with outputs low (TI, at
3.6 V), U4 charging eight host data lines (≈50 pF × 3.3 V × 5 MHz ≈ 0.8 mA
each, 6.6 mA), the host's input high currents (a few TTL loads per line,
≈3 mA), and R21 (0.33 mA): about 25 mA.

U5's load is the AG32 plus SW1's resistors. AGM's datasheet has a power
consumption table (section 5.3) with every current left blank, so 100 mA
is an assumption, not a figure. At 100 mA U5 is inside its limit, but
with about 13 °C to spare in a hot host.

**Must measure on the first boards:** the +3V3 current at the fastest
sys_clock the image uses, and U5's case temperature in a warm enclosure.
If the AG32 draws well over 100 mA, U5 needs copper to spread heat (a
local pour on its ground and input pins).

### USB VBUS: clamp decoupling and a hot-plug damper

**Reference followed.** ST, USBLC6-2SC6: 100 nF on VBUS (C19 at U6 pin 5).

**Added.** R27 1 Ω in series with C20 4.7 µF, from +5V_USB to ground, near
J4.

**Why.** A cable's inductance (≈1 µH) into the ceramic input capacitance
behind D3 rings at hot-plug. With about 0.3 Ω total resistance, Q ≈ 1.5, and
the peak can reach about 7–8 V, past the AP2112K's 6.5 V absolute maximum.

    R_opt ≈ √(L / C) = √(1 µH / 9.4 µF) ≈ 0.33 Ω

1 Ω errs on the overdamped side. A TVS was rejected: a 5 V-standoff part's
breakdown (≈6.4–7 V) sits right at the 6.5 V limit.

**USB limit.** USB 2.0 allows at most 10 µF on VBUS at the plug. The total is
C1 4.7 µF (behind D3) + C20 4.7 µF = 9.4 µF.

**Layout.** U6 is on top over J6, its clamp side (pins 1 and 3) facing
the AG32 and its connector side facing the receptacle. Pin 5 is the middle
pin of the connector-side row, between D+ and D−, so it can only be reached
from directly above or below. It drops through a via 1.1 mm below it, and
C19 sits underneath beside J6's left row, 1.0 mm from the via, clear of
the panel's mid-board tab ("Assembly panel"). There is no room there for a
ground via of its own, so C19's ground pad is joined straight to the
receptacle's front shell slot, 0.72 mm away, a plated hole into both
ground planes.

U6's only ground (pin 2), the middle of the clamp-side row, has two vias:
0.91 mm up on a 0.3 mm track to one between the pair, and 1.14 mm down on a
0.4 mm track to one under the body, between its pad rows.

The pair stays on top from the AG32's dog-bones to the receptacle, 0.25 mm
apart (centre to centre) down the left side and 0.4 mm apart into the pads, away from the bus
lines (those are underneath, over the other plane). The receptacle's pads
alternate D−, D+, D−, D+ (B7, A6, A7, B6): D+ comes down into A6 and is
joined to B6 under the pad row; D− comes down into A7, and A7 and B7 are
joined underneath through a via each, with a ground via 0.57 mm below each.

R27 and C20 are underneath, under the receptacle, fed through a via from
the A9/B4 VBUS pad; the two VBUS pads and the clamp's via are joined
through vias underneath.
C20 has its own ground via.

### C18: 100 nF at socket pin 20

**Why.** The data lines' return current shares J1 pin 10 (below). Without a
capacitor at pin 20, the nearest decoupling to the socket supply would be
C12 at U7, about 22 mm of track away, useless as an AC return. A 100 nF at the pin
lets the host's own socket decoupling share the return.

**Layout.** C18 is on top at the board's pin 1 end, right of the pin 1
notch; the buffers' regulator and its capacitors take the rest of that
end.
- Supply: a 0.3 mm track, 1.66 mm, to pin 20.
- Ground: a 0.3 mm track, 0.84 mm, to a via of its own into U2's exposed
  pad underneath and the planes.

### C10, U4's decoupling: 1 µF, not 100 nF

**Why.** U4 switches all eight data lines into the host at once.

    charge per edge ≈ 8 lines × C_load × V = 8 × 50 pF × 3 V ≈ 1.2 nC
                     (1–1.5 nC across the load range)
    ΔV on 100 nF ≈ 1.2 nC / 100 nF ≈ 12 mV;  on 1 µF ≈ 1.2 mV

The concern is U4's long supply run (about 27 mm from U7, ≈17 nH) and the
100 nF/17 nH resonance near 4 MHz, not the DC value. C10 pairs the larger
capacitor with a short loop to U4's own supply and ground pins (layout).

**Layout.** U4's supply pin 20 and ground pin 1 sit at its left end,
against the DIP strip's pin column, so on top there is no room for a
capacitor there. C10 is underneath, under U4's left end; pins 20 and 1
each drop through a via of their own just outside U4's end, 1.5 mm apart,
under the strip's insulator, where vias may go but parts may not.
- Supply: 0.71 mm on top and 0.67 mm underneath to C10's pad.
- Ground: 0.71 mm on top and 0.88 mm underneath.
- Path round the loop: about 5 mm, with supply and return 1.5 mm apart at
  most, two vias included.

U4's own ground pin is pin 10, at the other end; its return reaches C10
through the planes from pin 10's via. U4's exposed pad has no vias: the
A lines to the terminator packs run under it on the underside. It is
grounded on top through pins 1 and 10.

---

## Bus interface

### SN74LVT245B, not LVTH

The LVT and LVTH parts have identical switching characteristics; LVTH adds
bus hold. Bus hold takes 500-750 uA of overdrive to switch (II(hold) in the
SN74LVTH245A datasheet), and the 100 kOhm socket pulls ("Socket pulls")
supply 33-50 uA, so with LVTH buffers a floating address or enable pin would
hold its last level instead of reading as the pull sets it.

### 33 Ω series terminators (RN1, RN2); 47 Ω optional

**Model.** Source termination for the host's trace. The on-module path is
lumped: the driver-to-pin length is 8–19 mm, about 125 ps at 6.5 ps/mm,
against LVT edges of 1 ns or more (critical length about 25 mm).

**Source impedance.** 33 Ω + LVT output impedance (about 8 Ω low, 15 Ω high)
= 41–48 Ω into 70–100 Ω vintage traces:
- falling-edge ringback peaks at about +0.2 to +0.45 V, below the 0.8 V VIL;
- rising-edge first reflection settles near 3.1 V, above the 2.0 V VIH.

**Timing cost**, at the assumed 50 pF host load:

    33 Ω × 50 pF × ln 2 = 1.14 ns
    47 Ω × 50 pF × ln 2 = 1.63 ns   (+0.5 ns)

47 Ω (4D02WGJ0470TCE, LCSC C55183, same footprint) damps ground bounce
harder, but eats most of the fastest 512x8 tEA margin. It is for builds
targeting 45 ns parts and slower only.

### Socket pulls

- Pull-downs, 100 kΩ, on the high address pins: a floating or grounded
  address pin reads 0.
- Pull-up, 100 kΩ to +5V_SKT, on nCE0: a floating enable reads as
  deselected.
- Pin 16 is A5 on the 512x8 and a second enable, nCE1, on the 256x8. Both
  pads are laid out (R16 down, R20 up), and exactly one is fitted.
- The pulls reference the host's 5 V, not the module's 3.3 V, since the
  module may be on USB.

### Socket nets named by function, in the 512x8 pinout

The socket's signal nets are named for what they carry on the primary part,
the 512x8 (`tools/partdb.yaml`): `A0`–`A8`, `D0`–`D7` and `nCE0` on the
socket's 5 V side, and the same with a `BUF_` prefix for each line's
buffered 3.3 V side (`BUF_A0`, …, `BUF_nCE0`), which is what crosses the
stack connector. Names by pin number (`SKT_1`, `BUF_1`) are not used.
A pin's meaning does depend on the part emulated, but the board commits
each pin to a direction in copper (the receive and drive buffers), so a pin
number would hide fixed information: which lines are addresses, which are
data, which is the enable.

The 256x8 differs from the 512x8 only on pins 16–19: pin 16 is nCE1 and
17–19 are A5–A7. So on a 256x8 socket net `A5` carries nCE1, `A6` carries
A5, `A7` carries A6, and `A8` carries A7. Both sheets say so in a table of
the pins that differ, generated from partdb (`pin_table()` in
`tools/gen_kicad.py`): below J1 on the lower sheet, beside a table of what
to fit for each part (R16 or R20), and below J6, where the buffered lines
arrive, on the upper. The sheets do not explain what each pull does; that
is here. `nCE0` is spelled as partdb and the firmware spell it, not with
KiCad's overbar markup, so DSN, Freerouting and the routing files handle it
as a plain name.

### Rail symbols end their nets; schematic wires plot

A power symbol goes at the end of the net it names, as drawn by hand, so
it is easy to find. J1's pin 20 wire ends at the `+5V_SKT` symbol and C18
taps it halfway. `tools/grade_schematic.py` makes anything else a hard
defect (`rail_mid_net`): every rail symbol ends exactly one wire, or sits
on a pin with no wire.

The schematic's wire width, bus width and line style live in the net
classes of the project file the boards share. The default net class that
`gen_layout.py` configures through pcbnew (`GetDefaultNetclass()`) carries
none of them, and saved as it is it has width 0: its wires plot as
hairlines with no junction dots, and with no stroke in an SVG export. So
`gen_layout.py` gives every class the values a fresh `NETCLASS` has (6 mil
wires, 12 mil buses, solid).

### One SN74LVT245B symbol for all three buffers

One library symbol, `SN74LVT245B`, serves U2, U3 and U4 unchanged, so
they read as the same part and A and B line up on each: A1–A8 down the
left and B1–B8 down the right, row for row; DIR and ~OE in their own group
on the left below A8, two rows clear of it (room for the rail symbol on a
receiver's DIR between it and the A6–A8 tie) and a row apart (adjacent,
~OE's overbar reads as DIR underlined); VCC on top, GND and the thermal pad
below. Pin numbers are TI's (1 DIR, 2–9 A1–A8, 10 GND, 11–18 B8–B1,
19 ~OE, 20 VCC, 21 pad).

The controls are on the A side, so on U4, which drives B to A, they sit
on its output side. That is the accepted cost of one symbol: A has to face
J1 on every part, and mirroring U4 would put its A pins away from the
socket. D_BUF_NOE runs from ~OE under U4 to R21, and joins J5 by label
(below).

The ports are typed bidirectional, as the part is. A receiver's unused A6–A8
are tied to ground, and a bidirectional pin on a net with a power output
(J1 pin 10) is an ERC warning: KiCad cannot tell an input tied low from an
I/O shorted to a rail. Each port therefore carries KiCad alternate functions,
`An_IN` (input) and `An_OUT` (3-state output), and U2/U3 select `A6_IN`–
`A8_IN` on the tied pins, so ERC checks them as the inputs they are. The
sheet shows those three pins by that name.

RN1 and RN2 are stepped rather than stacked: each is level with the four
U4 channels it serves, RN1 beside U4 and RN2 further out, RN2's outputs
passing under RN1 and RN1's inputs over RN2, so every line between a pack
and U4 is straight. A pack's body ends half a pitch past its end elements
for that. D4–D7 run level into RN2.

### Connectors drawn as the parts are; labels only at their contacts

Each connector is drawn as it physically is, so the drawing looks like
the part and shows which contact sits where, and keeps its pin names and
types:

- J1, the DIP-20 plug: pins 1–10 down the left, 20–11 down the right,
  pin 20 opposite pin 1, seen from the top (footprint: pin 1 at the top
  left, 10 bottom left, 11 bottom right, 20 top right).
- J5 and J6, the DF40 pair: two rows of 15, in the order the pads sit on
  the boards (read from both footprints as placed, with pcbnew). J5 is on
  the lower board's top and J6 on the upper board's underside, at the same
  x, y and both turned 270°; seen from above, the odd contacts are the
  right row and the even the left, 1 and 2 at the socket's pin 1 end, on
  both. So both symbols show the same view - from the top of their own
  board, J6's looking through the upper board - with 2–30 down the left
  and 1–29 down the right, and since the halves mate face to face, contact
  n of J6 lands on contact n of J5: the two drawings read row for row
  against each other. Each symbol's description says this too.
- J3, the SOICbite: 1-4 down the left and 8-5 down the right, as the clip
  sees it (1 3V3, 2 BOOT0, 4 GND, 5 RX, 6 TX, 8 NRST).
- J4, the USB-C receptacle: its pads in footprint order, left to right seen
  from the top, the A and B rows interleaved as HRO's land pattern puts
  them (A1/B12, A4/B9, B8, A5, B7, A6, A7, B6, A8, B5, A9/B4, A12/B1, then
  the shell).

In that order the contacts do not line up with what they connect to, so
where wiring them would tangle, a contact ends in a short stub carrying a
net label, and the wiring it belongs to carries the same label on a stub
end of its own: J1's signal pins (the left column faces away from the
buffers, the right column counts up against them), every J5 and J6
signal (they come in the lower board's copper order), and J3's four
(they come in the clip's order; the AG32's lines run on past the image
select and reset blocks to stub ends carrying the same labels). A connector's
grounds and supplies are tied per rail by a vertical close to the pins
into one upright rail symbol, as the CM4 IO board draws its module.

Where a connector's contacts can be drawn to what they serve, they are,
with no labels: J4 meets the
ESD clamp on straight wires, D+ into A6 and D- into A7, level with the
clamp's pins. The flipped plug's pair, B6 and B7, ties onto those lines
by short verticals, each crossing the other line (USB-C alternates D-,
D+, D-, D+, so two crossings are unavoidable), and the CC pads run out to
drops to their 5k1 pull-downs in a row below J4, as the socket pulls are
drawn on the lower sheet. Where each connector sits on its sheet is set
out in docs/13-schematic-conventions.md.

### Lower sheet: address and enable apart from data

With J1 and J5 joined to the buffers by labels, the buffers need not follow
the socket's pin order (U2, U4, U3, which would put the data driver between
the two receivers). The lower sheet is drawn as two groups, one above the
other, each reading host on the left to AG32 on the right:
"Address and enable: host to AG32" - U2 (A0-A4) above U3 (nCE0, A5-A8) with
the pulls on U3's socket lines (R15, R20, R16, R12-R14) - and below a clear
gap, "Data: AG32 to host" - U4 with RN1/RN2 and its ~OE run to R21. Where
J1, the tables, J5 and the buffer supply sit around them, and how each
group is framed, is in docs/13-schematic-conventions.md.

`tools/grade_schematic.py` allows exactly this and nothing more. A label
name on more than one wire island (`label_joins`) is a hard defect,
unless every island carrying it but one is a connector stub: an island
whose only pin is one pin of a J-reference symbol. A label on connector
stubs only, with no wiring carrying the name, is a hard defect
(`dangling_labels`), so every connector label lands somewhere. Everywhere
else labels only name wires.

### Ground bounce through the single socket ground pin (must measure)

All eight data lines return through J1 pin 10: about 8 mm of 0.5 mm-diameter
conductor (machined pin plus socket contact). Self-inductance:

    L ≈ 0.2 · l · [ln(2l/r) − 0.75] nH  (l, r in mm)
      = 0.2 × 8 × (ln 64 − 0.75) ≈ 5.5 nH
    minus mutual inductance to pin 9 (≈1.35 nH) → ≈4.5 nH effective

Launch current per line ≈ 3 V / (≈45 Ω + ≈85 Ω) ≈ 23 mA, so 0.18 A for all
eight. LVT edges of about 1–1.5 ns give di/dt ≈ 0.12–0.18 A/ns:

    V_bounce = L · di/dt ≈ 4.5 nH × 0.12–0.18 A/ns ≈ 0.55–0.8 V, for ~1 ns

For comparison, a 74S472 in its socket sees about 3 nH with 2–3 ns edges,
≈0.3–0.4 V.

**Risk.** It lands at the enable event, when /CE must stay low. A host
"low" at VOL 0.4–0.5 V could briefly look like 1.1–1.3 V at the receivers,
near their switching threshold. This can't be fixed in copper. C18 and C10
help.

**Test.** Measure with an alternating 0x00/0xFF image on the worst host,
J1 pin 10 against host ground, and /CE at U3's input.

---

## Board to board

### Hirose DF40, 3.0 mm stack

The DIP pins solder on the lower board's top, leaving 20 solder fillets
there, and parts on both facing sides share the gap. So stack height has a
floor: the 0.6 mm BM28 and 0.9 mm DF30 don't clear. 3.0 mm is the DF40
height LCSC stocks (C531034); the 2.0 mm version isn't on LCSC. The full
survey is in `docs/09-two-board-stack.md`.

### Contact allocation

30 contacts:
- 18 buffered signals;
- D_BUF_NOE;
- 2 × +5V_SKT;
- 9 grounds, with a ground beside every pair of signals.

Mated-contact estimates:
- a mated contact ≈ 2.3 nH; nine grounds ≈ 0.4–0.6 nH effective;
- the AG32 switching all 8 data lines at once (8 × ~8 pF × 3.3 V/ns ≈
  0.2 A/ns) gives about 0.1 V of bounce against LVT thresholds of 0.8/2.0 V.

Both halves are generated from one function, and `make stack` checks them.

**Ground placement.** The grounds are contacts 6, 12, 16, 18, 20 and 26
in the left row and 9, 23 and 25 in the right (`stack_contacts()` in
`tools/gen_kicad.py`). Every signal is within three positions of a ground
in its own row; contact 29 (socket 12, right row k14) is two from one.
Socket 14 is on contact 17 (right row k8) and the right row's data-end
ground on 25 (k12), beside the one at 23, because:
- On the lower board the far-end contacts (k13–k14 in both rows) are fed
  straight up from U4, through the lane or beside the rows. A ground there
  has no via site: every lane line, the mounting pads and the terminator
  pack RN2 underneath surround it.
- Socket 14's line is already the outermost on the right, so it just runs on
  up to k8.
- The two right-row grounds at k11 and k12 share one via in the lane,
  between their pads.
- On the upper board socket 14 is the autorouter's, and k12's ground takes a
  lane via like the others.

### Pin-tail trim: 1.0 mm target, 1.5 mm limit

The strips' 4.17 mm solder tails stand 3.37 mm proud of the 0.8 mm board,
more than the 3.0 mm gap, so they must be trimmed. Two things on the upper
board's underside sit over the tails:

- **0603s beside pin 2** (≤0.9 mm): C1, U5's input capacitor, below FB1
  at the pin 1 end, where the regulator's parts fill the underside's
  width. That leaves 2.1 mm above the lower board. C1's pads are beside
  pin 2's tail rather than over it (0.04 mm from the tail's edge, seen
  from above), so the risk there is a bent or tall tail, not the
  stack-up. C2's body ends 0.3 mm short of pin 20's tail.
- **J4's shell legs over pins 9-12.** The USB-C receptacle's through-hole
  shell legs (slots at x 102.675 and 111.325) lie over pins 9 (D3) and 12
  (D5), and its rear legs 1.2 mm from pins 10 and 11. Per HRO's model the
  legs reach about 0.24 mm below the upper board, plus any solder on the
  underside, leaving about 1.0-1.2 mm above a 1.5 mm tail. The shell is
  grounded, so a tail left long shorts D3 or D5 to ground. The legs must be
  soldered flush underneath, with no solder spikes.

A freehand cut scatters by roughly ±0.3 mm, so the target is 1.0 mm, set
with a gauge or shim, and 1.5 mm is the limit. If the tallest part or leg
over a pin row changes, redo this.

### J3 at the pin 1 end: the upper board 4 mm longer

**Decision.** J3, the SOICbite, sits at the upper board's pin 1 end edge,
on 4 mm of board added past the lower board's end (y −4.0 to 0 in
`gen_layout.py`'s frame, `PROG_EXTRA`): the upper board is 11.02 ×
31.76 mm.

**The footprint at the edge.** SOICbite's Fab line marks the board edge:
the pads start 0.381 mm in (the copper-to-edge rule is 0.30 mm) and run
1.5 mm, 1-4 on top and 8-5 under them, pin 1 on the left; the clip's five
holes are 2.29 mm in, and the courtyard, the strip a clip's jaws and comb
take, ends 2.92 mm in, at y −1.08.

**Why the board grows.** A clip's jaws need the edge strip clear on both
faces, and more than the courtyard: the jaw's wall stands behind its comb.
Without the added 4 mm neither face has it:
- underneath, the regulator's parts (U5, C1, C2, FB1) and the switch and
  UART resistors fill the pin 1 end, with the AG32 right behind them
  (its courtyard from y 6.36). U5's courtyard alone is 3.49 mm across
  its narrow side: 2.92 + 3.49 = 6.41 mm, past the AG32's, with no jaw
  room at all. They have nowhere else to go: the strips beside the AG32
  and J6 carry its bundles, and under the receptacle the CC resistors
  and the VBUS damper leave little more;
- on top, SW1 (courtyard 4.77 mm deep) would sit 2 mm from the edge with
  the AG32's decoupling (C5 at pin 32's via, R10, X1) right behind it;
  moving SW1 3 mm in would move the decoupling off its vias, and the rest
  of the top, to the receptacle, is full.

**Clearances, beyond J3's courtyard** (courtyard to courtyard):
- **top:** 3.10 mm to SW1 (its body 3.46 mm), across the board; 1.10 mm
  to R24 and R25, two 0402s (0.35 mm high) between the strip and SW1, at
  x 2.24-4.37, by the left edge;
- **underneath:** 1.58 mm to U5 (its body 2.23 mm), 2.16 mm to C2 and
  FB1 (0603s) and 2.17 mm to R18 (an 0402), nothing else in between.
Tracks, vias (filled and capped, so flat) and the silk `PROG` are in the
strip; a jaw rests on the solder mask.

**Stacked.** The courtyard ends 1.08 mm short of the lower board's end,
so a clip goes on with the module stacked, and seated in the host: its
lower jaw comes up under the overhang, never into the 3 mm between the
boards. Below the overhang is the host, about 10 mm down (a neighbouring
seated DIP's top about 7.4 mm), which sets the room for the lower jaw.

**What it costs.** The upper board overhangs the socket's pin 1 end by
4 mm more (5.7 mm past the pin 1 row) at about 10 mm above the host, as
it overhangs the USB end by 1.5 mm: above a neighbouring DIP, not beside
it. The panel is 4 mm longer for it, and the pin 1 end needs tabs of its own on the
long sides to stay flat for the paste print ("Assembly panel").

**Routing.** Seen from above, J3's pads run 3V3, BOOT0, -, GND on top and
NRST, -, TX, RX underneath, while 3V3 and NRST are wanted at the right
(U5's output, the NRST network) and TX and RX at the left (the AG32's
UART edge, the switch's resistors), and BOOT0 runs straight down the
middle on top to the AG32. Each line leaves its pad straight in between
two holes (3V3 necked to 0.15 mm for the 0.57 mm gap). TX and RX turn
left underneath just past the holes; NRST leaves its pad sideways,
outside the hole row, to a via, comes down on top outside 3V3's pad and
back underneath below TX and RX, and runs right; 3V3 comes down on top
past TX and RX and NRST to a via, and runs right underneath into U5's
output, and down from the via into the ferrite FB1 ("Assembly panel").
TX and RX come up through vias into R24 and R25, which are on top for
room to do this; from the same vias they go on down underneath into their
pull-downs R18 and R17 and past C1 to the AG32 (`tools/layout/fans.py`,
`upper_prog`).

---

## AG32

### QFN-32, not LQFP-48

The 24 fabric I/O are exactly enough:
- 18 socket lines;
- the data-buffer enable;
- the 12 MHz HSE;
- USB D+/D−;
- UART TX/RX, which also read the two image-select bits.

The LQFP-48's 9 × 9 mm lead span would take most of an 11.02 mm board. The
cost is the toolchain: the open flow can't place the QFN-32, so builds use
AGM's Supra native flow (`docs/06-build-and-flash.md`).

### D_BUF_NOE on PIN_14, not on a pull-up pin

AGM's pinout spreadsheet shows PIN_26 and PIN_28 come up with an internal
pull-up before configuration. The enable is on PIN_14 instead, next to the
chip enable (PIN_9), because of timing (*model*):

| Enable pin | Fabric enable delay | 512x8 tEA vs 20 ns |
|---|---|---|
| PIN_18, far side of the die | 9.5–9.9 ns | −0.9 ns |
| PIN_28, pull-up pin | 8.6 ns | marginal |
| PIN_14, next to PIN_9 | 7.1–7.9 ns | +1.2 to +1.8 ns |

PIN_14 floats before configuration, and R21 holds it high. With the enable
there, socket 7 is on PIN_18 and socket 8 on PIN_15. That costs those bits
1–2.4 ns of tAA, but every tAA keeps at least 4.2 ns of margin.

### BOOT1 strap: 4.7 kΩ (R11), on a data line

PIN_15 is BOOT1, sampled at reset, and must read 0 for the UART loader. The
QFN-32 has no I/O to spare, so it carries a data line, which the AG32 drives
after configuration. A receive line would not do: the receive buffers drive
theirs from power-up and would override the strap. The strap must win
against the pin's internal 30–50 kΩ pull-up:

    10 kΩ:  3.3 × 10 / (10 + 30..50) = 0.55–0.83 V   (on the 0.8 V VIL limit)
    4.7 kΩ: 3.3 × 4.7 / (4.7 + 30..50) = 0.28–0.45 V

**Layout.** The line runs in the left-row bundle underneath, boxed in by
its neighbours, so R11 sits on top over J6's lane, on a via from contact
k 10's inner end: a 1.3 mm stub on the line, with R11's ground via beside
it.

### Image select on the UART pins

Two image bits are read through the DIP switch on UART0 RX/TX: switch 1
(pins 1-4, on RX) is bit 0 and switch 2 (pins 2-3, on TX) bit 1, so the
numbers printed on the switch name the bits and switch 1 alone selects
image 1. The pins are ordinary fabric I/O unless BOOT0 selects the
loader:
- each pin has a 100 kΩ pull-down, so a bit reads 0;
- a closed switch adds 10 kΩ to 3V3: 3.3 × 100 / 110 = 3.0 V, so a bit
  reads 1.

UART idles high, so the pull-up is invisible to the loader and to an adapter
whatever the switches are set to.

### 12 MHz oscillator into OSC_IN

USB needs a clock from the PLL with USB's accuracy: 60 MHz on PLL output 1,
which AGM's `gen_vlog` assigns whenever `USB0` is enabled (USB 2.0 full
speed allows ±0.25 %). The PLL multiplies the external HSE, and the
internal RC has no stated accuracy. A 12 MHz CMOS oscillator (X1) feeds
OSC_IN; the VCO runs at 1200 MHz (x100), which gives 100 MHz SYSCLK, 60 MHz
USB, 200 MHz for BRAM mode's read port and 50 MHz BUSCLK, all exact
divisions.

### Programming over UART only, no JTAG

AGM's UART0 mask-ROM loader writes the fabric bitstream as well as the
firmware, so JTAG isn't brought out. J3 carries 3V3, BOOT0, GND, RX, TX and
NRST on a SOICbite (Simon Merrett's footprint, BSD-2-Clause): eight pads on
both faces of the upper board's pin 1 end edge, which a SOIC-8 test clip
or a printed pogo clip grips without being held. Its pinout is SOICbite's
ESP8266 one (pin 2 the boot strap, pin 8 reset), so the fixture holds BOOT0
high and clipping on selects the loader. The update page drives the same
pins over Web Serial ("UART recovery", below), so an end user needs the
fixture, a 3.3 V adapter and a browser, not AGM's SDK.

**Powering a bare board through J3's 3V3 (pin 1).** The fixture drives
+3V3, U5's output, with no 5 V present. U5 (AP2112K) is a CMOS LDO whose
pass transistor has a body diode from output to input, so +3V3 back-feeds
the +5V node to about 2.7 V. Nothing is on that node but C1 and U5's own
enable: D2 and D3 are reverse-biased, so nothing reaches the socket or USB,
and the current only charges C1. Diodes does not specify the AP2112's
reverse current; with the input side blocked like this, LDO makers' notes
(Rohm's on reverse current) treat it as harmless. With 5 V present as well,
U5 and the fixture both drive 3.3 V; neither sinks, so the higher supplies.
The lower board's +3V3_BUF is its own regulator's and is not involved. The
limit is current: the fixture supplies the AG32 (not published; up to
100 mA assumed in "Regulator temperatures"), more than many USB-UART
adapters' 3.3 V outputs give, so the fixture needs its own 150 mA or better
regulator, or the board is powered over USB-C and pin 1 used as the
reference.

---

## Firmware

The firmware (`fw/`) is pure Rust: no vendor C is linked. Where it departs
from what AGM's AgRV_pio SDK would build, the entries below say so and why.
The memory map is `fw/fastprom-core/src/memmap.rs`, with its sources;
`docs/06-build-and-flash.md` has the build, `docs/10-updates.md` the USB
update design.

### riscv-rt startup, not the SDK's crt.S and syscalls.c

The SDK's FLASH boot mode (`misc/devices/AgRV2K_FLASH.ld`, `misc/mem.ld`,
`misc/crt.S`, `misc/syscalls.c`) runs code from flash, copies `.data`
**and `.rodata`** into SRAM, zeroes `.bss`, then calls `main`.
riscv-rt 0.18 (features `memory`, `single-hart`) does the same job with
three differences:
- `.rodata` stays in flash. The SDK's own flash example reads flash with
  plain loads (`examples/example/src/example_flash.c`), and nothing here is
  read often enough for SRAM to matter. The image table is read from flash
  in place too.
- There is nothing in `.data`. `.bss` is 8 KiB, the USB controller's
  4 KiB-aligned DMA memory (`docs/11-usb-driver.md`, "Memory"); the rest of
  SRAM is stack, less the 260 bytes the flash routines are copied to once
  flash is written ("Flash routines run from SRAM", below).
- Unwind tables (`.eh_frame`, 2.9 KiB) are discarded: the target aborts on
  panic, and no panic path is linked anyway (`tools/fw_layout.py` fails the
  build if one appears).

The panic handler and the exception handler both clear READY if the fabric
answers with FastPROM's ID, then park. Neither should ever run, but neither
may leave the socket driven.

### Flash map: a DFU stage, the application, the fabric, its manifest, the table

`docs/10-updates.md` ("Flash map") is the design; the sizes:

- **DFU stage: 0x80000000, 32 KiB.** The reset vector, so it is the one
  thing USB never rewrites (the AG32 has no USB boot loader in ROM). It
  uses 19.9 KiB (62 %, `make fw-build`): the update core, the USB driver
  and descriptors, and its own USB-only fabric, 3,226 bytes compressed,
  with the decoder (4.6 KiB together; below).
- **Application: 0x80008000, 48 KiB.** 256-byte header, then the code. It
  uses 33.5 KiB with the update core, the USB driver and the CDC log port
  (`make fw-build`), leaving 14.5 KiB. 0x80008000 is also AGM's own
  `DFU_APP_ADDRESS` (`examples/dfu/platformio.ini`, `env:usb_app`).
- **Fabric: 0x80014000, 100 KiB.** The SDK's own reserve for an
  uncompressed bitstream (99,944 bytes, `builder/common.py`), above
  Supra's default 0x80007000 to leave room below; the address is the
  option byte's, which `make fw-batch` writes (gen_batch `--logic-address`).
- **Manifest: 0x8002D000, one 4 KiB sector.** Its own sector, so that
  erasing it to invalidate the fabric touches nothing else.
- **Image table: 0x8002E000 to the end of flash, 72 KiB.** The table needs
  at most 16,640 bytes (four 4096-word images). The SDK has no equivalent;
  a separate region means changing an image is `make fw-flash-table` or a
  USB write from the application, not a firmware rebuild. Erased flash
  reads 0xFF and fails the magic check, so a module with no table fails
  safe and says so.

The regions tile flash in order with no gaps, every one on a 4 KiB sector
boundary, so erasing one never erases another; `memmap.rs` asserts it at
compile time, for the firmware and for both linker scripts.

### The DFU stage's boot check: 38 instructions, no CRC

At the reset vector, before riscv-rt's startup (`fastprom-dfu`'s
`boot.rs`), in assembly: after a power-on reset clear the reset flags and
`mscratch`; read and clear `mscratch`; if it is not the DFU request, and
the application header's commit word, version and length are sane, and so
are the fabric manifest's, jump to the application's `_start`; otherwise
DFU mode. The application then starts as if the mask ROM had jumped to it,
so its boot sequence (switch, CRC, load, READY) is its own and the DFU
stage's whole cost is this check.

**Why the manifest too.** The application's USB is the fabric's to enable
(`USB0 device` in the `.ve`). A fabric update erases the manifest first,
then the bitstream, and commits the manifest last. Cut part way, it leaves
a committed application over a bitstream the FCB cannot configure, and a
check of the header alone would jump to it, with no USB and only UART
recovery to get back. With the manifest in the check, any board without a committed
fabric stays in DFU mode, on the stage's own USB-only fabric, where the
page finishes the update (`fastprom-core`'s `tests/update.rs` cuts power
at every flash operation of a fabric update and finds DFU mode after each;
`fastprom-host` and the page then finish it over USB). The same rule puts
a board recovered with the DFU stage alone (no fabric in the bundle, so no
application and no manifest) in DFU mode, as recovery means it to.

- **38 instructions on a soft reset, 42 after a power-on reset** (the
  extra four clear the flags and `mscratch`; the manifest's half is 12,
  the header's 13), 160 bytes: three 64-byte cache lines (the SDK aligns
  its cache-sensitive code to 64 bytes), plus one line each for the three
  header words and the three manifest words, plus RST_CNTL's read and
  write. The application's first line, the sixth miss, is paid with or
  without the check.
- At HSI (10 MHz nominal) with every fetch missing the cache, a line fill
  from the in-package SPI NOR is about 150 flash clocks in quad mode (24
  for command, address and dummy, 128 for 64 bytes) or about 550 in single
  mode, so five fills are 75 to 275 µs. The hardware has just read the
  99,944-byte bitstream from the same flash, about 1,560 lines' worth,
  before the CPU ran: the check adds about 0.3 % to the configuration
  window `06-build-and-flash.md` asks to be measured.
- **No CRC here.** A CRC of the 33.5 KiB application at 10 MHz is about
  12 instructions a byte with the nibble table, so 410,000 instructions,
  40 ms or more: longer than anything else before READY. It is not needed:
  the payload's CRC is checked when it is committed, and the commit rules
  (below) never leave a committed header over a payload that changed, so a
  sane header is a checked one. Bit rot in flash after the fact is the
  one thing this misses; the application's own CRC check of the image it
  serves, which does run at boot, is unaffected.
- It runs before riscv-rt sets `mtvec`, so it does nothing that can trap:
  aligned loads from flash, RST_CNTL, two CSR accesses.
  `header::boot_application` (`boot_sane` and `manifest_sane`) is the same
  check in Rust, host-tested, and the simulator's boot; the assembly takes its constants
  from the same crate and `boot.rs` asserts their shape at compile time.

`mscratch` across a soft reset is AGM's pattern (`examples/dfu/src/dfu.c`):
the DFU stage clears it after a power-on reset (POR flag in RST_CNTL,
system.h `SYS_RSTF_POR`) and clears the flags (`SYS_RST_REMOVE`) so that
the sticky POR flag does not wipe a later request. Unverified on silicon:
that `mscratch` survives a soft reset (AGM's DFU depends on it) and that
the soft reset reconfigures the fabric (below: the SDK's evidence says it
does).

### The DFU stage's own fabric: USB0 and nothing else

USB is a hard block, but the fabric configuration enables it and clocks it
(`USB0 device` in the `.ve`: gen_vlog's USB enable and PLL output 1 at
60 MHz). A DFU stage that ran on whatever fabric flash holds would lose USB
with a bad fabric update, the one failure it exists to recover from. So DFU
mode first loads a configuration the stage carries, as AGM's DFU does
(`examples/dfu`, `DFU_FPGA_CONFIG=fpga_usb.inc`):

- **What is in it:** the `.ve` is `SYSCLK 100`, `BUSCLK 50`, `HSECLK 12`,
  `USB0 device` and no pin line (`tools/gen_pins.py --dfu-ve`; the clock
  lines are the ROM fabric's, from the same function, because the PLL is
  part of the configuration and `fastprom-hal` assumes 100 MHz from it).
  Built with AGM's flow for a design without custom logic
  (`tools/ag32/build_dfu_fabric.sh`), plus the USB pad pull bits every AGM
  board file sets with USB0 (`tools/ag32/usb0.asf`).
- **Why no pin can be driven:** with no port in the design there is nothing
  for Supra to place on an arbitrary pin. Supra's own result shows it:
  three ports, PIN_HSE, PIN_HSI, PIN_OSC, all inputs (the oscillator input
  and the two internal oscillators), three pad cells with output enable
  and data tied low, "Total Pins : 3/ 128", and no other pad cell in the
  routed netlist (`fw/fastprom-dfu/fabric/dfu_usb.evidence.txt`, from
  `tools/ag32/check_dfu_fabric.py`, which fails the build otherwise). D+
  and D- are the USB block's own pads. Every other pin is as before
  configuration: undriven. PIN_14 (`buf_noe`) floats and R21 holds it high,
  so U2-U4 are off; the socket's data pins are high-impedance behind them,
  and the AG32's own data pins are not driven either. `release_socket()`
  clears READY first, so the ROM fabric already has the buffers off when
  the configuration changes; during the change the pins float, as at
  power-up, with the same R21 behind them.
- **Size:** 3,226 bytes compressed (Supra with
  `ON_CHIP_BITSTREAM_DECOMPRESSION ON`, which AgRV_pio's
  `board_logic.compress` sets) against the 99,944 bytes of every AGRV2K
  bitstream, 3.2 %. AGM's own is 3,586. The decoder (`fastprom-core`'s
  `fcb.rs`, fcb.c's LZW with every index bounded so no panic path links)
  and the FCB sequence add about 1.4 KiB of code to the DFU stage, which
  uses 19.9 KiB, 62 % of its 32 KiB. The decoder's tables, 4 KiB, are
  on the stack (120 KiB) only while it runs. Uncompressed it would not fit
  the stage at all.
- **Cost:** only on entering DFU mode, never at power-up into the
  application: 24,986 words into the FCB, each from an LZW decode of about
  four bytes. At roughly 30 instructions a byte that is some 3 million
  instructions, a few hundred milliseconds on HSI (*estimate*, not
  measured; AGM raises HSI to its fastest setting for it, and so do we),
  before USB starts: the host waits that much longer for enumeration.
- **The clock handling around it** is AGM's `dfu.c`, step for step
  (`fastprom_hal::fcb::configure_compressed`): HSI to its fastest
  (MISC_CNTL 0xF), the core onto HSI with the PLL and HSE off (the
  bitstream reconfigures the PLL, so the core must not be running from
  it), every APB clock off but the FCB's ("to avoid any noise from FPGA"),
  AUTO mode, the words, CTRL back to 0, status (ACTIVE and no error bit),
  then APB gates and HSI setting restored. The PLL is started after, from
  the configuration just loaded.
- **Committed, with a check:** rebuilding the bitstream needs AGM's SDK,
  which CI does not have, so `fw/fastprom-dfu/fabric/` holds it, the `.ve`
  it came from, a manifest (length and CRC-32 of the compressed and the
  plain bitstream, CRC-32 of the Supra inputs) and the pin evidence.
  `fastprom-dfu`'s build.rs refuses to build if the file is not the one the
  manifest records or the inputs in `tools/ag32/` have changed since;
  `fastprom-core`'s `tests/fcb.rs` expands it and checks the plain
  bitstream's length and CRC. A rebuild is not byte-identical (Supra's
  router alternates between two equivalent clock routings for the same
  inputs), so the check is on the inputs, not the bytes.

Leaving DFU mode is a soft reset with `mscratch` cleared, and the fabric
must come back from flash. The SDK says a reset does that: RST_CNTL bit 2
is `SYS_RST_FCB_DIS`, "FCB reset disable" (system.h, with
`SYS_DisableFpgaReset`/`SYS_EnableFpgaReset`), so by default a reset resets
the FCB, which configures the fabric from the option bytes' address as at
power-on; and AGM's own flows depend on it. `env:usb_app` loads its USB
fabric at every power-on and soft-resets before jumping to the application,
"since the FPGA configuration is changed. Use a reset to restore it"
(`dfu.c`): otherwise every application behind it would run on that USB-only
fabric. And its serial booter's reset command, the one its flasher's `-R`
("Reset device at exit") ends a logic download with, is `SYS_SoftwareReset`
(`examples/dfu/src/boot.c`). AGM has no datasheet text on it. Nothing in
this firmware sets FCB_DIS, and `reset::reboot` clears it with the reset
bit. If the first boards show
otherwise, the application's first step fails safe: it finds no 'FPRM' ID,
reports E50 and never sets READY; the fix would then be an explicit
`FCB_AutoConfig` of the flash bitstream before the reset (AGM's
`example_fcb.c` does exactly that from the option bytes' address).

### The commit word: valid last, invalid first

The application header and the fabric manifest each begin with a commit
word that only `COMMIT` programs to the magic, after the payload's CRC-32
matches the page; writes of the page over USB hold it at erased. NOR flash
programs by clearing bits, so programming the magic over erased is always
possible, and a program cut short leaves bits still set: not the magic.

Erasing is the other half. An interrupted sector erase can leave cells
anywhere between old and erased, so erasing a committed header's sector
could leave the old magic over a half-erased payload. So the update core
first programs the commit word to zero, then erases: zero never needs an
erase, and an erase from zero passes through values whose bytes are
neither the magic's nor erased in the flash model the tests use. And while
a page is committed, every other write or erase of its payload is refused
(`PreconditionNotMet`), so a committed page cannot describe a payload that
changed after it. `tests/update.rs` cuts power at each of the 452 flash
operations of its firmware update and the 391 of its fabric update (and
before the first): the boot check, which needs both pages committed,
only ever boots the old image whole (a cut before the first operation) or
the new one committed, and stays in DFU mode through every cut of a fabric
update. The commit word is outside the page CRC, so the page
stays checkable before and after.

The board revision is checked again at `COMMIT` (the page checks it first):
a header or manifest for another `hw/REVISION` is refused on the device as
well.

### The header is stamped after linking

The application links its header (`fastprom-ag32`'s `header.rs`) with the
version (Cargo's), board revision (`hw/REVISION`, read by build.rs and
validated: 1 to 8 printable characters) and fabric ID, and a zero length
and CRC. `tools/fw_stamp.py app` then fills in the length and CRC of every
byte the ELF loads after the header, and the header CRC, in the ELF itself,
before `llvm-objcopy` makes the .bin: the ELF and the .bin stay identical
(`fw_layout.py` checks it), and so does the file the UART loader and the
USB page write. An unstamped build has length zero, which the boot check
refuses, so it can never run. `fw_layout.py` decodes the stamped header and
checks every field against the .bin, `fw/Cargo.toml` and `hw/REVISION`.
Python and Rust share the layout through fixtures: `gen_fixtures.py` has
`fw_stamp.py` write headers and manifests, and `tests/header.rs` decodes
them and re-encodes them byte for byte.

The fabric ID is `regmap::FABRIC_ID`, the revision of the fabric's
register interface: its ID register reads 'FPRM' in every build, so the
interface revision has to be recorded beside the bitstream instead.

### Flash routines run from SRAM, copied on first use

The erase and program sequences (start, wait for BSY), and the flexible
read that sends a command to the flash, are in `.ramfunc`:
SDK's flash example puts its flash routine in `.text.sram`, its DFU builds
everything into SRAM, and `FLASH_Start` is 64-byte aligned so that STRT is
not set while the flash is busy with a fetch. The routines call nothing
(checked in the disassembly: register access is `#[inline(always)]`), and
no interrupt is ever enabled. The linker loads `.ramfunc` from flash after
`.data` but outside riscv-rt's `.data` copy, and `Ag32Flash::init` copies
it, then `fence.i`: the application copies it after READY, so its boot
does not pay for it (260 bytes, 65 words). Waits are bounded: 10^8 polls
for a sector erase (a 25-series 4 KiB erase is 45 to 400 ms; AGM gives no
figure), 10^6 per word. Unprogrammed words are skipped, so a page of 0xFF
programs nothing, which is how the commit word is programmed alone.

Assumed, not verified: that reading flash straight after programming it
returns the new data (the SDK's example verifies that way), and that the
controller's PGERR does not fire for programming a word in a page that
already holds programmed words (STM32F1's FPEC would, for a non-erased
half-word; the AG32 programs an SPI NOR, where only the bits matter).

### The update protocol: our own, not PICOBOOT

The update path speaks a protocol of FastPROM's own (`fastprom-core`'s
`update/proto.rs`; `docs/10-updates.md`, "The update protocol", has the
wire format). One ROM uses PICOBOOT, Raspberry Pi's boot-ROM protocol,
through Piers Finlayson's `picobootx` crate. FastPROM does not, for three
reasons:

* **We own both ends.** The firmware, the DFU stage and the page's client
  are all this repository's Rust, and the page talks to nothing else. One
  module builds and reads the frames on both sides, so they cannot drift.
  Compatibility with `picotool` would buy nothing, since no FastPROM user
  runs it.
* **PICOBOOT's commands are RP-specific.** FastPROM could use six of them
  (`EXCLUSIVE_ACCESS`, `READ`, `WRITE`, `FLASH_ERASE`, `REBOOT2`,
  `GET_INFO`; the rest are XIP, OTP and RAM execution), and two only
  nominally: `GET_INFO`'s flags are the RP2350's, and `REBOOT2`'s types
  and flags would have to be mapped onto two boot targets. What FastPROM
  needs (`INFO`, `LOAD`, `COMMIT`) would go through `picobootx`'s
  custom-command hook under a second magic, with no data phase from the
  host. The protocol has exactly INFO, READ, WRITE, ERASE, COMMIT, LOAD
  and REBOOT.
* **`picobootx` links panic paths** into firmware that must have none
  (four `unreachable!()` arms and four slice indexes in its `state.rs`,
  every one driven by host bytes, in 0.5.0 as well), so it could only be
  used vendored with a patch. Its descriptor rules would also cost an
  empty interface 0 in the application, for picotool's sake. The update
  interface is 0 in both firmwares, and the CDC port is 1 and 2.

The design, and why each part:

* **Request and response frames**, a 16-byte header (magic `FPUP`,
  version, command, status, sequence number, length, CRC-32 over the
  header's first 12 bytes and the payload) then the payload. Status is a
  byte of the response, not a stall and a control request as in PICOBOOT:
  a refusal is an ordinary answer, the endpoints never halt, and the host
  needs no `GET_COMMAND_STATUS`, `CLEAR_FEATURE` or token bookkeeping. The
  CRC is `fastprom-core`'s CRC-32, already linked for the image table; USB
  checks every packet itself, so the frame CRC is there to catch a frame
  made of the wrong packets (an abandoned transfer's tail followed by a new
  one), not line noise.
* **A frame is one USB transfer, ending at a short packet** (a zero-length
  one when it fills its last), and the length field must agree. Both
  directions, so a transfer boundary is a frame boundary: the host asks
  each IN transfer for 4,160 bytes, more than the longest response
  (16 + 4,096), so every response arrives as exactly one `transferIn`, and
  the device knows a request is complete without parsing ahead. No
  request of protocol version 1 fills its last packet (INFO and LOAD 16
  bytes, READ 24, WRITE 276, ERASE 20, COMMIT and REBOOT 17), so in
  practice the zero-length packet only follows some responses.
* **Every transfer gets exactly one response**, an unreadable one
  `BadFrame` with sequence number 0. That keeps the host's loop "send,
  receive, check" with no case where it must guess whether an answer is
  still coming.
* **Recovery is one rule**: anything the host cannot match to its request
  (no decode, another sequence number or command, `BadFrame`) is followed
  by the vendor RESET request, which empties the device's half-received
  request, its unsent response, and both endpoints' packets, and the
  request is sent again under a new sequence number, three attempts in
  all. Resending is safe because every command is idempotent on NOR
  flash: programming a page with the data it already holds changes no bit,
  an erase is an erase, `COMMIT` of a committed page answers `Ok`, a READ
  or INFO has no effect. A sequence number therefore never needs to be
  remembered on the device. The one loss this cannot repair is a
  transfer's last packet with nothing before it to show for it (a lost
  one-packet request, or a lost one-packet answer): neither side knows a
  transfer is incomplete, and the host's transfer times out. That ends the
  operation, and the next session's RESET clears it; a WebUSB transfer
  cannot be withdrawn once pending, so retrying on a timeout would only
  queue a second transfer behind the first. USB's own CRC and retries make
  every one of these cases an abandoned transfer, not something seen in
  normal use.
* **WRITE is one page, ERASE one sector, READ up to 4 KiB.** One page per
  WRITE keeps the device's receive buffer at 276 bytes and needs no
  gathering of pages across packets; one sector per ERASE is one flash
  operation per request, so a refusal names it. The cost is round trips: a
  48 KiB application is 12 erases and up to 192 writes, a 100 KiB fabric
  25 and 400; at 1 to 2 ms a round trip (an estimate for full-speed WebUSB,
  not measured on a board) about a second for the larger. READ is 4 KiB so that verifying takes few round
  trips; the device streams it from flash a packet at a time (computing
  the CRC first by reading the range once more), so no response is held
  whole.
* **Versioned.** The header's layout, magic and CRC rule are frozen; a
  device answers a request of another version with `Version` and its own
  version in the header, and the page refuses a device whose answers carry
  another version, telling the user to use the page released with the
  board's firmware.

### USB: our own device stack, not usb-device

The USB device side is `fw/fastprom-usb` (stack, descriptors, the update
interface and CDC-ACM; no hardware, host-tested against a model of the controller) over
`fastprom-hal`'s `usb::Ag32Usb` (the ChipIdea registers and DMA memory).
The `usb-device` crate is not used: linked into a firmware it brings
`core::panicking` from 15 call sites inside it (every endpoint read, write
and stall checks "bus initialization not complete"; RefCell borrows in its
allocator; indexing in its descriptor writers and control pipe), and the
firmware links none. A device with one configuration and fixed functions
needs little of it: a control pipe, the standard requests, constant
descriptors. That is `device.rs` and `desc.rs`, written without a
panicking index, `RefCell` or `unwrap`. The USB code is about 11.1 KiB in
the application and 10.0 KiB in the DFU stage, against 17.6 and 15.7 KiB
with `usb-device`.

The update interface is interface 0 in both firmwares, the DFU stage's
only one; the application's CDC-ACM port is 1 and 2, behind an interface
association.

The application starts USB after the boot report; on a boot failure it
keeps serving USB, so that a page can write a good table, and repeats the
failure line every 2 s from the same loop. The report also goes to the CDC
port: the console keeps a 512-byte copy, written before USB is up, which
the main loop hands to the port as it has room.

### The update page: Rust for every rule, JavaScript for the page

The page (`docs/12-web.md`) has the update protocol's client, the update procedures,
the ROM-file checks, the image-table builder and the bundle checks in Rust
(`fw/fastprom-host`), compiled to WebAssembly (`fw/fastprom-web`); the
JavaScript is devices, files and text. The table, header, manifest, `INFO`
and flash map are `fastprom-core`'s, so the page cannot write a table the
firmware reads differently, and every built table is parsed with the
firmware's own parser before it is written. A JavaScript client would be a
second implementation of formats that already have one with tests.

The simulated board is the real device side - `fastprom-core::update`'s
server - over the NOR flash model `tests/update.rs` uses, in the same
WebAssembly module as the client (354 KB with it and recovery; one module, one
build, and the real page can demonstrate itself with `?sim`). Its USB
numbers and descriptor bytes are `fastprom-usb`'s, so the page's interface
search runs on what the firmware sends. Each simulated board leaks its
256 KiB flash to give the update core's borrows a `'static` lifetime
without `unsafe`: one board per page, a few hundred per test run.

Choices in the page, each a judgement:

* **Table count.** A table holds 1, 2 or 4 images; the page writes the
  fewest that reach the last switch position given an image, and an empty
  position below it a blank image (all ones, as an unprogrammed PROM). A
  position past the table serves nothing, as the firmware has it.
* **Keeping images.** "Keep" copies an image from the table read back on
  connecting, and is refused for one whose words no longer match its CRC
  (a table cut short by a power loss): kept, it would be stored under a
  fresh CRC over damaged words.
* **ROM files.** `.hex`, `.txt` and `.mem` (any case) are `mkimage.py`'s
  text; any other name (`.bin`, `.BIN`, `.rom`) is binary, one word per
  byte; `mkimagetable.py` reads them by the same rule, and the two agree
  byte for byte on tables from all four kinds (tested). Intel HEX is
  refused by name, by both. Short files are padded, as the tool pads. A
  name with an ASCII control character, which the firmware's parser
  refuses, is refused by the tool with a message and has the character
  turned into `?` by the page (a browser's file name the user may not be
  able to see), as it does any non-ASCII character.
* **Order.** Fabric first, then firmware, in one trip to DFU mode: a cut in
  the firmware leaves the board in DFU mode (its header invalid), where the
  page resumes. Everything is fetched and checked before the first reboot.
* **The part** comes from the fabric manifest. A board without one (an
  interrupted fabric update erases it first) falls back on the image
  table's part field, which every writer records as the part's
  `tools/partdb.yaml` key (`mkimagetable.py --part`, which refuses
  anything else, `make fw-table`, the page's builder), and which the page
  takes only if it is one of `parts.json`'s keys; for a part field that
  is not a key (such as "82S147"), and only then, the page asks.
  The table survives a fabric update (it is a region of its own), so the
  fallback is there exactly when it is needed.
* **Is the table whole?** `INFO`'s "table valid" flag means the header
  parses, which a power cut during a table write can leave true over
  images half written (the header page goes first). `INFO` format 2 has a
  flag and a bit per image for what the firmware's check at boot finds
  (`loader::verify`, the CRC and the width): `TableSummary::intact`,
  computed on `INFO` (a CRC over at most 16 KiB). The page shows damaged
  positions from it, and still reads the table back for "Keep". A format 1
  answer says nothing about the images; the page then judges from the
  read-back alone.

### UART recovery: our own loader in SRAM, through the mask ROM

A blank board, or one whose DFU stage or option bytes are damaged, has to
be written through the AG32's mask-ROM serial loader (BOOT0 high, J3).
AGM publishes nothing about that loader; what it does is inferred from
AGM's own tools, and recovery is built to need as little of it as
possible: only what AGM's tools themselves ask of it.

**What the mask ROM is inferred to be, and the evidence.**

- **ST's AN3155 USART protocol.** AGM's flasher
  (`packages/tool-agrv_flashloader/bin/agrv32flash`, an x86-64 ELF with
  its symbols) is a fork of `stm32flash`: its usage text, its messages
  ("Got NACK from device on command 0x%02x", "Re sync (len = %d)", "GET
  returns unknown commands"), its source files (`init.c`, `serial_posix`,
  `binary.c`, `hex.c`, a renamed `stm32.c` as `agrv32.c`). AGM's own boot
  loader source (`examples/dfu/src/boot.h`, `boot.c`) uses AN3155's
  numbers: sync `0x7F`, ACK `0x79`, NACK `0x1F`, `GET` `0x00`, `GID`
  `0x02`, `READ` `0x11`, `GO` `0x21`, `WRITE` `0x31`, `EXTENDED ERASE`
  `0x44`, `READOUT UNPROTECT` `0x92`, and its own `CRC` `0xA1`, reset
  `0xA2`, option-byte erase `0xA3`.
- **That boot.c is not the mask ROM, but AGM's RAM booter.** The flasher
  has `-T booter_address -B booter_rate -X max_version`: "upload external
  booter if version check passes". AgRV_pio passes `-T 0x20010000 -X 0x10`
  (`builder/main.py`, `boot_version = "0x10"`; `common.py`,
  `upload.boot_address`), and `dfu_uart.bin` (5,296 bytes) is the booter it
  uploads; boot.c answers `GET` with `BOOT_VERSION` `0x20` plus the
  interface. So the mask ROM answers `GET` with a version of at most
  `0x10`, and every AgRV_pio flash goes through the booter.
- **AGM's `examples/uart_master`**, firmware for one AG32 to program
  another, says the same from the other side: for a loader version below
  `0x20` (`DFU_MIN_VERSION`) it caps the baud rate at 200 kbaud, sends a
  memory-mode byte after every read's and write's address, uploads the
  same booter (`dfu_uart.inc`, 5,296 bytes) to `0x2000_8000`, `GO`es to
  it, waits 10 ms and syncs again; everything else (erase, option bytes,
  flash writes, CRC) goes to the booter. Before uploading it reads one
  byte and, if that fails, sends `READOUT UNPROTECT` (`0x92`) to the mask
  ROM and starts again.
- **The memory-mode byte.** The flasher's `agrv32_need_mmode`: device
  `0x4020_0001`, loader version at most `0x10`, and the device table's
  flag 4; then `agrv32_get_mmode` sends 0 for SRAM, 1 for flash, 2 for the
  option bytes, 3 for system memory, as the command-and-complement pair
  after the address, acknowledged. Its device table (in `.data`) gives
  the ranges: SRAM `0x2000_0000`-`0x2002_0000`, flash from `0x8000_0000`,
  option bytes `0x8100_0000`-`0x8100_0080`, system memory (the mask ROM
  itself) `0x1_0000`-`0x1_2000`, 8 KiB, as `boards/*.json`'s
  `upload.rom_address` has it.
- **`GID` answers four bytes, `0x4020_0001`.** The flasher looks the ID up
  in a table keyed `0x4020_0001` before it can know whether to upload the
  booter, and checks it against `-J`; boot.c sends `SYS->DEVICE_ID`
  big-endian with a count of 3; `boards/agrv2k_303.json`'s `device_id`.
- **8E1**, measured on the sync byte: the flasher's `-m 8e1`, boot.c's
  `UART_LCR_PARITY_EVEN` and its `waitForInit`, which times the `0x7F`'s
  edges to set the baud rate.
- **`GO` jumps to the address as code.** `dfu_uart.bin` starts with
  `fence.i` at offset 0 and is `GO`ne to at its load address.
- **Read protection.** boot.c refuses `READ`, `WRITE` and `GO` while
  `FLASH->OBR`'s RDPRT is set; the RDP half-word unprotected is `0x5AA5`
  (flash.h `RDP_WORD`); the flasher's `-L` ("unlock device if read
  protected", which the Makefile's `fw-flash` passes) and uart_master's
  probe suggest a fresh chip may come protected. Unprotecting erases the
  whole chip (AN3155; uart_master: writing the option bytes "will trigger
  an auto full chip erase"; gen_batch: "auto full chip erase").
- **The option bytes**: gen_batch's `gen_option_bytes` layout - RDP `0xA5`
  and its complement, user, data, misc, osc half-words erased, the fabric's
  configuration address and its complement at `0x30` (flash.h
  `FPGA_CONFIG`, `FPGA_CONFIGn`; `FLASH_GetFPGAConfigAddress` checks the
  complement). `make fw-batch` writes exactly these 128 bytes, and so does
  recovery (`recovery::OPTION_BYTES`, tested equal to gen_batch's layout
  and to what `mkbundle.py` writes).

**The design that follows.** The flasher's memory modes suggest the mask
ROM can write flash and option bytes itself, but nothing AGM ships relies
on it: both tools upload the booter to any loader of version `0x10` or
less. So recovery asks the mask ROM for exactly what both tools do before
they hand over - sync, `GET`, `GID`, `READ` and `WRITE MEMORY` to SRAM
behind memory mode 0, `GO`, and `READOUT UNPROTECT` when the probe read is
refused - and hands over to a loader of its own, since AGM's booter has no
licence and cannot be shipped. `fastprom-recovery` speaks the same
AN3155 framing (version `0x46`, and AGM's `0xA2` and `0xA3`), and its
flash code is the DFU stage's (`fastprom-hal`'s erase and program from
SRAM), plus the option bytes in flash.c's own sequence (`OPTKEYR`, `OPTER`,
`PG | OPTPG` and word stores, as `FLASH_SetFPGAConfigAddress`). Its
protocol is a byte-at-a-time state machine in `fastprom-core`, so the
simulator runs the same code the board does. Choices in it:

- **The upload window.** `0x2001_0000`, the flasher's booter address, and
  at most 5,296 bytes, the booter's size: that much there is shown not to
  collide with whatever the mask ROM keeps in SRAM while it receives
  (uart_master's `0x2000_8000` is the other window shown). The loader is
  4,372 bytes; the linker refuses one that outgrows the window, which is
  why `fastprom-core`'s baud divisor is 32-bit (as uart.c computes it):
  a 64-bit divide alone is 954 bytes. Its stack and `.bss` are the first
  32 KiB of SRAM, which the upload never touches and the mask ROM has
  finished with once it has jumped.
- **Its entry** is `fence.i` then a jump to riscv-rt's start, after a
  32-byte header (magic, length, CRC, version, board revision) that the
  page checks before uploading and the simulator checks before "running"
  it: code the mask ROM wrote as data, fetched afresh, as AGM's booter
  starts.
- **Its UART**: UART0 on PIN_20 and PIN_21 by their alternate function (as
  the booter takes them on a blank chip, with no fabric), at 115200 8E1
  from its own PLL clock, the application's: the host never changes its
  port between the mask ROM and the loader. 115200 is AN3155's own rate,
  under uart_master's 200 kbaud cap and AgRV_pio's 460800; the whole
  provisioning moves about 330 KB (every page written and read back), half
  a minute at it, plus 64 sector erases of 45 to 400 ms each.
- **The order**: the option bytes first (their erase or RDP's write may
  erase the chip), then every sector, then the DFU stage, the fabric, its
  manifest, and the application with its header page last, so that a cut
  part way leaves no committed header over a partial image. Every page is
  read back as it is written (the loader also compares before it ACKs).
- **Retries.** A NACK or timeout is retried after a resync: wait out the
  loader's 100 ms idle timeout, then sync bytes until one is answered (a
  loader mid-frame takes them as the rest of the frame, which then fails
  its checksum). A page written twice with the same bytes reads back the
  same, so a retry after a lost ACK is harmless. Run again after a
  failure, the procedure finds the loader still running and carries on
  from the top.
- **Board revision.** A blank board cannot say its revision: the user
  confirms the printed one is the bundle's, and a board whose flash still
  holds a header or manifest for another revision is refused before
  anything is erased.

**To confirm on the first board:** the mask ROM's `GET`
version (at most `0x10`) and that it lists `GID`, `READ`, `WRITE`, `GO`
(and `READOUT UNPROTECT`); that it takes memory mode 0 for SRAM; that the
5,296 bytes at `0x2001_0000` are free while it receives; that a fresh chip
is or is not read protected, and how `READOUT UNPROTECT` ends (a reset and
a new sync is assumed, with a 500 ms wait); 115200 baud through its
autobaud; that UART0 still reaches J3 after `GO` with the fabric
unconfigured (AGM's booter relies on it); the loader's PLL and baud rate;
the option-byte erase and word program, and whether programming RDP erases
the chip; that the option bytes take effect at the next power-on, the FCB
configuring from `0x8001_4000`; and that the reset `0xA2` ends in the mask
ROM again while BOOT0 is held high.

### A binary image table with a header CRC

A firmware reading a flash region it did not link needs a defined byte
layout, so `tools/mkimagetable.py --out *.bin` writes one: magic 'FPRM',
format version 1, depth, width, count, names, and `zlib.crc32` per image,
and a CRC over the 256-byte header, so a damaged depth or count is caught
before it can size anything. The image
count is limited to 1, 2 or 4, since the switch has two bits. The parser
treats the bytes as untrusted: every field is checked once, nothing it
reads is used to index, and a fuzz loop and every header bit flip are in
the tests.

### UART0's pins routed as plain bidirectional GPIO

The SDK's boards route UART0 with `UART0_UARTTXD PIN_n`, which makes TX an
output-only pad. FastPROM must read the image-select switch on TX before
the UART drives it, so `tools/gen_pins.py` writes `GPIO7_6 PIN_20` and
`GPIO6_1 PIN_21`: bidirectional pads on the very GPIO bits UART0's
alternate function uses (`AltaRiscv.h`). The firmware reads both pins with
the alternate function off, then turns it on for the UART.

A `.ve` that routes nothing to these pins does not work: gen_vlog then
ties the MCU's GPIO inputs to 0, so every boot would read image 0, and the
boot UART could not reach the pins.

### The switch must read the same 16 times

The pins are sampled 16 times over about a millisecond, after a 1 ms
settle. Any disagreement is an error (code 11), and the socket is not
served. A switch caught moving, or something else driving a pin, is a
reason to stay off the bus, not to guess. With a USB-UART adapter on J3,
its TX idles high and holds RX at 1, so it reads as switch 1 on (image 1).

### HSE bypass, and bounded waits

The SDK's board default is HSE bypass off, for a crystal. The stack drives
OSC_IN from a 12 MHz oscillator (`tools/boardmap.yaml`), so the firmware
starts HSE in bypass. `SYS_SwitchPLLClock` waits forever for HSE and PLL
lock. The port gives up after about 10^6 polls and reports code 60 or 61.
The PLL is not optional: BRAM mode's read port runs on PLL output 1, so
without it the fabric cannot serve, and READY stays clear.

### No interrupts, no DMA

The SDK's `board_init` enables the DMA controller and the interrupt
controller, and its AHB example loads the fabric by DMA. Here the load is
one CPU store per word, with read-backs between (below), over at most
4096 words. Interrupts are never enabled. The one bus master besides the
CPU is the USB controller's own, on its queue heads and packet buffers
(below).

### USB: 100 mA declared, polled

The configuration descriptor declares bus powered, 100 mA (`bMaxPower`
50). That is one unit load, what any USB device may draw before it is
configured, so the declaration asks the host for nothing more than it
already allows. It is not a measurement: the AG32 and its fabric at
100 MHz draw from VBUS through Schottky D3 when no socket
supplies 5 V, and that current is to be measured on the first boards; if
it exceeds 100 mA the descriptor changes with it. The bus buffers are not
on this rail (`docs/08-usb-updates.md`, "Power").

The controller is polled from the main loop, like everything else here (no
interrupts, above). Its own bus master reads and writes the queue heads and
packet buffers, a 4 KiB-aligned static in `.bss` (`docs/11-usb-driver.md`,
"Memory").

### Release first; never read a register straight after writing it

The first write after power-up clears READY, and it is read back clear.
That comes before the switch, the clocks or the table are looked at. A
soft reset should reconfigure the fabric (the FCB is reset with the chip:
"The DFU stage's own fabric", above), which would clear READY anyway, but
that is unmeasured, and a reset that left the fabric alone would leave
READY set: clearing it first costs one write.

`fastprom_ahb_slave.v` captures read data in the address phase, and a write
lands at the end of its data phase. On a master that pipelines, a read
issued straight after a write returns the old value, so reading ADDR
straight after the last DATA write would fail every boot. The loader puts
an ID read between every write and its read-back, and checks the pointer
after every word, so a fabric that ignores writes (its pointer stays at 0)
fails at the first word.
`sim/tb_fw_load.v` replays the firmware's own bus trace on the HDL: back to
back and with idle cycles, and the hazard itself.

---

## Fabric timing

### BRAM mode, read port on its own 200 MHz PLL output

BRAM mode uses one fixed bitstream, with contents and image select loaded
by firmware; LOGIC mode is the fallback. BRAM mode's address access
(*model*):

    socket tAA = 3.97 (pad in, buffer) + T (one read-clock period)
               + 13.4 (fixed fabric) + 5.11 (pad out, buffer, terminator) ns

- T = 5 ns at 200 MHz gives 27.5 ns: +2.5 ns against 30 ns (512x8), +0.5 ns
  against 28 ns (256x8). The 256x8 is accepted as is.
- 200 MHz = 400 MHz VCO / 2 from 12 MHz. Supra's restricted Fmax for every
  clock is 200.08 MHz, a hard ceiling.
- sys_clock stays at 100 MHz, because the CPU's AHB paths don't close at
  200 MHz.
- Not used:
  - a second synchroniser stage, or a registered RAM output, because each
    adds a period;
  - an asynchronous read, because the RAM block requires registered inputs;
  - dual-phase sampling, because it gains about 1 ns, uses the clock as
    data, and can't be checked by static timing.

---

## PCB process

### Track and space 0.1/0.1 mm; via hole to track 0.2 mm

JLC's multilayer minimum is 0.09/0.09 mm (1 oz). At 0.1/0.1 mm the 1 mm-wide
lanes beside the buffers, and between the connector's via columns, hold four
and three lines; at 0.15/0.127 mm they would hold two.

Via hole to track is 0.2 mm, JLC's figure. JLC's plated-through-hole-to-track
minimum is 0.28 mm, and it is met by geometry. J1's pads are 1.6 mm with
0.8 mm drills, so the pad ring is 0.4 mm. With 0.1 mm copper clearance,
foreign copper is at least 0.5 mm from the hole.

### Four layers on both boards: two ground planes

**Stack-up.** JLCPCB's JLC04081H-3313, 0.8 mm, recorded in both board files
(`gen_layout.py` `STACKUP`):

| Layer | Copper | Use |
|---|---|---|
| F.Cu | 35 µm (1 oz) | parts and signals |
| prepreg 3313 | 0.0994 mm, εr 4.1 | |
| In1.Cu | 15.2 µm (0.5 oz) | solid ground plane |
| core | 0.45 mm, εr 4.6 | |
| In2.Cu | 15.2 µm (0.5 oz) | solid ground plane |
| prepreg 3313 | 0.0994 mm, εr 4.1 | |
| B.Cu | 35 µm (1 oz) | parts and signals |

**Why two planes.** Every signal is microstrip over a ground plane 0.1 mm
away: F over In1, B over In2. A routing layer on In2 would have In1 on one
side and B.Cu's tracks on the other. There are no supply planes and no
split planes. The supplies are tracks on F and B,
0.2–0.3 mm (`+5V_SKT`, `+3V3_BUF`, `+5V`, `+5V_USB`, `+3V3`, `+3V3_MCU`), and
FB1's two sides never share copper.

**What it costs.** Two signal layers instead of three. Every crossing is a
via, so nearly all the routing is hand-placed (`tools/layout/fans.py`) and
Freerouting finishes a few short connections round it. The lower board's
two receivers face opposite ways, and the one underneath (U2) is mirrored,
so its lines reach the socket and the connector in the reverse order. The
same holds for the terminator packs under U4. Those lines change sides
through vias, or go round the far side of a DIP pin column. The upper
board's two crossings are the drive buffer's enable (`D_BUF_NOE`) and
`BUF_D7`, each over two others on top. Placement alone makes two layers
enough, with no jumper on an inner layer.

**Return paths.** A line that changes layer changes reference plane, from
In1 to In2 or back. Its return current crosses between the planes at the
nearest ground via. `gen_layout.py stitch()` runs after routing and places
ground vias wherever they clear every pad, hole, track and edge:
- one within 1 mm of every via off ground;
- along every edge, and round J5/J6, U1 and the three buffers, at most 4 mm
  apart.

KiCad's DRC then judges them. That places 34 ground vias on the lower board
and 30 on the upper.

Every ground via joins both planes. With the connectors' ground contacts,
the exposed pads' vias and the decoupling vias:

| | vias | ground | other | others with a ground via within 1 mm |
|---|---|---|---|---|
| lower | 78 | 57 | 21 | 11 of 21 |
| upper | 88 | 59 | 29 | 22 of 29 |

Where there is no room (`python3 tools/layout/route.py
build/layout/<board>.json` lists them):
- **lower, U4's A vias.** Eight vias at 0.5 mm pitch fill the 0.9 mm
  between U4's A row and the board's end, with a ground via at each end of
  the rows: A1 and A8 are within 1 mm of one, A2 and A7 1.11 mm, the middle
  four 1.6–2.1 mm.
- **lower, J5's lane.** BUF_A4 (1.27 mm), between the header's ground
  contacts.
- **lower, others.** D6's crossing via (1.5 mm), A2's (1.24 mm) and C9's
  supply via (1.4 mm).
- **upper, U1's right and far edges.** The dog-bones for NRST, pin 6 and
  BOOT0 (1.66–1.8 mm). The right edge's bundle and the far edge's wrap
  fill the copper round them on the underside.
- **upper, others.** The enable's first via (1.5 mm), USB D−'s dog-bone
  (1.37 mm), CC2's via (1.18 mm), and the 5 V hop beside SW1 (2.3 mm).
  The six vias of J3's lines (see "J3 at the pin 1 end") each have one,
  in the open strip between the clip's holes and the parts.

D−'s two vias at the receptacle each have a ground via 0.57 mm below.

**Why the remaining distances are harmless.** Both inner layers are ground,
so a layer change only moves the return between In1 and In2, through the
via pair's inductance across the 0.45 mm core, L ≈ (μ0·h/π)·ln(s/r), r =
0.1 mm: 0.41 nH at 1.0 mm, 0.55 nH at 2.11 mm. The worst case (U4 A4/A5)
adds 0.14 nH over the 1 mm rule, about 1.2 Ω in a 53 Ω line at 350 MHz
(0.35/tr), a 1 % reflection; U4's eight A lines switching together through
the end vias give about 50 mV of bounce. No via needs a closer one.

**Bus spacing below 2W.** The bundles run at 0.21–0.235 mm
pitch (0.11–0.135 mm gaps), not the 0.3 mm of 2W. A 2D field solve for
0.1 mm lines 0.0994 mm over the plane (εr 4.1) gives:

| pitch | kC | kL | Z0 | near end, saturated | far-end coefficient |
|---|---|---|---|---|---|
| 0.21 mm | 0.164 | ≈0.17 | 53.5 Ω | 8.3 % | 0.003 |
| 0.30 mm | 0.069 | 0.082 | 57.3 Ω | 3.8 % | 0.006 |

Every coupled run is electrically short (Td ≈ 0.1 ns against tr ≈ 1 ns), so
near-end noise is 8.3 % × 2Td/tr, about 1.7 % (55 mV) per neighbour, 110 mV
with both: at least 4× inside the 0.4 V margin of the LVT and AG32 inputs,
and only during edges. The longest tight pairs are BUF_D5/D6 (16.7 mm) on
the upper board's underside and BUF_A1/A2 (12.7 mm) on the lower. On the
host's TTL side the longest pair is under 10 mV. Meeting 2W would halve the
coupling and change nothing.

**Plane fill.** From KiCad's own fill: the lower board's planes each cover
231.5 mm² of 293.1 mm² (79 %), the upper's 256.7 mm² of 309.8 mm² (83 %).

### No pours on the outer layers

F.Cu and B.Cu carry tracks and pads only. The ground copper is the two inner
planes, each ground pad's own stub and via to them (`gen_layout.py
prewire()`), and the stitching vias.
- Outer pours fragment, because they don't fit between the 0.1/0.1 mm bus
  tracks: on the lower board a B.Cu pour is 16 islands, some 0.26–1.15 mm²,
  two with no via.
- They would give return currents a poorly stitched path beside the solid
  plane.
- No pad needs them: every ground pad has its own stub and via.
- F and B copper stays balanced without them (tracks, pads and vias): lower
  35 % / 33 %, upper 25 % / 27 %.
- Without them the 0402 and 0603 ground pads heat in reflow like their
  signal pads.

### J1's inner-layer pads removed (except pin 10)

With 1.6 mm pads on In1, the 19 signal pins would cut 2.0 mm voids at
2.54 mm pitch, leaving 0.54 mm necks. With unused inner-layer pads removed
(`remove_unused_layers`, `keep_end_layers`) the voids are the drill plus
the plane's 0.2 mm clearance, 1.2 mm, leaving 1.34 mm necks. Pin 10
(ground) keeps its pads on every layer.

### AG32 decoupling vias, at the pins

The AG32 is underneath and its capacitors are on top, so each supply and
ground passes through a via.

- **C5 (pin 32):** pin 32 leaves the end of its pad toward the corner,
  0.85 mm underneath to a via at (9.2, 6.75), then 0.72 mm on top into C5:
  1.57 mm. C5's ground is 0.56 mm on top to an exposed-pad via. The
  ferrite's +3V3_MCU comes down into the same via from above.
- **C3 (pin 6):** pin 6's dog-bone via, then 1.87 mm on top into C3. Its
  ground is 0.68 mm to a via of its own in the gap under U1's corner,
  between the bottom-edge and right-edge bundles, which C11 and C15 share
  (0.73 mm each).
- **C4 (pin 16):** the corner pin's via in the pocket the left-row bundle
  leaves, 0.63 mm underneath, then 0.74 mm on top into C4. Its ground
  pad has a via in it, and underneath that via runs 0.48 mm to pin 17,
  the ground pin beside pin 16: the capacitor sits across the two
  adjacent pins. C4 sits with that pad over the pocket because the
  pocket is the only place a ground via fits: underneath, the left-row
  bundle and the left edge's two outer lines fill x 3.8–8.4 below the
  AG32, and on top the USB pair runs just left of C4. The via is filled
  and capped like every via (`hw/ORDER-JLCPCB.md`), so the pad stays flat
  and keeps its solder.

**Every capacitor's ground**, pad centre to the via's centre
(`build/layout/<board>.json`):

| Board | Capacitor | Ground to via |
|---|---|---|
| upper | C4 | in the pad |
| upper | C5, C16, C3 | 0.56, 0.66, 0.68 mm |
| upper | C11, C15 | 0.73 mm (C3's via) |
| upper | C14, C20 | 0.85 mm |
| upper | C2 (U5's output) | 0.95 mm, to a via between it and U5's input pin |
| upper | C1 (U5's input) | 1.05 mm, to a via at the 0603's corner |
| upper | C19 (U6's VBUS) | 0.72 mm, into the receptacle's plated shell slot |
| lower | C8 | 0.65 mm |
| lower | C9, C13 | 0.73, 0.76 mm |
| lower | C18, C10 | 0.84, 0.88 mm |
| lower | C17 (U7's enable delay) | 0.95 mm |
| lower | C12 (U7's input) | 0.97 mm |

C12 has a ground via of its own beside its pad, clear of U2's pins and
exposed pad underneath. C19 is underneath, below the clamp U6 on top,
clear of the panel's mid-board tab; its ground goes straight into the
receptacle's front shell slot, which is plated and joins both ground
planes. U6 is a transient clamp, and its own ground pin has two vias
beside it.

**The AG32's rail, +3V3_MCU**, is one hand-routed tree on top
(`fans.py` `upper_mcu_supply`), 0.25 mm:
- from FB1's output via (left of U5, underneath) straight down the clear
  strip between SW1's left pads and BOOT0 into the branch to C14 (10 µF)
  at the left edge, so the ferrite meets the bulk capacitor first;
- from there right, round the top of the USB pair's vias, past X1's
  capacitor C16 into X1's VDD pad (pin 4);
- from there a spine straight down X1's left pin column, through its OE
  pad (pin 1), to C15 (1 uF), and on to the test pad TP3;
- off the spine at pin 4, up the 0.46 mm between R10 and C5 and right
  above C5 (0.2 mm) to pin 32's via and C5;
- off the spine under X1, one straight bar: right to C3's run from pin 6's
  via (and R9, NRST's pull-up), left to pin 16's via and C4.

Each supply pin's via reaches its own capacitor without the bar or the
spine in between, and FB1's input (+3V3) never comes near. C14's supply
pad faces the run, and C15 sits on the spine. The rail is 33.2 mm (2.85 of
it the spur to TP3) with 4 vias and 9 bends, every one 45 degrees.
It is routed by hand because Freerouting's route for it is 34.6 mm with 6
vias and 18 bends, among them a loop of about six bends round X1 and one
bend sharper than a right angle.

### Routing quality: 45-degree corners, no zigzags

**Decision.** No track turns through more than 45 degrees at a corner. A
right angle is two 45-degree bends; a turn sharper than a right angle, or at
an angle off the 45-degree grid, is not drawn at all. A run of track takes as
few bends as the copper around it allows. Freerouting can't be made to do
this, so `gen_layout.py --ses` cleans up its corners after routing, with
KiCad's DRC as the judge.

**The count.** Every joint where exactly two tracks of one layer meet
(`python3 tools/layout/route.py build/layout/<board>.json`; straight under
0.5°, 45 or 90 within 1°):

| board | segments | bends | 45° | 90° | sharper | off-grid | track |
|---|---|---|---|---|---|---|---|
| lower | 327 | 200 | 196 | 2 | 0 | 2 | 401.8 mm |
| upper | 325 | 188 | 180 | 3 | 0 | 5 | 470.1 mm |

The most-bent nets are supplies: lower +3V3_BUF 15 and +5V_SKT 13, upper
+5V_USB 14 and +3V3_MCU 9 (routed by hand, see "AG32 decoupling vias").
The most-bent signals are the upper board's UART_RX
(15), from J3 at the board's end round its corner, up to R25 and down the
left edge to R18 and the AG32, and the lower board's BUF_A4 (12), on its
detour outside the right pin column.

**Exceptions.** All 12 joints that aren't 45 degrees are at a via where two
separate stubs meet, on the same layer, from two pads or from a pad and a
track. They are branches, not a bend in a track, and there is nothing to
chamfer:
- lower: U3's exposed-pad via (8.10, 7.30), U3's pin 10 and its A6-A8 bar;
  the shared ground via (8.16, 18.35) of J5's k11 and k12; C10's supply
  via; the socket-power via V5 (9.65, 13.75), with J5's two socket-power
  contacts.
- upper: J6's shared lane vias (socket power, k8/k9, k11/k12); U1's pin 16
  via, with C4's stub and the rail's bar; VBUS's via by the receptacle,
  to C19 and to R27; the +3V3 via by SW1; BOOT0's via at the AG32, with
  the line from J3 and R10's; UART RX's via under R25, where J3's line
  meets the line down to R18.

**Why Freerouting doesn't do it.** Its sources at the v2.5.0 tag, and runs of
both boards (details in `scripts/freerouting.sh`):
- **Trace angle.** It is already 45 degrees, the default when the DSN has no
  `snap_angle`. The 45-degree mode is octilinear: a corner may turn through
  90 or 135 degrees as freely as through 45. No setting limits the turn.
- **Bend cost.** `default_bend_cost` is clamped to 9.9 board units, 0.25 µm
  of track here. At its maximum the session is byte for byte the same.
- **Optimizer.** It is on, but pass 1 stops after its 12 allowed failures and
  is rejected. With those limits opened up, every pass is rejected for
  breaking a connection.
- **Pull-tight accuracy.** Finer settings route the lower board differently
  (19 fewer bends, 8 % more track). They barely change the upper, and under
  load they leave it with a connection unrouted.
- **Not all the router's doing.** Before the cleanup, 36 of 57 of the
  90-degree and sharper corners on the lower board and 39 of 45 on the
  upper are in the hand-routed copper (`fans.py`, `prewire`), and every
  off-grid joint has a hand-routed track on at least one side. Those
  corners stay as written for Freerouting to route round, and the cleanup
  redraws them in the finished copper.

So Freerouting's angle, bend-cost, optimizer and pull-tight settings stay
at their defaults, and its corners are cleaned up after it.

**The cleanup** (`route.py` `smooth()`, run by `gen_layout.py
clean_corners()` after widening):
1. Two tracks of one net leaving a point in the same direction are trimmed
   so they don't overlap (R21's pull-up over D_BUF_NOE's last 0.9 mm, and
   the stubs of J4's paired ground pads).
2. Every track is cut where another ends on it, or a via of its net stands
   on it, so each connection is at a track end.
3. The copper is split into runs between fixed points. A fixed point is a
   via, a junction of three or more tracks, a change of width, a track end,
   or a pad where one of those is. Connections never move.
   - Where exactly two tracks of one width meet on a pad of their net, the
     run goes on through the pad. Whatever replaces it must keep a corner
     well inside the pad, or a point on a straight that crosses it.
4. Each run gets a cost: per joint, 1 for 45°, 4 for 90°, 8 off the grid,
   20 sharper than 90°; 0.5 per straight at an odd angle. A joint at a
   fixed point where just two tracks meet counts too, but at a via only if
   the two meet in a sharp V.
5. Every stretch between two of a run's corners is tried as each of these
   shapes: a straight (at an odd angle only as a stub of 1 mm or less), one
   45-degree bend either way round, a jog (straight, diagonal, straight), or
   a chamfered corner (horizontal, diagonal, vertical) at several sizes.
   - The cheapest shape that clears every other net's copper, pads, holes
     (slots as slots), no-track areas and the board edge, by the board's
     0.1 mm, replaces the stretch. It must be cheaper, or as cheap and
     shorter, and at most 10 % + 0.2 mm longer.
   - Runs are taken in a fixed order, and the board is swept until nothing
     changes.
6. A right angle where two widths meet is chamfered at the narrower width.
7. KiCad's DRC (errors and warnings) runs on the result, on the placed
   board with its ground planes. Any track it names, or that lies within
   1.5 mm of a new unattributed report, is forbidden to step 5, and the
   cleanup runs again. Both boards pass on the first run.

All of it is deterministic: `make reroute` gives the same file each time,
and `make layout` only lays it.

**Lengths other entries quote**, from the routing files, pad centre to pad
centre: C19 to U6 pin 5 is 2.1 mm with a via; C10's supply 1.25 mm and
ground 1.46 mm, a via each; C18 1.66 mm (supply) and 0.84 mm (ground); the
data lines' driver-to-pin length (U4 through the terminator to the socket
pin) is 5.8–14.5 mm; the longest socket-to-AG32 signal is 38.3 mm (socket
pin 14, both boards, connector excluded).

---

### Assembly panel: five matched pairs, tab-routed, 77.10 × 74.02 mm

**Decision.** One panel carries five upper boards above five lower boards
(`tools/gen_panel.py`, `make panel`). The layout:
- every gap is a 2 mm routed slot;
- 5 mm rails run along the top and bottom, and 5 mm frame bars down the
  sides;
- each board hangs from 5 mm tabs on its long sides, two per side on the
  lower board and three on the upper;
- each tab has a row of five 0.6 mm mouse bites along every board edge it
  meets.

The upper boards are turned 180°, so each USB-C receptacle faces the top
rail across its slot. Each instance's nets and references carry its number,
so DRC treats the ten boards as separate circuits, and the BOM and CPL name
every part once. JLC's rules used, as published on jlcpcb.com in Sep–Oct
2026:

| Rule | Value | Source |
|---|---|---|
| Standard PCBA: single board and panel size | 70 × 70 mm minimum, 250 × 250 mm maximum panel | PCB Assembly Capabilities |
| Economic PCBA | one side only; Standard is needed for double-sided | PCB Assembly Capabilities |
| Process rails for SMT | 5 mm; JLC adds rails on two sides by default to boards under 70 × 70 | after-sales FAQ, Capabilities |
| Fiducials | 1 mm, centred 3.85 mm from the panel edges | Capabilities; "Specifications for adding process edges" |
| Tooling holes | 2 mm | Capabilities; same article |
| Mouse bites | 0.6 mm holes, 0.35–0.4 mm apart (0.3 mm min), 5–8 per set; at least two sets per side for boards under 30 mm | blog, "Mouse Bite Panelization Guide" |
| Board-to-board gap | 1.6 or 2 mm | Capabilities; same guide |
| Tab width | ≥ 4 mm, ≥ 5 mm with mouse bites | Capabilities |
| V-cut | copper ≥ 0.4 mm from the cut; connecting edge ≥ 5 mm for boards ≤ 0.8 mm; straight lines across the whole panel; overhanging parts need routed slots | Capabilities; "V-Cut Panelization Standards" |
| Assembly prices | double-sided setup $51.12; panel fee $51.12 × (designs − 1); double-sided stencil $16.42; $0.0016 per joint; $1.53 feeder loading per part | "PCB assembly price", Oct 2026 |

**Why tabs, not V-cut.** The boards' copper runs to 0.3 mm from their
edges, inside V-cut's 0.4 mm. The USB-C overhangs an edge, so it needs a
routed slot in front of it. A V-cut grid would also put the cut lines
through the rails.

**Where the tabs are.** Neither board's short ends will take a tab:
- the lower board has socket pins 1, 10, 11 and 20 and C18 there, 0.34 mm
  from the edge;
- the upper board has J3 at the pin 1 end, a SOICbite whose clip must
  bite a clean edge, and the receptacle at the other.

So the tabs are on the long sides. A tab and its mouse bites are where
the board bends as it is snapped out, and a ceramic chip near the edge
there can crack: an MLCC (a crack that shows only later, as a short or a
leaky rail) or a ferrite bead, which is a ceramic chip too (a cracked FB1
leaves the AG32's rail open, or worse, open only now and then). So **no
tab or bite comes within 1.5 mm, along the edge, of a ceramic chip whose
courtyard is within 1.5 mm of that edge**, measured from the tab's end
(the bites lie 0.3 mm inside it) to the courtyard. `gen_panel.py` checks
it on the routed boards (`ceramic_clear`: every `C_` footprint and every
`FB` reference, both faces, both long edges, every tab, since a tab
across a gap joins one board's right edge to the next one's left) and
refuses to build the panel if any breaks it. Distances below are
courtyard and pad from the edge, over each 5 mm tab:
- **Lower,** centred at y 5.51 and 20.75, between socket pins. The nearest
  copper is J1's ring, 0.9 mm in. J1's pins are fitted by hand after
  depaneling. The nearest surface-mount part is 2.76 mm in, and no MLCC on
  the lower board is within 2.8 mm of a long edge.
- **Upper, at the pin 1 end,** y −2.5 to 2.5 (centre 0.0), 1.5 mm in
  from the end. The pin 1 corner's 1 mm chamfer ("Silkscreen marking")
  takes the left edge's first 1 mm (y −4.0 to −3.0), and a tab meets a
  straight edge only, so the tab starts 0.5 mm past the cut's end
  (`gen_panel.py` checks that, `chamfer_clear`; on both long sides,
  since a tab across a gap joins two boards at one y). J3's clip bites
  the end edge's middle 7.8 mm, which stays a clean routed edge: the tab
  and its nubs are on the long sides, the nearest bite 2.1 mm (centre)
  from the end. Without this tab the end, 15.8 mm of board past the
  mid-board tab carrying U1 (QFN-32), SW1 and U5, would hang free, and
  1 N from the stencil or the squeegee would bend it about 0.14 mm at the
  tip (a cantilever, F L³ / 3EI, 0.8 mm FR-4 at about 20 GPa, 11.02 mm
  wide), enough to spoil the paste print there. With it, the longest free
  span is the 9.3 mm between this tab and the mid-board one, which the
  same 1 N bends less than 0.003 mm (simply supported, F L³ / 48EI), and
  the 1.5 mm of end past the tab is too short to bend measurably. The
  right edge below the tab has no room for an 0603: the far edge's five
  data lines turn down it at y 5.1, and the 5 V comes down there. So the
  regulator group sits, left to right underneath, on the pin 1 end's free
  strip:
  - **R17/R18**, the UART pull-downs, left of FB1, side by side, sharing
    one ground via between their ground pads (the top's SW1 pads leave no
    room for a via under them). TX and RX come down from J3's vias into
    them and on past C1 to the AG32: TX down the left edge, RX between
    the pull-downs and FB1 and between C1's pads.
  - **FB1**, left of U5, 2.50 mm in from the left edge (outside the
    rule); input from the +3V3 line that comes down from J3's pin 1 above
    U5, output through a via between it and U5's enable pin up to the
    top, down the clear strip between SW1's left pads and BOOT0, into the
    +3V3_MCU tree at the branch to the 10 µF C14, so the ferrite feeds
    the bulk capacitor first.
  - **C1** (U5's 4.7 µF input) below FB1, 1.0 mm from the left edge but
    1.7 mm along it from the tab, fed from U5's enable pin along the 5 V
    track.
  - **U5** and **C2** (U5's 10 µF 0603 output capacitor, beside U5,
    courtyard 1.89 mm in, pads 2.19 mm), C2's +3V3 on through a via to
    SW1's commons.

  Paths (track length, pad centre to pad centre): U5's output to C2
  1.75 mm; U5's output to FB1 6.0 mm (on past C2 at 0.25 mm; C2 at the
  regulator is what matters); FB1 to C14 6.9 mm, to X1's VDD 9.2 mm, to
  pin 32 (via C5) 13.6 mm, to pins 16 and 6 14.1 / 15.6 mm; every supply
  pin reaches its own capacitor before the tree. U5's enable to C1
  3.1 mm, its input 4.6 mm. U5's output loop: VOUT (pin 5) to C2's +3V3
  pad 1.75 mm of 0.25 mm track; C2's ground pad to its via 0.95 mm, and
  that via to U5's ground via, the return through the planes, 1.88 mm;
  the loop round, from VOUT through C2 and back to U5's ground pin,
  9.7 mm. C2's via sits between it and U5's input pin. J3's clip
  clearances underneath are in "J3 at the pin 1 end".
- **Upper, mid-board,** y 11.835–16.835 (centre 14.335). The MLCCs within
  1.5 mm of the upper board's long edges here are C14 (left, on top, y
  7.23–10.28, pads 0.54 mm in) and C19 (left, underneath, y 18.40–20.31,
  pads 0.8 mm in), each 1.56 mm along the edge from the tab's ends (their
  pads 2.16 and 2.06 mm from the bites' hole edges, y 12.13–16.54). C11
  (NRST's capacitor), level with the tab's start on the right, is 1.56 mm
  in (pads 1.75 mm), outside the rule. Beside the tab: on the left the USB
  clamp U6 (pads 0.61 mm in) and the USB pair; on the right the OR-ing
  diodes D2 and D3 (pads 0.86 mm in) and the 5 V track (0.41 mm). None is
  a ceramic capacitor; U6's and the diodes' leads take a bend that an
  MLCC's terminations do not. U6's courtyard stays inside the edge, clear
  of the mouse bites, which reach 0.05 mm into the board.
- **Upper, at the USB end,** centred at y 25.25, flush with that end,
  beside the receptacle's metal shell: courtyard 0.15 mm, shield-leg pads
  0.69 mm. The receptacle is the board's full width and fixed at the end,
  so this can't improve. It is also the board's most rigidly anchored
  part.
- The tabs must sit at the same height on both sides, because neighbouring
  boards share them.
- **Mouse bites** are centred 0.25 mm out from the board edge, in the gap.
  A hole reaches 0.05 mm into the board, 0.25 mm from its copper; the
  hole-to-copper rule is 0.2 mm. The nub left after breaking is about
  0.25 mm.

**Why five pairs, and both boards on one panel.**
- Five columns are the fewest that make the panel 70 mm wide:
  2 × 5 + 5 × 11.02 + 6 × 2 = 77.1 mm. Two rows give 2 × 5 + 31.76 +
  26.26 + 3 × 2 = 74.02 mm.
- One combined panel costs the same setup as two separate orders: $51.12 for
  the first design plus the $51.12 panel fee for the second, against two
  $51.12 setups.
- It saves a stencil ($16.42), a second PCB order, and the feeder loading
  for the passives the two boards share ($1.53 a line).
- Every panel yields matched pairs.

**Rough assembly cost per order of two panels (ten modules), before parts
and the bare boards:**

    setup + panel fee   51.12 + 51.12 = 102.24
    stencil (2 sides)                    16.42
    joints   2 × 1565 × 0.0016  =         5.01
    feeder loading   22 lines × 1.53 =   33.66
                                         ≈ $157, about $16 a module

The fixed fees dominate, so larger orders cost much less per module. JLC's
quote decides; the "different designs" fee for fabrication is set on its
order form and isn't published.

**Is a larger panel worth it?** No. From JLC's published pricing:
- Setup, panel fee, stencil and feeder loading are charged per order and
  per design, not per panel. That is about $152 for this two-design,
  double-sided job whatever the panel size.
- Joints are per joint.
- The bare board is priced by area: a 2024 JLC example gives $70.6/m² for a
  small 4-layer batch. So the frame's waste is cents per module:
  - 5 pairs: 77.1 × 70.0 mm, 55% boards, about $0.03 a module of frame;
  - 10 pairs: 5 × 4 rows, 77.1 × 128 mm, 60%;
  - 20 pairs: 142 × 128 mm, 65%.
- JLC adds a board fee for panelized boards over 102 × 102 mm (amount not
  published). Only the 5-pair panel, or 6 pairs at 90.1 × 70.0 mm, stays
  under it.
- Bigger panels flex more at 0.8 mm through two reflows, and JLC prefers
  250 mm or less.

Estimated cost per module, from the published fees alone (excluding parts,
bare boards, shipping and the fabrication "different designs" fee):

| Modules | Fees | Joints | Per module |
|---|---|---|---|
| 10 | ≈ $152 | $5 | ≈ $15.7 |
| 25 | ≈ $152 | $13 | ≈ $6.6 |
| 50 | ≈ $152 | $25 | ≈ $3.5 |
| 100 | ≈ $152 | $50 | ≈ $2.0 |

The panel size moves none of these. Quantity does.

**Prototypes.**
- JLC's minimums: 5 bare panels, of which as few as 2 can be assembled
  (PCBA minimum is 2).
- The cheapest handful is therefore 5 panels with PCBA quantity 2. Either
  assemble whole panels (10 modules), or set `make jlc-panel PAIRS=1` (or 2,
  3...), which lists only the first N pairs of each panel in the BOM and
  CPL: 2N modules.
- The fixed fees (≈ $152) are the same either way. Fewer pairs saves only
  the parts and $0.50 of joints per module not built.
- The other boards come back bare. On an assembled panel, JLC's stencil
  still prints paste on the unassembled instances, so those come back with
  reflowed solder on their pads. The three unassembled panels are clean.
- A smaller dedicated panel can't be smaller than 70 × 70 mm (Standard
  PCBA), so it would save nothing over the 5-pair panel.

**Not checked by JLC's rules or by DRC:**
- JLC's own zero orientation for each package in the CPL (checked offline
  against the EasyEDA footprints, `tools/jlc_parts.yaml`; J4's entry is to
  be re-checked before ordering);
- The receptacle shell, U6, the diodes and the ferrite FB1 beside the
  upper board's tabs, under break-out stress (the MLCC rule is checked,
  by `gen_panel.py`); and the boards' flatness through printing and
  reflow.

Check both in the first order's preview and on the first boards.

### Fabrication options

JLC's order form leaves most of the board's specification to the order;
`hw/ORDER-JLCPCB.md` has every field. The ones the design decides:

- **Epoxy-filled and capped vias.** Eleven vias sit in pads: four in
  U1's exposed pad, three each in U2's and U3's, and one in C4's ground
  pad (all ground, 0.2 mm drill); U4's exposed pad has none. An open
  via in a pad that is pasted and reflowed draws solder down the barrel,
  starving the joint and voiding under the package; in a 0402's pad it
  would also tilt or tombstone the part. Tenting cannot help
  inside a mask opening, and JLC's "plugged" covering requires no mask
  opening on either side of the via, so only filled and capped vias leave
  the pads flat.
- **ENIG.** The QFN-32 and both DF40 connectors are 0.4 mm pitch; HASL's
  uneven thickness is a bridging and coplanarity risk at that pitch. J3's
  edge pads take a clip's contacts or pogo pins, which a flat gold finish
  suits too.
- **1 oz outer copper.** The boards use 0.1 mm track and space; JLC's
  4-layer minimum is 0.09 mm at 1 oz and 0.15 mm at 2 oz.
- **0.2 mm vias.** Every via is 0.2 mm drill, 0.45 mm diameter, so the
  order takes JLC's 0.2 mm via tier, which costs more than the 0.3 mm
  default.
- **Outline tolerance ±0.1 mm.** Both boards are exactly the socket
  footprint's 11.02 mm wide, which must not grow (`docs/05-hardware.md`).
  JLC's regular milling tolerance is ±0.2 mm; the form offers ±0.1 mm
  (Precision) for this board. The tab nubs, sanded to about 0.25 mm, come
  on top.
- **No impedance control.** Full-speed USB and the 5 V socket bus are not
  impedance-controlled lines.

### Test pads: the supplies, where they can be probed

**Decision.** Three bare 1 mm pads, on the rails that can't be reached
otherwise: two on the upper board, one on the lower (the schematic's
`TP1`-`TP3`), not assembled. Bring-up measures the rails on the assembled
module, so each pad sits where a probe reaches it, on a short run of its
rail:

| Board | Pad | Rail | At (frame mm) | Track to it | Reach |
|---|---|---|---|---|---|
| upper | TP2 | `+5V_USB` | (9.25, 18.15), top | on the straight VBUS run from the receptacle to D3: none | always, from above |
| upper | TP3 | `+3V3_MCU` | (7.41, 15.15), top | the rail's spine, 2.85 mm on from C15 | always, from above |
| lower | TP1 | `+3V3_BUF` | (5.67, 4.60), top | 0.5 mm stub off the run from U7's output pin down to C13 | in the host, upper board lifted off |

- **Numbering.** The lower board's pad is TP1, the upper's TP2 and TP3:
  the JLC panel carries both boards, so a reference may appear on only
  one.
- **Upper.** Its top shows whether or not the module is seated. TP2 is
  between D3 and the receptacle (with a cable in, the plug is outside the
  board edge); TP3 is mid-board over the DF40 socket, between U6 and the
  diodes, between J6's lane vias. With USB and the socket both live, TP2
  against the socket's 5 V (D2's anode, or the host) shows which way the
  OR-ing diodes conduct.
- **Lower.** Its top faces the upper board and its underside the host
  socket. Underneath is never reachable in the host, so the pad is on
  top: with the upper board lifted off J5 the lower board stays in the
  host socket, powered, and the pin 1 end of its top is open. TP1 is
  there, below-left of the regulator U7, between C13 and the pin 1-10
  column, its pad 7 mm short of J5; its neighbours are U7 (0.4 mm
  high), 0402s, U3 (1.0 mm) and the trimmed pin tails (1.5 mm) 2.5 mm
  away, so a probe comes in at any angle. To make room, C13 stands
  upright below U7's ground pin, and the enable delay R26/C17 and R14
  stand in a column at the board's right; the pad hangs on a 0.5 mm stub
  off the run from U7's output to C13. Nowhere else on that top has room
  for the stock pad's 2.1 mm courtyard: J5, U3, U4 and the regulator's
  parts fill the channel between the pin strips.
- **No pad for `+5V_SKT`.** It is J1 pin 20 itself: on the bench, the
  bench supply's lead on that pin; in the host, the socket's pin 20. A
  pad would only repeat the pin.
- **Ground.** J3's `GND` pad and the receptacle's shell on the upper
  board's top; on the lower board, J1 pin 10 (its tail on top, or the
  socket's pin 10).
- **Not assembled.** `tools/jlc_parts.yaml` marks them DNP: they appear in
  neither BOM nor CPL. ENIG leaves them flat.

### Silkscreen marking

**Decision.** Each board carries `FastPROM rev A`, `UPPER` or `LOWER`, and
`© 2026 mxshift`, and one kind of pin 1 mark, all inside JLC's legend
rules, which DRC enforces ("Silkscreen rules" below). The module's own
pin 1 is marked by a chamfered corner on each board, as a DIP is marked by
its shape ("The module's pin 1" below).

**One pin 1 mark: the triangle.** Every part with a pin 1 carries the same
filled triangle on its own silkscreen layer, drawn in its footprint (each
one a project footprint, `hw/lib/fastprom.pretty/README.md`), and no other
pin 1 mark. The triangle is 0.5 mm across the base and 0.35 mm tip
to base, filled, with a 0.15 mm stroke round it (JLC's minimum), so it
prints 0.65 x 0.5 mm. **Placement rule:** it lies entirely outside the
part's body (its Fab outline, the package itself), stroke included, and
outside every other part's body on that side, so it still shows with the
parts on, and it points in toward the package: its tip nearer the body
than its base. It points at pin 1: it sits on pad 1's centre line (its
axis), beyond the pad's outer end (away from the package; past the inner
end only where the outer end is the board edge, J3), tip toward the pad
and 0.2 mm from its copper, stroke included. It is never beside pad 1 or
pointing into the package edge next to it; where a neighbour would stand
in that spot, the neighbour is placed elsewhere. Its size and its
offset from pad 1 are the same in every footprint, so the mark looks the
same on both boards and both faces (underside parts carry it on B.SilkS, mirrored
with the part).

| Part | Footprint | Side of pad 1 | Nearest other pad | Edge |
|---|---|---|---|---|
| J1 (lower, top) | DIP-20 pin strips | outer end (outside the pin 1 column) | 1.84 (J1.2) | 0.25 |
| J5 (lower, top) | DF40C-30DP | outer end | 0.22 (J1.15) | 2.7 |
| U2-U4 (lower) | TI RGY VQFN-20 | outer end | 0.17 (C8.1, U2); 0.57 (C9.1, U3); 0.167 (J1.10, U4) | 2.1 |
| U7 (lower, top) | TI X2SON-4 | along the package edge, past the row's end | 0.62 (U7.4) | 2.7 |
| U1 (upper, under) | QFN-32 4x4 | outer end | 0.36 (U1.2) | 2.5 |
| U5 (upper, under) | SOT-23-5 | outer end | 0.68 (U5.2) | 4.0 |
| U6 (upper, top) | SOT-23-6 | outer end | 0.68 (U6.2) | 2.5 |
| X1 (upper, top) | SiT PQFN-4 2.0x1.6 | outer edge below pad 1 (its outer end is C16's) | 1.02 (X1.2) | 5.0 |
| J3 (upper, top) | SOICbite | inner end, past the clip's outline (the outer end is the board edge) | 0.58 (J3's hole) | 3.0 |
| J6 (upper, under) | DF40HC(3.0)-30DS | outer end | 0.36 (J6.3) | 2.5 |
| SW1 (upper, top) | KingTek DSHP02TS | outer end | 0.90 (SW1.2) | 0.41 |

Pad 1 itself is 0.20 mm from every triangle's tip, stroke included, but
J3's (1.1 mm, the clip's outline between). No mark on parts without a
pin 1 orientation: the 0402/0603 passives, the resistor arrays (four
equal, isolated resistors), the USB-C receptacle and the test pads. The
diodes D2/D3 keep their stock cathode bracket, a polarity mark, not a pin
1 mark.

The RGY's triangle (one footprint for U2-U4) is at pad 1's outer end, on
its axis: tip (-0.75, -2.725), 0.275 mm past the pad's end (0.20 mm from
its copper, stroke included), base 0.35 mm further out, the SOT-23's
geometry. Two neighbours are placed to clear it:
- **U2 and C8.** C8 (U2's 1 uF, underneath) is at y 6.48 (`fans.C8_Y`),
  the least that puts its pads 0.17 mm past the triangle's base (stroke
  included) and its body 0.23 mm. Its supply pad is on the +3V3_BUF run
  down the channel's left side, which leaves U2 pin 1 straight down,
  under the triangle, into C8's supply pad: 1.48 mm of track pin 1 to pad
  centre, with the +3V3_BUF via joining at pin 1's stub. Pin 1 is DIR,
  tied to +3V3_BUF; the loop that matters is VCC's, pin 20 to C8,
  2.83 mm, plus C8's 0.65 mm to its own ground via, level with the ground
  pad. Against C8 0.37 mm higher, that is about +0.05 nH, negligible. The
  +3V3_BUF via is at (6.0, 5.5), under the triangle's left corner
  (tented; it carries the supply from C13 on top).
- **U4 and J1.** U4 is at (7.2, 23.1) (`fans.U4_X`), so its triangle is
  0.167 mm from J1 pin 10's copper and 0.06 mm clear of J1's top Fab
  column (the pin tails' fillets), stroke included. U4's courtyard fills
  the channel, so J1's top courtyards are 0.10 mm round the pads (its
  footprint; the top carries only the pin tails' solder fillets, no
  body; the insulators' courtyards underneath are larger), leaving U4's
  courtyard 0.01 mm from J1's and its pads 0.36 mm from J1 pins 11 and
  12. U4's B row jogs 0.2 mm left just off the pads into the runs up to
  J5; pin 10's via is at (9.6, 24.35). C10's pad leaves C10's two vias
  where they are, so pins 20 and 1 reach them over 0.71 mm of track each
  (about +0.06 nH on a 1-1.5 nH loop, under 5 % more bounce, negligible).

Stitching keeps off every pin 1 mark (`gen_layout.py pin1_marks`) and
tries every 5 degrees round a via. J3's mark is outside the SOICbite's Fab
outline, the area the test clip covers, past that outline at pad 1's inner
end (its row end is NRST's via), 1.1 mm from pad 1. The X2SON's row-end
triangle crosses the line of the body's top edge, so that silk line starts
at the package's centre.

**The check.** `tools/check_pin1.py`, run by `make layout-drc` after DRC
(so by `make jlc`, `make panel` and `scripts/release_hw.sh`), fails the
build if any pin 1 mark - every filled silk polygon of a footprint's -
comes within its stroke of a part's body on its side of the
board: its own or a neighbour's. A body is what that side's Fab layer
draws: each rectangle, polygon and circle, and the convex hull of each
run of joined lines. It also fails a footprint's mark that does not
point in toward its own package: the tip (the vertex opposite the base,
the longest side) must be nearer that footprint's body than the base's
midpoint is. And it fails a footprint's mark that does not point at its
pad 1: with the mark's direction taken from the base's midpoint to the
tip, the tip must lie on pad 1's axis (no further from the line through
pad 1's centre in that direction than the pad extends either side of it,
measured across the direction), pad 1's centre must lie ahead of the
tip, and the tip must be nearer it than the base's midpoint is. A
filled silk polygon of the board's own fails outright: the module's pin
1 is the chamfer below, and the check fails a board whose outline's
corner nearest socket pin 1 (J1's pad 1, read from the lower board, in
the boards' shared frame) is not cut at 45 degrees, at least 0.8 mm
along each edge.

**The module's pin 1: a chamfered corner.** Plugging the module in
reversed puts 5 V and ground on the wrong socket pins, so its pin 1 end
must show with it seated. A DIP shows it by its shape (the notch, the
dimple), and so does the module: each board's corner at socket pin 1's
column on its pin 1 end is cut off at 45 degrees, 1.0 mm along each
edge (`CHAMFER` and spec `"chamfer"` in `tools/gen_layout.py`). Nothing
on either board's silk marks the module: a silk triangle at the upper
board's left edge, level with socket pin 1, would read as the pin 1 mark
of R24/R25 beside it.
- **Upper:** the J3 end's left corner (x 1.49, y −4.0), above socket pin
  1's column. The clip's jaws take the end edge's middle 7.8 mm (J3's
  holes' span plus 1 mm each side), which leaves 1.61 mm at each corner:
  the 1.0 mm cut stays 0.61 mm out of it. Nearest to the cut: the corner
  ground stitch (0.32 mm, copper), NRST's via (1.43 mm), J3's pad
  1 (2.0 mm); no silk within 2 mm (`PROG` is mid-board, R24/R25 5 mm
  down the edge). It shows from above with the module seated, beside the
  programming end.
- **Lower:** the corner beside J1 pin 1 (x 1.49, y 0). J1's pin 1 pad
  (1.6 mm, centred 1.70 mm from both edges, 0.90 mm from each) keeps
  0.90 mm of copper from the cut too, well past the 0.30 mm copper-to-edge
  rule, so the ring is untouched; the corner ground stitch is 0.32 mm
  from it and J1's own triangle 0.44 mm (ink). The rule would allow
  about 1.5 mm; 1.0 mm matches the upper's, so both read alike.
- **Copper.** Each board's corner ground stitch is the nearest place
  `stitch()` allows clear of the cut (copper to edge 0.30 mm): upper
  (2.435, −3.178), lower (2.725, 0.535). No track is near either corner.
  The planes follow the chamfered outline. The routing fingerprint hashes
  the outline's rectangle, so the cut needs no reroute; `stitch()` keeps
  stitches clear of it, `--amend` moves any stored stitch it cuts, and
  `--amend` refuses if the cut ever reaches routed copper.
- **Panel.** JLC routes the cut with the rest of the outline. The upper
  board's J3-end tab starts 1.5 mm in from the end to clear it
  ("Assembly panel").
- **Fit.** The lower board only loses a corner: it stays inside the
  socket footprint's 11.02 x 26.26 mm.

The lower board's J1 triangle is hidden once the boards are stacked.

The upper board's top is the only face that shows with the module seated,
so it also carries:
- the name, `FastPROM rev A`, up the strip between C4/U6 and X1/TP3. The
  full `FastPROM rev A UPPER` is 10.5 mm of ink at JLC's narrowest
  legible text, more than the 10.0 mm the strip leaves between C16 and
  the receptacle's pads at 0.15 mm each end, and no other strip on top is
  that long; the top of the upper board is the face that shows once
  stacked, so it needs no `UPPER`. The underside keeps the full name;
- `IMG` below the image-select switch SW1's pins 3-4, naming what the
  switch does. The part itself prints `ON` and the switch numbers 1 and
  2, and the firmware reads switch 1 as image bit 0 and switch 2 as bit
  1, so the board doesn't repeat them;
- `PROG` between J3 and SW1. The pinout is the clip's
  (docs/06-build-and-flash.md); the pads carry no names;
- each test pad's rail, `5VU` and `3V3M`. The lower board's pad is
  labelled `3V3B` on its top, just below the pad, up the left strip,
  ahead of the name.

References are on the fab layers only. Everything is data in
`tools/gen_layout.py` ("silk"), but for the pin 1 triangles, which are
the footprints'. The rules it keeps to, and how DRC enforces them, are in
"Silkscreen rules" below.

The text is 1.0 mm high, 0.6 mm wide (condensed to fit the free strips)
and drawn with 0.17 mm strokes. 0.6 mm is the narrowest it can be: KiCad
draws a character's strokes no wider than a quarter of its width, so at
0.55 mm the strokes would print 0.1375 mm, under JLC's 0.15 mm. Underside text is mirrored. Measured clearances (ink to pad
copper, ink to the nearest other legend, ink to the board edge):

| Item | Where | Nearest pad | Nearest legend | Edge |
|---|---|---|---|---|
| `3V3B` | top, left strip, vertical, below TP1 (y 5.5-7.7) | 0.21 (U3 pin 13) | 0.22 (U3) | 2.7 |
| `FastPROM rev A LOWER` | top, left strip, vertical (y 8.1-18.5) | 0.20 (U3 pin 16; R21 0.20) | 0.22 (U3) | 2.7 |
| `© 2026` / `mxshift` | underside, between the pin strips, y 11.5-15.1 | 1.30 (J1.5) / 0.32 (R15) | 0.40 (each other) | 3.8 |
| `PROG` | top, between J3 and SW1 (y −1.07 to 0.08) | 0.32 (J3's hole) | 0.43 (J3's mark) | 2.93 |
| `FastPROM rev A` | top, vertical, x 5.2-6.3, y 11.6-18.7 | 0.42 (C4.1) | 0.19 (`3V3M`) | 3.7 |
| `IMG` | top, below SW1's pins 3-4 | 0.21 (SW1.3) | 0.35 (SW1) | 0.86 |
| `3V3M` | top, between C15 and TP3 | 0.21 (D2.2) | 0.19 (the name) | 3.8 |
| `5VU` | top, right of TP2 | 0.43 (TP2) | 0.55 (D3) | 0.72 |
| `FastPROM rev A UPPER` | underside, left strip | 0.47 (R18) | 1.84 | 0.34 |
| `© 2026 mxshift` | underside, right strip | 1.39 | 0.87 (J6's mark) | 0.29 |

The underside's name lies beside the mid-board tab's window, 0.34 mm
from the edge; the mouse bites stand outside the edge and reach 0.05 mm
in, so breaking a tab can't reach the silk.

### Silkscreen rules

**Decision.** DRC enforces JLC's legend rules (jlcpcb.com/capabilities/
pcb-capabilities) on both boards and the panel, at the severity KiCad
gives each check, with no exclusions:

| Rule | Value | How KiCad checks it |
|---|---|---|
| Legend to legend | 0.15 mm | board setup `min_silk_clearance` (silk_overlap) |
| Legend to board edge | 0.15 mm | the same setting: KiCad's edge check takes the silk clearance for silk (silk_edge_clearance) |
| Legend to pad (its mask opening) | 0.15 mm | custom rule `silk to pad`: `silk_clearance` 0.15 mm where B is a pad (silk_over_copper) |
| Line, arc, circle and polygon stroke | ≥ 0.15 mm | custom rule `silk line width`: an assertion `A.Line_Width >= 0.15mm` on every silkscreen graphic (assertion_failure) |
| Text height | ≥ 1.0 mm | board setup `min_text_height` |
| Text stroke | ≥ 0.15 mm | board setup `min_text_thickness` (1:6 of 1.0 mm is 0.167; the text uses 0.17) |

JLC states no edge distance for the legend; 0.15 mm is its pad and
legend spacing applied to the edge, inside JLC's ±0.2 mm routing
tolerance, where a trimmed legend costs nothing. A custom edge rule
would have to be a `silk_clearance` rule too, and KiCad then also runs
its legend-to-legend check against the edge line's 0.1 mm stroke, so the
rule would mean two distances; the board setting means one.

The values are `RULES` in `tools/gen_layout.py`. `silk_settings()` puts
the board-setup ones in the board, `project_rules()` in the project file
(the template `gen_kicad.py` writes leaves the silk clearance at zero),
and `silk_rules()` writes the two custom rules to
`hw/<board>.kicad_dru`, which KiCad reads beside the board; the panel
gets the same files through `gen_panel.py`. KiCad's board setup has no
setting for either: with the silk clearance set alone, its silk-to-mask
check measures against zero. `scripts/release_hw.sh` checks the rule
files are what the generator writes and packages them with the boards.

**Footprints.**
- Every project footprint's silk stroke is 0.15 mm, and none comes closer
  than 0.15 mm to a pad.
- J4's side lines start 0.15 mm clear of the rear shell pads and stop
  0.25 mm inside the edge (their ink 0.175 mm).
- The stock passives, diodes, resistor arrays and test pads take
  project copies under their stock names (`hw/lib/fastprom.pretty/
  README.md`): the 0402s without their outline (two 0.3 mm dashes
  between the pads, which cannot stand 0.15 mm off them at any length
  that prints), the 0603s and arrays without theirs too, so no passive
  carries an outline (JLC places from the CPL, checked in its preview;
  orientation marks stay), the SOD-323s' cathode brackets at 0.15 mm, and
  the test pad's ring 0.75 mm in radius at 0.15 mm, 0.175 mm off its
  pad. The schematics name them `fastprom:...`, as they do the other
  project footprints.

**Result.** Both boards and the panel: DRC with every severity and the
schematic parity check reports nothing.

### USB-C land pattern: HRO's drawing, not KiCad's stock

**Decision.** J4's project footprint follows HRO's own drawing for the
TYPE-C-31-M-12 (rev A, 2020-12-08: "Recommend P.C.B layout", tolerance
±0.05, on LCSC C165948's datasheet), which differs from KiCad's stock
footprint:
- **Signal pads:** 1.14 mm long, front ends 0.50 mm behind the peg-hole
  line (stock: 1.45 mm, 0.72 mm). That puts the row 0.375 mm further back
  than the stock row, centre to centre.
- **GND and VBUS pads:** at ±3.20 and ±2.40 (stock ±3.25 and ±2.45).
- **Peg holes:** Ø0.60 mm for the Ø0.50 mm pegs (stock 0.65 mm).
- **Shell slots:** 0.50 mm behind and 3.68 mm in front of the peg holes
  (stock 0.53 and 3.65).
- **PCB edge:** 5.79 mm in front of the peg holes.

The stock footprint's peg holes and slots are within 0.03 mm of HRO's, so
the pegs would not bind, but its signal row would leave the leads' heels
0.22 mm off their pads. J4 sits so the board edge falls where HRO draws
it. JLC's EasyEDA footprint agrees with HRO on the holes and slots, and
the CPL offset for C165948 aligns its peg holes on ours
(`tools/jlc_parts.yaml`). Courtyard overhang past the edge: 0.96 mm.

## Open, to measure on the first boards

- Ground bounce at the enable event (above).
- U7's ramp against the LVT245B's 200 µs/V recommendation.
- Fabric configuration time after power-on, against the host's reset.
- The firmware's first boot: the items in `docs/06-build-and-flash.md`,
  "First-board bring-up".
- UART recovery of a blank board through the mask ROM: the list at the end
  of "UART recovery", above.
- tAA, tEA and tER at the socket, against the *model* figures.
- USB hot-plug overshoot at U5's input.
