//! FastPROM firmware for the AG32VF303KCU6: pure Rust, no vendor C.
//!
//! At power-up, in this order:
//!
//! 0. Release the socket outputs (READY clear) if the fabric is ours. A warm
//!    reset does not reconfigure the fabric, so READY may still be set.
//! 1. Read the image-select switch on PIN_20 / PIN_21 as GPIO inputs,
//!    before UART0 takes the pins.
//! 2. Start the PLL: the block RAM's read port needs it.
//! 3. Start UART0 and print the boot banner.
//! 4. Parse the image table, check the image's CRC, load it through the
//!    fabric's AHB slave verifying the load pointer word by word, set READY.
//! 5. Idle. On any failure READY is never set, so the module stays
//!    high-impedance to the host, and the failure is reported on UART0
//!    every two seconds.
//!
//! The logic is in `fastprom-core` (host-tested); this crate is startup
//! (riscv-rt), the memory map, the registers and the report.
#![no_std]
#![no_main]

mod clock;
mod delay;
mod error;
mod fabric;
#[allow(
    dead_code,
    reason = "shared with build.rs, which uses every constant; the firmware needs only some"
)]
mod memmap;
mod mmio;
mod regs;
mod report;
mod switch;
mod uart;

use core::panic::PanicInfo;

use fastprom_core::select::SAMPLES;
use fastprom_core::{boot, Released};

use crate::clock::Clocks;
use crate::error::FwError;
use crate::fabric::Ag32Fabric;
use crate::report::Console;
use crate::uart::Uart0;

/// The boot report's baud rate (J3, pad 1).
const BAUD: u32 = 115_200;
/// How often a failure is repeated.
const REPEAT_US: u32 = 2_000_000;

/// The image table's flash.
fn table() -> &'static [u8] {
    // SAFETY: TABLE_BASE..+TABLE_LEN is on-chip flash (memmap.rs), always
    // mapped and readable by plain loads (AGM's example_flash.c reads flash
    // this way), and nothing writes flash while the firmware runs, so a
    // shared 'static borrow is sound. Unprogrammed flash reads as 0xFF,
    // which the table parser rejects.
    unsafe {
        core::slice::from_raw_parts(
            memmap::TABLE_BASE as usize as *const u8,
            memmap::TABLE_LEN as usize,
        )
    }
}

#[riscv_rt::entry]
fn main() -> ! {
    let mut fabric = Ag32Fabric::new();
    let claim = Released::claim(&mut fabric);

    let samples = switch::sample::<SAMPLES>(Clocks::hsi());
    let clocks = clock::start_pll();
    // What the core is running on, for baud rates and delays. A clock error
    // is not dropped: `clocks` keeps it for the banner and the result.
    let running = *clocks.as_ref().unwrap_or(&Clocks::hsi());

    switch::give_to_uart0();
    let mut console = Console::new(Uart0::new(running.pclk_hz(), BAUD));
    report::banner(&mut console, &samples, &clocks, &claim);

    let result = claim.map_err(FwError::Claim).and_then(|released| {
        clocks?;
        Ok(boot(released, table(), &samples)?)
    });
    report::outcome(&mut console, &result);

    match result {
        Ok(_) => loop {
            riscv::asm::wfi();
        },
        Err(e) => loop {
            delay::us(running.core_hz(), REPEAT_US);
            report::failure(&mut console, &e);
        },
    }
}

/// Never leave the socket driven: drop READY, then park.
fn halt() -> ! {
    fabric::force_release();
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
