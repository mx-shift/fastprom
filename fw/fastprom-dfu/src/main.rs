//! FastPROM's DFU stage, at the reset vector (`memmap::DFU_BASE`).
//!
//! What the mask ROM boot loader is on a chip that has a USB one: the code
//! that can always rewrite the application, the fabric and the image table
//! over USB, because USB never rewrites it (`docs/10-updates.md`).
//!
//! At power-up it decides in a few register reads, before RAM is even
//! initialised, whether to jump to the application (`boot.rs`). Only if it
//! stays does `main` run: DFU mode, on its own USB-only fabric
//! (`fabric.rs`), the update core
//! (`fastprom_core::update`) in `Mode::Dfu`, until a `REBOOT` soft-resets
//! the chip.
#![no_std]
#![no_main]

mod board;
mod boot;
mod fabric;
mod build_info {
    include!(concat!(env!("OUT_DIR"), "/build_info.rs"));
}

use core::panic::PanicInfo;

use fastprom_core::fcb::Scratch;
use fastprom_core::update::{self, Identity, Mode};
use fastprom_hal::flash::Ag32Flash;
use fastprom_hal::usb::Ag32Usb;
use fastprom_hal::{clock, fcb};
use fastprom_usb::{Device, NoFunction, Serial};

use crate::board::{release_socket, DfuBoard};

/// Who `INFO` says is serving.
const IDENTITY: Identity = Identity {
    mode: Mode::Dfu,
    fw_version: build_info::FW_VERSION,
    board_rev: build_info::BOARD_REV,
};

#[riscv_rt::entry]
fn main() -> ! {
    release_socket();

    // The DFU stage's own fabric: USB0 and the clocks, nothing on any
    // socket pin (fabric.rs). USB's controller is enabled and clocked only
    // by the fabric configuration, so this is what keeps USB working when
    // the ROM's bitstream in flash is missing or damaged
    // (docs/10-updates.md, "DFU mode"). From here until the reboot out of
    // DFU mode, which configures the ROM's fabric from flash again, the
    // fabric has no AHB slave (gen_vlog's netlist ties the CPU's fabric port
    // ready, data zero), so release_socket() reads no FastPROM ID and writes
    // nothing. If the FCB refuses the bitstream there is no
    // better fabric to fall back to; carry on and serve what USB can.
    let mut scratch = Scratch::new();
    if fcb::configure_compressed(fabric::BITSTREAM, &mut scratch).is_err() {
        // Nothing to report it on: USB is what would carry it.
    }

    // USB's 60 MHz comes from the PLL (PLL output 1, which gen_vlog gives
    // USB when USB0 is enabled), configured by the fabric just loaded. If
    // it does not start, the core stays on HSI and serves what it can;
    // there is nothing better to fall back to.
    if clock::start_pll().is_err() {
        // HSI it is: the update core does not depend on the clock rate.
    }

    let flash = Ag32Flash::init();
    // The same serial number as the application's, so a browser that was
    // given the device finds it again in DFU mode.
    let serial = Serial::from_uid(flash.unique_id());
    let board = DfuBoard::new(flash);
    let server = update::Server::new(&board, IDENTITY);
    // The DFU variant: the update interface 0 and nothing else
    // (docs/11-usb-driver.md).
    let Some(controller) = Ag32Usb::take() else {
        halt()
    };
    let mut usb = Device::new(controller, serial, server, NoFunction);
    loop {
        usb.poll();
    }
}

/// Never leave the socket driven, then park.
fn halt() -> ! {
    release_socket();
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
