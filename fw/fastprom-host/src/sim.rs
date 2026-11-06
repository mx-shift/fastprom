//! A simulated FastPROM board, for the page's demo mode and for tests: the
//! real device side, `fastprom-core`'s update protocol server, on a model
//! of the AG32's flash, behind a model of its USB endpoints.
//!
//! What it models:
//!
//! - **flash**: 256 KiB of NOR at `FLASH_BASE`, sector erase, page program
//!   that only clears bits, as `fastprom-core`'s `tests/update.rs` models
//!   it. [`SimDevice::cut_power_after`] loses power at a given flash
//!   operation: an erase cut leaves its sector half erased, a program cut
//!   its word half programmed, and the device is gone until
//!   [`SimDevice::power_on`].
//! - **boot**: power-on clears `mscratch`; the DFU stage's check
//!   (`header::boot_application`: the application header and the fabric
//!   manifest both sane) or a DFU request in `mscratch` picks the mode, as
//!   `fastprom-dfu`'s `boot.rs` does; the application loads the image the
//!   switch selects. `REBOOT` disconnects the device; it enumerates again
//!   on [`SimDevice::complete_reboot`], with `bcdDevice` `0x0001` in DFU
//!   mode and `0x0100` in the application.
//! - **USB**: the bulk endpoints a packet of 64 bytes at a time, as
//!   `fastprom-usb`'s update class moves them, and the interface's reset
//!   request. A request sent while a response is unread is NAKed (here, a
//!   timeout). Faults can be injected on the wire ([`UsbFault`]): a packet
//!   lost or damaged on its way in or out, which USB's own CRC and retries
//!   make rare, but which an abandoned transfer amounts to.
//!
//! - **UART recovery** (J3, BOOT0): with BOOT0 bridged, power-on starts a
//!   model of the AG32's mask-ROM serial loader instead of the DFU stage,
//!   with only what recovery asks of it (`DESIGN_NOTES.md`, "UART
//!   recovery", has why this much is inferred): the first sync measures the
//!   baud rate; `GET` (version 0x10), `GID`, reads and writes of SRAM with
//!   AGM's memory-mode byte, `GO`, and `READOUT UNPROTECT`; read protection
//!   on while the option bytes' RDP is not 0x5AA5, as erased option bytes
//!   leave it. `GO` to a valid recovery loader in SRAM starts
//!   `fastprom-core`'s loader server on this board's flash and option
//!   bytes; `GO` anywhere else and the chip goes quiet until power-on. The
//!   application has USB only when the option bytes point the fabric's
//!   configuration at the bitstream ([`fastprom_core::recovery::fabric_address`]);
//!   the DFU stage always has it (its own fabric). Faults can be injected
//!   on the wire: a byte corrupted on its way in, or one lost on its way
//!   out.
//!
//! What it does not: timing, the fabric (block RAM is a CRC), the CDC port.

use std::cell::{Cell, RefCell};
use std::collections::VecDeque;

use fastprom_core::crc32::crc32;
use fastprom_core::header::{
    boot_application, AppHeader, BoardRev, Commit, Manifest, Part, Version, DFU_REQUEST, PAGE_LEN,
};
use fastprom_core::memmap::{
    APP_BASE, DFU_BASE, FABRIC_BASE, FLASH_BASE, FLASH_SIZE, MANIFEST_BASE, OPTION_LEN, PAGE,
    RECOVERY_BASE, RECOVERY_ENTRY, SECTOR, SRAM_BASE, SRAM_SIZE, TABLE_BASE,
};
use fastprom_core::recovery::server::{self as loader, Server, Target};
use fastprom_core::recovery::{
    self, xor, LoaderHeader, ACK, CMD_GET, CMD_GID, CMD_GO, CMD_READ, CMD_READOUT_UNPROTECT,
    CMD_WRITE, DEVICE_ID, MMODE_SRAM, NACK, OPTION_BYTES, SYNC,
};
use fastprom_core::regmap::FABRIC_ID;
use fastprom_core::update::proto::PACKET;
use fastprom_core::update::{
    BootTarget, Flash, FlashError, Identity, Mode, Platform, ReloadError, Server as UpdateServer,
    Status,
};
use fastprom_core::{Bank, ImageTable, TableSummary};

use fastprom_usb::{desc, Serial, Variant};

use crate::rom::Geometry;
use crate::serial::{SerialError, SerialPort};
use crate::transport::{Transport, TransportError};

// What the device enumerates as is the firmware's, from fastprom-usb: the
// simulator and the page take every number from there.
pub use fastprom_usb::ids::{PID, VID};
/// `bcdDevice` of the application and of the DFU stage.
pub const BCD_APPLICATION: u16 = Variant::Application.bcd_device();
pub const BCD_DFU: u16 = Variant::Dfu.bcd_device();

/// The simulated chip's unique ID, for its serial number.
const UID: [u32; 4] = [0x4147_3332, 0x5349_4D00, 0x0000_0001, 0x2026_1105];

// ---- the board ------------------------------------------------------------

