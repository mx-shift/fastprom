# FastPROM

A drop-in replacement for 20-pin bipolar PROMs (82S147, 63S481, 82S135,
63S281 and their kin), in the spirit of [One ROM](https://onerom.org/) but
aimed at the parts One ROM does not chase: 25-70 ns access times.

A software ROM - watch the address pins, look up a table, drive the data
pins - answers in 70-150 ns at best. Bipolar PROMs sit in palette lookups,
address decoders and microcode sequencers that need an answer in 30-60 ns,
and their enable path is faster still (tEA 20-30 ns), which designs of the
period relied on. FastPROM puts the read path in the FPGA fabric of an AGM
AG32, a RISC-V microcontroller with an AGRV2K fabric on the same die. The
processor loads the contents at boot and is then out of the path.

## Status

Rev A hardware is released
([hw-rev-A](https://github.com/mx-shift/fastprom/releases/tag/hw-rev-A))
and on order from JLCPCB: five assembled panels, 25 modules. The fabric,
firmware and update page are built and tested in simulation.

Nothing has run on silicon yet. Next: bring rev A up, measure how long
after power-on the fabric is live, and check the timing below on real
parts.

## Timing

In BRAM mode the bitstream is fixed and the firmware loads the ROM contents
from flash into block RAM at boot. The figures below are maxima from AGM's
timing model for the placed and routed design, including the bus buffers
and board-to-board connector (`docs/03-timing-budget.md`).

| | 512x8, fastest grade | 256x8, fastest grade |
|---|---|---|
| tAA, address to data | 27.5 ns against 30 ns | 27.5 ns against 28 ns |
| tEA, enable to outputs on | 18.2 ns against 20 ns | 18.6 ns against 25 ns |
| tER, enable to outputs off | 17.5 ns against 20 ns | 17.8 ns against 25 ns |

LOGIC mode instead synthesises the contents into the fabric as
combinational logic. It is faster (25.1 ns tAA on incompressible 512x8
contents), but changing the contents means rebuilding the bitstream.

## Hardware

A DIP-20 footprint leaves too little room between its pin rows for the
AG32, three bus buffers, two regulators and USB-C, so FastPROM is two boards
of the socket's width, stacked 3 mm apart on a Hirose DF40 connector:

* the **lower board** plugs into the socket and carries three 5 V-tolerant
  SN74LVT245B buffers, terminators and pulls, and the buffers' own 3.3 V
  regulator, powered from the socket. The module is high-impedance to the
  host from power-on until a verified image is loaded.
* the **upper board** carries the AG32, its regulator and clock, USB-C, a
  two-way switch selecting one of four images, and a SOICbite edge for a
  clip-on UART programmer.

The boards support the 512x8 and 256x8 tri-state parts. The fabric also
covers 4096x4 and open-collector outputs, which would need a different lower
board. `docs/09-two-board-stack.md` and `docs/05-hardware.md` have the
design, `docs/07-bom.md` the parts and ordering.

## Updating it

A web page (`web/`) loads ROM images and updates the firmware and fabric
over USB, using WebUSB in a Chromium browser. It can also recover a blank
or bricked board through the AG32's mask-ROM loader, using a 3.3 V
USB-serial adapter clipped onto the SOICbite. See `docs/10-updates.md` and
`docs/12-web.md`.

## Repository

```
hdl/      fabric: both ROM modes, enable decode, AHB slave
sim/      testbenches
fw/       firmware in Rust: application, DFU stage, recovery loader, USB stack
web/      the update page
tools/    part database, board map, and the generators for constraints,
          schematics, layout and panel
hw/       footprints, routing and revision; KiCad files are built here by make
docs/     design documents, numbered
DESIGN_NOTES.md  the reasoning and numbers behind each electrical choice
```

Everything downstream of `tools/partdb.yaml`, `tools/boardmap.yaml` and the
committed routing is generated; the schematics, boards and fabrication files
are built, not checked in. Building, flashing and releasing:
`docs/06-build-and-flash.md`. In short:

```
make test                          # simulation suite + firmware tests
make fw-build                      # firmware for the AG32
make web                           # the update page
make jlc KICAD="scripts/kicad.sh cli"   # boards, DRC, JLCPCB files
make help                          # everything else
```

The firmware, KiCad and Freerouting run in containers (podman by default).

## License

Copyright 2026 mxshift. Software (`fw/`, `hdl/`, `sim/`, `tools/`,
`scripts/`, `web/`) is [Apache 2.0](LICENSES/Apache-2.0.txt); hardware
(`hw/`, `docs/netlist/`) is [CERN-OHL-P 2.0](LICENSES/CERN-OHL-P-2.0.txt);
documentation is [CC BY 4.0](LICENSES/CC-BY-4.0.txt). Eighteen footprints
derived from KiCad's library keep its [CC BY-SA 4.0](LICENSES/CC-BY-SA-4.0.txt).
`LICENSE` and `REUSE.toml` have the details.
