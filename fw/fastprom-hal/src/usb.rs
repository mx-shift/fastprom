//! The AG32's USB controller, as fastprom-usb's [`Controller`]: the
//! register-level half of the USB driver, and its DMA memory.
//!
//! A ChipIdea device controller at USB0_BASE, used at full speed. Every
//! register and bit is from AGM's AgRV_pio SDK,
//! `packages/framework-agrv_sdk/src/` (`AltaRiscv.h`, `usb.h`, `usb.c`,
//! `system.h`, `peripherals.h`; `regs.rs` names them), and every step of the
//! sequence from AGM's TinyUSB port of the same controller,
//! `packages/framework-agrv_tinyusb/hw/mcu/agm/` (`agrv2k.c`,
//! `agrv2k_dcd.c`), unless it says otherwise. `docs/11-usb-driver.md` has
//! the bring-up in order, and what only hardware can confirm. The device
//! stack, the descriptors and the classes above this are `fw/fastprom-usb`,
//! host-tested against a model of this same interface.
//!
//! What the CPU does not do: the 60 MHz USB clock and the controller's
//! enable come from the fabric configuration (`USB0 device` in the `.ve`;
//! gen_vlog gives USB PLL output 1). The SDK has no PHY register, and
//! neither does AGM's port: there is nothing to configure between the
//! controller and the pins.

use core::cell::UnsafeCell;
use core::ptr::{addr_of, addr_of_mut};
use core::sync::atomic::{AtomicBool, Ordering};

use fastprom_usb::bus::{Controller, Status};
use fastprom_usb::dqh::{self, Buffer, EndpointMemory, Qh, Td, TERMINATE};
use fastprom_usb::eps::{EpIndex, EpType, MAX_PACKET, QUEUE_HEADS};

use crate::mmio::Reg;
use crate::regs::{sys, usb};

/// How many polls a register wait gets before it is given up. The
/// controller's own operations (reset, prime, flush, the end of a bus
/// reset) take microseconds; this is milliseconds at 100 MHz.
const POLLS: u32 = 100_000;

/// The DMA memory: dQHs, dTDs and packet buffers, in one static.
struct Memory(UnsafeCell<EndpointMemory>);

// SAFETY: reached only through the one Ag32Usb that TAKEN lets exist,
// which accesses it volatile.
unsafe impl Sync for Memory {}

static MEMORY: Memory = Memory(UnsafeCell::new(EndpointMemory::ZERO));
static TAKEN: AtomicBool = AtomicBool::new(false);

/// Order the dTD and dQH writes before the register write that hands them
/// to the controller, and its DMA writes before the reads that follow a
/// completion. AGM's C has no barrier; the core may not need one, but the
/// RISC-V memory model does not promise it without.
#[inline(always)]
fn io_fence() {
    #[cfg(target_arch = "riscv32")]
    // SAFETY: a fence has no operands and no effect beyond ordering.
    unsafe {
        core::arch::asm!("fence iorw, iorw", options(nostack, preserves_flags));
    }
    #[cfg(not(target_arch = "riscv32"))]
    core::sync::atomic::fence(Ordering::SeqCst);
}

/// Spin until `done`, at most [`POLLS`] times.
fn wait(mut done: impl FnMut() -> bool) -> bool {
    (0..POLLS).any(|_| done())
}

/// The AG32's USB0 controller, as a [`Controller`].
pub struct Ag32Usb {
    mem: *mut EndpointMemory,
}

// SAFETY: the pointer is to a static, and there is only ever one Ag32Usb.
unsafe impl Send for Ag32Usb {}

impl Ag32Usb {
    /// The controller, once: None if it has already been taken.
    ///
    /// Nothing else may drive USB0 or its AHB clock while this exists.
    pub fn take() -> Option<Self> {
        if TAKEN.swap(true, Ordering::AcqRel) {
            return None;
        }
        Some(Self {
            mem: MEMORY.0.get(),
        })
    }

    fn qh(&self, i: usize) -> *mut Qh {
        // SAFETY: mem points to the static EndpointMemory; the index is
        // masked into the array, so the pointer stays inside it. No
        // reference is formed.
        unsafe {
            addr_of_mut!((*self.mem).qh)
                .cast::<Qh>()
                .add(i % QUEUE_HEADS)
        }
    }

