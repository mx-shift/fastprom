//! FastPROM's recovery loader, at `memmap::RECOVERY_BASE` in SRAM.
//!
//! Never in flash: the page uploads it through the mask ROM's UART loader
//! (`WRITE MEMORY` to SRAM, then `GO` to [`RECOVERY_ENTRY`]), as AGM's
//! flasher uploads its own RAM booter, so that recovery asks the mask ROM
//! for nothing but SRAM writes and a jump (`docs/10-updates.md`,
//! "Recovery"). From there it is FastPROM's own code that writes the flash:
//! the option bytes, sector erases, page writes read back, over the same
//! AN3155 framing (`fastprom_core::recovery::server`).
//!
//! In order: the PLL from the 12 MHz oscillator, as the application starts
//! it; UART0 on PIN_20 and PIN_21 at [`BAUD`] 8E1, the mask ROM's framing,
//! so the host's port stays as it was; the flash routines; then the
//! server, until the host resets the chip. Nothing drives the socket: the
//! fabric is left as it is (blank, or the ROM's with its outputs released).
//!
//! [`RECOVERY_ENTRY`]: fastprom_core::memmap::RECOVERY_ENTRY
#![no_std]
#![no_main]

mod build_info {
    include!(concat!(env!("OUT_DIR"), "/build_info.rs"));
}

use core::panic::PanicInfo;

use fastprom_core::memmap::{OPTION_LEN, RECOVERY_HEADER_LEN};
use fastprom_core::recovery::server::{Server, Target, IDLE_MS};
use fastprom_core::recovery::{LoaderHeader, BAUD};
use fastprom_core::update::{BootTarget, Flash, FlashError, PAGE_LEN};
use fastprom_hal::clock::{self, Clocks};
use fastprom_hal::delay::Every;
use fastprom_hal::flash::Ag32Flash;
use fastprom_hal::regs::sys;
use fastprom_hal::reset;
use fastprom_hal::uart::{self, Uart0};

/// The header `tools/fw_stamp.py loader` stamps: the length and CRC of
/// everything after it are zero until then.
#[unsafe(link_section = ".fastprom_loader_header")]
#[used]
static HEADER: [u8; RECOVERY_HEADER_LEN as usize] =
    LoaderHeader::unstamped(build_info::FW_VERSION, build_info::BOARD_REV);

// Where GO lands: the mask ROM wrote this code as data, so make the core
// fetch it afresh (AGM's booter starts with the same `fence.i`), then
// riscv-rt's start-up.
core::arch::global_asm!(
    ".section .fastprom_loader_entry, \"ax\"",
    ".global __fastprom_loader_entry",
    ".option push",
    ".option norvc",
    "__fastprom_loader_entry:",
    "fence.i",
    "j _start",
    ".option pop",
);

/// The chip, for the server.
struct Chip {
    flash: Ag32Flash,
}

impl Flash for Chip {
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

impl Target for Chip {
    fn device_id(&self) -> u32 {
        sys::DEVICE_ID.read()
    }

    fn read_options(&self, buf: &mut [u8; OPTION_LEN as usize]) {
        self.flash.read_options(buf);
    }

    fn erase_options(&self) -> Result<(), FlashError> {
        self.flash.erase_options()
    }

    fn program_options(&self, image: &[u8; OPTION_LEN as usize]) -> Result<(), FlashError> {
        self.flash.program_options(image)
    }

    fn reset(&self) {
        // The two ACKs out first; then the soft reset (with BOOT0 still
        // high, back into the mask ROM).
        uart::flush();
        reset::reboot(BootTarget::Application)
    }
}

#[riscv_rt::entry]
fn main() -> ! {
    // If the oscillator or the PLL does not start, the core stays on HSI,
    // whose rate is only nominal: the baud rate is then a guess, and the
    // host will hear nothing it can read, which it reports.
    let clocks = clock::start_pll().unwrap_or(Clocks::hsi());
    let Some(mut port) = Uart0::new(clocks.pclk_hz(), BAUD) else {
        halt()
    };
    let chip = Chip {
        flash: Ag32Flash::init(),
    };
    let mut server = Server::new();
    let mut quiet = Every::new(clocks.core_hz(), IDLE_MS * 1000);
    loop {
        if let Some(b) = port.read() {
            server.feed(b, &chip, &mut |reply: &[u8]| port.write(reply));
            quiet = Every::new(clocks.core_hz(), IDLE_MS * 1000);
        } else if server.busy() && quiet.due() {
            server.idle();
        }
    }
}

fn halt() -> ! {
    loop {
        riscv::asm::wfi();
    }
}

#[panic_handler]
fn panic(_info: &PanicInfo<'_>) -> ! {
    halt()
}

#[export_name = "ExceptionHandler"]
fn exception_handler(_trap_frame: &riscv_rt::TrapFrame) -> ! {
    halt()
}