/// The flash and the board's other state, shared by the update core's
/// borrows and the simulator.
pub struct Board {
    flash: RefCell<Vec<u8>>,
    /// Flash operations (sector erases, word programs) left before power is
    /// lost; `None` for none.
    budget: Cell<Option<u64>>,
    powered: Cell<bool>,
    ops: Cell<u64>,
    reboot: Cell<Option<BootTarget>>,
    /// The switch as it reads now, and as it read at boot.
    switch: Cell<u8>,
    bank: Cell<Option<Bank>>,
    /// CRC of what block RAM holds, and READY.
    served: Cell<Option<u32>>,
    loads: Cell<u32>,
    /// The 128 option bytes.
    options: RefCell<[u8; OPTION_LEN as usize]>,
    /// SRAM, for what the mask ROM is asked to write there.
    sram: RefCell<Vec<u8>>,
    /// The recovery loader asked for a reset.
    reset: Cell<bool>,
    device_id: Cell<u32>,
}

impl Board {
    fn new() -> Self {
        Self {
            flash: RefCell::new(vec![0xFF; FLASH_SIZE as usize]),
            budget: Cell::new(None),
            powered: Cell::new(false),
            ops: Cell::new(0),
            reboot: Cell::new(None),
            switch: Cell::new(0),
            bank: Cell::new(None),
            served: Cell::new(None),
            loads: Cell::new(0),
            options: RefCell::new([0xFF; OPTION_LEN as usize]),
            sram: RefCell::new(vec![0; SRAM_SIZE as usize]),
            reset: Cell::new(false),
            device_id: Cell::new(DEVICE_ID),
        }
    }

    /// The option bytes as they are now.
    pub fn options(&self) -> [u8; OPTION_LEN as usize] {
        *self.options.borrow()
    }

    /// Read protection: on unless RDP reads 0x5AA5 (flash.h `RDP_WORD`),
    /// so on with the option bytes erased.
    fn protected(&self) -> bool {
        self.options.borrow()[..2] != [0xA5, 0x5A]
    }

    /// The fabric configures at power-on from where the option bytes point,
    /// if they point at a bitstream.
    fn fabric_configures(&self) -> bool {
        recovery::fabric_address(&self.options()) == Some(FABRIC_BASE)
            && self.word(FABRIC_BASE) != u32::MAX
    }

    fn sram_range(addr: u32, len: usize) -> Option<core::ops::Range<usize>> {
        let at = addr.checked_sub(SRAM_BASE)? as usize;
        (at + len <= SRAM_SIZE as usize).then_some(at..at + len)
    }

    fn offset(addr: u32) -> usize {
        (addr - FLASH_BASE) as usize
    }

    /// Bytes of flash.
    pub fn bytes(&self, addr: u32, len: u32) -> Vec<u8> {
        let at = Self::offset(addr);
        self.flash.borrow()[at..at + len as usize].to_vec()
    }

    fn word(&self, addr: u32) -> u32 {
        let b = self.bytes(addr, 4);
        u32::from_le_bytes([b[0], b[1], b[2], b[3]])
    }

    /// Put bytes in flash as the UART loader would: no policy, no cost.
    fn provision(&self, addr: u32, data: &[u8]) {
        let at = Self::offset(addr);
        self.flash.borrow_mut()[at..at + data.len()].copy_from_slice(data);
    }

    /// One flash operation: `Ok(true)` whole, `Ok(false)` cut in the middle
    /// (power is lost during it), `Err` with power already gone.
    fn spend(&self) -> Result<bool, FlashError> {
        if !self.powered.get() {
            return Err(FlashError::Failed);
        }
        self.ops.set(self.ops.get() + 1);
        match self.budget.get() {
            None => Ok(true),
            Some(0) => {
                self.powered.set(false);
                Err(FlashError::Failed)
            }
            Some(1) => {
                self.budget.set(None);
                self.powered.set(false);
                Ok(false)
            }
            Some(n) => {
                self.budget.set(Some(n - 1));
                Ok(true)
            }
        }
    }

    /// Serve the switch's image from the table, as the application does at
    /// boot and on `LOAD`.
    fn serve(&self) -> Result<(), Status> {
        self.served.set(None);
        let flash = self.flash.borrow();
        let table = ImageTable::parse(&flash[Self::offset(TABLE_BASE)..])
            .map_err(|_| Status::InvalidData)?;
        let bank = self.bank.get().ok_or(Status::PreconditionNotMet)?;
        let image = table.image(bank).ok_or(Status::InvalidData)?;
        if crc32(image.words()) != image.crc32() {
            return Err(Status::InvalidData);
        }
        self.served.set(Some(image.crc32()));
        Ok(())
    }
}

impl Flash for Board {
    fn erase_sector(&self, addr: u32) -> Result<(), FlashError> {
        let whole = self.spend()?;
        let at = Self::offset(addr);
        let mut f = self.flash.borrow_mut();
        for (i, b) in f[at..at + SECTOR as usize].iter_mut().enumerate() {
            if whole || i % 2 == 0 {
                *b = 0xFF;
            }
        }
        if whole {
            Ok(())
        } else {
            Err(FlashError::Failed)
        }
    }

    fn program_page(&self, addr: u32, page: &[u8; PAGE_LEN]) -> Result<(), FlashError> {
        let at = Self::offset(addr);
        for (w, chunk) in page.chunks(4).enumerate() {
            if chunk == [0xFF; 4] {
                continue;
            }
            let whole = self.spend()?;
            let mut f = self.flash.borrow_mut();
            for (i, &b) in chunk.iter().enumerate() {
                let b = if whole { b } else { b | 0xF0 };
                f[at + w * 4 + i] &= b;
            }
            if !whole {
                return Err(FlashError::Failed);
            }
        }
        Ok(())
    }

