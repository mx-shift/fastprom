# The USB driver

The USB device both firmwares present (`10-updates.md`, "USB device"): the
AG32's controller, a small device stack, the descriptors, and the update
and CDC-ACM functions. This page is how the controller is brought up and
where each step comes from, how the stack is built, and the checks that
need a board. Nothing here has run on hardware.

| where | what |
|---|---|
| `fw/fastprom-hal/src/usb.rs` | `Ag32Usb`: the ChipIdea controller's registers and DMA memory (registers in `regs.rs`, module `usb`) |
| `fw/fastprom-hal/src/flash.rs` | `Ag32Flash::unique_id`: the serial number's source |
| `fw/fastprom-usb` | everything above the registers, no hardware: `bus.rs` (endpoints over the `Controller` trait), `device.rs` (control pipe, standard requests, dispatch), `desc.rs` and `msos.rs` (descriptors), `update.rs` (the update interface), `cdc.rs`, `eps.rs` (the endpoint plan, the one place endpoint numbers are), `ids.rs`, `dqh.rs` |
| `fw/fastprom-ag32/src/main.rs`, `fw/fastprom-dfu/src/main.rs` | `Device::new(Ag32Usb, serial, update::Server, CdcAcm or NoFunction)`, then `poll` in the main loop |

## Sources

| | what | used for |
|---|---|---|
| AGM SDK | `framework-agrv_sdk/src/AltaRiscv.h`, `usb.h`, `usb.c`, `system.h`, `peripherals.h`, `flash.c` | every register, bit and structure layout |
| AGM's TinyUSB port | `framework-agrv_tinyusb/hw/mcu/agm/agrv2k.c`, `agrv2k_dcd.c`, `agrv2k.h` (MIT) | the bring-up sequence, endpoint numbering, dQH/dTD use |
| AGM's DFU example | `platforms/AgRV/examples/dfu` (`src/boot.c`, `platformio.ini`) | what the CPU does before `tusb_init` (reference only: no licence) |
| imxrt-usbd 0.3.0 | `src/driver.rs`, `bus.rs`, `endpoint.rs` (MIT OR Apache-2.0, imxrt-rs) | how a polled driver over the same ChipIdea core is shaped |
| USB 2.0 ch. 9; CDC 1.2 / PSTN 1.2; WebUSB 3.2; MS OS 2.0 descriptors | | the stack and every descriptor |

No code is copied from any of them; `bus.rs` follows imxrt-usbd's structure
(one dTD per endpoint half, a packet at a time) and says where it departs.

## Bring-up, in order

The fabric does the first two steps: the configuration's `USB0 device`
enables the controller and gen_vlog gives it PLL output 1, 60 MHz. The CPU
cannot make that clock itself and does not try. Everything after is
`Ag32Usb::init` and `Bus::enable`, run by `Device::new`:

1. **AHB clock.** `SYS->AHB_CLKENABLE |= AHB_MASK_USB0` (bit 1).
   `peripherals.h` PERIPHERAL_USB_ENABLE is exactly this, and the DFU
   example's `setupBoot` calls it before `tusb_init`. No AHB reset is
   pulsed: the SDK does not.
2. **Core reset.** `USBCMD.RST`, then wait for it to clear (`usb.c`
   USB_Reset). Bounded here; a timeout is reported as `Fault::Init`.
3. **Device mode.** `USBMODE = CM_DEVICE (2) | SLOM`, one write (`usb.c`
   USB_InitDevice sets CM; SLOM is `usb.h` USB_MODE_SLOM, see below).
4. **Interrupts off.** `USBINTR = 0`. The driver polls `USBSTS`.
5. **Interrupt threshold 0** (`USBCMD.ITC`), as `agrv2k_dcd.c` dcd_init.
6. **Endpoint list.** The dQH/dTD memory zeroed, `ENDPOINTLISTADDR` = its
   address (`agrv2k_dcd.c` dcd_init). `DEVICEADDR = 0`, `USBSTS` cleared.
7. **Endpoint 0.** Both dQHs: max packet 64, ZLT disabled, IOS on the OUT
   one (`usb.c` USB_InitDQH; `agrv2k_dcd.c` bus reset).
8. **Connect.** `USBCMD.RS = 1` (`usb.h` USB_Run). In device mode this is
   the D+ pull-up: AGM's `dcd_connect` is USB_Run and `dcd_disconnect`
   USB_Stop. There is no separate pull-up or PHY register in the SDK or
   the port.

What the AG32 does **not** need, unlike the i.MX RT: no USBPHY (power-down,
clock gate, soft reset) and no USBNC block. AGM's port has neither and the
SDK names neither. The PHY is a hard block on PIN_22/PIN_23 with no
CPU-side configuration.

