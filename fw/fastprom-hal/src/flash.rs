//! The on-chip flash, for the update core: sector erase, page program, read;
//! and, for the recovery loader alone, the option bytes.
//!
//! A port of AGM's `FLASH_Unlock`, `FLASH_EraseSector`, `FLASH_ProgramWord`
//! and `FLASH_Lock` (framework-agrv_sdk/src/flash.c), with every wait
//! bounded and errors checked.
//!
//! **The erase and program sequences run from SRAM.** The SDK's own flash
//! example puts its flash routine in `.text.sram`, the AgRV DFU example
//! builds as `flash_sram` (everything copied to SRAM), and `FLASH_Start` is
//! 64-byte aligned "to make sure FLASH_CR_STRT is set when flash is not
//! active": an erase or program must not race an instruction fetch from the
//! flash it is changing. So the two routines that start an operation and
//! wait for it are in `.ramfunc`, which the linker loads from flash
//! (`linker.rs`) and [`Ag32Flash::init`] copies to SRAM, not riscv-rt's
//! startup: the application does not pay for the copy before READY. They
//! call nothing (register access is `#[inline(always)]`), so no fetch from
//! flash happens while the controller is busy. Interrupts are never enabled
//! in this firmware, so nothing else can fetch either.

use fastprom_core::memmap::{OPTION_BASE, OPTION_LEN};
use fastprom_core::update::{self, FlashError, PAGE_LEN};

use crate::regs::{flash, sys};

/// The option bytes, as words.
const OPTION_WORDS: usize = OPTION_LEN as usize / 4;
/// Polls of SR for an option-byte erase or program. Programming RDP to
/// "unprotected" may erase the whole chip first (AGM's uart_master.c: "this
/// will trigger an auto full chip erase"; its gen_batch: "auto full chip
/// erase"), several seconds of SPI NOR at most: ten times the sector bound.
const OPTION_POLLS: u32 = 10 * ERASE_POLLS;

/// Polls of SR before an erase is given up on: a 4 KiB SPI NOR sector
/// erase is 45 to 400 ms (typical to maximum, for the common 25-series
/// parts; AGM gives no figure), and a poll is at least 2 cycles, so this is
/// well over a second even at 100 MHz.
const ERASE_POLLS: u32 = 100_000_000;
/// Polls before a word program is given up on (a page program is 0.7 to
/// 3 ms on the same parts, and a word is at most that).
const PROGRAM_POLLS: u32 = 1_000_000;

unsafe extern "C" {
    // .ramfunc's run address and its load address in flash (linker.rs).
    static mut __sramfunc: u32;
    static mut __eramfunc: u32;
    static __siramfunc: u32;
}

/// The flash controller. Only `main` makes one.
#[derive(Debug)]
pub struct Ag32Flash {
    _owned: (),
}

impl Ag32Flash {
    /// Copy the erase and program routines into SRAM; then the flash may be
    /// written.
    pub fn init() -> Self {
        // SAFETY: the linker places .ramfunc (__sramfunc..__eramfunc) in
        // SRAM that nothing else uses (below the stack, after .bss) and its
        // image at __siramfunc in flash, both word aligned and the same
        // length. Copying it is all that makes the routines callable.
        unsafe {
            let mut dst = &raw mut __sramfunc;
            let end = &raw mut __eramfunc;
            let mut src = &raw const __siramfunc;
            while dst < end {
                dst.write_volatile(src.read_volatile());
                dst = dst.add(1);
                src = src.add(1);
            }
        }
        // The stores above are instructions from here on.
        riscv::asm::fence();
        riscv::asm::fence_i();
        Self { _owned: () }
    }
}

fn unlock() {
    if flash::CR.read() & flash::CR_LOCK != 0 {
        flash::KEYR.write(flash::KEY1);
        flash::KEYR.write(flash::KEY2);
    }
    // Clear any flags a previous operation left.
    flash::SR.write(flash::SR_PGERR | flash::SR_WRPRTERR | flash::SR_EOP);
}

fn lock() {
    flash::CR.modify(flash::CR_OPTWRE, flash::CR_LOCK);
}

/// flash.c FLASH_OptionByteUnlock: the flash, then the option bytes.
fn unlock_options() {
    unlock();
    if flash::CR.read() & flash::CR_OPTWRE == 0 {
        flash::OPTKEYR.write(flash::KEY1);
        flash::OPTKEYR.write(flash::KEY2);
    }
}

/// What an operation's status says.
fn outcome(sr: u32, done: bool) -> Result<(), FlashError> {
    if !done {
        Err(FlashError::Timeout)
    } else if sr & (flash::SR_PGERR | flash::SR_WRPRTERR) != 0 {
        Err(FlashError::Failed)
    } else {
        Ok(())
    }
}

