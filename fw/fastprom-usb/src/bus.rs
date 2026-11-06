//! Endpoints over a ChipIdea device controller, polled: which halves
//! exist, when a transfer is armed, what finished.
//!
//! The registers sit behind [`Controller`] (`fastprom_hal::usb` on the chip; a model
//! of the controller in the host tests), so everything here is host-tested.
//! It follows the shape of imxrt-usbd's driver (`driver.rs`, `bus.rs`;
//! MIT OR Apache-2.0, by the imxrt-rs project), the same core on NXP's
//! i.MX RT: one dTD per endpoint half, a packet at a time, the setup packet
//! read with the tripwire. Where this differs it says why. The control
//! pipe and the requests are `device.rs`.

use crate::eps::{in_mask, out_mask, EpConfig, EpIndex, EpType, ENDPOINTS, QUEUE_HEADS};

/// What the controller saw since the last look.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Status {
    /// A bus reset (USBSTS URI).
    pub reset: bool,
}

/// The register-level operations the bus needs from a ChipIdea device
/// controller. Each is a few register or descriptor accesses; `fastprom_hal::usb`
/// cites the source of every one.
pub trait Controller {
    /// Start the controller in device mode, disconnected: clock, core
    /// reset, mode, endpoint list, interrupts off. False if the core did
    /// not come out of reset.
    fn init(&mut self) -> bool;
    /// Connect to the bus (the D+ pull-up).
    fn attach(&mut self);
    /// Read and clear the controller's status events.
    fn take_status(&mut self) -> Status;
    /// After a bus reset: clear every endpoint's pending state, flush,
    /// address 0, all queue heads and dTDs cleared. False on a timeout.
    fn bus_reset(&mut self) -> bool;
    /// Set the device address, to take effect after the status stage.
    fn set_address(&mut self, addr: u8);
    /// Whether endpoint 0 holds a SETUP packet not yet read.
    fn setup_pending(&self) -> bool;
    /// Take the SETUP packet, or None if no stable copy could be read.
    fn read_setup(&mut self) -> Option<[u8; 8]>;
    /// The transfer-complete bits (ENDPTCOMPLETE's layout,
    /// [`EpIndex::bit`]).
    fn complete(&self) -> u32;
    /// Clear transfer-complete bits.
    fn clear_complete(&mut self, bits: u32);
    /// Set up an endpoint half: its queue head, and for endpoints other
    /// than 0, its type, data toggle reset, enabled.
    fn open(&mut self, ep: EpIndex, kind: EpType, max_packet: u16);
    /// Both halves of endpoint `num` (not 0) disabled and of bulk type.
    fn close(&mut self, num: u8);
    /// Arm a transfer of `len` bytes into or out of the half's buffer.
    /// False if the controller did not take it.
    fn prime(&mut self, ep: EpIndex, len: u16) -> bool;
    /// Whether a transfer is armed and not finished.
    fn is_primed(&self, ep: EpIndex) -> bool;
    /// Cancel an armed transfer. False on a timeout.
    fn flush(&mut self, ep: EpIndex) -> bool;
    /// Bytes the last transfer moved, or None if it failed.
    fn transferred(&self, ep: EpIndex) -> Option<usize>;
    /// Copy into the half's buffer (at most one packet), and say how many.
    fn load(&mut self, ep: EpIndex, data: &[u8]) -> usize;
    /// Copy the first `len` bytes of the half's buffer out.
    fn unload(&self, ep: EpIndex, out: &mut [u8], len: usize) -> usize;
    /// Halt or unhalt; unhalting resets the data toggle.
    fn set_stalled(&mut self, ep: EpIndex, stalled: bool);
    /// Whether the half is halted.
    fn is_stalled(&self, ep: EpIndex) -> bool;
}

/// Something the bus could not do, kept for the firmware to report.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Fault {
    /// The core did not come out of reset.
    Init,
    /// A bus reset's flush did not finish.
    Reset,
    /// The controller did not take a transfer.
    Prime,
}

/// Why a read or write did not happen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Error {
    /// Nothing to read yet, or the last write has not gone.
    WouldBlock,
    /// The packet is larger than the buffer, or than the endpoint's.
    BufferOverflow,
    /// Not an endpoint of this device, or the wrong direction.
    InvalidEndpoint,
}

/// What [`Bus::poll`] found.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Event {
    /// Nothing.
    None,
    /// The host reset the bus; [`Bus::reset`] has been done.
    Reset,
    /// Transfers: a SETUP on endpoint 0, OUT halves with a packet to read
    /// (bit n for endpoint n), IN halves that finished.
    Data {
        /// A SETUP packet waits.
        setup: bool,
        /// OUT halves with a packet not yet read.
        out: u16,
        /// IN halves whose packet the host took.
        in_complete: u16,
    },
}

