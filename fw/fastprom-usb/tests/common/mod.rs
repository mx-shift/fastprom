//! A model of the ChipIdea device controller at the level of
//! `fastprom_usb::Controller`, a host that drives it a packet at a time,
//! and the device (the stack and FastPROM's classes) between them.
//!
//! The model is the controller as the driver sees it: per endpoint half a
//! 64-byte buffer, armed or not, the bytes the last transfer moved, a halt
//! bit; one SETUP buffer; the complete bits. A host transaction on a half
//! that is not armed is NAKed (returns None); on a halted half, STALLed.
#![allow(dead_code, reason = "each test crate uses a different subset")]

use std::sync::{Arc, Mutex, MutexGuard};

use fastprom_usb::bus::{Controller, Status};
use fastprom_usb::eps::EpType;
use fastprom_usb::eps::{EpIndex, MAX_PACKET, QUEUE_HEADS};
use std::cell::{Cell, RefCell};

use fastprom_core::header::{BoardRev, Version};
use fastprom_core::memmap::{FLASH_BASE, FLASH_SIZE};
use fastprom_core::update::{
    BootTarget, Flash, FlashError, Identity, Mode, Platform, ReloadError, Server, PAGE_LEN,
};
use fastprom_core::{Bank, TableSummary};
use fastprom_usb::{CdcAcm, Device as UsbDevice, NoFunction, Serial, Variant};

#[derive(Debug, Clone, Default)]
pub struct Half {
    pub open: Option<(EpType, u16)>,
    pub buffer: Vec<u8>,
    /// Armed for this many bytes.
    pub primed: Option<u16>,
    pub transferred: usize,
    pub failed: bool,
    pub stalled: bool,
    /// Data toggle resets seen (open or unhalt).
    pub toggle_resets: u32,
}

#[derive(Debug, Default)]
pub struct Hw {
    pub halves: Vec<Half>,
    pub initialised: bool,
    pub attached: bool,
    pub reset_event: bool,
    pub setup: Option<[u8; 8]>,
    pub complete: u32,
    pub address: u8,
    /// Every address write, in order.
    pub address_log: Vec<u8>,
    pub closed: Vec<u8>,
    pub bus_resets: u32,
}

impl Hw {
    fn half(&mut self, ep: EpIndex) -> &mut Half {
        &mut self.halves[ep.queue_head()]
    }
}

/// The controller side of the model, owned by the bus.
pub struct FakeController(pub Arc<Mutex<Hw>>);

impl FakeController {
    pub fn new() -> (Self, Arc<Mutex<Hw>>) {
        let hw = Arc::new(Mutex::new(Hw {
            halves: vec![
                Half {
                    buffer: vec![0; MAX_PACKET as usize],
                    ..Half::default()
                };
                QUEUE_HEADS
            ],
            ..Hw::default()
        }));
        (Self(hw.clone()), hw)
    }

    fn hw(&self) -> MutexGuard<'_, Hw> {
        self.0.lock().unwrap()
    }
}

impl Controller for FakeController {
    fn init(&mut self) -> bool {
        self.hw().initialised = true;
        true
    }

    fn attach(&mut self) {
        self.hw().attached = true;
    }

    fn take_status(&mut self) -> Status {
        let mut hw = self.hw();
        let reset = std::mem::take(&mut hw.reset_event);
        Status { reset }
    }

    fn bus_reset(&mut self) -> bool {
        let mut hw = self.hw();
        hw.bus_resets += 1;
        hw.setup = None;
        hw.complete = 0;
        hw.address = 0;
        for h in &mut hw.halves {
            h.primed = None;
            h.stalled = false;
            h.open = None;
        }
        true
    }

    fn set_address(&mut self, addr: u8) {
        let mut hw = self.hw();
        hw.address = addr;
        hw.address_log.push(addr);
    }

    fn setup_pending(&self) -> bool {
        self.hw().setup.is_some()
    }

    fn read_setup(&mut self) -> Option<[u8; 8]> {
        self.hw().setup.take()
    }

    fn complete(&self) -> u32 {
        self.hw().complete
    }

    fn clear_complete(&mut self, bits: u32) {
        self.hw().complete &= !bits;
    }

    fn open(&mut self, ep: EpIndex, kind: EpType, max_packet: u16) {
        let mut hw = self.hw();
        let h = hw.half(ep);
        h.open = Some((kind, max_packet));
        h.primed = None;
        h.stalled = false;
        h.toggle_resets += 1;
    }

    fn close(&mut self, num: u8) {
        self.hw().closed.push(num);
    }

