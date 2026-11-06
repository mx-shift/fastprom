//! Volatile 32-bit register access.
//!
//! The firmware's `unsafe` is here, where [`Reg`]s are named (`regs.rs`,
//! `fastprom-ag32`'s `fabric.rs`), where flash is borrowed or written
//! (`flash.rs`, `fastprom-ag32`'s `main.rs` and `board.rs`), at `mscratch`
//! (`reset.rs`), and in the DFU stage's boot check and jump
//! (`fastprom-dfu`'s `boot.rs`).

/// A 32-bit memory-mapped register.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Reg(usize);

impl Reg {
    /// Name the register at `addr`.
    ///
    /// # Safety
    ///
    /// `addr` must be a 4-byte-aligned device register (or mapped flash)
    /// that can be read and written as a `u32` at any time without
    /// violating memory safety; the side effects are the caller's concern.
    pub const unsafe fn new(addr: usize) -> Self {
        Self(addr)
    }

    /// One 32-bit read.
    #[inline(always)]
    pub fn read(self) -> u32 {
        // SAFETY: Reg::new's contract.
        unsafe { core::ptr::read_volatile(self.0 as *const u32) }
    }

    /// One 32-bit write.
    #[inline(always)]
    pub fn write(self, value: u32) {
        // SAFETY: Reg::new's contract.
        unsafe { core::ptr::write_volatile(self.0 as *mut u32, value) }
    }

    /// Read, clear `clear`, set `set`, write.
    #[inline(always)]
    pub fn modify(self, clear: u32, set: u32) {
        self.write((self.read() & !clear) | set);
    }
}