/// The endpoints of one device, on one controller.
pub struct Bus<C> {
    ctl: C,
    slots: [Option<EpConfig>; QUEUE_HEADS],
    /// OUT halves with a finished transfer not yet read (bit n for
    /// endpoint n). Latched here and cleared in the controller at once, so
    /// it is reported every poll until it is read, as imxrt-usbd does by
    /// leaving ENDPTCOMPLETE set.
    out_pending: u16,
    fault: Option<Fault>,
}

impl<C: Controller> Bus<C> {
    /// The endpoints in `plan` (endpoint 0's two halves included) on
    /// `ctl`. Nothing is touched until [`Bus::enable`]. An entry the
    /// controller has no half for is left out.
    pub fn new(ctl: C, plan: &[EpConfig]) -> Self {
        let mut slots = [None; QUEUE_HEADS];
        for cfg in plan {
            if let Some(ep) = EpIndex::from_address(cfg.addr) {
                if let Some(slot) = slots.get_mut(ep.queue_head()) {
                    *slot = Some(*cfg);
                }
            }
        }
        Self {
            ctl,
            slots,
            out_pending: 0,
            fault: None,
        }
    }

    /// The first thing the bus could not do, if anything.
    pub fn fault(&self) -> Option<Fault> {
        self.fault
    }

    fn record(&mut self, fault: Fault) {
        if self.fault.is_none() {
            self.fault = Some(fault);
        }
    }

    fn slot(&self, ep: EpIndex) -> Option<EpConfig> {
        self.slots.get(ep.queue_head()).copied().flatten()
    }

    /// Whether `addr` is one of this device's endpoints.
    pub fn has(&self, addr: u8) -> bool {
        EpIndex::from_address(addr).is_some_and(|ep| self.slot(ep).is_some())
    }

    fn checked(&self, addr: u8) -> Result<(EpIndex, EpConfig), Error> {
        let ep = EpIndex::from_address(addr).ok_or(Error::InvalidEndpoint)?;
        let cfg = self.slot(ep).ok_or(Error::InvalidEndpoint)?;
        Ok((ep, cfg))
    }

    /// Arm an OUT half for a whole packet.
    fn arm_out(&mut self, ep: EpIndex) {
        let mps = self.slot(ep).map_or(0, |s| s.max_packet);
        if !self.ctl.prime(ep, mps) {
            self.record(Fault::Prime);
        }
    }

    fn open_all(&mut self) {
        for num in 1..ENDPOINTS {
            self.ctl.close(num);
        }
        for i in 0..QUEUE_HEADS {
            let Some(cfg) = self.slots.get(i).copied().flatten() else {
                continue;
            };
            let Some(ep) = EpIndex::new((i / 2) as u8, i % 2 == 1) else {
                continue;
            };
            self.ctl.open(ep, cfg.kind, cfg.max_packet);
        }
        // Every OUT half other than 0 armed for its first packet. The host
        // sends none before SET_CONFIGURATION; they are made ready here,
        // at the reset that precedes it, and their toggles start at DATA0.
        for num in 1..ENDPOINTS {
            if let Some(ep) = EpIndex::new(num, false) {
                if self.slot(ep).is_some() {
                    self.arm_out(ep);
                }
            }
        }
    }

    /// Start the controller, open endpoint 0, connect.
    pub fn enable(&mut self) {
        let ok = self.ctl.init();
        for ep in [EpIndex::EP0_OUT, EpIndex::EP0_IN] {
            let mps = self.slot(ep).map_or(0, |s| s.max_packet);
            self.ctl.open(ep, EpType::Control, mps);
        }
        self.ctl.attach();
        if !ok {
            self.record(Fault::Init);
        }
    }

    /// After a bus reset: flush, address 0, every endpoint opened again.
    pub fn reset(&mut self) {
        if !self.ctl.bus_reset() {
            self.record(Fault::Reset);
        }
        self.out_pending = 0;
        self.open_all();
    }

    /// Set the device address. The controller latches it and applies it
    /// after the next IN transaction, the status stage (`usb.h`
    /// USB_SetDeviceAddress sets bit 24, USBADRA), so it is set on receipt
    /// of SET_ADDRESS.
    pub fn set_address(&mut self, addr: u8) {
        self.ctl.set_address(addr);
    }

    /// Bus reset first (done before returning); otherwise the SETUP and
    /// completion bits.
    ///
    /// Suspend is not reported: the module does not lower its power when
    /// the host suspends it (it is powered from the socket as well).
    pub fn poll(&mut self) -> Event {
        if self.ctl.take_status().reset {
            self.reset();
            return Event::Reset;
        }
        let setup = self.ctl.setup_pending();
        let complete = self.ctl.complete();
        self.ctl.clear_complete(complete);
        self.out_pending |= out_mask(complete);
        let in_complete = in_mask(complete);
        if !setup && self.out_pending == 0 && in_complete == 0 {
            return Event::None;
        }
        Event::Data {
            setup,
            out: self.out_pending,
            in_complete,
        }
    }