/// Erase the sector at `addr` and wait (flash.c FLASH_EraseSector, with
/// FLASH_Start's idle first). Returns (SR, finished).
#[unsafe(link_section = ".ramfunc.erase")]
#[inline(never)]
fn erase_in_ram(addr: u32, idle: u32) -> (u32, bool) {
    flash::AR.write(addr);
    flash::CR.modify(flash::CR_ERASE_MASK, flash::CR_SER);
    // FLASH_Start: UTIL_IdleCycle(SYS_GetSclkDividerHigh() * 256) before
    // STRT; each SR read is at least a cycle.
    for _ in 0..idle {
        flash::SR.read();
    }
    flash::CR.modify(0, flash::CR_STRT);
    let mut done = false;
    for _ in 0..ERASE_POLLS {
        if flash::SR.read() & flash::SR_BSY == 0 {
            done = true;
            break;
        }
    }
    flash::CR.modify(flash::CR_ERASE_MASK, 0);
    (flash::SR.read(), done)
}

/// Program `words` at `addr`, one word at a time, skipping erased words
/// (flash.c FLASH_ProgramWord: PG, store, wait for BSY to clear). Returns
/// (SR, finished).
#[unsafe(link_section = ".ramfunc.program")]
#[inline(never)]
fn program_in_ram(addr: u32, words: &[u32; PAGE_LEN / 4]) -> (u32, bool) {
    let mut done = true;
    flash::CR.modify(0, flash::CR_PG);
    let mut at = addr as usize as *mut u32;
    for &w in words {
        if w != u32::MAX {
            // SAFETY: `at` walks the page at `addr`, which the update core
            // checked is a page-aligned page of the main array; with PG set,
            // a word store there is the program operation.
            unsafe { at.write_volatile(w) };
            let mut idle = false;
            for _ in 0..PROGRAM_POLLS {
                if flash::SR.read() & flash::SR_BSY == 0 {
                    idle = true;
                    break;
                }
            }
            done &= idle;
        }
        at = at.wrapping_add(1);
    }
    flash::CR.modify(flash::CR_PROGRAM_MASK, 0);
    (flash::SR.read(), done)
}

/// Erase the option bytes and wait (flash.c FLASH_EraseOptionByte: OPTER,
/// FLASH_Start, wait). Returns (SR, finished).
#[unsafe(link_section = ".ramfunc.option_erase")]
#[inline(never)]
fn erase_options_in_ram(idle: u32) -> (u32, bool) {
    flash::CR.modify(flash::CR_ERASE_MASK, flash::CR_OPTER);
    for _ in 0..idle {
        flash::SR.read();
    }
    flash::CR.modify(0, flash::CR_STRT);
    let mut done = false;
    for _ in 0..OPTION_POLLS {
        if flash::SR.read() & flash::SR_BSY == 0 {
            done = true;
            break;
        }
    }
    flash::CR.modify(flash::CR_ERASE_MASK, 0);
    (flash::SR.read(), done)
}

/// Program the option bytes a word at a time, skipping erased words: PG
/// and OPTPG, then a word store, as flash.c FLASH_SetFPGAConfigAddress
/// writes FPGA_CONFIG (and AGM's booter its option-byte writes, by 32-bit
/// DMA). Returns (SR, finished).
#[unsafe(link_section = ".ramfunc.option_program")]
#[inline(never)]
fn program_options_in_ram(words: &[u32; OPTION_WORDS]) -> (u32, bool) {
    let mut done = true;
    flash::CR.modify(0, flash::CR_PG | flash::CR_OPTPG);
    let mut at = OPTION_BASE as usize as *mut u32;
    for &w in words {
        if w != u32::MAX {
            // SAFETY: `at` walks the 128 option bytes from OPTION_BASE
            // (flash.h FLASH_OPTION_BASE, FLASH_OptionByteTypeDef); with PG
            // and OPTPG set a word store there programs it.
            unsafe { at.write_volatile(w) };
            let mut idle = false;
            for _ in 0..OPTION_POLLS {
                if flash::SR.read() & flash::SR_BSY == 0 {
                    idle = true;
                    break;
                }
            }
            done &= idle;
        }
        at = at.wrapping_add(1);
    }
    flash::CR.modify(flash::CR_PROGRAM_MASK, 0);
    (flash::SR.read(), done)
}

/// Polls of SR before a flexible read is given up on (a few bytes over SPI:
/// microseconds).
const READ_POLLS: u32 = 1_000_000;