    fn read(&self, addr: u32, buf: &mut [u8]) {
        let at = Self::offset(addr);
        buf.copy_from_slice(&self.flash.borrow()[at..at + buf.len()]);
    }
}

impl Platform for Board {
    fn reload(&self) -> Result<(), ReloadError> {
        self.loads.set(self.loads.get() + 1);
        self.serve()
    }

    fn reboot(&self, to: BootTarget) {
        self.reboot.set(Some(to));
    }

    fn switch_bank(&self) -> Option<Bank> {
        self.bank.get()
    }

    fn table_summary(&self) -> Option<TableSummary> {
        let flash = self.flash.borrow();
        ImageTable::parse(&flash[Self::offset(TABLE_BASE)..])
            .ok()
            .map(|t| t.summary())
    }
}

impl Target for Board {
    fn device_id(&self) -> u32 {
        self.device_id.get()
    }

    fn read_options(&self, buf: &mut [u8; OPTION_LEN as usize]) {
        *buf = self.options();
    }

    fn erase_options(&self) -> Result<(), FlashError> {
        let whole = self.spend()?;
        let mut o = self.options.borrow_mut();
        for (i, b) in o.iter_mut().enumerate() {
            if whole || i % 2 == 0 {
                *b = 0xFF;
            }
        }
        if whole {
            Ok(())
        } else {
            Err(FlashError::Failed)
        }
    }

    fn program_options(&self, image: &[u8; OPTION_LEN as usize]) -> Result<(), FlashError> {
        for (w, chunk) in image.chunks(4).enumerate() {
            if chunk == [0xFF; 4] {
                continue;
            }
            let whole = self.spend()?;
            let mut o = self.options.borrow_mut();
            for (i, &b) in chunk.iter().enumerate() {
                o[w * 4 + i] &= if whole { b } else { b | 0xF0 };
            }
            if !whole {
                return Err(FlashError::Failed);
            }
        }
        Ok(())
    }

    fn reset(&self) {
        self.reset.set(true);
    }
}

// ---- the mask ROM -----------------------------------------------------------

/// The mask ROM's version in the model: the highest AGM's flasher treats as
/// the mask ROM (`fastprom_core::recovery::MASK_ROM_MAX_VERSION`).
pub const MASK_ROM_VERSION: u8 = recovery::MASK_ROM_MAX_VERSION;
/// The commands the model's `GET` lists: AN3155's `GET`, `GET VERSION`,
/// `GID`, `READ`, `GO`, `WRITE`, `READOUT UNPROTECT`.
pub const MASK_ROM_COMMANDS: [u8; 7] = [0x00, 0x01, 0x02, 0x11, 0x21, 0x31, 0x92];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Rom {
    /// After reset: waiting for the sync byte to measure the baud rate on.
    Unsynced,
    Idle,
    Complement(u8),
    Address {
        cmd: u8,
        got: usize,
    },
    /// AGM's memory-mode byte and its complement.
    Mode {
        cmd: u8,
        mode: Option<u8>,
    },
    ReadCount {
        count: Option<u8>,
    },
    WriteCount,
    WriteData {
        len: usize,
        got: usize,
    },
}

/// What a byte to the mask ROM led to, beyond its replies.
enum RomEvent {
    None,
    /// GO: run what is at this address.
    Go(u32),
    /// READOUT UNPROTECT done: the chip resets.
    Reset,
}

/// The mask ROM model: what recovery relies on, and no more.
struct MaskRom {
    state: Rom,
    addr: u32,
    sum: u8,
    buf: Vec<u8>,
}

impl MaskRom {
    fn new() -> Self {
        Self {
            state: Rom::Unsynced,
            addr: 0,
            sum: 0,
            buf: Vec::new(),
        }
    }