    /// Take the SETUP packet waiting on endpoint 0.
    pub fn read_setup(&mut self) -> Option<[u8; 8]> {
        if !self.ctl.setup_pending() {
            return None;
        }
        // A SETUP ends whatever endpoint 0 was doing: anything still armed
        // from the last transfer would otherwise answer this one. (Linux's
        // ChipIdea UDC empties both halves of endpoint 0 on a SETUP the same
        // way.)
        for ep in [EpIndex::EP0_OUT, EpIndex::EP0_IN] {
            if self.ctl.is_primed(ep) && !self.ctl.flush(ep) {
                self.record(Fault::Prime);
                return None;
            }
        }
        self.out_pending &= !1;
        let setup = self.ctl.read_setup()?;
        // Ready for a data stage, or the status stage of an IN transfer.
        self.arm_out(EpIndex::EP0_OUT);
        Some(setup)
    }

    /// Read the packet waiting on OUT endpoint `addr`, and arm for the
    /// next.
    pub fn read(&mut self, addr: u8, buf: &mut [u8]) -> Result<usize, Error> {
        let (ep, _) = self.checked(addr)?;
        if ep.is_in() {
            return Err(Error::InvalidEndpoint);
        }
        let bit = 1u16 << ep.num();
        if self.out_pending & bit == 0 || self.ctl.is_primed(ep) {
            return Err(Error::WouldBlock);
        }
        self.out_pending &= !bit;
        let result = match self.ctl.transferred(ep) {
            // A failed transfer leaves nothing to deliver; the next packet
            // is armed for below.
            None => Err(Error::WouldBlock),
            Some(n) if n > buf.len() => Err(Error::BufferOverflow),
            Some(n) => Ok(self.ctl.unload(ep, buf, n)),
        };
        self.arm_out(ep);
        result
    }

    /// Send one packet (at most the endpoint's size) on IN endpoint
    /// `addr`; a zero-length one for an empty `data`.
    pub fn write(&mut self, addr: u8, data: &[u8]) -> Result<usize, Error> {
        let (ep, cfg) = self.checked(addr)?;
        if !ep.is_in() {
            return Err(Error::InvalidEndpoint);
        }
        if data.len() > usize::from(cfg.max_packet) {
            return Err(Error::BufferOverflow);
        }
        if self.ctl.is_primed(ep) {
            return Err(Error::WouldBlock);
        }
        let n = self.ctl.load(ep, data);
        if !self.ctl.prime(ep, n as u16) {
            self.record(Fault::Prime);
            return Err(Error::WouldBlock);
        }
        // A control IN is followed by the host's zero-length OUT status
        // stage; an OUT control's status is this write itself. Either way
        // endpoint 0 OUT is kept armed (imxrt-usbd ctrl0_write).
        if ep == EpIndex::EP0_IN && !self.ctl.is_primed(EpIndex::EP0_OUT) {
            self.arm_out(EpIndex::EP0_OUT);
        }
        Ok(n)
    }

    /// Drop the packet armed on IN endpoint `addr` (other than 0), if the
    /// host has not taken it.
    pub fn flush(&mut self, addr: u8) {
        let Ok((ep, _)) = self.checked(addr) else {
            return;
        };
        if ep.is_in() && ep.num() != 0 && self.ctl.is_primed(ep) && !self.ctl.flush(ep) {
            self.record(Fault::Prime);
        }
    }

    /// Halt or unhalt endpoint `addr`.
    pub fn set_stalled(&mut self, addr: u8, stalled: bool) {
        let Ok((ep, _)) = self.checked(addr) else {
            return;
        };
        self.ctl.set_stalled(ep, stalled);
        // A packet armed on an IN half being halted is dropped: once the
        // halt is cleared it would answer a request it does not belong to.
        if stalled && ep.is_in() && ep.num() != 0 && self.ctl.is_primed(ep) && !self.ctl.flush(ep) {
            self.record(Fault::Prime);
        }
        // An OUT half coming out of a halt is armed again (imxrt-usbd
        // ep_stall); endpoint 0 is armed by each SETUP instead.
        if !stalled && !ep.is_in() && ep.num() != 0 && !self.ctl.is_primed(ep) {
            self.arm_out(ep);
        }
    }

    /// Whether endpoint `addr` is halted.
    pub fn is_stalled(&self, addr: u8) -> bool {
        self.checked(addr)
            .is_ok_and(|(ep, _)| self.ctl.is_stalled(ep))
    }
}