/// One flash.c FLASH_FlexRead without an address: READ_CTRL, CR READ, the
/// FLASH_Start idle, STRT, wait for BSY to clear, READ off, READ_DATA. From
/// SRAM, like the erase and program: the command goes to the flash the core
/// executes from. Returns (READ_DATA, finished).
#[unsafe(link_section = ".ramfunc.flexread")]
#[inline(never)]
fn flex_read_in_ram(ctrl: u32, idle: u32) -> (u32, bool) {
    flash::READ_CTRL.write(ctrl);
    flash::CR.modify(0, flash::CR_READ);
    for _ in 0..idle {
        flash::SR.read();
    }
    flash::CR.modify(0, flash::CR_STRT);
    let mut done = false;
    for _ in 0..READ_POLLS {
        if flash::SR.read() & flash::SR_BSY == 0 {
            done = true;
            break;
        }
    }
    flash::CR.modify(flash::CR_READ, 0);
    (flash::READ_DATA.read(), done)
}

impl Ag32Flash {
    /// The chip's unique ID, for the USB serial number: flash.c
    /// FLASH_GetUniqueID, the flash's Read Unique ID command (0x4B) through
    /// four flexible reads of 4, 8, 12 and 16 bytes after 4 dummy bytes,
    /// READ_DATA keeping the last word of each. The AG32 has no unique-ID
    /// register of its own in the SDK. A read that did not finish gives 0
    /// for its word.
    pub fn unique_id(&self) -> [u32; 4] {
        let div = (sys::CLK_CNTL.read() >> sys::SCLK_DIV_HIGH_SHIFT) & sys::SCLK_DIV_MASK;
        let mut id = [0u32; 4];
        for (word, bytes) in id.iter_mut().zip([4u8, 8, 12, 16]) {
            let ctrl = flash::read_ctrl(flash::CMD_READ_UNIQUE_ID, bytes, 4);
            let (value, done) = flex_read_in_ram(ctrl, div * 256);
            *word = if done { value } else { 0 };
        }
        id
    }
}

impl Ag32Flash {
    /// Erase the option bytes. Only the recovery loader does: the
    /// application and the DFU stage never touch them.
    ///
    /// # Errors
    ///
    /// When the controller flags an error or does not finish.
    pub fn erase_options(&self) -> Result<(), FlashError> {
        let div = (sys::CLK_CNTL.read() >> sys::SCLK_DIV_HIGH_SHIFT) & sys::SCLK_DIV_MASK;
        unlock_options();
        let (sr, done) = erase_options_in_ram(div * 256);
        lock();
        outcome(sr, done)
    }

    /// Program the option bytes from `image` (erase them first).
    ///
    /// # Errors
    ///
    /// When the controller flags an error or does not finish.
    pub fn program_options(&self, image: &[u8; OPTION_LEN as usize]) -> Result<(), FlashError> {
        let mut words = [u32::MAX; OPTION_WORDS];
        let (chunks, _) = image.as_chunks::<4>();
        for (w, b) in words.iter_mut().zip(chunks) {
            *w = u32::from_le_bytes(*b);
        }
        // AGM's booter unlocks the option bytes again after an erase.
        unlock_options();
        let (sr, done) = program_options_in_ram(&words);
        lock();
        outcome(sr, done)
    }

    /// The option bytes as they read now.
    pub fn read_options(&self, buf: &mut [u8; OPTION_LEN as usize]) {
        // SAFETY: OPTION_BASE..+OPTION_LEN is the option-byte area, mapped
        // and readable by plain loads (flash.h FLASH_OB, which the SDK reads
        // as a struct); no option-byte operation is in progress here.
        let src = unsafe {
            core::slice::from_raw_parts(OPTION_BASE as usize as *const u8, OPTION_LEN as usize)
        };
        buf.copy_from_slice(src);
    }
}

impl update::Flash for Ag32Flash {
    fn erase_sector(&self, addr: u32) -> Result<(), FlashError> {
        let div = (sys::CLK_CNTL.read() >> sys::SCLK_DIV_HIGH_SHIFT) & sys::SCLK_DIV_MASK;
        unlock();
        let (sr, done) = erase_in_ram(addr, div * 256);
        lock();
        outcome(sr, done)
    }

    fn program_page(&self, addr: u32, page: &[u8; PAGE_LEN]) -> Result<(), FlashError> {
        let mut words = [0u32; PAGE_LEN / 4];
        let (chunks, _) = page.as_chunks::<4>();
        for (w, b) in words.iter_mut().zip(chunks) {
            *w = u32::from_le_bytes(*b);
        }
        unlock();
        let (sr, done) = program_in_ram(addr, &words);
        lock();
        outcome(sr, done)
    }

    fn read(&self, addr: u32, buf: &mut [u8]) {
        // SAFETY: the update core reads only inside the main array
        // (fastprom-core update::Device::read_prepare and its own fixed
        // pages), which is always mapped and readable by plain loads
        // (AGM's example_flash.c reads it this way). No erase or program is
        // in progress: those run to completion before returning.
        let src = unsafe { core::slice::from_raw_parts(addr as usize as *const u8, buf.len()) };
        buf.copy_from_slice(src);
    }
}
