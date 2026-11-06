//! The AG32 board support FastPROM's binaries share: the registers,
//! volatile access to them, the clocks, busy-wait delays, the on-chip flash
//! for the update core (and its option bytes, for the recovery loader),
//! fabric configuration through the FCB (the DFU stage's own fabric), the
//! soft reset that carries `mscratch` into the DFU stage, UART0 both ways
//! (`uart`, the recovery loader's), and the USB controller (`usb`, under
//! fastprom-usb's device stack).
//!
//! `linker.rs` beside `src/` is not part of the crate: each binary's
//! build.rs includes it to write its memory.x from fastprom-core's memmap.
#![no_std]

pub mod clock;
pub mod delay;
pub mod fcb;
pub mod flash;
pub mod mmio;
pub mod regs;
pub mod reset;
pub mod uart;
pub mod usb;