    fn td(&self, i: usize) -> *mut Td {
        // SAFETY: as qh.
        unsafe {
            addr_of_mut!((*self.mem).td)
                .cast::<Td>()
                .add(i % QUEUE_HEADS)
        }
    }

    fn buffer(&self, i: usize) -> *mut u8 {
        // SAFETY: as qh; a Buffer is MAX_PACKET bytes.
        unsafe {
            addr_of_mut!((*self.mem).buffer)
                .cast::<Buffer>()
                .add(i % QUEUE_HEADS)
                .cast::<u8>()
        }
    }

    fn list_address(&self) -> u32 {
        self.qh(0) as usize as u32
    }

    fn write_td(&self, i: usize, td: Td) {
        // SAFETY: an in-bounds, aligned pointer into the static; the
        // controller does not own this dTD while it is rewritten (the
        // half is not primed).
        unsafe { self.td(i).write_volatile(td) }
    }

    fn init_qh(&self, i: usize, info: u32) {
        let qh = self.qh(i);
        // SAFETY: as write_td, for the queue head; the half is idle.
        unsafe {
            qh.write_volatile(Qh::ZERO);
            addr_of_mut!((*qh).info).write_volatile(info);
            addr_of_mut!((*qh).overlay.next).write_volatile(TERMINATE);
        }
    }

    /// Every queue head and dTD zeroed, one at a time (a whole
    /// EndpointMemory::ZERO would be a 4 KiB constant in flash). The
    /// caller makes sure the controller is using none of them.
    fn clear_queues(&self) {
        for i in 0..QUEUE_HEADS {
            // SAFETY: in-bounds, aligned pointers into the static; the
            // caller's guarantee covers the controller.
            unsafe {
                self.qh(i).write_volatile(Qh::ZERO);
                self.td(i).write_volatile(Td::ZERO);
            }
        }
    }

    fn ctrl(num: u8) -> Option<Reg> {
        usb::ENDPTCTRL.get(usize::from(num)).copied()
    }
}

/// The two bits of an endpoint type's bmAttributes are ENDPTCTRL's type
/// field (AGM's dcd_edpt_open: `bmAttributes.xfer << 2`).
fn type_bits(kind: EpType) -> u32 {
    (kind as u32 & 0b11) << usb::CTRL_TYPE_SHIFT
}

impl Controller for Ag32Usb {
    fn init(&mut self) -> bool {
        // PERIPHERAL_ENABLE(USB, 0): the controller's AHB clock.
        sys::AHB_CLKENABLE.modify(0, sys::AHB_USB0);
        // usb.c USB_Reset: reset the core, wait for it to finish.
        usb::USBCMD.modify(0, usb::CMD_RST);
        let ok = wait(|| usb::USBCMD.read() & usb::CMD_RST == 0);
        // usb.c USB_InitDevice: device mode. Setup lockout off as well, in
        // the same write: the setup packet is read with the tripwire, as
        // imxrt-usbd and TinyUSB's ci_hs driver do. AGM's port leaves
        // SLOM at its reset value and reads the setup packet without the
        // tripwire.
        usb::USBMODE.write(usb::MODE_DEVICE | usb::MODE_SLOM);
        // Polled: no interrupt sources. USB_InitDevice enables six for its
        // ISR; USBSTS reports events whatever USBINTR holds (imxrt-usbd
        // polls with USBINTR 0).
        usb::USBINTR.write(0);
        // agrv2k_dcd.c dcd_init: interrupt threshold 0, endpoint list.
        usb::USBCMD.modify(usb::CMD_ITC, 0);
        // agrv2k_dcd.c dcd_init clears the endpoint memory first. Run is
        // off and nothing is primed, so the controller is not using it.
        self.clear_queues();
        usb::ENDPOINTLISTADDR.write(self.list_address());
        usb::DEVICEADDR.write(0);
        usb::USBSTS.write(usb::STS_EVENTS);
        ok
    }

    fn attach(&mut self) {
        io_fence();
        usb::USBCMD.modify(0, usb::CMD_RS);
    }

    fn take_status(&mut self) -> Status {
        let sts = usb::USBSTS.read() & usb::STS_EVENTS;
        if sts != 0 {
            usb::USBSTS.write(sts);
        }
        Status {
            reset: sts & usb::STS_URI != 0,
        }
    }

