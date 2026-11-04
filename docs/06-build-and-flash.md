# Building and flashing

Two things get programmed into the AG32: the fabric bitstream and the RISC-V
firmware. Both live in the MCU's flash, and the fabric image is downloaded
into the fabric's configuration SRAM automatically at power-on.

Which build you run depends on the mode.

## LOGIC mode

Contents are synthesised into the fabric, so the image is part of the
bitstream and changing it means re-running synthesis.

```
# 1. pick the part and generate its constraints and HDL parameters
python3 tools/gen_pins.py --part 512x8 --board fastprom-upper

# 2. turn the ROM dump into logic
python3 tools/gen_rom_logic.py --image mypalette.bin --aw 9 --dw 8 \
        --out gen/rom.v

# 3. check what it costs before committing to it
python3 tools/synth_matrix.py

# 4. build the bitstream (see toolchain options below)
```

Worth doing step 3 on a real image. The fabric cost of LOGIC mode depends on
how compressible the contents are, not just their size - a palette collapses
to a handful of LUT4s while incompressible data of the same size can need
hundreds. `docs/synth-results.md` has the measured spread.

The fabric needs no clock in this mode. `core_ready` is the AHB slave's
READY bit in both modes, so a LOGIC bitstream also keeps its outputs off
until the firmware writes READY. The firmware is written for BRAM mode and
does not special-case LOGIC builds. Nor can a LOGIC build hold several
images: the firmware reads the image-select switch, but the AHB slave has
no bank register to pass it into the fabric. BRAM mode needs none, since
the firmware loads the selected image.

## BRAM mode

BRAM mode is the one shipped. One bitstream per organisation, and the images live in
flash where the firmware can load them. Changing an image is a firmware
rebuild, not a resynthesis.

```
tools/ag32/run_flow.sh /opt/agm/AgRV_pio 512x8 bram build/ag32/512x8-bram
make fw-batch PART=512x8 FW_IMAGES="a.bin b.bin"
make fw-flash PART=512x8 PORT=/dev/ttyUSB0       # BOOT0 held high
```

The fabric holds one image (no bank bits: `--banks` stays 0). Image select
is the firmware's job: it reads the switch on the UART pins at power-up,
checks the selected image's CRC, loads that one image through the AHB slave
and only then lets the outputs drive ("Firmware", below). The
generated `.ve` clocks the block RAM's read port at 200 MHz from its own PLL
output (`PLLCLK2`), which is what makes the fastest grades; see below.

AGM publishes no M9K clock-to-data-valid time. Supra's timing model has one
(3.24 ns), and `03-timing-budget.md` uses it, but it is AGM's model rather
than a characterisation, so BRAM-mode access time still wants measuring on
silicon before it is relied on near a grade boundary.

## Toolchain options for the bitstream

**AGM Supra** - the vendor tool, in AGM's AgRV_pio SDK. Its Linux x86-64
binaries (`af`, `Supra`) run the whole native flow - AGM's own yosys build for
synthesis, then map, place, route, static timing and bitgen - with no Quartus
and no licence file. It is reported to route electrically dead edges without
complaint, so a wrong bitstream can fail silently. See "Building with Supra"
below for the recipe this project uses.

**AGaMEMnon** - the open flow: Verilog through Yosys and a custom nextpnr
backend to a bitstream, no vendor binaries. Version 0.4.0 is a qualified
subset rather than vendor parity, and its silicon evidence is LQFP-48 only.
For the QFN-32 it supports only builds that assign no pads, so it cannot
build this design, and Supra is the build path.

Either way the pin constraints come from `tools/gen_pins.py`, which emits both
formats: `.pcf` for the open flow and `.ve` for Supra.

## Building with Supra (the native flow)

`tools/ag32/run_flow.sh` places and routes the fabric for the stack's AG32
(device `AGRV2KQ32`) and writes the bitstream and a timing summary;
`tools/ag32/run_matrix.sh` (`make pnr AGRV_PIO=...`) runs every
configuration the timing budget quotes and collects the results into
`tools/ag32/pnr-results.json`, which `tools/timing_budget.py` reads.

```
tools/ag32/run_flow.sh <AgRV_pio dir> <part> <logic|bram> <work dir> [kind] [bank bits]
#   e.g. tools/ag32/run_flow.sh /opt/agm/AgRV_pio 512x8 logic build/ag32/512x8-logic
#   -> <work>/logic/top.bin, <work>/logic/top_batch.bin, <work>/timing.json
make pnr AGRV_PIO=/opt/agm/AgRV_pio       # all configurations, ~15 min, then make budget
```

What it needs:

* **The AgRV_pio SDK** (AGM's PlatformIO package, 1.1.0 here; Supra
  `2024.11.b0`). The path is the script's first argument. It is not in this
  repo and not on PATH; anything with `packages/tool-agrv_logic/bin/af_cmd`
  and `platforms/AgRV/etc/gen_vlog` in it will do.