    fn prime(&mut self, ep: EpIndex, len: u16) -> bool {
        let mut hw = self.hw();
        let h = hw.half(ep);
        assert!(h.primed.is_none(), "{ep:?} primed twice");
        h.primed = Some(len.min(MAX_PACKET));
        true
    }

    fn is_primed(&self, ep: EpIndex) -> bool {
        self.hw().halves[ep.queue_head()].primed.is_some()
    }

    fn flush(&mut self, ep: EpIndex) -> bool {
        self.hw().half(ep).primed = None;
        true
    }

    fn transferred(&self, ep: EpIndex) -> Option<usize> {
        let hw = self.hw();
        let h = &hw.halves[ep.queue_head()];
        (!h.failed).then_some(h.transferred)
    }

    fn load(&mut self, ep: EpIndex, data: &[u8]) -> usize {
        let mut hw = self.hw();
        let h = hw.half(ep);
        let n = data.len().min(MAX_PACKET as usize);
        h.buffer[..n].copy_from_slice(&data[..n]);
        n
    }

    fn unload(&self, ep: EpIndex, out: &mut [u8], len: usize) -> usize {
        let hw = self.hw();
        let h = &hw.halves[ep.queue_head()];
        let n = len.min(out.len());
        out[..n].copy_from_slice(&h.buffer[..n]);
        n
    }

    fn set_stalled(&mut self, ep: EpIndex, stalled: bool) {
        let mut hw = self.hw();
        let h = hw.half(ep);
        if h.stalled && !stalled && ep.num() != 0 {
            h.toggle_resets += 1;
        }
        h.stalled = stalled;
    }

    fn is_stalled(&self, ep: EpIndex) -> bool {
        self.hw().halves[ep.queue_head()].stalled
    }
}

/// What a host transaction got.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Reply {
    Nak,
    Stall,
    Data(Vec<u8>),
}

/// The host's side of the wire.
#[derive(Clone)]
pub struct Wire(pub Arc<Mutex<Hw>>);

impl Wire {
    pub fn hw(&self) -> MutexGuard<'_, Hw> {
        self.0.lock().unwrap()
    }

    pub fn bus_reset(&self) {
        self.hw().reset_event = true;
    }

    pub fn setup(&self, packet: [u8; 8]) {
        let mut hw = self.hw();
        // A SETUP clears endpoint 0's halt, in the controller.
        hw.halves[0].stalled = false;
        hw.halves[1].stalled = false;
        hw.setup = Some(packet);
    }

    /// An OUT transaction of `data` to endpoint `num`.
    pub fn out(&self, num: u8, data: &[u8]) -> Reply {
        let ep = EpIndex::new(num, false).unwrap();
        let mut hw = self.hw();
        let h = hw.half(ep);
        if h.stalled {
            return Reply::Stall;
        }
        let Some(len) = h.primed else {
            return Reply::Nak;
        };
        assert!(data.len() <= len as usize, "host sent more than armed");
        h.buffer[..data.len()].copy_from_slice(data);
        h.transferred = data.len();
        h.primed = None;
        hw.complete |= ep.bit();
        Reply::Data(Vec::new())
    }

    /// An IN transaction from endpoint `num`.
    pub fn r#in(&self, num: u8) -> Reply {
        let ep = EpIndex::new(num, true).unwrap();
        let mut hw = self.hw();
        let h = hw.half(ep);
        if h.stalled {
            return Reply::Stall;
        }
        let Some(len) = h.primed else {
            return Reply::Nak;
        };
        let data = h.buffer[..len as usize].to_vec();
        h.transferred = len as usize;
        h.primed = None;
        hw.complete |= ep.bit();
        Reply::Data(data)
    }
}

/// A board for the update server: the flash as memory (a pattern, so
/// reads show where they came from), reboots recorded.
pub struct RamBoard {
    pub flash: RefCell<Vec<u8>>,
    pub reboots: RefCell<Vec<BootTarget>>,
    pub loads: Cell<u32>,
}

impl RamBoard {
    pub fn new() -> Self {
        Self {
            flash: RefCell::new((0..FLASH_SIZE).map(|i| (i * 7 + 3) as u8).collect()),
            reboots: RefCell::new(Vec::new()),
            loads: Cell::new(0),
        }
    }

    pub fn bytes(&self, addr: u32, len: usize) -> Vec<u8> {
        let at = (addr - FLASH_BASE) as usize;
        self.flash.borrow()[at..at + len].to_vec()
    }
}