    fn bus_reset(&mut self) -> bool {
        // usb.c USB_BusReset, step for step, each wait bounded.
        usb::ENDPTNAKEN.write(0);
        usb::ENDPTSETUPSTAT.write(usb::ENDPTSETUPSTAT.read());
        usb::ENDPTCOMPLETE.write(usb::ENDPTCOMPLETE.read());
        usb::ENDPTNAK.write(usb::ENDPTNAK.read());
        usb::USBSTS.write(usb::USBSTS.read() & usb::STS_EVENTS);
        let mut ok = wait(|| usb::ENDPTPRIME.read() == 0);
        usb::ENDPTFLUSH.write(u32::MAX);
        ok &= wait(|| usb::ENDPTFLUSH.read() == 0);
        ok &= wait(|| usb::PORTSC.read() & usb::PORTSC_PR == 0);
        // Address 0 again. The SDK leaves this to the controller; Linux's
        // ChipIdea UDC writes it (hw_usb_reset), and it costs nothing.
        usb::DEVICEADDR.write(0);
        // agrv2k_dcd.c: the whole endpoint memory cleared on a bus reset.
        // Everything is flushed, so the controller owns none of it.
        self.clear_queues();
        ok
    }

    fn set_address(&mut self, addr: u8) {
        usb::DEVICEADDR.write((u32::from(addr & 0x7F) << usb::ADDR_SHIFT) | usb::ADDR_ADVANCE);
    }

    fn setup_pending(&self) -> bool {
        usb::ENDPTSETUPSTAT.read() & EpIndex::EP0_OUT.bit() != 0
    }

    fn read_setup(&mut self) -> Option<[u8; 8]> {
        // Acknowledge first, then copy under the tripwire: the controller
        // clears SUTW if another SETUP lands while the buffer is read, and
        // the copy is taken again (imxrt-usbd endpoint.rs read_setup).
        usb::ENDPTSETUPSTAT.write(EpIndex::EP0_OUT.bit());
        let qh = self.qh(EpIndex::EP0_OUT.queue_head());
        for _ in 0..POLLS {
            usb::USBCMD.modify(0, usb::CMD_SUTW);
            io_fence();
            // SAFETY: an aligned u32 pair inside the endpoint-0 OUT queue
            // head, which the controller writes and this only reads.
            let words = unsafe {
                [
                    addr_of!((*qh).setup[0]).read_volatile(),
                    addr_of!((*qh).setup[1]).read_volatile(),
                ]
            };
            if usb::USBCMD.read() & usb::CMD_SUTW != 0 {
                usb::USBCMD.modify(usb::CMD_SUTW, 0);
                let mut out = [0u8; 8];
                let (lo, hi) = out.split_at_mut(4);
                lo.copy_from_slice(&words[0].to_le_bytes());
                hi.copy_from_slice(&words[1].to_le_bytes());
                return Some(out);
            }
        }
        None
    }

    fn complete(&self) -> u32 {
        usb::ENDPTCOMPLETE.read()
    }

    fn clear_complete(&mut self, bits: u32) {
        if bits != 0 {
            usb::ENDPTCOMPLETE.write(bits);
        }
    }

    fn open(&mut self, ep: EpIndex, kind: EpType, max_packet: u16) {
        let i = ep.queue_head();
        // usb.h USB_InitDQH, with IOS on endpoint 0 OUT (agrv2k_dcd.c).
        self.init_qh(i, dqh::qh_info(max_packet, ep == EpIndex::EP0_OUT));
        self.write_td(
            i,
            Td {
                next: TERMINATE,
                ..Td::ZERO
            },
        );
        if ep.num() == 0 {
            // Endpoint 0 is always enabled, as a control endpoint.
            return;
        }
        // agrv2k_dcd.c dcd_edpt_open: type, toggle reset, enable.
        let Some(ctrl) = Self::ctrl(ep.num()) else {
            return;
        };
        let shift = ep.ctrl_shift();
        let field =
            (usb::CTRL_STALL | usb::CTRL_TYPE | usb::CTRL_TOGGLE_RESET | usb::CTRL_ENABLE) << shift;
        let set = (type_bits(kind) | usb::CTRL_TOGGLE_RESET | usb::CTRL_ENABLE) << shift;
        ctrl.modify(field, set);
    }

