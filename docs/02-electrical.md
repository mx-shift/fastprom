# Putting a 3.3 V part in a 5 V TTL socket

This is the hardest constraint in the project, and it is not negotiable:

> AG32 absolute maximum input voltage: **VSS - 0.3 V to VSS + 3.6 V**.
> No 5 V-tolerant pin class is documented anywhere in AGM's material.

The sockets being targeted are 5 V TTL. Address and chip-enable lines are
driven by whatever the host uses - 74LS outputs sit around 3.4 V, but 74HC and
74HCT drive to within a few hundred millivolts of 5 V, and some hosts use
pull-ups to 5 V. Data lines are worse: if the socket's data bus is shared with
anything else, another device will drive 5 V onto pins that, when our part is
deselected, are inputs.

Connecting an AG32 directly to that bus exceeds its absolute maximum rating.

## Options considered

### 1. Direct connection

Fastest - it removes about 7 ns from the round trip - and wrong by default. It
is only defensible when every driver on every connected line has been measured
and shown to stay under 3.6 V, which in practice means a host built entirely
from LS parts with no 5 V pull-ups and an unshared data bus.

The build supports it (`--iface direct` in `tools/timing_budget.py` shows the
numbers) because for some boards it is genuinely safe and the speed is free.
It is not the default and it is not recommended.

### 2. Series resistor into the internal clamp

The popular retro-hardware shortcut: a series resistor per line, letting the
pin's ESD clamp conduct the difference. AGM specifies an injected current
limit of +/-5 mA, so surviving a 5 V drive needs

```
R >= (5.0 - 3.6) / 5 mA = 280 ohm    ->  say 470 ohm
```

Two problems. First, 470 ohm into the bus capacitance is slow: at a
conservative 20 pF that is about 9 ns of added rise time, which costs more
than a proper buffer. Second, and worse, it depends on the clamp diode
conducting continuously during normal operation, which is not what a clamp is
specified for - AGM rates injected current as a fault condition, not a
steady-state operating mode. Current pushed into the clamp also has to go
somewhere, and on a lightly-loaded 3.3 V rail it can lift the supply.

Not used.

### 3. Level translation through 5 V-tolerant 3.3 V logic

The design uses **SN74LVT245B**. Its figures, which are the same as the
bus-hold variant SN74LVTH245A's:

| Parameter | Value |
|---|---|
| tPLH / tPHL, VCC 3.3 V +/- 0.3 V, over temperature | **3.5 ns max** |
| Output enable, max(tPZH, tPZL) | **5.5 ns max** |
| Output disable, max(tPHZ, tPLZ) | **5.9 ns max** |
| Input voltage, recommended operating | **5.5 V** |
| Input voltage, absolute maximum | 7 V |
| Voltage on an output in high-Z or powered-off state | 7 V |
| IOL / IOH | **64 mA / -32 mA** |

That last pair matters as much as the voltage rating. It means the interface
is not a compromise against the original part - it drives harder than the
bipolar PROM did. The envelope these parts need is sink 16 mA at 0.45 V and
source 6.5 mA at 2.4 V, taken across every vendor's worst case; 64 mA and
32 mA clear it comfortably.

### One limit on the 5 V tolerance that matters

The tolerance is asymmetric, and the datasheet is precise about it:

| Condition | Absolute maximum |
|---|---|
| Input voltage | 7 V |
| Voltage on an output in high-Z or powered off | 7 V |
| Voltage on an output **driving high** | **VCC + 0.5 V** |

So the part tolerates 5 V as an input, while released, and while pulling low -
but when it actively drives high it drives to 3.3 V, not 5 V.

That is fine for a **5 V TTL** bus, where VIH is 2.0 V, which is what these
sockets are. It is **not** sufficient for a **5 V CMOS** receiver, where VIH is
0.7 x VCC = 3.5 V. If a host's receivers on these data lines are HC rather
than HCT or LS, 3.3 V will not reliably read as a one and this interface is
the wrong one.

Check it on any given board. The original part was a bipolar TTL output
with VOH specified at only 2.4 V minimum, so a host that worked with the
original cannot have been relying on a true 5 V CMOS level, which makes this
mostly a theoretical concern.