    fn feed(&mut self, b: u8, board: &Board, out: &mut Vec<u8>) -> RomEvent {
        let mut event = RomEvent::None;
        self.state = match self.state {
            Rom::Unsynced if b == SYNC => {
                out.push(ACK);
                Rom::Idle
            }
            Rom::Unsynced => Rom::Unsynced,
            Rom::Idle => Rom::Complement(b),
            Rom::Complement(c) if b != !c => {
                out.push(NACK);
                Rom::Idle
            }
            Rom::Complement(c) => match c {
                CMD_GET => {
                    out.extend([ACK, MASK_ROM_COMMANDS.len() as u8, MASK_ROM_VERSION]);
                    out.extend(MASK_ROM_COMMANDS);
                    out.push(ACK);
                    Rom::Idle
                }
                CMD_GID => {
                    out.extend([ACK, 3]);
                    out.extend(board.device_id.get().to_be_bytes());
                    out.push(ACK);
                    Rom::Idle
                }
                CMD_READ | CMD_WRITE | CMD_GO if board.protected() => {
                    out.push(NACK);
                    Rom::Idle
                }
                CMD_READ | CMD_WRITE | CMD_GO => {
                    out.push(ACK);
                    self.buf.clear();
                    Rom::Address { cmd: c, got: 0 }
                }
                CMD_READOUT_UNPROTECT => {
                    out.push(ACK);
                    board.flash.borrow_mut().fill(0xFF);
                    let mut o = [0xFF; OPTION_LEN as usize];
                    o[..2].copy_from_slice(&[0xA5, 0x5A]);
                    *board.options.borrow_mut() = o;
                    out.push(ACK);
                    event = RomEvent::Reset;
                    Rom::Unsynced
                }
                _ => {
                    out.push(NACK);
                    Rom::Idle
                }
            },
            Rom::Address { cmd, got } if got < 4 => {
                self.buf.push(b);
                Rom::Address { cmd, got: got + 1 }
            }
            Rom::Address { cmd, .. } => {
                let a = [self.buf[0], self.buf[1], self.buf[2], self.buf[3]];
                let addr = u32::from_be_bytes(a);
                if b != xor(&a) {
                    out.push(NACK);
                    Rom::Idle
                } else if cmd == CMD_GO {
                    out.push(ACK);
                    event = RomEvent::Go(addr);
                    Rom::Idle
                } else if Board::sram_range(addr, 1).is_none() {
                    // The model writes and reads SRAM only: all recovery asks.
                    out.push(NACK);
                    Rom::Idle
                } else {
                    self.addr = addr;
                    out.push(ACK);
                    Rom::Mode { cmd, mode: None }
                }
            }
            Rom::Mode { cmd, mode: None } => Rom::Mode { cmd, mode: Some(b) },
            Rom::Mode { cmd, mode: Some(m) } => {
                if m == MMODE_SRAM && b == !m {
                    out.push(ACK);
                    if cmd == CMD_READ {
                        Rom::ReadCount { count: None }
                    } else {
                        Rom::WriteCount
                    }
                } else {
                    out.push(NACK);
                    Rom::Idle
                }
            }
            Rom::ReadCount { count: None } => Rom::ReadCount { count: Some(b) },
            Rom::ReadCount { count: Some(n) } => {
                let len = usize::from(n) + 1;
                match Board::sram_range(self.addr, len) {
                    Some(r) if b == !n => {
                        out.push(ACK);
                        out.extend_from_slice(&board.sram.borrow()[r]);
                    }
                    _ => out.push(NACK),
                }
                Rom::Idle
            }
            Rom::WriteCount => {
                self.sum = b;
                self.buf.clear();
                Rom::WriteData {
                    len: usize::from(b) + 1,
                    got: 0,
                }
            }
            Rom::WriteData { len, got } if got < len => {
                self.buf.push(b);
                self.sum ^= b;
                Rom::WriteData { len, got: got + 1 }
            }
            Rom::WriteData { len, .. } => {
                match Board::sram_range(self.addr, len) {
                    Some(r) if b == self.sum => {
                        board.sram.borrow_mut()[r].copy_from_slice(&self.buf);
                        out.push(ACK);
                    }
                    _ => out.push(NACK),
                }
                Rom::Idle
            }
        };
        event
    }
}

/// Who answers on UART0.
enum Uart {
    /// Nobody recovery could talk to: no power, the firmware, or a GO into
    /// something that is not the recovery loader.
    Silent,
    MaskRom(MaskRom),
    Loader(Box<Server>),
}

/// A fault on the wire, `after` bytes from when it was set (1 is the next).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WireFault {
    /// A byte from the host arrives with a bit flipped.
    Corrupt { after: u32 },
    /// A byte from the board never arrives.
    Drop { after: u32 },
}

// ---- the wire ------------------------------------------------------------------

/// A fault on the USB wire, `after` packets from when it was set (1 is the
/// next) in its direction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UsbFault {
    /// A packet from the host never reaches the device.
    DropOut { after: u32 },
    /// A packet from the host arrives with a bit flipped.
    CorruptOut { after: u32 },
    /// A packet from the device never reaches the host.
    DropIn { after: u32 },
    /// A packet from the device arrives with a bit flipped.
    CorruptIn { after: u32 },
}

/// What the fault, if any, does to the next packet in one direction.
enum Fate {
    Deliver,
    Drop,
    Corrupt,
}

// ---- the device -------------------------------------------------------------

/// How a simulated board is made.
#[derive(Debug, Clone)]
pub struct SimConfig {
    /// The board revision the DFU stage was built for.
    pub board_rev: BoardRev,
    /// The DFU stage's version.
    pub dfu_version: Version,
    /// The part, and its organisation, for the factory fabric and table.
    pub part: Part,
    pub geometry: Geometry,
}

/// Why a simulated transfer did not happen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UsbError {
    /// Not enumerated: rebooting, or no power.
    Gone,
    /// The device had nothing to send, or NAKed what the host sent (a real
    /// host would wait for ever).
    Timeout,
}

impl UsbError {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Gone => "gone",
            Self::Timeout => "timeout",
        }
    }
}

/// What the simulator reports about itself.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct SimStatus {
    pub powered: bool,
    pub connected: bool,
    /// "application" or "dfu", while powered.
    pub mode: Option<&'static str>,
    pub switch: u8,
    /// CRC of the image the socket serves; `None` when it is released.
    pub served: Option<u32>,
    pub flash_ops: u64,
    pub cut_armed: Option<u64>,
    pub enumerations: u32,
    /// BOOT0 bridged to 3.3 V: power-on starts the mask ROM's loader.
    pub boot0: bool,
    /// Who answers on the UART: "mask ROM" or "recovery loader".
    pub uart: Option<&'static str>,
}