impl Flash for RamBoard {
    fn erase_sector(&self, addr: u32) -> Result<(), FlashError> {
        let at = (addr - FLASH_BASE) as usize;
        self.flash.borrow_mut()[at..at + 4096].fill(0xFF);
        Ok(())
    }

    fn program_page(&self, addr: u32, page: &[u8; PAGE_LEN]) -> Result<(), FlashError> {
        let at = (addr - FLASH_BASE) as usize;
        for (f, p) in self.flash.borrow_mut()[at..at + PAGE_LEN]
            .iter_mut()
            .zip(page)
        {
            *f &= p;
        }
        Ok(())
    }

    fn read(&self, addr: u32, buf: &mut [u8]) {
        let at = (addr - FLASH_BASE) as usize;
        buf.copy_from_slice(&self.flash.borrow()[at..at + buf.len()]);
    }
}

impl Platform for RamBoard {
    fn reload(&self) -> Result<(), ReloadError> {
        self.loads.set(self.loads.get() + 1);
        Ok(())
    }

    fn reboot(&self, to: BootTarget) {
        self.reboots.borrow_mut().push(to);
    }

    fn switch_bank(&self) -> Option<Bank> {
        Bank::new(0)
    }

    fn table_summary(&self) -> Option<TableSummary> {
        None
    }
}

fn identity(variant: Variant) -> Identity {
    Identity {
        mode: match variant {
            Variant::Application => Mode::Application,
            Variant::Dfu => Mode::Dfu,
        },
        fw_version: Version {
            major: 0,
            minor: 1,
            patch: 0,
        },
        board_rev: BoardRev::new("A").unwrap(),
    }
}

type Srv = Server<'static, RamBoard>;
type App = UsbDevice<FakeController, Srv, CdcAcm>;
type Dfu = UsbDevice<FakeController, Srv, NoFunction>;

/// The device under test, either build, on the model controller.
pub enum Device {
    App(Box<App>, &'static RamBoard),
    Dfu(Box<Dfu>, &'static RamBoard),
}

/// The serial number the tests give the device, and its unique ID.
pub const SERIAL: &str = "0123456789ABCDEF0011223344556677";
pub const UID: [u32; 4] = [0x0123_4567, 0x89AB_CDEF, 0x0011_2233, 0x4455_6677];

impl Device {
    pub fn new(variant: Variant) -> (Self, Wire) {
        let (ctl, hw) = FakeController::new();
        // The server borrows the board for as long as it serves.
        let board: &'static RamBoard = Box::leak(Box::new(RamBoard::new()));
        let server = Server::new(board, identity(variant));
        let serial = Serial::from_uid(UID);
        let dev = match variant {
            Variant::Application => Self::App(
                Box::new(UsbDevice::new(ctl, serial, server, CdcAcm::new())),
                board,
            ),
            Variant::Dfu => Self::Dfu(
                Box::new(UsbDevice::new(ctl, serial, server, NoFunction)),
                board,
            ),
        };
        (dev, Wire(hw))
    }

    /// What the bus could not do, if anything.
    pub fn alloc_fault(&self) -> Option<fastprom_usb::Fault> {
        match self {
            Self::App(d, _) => d.fault(),
            Self::Dfu(d, _) => d.fault(),
        }
    }

    pub fn configured(&self) -> bool {
        match self {
            Self::App(d, _) => d.configured(),
            Self::Dfu(d, _) => d.configured(),
        }
    }

    pub fn variant(&self) -> Variant {
        match self {
            Self::App(d, _) => d.variant(),
            Self::Dfu(d, _) => d.variant(),
        }
    }

    /// The application's serial port.
    pub fn cdc(&mut self) -> &mut CdcAcm {
        match self {
            Self::App(d, _) => d.function(),
            Self::Dfu(..) => panic!("the DFU stage has no CDC port"),
        }
    }

    /// The board behind the update server.
    pub fn board(&self) -> &'static RamBoard {
        match self {
            Self::App(_, b) | Self::Dfu(_, b) => b,
        }
    }

    /// One turn of the firmware's main loop.
    pub fn step(&mut self) {
        match self {
            Self::App(d, _) => d.poll(),
            Self::Dfu(d, _) => d.poll(),
        }
    }

    pub fn steps(&mut self, n: usize) {
        for _ in 0..n {
            self.step();
        }
    }
}

/// A SETUP packet.
pub fn setup(request_type: u8, request: u8, value: u16, index: u16, length: u16) -> [u8; 8] {
    let mut p = [0u8; 8];
    p[0] = request_type;
    p[1] = request;
    p[2..4].copy_from_slice(&value.to_le_bytes());
    p[4..6].copy_from_slice(&index.to_le_bytes());
    p[6..8].copy_from_slice(&length.to_le_bytes());
    p
}

/// The host's control transfers, done a transaction at a time with the
/// device stepped between them, as a host's schedule would.
pub struct Host<'d> {
    pub dev: &'d mut Device,
    pub wire: Wire,
}

