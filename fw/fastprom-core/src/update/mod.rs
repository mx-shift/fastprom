//! The update core: FastPROM's update protocol, device side.
//!
//! Both board binaries serve the same protocol (`docs/10-updates.md`, "The
//! update protocol"; [`proto`] has the wire format): the application, after
//! the socket is up, and the DFU stage in DFU mode. What differs is the
//! [`Mode`], which decides what may be written, and the board, which
//! supplies two things through traits:
//!
//! - [`Flash`]: erase a sector, program a page, read. NOR semantics:
//!   programming only clears bits, and a word of `0xFFFFFFFF` programs
//!   nothing.
//! - [`Platform`]: reload the block RAM (the application's `LOAD`), reboot
//!   into the application or the DFU stage (`REBOOT`, through `mscratch`),
//!   and what `INFO` reports about the switch and the image table.
//!
//! [`Server`] takes the requests and makes the responses; the range policy
//! (below) is `policy.rs`, `COMMIT` is `commit.rs`, `INFO`'s answer
//! [`info`]. The server borrows the one board: its methods take `&self`, as
//! registers do.
//!
//! # The range policy
//!
//! Reads: anywhere in the main flash array. Erases and writes: sector- and
//! page-aligned, inside one region of [`crate::memmap`], and only:
//!
//! | region | application | DFU stage |
//! |---|---|---|
//! | DFU stage, option bytes | no | no |
//! | application | no | yes |
//! | fabric | no | yes |
//! | manifest | no | yes |
//! | image table | yes | yes |
//!
//! # Committing
//!
//! The application header and the fabric manifest begin with a commit word
//! ([`crate::header`]) that only `COMMIT` programs, after checking the
//! payload's CRC-32 against the page. So, in DFU mode:
//!
//! - a write of the header page (or the manifest page) programs everything
//!   but the commit word, which stays erased: the file written is the one
//!   the UART loader would write, magic and all, and the device holds the
//!   magic back;
//! - an erase that covers the header sector (or the manifest sector) first
//!   programs the commit word to zero, so that an interrupted erase cannot
//!   leave the old magic over a half-erased payload;
//! - any other erase or write of the application (or the fabric) is refused
//!   with `PreconditionNotMet` while the header (or the manifest) is still
//!   committed, so a committed page can never describe a payload that has
//!   changed since.
//!
//! Whatever the moment power is cut, then, the commit word is either the
//! magic over the payload it was committed with, or not the magic.
//! `tests/update.rs` cuts power at every flash operation of an update and
//! checks exactly that.

mod commit;
pub mod info;
mod policy;
pub mod proto;
mod server;

pub use commit::CommitTarget;
pub use proto::Status;
pub use server::Server;

use crate::bank::Bank;
use crate::header::{BoardRev, Version};
use crate::memmap::{
    APP_BASE, APP_LEN, DFU_BASE, DFU_LEN, FABRIC_BASE, FABRIC_LEN, FLASH_BASE, FLASH_SIZE,
    MANIFEST_BASE, MANIFEST_LEN, OPTION_BASE, OPTION_LEN, PAGE, TABLE_BASE, TABLE_LEN,
};
use crate::table::TableSummary;

/// One flash page.
pub const PAGE_LEN: usize = PAGE as usize;

/// Which binary is serving.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// `fastprom-ag32`, with the socket up (or failed).
    Application,
    /// `fastprom-dfu` in DFU mode.
    Dfu,
}

/// A region of the flash map.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Region {
    Dfu,
    Application,
    Fabric,
    Manifest,
    Table,
}

impl Region {
    const ALL: [(Self, u32, u32); 5] = [
        (Self::Dfu, DFU_BASE, DFU_LEN),
        (Self::Application, APP_BASE, APP_LEN),
        (Self::Fabric, FABRIC_BASE, FABRIC_LEN),
        (Self::Manifest, MANIFEST_BASE, MANIFEST_LEN),
        (Self::Table, TABLE_BASE, TABLE_LEN),
    ];