On every **bus reset** (`USBSTS.URI`): `usb.c` USB_BusReset step for step -
ENDPTNAKEN 0, write back ENDPTSETUPSTAT, ENDPTCOMPLETE, ENDPTNAK and USBSTS,
wait for ENDPTPRIME to clear, flush everything, wait for the flush, wait for
`PORTSC.PR` to clear - then `DEVICEADDR = 0` and every dQH and dTD zeroed
(as `agrv2k_dcd.c`), then endpoint 0 and every endpoint of the plan opened
again and the bulk OUT endpoints armed. Each wait is bounded
(`Fault::Reset`).

## Endpoints

The SDK states four endpoints (`USB_ENDPOINTS 4`, `ENDPTCTRL[4]`), each with
an OUT and an IN half, 64 bytes at full speed (`USB_FS 1`,
`USB_MAX_PACKET_LEN`). The application needs seven halves
(`eps::PLAN_APP`); the DFU stage four (`eps::PLAN_DFU`):

| endpoint | OUT | IN |
|---|---|---|
| 0 | control | control |
| 1 | update bulk 64 | update bulk 64 |
| 2 | CDC data bulk 64 (application) | CDC data bulk 64 (application) |
| 3 | unused | CDC notification, interrupt 16, 255 ms (application) |

So CDC fits beside the update interface with one half to spare.

The controller numbers halves as `agrv2k.h` does: queue head `2 * n + dir`
(dcd_ep_num2idx) and bit `n + 16 * dir` in ENDPTPRIME / ENDPTFLUSH /
ENDPTSTATUS / ENDPTCOMPLETE / ENDPTSETUPSTAT (dcd_ep_idx2bit), fields at
shift `16 * dir` in ENDPTCTRL[n] (`usb.h` USB_ENDPT_OFFSET). The tests check
all eight.

Endpoint 3's unused OUT half is set to bulk (and every other non-zero
endpoint's halves to bulk, disabled, before the used ones are enabled): a
half left at the reset type, control, beside an enabled one is a known
ChipIdea hazard for the enabled half's data toggle (the i.MX RT manual;
imxrt-usbd's `initialize` does the same).

## Memory

One static `EndpointMemory`, 4 KiB aligned: 8 dQHs (64 bytes, 64-aligned,
`usb.h` USB_DQH), 8 dTDs (32 bytes, 32-aligned, USB_DTD), 8 packet buffers
of 64 bytes. The dQH's transfer overlay sits at offset 0x08, so it is its
own unaligned type, not a dTD. AGM's port
aligns its `usb_data` to 4096 (`agrv2k.c`) and so does imxrt-usbd's
queue-head list; the SDK states no requirement for ENDPOINTLISTADDR, so the
larger known one is kept. A 64-byte buffer at a 64-byte boundary never
crosses a 4 KiB page, so a dTD needs one page pointer (the others are
filled as `usb.c` USB_InitDTD does anyway).

Cost: the struct is 1280 bytes, padded to 4096 by its alignment; with the
rest of `.bss` before it, both binaries have 8 KiB of `.bss`. No flash
is spent on it: the memory is zeroed one queue head at a time, not copied
from a 4 KiB constant.

## Transfers

One dTD per half, one packet per dTD (as imxrt-usbd): length, Active, IOC
(`usb.h` USB_InitDTD; `agrv2k_dcd.c` dcd_edpt_xfer sets IntOnComplete),
linked into the dQH overlay with the overlay's status cleared, a fence, a
write to ENDPTPRIME, a bounded wait for its bit to clear. A half counts as
busy while its bit is set in ENDPTPRIME or ENDPTSTATUS. Bytes received are
the dTD's length less what the controller left in TotalBytes; a dTD with
Halted, DataError or TranError set delivers nothing.

