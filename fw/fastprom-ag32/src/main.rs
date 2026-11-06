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
//! 5. Serve the update core (`fastprom_core::update`) over USB, which can
//!    rewrite the image table, reload the block RAM, or reboot into the DFU
//!    stage. On any boot failure READY is never set, so the module stays
//!    high-impedance to the host, the failure is reported on UART0 every
//!    two seconds, and USB is served all the same, so that the page can put
//!    a good table in.
//!
//! The binary sits at `memmap::APP_BASE` behind its header (`header.rs`);
//! the DFU stage at the reset vector jumps to it. Nothing above changed for
//! that: USB starts only once the outcome is reported.
//!
//! The logic is in `fastprom-core` (host-tested); the registers, clocks and
//! flash are `fastprom-hal`; this crate is startup (riscv-rt), the fabric
//! window, the switch, the UART, the report and the board's update traits.
#![no_std]
#![no_main]

mod board;
mod build_info {
    include!(concat!(env!("OUT_DIR"), "/build_info.rs"));
}
mod error;
mod fabric;
mod header;
mod report;
mod switch;
mod uart;

use core::panic::PanicInfo;

use fastprom_core::select::{self, SAMPLES};
use fastprom_core::update::{self, Identity, Mode};
use fastprom_core::{boot, Released};
use fastprom_hal::clock::{self, Clocks};
use fastprom_hal::delay::Every;
use fastprom_hal::flash::Ag32Flash;
use fastprom_hal::usb::Ag32Usb;
use fastprom_usb::{CdcAcm, Device, Serial};

use crate::board::{table, AppBoard};
use crate::error::FwError;
use crate::fabric::Ag32Fabric;
use crate::report::Console;
use crate::uart::Uart0;

/// The boot report's baud rate (J3, pad 1).
const BAUD: u32 = 115_200;
/// How often a failure is repeated.
const REPEAT_US: u32 = 2_000_000;

/// Who `INFO` says is serving.
const IDENTITY: Identity = Identity {
    mode: Mode::Application,
    fw_version: build_info::FW_VERSION,
    board_rev: build_info::BOARD_REV,
};

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
    let failed = result.err();

    // Only now, with the socket served (or refused): the update core.
    let flash = Ag32Flash::init();
    let serial = Serial::from_uid(flash.unique_id());
    let board = AppBoard::new(flash, select::decide(&samples).ok());
    let server = update::Server::new(&board, IDENTITY);
    // The application variant: the update interface 0, the CDC-ACM port
    // carrying this report on 1 and 2 (docs/11-usb-driver.md).
    let Some(controller) = Ag32Usb::take() else {
        halt()
    };
    let mut usb = Device::new(controller, serial, server, CdcAcm::new());
    let mut repeat = Every::new(running.core_hz(), REPEAT_US);
    loop {
        usb.poll();
        console.drain_log(|bytes| usb.function().write(bytes));
        if let Some(e) = &failed {
            if repeat.due() {
                report::failure(&mut console, e);
            }
        }
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
