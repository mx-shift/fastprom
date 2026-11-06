//! The update core's view of the application's board: the flash, a reload
//! of the block RAM through the existing loader, and the reboot.

use fastprom_core::memmap::{TABLE_BASE, TABLE_LEN};
use fastprom_core::update::{
    self, BootTarget, FlashError, Platform, ReloadError, Status, PAGE_LEN,
};
use fastprom_core::{Bank, ImageTable, Released, TableSummary};
use fastprom_hal::flash::Ag32Flash;
use fastprom_hal::reset;

use crate::fabric::{self, Ag32Fabric};

/// The image table's flash.
pub fn table() -> &'static [u8] {
    // SAFETY: TABLE_BASE..+TABLE_LEN is on-chip flash (memmap.rs), always
    // mapped and readable by plain loads (AGM's example_flash.c reads flash
    // this way). Only the update core writes it, between whole calls, on
    // this one thread: no borrow of it is held across a write, since every
    // reader here parses it and returns owned values (`reload` loads
    // before returning; `table_summary` copies). Unprogrammed flash reads
    // as 0xFF, which the table parser rejects.
    unsafe { core::slice::from_raw_parts(TABLE_BASE as usize as *const u8, TABLE_LEN as usize) }
}

/// The board, after boot.
pub struct AppBoard {
    flash: Ag32Flash,
    bank: Option<Bank>,
}

impl AppBoard {
    /// `bank` is what the switch read at boot (the pins are UART0's now).
    pub fn new(flash: Ag32Flash, bank: Option<Bank>) -> Self {
        Self { flash, bank }
    }
}

impl update::Flash for AppBoard {
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

impl Platform for AppBoard {
    fn reload(&self) -> Result<(), ReloadError> {
        // Release first, as at boot: READY clears before the table is read.
        let mut fabric = Ag32Fabric::new();
        let released = Released::claim(&mut fabric).map_err(|_| Status::PreconditionNotMet)?;
        let bank = self.bank.ok_or(Status::PreconditionNotMet)?;
        let table = ImageTable::parse(table()).map_err(|_| Status::InvalidData)?;
        let image = table.image(bank).ok_or(Status::InvalidData)?;
        released
            .load(&image)
            .map(|_| ())
            .map_err(|_| Status::InvalidData)
    }

    fn reboot(&self, to: BootTarget) {
        // The host is off for an update; never leave READY set across it.
        fabric::force_release();
        reset::reboot(to)
    }

    fn switch_bank(&self) -> Option<Bank> {
        self.bank
    }

    fn table_summary(&self) -> Option<TableSummary> {
        ImageTable::parse(table()).ok().map(|t| t.summary())
    }
}