- **SETUP.** Read with the tripwire: clear ENDPTSETUPSTAT bit 0, then set
  `USBCMD.SUTW`, copy the 8 bytes from dQH 0 offset 0x28, and keep the copy
  only if SUTW is still set (imxrt-usbd `read_setup`; TinyUSB's `ci_hs`).
  This is why SLOM is set (setup lockout off). AGM's port leaves SLOM at
  reset and copies without the tripwire. A SETUP also flushes whatever is
  still armed on endpoint 0 (Linux's ChipIdea UDC does the same), then
  endpoint 0 OUT is armed for a data or status stage.
- **SET_ADDRESS** is applied on receipt, with `DEVICEADDR = addr << 25 |
  1 << 24` (`usb.h` USB_SetDeviceAddress; bit 24, USBADRA, defers it to
  after the status stage).
- **Halts.** ENDPTCTRL TXS/RXS. Clearing one on endpoints 1-3 also resets
  the data toggle (`agrv2k_dcd.c` dcd_edpt_clear_stall) and re-arms an OUT
  half; halting an IN half flushes the packet armed on it.
- **Suspend** is not acted on: the module does not lower its power.
- Endpoints 1-3 are opened and their OUT halves armed at the bus reset, not
  at SET_CONFIGURATION. The host sends nothing to them before configuring,
  and their toggles start at DATA0.

## The device stack

### Why not usb-device

A scratch application implementing `usb-device` 0.3's `UsbBus` and linking
it (release, LTO, as the firmware) pulls in `core::panicking::panic`,
`panic_fmt`, `panic_bounds_check` and `panic_already_borrowed` from 15 call
sites, every one inside usb-device 0.3.2: each `Endpoint` read, write, stall
and unstall checks "UsbBus initialization not complete"; the allocator
borrows a `RefCell`; its descriptor writers and control pipe index slices.
`tools/fw_layout.py` fails any firmware that links one, so `device.rs` is
used instead.

### What it does

`device.rs` is what a device with one configuration and fixed functions
needs (USB 2.0 chapter 9), with no panicking index, no `RefCell`, no
`unwrap`:

- **Control pipe.** SETUP; an IN data stage in 64-byte packets, ending with
  a short packet, and with a zero-length one only when the answer is shorter
  than wLength and a multiple of 64; then the host's zero-length OUT status.
  An OUT data stage of up to 128 bytes, then a zero-length IN status. A
  refused request stalls both halves of endpoint 0 until the next SETUP.
- **Standard requests.** GET_STATUS (device: bus powered, no remote wakeup;
  interface; endpoint: halted or not), CLEAR/SET_FEATURE(ENDPOINT_HALT),
  SET_ADDRESS, GET_DESCRIPTOR (device, configuration, strings 0-3, BOS),
  GET/SET_CONFIGURATION (value 1), GET/SET_INTERFACE (alternate setting 0
  only). Everything else stalls: DEVICE_QUALIFIER (a full-speed-only device
  has none), remote wakeup (not offered), other strings or configurations.
- **Vendor requests to the device.** The MS OS 2.0 descriptor set
  (`bMS_VendorCode` 2, wIndex 7). WebUSB's GET_URL stalls: there is no
  landing page.
- **Class and vendor requests to an interface**, by number: the update
  interface's to `UpdateClass` (its one request, RESET: vendor, host to
  device, `bRequest` 0x01, no data); the CDC interfaces' to `CdcAcm`
  (SET/GET_LINE_CODING, SET_CONTROL_LINE_STATE, SEND_BREAK); any other
  stalls.

The second function is a type parameter, `CdcAcm` or `NoFunction`, and it
decides the variant: the descriptors, the endpoint plan and the interface
numbers follow from it at compile time, so the two cannot disagree.

## Descriptors

Constants built at compile time (`desc.rs`, `msos.rs`); the bytes are
written out by hand in `fw/fastprom-usb/tests/descriptors.rs`.

| | application | DFU stage |
|---|---|---|
| device class | 0xEF/0x02/0x01 (IAD) | 0x00 |
| bcdDevice | 0x0100 | 0x0001 |
| product | `FastPROM` | `FastPROM (DFU)` |
| interfaces | 0 update (vendor FF/00/00, EP1 bulk OUT and IN); 1, 2 CDC-ACM (IAD) | 0 update |
| configuration | 98 bytes | 32 bytes |
| BOS | 64 bytes: USB 2.0 ext., WebUSB, MS OS 2.0 (set length 178) | the same, set length 162 |
| MS OS 2.0 set | 178 bytes: header, configuration subset, function subset for interface 0, `WINUSB`, `DeviceInterfaceGUIDs` | 162 bytes: header, `WINUSB`, `DeviceInterfaceGUIDs` |

Both: VID 0x1209, PID 0x0001, `bcdUSB` 0x0210, `bMaxPacketSize0` 64,
manufacturer `FastPROM`, serial number the chip's unique ID, bus powered,
100 mA. WebUSB vendor code 1 with no landing page; MS OS 2.0 vendor code 2,
Windows 8.1 and later. The device interface GUID,
`{B4F828EE-BE3A-4C34-A662-FE495ADE6F92}`, was generated for FastPROM.

**Interface numbers.** The update interface is 0 in both firmwares, so the
MS OS 2.0 function subset and the page's claim name the same number in
either; the page still finds it by class and endpoints, not by number.

**Serial number.** The AG32 has no unique-ID register in the SDK; its ID is
the flash's, read with command 0x4B (Read Unique ID) through the flash
controller's flexible read (`flash.c` FLASH_GetUniqueID: four FlexReads of
4, 8, 12 and 16 bytes after 4 dummy bytes, READ_DATA keeping each read's
last word). `Ag32Flash::unique_id` does that from a `.ramfunc` routine,
beside the erase and program ones, so no instruction is fetched from the
flash while the command runs (`DESIGN_NOTES.md`, "Flash routines run from
SRAM"). The serial is the four words as 32 upper-case hex digits; both
firmwares read the same ID, so they share it.

## The update interface

`UpdateClass` (`update.rs`) owns the update protocol's server
(`fastprom_core::update::Server`, behind the `Service` trait so the stack
can be tested with any protocol) and only moves packets:

- **OUT.** A packet waiting on EP1 OUT is read and handed to the server
  only while the server has no response still to send. Until then it stays
  in the controller, which NAKs the host's next one. A host reads each
  response before its next request, so in normal use nothing waits. A
  short packet ends a request; the server runs it then.
- **IN.** One packet armed at a time, as the server makes it (a READ's
  bytes come from flash packet by packet); when the host takes it, the
  next, and a zero-length one after a response that fills its last packet.
  `REBOOT` reboots when its response's last packet has been taken.
- **RESET** (the vendor request): the server drops its half-received
  request and unsent response, a packet armed on EP1 IN is flushed
  (`Bus::flush`), and a packet the controller holds on EP1 OUT is read and
  dropped, whether or not the bus has reported it yet (it may have come in
  the same poll as the SETUP). A bus reset does the same.
- **Halts.** The class never halts an endpoint: a refusal is a status in
  the response. A halt the host sets and clears (SET/CLEAR_FEATURE) is the
  stack's, as on any endpoint, toggle reset included.

## The CDC port

`CdcAcm`, application only: interfaces 1 and 2, a 512-byte transmit FIFO
drained a packet at a time, a zero-length packet after a transfer that ends
on a full packet (Linux reads two packets at a time), nothing ever sent on
the notification endpoint. It carries the boot report: `report.rs`'s
console keeps a 512-byte copy of everything it prints, written before USB
starts, and the main loop hands it to the port as the port has room; the
repeated failure line follows. What the copy cannot hold is dropped, and
the port keeps what the host has not yet read, so a terminal opened late
sees the start of the report and the next repeat.

## Size and panics

`make fw-build`: both ELFs "panic paths linked: none".

| | DFU stage | application |
|---|---|---|
| flash used | 19.9 KiB of 32 KiB (20,332 bytes) | 33.5 KiB of 48 KiB (34,308 bytes, header and code) |
| USB stack alone (scratch link, trivial update operations) | 10.0 KiB | 11.1 KiB |
| the same with usb-device, for comparison | 15.7 KiB | 17.6 KiB |

The DFU stage's flash use includes its own fabric bitstream (3,226 bytes).

## Unverified until a board runs it

1. That `USB0 device` in the `.ve` really leaves the controller clocked
   and out of reset for the CPU, with nothing else to enable.
2. That nothing besides `USBCMD.RS` is needed for the D+ pull-up (no OTG
   or VBUS-sense condition: `OTGSC` is never touched, as in AGM's port).
3. That `USBSTS` latches URI and UI with `USBINTR` 0 (standard EHCI
   behaviour, and what imxrt-usbd relies on; AGM's port only ever reads it
   from its ISR).
4. That the core accepts SLOM and the tripwire (both bits are in `usb.h`,
   neither is used by AGM's port).
5. That a 4 KiB-aligned endpoint list is what ENDPOINTLISTADDR needs (2 KiB
   is ChipIdea's usual requirement; AGM uses 4 KiB).
6. That no data-cache maintenance is needed between the CPU and the
   controller's DMA (AGM's port has none; the `fence iorw, iorw` is cheap
   insurance).
7. That DEVICEADDR keeps 7 address bits: `usb.h` USB_GetDeviceAddress masks
   the field with 0x0F, which, if the hardware really has only 4 bits,
   breaks enumeration at addresses above 15.
8. That the unique-ID read works from SRAM as written and that the four
   words differ between chips.
9. That the endpoint 3 OUT half left unused, set to bulk and disabled,
   does not disturb endpoint 3 IN.
10. That Windows treats the application (composite, bcdDevice 0x0100) and
    the DFU stage (single interface, 0x0001) under one serial number as the
    same device without a stale driver binding: the distinct bcdDevice is
    meant to keep its MS OS descriptor cache apart; to be checked on
    Windows 10 and 11. And that Windows binds WinUSB to the application's
    interface 0 through the function subset and the composite driver
    binds the CDC port beside it.
11. That Chrome on each platform lets the page claim interface 0 of the
    composite application with the CDC port bound to the OS's serial
    driver, and sends the vendor RESET request to it.
12. Current: 100 mA (one unit load, what any device may draw before it is
    configured) is declared, not measured.