/// The simulated board on its USB cable.
pub struct SimDevice {
    board: &'static Board,
    config: SimConfig,
    server: Option<UpdateServer<'static, Board>>,
    usb_fault: Option<UsbFault>,
    mode: Mode,
    mscratch: u32,
    connected: bool,
    enumerations: u32,
    boot0: bool,
    uart: Uart,
    /// Bytes the board has sent on UART0 that the host has not read.
    uart_out: VecDeque<u8>,
    /// Milliseconds since the host last sent a byte.
    uart_quiet: u32,
    fault: Option<WireFault>,
}

/// A synthetic application binary as `make fw-build` writes one: its
/// header (committed), then `code_len` bytes of code.
pub fn app_file(version: Version, board_rev: BoardRev, code_len: usize, seed: u32) -> Vec<u8> {
    let code = pattern(code_len, seed);
    let header = AppHeader {
        image_len: code.len() as u32,
        image_crc: crc32(&code),
        fw_version: version,
        board_rev,
        fabric_id: FABRIC_ID,
    };
    let mut file = header.encode(Commit::Committed).to_vec();
    file.extend_from_slice(&code);
    file
}

/// A synthetic bitstream and its manifest page (committed).
pub fn fabric_files(part: Part, board_rev: BoardRev, len: usize, seed: u32) -> (Vec<u8>, Vec<u8>) {
    let fabric = pattern(len, seed);
    let m = Manifest {
        fabric_len: fabric.len() as u32,
        fabric_crc: crc32(&fabric),
        part,
        board_rev,
        fabric_id: FABRIC_ID,
    };
    (fabric, m.encode(Commit::Committed).to_vec())
}

/// Deterministic filler.
pub fn pattern(len: usize, seed: u32) -> Vec<u8> {
    let mut x = seed.wrapping_mul(2_654_435_761).wrapping_add(1);
    (0..len)
        .map(|_| {
            x ^= x << 13;
            x ^= x >> 17;
            x ^= x << 5;
            (x >> 8) as u8
        })
        .collect()
}

impl SimDevice {
    /// A board with blank flash, unpowered.
    pub fn blank(config: SimConfig) -> Self {
        // The update core borrows the board for as long as it serves; the
        // simulator lives as long as the page (or the test), so the board
        // is given that lifetime outright. One board per simulator.
        let board: &'static Board = Box::leak(Box::new(Board::new()));
        Self {
            board,
            config,
            server: None,
            usb_fault: None,
            mode: Mode::Dfu,
            mscratch: 0,
            connected: false,
            enumerations: 0,
            boot0: false,
            uart: Uart::Silent,
            uart_out: VecDeque::new(),
            uart_quiet: 0,
            fault: None,
        }
    }

    /// A board as provisioned over the UART: an application at version
    /// `app_version`, a fabric and a manifest for the part, and an image
    /// table of a test pattern per switch position. Powered on.
    pub fn factory(config: SimConfig, app_version: Version) -> Self {
        let s = Self::blank(config);
        let rev = s.config.board_rev;
        // The DFU stage: its code is not run (the simulator is the code),
        // but a chip with an erased reset vector runs nothing.
        s.board.provision(DFU_BASE, &pattern(20_000, 3));
        s.board
            .provision(APP_BASE, &app_file(app_version, rev, 6000, 1));
        let (fabric, manifest) = fabric_files(s.config.part, rev, 4000, 2);
        s.board.provision(FABRIC_BASE, &fabric);
        s.board.provision(MANIFEST_BASE, &manifest);
        *s.board.options.borrow_mut() = OPTION_BYTES;
        let g = s.config.geometry;
        let images: Vec<(String, Vec<u8>)> = (0..4u32)
            .map(|i| {
                let words = pattern(g.depth as usize, 10 + i)
                    .into_iter()
                    .map(|w| w & Geometry::mask(g))
                    .collect();
                (format!("factory-{i}.bin"), words)
            })
            .collect();
        let refs: Vec<(&str, &[u8])> = images
            .iter()
            .map(|(n, w)| (n.as_str(), w.as_slice()))
            .collect();
        if let Ok(table) = crate::table::build(s.config.part.as_str(), g, &refs) {
            s.board.provision(TABLE_BASE, &table);
        }
        let mut s = s;
        s.power_on();
        s
    }

    /// The board, for inspection.
    pub fn board(&self) -> &Board {
        self.board
    }

    /// Put bytes in flash as the UART loader would.
    pub fn provision(&self, addr: u32, data: &[u8]) {
        self.board.provision(addr, data);
    }

    /// Set the image-select switch; it is read at the next boot.
    pub fn set_switch(&self, position: u8) {
        self.board.switch.set(position & 3);
    }

    /// Lose power at the `ops`-th flash operation from now (1 is the next).
    pub fn cut_power_after(&self, ops: u64) {
        self.board.budget.set(Some(ops));
    }

    /// Power off now.
    pub fn power_off(&mut self) {
        self.board.powered.set(false);
        self.drop_bus();
        self.uart = Uart::Silent;
        self.uart_out.clear();
    }

    /// Power on: `mscratch` cleared, then boot.
    pub fn power_on(&mut self) {
        self.board.powered.set(true);
        self.board.budget.set(None);
        self.mscratch = 0;
        self.reset_chip();
    }