The cost is 3.5 ns in each direction on the address path, about 7 ns of
the budget, and on the enable path the drive buffer's 5.5 ns output enable
plus the receive buffer's 3.5 ns. It is still the right trade: it converts an
out-of-spec design into an in-spec one, and address access still meets the
fastest 30 ns grades. The enable path is where it hurts: tEA is 18.5 ns
against the fastest parts' 20 ns, and only with the enable on a pad next to
the chip-enable input (`03-timing-budget.md`).

`SN74LVC245A` is not used: 6.3 ns tpd and 8.5 ns enable, the latter landing
on top of the enable-to-output times these parts specify. It is kept in the
tool as a comparison.

**SN74LVT245B rather than the LVTH variant.** Its switching characteristics
are identical (3.5 ns, 5.5 ns and 5.9 ns, from its own datasheet). It has no
bus hold. The `LVTH` suffix means active bus hold on the A and B ports, which
holds an undriven input at its last state and takes 500 uA (low to high) or
750 uA (high to low) of overdrive to switch (SN74LVTH245A datasheet, II(hold)
at 3.6 V). The 100 kOhm socket pulls supply 33-50 uA, so on the lines we
*receive*, address and chip enable, a floating pin would keep whatever level
it last had instead of the pull's; TI advises against pull resistors with bus
hold.

## Direction and arrangement

A PROM only ever drives its data lines, so the arrangement is fixed:

| Signals | Count (512x8) | Direction | Buffer |
|---|---:|---|---|
| Address A0..A8 | 9 | host -> AG32 | receive |
| Chip enable(s) | 1-4 | host -> AG32 | receive |
| Data O0..O7 | 8 | AG32 -> host | drive, with OE |

Two '245s cover the 10-13 receive lines, one covers the 8 data lines. The data
'245's OE comes from the fabric's enable decode, so the bus is released
whenever the part is deselected or the image is not yet loaded.

The receive buffers are permanently enabled in one direction. Their outputs
face the AG32 only, so nothing on the host side ever sees them go high-Z.

### Series termination

The interface can drive considerably harder than the part it replaces, and
hard edges into an unterminated trace ring. A 22-33 ohm series resistor per
data line is cheap insurance and costs about a nanosecond (33 R into 50 pF,
R x C x ln 2 = 1.1 ns; less into a 30 pF test load). 33 R is fitted (RN1,
RN2) and carried in the timing budget.

### Open-collector parts

Some 20-pin PROMs have open-collector outputs and must never source current.
A '245 cannot do this - it drives both rails, and its output enable is common
to all eight bits rather than per-bit.

**The module cannot emulate them.** Its lower board drives the data lines
through a '245 (U4), so it is tri-state only. An open-collector build would
need a lower board with open-drain buffers in U4's place. `74LVC07A`-class
parts work: open-drain outputs rated to 5.5 V, so the host's pull-up still
defines the high level and nothing sources current. No such board exists;
the parts that look right and are not are in `07-bom.md`.

The fabric side supports this. `fastprom_enable` computes a per-bit
output enable rather than a single common one, which is exactly the form an
open-drain buffer needs: feed it `selected ? data : 1` and it pulls low on a
zero and floats on a one. `sim/tb_oc.v` verifies the behaviour.

## Power

Bipolar PROMs are power hungry - they are fundamentally arrays of fusible
links read through sense amplifiers, and they burn current whether or not
anything is being read. An 82S147 draws well over a hundred milliamps.

The module draws far less: an AG32 plus three bus buffers is a few tens of
milliamps, and a host's 5 V rail does not care about being loaded less. A
single small LDO from the socket's 5 V
rail is sufficient, with plenty of headroom.

Requirements for the regulator:

* 5 V in, 3.3 V out, 150 mA or better
* small package, since the board is a DIP-20 footprint
* stocked at LCSC, for JLC assembly

There are two: the AP2112K-3.3 (U5) for the AG32 on the upper board,
behind the socket/USB OR-ing diodes, and a TLV74233 (U7) for the bus buffers
on the lower board, fed from socket pin 20 only (below).

