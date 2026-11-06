# The update page

The end user's side of `10-updates.md`: a static web page that talks to a
FastPROM board over WebUSB and changes its ROM images, firmware and fabric,
and recovers a blank or bricked board over Web Serial and the chip's
mask-ROM loader. This is how it is built; `10-updates.md` ("The web page",
"Recovery") is what it does.

## Pieces

| piece | where | what |
|---|---|---|
| host crate | `fw/fastprom-host` | the update protocol's client, the update procedures, ROM files, the image-table builder, update bundles, the AN3155 client and UART recovery, the simulated board (std Rust) |
| bindings | `fw/fastprom-web` | `fastprom-host` and the simulator for JavaScript (wasm-bindgen), built for `wasm32-unknown-unknown` |
| page | `web/index.html`, `web/style.css`, `web/js/app.js` | the user interface; no framework, nothing fetched from anywhere but the page's own directory |
| WebUSB | `web/js/usb.js` | finds the update interface, gives the WebAssembly a `Link` of three transfers |
| Web Serial | `web/js/serial.js` | opens the adapter at 115200 8E1, gives the WebAssembly a `SerialLink` of four calls |
| fake WebUSB, fake Web Serial | `web/js/sim-usb.js`, `web/js/sim-serial.js` | `navigator.usb`'s and `navigator.serial`'s shapes over the simulated board |
| recovery loader | `fw/fastprom-recovery` | the AN3155 loader the page uploads into SRAM through the mask ROM (riscv32, `make fw-build`) |
| site | `tools/mkweb.py` | copies `web/` and writes `parts.json` from `tools/partdb.yaml` |
| bundle | `tools/mkbundle.py` | `bundle/`: `manifest.json`, the application, each built part's fabric and manifest page, the recovery set |
| tests | `fw/fastprom-host/tests`, `web/tests` | `cargo test`; Playwright in headless Chromium |