/// A control transfer the device refused.
#[derive(Debug, PartialEq, Eq)]
pub struct Stalled;

impl Host<'_> {
    /// Retry an IN transaction until it is not NAKed.
    fn in_packet(&mut self, num: u8) -> Reply {
        for _ in 0..20 {
            self.dev.step();
            match self.wire.r#in(num) {
                Reply::Nak => continue,
                r => return r,
            }
        }
        Reply::Nak
    }

    /// Retry an OUT transaction until it is not NAKed.
    fn out_packet(&mut self, num: u8, data: &[u8]) -> Reply {
        for _ in 0..20 {
            self.dev.step();
            match self.wire.out(num, data) {
                Reply::Nak => continue,
                r => return r,
            }
        }
        Reply::Nak
    }

    pub fn control_in(&mut self, packet: [u8; 8]) -> Result<Vec<u8>, Stalled> {
        let length = u16::from_le_bytes([packet[6], packet[7]]) as usize;
        self.wire.setup(packet);
        let mut data = Vec::new();
        loop {
            match self.in_packet(0) {
                Reply::Data(p) => {
                    let short = p.len() < MAX_PACKET as usize;
                    data.extend_from_slice(&p);
                    if short || data.len() >= length {
                        break;
                    }
                }
                Reply::Stall => return Err(Stalled),
                Reply::Nak => panic!("control IN data stage timed out"),
            }
        }
        // Status stage: a zero-length OUT.
        match self.out_packet(0, &[]) {
            Reply::Data(_) => {}
            r => panic!("control IN status stage: {r:?}"),
        }
        self.dev.steps(2);
        Ok(data)
    }

    pub fn control_out(&mut self, packet: [u8; 8], data: &[u8]) -> Result<(), Stalled> {
        self.wire.setup(packet);
        for chunk in data.chunks(MAX_PACKET as usize) {
            match self.out_packet(0, chunk) {
                Reply::Data(_) => {}
                Reply::Stall => return Err(Stalled),
                Reply::Nak => panic!("control OUT data stage timed out"),
            }
        }
        // Status stage: a zero-length IN.
        match self.in_packet(0) {
            Reply::Data(p) if p.is_empty() => {}
            Reply::Stall => return Err(Stalled),
            r => panic!("control OUT status stage: {r:?}"),
        }
        self.dev.steps(2);
        Ok(())
    }

    /// Reset, address, configure: what a host does before using the
    /// device.
    pub fn enumerate(&mut self) {
        self.wire.bus_reset();
        self.dev.steps(2);
        self.control_out(setup(0x00, 0x05, 9, 0, 0), &[]).unwrap();
        self.control_out(setup(0x00, 0x09, 1, 0, 0), &[]).unwrap();
    }

    pub fn get_descriptor(&mut self, kind: u8, index: u8, length: u16) -> Result<Vec<u8>, Stalled> {
        let value = (u16::from(kind) << 8) | u16::from(index);
        let lang = if kind == 3 && index != 0 { 0x0409 } else { 0 };
        self.control_in(setup(0x80, 0x06, value, lang, length))
    }

    /// Bulk OUT, a packet at a time.
    pub fn bulk_out(&mut self, num: u8, data: &[u8]) -> Reply {
        if data.is_empty() {
            return self.out_packet(num, &[]);
        }
        let mut last = Reply::Nak;
        for chunk in data.chunks(MAX_PACKET as usize) {
            last = self.out_packet(num, chunk);
            if last != Reply::Data(Vec::new()) {
                return last;
            }
        }
        last
    }

    /// Bulk IN until a short packet or `length` bytes.
    pub fn bulk_in(&mut self, num: u8, length: usize) -> Result<Vec<u8>, Reply> {
        let mut data = Vec::new();
        loop {
            match self.in_packet(num) {
                Reply::Data(p) => {
                    let short = p.len() < MAX_PACKET as usize;
                    data.extend_from_slice(&p);
                    if short || data.len() >= length {
                        return Ok(data);
                    }
                }
                r => return Err(r),
            }
        }
    }
}