    /// The region `addr..addr + len` lies in.
    ///
    /// # Errors
    ///
    /// `InvalidAddress` outside the main array and the option bytes,
    /// `NotPermitted` in the option bytes, `BadRequest` for an empty range
    /// or one that crosses a region boundary (neither can come from one
    /// page or one sector, which is all a request names).
    pub fn of(addr: u32, len: u32) -> Result<Self, Status> {
        let end = addr.checked_add(len).ok_or(Status::InvalidAddress)?;
        if overlaps(addr, end, OPTION_BASE, OPTION_LEN) {
            return Err(Status::NotPermitted);
        }
        if !within(addr, end, FLASH_BASE, FLASH_SIZE) {
            return Err(Status::InvalidAddress);
        }
        if len == 0 {
            return Err(Status::BadRequest);
        }
        Self::ALL
            .iter()
            .find(|&&(_, base, size)| within(addr, end, base, size))
            .map(|&(region, _, _)| region)
            .ok_or(Status::BadRequest)
    }

    /// Whether `mode` may erase and write it.
    pub const fn writable(self, mode: Mode) -> bool {
        match (self, mode) {
            (Self::Table, _) => true,
            (Self::Application | Self::Fabric | Self::Manifest, Mode::Dfu) => true,
            (Self::Dfu, _) | (_, Mode::Application) => false,
        }
    }
}

fn within(addr: u32, end: u32, base: u32, len: u32) -> bool {
    addr >= base && end <= base.saturating_add(len)
}

fn overlaps(addr: u32, end: u32, base: u32, len: u32) -> bool {
    addr < base.saturating_add(len) && end > base
}

/// A flash operation the board could not complete.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FlashError {
    /// The controller stayed busy past its bound.
    Timeout,
    /// The controller flagged a program or protection error.
    Failed,
}

/// The board's flash. The address is absolute; the update core checks
/// every range against the map before calling.
pub trait Flash {
    /// Erase the 4 KiB sector at `addr` (sector aligned).
    ///
    /// # Errors
    ///
    /// When the controller reports a failure or does not finish.
    fn erase_sector(&self, addr: u32) -> Result<(), FlashError>;

    /// Program the page at `addr` (page aligned): each bit becomes the
    /// AND of what was there and `page`. Words of all ones program nothing.
    ///
    /// # Errors
    ///
    /// When the controller reports a failure or does not finish.
    fn program_page(&self, addr: u32, page: &[u8; PAGE_LEN]) -> Result<(), FlashError>;

    /// Fill `buf` from `addr` (inside the main array).
    fn read(&self, addr: u32, buf: &mut [u8]);
}

/// Where a reboot goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BootTarget {
    /// Through the DFU stage's boot check to the application.
    Application,
    /// The DFU stage, in DFU mode (`mscratch` = `header::DFU_REQUEST`).
    Dfu,
}

/// Why a `LOAD` failed, for the host: the socket is released either way.
pub type ReloadError = Status;

/// The rest of what the update core needs from the board.
pub trait Platform {
    /// Load the image the switch selected at boot from the image table
    /// into block RAM again, with the socket released throughout, and set
    /// READY. The application's `LOAD`; the DFU stage refuses it before
    /// this is reached.
    ///
    /// # Errors
    ///
    /// The status to report; the socket stays released.
    fn reload(&self) -> Result<(), ReloadError>;

    /// Soft-reset into `to`. Does not return on hardware.
    fn reboot(&self, to: BootTarget);

    /// The bank the image-select switch read at boot, if it was read.
    fn switch_bank(&self) -> Option<Bank>;

    /// The image table in flash, if it parses.
    fn table_summary(&self) -> Option<TableSummary>;
}

/// Who is serving: what `INFO` reports about the running binary, and what
/// `COMMIT` checks a payload's board revision against.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Identity {
    pub mode: Mode,
    pub fw_version: Version,
    pub board_rev: BoardRev,
}
