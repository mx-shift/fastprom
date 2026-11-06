# Updates over USB

How an end user changes the ROM images, the firmware and the fabric: a web
page in a Chromium browser, a USB-C cable, no tools to install. The model is
One ROM's (`onerom.org/web`), adapted to a chip with no USB boot loader in
mask ROM. This page is the design the code follows; `08-usb-updates.md` is
the hardware it runs on.

**Status:** implemented and tested without hardware: the flash map, the
DFU stage's boot check and DFU mode, the application header and fabric
manifest, the update core (FastPROM's own update protocol, the range
policy, commit), the USB device that serves it from both binaries
(`fw/fastprom-usb` over `fastprom-hal`'s `usb.rs`; `11-usb-driver.md`), the
DFU stage's own USB-only fabric, loaded on entering DFU mode, the web page
(`web/`, `fw/fastprom-host`, `fw/fastprom-web`; `12-web.md`), and the
page's UART recovery of a blank or bricked board ("Recovery", below: Web
Serial, the mask ROM's loader, FastPROM's own recovery loader in SRAM).
Nothing here has run on hardware yet; "Verification" at the end lists what
is checked and how.

## The pieces

| Piece | Where | Speaks |
|---|---|---|
| DFU stage | flash `0x8000_0000`, 32 KiB | the update protocol over USB (vendor interface) |
| application | flash `0x8000_8000`, 48 KiB | the update protocol over USB, plus a CDC log port |
| web page | a static site | WebUSB (devices), Web Serial (UART recovery) |
| mask-ROM loader | the AG32 | ST AN3155 over UART0 (J3), BOOT0 high |
| recovery loader | SRAM `0x2001_0000`, uploaded by the page | AN3155 over UART0, the same framing |

One protocol, FastPROM's own ("The update protocol", below), from both the
application and the DFU stage, so the page has one code path whichever is
running. One ROM's firmware speaks Raspberry Pi's PICOBOOT; FastPROM does
not, because FastPROM owns both ends (the firmware, the DFU stage and the
page's client are all this repository's Rust) and PICOBOOT's commands are
the RP2040's and RP2350's
(`../DESIGN_NOTES.md`, "The update protocol: our own, not PICOBOOT").

## Why a DFU stage

One ROM's microcontrollers have a USB boot loader in mask ROM; when the
firmware is to be replaced, it reboots into that. The AG32 has none
(`08-usb-updates.md`), so the DFU stage is that boot loader, kept in its own
flash region that USB never writes. A failed application, fabric or image
update therefore leaves USB working (the boot check below stays in DFU
mode unless the application and the fabric are both committed); only a
damaged DFU stage or option bytes need the UART ("Recovery").

