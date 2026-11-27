# Choosing the AG32 variant

Every AG32 carries the same AGRV2K fabric and the same RV32IMAFC core with
128 KiB SRAM. The choice is therefore about three things only: how many fabric
I/O the package exposes, how big the package is relative to a DIP-20
footprint, and whether its pinout is actually documented.

## Pin budget

A 20-pin PROM spends two pins on power, so at most 18 pins carry signal. With
the level translators from `02-electrical.md` in the path, the AG32 needs:

| Function | Pins | Notes |
|---|---:|---|
| Address inputs | up to 11 | 10 for 1024x4, 9 for 512x8 |
| Chip enables | 1-4 | part dependent |
| Data lines | 4-8 | |
| Data buffer output enable | 1 | drives the '245 OE |
| UART programming and recovery | 2 | TX/RX; on the stack they also read the two image-select bits |
| USB D+/D- | 2 | hard PHY, fixed pins |
| USB clock into OSC_IN | 1 | the PLL needs an accurate external reference for USB's 60 MHz |

The socket side can never exceed 18, so the stack needs **18 + 1 + 2 + 2 + 1
= 24** fabric I/O. Image select costs no pins of its own: the two bits are
read from the UART pins through a DIP switch (`../DESIGN_NOTES.md`). There
is no status LED.

Configuration does not eat into this: on AGRV2K only TMS and TCK are consumed,
and NRST, BOOT0 and the supplies are dedicated pins outside the user I/O
count.

## Candidates

| Part | Package | Body | Fabric I/O | Pinout documented? | LCSC | ~$1 |
|---|---|---|---:|---|---|---|
| AG32VF303KCU6 | QFN-32-EP | 4.0 x 4.0 mm | 24 | not in the datasheet; in AGM's pinout spreadsheet (below) | C41397171 | 1.64 |
| AG32VF303CCT6 | LQFP-48 | 7.0 x 7.0 mm | 32 | yes | C7541224 | 1.88 |
| AG32VF407RGT6 | LQFP-64 | 10 x 10 mm | 47 | yes | C5336267 | 2.73 |

## Mechanical fit

The AG32 sits on the upper board, which is the socket's 11.02 mm width
(`09-two-board-stack.md`). The LQFP-48's 9 x 9 mm lead span would take most
of that width; the QFN-32 takes 4 mm and leaves the rest of the underside
for the DF40 socket and the regulator. The LQFP-64, at 10 x 10 mm, is larger
still and buys I/O the design cannot use.

The lower board carries the bus buffers in the socket's own footprint. A
DIP-20 is 25.4 mm long with 7.62 mm between the two pin rows. The 20 pins
are round machined pins in two 10-way strips whose insulators sit *under* the
board (see `05-hardware.md`), so on the top surface the obstruction is the
two rows of through-hole pads themselves, and nothing can be placed on top
of those.

That leaves, on the top side of a board in the socket's footprint:

| Region | Size |
|---|---|
| Channel between the pad rows | **5.08 mm wide x 22.86 mm long** |
| Strips beyond the ends of the pin field | about 1.7 mm x 11 mm each |
| Outer margins beside each pad row | 1.19 mm - too narrow to place anything |

Every part on top has to fit in that 5.08 mm channel, which is why the BOM
calls for VQFN buffers rather than the more convenient packages: a 20-VQFN
at 3.5 mm wide fits the channel, while a 20-TSSOP (6.4 mm across its leads)
and a 20-SSOP (7.8 mm) do not. See `07-bom.md`.

## Pinout source

The MCU datasheet and the selection guide cover LQFP-100, LQFP-64 and
LQFP-48 only. The QFN-32 pinout comes from AGM's pinout spreadsheet,
`AG32_pinout_100_64_48_32_2K.xlsx` (its URL is in `00-ag32-reference.md`;
not redistributed here), which covers all packages and is the authoritative
source.

The MCU reference manual gives QFN-32 pin 30 as BOOT0 and the AGRV2K CPLD
datasheet gives it as GND. The spreadsheet has one column per variant, and
pin 30 is **BOOT0 on `AG32VFxxxK`** (the MCU parts, which is what
`AG32VF303KCU6` is) and **GND on `AGRV2KQ32`** (the CPLD-branded SKU); each
document describes its own SKU of the same die.

The full pinout, the I/O budget and the reset states are in
`00-ag32-reference.md`, and the board mapping is in `04-board-mapping.md`.
24 usable fabric I/O remain after configuration takes JTMS and JTCK, and
the stack uses all 24, so there is no pin left for a status LED.

## Recommendation

**The part is the QFN-32 AG32VF303KCU6.** It is also the cheapest part in
the family with the most stock.

The open AGaMEMnon toolchain's silicon evidence is LQFP-48 only, and for
the QFN-32 it supports only builds that assign no pads, because the bond map
there is recovered from metadata rather than verified. The stack's
bitstreams therefore come from AGM's Supra (`06-build-and-flash.md`). The
first measurements - fabric configuration time, M9K access time, the socket
timings and the edge quality - are taken on the stack itself
(`../DESIGN_NOTES.md`, "Open, to measure on the first boards").

Do not substitute the AGRV2K-branded SKUs. AGRV2KQ32 is the same die at within
0.1 % of the same price in every quantity tier, with the MCU nominally
dormant - there is nothing to gain and a working RISC-V core to lose.
