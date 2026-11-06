//! Soft reset, and the `mscratch` flag that survives it.
//!
//! AGM's DFU pattern (platforms/AgRV/examples/dfu/src/dfu.c): `mscratch` is
//! not cleared by a soft reset, so a value written there before
//! `SYS_SoftwareReset` (system.h: RST_CNTL |= SYS_RST_SFT) is what the boot
//! loader reads after it. A power-on reset leaves it undefined, which is why
//! the DFU stage clears it when RST_CNTL's POR flag is set.
//!
//! The DFU stage reads it in its `__pre_init` (`fastprom-dfu`'s `boot.rs`):
//! [`DFU_REQUEST`] there means stay in DFU mode; anything else, boot the
//! application if its header is sane.
//!
//! The soft reset also configures the fabric from flash again, as power-on
//! does (the FCB is reset with the chip unless RST_CNTL FCB_DIS is set), so
//! a reboot out of DFU mode brings back the ROM's fabric in place of the DFU
//! stage's USB-only one.

use fastprom_core::header::DFU_REQUEST;
use fastprom_core::update::BootTarget;

use crate::regs::sys;

/// Soft-reset into `to`.
pub fn reboot(to: BootTarget) -> ! {
    let flag = match to {
        BootTarget::Application => 0,
        BootTarget::Dfu => DFU_REQUEST,
    };
    // SAFETY: mscratch is a scratch register; nothing in this firmware
    // (riscv-rt included: single-hart, no trap stack swap) reads it except
    // the DFU stage's boot check.
    unsafe { riscv::register::mscratch::write(flag as usize) };
    // The reset must take the fabric with it: leaving DFU mode, the fabric
    // is the DFU stage's USB-only one, and the application needs its own
    // from flash. A reset resets the FCB, which reconfigures the fabric
    // from the option bytes' address, unless RST_CNTL's FCB_DIS is set
    // (system.h SYS_RST_FCB_DIS): nothing here sets it, and clearing it
    // makes sure (docs/10-updates.md, "Leaving DFU mode").
    sys::RST_CNTL.modify(sys::RST_FCB_DIS, sys::RST_SFT);
    loop {
        riscv::asm::wfi();
    }
}