    /// The chip comes out of reset: the mask ROM's loader with BOOT0
    /// bridged, else the DFU stage's boot path.
    fn reset_chip(&mut self) {
        self.uart_out.clear();
        self.board.reset.set(false);
        if self.boot0 {
            self.drop_bus();
            self.board.served.set(None);
            self.uart = Uart::MaskRom(MaskRom::new());
        } else {
            self.uart = Uart::Silent;
            self.boot();
        }
    }

    /// Bridge (or unbridge) BOOT0 to 3.3 V: read at the next reset.
    pub fn set_boot0(&mut self, high: bool) {
        self.boot0 = high;
    }

    /// The chip's device ID, as `GID` reports it (a FastPROM board's is
    /// the AG32VF303's).
    pub fn set_device_id(&self, id: u32) {
        self.board.device_id.set(id);
    }

    /// A fault on the UART wire (one, until it fires).
    pub fn inject_fault(&mut self, fault: WireFault) {
        self.fault = Some(fault);
    }

    /// The host sends `data` on the UART.
    pub fn serial_write(&mut self, data: &[u8]) {
        self.uart_quiet = 0;
        for &b in data {
            if !self.board.powered.get() {
                return;
            }
            let b = match self.fault {
                Some(WireFault::Corrupt { after: 1 }) => {
                    self.fault = None;
                    b ^ 0x10
                }
                Some(WireFault::Corrupt { after }) => {
                    self.fault = Some(WireFault::Corrupt { after: after - 1 });
                    b
                }
                _ => b,
            };
            let mut out = Vec::new();
            match &mut self.uart {
                Uart::Silent => {}
                Uart::MaskRom(rom) => match rom.feed(b, self.board, &mut out) {
                    RomEvent::None => {}
                    RomEvent::Reset => self.board.reset.set(true),
                    RomEvent::Go(addr) => {
                        self.uart = if addr == RECOVERY_ENTRY && self.loader_in_sram() {
                            Uart::Loader(Box::new(Server::new()))
                        } else {
                            Uart::Silent
                        };
                    }
                },
                Uart::Loader(server) => {
                    server.feed(b, self.board, &mut |r: &[u8]| out.extend_from_slice(r));
                }
            }
            self.send(&out);
            self.settle();
            if !self.board.powered.get() {
                // Power lost part way (a cut armed in the flash): the chip
                // is gone until power-on.
                self.uart = Uart::Silent;
                return;
            }
            if self.board.reset.take() {
                // The reply went out first; then the chip resets.
                let pending = std::mem::take(&mut self.uart_out);
                self.reset_chip();
                self.uart_out = pending;
            }
        }
    }

    /// Bytes from the board onto the wire, a dropped one lost.
    fn send(&mut self, out: &[u8]) {
        for &b in out {
            match self.fault {
                Some(WireFault::Drop { after: 1 }) => self.fault = None,
                Some(WireFault::Drop { after }) => {
                    self.fault = Some(WireFault::Drop { after: after - 1 });
                    self.uart_out.push_back(b);
                }
                _ => self.uart_out.push_back(b),
            }
        }
    }

    /// Whether SRAM holds a recovery loader whose header checks.
    fn loader_in_sram(&self) -> bool {
        let sram = self.board.sram.borrow();
        let at = (RECOVERY_BASE - SRAM_BASE) as usize;
        let head = &sram[at..at + 12];
        let len = u32::from_le_bytes([head[8], head[9], head[10], head[11]]) as usize;
        let end = (at + 32 + len).min(sram.len());
        LoaderHeader::check(&sram[at..end]).is_ok()
    }

    /// Up to `max` bytes the board has sent.
    pub fn serial_read(&mut self, max: usize) -> Vec<u8> {
        let n = max.min(self.uart_out.len());
        self.uart_out.drain(..n).collect()
    }

    /// Time passes on the wire with nothing from the host: the recovery
    /// loader forgets a command left part way after its idle timeout.
    pub fn serial_idle(&mut self, ms: u32) {
        self.uart_quiet = self.uart_quiet.saturating_add(ms);
        if let Uart::Loader(server) = &mut self.uart {
            if server.busy() && self.uart_quiet >= loader::IDLE_MS {
                server.idle();
            }
        }
    }

    fn drop_bus(&mut self) {
        self.connected = false;
        self.server = None;
    }

    /// The DFU stage's power-up path, then the application's or DFU mode's.
    fn boot(&mut self) {
        self.drop_bus();
        let b = self.board;
        b.reboot.set(None);
        b.served.set(None);
        let dfu_request = std::mem::take(&mut self.mscratch) == DFU_REQUEST;
        let three = |at: u32| [b.word(at), b.word(at + 4), b.word(at + 8)];
        let sane = boot_application(three(APP_BASE), three(MANIFEST_BASE));
        let rev = self.config.board_rev;
        if b.word(DFU_BASE) == u32::MAX {
            // No DFU stage at the reset vector: the core runs erased flash,
            // and nothing answers anywhere (a blank chip, until recovery).
            self.mode = Mode::Dfu;
            b.bank.set(None);
            return;
        }
        if !dfu_request && sane && !b.fabric_configures() {
            // The application runs, but USB is the fabric's to enable, and
            // there is no fabric: nothing enumerates.
            self.mode = Mode::Application;
            b.bank.set(Bank::new(b.switch.get()));
            return;
        }
        let id = if !dfu_request && sane {
            self.mode = Mode::Application;
            b.bank.set(Bank::new(b.switch.get()));
            // The image is loaded before USB starts.
            let _ = b.serve();
            let page: [u8; PAGE_LEN] = b.bytes(APP_BASE, PAGE).try_into().unwrap_or([0; PAGE_LEN]);
            let (fw_version, board_rev) = match AppHeader::decode(&page) {
                Ok((_, h)) => (h.fw_version, h.board_rev),
                Err(_) => (Version::default(), rev),
            };
            Identity {
                mode: Mode::Application,
                fw_version,
                board_rev,
            }
        } else {
            self.mode = Mode::Dfu;
            b.bank.set(None);
            Identity {
                mode: Mode::Dfu,
                fw_version: self.config.dfu_version,
                board_rev: rev,
            }
        };
        self.server = Some(UpdateServer::new(b, id));
        self.connected = true;
        self.enumerations += 1;
    }

