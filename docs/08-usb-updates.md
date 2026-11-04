# USB

Plugging in a USB cable to change the ROM image is a large part of what makes
One ROM pleasant to use. FastPROM does it the same way from the user's side:
a web page in Chrome or Edge, a USB-C cable, nothing to install. This page is
the hardware that makes it possible and what it costs; the design built on
it is `10-updates.md` (flash map, DFU stage, the update protocol), the USB
driver `11-usb-driver.md`, and the page `12-web.md`. None of it has run on
hardware yet.

## What is on the board

On `fastprom-upper`:

* **J4**, a USB 2.0 Type-C receptacle (HRO TYPE-C-31-M-12), on top at the
  board's far end (the socket's pin 11 end), opening outward;
* **R22, R23**, 5.1 k on CC1 and CC2, so a source sees a device;
* **U6**, a USBLC6-2SC6 ESD clamp on D+ and D-, with C19 (100 nF) on its
  VBUS pin;
* **R27 and C20**, 1 R in series with 4.7 uF, damping the cable's hot-plug
  ringing;
* **D2, D3**, Schottkys OR-ing the socket's 5 V and VBUS into the AG32's
  regulator;
* **X1**, a 12 MHz oscillator into OSC_IN (HSE bypass). The PLL runs its VCO
  at 1200 MHz from it, and the fabric configuration gives USB its 60 MHz
  from output 1 (`06-build-and-flash.md`, "Pitfalls").

The calculations behind C19, R27/C20 and the 10 uF VBUS limit are in
`../DESIGN_NOTES.md` ("USB VBUS: clamp decoupling and a hot-plug damper").

## What One ROM gets for free and this does not

One ROM's microcontrollers have a **USB bootloader in mask ROM**. An RP2350
held in BOOTSEL appears as a mass-storage device with no firmware present at
all; an STM32F4 has DFU in system memory. Either way, a virgin chip talks USB
out of the box.

**The AG32 has no factory USB bootloader.** It has USB FS with OTG in
hardware, but the uploader on AGM's own boards is an *application* that has to
be installed first, over the UART mask-ROM loader. There is no DFU and no
mask-ROM USB path.

So FastPROM supplies that boot loader itself, and that shapes the design:

* **A DFU stage at the reset address** (`10-updates.md`), never written over
  USB, serves updates when the application asks for it or is missing. It
  carries its own USB-only fabric configuration, because the fabric enables
  the USB controller: a damaged main fabric cannot take USB with it.
* **One-time provisioning is still over UART.** A blank chip has no DFU
  stage; the page writes one through the mask ROM's UART loader, with a
  USB-UART adapter on J3 (`10-updates.md`, "Recovery"). The same path
  recovers a board whose DFU stage is damaged, which is why the UART pads
  stay on the board.

## The pin cost

The USB PHY is a hard block wired to **PIN_22 and PIN_23**. It is not
fabric-routable - unlike every other signal on the board, it cannot be moved.

The QFN-32 has 24 usable fabric I/O and the stack uses all of them: 18 socket
lines, the data buffer's enable, the 12 MHz clock into OSC_IN, USB D+/D-, and
UART TX/RX. The two image-select bits do not get pins of their own: they are
read from the UART pins through the DIP switch SW1, with 100 k pull-downs,
before firmware enables the UART (`../DESIGN_NOTES.md`, "Image select on the
UART pins"). So the stack has USB, four images and UART recovery together.

UART stays because USB only works if the firmware implementing it is intact,
while the UART mask-ROM path works on blank or corrupted flash with no
firmware at all. Without it, USB and its own recovery route would share a
single point of failure.

## Which mode would benefit

The two fabric modes have quite different update stories.

**BRAM mode is the natural fit.** Contents live in flash as a plain data blob
that the core copies into block RAM at boot, so an update is: upload bytes,
verify CRC, reboot. BRAM mode is the shipped mode; its timing against each grade is
in `03-timing-budget.md`.

**LOGIC mode would still work, but the host does the work.** The contents are
synthesised into the fabric bitstream, so changing them means re-running
synthesis on the PC and uploading a new 99,944-byte fabric image. The user
sees one USB operation either way, but the host has to run Yosys.

The update path keeps USB alive through any failed update. The DFU stage
at `0x80000000` serves USB and is not what an update replaces; the
application at `0x80008000`, the fabric at `0x80014000` and the image table
at `0x8002E000` are payloads it writes (`10-updates.md`, "Flash map"). An
interrupted firmware or fabric update leaves an uncommitted header or
manifest, and the DFU stage then stays in update mode on its own fabric
instead of booting it. Only a damaged DFU stage needs the UART.

## Power, and one way to damage a host board

With USB-C on the module there are two possible 5 V sources: the socket when
it is seated, and VBUS on the bench. Both need to work, and they must not
fight.

D2 and D3 OR them into the AG32's regulator. The USB-side diode is the
obvious one. **The socket-side diode is the one that matters**: without it,
plugging in a USB cable while the module is seated would push 5 V back out of
socket pin 20 and into the host's own supply rail, powering part of a vintage
machine through a PROM socket. With the diode, current cannot leave the
module that way. The cost is a few tenths of a volt of drop, which the 3.3 V
regulator does not notice.

The bus buffers are not on this rail at all. They run from their own
regulator on the lower board, fed from socket pin 20 only, so with the host
switched off and the module on USB they are unpowered and high-impedance
(`02-electrical.md`). Their enable pull-up, R21, then back-feeds at most
0.33 mA into that dead rail, which powers nothing (`../DESIGN_NOTES.md`).

USB and a powered host connected at once join their grounds through the
module; `06-build-and-flash.md` ("Handling") has what to do about it.

The exposed data lines get an ESD clamp. The AG32's USB PHY is not 5 V
tolerant and the connector is the only part of the module a person touches,
so it is not optional.

## Mechanical

A USB-C receptacle is about 8.9 mm wide and needs roughly 7 mm of depth
behind the opening. On a single board in a DIP-20 footprint it does not fit:
after the socket's pad rows there is a 5.08 mm channel, and the 1.7 mm past
each end of the pin field is nowhere near deep enough. One ROM's 24- and
28-pin targets are on 0.6 inch row spacing, twice the width.

The stack is what makes it fit. The receptacle sits on top of the upper
board, which has the whole board to itself, at the end opening outward. The
upper board is the lower board's width and only 1.5 mm longer, at that end,
so the receptacle's locating-peg holes clear the DF40 socket underneath
(`09-two-board-stack.md`). The top of the receptacle, about 14 mm above the
host PCB, is the tallest point of the module.

## How it is done

* the USB device: the AG32's ChipIdea controller at full speed, clocked at
  60 MHz from PLL output 1 by the fabric configuration (`USB0 device` in the
  `.ve`), WebUSB and Microsoft OS 2.0 descriptors so no driver is installed
  (`11-usb-driver.md`);
* the update protocol: FastPROM's own, one request and one response per USB
  transfer, each with a sequence number and a CRC-32, and every payload
  committed only once its CRC matches (`10-updates.md`, "The update
  protocol");
* the page: Rust compiled to WebAssembly for the protocol, the image table
  and the update procedures, tested against a simulated board
  (`12-web.md`).

The checks that need a board are listed in `10-updates.md` and
`11-usb-driver.md`.
