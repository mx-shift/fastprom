# AG32 / AGRV2K reference figures

Every number this project's analysis depends on, with its source and how much
weight it carries. Figures are tagged:

* **[VENDOR]** from an AGM datasheet or manual
* **[MEAS]** measured on silicon or recovered from vendor tooling by the
  [AGaMEMnon](https://github.com/bbenchoff/AGaMEMnon) project
* **[INFER]** arithmetic or deduction of ours, stated as such

Primary sources:

* AGRV2K Datasheet Rev 3.0 — `https://www.agm-micro.com/upload/userfiles/files/AGRV2K_Rev_3_0.pdf`
* AG32 MCU Datasheet Rev 1.0 — `https://www.agm-micro.com/upload/userfiles/files/AG32_DATASHEET_202303.pdf`
* AG32 MCU Reference Manual Rev 1.1 — mirrored at `https://archive.org/download/agm-micro/Docs/AG32%20MCU%20Reference%20Manual.pdf`
* AGRV2K Application Guide v4.2 — `https://archive.org/download/agm-micro/Docs/MANUAL_AGRV2K_4.2_Native.pdf`
* AGaMEMnon open toolchain and its recovered device database

* **Pinout spreadsheet, all packages including QFN-32** -
  `https://www.agm-micro.com/upload/userfiles/files/AG32_pinout_100_64_48_32_2K.xlsx`
  (AGM's copyright; not redistributed here.) This is the authoritative pin
  source and it covers more packages than the datasheet does.

The whole AGM document set is mirrored at `https://archive.org/details/agm-micro`.

## Part options

All AG32 variants carry the same AGRV2K fabric and the same RV32IMAFC core
with 128 KiB SRAM; packages, flash and analog channel counts differ. [VENDOR]

| Part | Package | Fabric user I/O | Flash | LCSC | ~$1 / ~$1k |
|---|---|---:|---:|---|---|
| AG32VF303KCU6 | QFN-32-EP 4x4, 0.4 mm | 24 | 256 K | C41397171 | 1.64 / 0.96 |
| AG32VF303CCT6 | LQFP-48 7x7 | 32 | 256 K | C7541224 | 1.88 / 1.13 |
| AG32VF407RGT6 | LQFP-64 10x10 | 47 | 1024 K | C5336267 | 2.73 / 1.54 |

The AGRV2K-branded SKUs (AGRV2KQ32 etc.) are the same die at within 0.1 % of
the same price, so there is no reason to prefer them. [INFER]

Not available from DigiKey or Mouser - AGM is not franchised by either. LCSC
is the practical channel, MOQ 1. [INFER, high confidence]

## Fabric resources

| Resource | Value | Tag |
|---|---|---|
| Logic | 2112 LUT4 + 2112 FF (132 tiles x 16 slices) | [MEAS] |
| Block RAM | 4 x M9K, 8192 bits each = 32768 bits / 4 KiB | [VENDOR] |
| Block RAM placement | all four in one column, tiles X13Y1..X13Y4 | [MEAS] |
| Multipliers / DSP | none | [VENDOR, absent] |
| PLL | 1 (PLLVE), output 2-300 MHz, input 4-50 MHz | [VENDOR] |
| Global clocks | 4 per datasheet, 5 in the recovered database | [VENDOR] / [MEAS] |
| Internal oscillator | 8 MHz RC, usable as a fabric clock | [VENDOR] |

## Fabric timing primitives

From AGRV2K Datasheet Rev 3.0 section 4. [VENDOR]

| Symbol | Parameter | Min | Max |
|---|---|---:|---:|
| T_IN | input pad + buffer | 0.767 | **1.308** |
| T_DO | output buffer + pad | 1.421 | **2.140** |
| T_LUT | LUT4 combinational delay | - | **1.111** |
| T_LOCAL | local routing | - | 0.434 |
| T_R | row interconnect, average load | - | 0.523 |
| T_C | column interconnect, average load | - | 0.679 |
| T_CO | register clock to output | - | 0.455 |
| T_SU | register setup | 0.664 | - |
| T_XZ | output disable | 0.867 | 1.322 |
| T_ZX | output enable | 1.242 | 1.532 |

All in nanoseconds.

**Caveat that matters.** These tables are byte-identical to the ones in AGM's
datasheet for the much smaller AG1K family, and no process, voltage or
temperature corner is stated. They are shared architectural boilerplate rather
than per-device characterisation. [INFER, high confidence] Treat them as
indicative and keep margin.

Worst-case routing is considerably worse than the "average loading" figures
above. Recovered per-mux worst-case delays include RMUX 1.175 ns and BufMUX
1.0 ns, against an exactly-pinned 2-hop OMUX->IMUX of 0.401 ns. [MEAS] The
budget in `03-timing-budget.md` is computed both ways.

## Fabric clock ceiling

No vendor fMAX for the fabric exists in any document. The available proxies:

* PLL output electrical maximum 300 MHz [VENDOR]
* vendor static timing analysis scored **119.474 MHz** on a real wide design
  at a 32-bit AHB boundary [MEAS], the closest to what a real design reaches
* the AGM reference board fits a 50 MHz oscillator for the fabric [VENDOR]
* RISC-V core maximum 248 MHz [VENDOR]

This project plans on **100 MHz as a comfortable ceiling and 50 MHz as
conservative**. [INFER]

## Block RAM behaviour

Quoting the datasheet directly: "M9K memory blocks do not support
asynchronous (unregistered) memory inputs." [VENDOR]

So an asynchronous address-to-data read straight out of block RAM is not
possible; the address is always captured on a clock edge. Base latency is one
clock, with the address registered and the array output combinational.
`CFG_SELOUT_A` optionally registers the output for one more clock. [MEAS]

**The M9K clock-to-data-valid time is not published anywhere.** Supra's own
timing model, the one its static timing analysis uses, has it: **3.24 ns**
from the read-port clock to `DataOutB`, combinational output (seen in every
BRAM-mode run of `tools/ag32/run_flow.sh`). [MEAS, vendor model - not a
silicon characterisation] BRAM mode depends on it, so it is the first
figure to check on hardware.

## Pad and routing delays in Supra's model

From Supra's timing reports for the QFN-32 (`AGRV2KQ32`), which differ from
the datasheet primitives above in ways that matter for a pad-to-pad design:
[MEAS, vendor model]

| Element | Supra | Datasheet |
|---|---:|---:|
| Input pad to fabric | 1.307 ns | T_IN 1.308 ns |
| Output pad | 3.481 ns | T_DO 2.140 ns |
| Route from a LUT into an output pad | 3.1-3.4 ns | (T_C 0.679 ns) |
| LUT, input to output | 0.09-0.56 ns | T_LUT 1.111 ns |

The QFN-32's pads are bonded to tiles on three sides of the 23 x 14-tile
fabric; PIN_18 and PIN_19 (and PIN_26 near them) sit at the opposite edge from
the rest. A one-LUT path between the two sides costs about 2 ns more than one
within a side; see `03-timing-budget.md`.

Supra reports a **restricted Fmax of 200.08 MHz** for every clock in these
designs - the clock-network limit, separate from path timing (which reaches
342 MHz for the block-RAM read port). [MEAS, vendor model]

## I/O electrical

| Parameter | Value | Tag |
|---|---|---|
| Supply | VDD33 3.0-3.6 V, recommend 3.135-3.465 | [VENDOR] |
| I/O standards | 3.3 V LVCMOS and LVTTL only | [VENDOR] |
| **5 V tolerance** | **none.** Absolute max VIN = VSS-0.3 to VSS+3.6 V | [VENDOR] |
| VIL / VIH | 0.8 V max / 1.5 V min | [VENDOR] |
| VOL / VOH | 0.2 V max / 2.8 V min, **no load current stated** | [VENDOR] |
| Drive strength | programmable 2-30 mA, default 8 mA | [VENDOR] |
| Absolute max per-pin current | 25 mA | [VENDOR] |
| Injected current | +/-5 mA | [VENDOR] |
| Other I/O features | open-drain, bus-hold, Schmitt input, programmable slew | [VENDOR] |
| I/O banks | one 3.3 V domain, no separate VCCIO | [INFER, high confidence] |
| Reset state | user I/O input floating during and after reset | [VENDOR] |
| Power-on reset | VPOR 2.0-2.4 V, reset temporisation 4 ms | [VENDOR] |

The programmable 30 mA drive setting exceeds the 25 mA absolute maximum, so
cap it at 24 mA. [INFER]

AGM recommends feeding the 3.3 V rail through its own ferrite-isolated branch.
[VENDOR]

## QFN-32 pinout

**Pin 30 is BOOT0** on the MCU part this project uses. [VENDOR, from the
pinout spreadsheet above] The MCU reference manual gives QFN-32 pin 30 as
BOOT0 and the AGRV2K CPLD datasheet gives it as GND; each describes its own
SKU of the same die, and the spreadsheet has a column per variant:

| Pin | Name | `AG32VFxxxK` (MCU) | `AGRV2KQ32` (CPLD) |
|---:|---|---|---|
| 30 | BOOT0 | **BOOT0** | GND |

Full QFN-32 pinout for the `AG32VFxxxK` parts, which is what
`AG32VF303KCU6` is:

| Pin | Name | Function | Before configuration |
|---:|---|---|---|
| 1 | PIN_1 | IO / RTC, **IO_GB** (global clock capable) | |
| 2 | PIN_2 | IO / OSC_IN | |
| 3 | PIN_3 | IO / OSC_OUT | |
| 4 | NRST | NRST | pull-up |
| 5 | PIN_5 | IO, ADC_IN12 | |
| 6 | VDDA33 | analog supply | |
| 7 | PIN_7 | IO, WKUP / ADC_IN0 / CMP_PA0 | |
| 8-10 | PIN_8..PIN_10 | IO, ADC / comparator | |
| 11 | PIN_11 | IO, ADC_IN4 / CMP_PA4 / **DAC0** | |
| 12 | PIN_12 | IO, ADC_IN5 / CMP_PA5 / **DAC1** | |
| 13, 14 | PIN_13, PIN_14 | IO, ADC_IN6 / ADC_IN7 | |
| 15 | PIN_15 | IO, **BOOT1** | **pull-up** |
| 16 | VDD33 | supply | |
| 17 | GND | ground | |
| 18, 19 | PIN_18, PIN_19 | IO | |
| 20 | PIN_20 | IO, **UART0_TX** | output when BOOT1=0, BOOT0=1 |
| 21 | PIN_21 | IO, **UART0_RX** | input pull-up when BOOT1=0, BOOT0=1 |
| 22 | PIN_22 | IO, USBDM | |
| 23 | PIN_23 | IO, USBDP | |
| 24 | PIN_24 | **JTMS** - fabric configuration | **pull-up** |
| 25 | PIN_25 | **JTCK** - fabric configuration | **pull-down** |
| 26 | PIN_26 | IO, JTDI | **pull-up** |
| 27 | PIN_27 | IO, JTDO | |
| 28 | PIN_28 | IO, JNTRST | **pull-up** |
| 29 | PIN_29 | IO | |
| 30 | BOOT0 | BOOT0 | |
| 31 | PIN_31 | IO | |
| 32 | VDD33 | supply | |

26 pins are named `PIN_n`; JTMS and JTCK are consumed by configuration,
leaving **24 usable fabric I/O**, which matches the datasheet's headline
figure exactly.

For the schematic: the QFN-32 has **no VBAT pin**, and only **one ground
pin** (17) against three supply pins (16, 32, and VDDA33 on 6).

### The "before configuration" column is load bearing

This column does not appear in the datasheet and it answers a question the
datasheet leaves open: what the pins do during the window after power-on while
the fabric is still being configured from flash.

Most user I/O has no entry, which means input floating. But **PIN_26 and
PIN_28 come up with an internal pull-up**, and that is directly useful: an
active-low control signal assigned to one of those is guaranteed to be
*inactive* from the moment power is applied, before any logic exists to drive
it. The board does not use this: its enable is on PIN_14,
chosen for timing (it is next to the chip-enable input on the die), and an
external pull-up holds it (see `02-electrical.md`).

## Pin assignment

The fabric owns the I/O ring, so fabric signals can be assigned to
essentially any user I/O pin - there is no fixed alternate-function matrix,
and hard MCU peripheral signals are themselves routed through the fabric.
[VENDOR] + [MEAS]

So the socket pinout is matched in the constraint file rather than in
copper.

Constraint syntax, vendor `.ve`: `<signal> PIN_<n>`, where `n` is the decimal
package lead. Open flow `.pcf`: `set_io <port> PIN_<n>`. [VENDOR] / [MEAS]

Not fabric-routable: USB D+/D-, NRST, BOOT0, and the supplies. On AGRV2K only
TMS and TCK are consumed for configuration, so the other JTAG pads remain
usable as fabric I/O. [VENDOR]

## Configuration

The fabric is SRAM-based. Configuration bits live in flash and are downloaded
automatically at power-on; the raw image is 99,936 bytes. [VENDOR] + [MEAS]

Two consequences for a drop-in replacement:

1. Configuration happens at **power-on only**, not on a warm debugger reset.
2. There is a window after power-up during which the fabric is unconfigured
   and user I/O is floating. Its length is not published. See the risk
   discussion in `03-timing-budget.md`.

## Known gaps

Things no source establishes, each of which is called out where it affects a
decision:

1. M9K clock-to-data-valid time on silicon (Supra's model says 3.24 ns).
2. Any vendor fabric fMAX.
3. PVT corners for the timing primitives above.
4. Load currents for the VOH/VOL figures, so no drive curve can be built.
5. Power consumption. The datasheet's current table is printed with every
   cell empty.
6. Fabric configuration time. The board holds the bus released through it
   (`02-electrical.md`), but its duration is unpublished.