    /// A `REBOOT` was answered: the device is gone until
    /// [`SimDevice::complete_reboot`].
    pub fn reboot_pending(&self) -> bool {
        !self.connected
            && self.board.powered.get()
            && self.server.is_none()
            && self.board.reboot.get().is_some()
    }

    /// Enumerate again after a reboot.
    pub fn complete_reboot(&mut self) {
        if let Some(to) = self.board.reboot.take() {
            self.mscratch = match to {
                BootTarget::Dfu => DFU_REQUEST,
                BootTarget::Application => 0,
            };
            if self.board.powered.get() {
                self.boot();
            }
        }
    }

    pub const fn connected(&self) -> bool {
        self.connected
    }

    /// Which firmware enumerates now.
    pub const fn variant(&self) -> Variant {
        match self.mode {
            Mode::Application => Variant::Application,
            Mode::Dfu => Variant::Dfu,
        }
    }

    /// `bcdDevice` as the device enumerates now.
    pub const fn bcd_device(&self) -> u16 {
        self.variant().bcd_device()
    }

    /// The device descriptor, as the firmware sends it.
    pub const fn device_descriptor(&self) -> &'static [u8; 18] {
        match self.mode {
            Mode::Application => &desc::DEVICE_APP,
            Mode::Dfu => &desc::DEVICE_DFU,
        }
    }

    /// The configuration descriptor, as the firmware sends it: the page's
    /// fake WebUSB builds its interfaces and endpoints from these bytes.
    pub const fn configuration_descriptor(&self) -> &'static [u8] {
        match self.mode {
            Mode::Application => &desc::CONFIG_APP,
            Mode::Dfu => &desc::CONFIG_DFU,
        }
    }

    /// The serial number, as the firmware makes it from the chip's unique
    /// ID: the same in both modes.
    pub fn serial(&self) -> String {
        String::from_utf8_lossy(Serial::from_uid(UID).digits()).into_owned()
    }

    pub fn status(&self) -> SimStatus {
        let b = self.board;
        SimStatus {
            powered: b.powered.get(),
            connected: self.connected,
            mode: b.powered.get().then_some(match self.mode {
                Mode::Application => "application",
                Mode::Dfu => "dfu",
            }),
            switch: b.switch.get(),
            served: b.served.get(),
            flash_ops: b.ops.get(),
            cut_armed: b.budget.get(),
            enumerations: self.enumerations,
            boot0: self.boot0,
            uart: match self.uart {
                Uart::Silent => None,
                Uart::MaskRom(_) => Some("mask ROM"),
                Uart::Loader(_) => Some("recovery loader"),
            },
        }
    }

    /// After each step: power lost takes the device off the bus.
    fn settle(&mut self) {
        if !self.board.powered.get() {
            self.drop_bus();
        }
    }

    /// The server, while the device is on the bus.
    fn live(&mut self) -> Result<&mut UpdateServer<'static, Board>, UsbError> {
        match (self.connected, self.server.as_mut()) {
            (true, Some(s)) => Ok(s),
            _ => Err(UsbError::Gone),
        }
    }

    /// A fault on the USB wire (one, until it fires).
    pub fn inject_usb_fault(&mut self, fault: UsbFault) {
        self.usb_fault = Some(fault);
    }

    /// Count a packet in direction `out` against the fault armed.
    fn fate(&mut self, out: bool) -> Fate {
        let (after, fate) = match self.usb_fault {
            Some(UsbFault::DropOut { after }) if out => (after, Fate::Drop),
            Some(UsbFault::CorruptOut { after }) if out => (after, Fate::Corrupt),
            Some(UsbFault::DropIn { after }) if !out => (after, Fate::Drop),
            Some(UsbFault::CorruptIn { after }) if !out => (after, Fate::Corrupt),
            _ => return Fate::Deliver,
        };
        if after <= 1 {
            self.usb_fault = None;
            fate
        } else {
            self.usb_fault = self.usb_fault.map(|f| match f {
                UsbFault::DropOut { .. } => UsbFault::DropOut { after: after - 1 },
                UsbFault::CorruptOut { .. } => UsbFault::CorruptOut { after: after - 1 },
                UsbFault::DropIn { .. } => UsbFault::DropIn { after: after - 1 },
                UsbFault::CorruptIn { .. } => UsbFault::CorruptIn { after: after - 1 },
            });
            Fate::Deliver
        }
    }

    /// The host sends `data` on the bulk OUT endpoint: 64-byte packets, the
    /// last as short as it comes out (empty `data`: one zero-length
    /// packet). The device runs each request as its last packet arrives.
    ///
    /// # Errors
    ///
    /// `Gone` off the bus; `Timeout` when the device has a response the host
    /// has not read and so NAKs.
    pub fn bulk_out(&mut self, data: &[u8]) -> Result<(), UsbError> {
        let packets: Vec<&[u8]> = if data.is_empty() {
            vec![&[]]
        } else {
            data.chunks(PACKET).collect()
        };
        for p in packets {
            if !self.live()?.ready() {
                return Err(UsbError::Timeout);
            }
            let mut packet = p.to_vec();
            match self.fate(true) {
                Fate::Deliver => {}
                Fate::Drop => continue,
                Fate::Corrupt => {
                    if let Some(b) = packet.get_mut(p.len() / 2) {
                        *b ^= 0x08;
                    }
                }
            }
            self.live()?.receive(&packet);
            self.settle();
            if !self.connected {
                return Err(UsbError::Gone);
            }
        }
        Ok(())
    }

    /// The host reads up to `len` bytes from the bulk IN endpoint: the
    /// transfer ends at `len` or at a short packet.
    ///
    /// # Errors
    ///
    /// `Gone`, or `Timeout` when the device has nothing to send.
    pub fn bulk_in(&mut self, len: usize) -> Result<Vec<u8>, UsbError> {
        let mut got = Vec::new();
        loop {
            let s = self.live()?;
            let mut packet = [0u8; PACKET];
            let Some(n) = s.next_packet(&mut packet) else {
                // Nothing (more) to send: a real host would wait.
                return if got.is_empty() {
                    Err(UsbError::Timeout)
                } else {
                    Ok(got)
                };
            };
            s.packet_armed();
            s.packet_sent();
            let mut p = packet.get(..n).unwrap_or(&[]).to_vec();
            let delivered = match self.fate(false) {
                Fate::Deliver => true,
                // The host never sees it, and waits for the next.
                Fate::Drop => false,
                Fate::Corrupt => {
                    if let Some(b) = p.get_mut(n / 2) {
                        *b ^= 0x08;
                    }
                    true
                }
            };
            if delivered {
                got.extend_from_slice(&p);
            }
            if self.board.reboot.get().is_some() {
                // The answer has gone: the device resets.
                self.drop_bus();
                return Ok(got);
            }
            if delivered && (p.len() < PACKET || got.len() >= len) {
                return Ok(got);
            }
        }
    }

    /// The update interface's reset request.
    ///
    /// # Errors
    ///
    /// `Gone` off the bus.
    pub fn reset_interface(&mut self) -> Result<(), UsbError> {
        self.live()?.reset();
        Ok(())
    }

    /// Whether the device is idle: on the bus, nothing half received,
    /// nothing unsent. For tests.
    pub fn idle(&mut self) -> bool {
        self.live().is_ok_and(|s| !s.responding())
    }
}