Everything with a format or a rule in it is Rust: the protocol's frames
(`fastprom-core`'s `update/proto.rs`, the device's own module), the client
and its recovery (`client.rs`), `INFO`
(`info.rs`, over `fastprom-core`'s layout), the procedures (`update.rs`),
ROM files and the table (`rom.rs`, `table.rs`, building through and
checking with `fastprom-core`'s parser), the bundle checks and what a bundle
offers a board (`bundle.rs`), the AN3155 client (`an3155.rs`) and the
recovery procedure (`recovery.rs`, over `fastprom-core`'s `recovery`:
the option bytes, the loader's header, the loader's command server). The
JavaScript is the page: devices, ports, files, buttons, text.

### The transport

`fastprom_host::Transport` is three async calls: bulk OUT, bulk IN, and
the update interface's RESET (a vendor control request to the interface,
no data). The page implements it over WebUSB (`usb.js`'s `UpdateLink`,
which copies each WebAssembly buffer before it awaits, since wasm memory
can move); the tests over the simulated board directly
(`sim::SimTransport`, whose futures are ready at once, run by
`sim::block_on`).

A request is one frame in one bulk OUT transfer, its response one bulk IN
transfer of 4,160 bytes, which always ends at the response's short packet
(`10-updates.md`, "The update protocol"). A refusal is a status in the
response, reported as the device's refusal. A response the client cannot
match to its request - it does not decode, it answers another sequence
number or command, or it says the request arrived damaged - is followed by
RESET and the request again, three attempts in all; every command is
idempotent, so a resend is harmless. A device answering in another
protocol version is refused at once with a message to use the page
released with its firmware. A transfer that times out (30 s, `usb.js`)
ends the operation: WebUSB cannot withdraw it, so it is not retried. `tests/protocol.rs` covers each case.

The update interface is the first vendor interface (`FF/00/00`) with a
bulk OUT and a bulk IN endpoint, found in the configuration, never assumed:
interface 0 in both firmwares. `transport::find_update_interface` is the
rule, tested on `fastprom-usb`'s descriptor bytes; `usb.js`'s
`findUpdateInterface` applies it to WebUSB's parsed configuration, and the
page's tests run that on both firmwares' descriptors as the WebAssembly
gives them (`firmwareConfiguration()`). The VID, PID, both `bcdDevice`
values, the interface number and RESET's `bRequest` the page uses come
from `fastprom-usb` and `fastprom-core` too (`usbIds()`).

### The serial link

`fastprom_host::SerialPort` is a byte stream with timeouts: write; read up
to `n` bytes, returning when they have arrived or after a timeout with
what has; drop what is buffered; sleep. `serial.js`'s `SerialLink` keeps a
reader running into a buffer, so a read with a timeout is a wait on the
buffer. The port is opened at the mask ROM's framing (115200 8E1) and
never changed: the recovery loader sets UART0 to the same from its own
clock. In tests, `sim::SimSerial` is the simulated board's UART, a read's
timeout being time the simulator lets pass.

After `REBOOT` the page waits (`getDevices`, every 200 ms, 20 s at most)
for the device with the same serial number and the other firmware's
`bcdDevice` (`0x0001` DFU, `0x0100` application): WebUSB keeps the
permission for the serial number, so the user is not asked again.

### The simulated board

`fastprom_host::sim::SimDevice` is the firmware's update core
(`fastprom_core::update::Server`) on a model of the AG32's 256 KiB NOR
flash - the same model `fastprom-core/tests/update.rs` cuts power in -
behind a model of the update interface's endpoints: packets of 64 bytes
handed to the server and taken from it as `fastprom-usb`'s update class
does, a request sent while an answer is unread NAKed (in the simulator, a
timeout), and RESET. It can lose or damage a packet in either direction
(`sim::UsbFault`), for the client's recovery tests. It boots as the board
does: power-on clears `mscratch`, the DFU stage's check (`boot_application`: header and manifest) or a
DFU request picks the mode, and the application loads the switch's image
(block RAM is its CRC). `REBOOT` takes it off the bus; it enumerates again
with `fastprom-usb`'s device and configuration descriptors for the mode it
booted into and the same serial number (`fastprom_usb::Serial` of a fixed
ID). A power cut at a chosen flash operation leaves that operation half
done (a sector half erased, a word half programmed) and the device gone
until power-on.

`sim-usb.js` puts `navigator.usb`'s shape on it - `getDevices`,
`requestDevice`, `connect`/`disconnect` events, and a `USBDevice` whose
configuration is parsed from the descriptor bytes - so the page runs the
same code against it as against a board. The board is "factory
provisioned": an application at `?version=` (0.1.0), a fabric and manifest
for `?part=` (512x8), a test-pattern image per switch position, board
revision `?rev=` (A).

With BOOT0 held high (the simulator panel's checkbox, read at power-on), the
chip comes up in a model of the mask ROM's serial loader instead, with no
USB, and `sim-serial.js` puts `navigator.serial`'s shape on its UART: a
port whose stream polls the simulator and lets its time pass when quiet.
The model has only what recovery asks of the mask ROM (`10-updates.md`,
"Recovery"): sync, `GET` (version 0x10), `GID`, reads and writes of SRAM
behind AGM's memory-mode byte, `GO`, and `READOUT UNPROTECT`, with read
protection on while the option bytes' RDP is erased. `GO` to a recovery
loader whose header checks starts `fastprom-core`'s loader server on the
simulated flash and option bytes; `GO` anywhere else and the chip is quiet
until power-on. A blank board (`?sim&blank`) runs nothing at all until it
is recovered (its reset vector is erased); the application has USB only
when the option bytes point the fabric at a bitstream. The panel can
corrupt a byte on its way to the board or lose one on its way back, to
exercise the retries; `?device=` sets the ID `GID` answers.

What it does not model: timing, USB enumeration itself, the CDC port, the
fabric (`fastprom-usb`'s own tests cover the device stack against a model
of the controller; the simulator drives the update server directly), the baud
rate, and a fabric the FCB cannot configure (which the boot check keeps it
from: without a committed manifest it stays in DFU mode).

## The update bundle

`bundle/manifest.json` (format 1):

```json
{
  "format": 1,
  "board_rev": "A",
  "firmware": { "file": "fastprom-fw-0.1.0.bin", "version": "0.1.0", "board_rev": "A",
                "fabric_id": 1, "size": 34308, "crc32": "0x44670cac",
                "image_len": 34052, "image_crc32": "0xeb63167b" },
  "fabrics": [ { "part": "512x8", "file": "fabric-512x8.bin",
                 "manifest": "fabric-512x8.manifest.bin", "board_rev": "A",
                 "fabric_id": 1, "size": 99944, "crc32": "0x..." } ],
  "recovery": {
    "loader": { "file": "fastprom-recovery-0.1.0.bin", "size": 4372, "crc32": "0x...",
                "version": "0.1.0", "board_rev": "A" },
    "dfu": { "file": "fastprom-dfu-0.1.0.bin", "size": 20332, "crc32": "0x...",
             "version": "0.1.0", "board_rev": "A" },
    "option_bytes": { "file": "option-bytes.bin", "size": 128, "crc32": "0x2d97ce22" } },
  "notes": [ "The DFU stage is not updated over USB: only by UART recovery (J3)." ]
}
```

`recovery` is the set only UART recovery writes: the recovery loader
(`make fw-build`'s stamped `fastprom-recovery.bin`), the DFU stage
(`fastprom-dfu.bin`) and the option bytes (`fw_stamp.option_bytes()`,
gen_batch's layout). The page checks the loader against its own header,
and the option bytes byte for byte against `fastprom-core`'s
`recovery::OPTION_BYTES`, so the Python that writes them and the Rust that
reads them cannot drift apart; both files' board revisions must be the
bundle's. A bundle built without the firmware has no recovery set, and the
page then cannot recover a board.

A board being recovered may have nothing to report its revision with, so
the page asks the user to confirm the board's printed revision is the
bundle's before it starts, and the recovery procedure refuses a board whose
flash still holds a header or manifest for another revision.

The application is `make fw-build`'s stamped `.bin` (header committed, as
the UART loader writes it; the device holds the commit word back until
`COMMIT`). A fabric is `build/ag32/<part>-bram/logic/top.bin` with its
manifest page from `fw_stamp.manifest`, for each part whose bitstream was
built; the rest are left out with a note. `make web` without a firmware
build writes a manifest with no firmware, so the page always finds one.

The page fetches every file and checks it - size and CRC against the
manifest, the header or manifest page against the file and against the
manifest's version, board revision and fabric ID - before the board is
rebooted.

## Build, run, test

```
make fw-image                  # once: the Rust image, with wasm32 and wasm-bindgen-cli
make fw-build                  # the application, for the bundle (optional)
make web                       # -> build/web
python3 -m http.server -d build/web 8000
#   http://localhost:8000/        a real board (Chromium)
#   http://localhost:8000/?sim    the simulated board, any browser
make web-test                  # Playwright in mcr.microsoft.com/playwright:v1.64.0-noble
```

WebUSB needs a secure context: `localhost` or HTTPS.

URL parameters: `sim` (the simulated board), `rev`, `part`, `version` (its
board revision, part and application version), `blank` (a board never
provisioned, for recovery), `device` (the device ID its boot loader
reports), `bundle` (the bundle's directory, default `bundle/`).

Web Serial is desktop Chromium's (not Android's); it too needs a secure
context.

`WEB_FABRICS=<dir>` points `make web` at another `build/ag32` for the
fabrics.

Sizes, `make web` at this commit: `fastprom_web_bg.wasm` 354,027 bytes,
`fastprom_web.js` 49,231 bytes (wasm-bindgen's glue), the page's own
JavaScript about 42 KB, unminified.

### Tests

* `cargo test` (`make fw-test`), `fw/fastprom-host/tests`:
  - `protocol.rs`: refusals of each kind and the next request after them;
    a packet lost or damaged at every packet of a small update, in either
    direction, recovered from by reset and resend (and the seven losses of
    a transfer's only short packet ending in a timeout, then a reset); a
    stale answer; a device of another protocol version refused at once;
    answers that never make sense ending after three attempts; the frames
    and fresh sequence numbers on the wire; the interface rule on both
    firmwares' descriptors;
  - `table.rs`: every table `fw/tools/gen_fixtures.py` had
    `mkimagetable.py` write - from `mkimage.py`'s text files and from
    binary files with non-ASCII and over-long names - built again from the
    same files and compared byte for byte; ROM-file checks; the plan;
  - `update.rs`: connect, table write/verify/`LOAD`, firmware and fabric
    through DFU and back, a wrong board revision, power cuts during a
    firmware, fabric and table update and the recovery from each (the
    table's: INFO's whole header over images that are not), and the image
    table's part standing in for a missing manifest;
  - `recovery.rs`: UART recovery of a blank board through the mask-ROM
    model and the loader's server, then on USB; the DFU stage alone;
    faults on the wire retried; a wrong device ID, another revision's
    flash and bad recovery files refused; a second run finding the loader
    running; a power cut part way and a second run.
* `make web-test` (`web/tests/page.spec.js`), with synthetic bundles for
  board revisions A and B: connect; the update interface found by
  `usb.js` in both firmwares' configuration descriptors; an image update with `LOAD` behind the
  host-off confirmation; firmware and fabric through DFU; a power cut
  mid-update, power on, reconnect (DFU mode) and finish; the wrong board
  revision refused; a browser without WebUSB or Web Serial told so;
  a power cut deep in a fabric update leaving the board in update mode on
  USB, finished there; the image table's part used when the manifest is
  gone (a fabric update
  cut after its first erase), and a table write cut short shown from INFO;
  UART recovery of a blank board (`?sim&blank`), BOOT0 held high, then
  released and connected over USB; a lost and a corrupted byte on the
  wire, retried; a chip with another device ID refused with nothing
  written; the revision confirmation; phone width (Pixel 7, no horizontal
  scroll); and the bundle `make web` published, updating a board with an
  older version and recovering a blank one. Every test fails on any
  console error or warning. About 20 s with the image pulled;
  `scripts/release_fw.sh` runs it.

## Release

`scripts/release_fw.sh` builds the page with the release's application in
its bundle, runs `make web-test`, and writes `fastprom-web-<version>.zip`
(fixed timestamps, name order) beside the firmware; the release workflow
attaches it. That page's bundle has no fabric (building one needs AGM's
SDK), so it offers firmware only. `scripts/release_fabric.sh`, run at the
tag on a machine with the SDK, builds the BRAM fabric for each part, rebuilds
the page with them in its bundle, runs the browser tests, and with
`--upload` adds `fastprom-fabric-<version>.zip` to the release and replaces
the page zip (`06-build-and-flash.md`, "Releases"). Nothing is deployed:
where the page is hosted is not decided.