AGM specifically recommends feeding the AGRV2K's 3.3 V rail through its own
branch isolated by a ferrite bead rather than sharing a plane with other
loads. The bus buffers have their own regulator on the lower
board, so the other loads on the upper board's +3V3 are the image-select
switch, its resistors, and J3's 3V3 pin - where a programming fixture
connects. FB1 (220 R at 100 MHz) puts all the AG32's supply pins - VDD33 on
16 and 32, VDDA33 on 6 - and its oscillator on +3V3_MCU, behind the bead,
which is AGM's recommendation as written: the whole AG32 rail isolated, not
VDDA alone. The bead and its roughly 6-11 uF form an LC near 100 kHz with a
Q of 3-4, harmless at tens of mA.

Decoupling: bulk capacitance at each regulator's input and output, 100 nF
per AG32 supply pin placed as close as the QFN's pad pitch allows (plus 1 uF
at VDDA and 10 uF bulk behind the bead), 100 nF per receive buffer, and 1 uF
at the drive buffer U4, which switches all eight data lines at once. The
reasons and the loop lengths are in `../DESIGN_NOTES.md`.

## The configuration window, and why the bus is safe during it

The AG32's fabric is SRAM-based and configured from flash at power-on. During
that window there is no logic, so nothing exists to hold the data buffer off -
and a module that drives a host's bus with garbage for a few milliseconds at
every power-up is worse than one that does nothing.

Two things make the bus safe here, and neither needs the fabric to be
running.

**The drive buffer's enable is pulled up to the buffer's own supply.**
`D_BUF_NOE` is active low, and R21 (10 k, on the lower board next to U4)
holds it at +3V3_BUF. The SN74LVT245B's power-up 3-state only guarantees
high-impedance outputs while its supply is below 1.5 V; above that OE
decides, and TI's condition is that OE be "tied to VCC through a pullup
resistor" - the device's own VCC. Pulled to the buffer's rail, OE rises with
it from the first millivolt. The enable is on the AG32's PIN_14, which floats
until the fabric is configured, so nothing fights R21 in that window. The
fabric sinks 0.33 mA through it once it starts driving, which is nothing.

With the host off and the module on USB, the AG32 driving the enable high
sources at most 0.33 mA through R21 into the buffers' unpowered rail, which
U7 holds near the dead host's 5 V. That powers nothing, and keeps the
buffers below the 1.5 V at which their outputs are high-impedance anyway.

**The buffers power up after the AG32.** Both regulators start from the
socket's 5 V, but U7 (the buffers', 1.3 uF to charge) would naturally win:
the AG32's AP2112K is behind the OR-ing diode, has some 20 uF to charge and
folds back to 50 mA into a discharged output. U7's enable is therefore
delayed, by R26 and C17 (100 k, 100 nF: tau 10 ms, threshold 0.9 V, about
2 ms after a 5 V step and later still on a slow ramp). The AG32's rail
leading matters because its unpowered I/O could otherwise clamp the enable
line toward its own low supply while the buffers were already up.

Not settled by the datasheets: the SN74LVT245B recommends a power-up ramp no
faster than 200 us/V, and U7 ramps in about 100 us once enabled. With OE held
at the buffer's own supply that should not matter, but TI does not state it
below 1.5 V.

AGM's pinout spreadsheet, whose "before configuration" column the datasheet
does not carry, shows two QFN-32 user I/O that come up with an internal
pull-up: PIN_26 (JTDI) and PIN_28 (JNTRST). Putting the enable on one of them
would add a further guarantee in silicon. Both carry data lines instead, and
the enable is on PIN_14, next to the chip-enable input, because the
enable path's timing needs it (`../DESIGN_NOTES.md`, "D_BUF_NOE on PIN_14").

So the module presents high impedance to the host from power-on until an
image is verified and the fabric enables the buffer, and the board enforces
it.

What remains is a *liveness* problem: a host that
starts fetching from this socket before configuration completes reads nothing
useful. The configuration time is still unpublished and needs measuring
against the host's own power-on reset delay. Most host boards hold reset for
tens or hundreds of milliseconds through an RC or a supervisor, which is
likely to be ample, but it is unmeasured and is the first measurement to
take on real hardware.