// ---- in-process transport -------------------------------------------------------

/// [`Transport`] straight onto a [`SimDevice`], for tests: every future is
/// ready when first polled.
pub struct SimTransport<'a>(pub &'a mut SimDevice);

fn map(e: UsbError) -> TransportError {
    match e {
        UsbError::Gone => TransportError::Gone,
        other => TransportError::Other(other.as_str().into()),
    }
}

impl Transport for SimTransport<'_> {
    async fn bulk_out(&mut self, data: &[u8]) -> Result<(), TransportError> {
        self.0.bulk_out(data).map_err(map)
    }
    async fn bulk_in(&mut self, len: usize) -> Result<Vec<u8>, TransportError> {
        self.0.bulk_in(len).map_err(map)
    }
    async fn reset(&mut self) -> Result<(), TransportError> {
        self.0.reset_interface().map_err(map)
    }
}

/// [`SerialPort`] straight onto a [`SimDevice`]'s UART, for tests: every
/// future is ready when first polled, and a read's timeout is time the
/// simulator lets pass.
pub struct SimSerial<'a>(pub &'a mut SimDevice);

impl SerialPort for SimSerial<'_> {
    async fn write(&mut self, data: &[u8]) -> Result<(), SerialError> {
        self.0.serial_write(data);
        Ok(())
    }
    async fn read(&mut self, len: usize, timeout_ms: u32) -> Result<Vec<u8>, SerialError> {
        let mut got = self.0.serial_read(len);
        if got.len() < len {
            self.0.serial_idle(timeout_ms);
            got.extend(self.0.serial_read(len - got.len()));
        }
        Ok(got)
    }
    async fn discard(&mut self) -> Result<(), SerialError> {
        self.0.serial_read(usize::MAX);
        Ok(())
    }
    async fn sleep(&mut self, ms: u32) {
        self.0.serial_idle(ms);
    }
}

/// Run a future whose every await is ready at once (the simulator's) to
/// completion.
///
/// # Panics
///
/// If the future waits on something: nothing would ever wake it.
pub fn block_on<F: core::future::Future>(f: F) -> F::Output {
    let mut f = core::pin::pin!(f);
    let mut cx = core::task::Context::from_waker(core::task::Waker::noop());
    match f.as_mut().poll(&mut cx) {
        core::task::Poll::Ready(v) => v,
        core::task::Poll::Pending => panic!("a simulated transfer waited"),
    }
}
