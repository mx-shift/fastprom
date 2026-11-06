//! The update core's view of the board in DFU mode.

use fastprom_core::memmap::{TABLE_BASE, TABLE_LEN};
use fastprom_core::regmap::{ReadReg, WriteReg, FABRIC_MAGIC};
use fastprom_core::update::{
    self, BootTarget, FlashError, Platform, ReloadError, Status, PAGE_LEN,
};
use fastprom_core::{Bank, ImageTable, TableSummary};
use fastprom_hal::flash::Ag32Flash;
use fastprom_hal::mmio::Reg;
use fastprom_hal::regs::fabric::BASE;
use fastprom_hal::reset;

/// The board, in DFU mode.
pub struct DfuBoard {
    flash: Ag32Flash,
}

impl DfuBoard {
    pub const fn new(flash: Ag32Flash) -> Self {
        Self { flash }
    }
}

/// Take the socket outputs down if the fabric is FastPROM's: DFU mode never
/// drives the socket, whatever a warm reset left behind.
pub fn release_socket() {
    // SAFETY: CTRL and ID of the fabric's AHB slave (fastprom-core regmap),
    // inside the AHB window, which reads as zero when unconfigured.
    let (id, ctrl) = unsafe {
        (
            Reg::new(BASE + ReadReg::Id.offset()),
            Reg::new(BASE + WriteReg::Ctrl.offset()),
        )
    };
    if id.read() == FABRIC_MAGIC {
        ctrl.write(0);
    }
}

impl update::Flash for DfuBoard {
    fn erase_sector(&self, addr: u32) -> Result<(), FlashError> {
        self.flash.erase_sector(addr)
    }

    fn program_page(&self, addr: u32, page: &[u8; PAGE_LEN]) -> Result<(), FlashError> {
        self.flash.program_page(addr, page)
    }

    fn read(&self, addr: u32, buf: &mut [u8]) {
        self.flash.read(addr, buf);
    }
}

impl Platform for DfuBoard {
    fn reload(&self) -> Result<(), ReloadError> {
        // The update core refuses LOAD in DFU mode before it gets here.
        Err(Status::NotPermitted)
    }

    fn reboot(&self, to: BootTarget) {
        reset::reboot(to)
    }

    fn switch_bank(&self) -> Option<Bank> {
        // The switch is the application's to read.
        None
    }

    fn table_summary(&self) -> Option<TableSummary> {
        // SAFETY: TABLE_BASE..+TABLE_LEN is on-chip flash, always mapped;
        // the borrow ends before this returns, and nothing writes flash
        // meanwhile (one thread, no interrupts).
        let table = unsafe {
            core::slice::from_raw_parts(TABLE_BASE as usize as *const u8, TABLE_LEN as usize)
        };
        ImageTable::parse(table).ok().map(|t| t.summary())
    }
}