* **A podman image for the AGM binaries**, which need glibc 2.35 and some
  X/GL libraries even headless: `podman build -t ct-supra tools/ag32`
  (the `Containerfile` there; the name comes from cosmic-toad, and
  `SUPRA_IMAGE` overrides it).
* python3 with PyYAML on the host, for `tools/gen_pins.py`.

The steps, in order:

1. `tools/gen_pins.py` turns `tools/boardmap.yaml` into the `.ve` (pins,
   `SYSCLK 100`, `HSECLK 12` for the stack's oscillator on PIN_2) and the
   part's HDL parameters.
2. AgRV_pio's `gen_vlog -d AGRV2KQ32` turns the `.ve` into the top level
   (`logic/top.v`, the CPU, the PLL and the pads) and the pin file
   (`logic/top.vex`), and writes `logic/fastprom_ip_tmpl.v`, the port list
   the IP must have. `tools/ag32/fastprom_ip.v` is that IP: the AHB slave on
   `sys_clock` and `fastprom_top`.
3. `pre_logic.tcl` (through `af_cmd`) writes the project: `af_map.tcl`,
   `af_run.tcl`, `top.sdc`. The flow then appends `tools/ag32/fastprom.sdc`
   (and `fastprom_bram.sdc` in BRAM mode).
4. `tools/ag32/patch_af_map.py` patches the generated `af_map.tcl`.
5. `af_map` (AGM's yosys 0.47) synthesises to `top.vqm`.
6. `af_cmd ... -X 'set MODE NATIVE' -F af_run.tcl` runs Supra: pack, place,
   route, timing (`logic/alta_db/setup.rpt.gz` and friends) and bitgen.
7. `tools/ag32/timing_report.py` checks every port landed on the pin the
   `.ve` asked for, then extracts the socket paths into `timing.json`.

### The patches, and why

AGM's generated `af_map.tcl` does not build real designs as shipped.
`patch_af_map.py` (from cosmic-toad) fixes four things:

1. **No carry chains.** The template runs yosys' generic `techmap.v` before
   AGM's `arith_map.v`, so every adder is already ripple LUTs by the time the
   carry mapping runs. The patch runs the arithmetic map first. (FastPROM's
   only adder is the AHB slave's load pointer.)
2. **Unroutable carry taps.** AGM's arithmetic map takes every requested
   carry-out from the middle of the chain; a signed compare then drives two
   carry-ins from one cout, which Supra cannot place. The patch uses a copy
   (`logic/arith_map_fp.v`) where only the last carry leaves the chain.
3. **`$print` / `$scopeinfo` cells** from `$display` in AGM's RAM models,
   which Supra's netlist reader cannot parse: deleted.
4. **Leftover `$not` / `$mux` / `$logic_not` cells** that the last LUT map
   misses and that are written as Verilog expressions Supra rejects: lowered
   and LUT-mapped.

### Pitfalls

* **USB0 takes PLL output 1, silently.** With `USB0 device` in the `.ve`,
  `gen_vlog` forces PLL output 1 to USB's 60 MHz and only warns that any
  `PLLCLK1` was ignored; a read clock on output 1 runs at 60 MHz and adds
  12 ns to the socket's tAA. The read clock is on output 2 (output 0 is
  SYSCLK, 3 BUSCLK, 2 only the unused MAC's). The `.ve` needs `USB0 device`
  for the firmware's USB controller to have a clock at all
  (`10-updates.md`).
* **Never let yosys infer the block RAM.** It goes through AGM's
  `alta_ram9k` wrapper, whose mapping functions this yosys cannot evaluate:
  the read data becomes `x` and everything downstream is silently optimised
  away (the build "passes" with no ROM in it). `hdl/fastprom_rom_bram.v`
  instantiates `alta_bram9k` directly under `ALTA_SYN`, which `af_map`
  defines; simulation takes the behavioural branch.
* **Don't use Quartus.** `MODE QUARTUS` relies on a design partition around
  the CPU, Quartus Web Edition has no incremental compilation, drops the
  partition and folds the CPU bus to constants, and the IP vanishes.
* **One-bit buses.** `gen_vlog` folds a one-bit bus to a scalar port (a part
  with one enable gets `ce_pin`, not `ce_pin[0]`) and then cannot match an
  indexed name in the `.ve`; Supra warns "IO ce_pin is not assigned" and
  places it anywhere. `gen_pins.py` therefore names one-bit buses without the
  index, and `timing_report.py` refuses a run where any port is not on its
  constrained pin. Check that line whenever the constraints change.
* **The data pads are plain OUTPUTs.** `gen_vlog` makes `:OUTPUT` pins
  always enabled, so the IP drives `fastprom_top`'s `pad_data`, and the
  drive buffer's `/OE` (`buf_noe`) is the only bus enable - which is how the
  stack is wired.
* **sys_clock is the AHB clock.** `mem_ahb` runs on `sys_clock`
  (`SYSCLK` in the `.ve`, `pll_inst|clkout0` in the SDC), not `bus_clock`.
  BRAM mode's read port does not use it: CPU-block AHB paths do not close
  at 200 MHz (cosmic-toad's finding), so `tools/gen_pins.py` gives the read
  port its own PLL output (`PLLCLK2 200` / `PLL_CLKOUT2 rd_clk:OUTPUT` in the
  `.ve`, `--rdclk` to change it; output 1 is USB's whenever `USB0` is
  enabled, see "Pitfalls") and `run_flow.sh` clocks the M9K from it and
  retargets `fastprom_bram.sdc`. `RDCLK=<MHz>` overrides the frequency for a
  run, and `RDCLK=0` reads on sys_clock (then `SYSCLK=150` raises
  sys_clock). 200 MHz is the ceiling: Supra flags any
  clock above its restricted Fmax of 200.08 MHz.
* **Seeds.** `SEED=n` passes Supra's placement seed (AGM's default is 666).
  Use two or three when a margin is within the ~0.7 ns of placement noise.
* **Read the right report.** Supra prints the timing summary twice (it
  re-times after routing); the second is final. `setup.rpt.gz` holds the
  worst path to each endpoint, which is what `timing_report.py` reads.
* **Unconstrained is not met.** A combinational pad-to-pad path only appears
  in the report if `fastprom.sdc` names both ends; `timing_report.py` fails
  if the address or enable paths are missing.
* **Congestion.** A four-image incompressible LOGIC build is 89 % of the
  LUTs, and Supra's router had not converged after an hour (single-image
  builds route in about a minute). `run_matrix.sh` leaves it out unless
  `X4=1`. Watch `logic/supra_console.log`: `iter = ... violation#` should
  fall to zero.
* **Never edit `run_flow.sh` while a run is in progress.** `sh` reads the
  script as it executes, so an edit mid-run corrupts the running instances
  (it shows as a syntax error at the last line, after Supra has finished).
* **USB0 needs its pad pulls set too.** Every AGM board file
  (`platforms/AgRV/boards/agrv2k_*/board.asf`) clears the USB pads'
  `CFG_PULLUP_ENB` and `CFG_PULLDN_ENB` whenever the `.ve` enables USB0;
  `run_flow.sh` passes no board file, which would leave both set
  (bitstream byte 41: 0xA1 where AGM's setting gives 0x01), so both flows
  read `tools/ag32/usb0.asf`, which does what AGM's boards do;
  it changes no socket timing (the 512x8 BRAM build's pad-to-pad and
  read paths are unchanged to the picosecond). AGM documents the bits no
  further.
* **Supra is not byte-reproducible.** The same inputs can route the clock
  network either of two equivalent ways, a few bits apart in the
  bitstream (seen on the DFU fabric: 3,226 or 3,240 bytes compressed); even
  a comment line added to an `.asf` can flip it. Compare builds by their
  reports and timing, not by their bytes.
* **`RESERVE_ALL_UNUSED_PINS` does nothing here.** Supra accepts the
  Quartus assignment, but "AS OUTPUT DRIVING GROUND" gives the same
  bitstream as no assignment at all on the AGRV2K: an unused pin has no pad
  cell and is undriven whatever it says. (Which is what the DFU fabric
  relies on; its evidence is the routed netlist, below.)

### The DFU stage's fabric

The DFU stage loads its own fabric configuration on entering DFU mode
(`10-updates.md`, "DFU mode"): the clocks and USB0, and no pin at all, so
that the socket's data pins and the buffer enable are left alone. It is
built apart from the ROM's, with AGM's flow for a design without custom
logic, the one AGM's DFU example uses:

```
tools/ag32/build_dfu_fabric.sh /opt/agm/AgRV_pio            # build and check against the committed one
tools/ag32/build_dfu_fabric.sh /opt/agm/AgRV_pio --install  # and replace fw/fastprom-dfu/fabric/
make dfu-fabric AGRV_PIO=/opt/agm/AgRV_pio                  # the first
```

1. `tools/gen_pins.py --dfu-ve` writes `dfu_usb.ve`: `SYSCLK 100`,
   `BUSCLK 50`, `HSECLK 12` (the same lines as the ROM's `.ve`, from the
   same code: the PLL is part of the configuration, and the firmware
   assumes 100 MHz from it), `USB0 device`.
2. `gen_vlog` without `-s` writes `top.vx`, AGM's netlist of the CPU block
   with the fabric side tied off (the CPU's AHB port into the fabric reads
   ready, data zero), and `top.vex`.
3. `af_cmd` runs `gen_logic.tcl` as AgRV_pio's builder does, with
   `LOGIC_COMPRESS true` (`board_logic.compress = true`): bitgen writes the
   LZW-compressed bitstream that the FCB decoder reads. Our additions:
   `usb0.asf` (above), `dfu_usb.sdc` (the clocks; without it
   `gen_logic.tcl` leaves PIN_HSI to an auto constraint, a warning),
   `dfu_usb_pre.tcl` (the coverage limit off: this design's only
   connections are clock network, half of which end at no register, so
   coverage is 50 % whatever the constraints say) and `dfu_usb_post.tcl`
   (the same routing written again uncompressed, as the reference for the
   compressed one). No yosys, no IP, no patches. About 10 s.
4. `tools/ag32/check_dfu_fabric.py` reads Supra's own output and fails the
   build unless: the routed netlist's ports are exactly PIN_HSE, PIN_HSI
   and PIN_OSC, all inputs; its only pad cells are theirs, with output
   enable and data tied low; `io.asf` places only those; the packer reports
   3 pins; Supra reports 0 warnings; every unconstrained connection is
   clock network; and the compressed bitstream starts with the AGRV2K's
   IDCODE and expands to exactly the plain one. It writes the evidence and
   a manifest.

The result is committed, because CI has no SDK: `fw/fastprom-dfu/fabric/`
holds `dfu_usb.bin` (3,226 bytes; 99,944 plain), `dfu_usb.ve`,
`dfu_usb.manifest` (length and CRC-32 of both bitstreams, CRC-32 of the
inputs) and `dfu_usb.evidence.txt` (step 4's output). `fastprom-dfu`'s
build.rs refuses to build if the bitstream is not the one the manifest
records or any input in `tools/ag32/` changed since, and says to rebuild;
`fastprom-core`'s `tests/fcb.rs` expands it in `make fw-test`. Without
`--install` the script checks a fresh build's inputs against the committed
manifest, not its bytes (see "Supra is not byte-reproducible").

## Handling

* **Ship and store the module in conductive foam**, as PROMs always were.
  The socket pins reach the bus buffers directly (U2 and U3, and U4 through
  its 33 R terminators): an SN74LVT245B is rated about 2 kV HBM, and there is
  no room in the channel between the pin strips for 18 TVS diodes.
* **With USB and the host connected at once, their grounds are joined**
  through the board-to-board grounds and socket pin 10. Either power the
  host down while the module is on USB, or make sure the PC and the host
  share a ground, or ground currents between them flow through the module.
* **Seat the module by pressing on the lower board**, not on the upper one:
  the board-to-board connector is built for retention, not insertion force.

## Programming

The stack is programmed **over UART0 only**, through the AG32's mask-ROM
loader. It has no JTAG or SWD connection: JTMS and JTCK (PIN_24, PIN_25) are
left on their internal pulls. AGM's serial loader writes both the firmware
and the fabric bitstream, so the UART is enough for everything the board
needs; the one path that always works is also the only path
(`../DESIGN_NOTES.md`, "Programming over UART only, no JTAG").

`J3` is a [SOICbite](https://github.com/SimonMerrett/SOICbite): eight bare
pads at the **upper board's pin 1 end edge**, 1-4 on top and 8-5 directly
under them, 1.27 mm pitch, with a row of five 0.7 mm holes behind them. A
fixture that grips the edge holds itself on:
- a SOIC-8 test clip (e.g. Pomona's), closed further than it leaves the
  factory: it opens about 2.2 mm released, and the board is 0.8 mm, so its
  jaws need about 1.4 mm more travel plus some preload;
- or a printed clip with pogo pins top and bottom, located by the holes.

The pinout is SOICbite's ESP8266 one, so cables made for that work here:

| Pin | Signal | |
|---|---|---|
| 1 | +3V3 | supply for a bare board, or a reference for the adapter |
| 2 | BOOT0 (PIN_30) | hold at 3.3 V to enter the loader; R10 pulls it low |
| 3 | - | unused |
| 4 | GND | |
| 5 | UART0 RX (PIN_21) | to the adapter's TX |
| 6 | UART0 TX (PIN_20) | to the adapter's RX |
| 7 | - | unused |
| 8 | NRST (PIN_4) | reset; R9 pulls it up, C11 to ground |

Wire pin 2 to 3.3 V in the fixture (or to an adapter line driven high), so
clipping on selects the loader. Powering a bare board through pin 1 takes a
supply good for 150 mA or more: the AG32's current is not published, and a
USB-UART adapter's own 3.3 V output is often 50 mA or less (FT232R) - use a
separate regulator in the fixture, or power the board over USB-C and treat
pin 1 as the reference. Back-feeding +3V3 is safe with the board otherwise
unpowered (`../DESIGN_NOTES.md`, "Programming over UART only, no JTAG").

With the module stacked, the upper board is held to the lower one only
by the DF40 (J5/J6), 13-15 mm from J3's edge: rocking a clip on or off
levers the connector apart. Support the upper board at J6's end while
fitting or removing the clip, or program the upper board before
stacking.

To program: hold BOOT0 at 3.3 V, power the board (USB-C, the host socket, or
3.3 V on pin 1), pulse NRST low (or power-cycle), then run a loader over a
3.3 V USB-UART adapter: the update page's recovery (below), or AGM's
(`make fw-flash`, "Flashing"). BOOT1 (PIN_15) is held low by R11; it shares
its pin with a socket data line, which is only driven once the fabric is
running. Release BOOT0 and power-cycle to boot the new image.

**From the update page** (`10-updates.md`, "Recovery"; `12-web.md`): in
desktop Chrome or Edge, section 4 of the page walks through the wiring
above, then writes the board over Web Serial at 115200 8E1: it uploads
FastPROM's recovery loader (`fw/fastprom-recovery`) into SRAM through the
mask ROM's loader, and that writes the option bytes, the DFU stage, the
fabric and its manifest, and the application, every page read back; the
image table is left empty, for the page's USB image step. The files come
from the update bundle published with the page, so nothing is installed,
and nothing of AGM's is used.

**The UART pins double as image select.** `SW1` reaches TX and RX through
10k (R24, R25), and 100k pull-downs (R17, R18) hold them low when the
switch is open. Firmware reads the two pins as a bank number at power-up,
before it enables the UART: switch 1 (pins 1-4) sets bit 0, on RX; switch 2
(pins 2-3) sets bit 1, on TX. On is 1, so the image number is the switch's
own printed numbers read as bits: switch 1 alone is image 1, switch 2 alone
image 2, both image 3. The values are chosen so an attached
adapter still works with the switch in either position - the adapter's drive
wins over the 10k. The firmware is in "Firmware" below. An attached
adapter's TX idles high and holds RX at 1, so with one connected the
switch reads as switch 1 on.

There is **no factory USB bootloader**. The firmware has its own DFU stage
and update core over its own USB device stack (`10-updates.md`,
`11-usb-driver.md`): once a board has been provisioned over the UART, the
page updates it over USB-C. The UART stays the way back for a board USB
cannot reach.

## Firmware

Pure Rust in `fw/`, a Cargo workspace; no vendor C is linked. It is
structured like cosmic-toad's firmware, and built in a container from
`tools/rust/Containerfile` (cosmic-toad's `ct-rust` toolchain plus
`llvm-tools`, image `fastprom-rust`).

| crate | | |
|---|---|---|
| `fastprom-recovery` | the recovery loader, at `0x20010000` in SRAM | uploaded by the page's UART recovery through the mask ROM, never flashed: AN3155 framing over UART0, the flash and option bytes written through `fastprom-hal` (`fastprom-core`'s `recovery` server) |
| `fastprom-core` | `no_std` library, host-tested | CRC-32 (`zlib.crc32`); the binary image table, parsed into checked types; the image-select decode; the load sequence against a `Fabric` trait, with `Released` as the typestate that proves the outputs are down; failure codes; the flash map (`src/memmap.rs`); the application header and fabric manifest (`src/header.rs`); the USB update core, the device side of FastPROM's update protocol, and the protocol's wire format, shared with the page (`src/update/`); the decoder for compressed fabric bitstreams (`src/fcb.rs`) |
| `fastprom-hal` | `no_std` library, AG32 only | the registers (each with its SDK source), volatile access, clocks, delays, the flash driver, fabric configuration through the FCB, the soft reset into the DFU stage, the USB boundary; `build_support.rs`, which both binaries' `build.rs` include to write `memory.x` from `memmap.rs` |
| `fastprom-ag32` | the application, at `0x80008000` | its header; riscv-rt startup; the switch pins; UART0; the fabric's AHB window; the boot report; the update core's board traits; `main` |
| `fastprom-dfu` | the DFU stage, at `0x80000000` | the boot check at the reset vector (`src/boot.rs`); DFU mode: its own USB-only fabric (`src/fabric.rs`, the committed `fabric/`), then the update core |

```
make fw-image        # once: podman build -t fastprom-rust tools/rust (FW_ENGINE=docker for docker)
make fw-test         # host tests; fixtures from zlib and tools/mkimagetable.py
make fw-lint         # cargo fmt --check, clippy -D warnings on host and riscv32, cargo-cycles
make fw-build        # build/fw/fastprom-{dfu,ag32,recovery}.{elf,bin}, the headers stamped, and the layout report
make fw-table        # build/fw/images.bin from FW_IMAGES (a test pattern if unset)
make fw-batch        # build/fw/fastprom-$(PART)-batch.bin: everything, for the loader
```

`make fw-test` writes the fixtures (`fw/tools/gen_fixtures.py`: 22 CRC
vectors from Python's zlib; image
tables written by `tools/mkimagetable.py` itself; headers and manifests
written by `tools/fw_stamp.py` itself), runs `cargo test`, and records the
result in `build/fw/host-tests.stamp`. The update core's tests
(`fastprom-core/tests/update.rs`) drive the update server end to end with
the protocol's own frames, packet by packet, against a model of the NOR
flash, and cut power at every flash
operation of a firmware and a fabric update. The tests also write
traces of the load sequence, which `sim/tb_fw_load.v` replays on the AHB
slave. `scripts/run_sim.sh` runs both. Inside the `hdlc/sim` container,
which has neither cargo nor podman, it accepts the recorded result only if
nothing under `fw/` has changed since. So run `make fw-test` before the
containerised suite.

### What it does at power-up

0. The DFU stage's boot check, at the reset vector (`fastprom-dfu`'s
   `boot.rs`): after a power-on reset, clear the reset flags and `mscratch`;
   if `mscratch` holds the DFU request, stay in DFU mode; otherwise, if the
   application header's and the fabric manifest's commit words, versions and
   lengths are sane, jump to
   the application at `0x80008100`; else DFU mode. 38 instructions, 42
   after a power-on reset; no CRC, no clock change, no USB
   (`../DESIGN_NOTES.md`, "The DFU stage's boot check").

Then the application:

1. Clear READY, read back clear, if the fabric answers with ID 'FPRM'. A
   soft reset should reconfigure the fabric (`10-updates.md`, "Leaving DFU
   mode"), which clears READY, but until that is measured the firmware
   does not rely on it.
2. Read the switch on PIN_21 (RX, bit 0) and PIN_20 (TX, bit 1) as GPIO
   inputs, 16 samples over about 1 ms, before UART0 has the pins. All 16
   must agree.
3. Start the PLL: HSE bypass from the 12 MHz oscillator, sys_clock 100 MHz.
   BRAM mode's read port needs PLL output 2.
4. Hand the pins to UART0 and print the boot banner.
5. Parse the image table at 0x8002E000, check the selected image's CRC, and
   probe the fabric's block RAM depth: it must equal the table's. Write each
   word to DATA and check the pointer advanced by exactly one. Set READY and
   read it back.
6. Serve the update core over USB (`10-updates.md`), so the page can
   rewrite the table, reload the block RAM, or reboot into DFU mode. If
   anything failed, READY was never set, so the module stays
   high-impedance to the host; the failure line repeats every 2 s, and USB
   is served all the same.

### Memory map

| region | address | size | contents |
|---|---|---|---|
| flash | `0x80000000` | 32 KiB | DFU stage: the boot check (160 bytes), then `.text`, `.rodata` with its USB-only fabric (3,226 bytes), `.ramfunc` (20,332 bytes, 62 %) |
| flash | `0x80008000` | 48 KiB | application: its 256-byte header, then `.text` from `0x80008100`, `.rodata` (34,308 bytes with the header, 70 %) |
| flash | `0x80014000` | 100 KiB | fabric bitstream, 99,944 bytes, uncompressed |
| flash | `0x8002D000` | 4 KiB | fabric manifest, one 256-byte page (`fw_stamp.py manifest`) |
| flash | `0x8002E000` | 72 KiB | image table (`mkimagetable.py --out *.bin`; at most 16,640 bytes, four 4096-word images) |
| option bytes | `0x81000030` | 8 | fabric configuration address and its complement |
| SRAM | `0x20000000` | 128 KiB | `.bss` (8 KiB: the USB controller's queue heads and buffers), the flash routines (260 bytes, `.ramfunc`, copied there before the first flash write), the rest stack |
| SRAM | `0x20010000` | 5,296 | during UART recovery only: the recovery loader as uploaded, its 32-byte header then its code (4,372 bytes, 83 %); its stack in the first 32 KiB of SRAM |
| fabric AHB | `0x60000000` | 16 | CTRL, ADDR, DATA, ID (repeats every 16 bytes) |

The source is `fw/fastprom-core/src/memmap.rs`, which says why each
boundary is where it is:
- the SDK's `misc/devices/AgRV2K_mem.ld` and `boards/agrv2k_303.json` for
  flash, SRAM and sizes;
- `flash.h` for the 4 KiB sector and 256-byte page;
- `builder/common.py` for the 100 KiB fabric reserve;
- FastPROM's own choices for the rest (`10-updates.md`, "Flash map").

The registers come from `framework-agrv_sdk/src/` (`system.h`, `flash.h`,
`gpio.h`, `uart.h`, `AltaRiscv.h`, `boards/agrv2k_303/board.h` for
`MMIO_BASE`), and `fw/fastprom-hal/src/regs.rs` names each source. Every
region is 4 KiB-sector aligned. `tools/fw_layout.py` checks both flash ELFs
against this map on every `fw-build` (and that they agree on it), checks
the application header, checks the recovery loader against its SRAM window
and its header, and checks that no panic path is linked into any of the
three.

### The combined image

AGM's serial loader takes one "batch" file. `make fw-batch` builds it with
the SDK's own `platforms/AgRV/etc/gen_batch`, as AgRV_pio's "Create Batch"
does, after writing the bitstream's manifest
(`tools/fw_stamp.py manifest --fabric top.bin --part 512x8 --out build/fw/manifest.bin`):

```
gen_batch --flash-size 262144 -d 0x40200001 \
    --logic-config build/ag32/512x8-bram/logic/top.bin --logic-address 0x80014000 \
    -i build/fw/fastprom-dfu.bin build/fw/fastprom-ag32.bin build/fw/manifest.bin \
       build/fw/images.bin \
    -a 0x80000000 0x80008000 0x8002D000 0x8002E000 \
    -o build/fw/fastprom-512x8-batch.bin
```

That gives option bytes (128 bytes at 0x81000000, pointing the fabric
configuration at 0x80014000), the fabric at 0x80014000, the DFU stage at
0x80000000, the application at 0x80008000, the manifest at 0x8002D000 and
the table at 0x8002E000. The whole chip is erased first, and the device
resets after the last section. The bitstream is the one in Supra's
`top_batch.bin`; that file's address, and its option bytes, say
0x80007000, inside the DFU stage's region, so it is not flashed as it is. The application and the manifest go in
with their commit words set, so a board provisioned over the UART boots
straight to the application. `fw_layout.py --batch` decodes every header
and checks each section is where the ELFs say, and the manifest against
the bitstream.

### Flashing

From the update page, as in "Programming" above; or with AGM's tools, as
here. Over UART0 only (J3, "Programming" above). Hold BOOT0 high and reset,
then:

```
make fw-flash PART=512x8 PORT=/dev/ttyUSB0
#  = agrv32flash_cmd -m 8e1 -G 1 -T 0x20010000 -L -B 460800 -X 0x10 -b 460800 -R -Z \
#        -N build/fw/fastprom-512x8-batch.bin /dev/ttyUSB0
make fw-flash-table FW_IMAGES="a.bin b.bin" PORT=/dev/ttyUSB0   # images only, at 0x8002E000
```

The flags are AgRV_pio's `batch_upload` and serial `upload` targets
(`platforms/AgRV/builder/main.py`). `agrv32flash_cmd` is in
`packages/tool-agrv_flashloader/bin/`. Release BOOT0 and power-cycle: the
fabric configures at power-on only. **Neither has been run on hardware.**

### The boot report

UART0 at 115200 8N1 on J3 pad 1, after the switch has been read:

```
FastPROM firmware 0.1.0, AG32VF303 BRAM mode
SW1: A (RX, PIN_21) open, B (TX, PIN_20) closed: image 1
clock: 100 MHz from the PLL (12 MHz oscillator, HSE bypass)
fabric: FastPROM logic, 512-word block RAM, outputs released
OK: serving image 1 'b.bin' (2 of 2, 512x8, part 512x8), crc32 0x1321d8fa at 0x8002e000, READY set
```

or, every 2 s until power-off:

```
FAIL E40: loading the selected image: checking image 1: image 1 has CRC 0x..., but the table says 0x...
  socket outputs stay released (high impedance); idle
```

| code | failure |
|---|---|
| 10-11 | image-select switch not read, or not steady across the 16 samples |
| 20-29 | image table: too short, no magic (erased flash says so), version, header CRC, depth, width, count, text field, reserved bytes, truncated |
| 30 | the switch selects an image the table does not have |
| 40-41 | image CRC mismatch; a word wider than the table's width |
| 50-57 | fabric: not configured or not FastPROM's ID, lost mid-load, READY would not clear, pointer probe, block RAM depth differs from the table, pointer reset, pointer did not advance, READY would not set |
| 60-61 | HSE did not start; PLL did not lock (the core stays on HSI) |

### First-board bring-up

None of this has run on silicon. In order:

1. **UART banner.** It is also the first sign that the DFU stage's boot
   check jumped to the application. If nothing appears, the core may not be
   reaching `main` (a DFU stage that stayed in DFU mode prints nothing:
   check `layout.txt` shows the header and the manifest committed and the sections in the
   batch), or the HSI-derived baud may be off (HSI has no stated accuracy).
   The banner goes out after the PLL starts, so a clock failure prints at
   HSI's rate, and E60/E61 may look garbled.
2. **Switch.** Check the `SW1:` line for all four positions, with the
   adapter's TX disconnected (it holds RX high).
3. **Fabric.** `fabric: ... 512-word block RAM` proves the AHB window, the
   ID and the pointer probe. E50 with ID `0x00000000` means unconfigured
   fabric: check the option bytes and the bitstream address.
4. **Load.** `OK ... READY set`, then read the socket with a PROM reader
   against the image.
5. **Configuration window.** Time from power-on to `buf_noe` low, against
   the host's reset (see below).

## Two things to know about the fabric image

**It loads from flash at power-on, and (by the SDK's evidence) at every
reset.** RST_CNTL's `SYS_RST_FCB_DIS` (system.h, "FCB reset disable") is
clear out of power-on, so a reset resets the FCB with the chip, and the FCB
configures the fabric from the option bytes' address again; AGM's DFU
example soft-resets precisely "to restore" the configuration
(`examples/dfu/src/dfu.c`), and `10-updates.md` relies on that for leaving
DFU mode. Unmeasured, and the SDK has no datasheet text on it, so the
firmware clears READY first after any reset. Software can also load
a configuration itself through the FCB, which is how the DFU stage loads its
own ("The DFU stage's fabric", above). Over JTAG the fabric can be loaded
volatile into its configuration SRAM - that is what Supra's `_SRAM.prg` and
`agamemnon sram` are for - but the stack has no JTAG, so a change to the
ROM's fabric is a UART flash or a USB fabric update.

**0x80014000 to 0x8002D000 belongs to the fabric.** `make fw-batch` writes
the bitstream there uncompressed (99,944 bytes, to 0x8002C668), with the
option byte at `0x81000030` pointing at it. Supra's own `top_batch.bin`
puts it at 0x80007000, inside the DFU stage's region: flash `make
fw-batch`'s image, not Supra's. A compressed bitstream would put a
decompressor first and the image 0x1100 bytes further on, with pointers at
`0x81000038` and `0x81000040`; FastPROM does not use one. The linker
refuses a DFU stage or an application that outgrows its region (the map is
above).

## The configuration window

There is a window after power-up during which the fabric is unconfigured, the
AG32's I/O is floating, and the socket is dead. AGM publishes no figure for
how long it lasts.

The design fails safe: outputs stay released rather than driving garbage. But
a host that starts fetching from this socket before configuration completes
reads nothing useful, and a module in a PROM socket has no access to the
host's reset to hold it off.

**This is the measurement to take first on real hardware**, before any timing
work: how long after power-on the outputs become valid, compared against the
host board's own reset delay. It is the one open risk that could rule out
particular hosts.

## Releases

The boards and the firmware are released separately, each from a tag, by
GitHub Actions; each workflow runs a script that also runs locally.

| Release | Tag | Version from | Script | Publishes |
|---|---|---|---|---|
| Hardware | `hw-rev-<REV>`, e.g. `hw-rev-A` | `hw/REVISION` | `scripts/release_hw.sh` | schematic and board PDFs for each stack board; `fastprom-hw-rev-<REV>.zip` with the KiCad projects (built from the sources), BOMs, and the JLCPCB gerbers, BOM and CPL for each board and the panel |
| Firmware | `fw-v<VERSION>`, e.g. `fw-v0.1.0` | `fw/Cargo.toml` (`[workspace.package] version`) | `scripts/release_fw.sh` | the application `fastprom-fw-<VERSION>.{elf,bin}` (stamped header included, for `0x80008000`), the DFU stage `-dfu.{elf,bin}` (for `0x80000000`), the recovery loader `-recovery.{elf,bin}`, the layout report, and the update page `fastprom-web-<VERSION>.zip` |

`hw/REVISION` is the board revision's one source: `tools/gen_wired.py`
writes it into the schematics' title blocks and `tools/gen_layout.py` into
the boards' title blocks and silkscreen (`FastPROM rev A LOWER` / `UPPER`).
To start a new revision, change it and commit. The firmware's version is
what its boot report prints.

The repository holds the hardware's sources only: the generators and part
tables in `tools/`, the footprint library and library tables in `hw/`, each
board's routing (`hw/<board>.route.json`) and `hw/REVISION`. The
schematics, KiCad projects, boards, panel, netlists and BOMs are built from
them into `hw/` (`make kicad netlist layout panel`, each rebuilt when a
source changes) and are not tracked; to open the design in KiCad, build it
first. A release publishes them as built.

Each script refuses to release when:

* the tag does not match the version in the tree;
* (hardware) building the schematics, boards and panel from scratch changes
  a tracked file (the routing, the symbol library), which would mean a
  committed source is not what its generators make;
* (hardware) ERC, DRC, the drafting grade, the netlist comparison or the
  stack check is not clean;
* (firmware) the host tests or the lints fail.

For a dry run, start either workflow by hand from the Actions tab (Run
workflow): it builds and checks everything at the version in the tree and
publishes nothing. On the runner both use docker for their containers
(`KICAD_ENGINE=docker`, `FW_ENGINE=docker`): under its rootless podman the
containers' user cannot write the checkout.

Push the tag on the commit to release, e.g.
`git tag hw-rev-A && git push origin hw-rev-A`.

**The fabric is added to a firmware release by hand.** Supra comes with
AGM's AgRV_pio SDK, which CI does not have and which is not
redistributable, so the release as CI publishes it has no bitstream: its
page updates firmware and images, and UART recovery brings a blank board
only as far as its update mode. Once `release-fw.yml` has published
`fw-v<VERSION>`, on a machine with the SDK:

```
git checkout fw-v0.1.0
scripts/release_fabric.sh fw-v0.1.0            # build and check
scripts/release_fabric.sh fw-v0.1.0 --upload   # ... and add to the release
```

It downloads the release's firmware, DFU stage and recovery loader and
checks them against the release's `SHA256SUMS` (the bundle carries what was
released, not a local rebuild), builds the BRAM fabric for each part the
upper board supports, rebuilds the page with the fabrics in its bundle,
runs the browser tests, and with `--upload` adds
`fastprom-fabric-<VERSION>.zip` and replaces the page's zip and
`SHA256SUMS` (`gh release upload --clobber`). It refuses unless HEAD is the
tag and the tree is clean. The release has no combined flash image:
`make fw-batch` needs AGM's `gen_batch` too.

Every push to `main` and every pull request runs `.github/workflows/ci.yml`:
`fw-lint`, `fw-test`, `fw-build`, `web`, `web-test` and the simulation suite
(`make test`, in the `hdlc/sim` image where there is no `iverilog`), and the
boards' checks (`scripts/release_hw.sh`, which publishes nothing).
