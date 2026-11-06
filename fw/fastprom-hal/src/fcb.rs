//! Configuring the fabric from the CPU: the FCB's AUTO mode.
//!
//! At power-on the AG32 configures its fabric from flash in hardware, from
//! the address in the option bytes. Software can configure it again at any
//! time by feeding a bitstream to the fabric configuration block, one word
//! per write to its AUTO register (fcb.c FCB_AutoConfig). The DFU stage does
//! that with the small USB-only configuration it carries, compressed, as
//! AGM's DFU does with `FCB_AutoDecompress` (examples/dfu/src/dfu.c), so
//! that USB works whatever is, or is not, in the fabric's flash region
//! (docs/10-updates.md, "DFU mode").
//!
//! [`configure_compressed`] is dfu.c's sequence around the FCB, step for
//! step; the expansion itself is `fastprom_core::fcb::decompress`, fcb.c's
//! decoder with its indexes bounded.

use fastprom_core::fcb::{self as bitstream, LzwError, Scratch};
use thiserror::Error;

use crate::clock;
use crate::regs::{fcb, sys};

/// The fabric could not be configured.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum FcbError {
    #[error("the bitstream does not expand: {0}")]
    Bitstream(#[from] LzwError),
    #[error("the FCB did not take the bitstream: STAT {stat:#x}")]
    Rejected { stat: u32 },
}

/// Configure the fabric from a compressed Supra bitstream
/// (`fastprom_core::fcb`), as AGM's DFU stage does at startup.
///
/// The core is left on HSI with the PLL off: the PLL is part of the fabric
/// configuration (fcb.h FCB_PLL_Typedef, configured from the bitstream), so
/// [`clock::start_pll`] comes after this, never before. Whatever the
/// result, the APB clock gates and the HSI setting are as they were.
pub fn configure_compressed(src: &[u8], scratch: &mut Scratch) -> Result<(), FcbError> {
    // dfu.c main() keeps APB_CLKENABLE and MISC_CNTL, and enter_boot()
    // restores them once the fabric is loaded.
    let apb = sys::APB_CLKENABLE.read();
    let misc = sys::MISC_CNTL.read();

    // SYS_SetHSIConfig(0xf): HSI at its fastest, to shorten the ~100 k
    // words of decoding and FCB writes on the internal oscillator.
    sys::MISC_CNTL.write(sys::HSI_CONFIG_FASTEST);
    // SYS_SwitchHSIClock: off the PLL before the bitstream reconfigures it.
    // The DFU stage arrives here from reset, already on HSI; this makes it
    // so whatever ran before.
    clock::switch_to_hsi();
    // "Disable all peripheral clocks except for FCB to avoid any noise from
    // FPGA" (dfu.c): only the FCB's APB clock while the fabric changes under
    // the peripherals' pins.
    sys::APB_CLKENABLE.write(sys::APB_FCB0);

    // FCB_AutoDecompress: AUTO mode, every word to AUTO, then CTRL = 0 and
    // the status. fcb.c returns early on a bad stream without clearing
    // CTRL; here AUTO mode ends either way.
    fcb::CTRL.write(fcb::CTRL_AUTO);
    let expanded = bitstream::decompress(src, scratch, |word| fcb::AUTO.write(word));
    fcb::CTRL.write(0);
    let stat = fcb::STAT.read();

    sys::APB_CLKENABLE.write(apb);
    sys::MISC_CNTL.write(misc);

    expanded?;
    // fcb.c: RET_OK iff FCB_IsActive() && !FCB_IsAnyError().
    if stat & fcb::STAT_ACTIVE == 0 || stat & fcb::STAT_ERR_ALL != 0 {
        return Err(FcbError::Rejected { stat });
    }
    Ok(())
}
