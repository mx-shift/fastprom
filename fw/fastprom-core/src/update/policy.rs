//! The range policy over the board's flash: what `READ`, `WRITE` and
//! `ERASE` may touch in each mode, and the commit-word rules
//! (`super`'s documentation has both).

use crate::header::{APP_MAGIC, ERASED, MANIFEST_MAGIC, OFF_COMMIT};
use crate::memmap::{APP_BASE, FLASH_BASE, FLASH_SIZE, MANIFEST_BASE, PAGE, SECTOR};

use super::{Flash, Mode, Region, Status, PAGE_LEN};

/// Bytes read at a time when blank-checking an erase.
const CHECK_LEN: usize = 64;

/// The policy for one mode, on one board.
pub struct Policy<'a, B> {
    board: &'a B,
    mode: Mode,
}

impl<'a, B: Flash> Policy<'a, B> {
    pub const fn new(board: &'a B, mode: Mode) -> Self {
        Self { board, mode }
    }

    /// Whether `addr..addr + len` may be read: anywhere in the main array.
    pub fn readable(addr: u32, len: u32) -> Result<(), Status> {
        let end = addr.checked_add(len).ok_or(Status::InvalidAddress)?;
        if addr >= FLASH_BASE && end <= FLASH_BASE + FLASH_SIZE {
            Ok(())
        } else {
            Err(Status::InvalidAddress)
        }
    }

    /// Whether a range may be erased or written in this mode.
    fn writable(&self, addr: u32, len: u32, granule: u32) -> Result<Region, Status> {
        let region = Region::of(addr, len)?;
        if !region.writable(self.mode) {
            return Err(Status::NotPermitted);
        }
        if !addr.is_multiple_of(granule) || !len.is_multiple_of(granule) {
            return Err(Status::BadAlignment);
        }
        Ok(region)
    }

    /// The commit word at `page`.
    fn commit_word(&self, page: u32) -> u32 {
        let mut w = [0u8; 4];
        self.board.read(page, &mut w);
        u32::from_le_bytes(w)
    }

    /// The commit word that guards `region`, and where it is: the header
    /// for the application, the manifest for the fabric and itself. Only in
    /// DFU mode: the application writes nothing that is committed.
    fn guard(&self, region: Region) -> Option<(u32, u32)> {
        match (self.mode, region) {
            (Mode::Dfu, Region::Application) => Some((APP_BASE, APP_MAGIC)),
            (Mode::Dfu, Region::Fabric | Region::Manifest) => Some((MANIFEST_BASE, MANIFEST_MAGIC)),
            _ => None,
        }
    }

    /// Refuse to change a payload while its page is committed.
    fn guarded(&self, region: Region, addr: u32, len: u32, erase: bool) -> Result<(), Status> {
        let Some((page, _)) = self.guard(region) else {
            return Ok(());
        };
        // An erase that takes the page invalidates it first (`erase`).
        let takes_page = erase && addr <= page && page < addr.saturating_add(len);
        if takes_page || self.commit_word(page) == ERASED {
            Ok(())
        } else {
            Err(Status::PreconditionNotMet)
        }
    }

    /// `ERASE`: the sector at `addr` (checked), its commit word invalidated
    /// first if it holds one, then blank-checked.
    pub fn erase(&self, addr: u32) -> Result<(), Status> {
        let region = self.writable(addr, SECTOR, SECTOR)?;
        self.guarded(region, addr, SECTOR, true)?;
        if let Some((page, _)) = self.guard(region) {
            if addr <= page && page < addr + SECTOR && self.commit_word(page) != ERASED {
                // Zero it: always programmable, never the magic, and no
                // interrupted erase from zero can make the magic's bytes.
                let mut zero = [0xFFu8; PAGE_LEN];
                if let Some(word) = zero.get_mut(OFF_COMMIT..OFF_COMMIT + 4) {
                    word.fill(0);
                }
                self.board
                    .program_page(page, &zero)
                    .map_err(|_| Status::FlashFailed)?;
                if self.commit_word(page) != 0 {
                    return Err(Status::FlashFailed);
                }
            }
        }
        self.board
            .erase_sector(addr)
            .map_err(|_| Status::FlashFailed)?;
        self.blank_check(addr)
    }

    fn blank_check(&self, sector: u32) -> Result<(), Status> {
        let mut buf = [0u8; CHECK_LEN];
        let mut at = sector;
        while at < sector + SECTOR {
            self.board.read(at, &mut buf);
            if buf.iter().any(|&b| b != 0xFF) {
                return Err(Status::FlashFailed);
            }
            at += CHECK_LEN as u32;
        }
        Ok(())
    }

    /// `WRITE`: program the page at `addr` (checked) and read it back. The
    /// header's (or the manifest's) commit word is held back for `COMMIT`.
    pub fn write(&self, addr: u32, page: &[u8; PAGE_LEN]) -> Result<(), Status> {
        let region = self.writable(addr, PAGE, PAGE)?;
        self.guarded(region, addr, PAGE, false)?;
        let mut data = *page;
        if self.guard(region).is_some_and(|(at, _)| at == addr) {
            if let Some(word) = data.get_mut(OFF_COMMIT..OFF_COMMIT + 4) {
                word.fill(0xFF);
            }
        }
        self.board
            .program_page(addr, &data)
            .map_err(|_| Status::FlashFailed)?;
        let mut back = [0u8; PAGE_LEN];
        self.board.read(addr, &mut back);
        if back == data {
            Ok(())
        } else {
            Err(Status::FlashFailed)
        }
    }
}