    fn close(&mut self, num: u8) {
        // A half left at the reset type (control) beside an enabled one is
        // a known ChipIdea hazard for the enabled half's data toggle; the
        // i.MX RT manual has the unused half set to bulk, and imxrt-usbd
        // sets each endpoint bulk and disabled first (endpoint.rs
        // initialize). Both halves are set so here, then open enables the
        // ones in use.
        if num == 0 {
            return;
        }
        if let Some(ctrl) = Self::ctrl(num) {
            ctrl.write(usb::CTRL_TYPE_BULK | (usb::CTRL_TYPE_BULK << 16));
        }
    }

    fn prime(&mut self, ep: EpIndex, len: u16) -> bool {
        let i = ep.queue_head();
        let len = len.min(MAX_PACKET);
        // usb.h USB_InitDTD, and agrv2k_dcd.c dcd_edpt_xfer: one dTD,
        // linked into the overlay, Active and Halted cleared there.
        let td = Td {
            next: TERMINATE,
            token: dqh::td_token(len),
            buffer: dqh::buffer_pages(self.buffer(i) as usize as u32),
            expected: u32::from(len),
        };
        self.write_td(i, td);
        let qh = self.qh(i);
        // SAFETY: the half is not primed, so the controller is not using
        // the overlay.
        unsafe {
            addr_of_mut!((*qh).overlay.next).write_volatile(self.td(i) as usize as u32);
            addr_of_mut!((*qh).overlay.token).write_volatile(0);
        }
        io_fence();
        let bit = ep.bit();
        usb::ENDPTPRIME.write(bit);
        // The controller clears the prime bit once it has the dTD
        // (imxrt-usbd schedule_transfer waits for it the same way).
        wait(|| usb::ENDPTPRIME.read() & bit == 0)
    }

    fn is_primed(&self, ep: EpIndex) -> bool {
        (usb::ENDPTPRIME.read() | usb::ENDPTSTATUS.read()) & ep.bit() != 0
    }

    fn flush(&mut self, ep: EpIndex) -> bool {
        let bit = ep.bit();
        usb::ENDPTFLUSH.write(bit);
        wait(|| usb::ENDPTFLUSH.read() & bit == 0)
    }

    fn transferred(&self, ep: EpIndex) -> Option<usize> {
        io_fence();
        let td = self.td(ep.queue_head());
        // SAFETY: an aligned dTD in the static, finished (not primed).
        let (token, expected) = unsafe {
            (
                addr_of!((*td).token).read_volatile(),
                addr_of!((*td).expected).read_volatile(),
            )
        };
        if dqh::td_failed(token) || token & dqh::TD_ACTIVE != 0 {
            return None;
        }
        Some(dqh::td_transferred(expected, token))
    }

    fn load(&mut self, ep: EpIndex, data: &[u8]) -> usize {
        let buf = self.buffer(ep.queue_head());
        let n = data.len().min(usize::from(MAX_PACKET));
        for (k, &b) in data.iter().take(n).enumerate() {
            // SAFETY: k < MAX_PACKET, inside this half's buffer, which the
            // controller is not reading (not primed).
            unsafe { buf.add(k).write_volatile(b) };
        }
        n
    }

    fn unload(&self, ep: EpIndex, out: &mut [u8], len: usize) -> usize {
        io_fence();
        let buf = self.buffer(ep.queue_head());
        let n = len.min(out.len()).min(usize::from(MAX_PACKET));
        for (k, o) in out.iter_mut().take(n).enumerate() {
            // SAFETY: as load; the controller has finished writing it.
            *o = unsafe { buf.add(k).read_volatile() };
        }
        n
    }

    fn set_stalled(&mut self, ep: EpIndex, stalled: bool) {
        let Some(ctrl) = Self::ctrl(ep.num()) else {
            return;
        };
        let shift = ep.ctrl_shift();
        if stalled {
            // usb.h USB_StallEndPt.
            ctrl.modify(0, usb::CTRL_STALL << shift);
        } else if ep.num() == 0 {
            // The controller clears endpoint 0's halt itself on the next
            // SETUP; clearing it here as well does no harm.
            ctrl.modify(usb::CTRL_STALL << shift, 0);
        } else {
            // agrv2k_dcd.c dcd_edpt_clear_stall: unhalt, toggle reset.
            ctrl.modify(usb::CTRL_STALL << shift, usb::CTRL_TOGGLE_RESET << shift);
        }
    }

    fn is_stalled(&self, ep: EpIndex) -> bool {
        Self::ctrl(ep.num()).is_some_and(|c| c.read() & (usb::CTRL_STALL << ep.ctrl_shift()) != 0)
    }
}