AGM's SDK has a USB DFU of this shape (`platforms/AgRV/examples/dfu`,
`env:usb_app`). Ours follows its pattern - a boot loader at the reset
address, the application above it, `mscratch` as the "stay in DFU" flag
across a soft reset - but not its power-up path: AGM's loads a fabric
configuration of its own and soft-resets on *every* power-on before reaching
the application, a second fabric configuration in front of the ROM coming up,
which is the one open risk this design has (`06-build-and-flash.md`, "The
configuration window").

## Flash map

256 KiB, 4 KiB sectors, 256-byte pages.

| region | address | size | written by |
|---|---|---|---|
| DFU stage | `0x8000_0000` | 32 KiB | UART recovery only |
| application | `0x8000_8000` | 48 KiB | DFU stage |
| fabric bitstream | `0x8001_4000` | 100 KiB | DFU stage |
| manifest | `0x8002_D000` | 4 KiB | DFU stage, last |
| image table | `0x8002_E000` | 72 KiB | application, or DFU stage |
| option bytes | `0x8100_0000` | 128 | UART recovery only (the fabric address at `+0x30`) |

The fabric address is the option byte's, set once at provisioning; the image
table needs at most 16,640 bytes (four 4096-word images), so it gives up the
room. `fw/fastprom-core/src/memmap.rs` is the map's one source: the linker
scripts, the firmware, the update policy and `tools/fw_layout.py` all take
it from there.

The application's region starts with a 256-byte header: magic, header
version, image length, CRC-32 of the image, firmware version, the board
revision it was built for (`hw/REVISION`) and the fabric it expects
(`regmap::FABRIC_ID`, the revision of the fabric's register interface);
its entry point follows the header, at `0x8000_8100`. The manifest sector's
first page records the fabric's length, CRC-32, part, board revision and
fabric ID. Both are little-endian fields with a version and a CRC-32 over
the page; `fw/fastprom-core/src/header.rs` has the layouts. The firmware
links its header with a zero length and CRC, and `tools/fw_stamp.py app`
fills them in after linking; `tools/fw_stamp.py manifest` writes a
bitstream's manifest.

Each page begins with a **commit word**, and the magic in it is the last
thing written, by `COMMIT`, after the payload's CRC-32 has been checked
against the page, so an interrupted update leaves an invalid header or
manifest, never a valid one over a partial payload. The rules that make
that hold, whenever power is lost, are under "The update protocol".

## Power-up

1. The fabric configures from flash, in hardware, before the CPU runs. The bus buffers stay disabled: their enable is pulled up (R21) and
   the fabric holds it high until the firmware sets READY.
2. The DFU stage runs from the reset vector. After a power-on reset it
   clears `mscratch`. If `mscratch` holds the DFU magic it enters DFU mode;
   otherwise, if the application header's magic, version and length are
   sane, and so are the fabric manifest's, it jumps to the application;
   else DFU mode. The manifest is there because the application's USB is
   the fabric's to enable: a fabric update cut part way leaves the manifest
   uncommitted (it is erased first, committed last), and the board then
   stays in DFU mode on the stage's own fabric, with USB, rather than
   booting an application whose fabric will not configure. That is a few
   register reads: no clock changes, no CRC (verified when written), no
   USB. It is 38 instructions (42 after a power-on reset) of assembly at
   the reset vector itself, ahead of riscv-rt's
   startup (`fastprom-dfu`'s `boot.rs`), so not even the stack or RAM is
   set up on this path; the application's own startup is the first thing
   after it.
   `mscratch` is read and cleared in one instruction, so a DFU request is
   used once.
3. The application reads the switch, checks the image's CRC, loads it into block RAM, sets READY - and only then starts
   USB. USB never adds to the time the host waits.

## DFU mode

Entered when the application asks for it (`REBOOT` to DFU mode from the page:
the application sets `mscratch` and soft-resets), or when the application
header or the fabric manifest is not committed. The DFU stage loads a small USB-only fabric configuration
it carries (as AGM's does): the USB controller is enabled by the fabric
(`USB0 device` in the `.ve`), so a missing or corrupt fabric would otherwise
take USB with it. That configuration drives no socket pin and leaves the
buffer enable to R21, so the socket stays high-impedance throughout. On exit
the DFU stage soft-resets with `mscratch` cleared, and the fabric configures
from flash again.

As built (`fastprom-dfu`'s `main.rs`), in order:

1. Clear READY if the fabric answers with FastPROM's ID, so the ROM fabric
   has the buffers off before anything changes.
2. Load the DFU stage's own fabric (`fabric.rs`, `fastprom_hal::fcb`): the
   clocks and USB0, no pin at all. AGM's sequence from its `dfu.c`: HSI to
   its fastest, the core on HSI with the PLL off (the configuration
   includes the PLL), every APB clock off but the FCB's, the bitstream
   expanded word by word into the FCB's AUTO register (`fastprom_core::fcb`,
   AGM's LZW format), then the FCB's status, and the clock gates and HSI
   setting put back. The bitstream is 3,226 bytes, compressed, from
   `tools/ag32/build_dfu_fabric.sh`, committed in `fw/fastprom-dfu/fabric/`
   with its manifest and the pin evidence: Supra's routed design has pad
   cells on the three clock inputs only, so every package pin but those and
   USB's D+/D- is undriven while DFU mode runs (`06-build-and-flash.md`,
   "The DFU stage's fabric"). If the FCB refuses it, DFU mode carries on
   regardless; there is nothing better to fall back to.
3. Start the PLL (USB's 60 MHz is PLL output 1), from the DFU fabric's
   PLL configuration.
4. Copy the flash routines to SRAM and serve the update core in
   `Mode::Dfu` until a `REBOOT`.

### Leaving DFU mode

`REBOOT` soft-resets (`fastprom_hal::reset::reboot`): `mscratch` cleared
for the application, the DFU request for DFU mode, then RST_CNTL's software reset
with its FCB-reset-disable bit cleared. A reset resets the FCB unless that
bit (system.h `SYS_RST_FCB_DIS`) is set, and the FCB then configures the
fabric from the option bytes' address, as at power-on, so the application
starts on the ROM's fabric again. That is the SDK's evidence, not a
datasheet's: the bit's existence and name, AGM's `usb_app` DFU, which
loads its own USB fabric at every power-on and soft-resets to "restore" the
configuration before every application start, and AGM's serial booter,
whose reset after a logic download is the same soft reset
(`../DESIGN_NOTES.md`, "The DFU stage's own fabric", has the detail).

The risk, until a board shows it: if a soft reset did not reconfigure, the
application would come up on the DFU fabric. It would fail safe, because
its first step reads the fabric ID: no 'FPRM', E50 on the UART, READY never
set, socket high-impedance; and the remedy would be an explicit
reconfiguration from flash before the reset (`FCB_AutoConfig` of the
bitstream at the option bytes' address, as AGM's `example_fcb.c` does).
A power cycle always reconfigures.

## USB device

* VID `0x1209` (pid.codes). PID `0x0001`, pid.codes' test PID, until one is
  allocated for FastPROM; the application and the DFU stage share it and
  share a serial number (the chip's unique ID), so a browser that was given
  the device once finds it again after the reboot into DFU without asking.
* `bcdUSB 0x0210` with a BOS descriptor carrying the WebUSB and Microsoft OS
  2.0 platform capabilities, so Windows binds WinUSB itself and no driver is
  installed anywhere.
* The update interface: a vendor interface (class `0xFF`, subclass and
  protocol 0) with two bulk endpoints of 64 bytes, EP1 OUT and IN;
  interface 0 in both firmwares, the DFU stage's only one.
  `bMaxPacketSize0` is 64. Every endpoint number is in `fastprom_usb::eps`.
* The application adds a CDC-ACM pair, interfaces 1 and 2, carrying the
  boot report and log (`06-build-and-flash.md`, "The boot report").
* Full speed: the AG32's controller is a ChipIdea core (AGM's TinyUSB port
  uses the `ci_hs` driver) at `0x4100_1000`, clocked at 60 MHz from PLL
  output 1, which `gen_vlog` assigns to USB whenever `USB0` is enabled.
* The driver, the descriptors and both functions are `fw/fastprom-usb`;
  `11-usb-driver.md` has the controller's bring-up with its sources, the
  endpoint plan (the update interface on endpoint 1, CDC data on 2, its notification on
  3 IN: the controller's four endpoints suffice), the descriptor bytes, and
  the checks that need a board. The two firmwares differ in `bcdDevice`
  (application 0x0100, DFU stage 0x0001) and product string; VID, PID and
  serial number are shared.

## The update protocol

FastPROM's own, served by both firmwares on the update interface
(`fw/fastprom-core/src/update/proto.rs` is the wire format and its one
implementation: the device's server and the page's client both build and
read frames with it). Request and response, one at a time.

### Frames

Every request and every response is one frame: a 16-byte header, then
`len` bytes of payload. Little-endian.

| offset | size | field |
|---|---|---|
| 0 | 4 | magic, the bytes `FPUP` |
| 4 | 1 | protocol version: 1 |
| 5 | 1 | command; a response echoes its request's |
| 6 | 1 | status (below); 0 in a request |
| 7 | 1 | reserved, 0 |
| 8 | 2 | sequence number; a response echoes its request's |
| 10 | 2 | payload length |
| 12 | 4 | CRC-32 (`zlib.crc32`, `fastprom_core::crc32`) of bytes 0 to 11 as sent, then the payload |

The header's layout, the magic and the CRC rule never change between
versions; a new version changes only the commands and their payloads.

**On USB.** A frame is exactly one bulk transfer: requests on EP1 OUT,
responses on EP1 IN, in 64-byte packets, the last one short (shorter than
64 bytes); a frame whose length is a multiple of 64 is followed by a
zero-length packet. The payload length must agree with the bytes the
transfer carried. The host asks each IN transfer for 4,160 bytes, more
than the longest response (16 + 4,096), so each `transferIn` returns
exactly one response. The host sends a request only once it has the
previous response; until then the device leaves a request's packets in the
controller, which NAKs them.

**Exactly one response per request transfer.** The device answers every
transfer, even one it cannot read as a request: too short for a header or
without the magic (`BadFrame`, command and sequence number 0), a length or
CRC that disagrees with what arrived (`BadFrame`, echoing the header),
another protocol version (`Version`, with the device's own version in the
response header), longer than any request whose length says so
(`BadRequest`). None of these touches the flash.

### Recovery

USB checks and retries every packet itself, so a packet goes missing only
when a transfer is abandoned part way (a host gives up, a page is closed).
The rule for every failure the host can see - a response that does not
decode, one with another sequence number or command, or `BadFrame` - is
the same: send **RESET** (a vendor control request to the update
interface: `bmRequestType` 0x41, `bRequest` 0x01, `wValue` 0, `wIndex` the
interface, no data), which empties the device's half-received request, its
unsent response, the packet armed on EP1 IN and any packet held on EP1 OUT;
then send the request again under a new sequence number. The client
(`fastprom-host`'s `client.rs`) makes three attempts. Every command is
idempotent - programming a page with the data it already holds changes no
bit, an erase is an erase, `COMMIT` of a committed page answers `Ok`, READ
and INFO change nothing - so a request that ran before its response was
lost runs again harmlessly. A host also sends RESET at the start of every
session.

A transfer whose only short packet is lost (a one-packet request, or a
one-packet response) leaves the other side waiting: the host's transfer
times out, which ends the operation (a WebUSB transfer cannot be withdrawn,
so the page does not retry on a timeout), and the next session's RESET
clears the device. Running the operation again is safe for the same reason
a resend is.

A host refuses a response in another protocol version outright: the page
and the firmware come from one release.

### Commands

| command | id | request payload | response payload | from |
|---|---|---|---|---|
| `INFO` | 1 | none | 144 bytes (below) | both |
| `READ` | 2 | `u32` address, `u32` length, 1 to 4,096 | the bytes | both |
| `WRITE` | 3 | `u32` address, one 256-byte page | none | both, by region |
| `ERASE` | 4 | `u32` address of one 4 KiB sector | none | both, by region |
| `COMMIT` | 5 | `u8`: 0 application, 1 fabric | none | DFU stage |
| `LOAD` | 6 | none | none | application |
| `REBOOT` | 7 | `u8`: 0 the application, 1 DFU mode | none | both |

A payload of the wrong length, an unknown command or an argument out of
range is `BadRequest`. Address ranges are checked against the flash map:
the application may write only the image table; the DFU stage may write the
application, fabric, manifest and table, never itself or the option bytes.
In detail (`fastprom-core`'s `update`, which has the table):

* `READ`: anywhere in the 256 KiB main array, in either mode. The device
  streams it from flash a packet at a time.
* `ERASE` and `WRITE`: inside a region the mode may write, on a sector
  (`ERASE`) or page (`WRITE`) boundary; the host pads a file with `0xFF`
  to whole pages. Each page is read back after it is programmed and each
  sector blank-checked after it is erased.
* In DFU mode, a write of the header page (or the manifest page) programs
  everything but the commit word, which stays erased: the file the page
  writes is the one the UART loader writes, magic included, and the device
  holds the magic back for `COMMIT`.
* An erase of the header's (or the manifest's) sector first programs its
  commit word to zero, so an erase cut short cannot leave the old magic
  over a half-erased payload. Any other erase or write of the application
  (or the fabric) is refused with `PreconditionNotMet` while the header
  (or manifest) is still committed: a committed page never describes a
  payload changed since. A firmware update is therefore: erase the
  application region, write the file, read back, `COMMIT 0`; a fabric
  update: erase the manifest, erase the fabric, write it, write the
  manifest page, `COMMIT 1`.
* `COMMIT`: check the payload's CRC against the header or manifest just
  written, and only then make it valid: program the commit word. Refused
  for a page built for another board revision than the DFU stage's
  (`NotPermitted`); `CrcMismatch` when the CRC does not match;
  `InvalidData` when the page is not well formed.
* `LOAD`: reload block RAM from the image table now, for the bank the
  switch read at boot (the host must be off: the socket's data changes
  under it); READY is cleared first and set only by a load that passed
  every check (`InvalidData` for a table or image that does not).
* `REBOOT`: 0 boots through the DFU stage's check into the application, 1
  into DFU mode. The reboot happens once the host has taken the response's
  last packet, so the host knows it was accepted.
* `INFO`: firmware version, board revision, part, fabric ID, mode
  (application or DFU), image table summary, the switch's bank; the
  header's and manifest's fields and whether each is committed; whether
  the table's header parses and, apart, whether every image under it
  matches its CRC, with a bit per image (format 2; layout in
  `update/info.rs`).

### Statuses

| status | code | meaning |
|---|---|---|
| `Ok` | 0 | done |
| `BadFrame` | 1 | the transfer was not a whole request: reset and send again |
| `Version` | 2 | the device does not speak the request's version |
| `BadRequest` | 3 | unknown command, wrong payload length, argument out of range |
| `InvalidAddress` | 4 | outside the flash |
| `BadAlignment` | 5 | not on a page (`WRITE`) or sector (`ERASE`) boundary |
| `NotPermitted` | 6 | refused by the mode's policy: a region this mode may not write, the DFU stage, the option bytes, `LOAD` in DFU mode, `COMMIT` in the application, a page for another board revision |
| `PreconditionNotMet` | 7 | a payload its committed page guards; `LOAD` without a switch reading or with the socket not released |
| `CrcMismatch` | 8 | `COMMIT`: the payload does not match its page's CRC-32 |
| `InvalidData` | 9 | `COMMIT`: no well-formed page; `LOAD`: no valid image |
| `FlashFailed` | 10 | an erase or program failed, or read back otherwise |

There is no "busy": the device runs each request to completion before it
answers, and takes no request while an answer is unsent.

`fastprom-core`'s `tests/update.rs` cuts power at every flash operation of a
firmware and a fabric update (an erase cut leaves its sector half erased, a
program cut its word half programmed) and checks that what the DFU stage
would boot is either the old image whole or the new one committed, that
every cut of a fabric update leaves it in DFU mode, then that the update
run again from there succeeds.

## The web page

A static site (`web/`, built by `make web` into `build/web/`; `12-web.md`
has the design), for Chromium browsers: WebUSB is Chromium's, and any other
browser is told so. As built:

* **Connect**: the board's `INFO` and its image table read back - running
  firmware and mode, board revision, part, fabric, the switch's position at
  power-up, and what each switch position holds (name, CRC, whether the
  words still match it). The update interface is found by walking the
  configuration for the vendor interface with two bulk endpoints
  (interface 0 in both firmwares, but the number is not assumed).
* **Images**: per switch position, keep the current image, a ROM file, or
  nothing. Files are checked against the part's organisation (from the
  board's part, `tools/partdb.yaml`; "The part" below) as `mkimagetable.py` checks
  them; the table is built byte for byte as `mkimagetable.py` builds it,
  written, read back and compared. `LOAD` is a separate step behind a
  "the host is powered off" confirmation.
* **Firmware and fabric**: from the update bundle published beside the page
  (`tools/mkbundle.py`: the application, a fabric per part that was built,
  never the DFU stage). Refused outright if the bundle's board revision is
  not the board's. Every file is fetched and checked against the manifest
  and its own header before the board is touched; then `REBOOT` into DFU
  mode, the fabric (manifest erased first, written last), the firmware,
  each read back and `COMMIT`ted, and `REBOOT` back. A board found in DFU
  mode (after a cut) is updated without the first reboot.
* **Recovery**: Web Serial to a 3.3 V USB-UART adapter on J3, as below:
  wiring instructions with J3's pads, the part to provision (or none),
  the board revision confirmed by the user, the port, progress, and what
  to do next.
* **The part** is the fabric manifest's; when there is none (an
  interrupted fabric update erases it first) it is the image table's, which
  records the part's `tools/partdb.yaml` key, if it is a known part; only
  when both are missing does the page ask.
* **Simulated board** (`?sim`): `fastprom-core`'s update core over a NOR
  flash model, compiled to WebAssembly, behind a fake WebUSB, with a panel
  to set the switch, cut the power at a chosen flash operation and power
  on again.

The image-table format, the header and manifest, `INFO`, the flash map and
the update procedures are Rust (`fastprom-core`, and `fastprom-host` for
the host side), compiled to WebAssembly for the page and natively for the
firmware and its tests, so the page and the firmware cannot disagree about
them. The files the page offers are published with the site: GitHub's
release downloads carry no CORS headers, so a page cannot fetch them. The
firmware release attaches the built site as a zip; where it is hosted is
not decided.

## Recovery

For a board USB cannot reach: new and blank, or with a damaged DFU stage
or option bytes.
From the page, in desktop Chrome or Edge (Web Serial), with a 3.3 V
USB-UART adapter:

1. The user wires J3: pad 1 (UART0 TX) to the adapter's RX, pad 2 (RX)
   to its TX, pad 3 to GND; bridges BOOT0 (pad 4) to 3.3 V (pad 5); powers
   the board and resets it (pad 6, NRST, to GND, or a power cycle). The
   mask ROM's serial loader now waits on UART0.
2. The page opens the port at 115200 8E1 and runs
   `fastprom_host::recovery::recover`:
   - sync (`0x7F`, the mask ROM measures the baud rate on it), `GET`
     (version at most `0x10` is the mask ROM; FastPROM's recovery loader
     answers `0x46`; anything else is refused), `GID` (must be
     `0x4020_0001`, the AG32VF303; anything else is refused before a byte
     is written);
   - with the mask ROM: a one-byte read of SRAM, which a read-protected
     chip refuses, and then `READOUT UNPROTECT` (the chip erases its flash
     and resets; sync again); the recovery loader (`fw/fastprom-recovery`,
     4,372 bytes) written into SRAM at `0x2001_0000` 256 bytes at a time,
     each read back, with AGM's memory-mode byte after each address; `GO`
     to its entry, `0x2001_0020`; sync with the loader;
   - from here FastPROM's own code (`fastprom_core::recovery::server` on
     the board, over the DFU stage's flash routines): the application
     header and manifest pages read first, and a board whose flash holds
     either for another board revision refused; the option bytes erased,
     written (read protection off, the fabric at `0x8001_4000`) and read
     back, unless already right; all 64 sectors erased (the image table
     stays empty); the DFU stage, then the fabric and its manifest, then
     the application with its header page last, every page read back as
     it is written; the header and manifest committed, as AGM's loader
     writes them; then a reset.
   Every command that fails part way (a NACK, a timeout) is retried after
   a resync, four times in all; a retried page write is harmless.
3. The user removes the bridge and power-cycles: the fabric configures
   from flash at power-on, the DFU stage boots the application, which is
   on USB with an empty image table (the page's step 2 writes the images).
   A bundle with no fabric for the part (a release's) provisions the DFU
   stage alone: with no application header and no manifest the boot check
   stays in DFU mode, on the stage's own fabric, for a USB update from a
   bundle that has one.

What recovery asks of the mask ROM is only what AGM's own tools ask of it
before they hand over to a loader of their own in SRAM: sync, `GET`,
`GID`, `READ` and `WRITE MEMORY` to SRAM, `GO`, and `READOUT UNPROTECT`.
The flash itself, the option bytes included, is written by FastPROM's
code. `../DESIGN_NOTES.md`, "UART recovery", has the evidence for each
inference about the mask ROM and the checks that need a board. The bundle carries the recovery loader, the DFU stage and the
option bytes (`12-web.md`, "The update bundle"); USB never writes the
last two.

## Verification

Without hardware:

* the update protocol and range checks, driven packet by packet through
  `update::Server` against a memory model of the flash, in `cargo test`
  (`fastprom-core/tests/update.rs`), with power cut at every flash
  operation of an update; the protocol itself there too: every damaged
  byte of a frame caught, transfers that are not requests (no magic, too
  short, a length or CRC that disagrees, another version, oversized)
  answered and refused without effect, a packet lost from a request, a
  READ of 65 packets, responses closed by a zero-length packet, sequence
  numbers through their wrap, RESET part way through either direction,
  REBOOT after the last packet, 3,000 random transfers; the header and
  manifest against
  `tools/fw_stamp.py`'s output (`tests/header.rs`);
* the USB stack above the registers - the bus logic, the control pipe and
  standard requests, the classes, every descriptor byte - against a model
  of the controller (`fw/fastprom-usb/tests`, `11-usb-driver.md`);
* the host side (`fw/fastprom-host/tests`): the client's refusals, its
  recovery (reset and send again) from a packet lost or damaged at every
  packet of an update in either direction and from a stale answer, a
  device of another protocol version refused, the
  image table byte for byte against `mkimagetable.py`'s, and every
  procedure against the simulated board, power cuts included;
* the page against the simulated board - the same Rust device side,
  compiled to WebAssembly, behind a fake WebUSB that enumerates with
  `fastprom-usb`'s descriptors, and a fake Web Serial onto its UART - in
  headless Chromium (`make web-test`, `12-web.md`);
* UART recovery (`fastprom-host/tests/recovery.rs`): a blank board
  provisioned through the mask-ROM model and the recovery loader's server
  on the flash model, then booted onto USB; the DFU stage alone without a
  fabric; bytes corrupted and lost on the wire, retried; a wrong device ID
  and another board revision refused before anything is written; a second
  run finding the loader already running; power cut part way and run
  again; the loader's server byte by byte, every refusal
  (`fastprom-core/tests/recovery.rs`); the loader under its 5,296-byte
  window, entered after its header, no panic paths (`tools/fw_layout.py`);
* the flash map: `tools/fw_layout.py` checks both ELFs, the stamped
  header, the manifest and the batch image against `memmap.rs`, and that
  neither binary links a panic path;
* the fabric with USB enabled: placed and routed by Supra, timing in
  `03-timing-budget.md`;
* the DFU stage's fabric: Supra's routed design checked for pad cells on
  the clock inputs only, and its compressed bitstream checked to expand to
  Supra's plain one, by a Python decoder (`tools/ag32/check_dfu_fabric.py`)
  and by the firmware's own (`fastprom-core`'s `tests/fcb.rs`, which also
  round-trips an independent encoder and feeds it malformed streams); the
  committed file against its manifest and inputs at every build
  (`fastprom-dfu`'s build.rs).

On the first boards: enumeration, the reboot into DFU and back, an image
update, a firmware update, a fabric update, an interrupted update of each,
and UART recovery of a blank board (the mask-ROM items in
`../DESIGN_NOTES.md`, "UART recovery"). For the DFU fabric in particular: that
the FCB takes it (DFU mode enumerates with the fabric region erased), how
long DFU entry takes, that the socket's data pins and `buf_noe` stay
undriven throughout (a scope on PIN_14 and a data pin across entry), and
that `REBOOT` back to the application brings the ROM's fabric back
without a power cycle.
