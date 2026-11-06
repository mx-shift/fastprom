//! Moving the core from HSI to the PLL.
//!
//! A port of AGM's `SYS_SwitchPLLClock(SYS_HSE_BYPASS_ON)` and
//! `SYS_SetSclkAuto` (framework-agrv_sdk/src/system.c), with every wait
//! bounded: the SDK spins forever, which here would leave no report.
//!
//! The PLL matters beyond speed: in BRAM mode the block RAM's read port is
//! clocked by PLL output 2 (200 MHz, `PLLCLK2` in the .ve), so with the PLL
//! off the fabric cannot serve the socket at all. The AHB slave runs on
//! `sys_clock`, which follows the core's clock source, so the fabric can be
//! reached on HSI too.

use fastprom_core::FailureCode;
use thiserror::Error;

use crate::regs::{hz, sys};

/// The PLL could not be started; the core stays on HSI.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum ClockError {
    #[error(
        "the 12 MHz oscillator on OSC_IN (HSE, bypass) did not report ready after {polls} polls"
    )]
    HseNotReady { polls: u32 },
    #[error("the PLL did not lock after {restarts} restarts of {polls} polls each")]
    PllNotLocked { restarts: u32, polls: u32 },
}

impl FailureCode for ClockError {
    fn code(&self) -> u8 {
        match self {
            Self::HseNotReady { .. } => 60,
            Self::PllNotLocked { .. } => 61,
        }
    }
}

/// Polls of CLK_CNTL before giving up on HSE (several hundred ms on HSI).
const HSE_POLLS: u32 = 1_000_000;
/// The SDK restarts the PLL after this many polls without lock.
const PLL_POLLS: u32 = 5_000;
/// Restarts before giving up.
const PLL_RESTARTS: u32 = 200;

/// The core clock now running.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Clocks {
    core_hz: u32,
}

impl Clocks {
    /// What the core runs on out of reset.
    pub const fn hsi() -> Self {
        Self { core_hz: hz::HSI }
    }

    /// Core (and AHB) clock.
    pub const fn core_hz(self) -> u32 {
        self.core_hz
    }

    /// APB clock: the core clock over PBUS_DIVIDER + 1 (system.c
    /// SYS_GetPclkFreq). UART0's baud rate is set from it.
    pub fn pclk_hz(self) -> u32 {
        let div = (sys::PBUS_DIVIDER.read() & 0xFF) + 1;
        self.core_hz / div
    }
}

/// Run the core on HSI, with HSE and the PLL off (system.c
/// SYS_SwitchHSIClock): source to HSI first, then the others off, in that
/// order.
pub fn switch_to_hsi() -> Clocks {
    sys::CLK_CNTL.modify(sys::CLK_SOURCE_MASK, 0);
    sys::CLK_CNTL.modify(sys::CLK_PLL_ON | sys::CLK_HSE_ON, 0);
    Clocks::hsi()
}

/// Switch the core to the PLL, fed by the 12 MHz oscillator in HSE bypass.
pub fn start_pll() -> Result<Clocks, ClockError> {
    switch_to_hsi();

    // SYS_SetSclkAuto: divide the flash clock so it stays at or under
    // FLASH_MAX_FREQ (no division at 100 MHz).
    let div = (hz::PLL - 1) / hz::FLASH_MAX;
    sys::CLK_CNTL.modify(
        (sys::SCLK_DIV_MASK << sys::SCLK_DIV_HIGH_SHIFT)
            | (sys::SCLK_DIV_MASK << sys::SCLK_DIV_LOW_SHIFT),
        (div << sys::SCLK_DIV_HIGH_SHIFT) | (div << sys::SCLK_DIV_LOW_SHIFT),
    );

    // An oscillator, not a crystal, drives OSC_IN: HSE in bypass mode
    // (tools/boardmap.yaml, control.hse_in).
    let on = sys::CLK_CNTL.read() | sys::CLK_HSE_BYP | sys::CLK_HSE_ON | sys::CLK_PLL_ON;
    sys::CLK_CNTL.write(on);

    if !(0..HSE_POLLS).any(|_| sys::CLK_CNTL.read() & sys::CLK_HSE_RDY != 0) {
        return Err(ClockError::HseNotReady { polls: HSE_POLLS });
    }
    let mut locked = false;
    for _ in 0..PLL_RESTARTS {
        if (0..PLL_POLLS).any(|_| sys::CLK_CNTL.read() & sys::CLK_PLL_RDY != 0) {
            locked = true;
            break;
        }
        // As the SDK does: drop PLL_ON for one write and try again.
        sys::CLK_CNTL.write(on & !sys::CLK_PLL_ON);
        sys::CLK_CNTL.write(on);
    }
    if !locked {
        return Err(ClockError::PllNotLocked {
            restarts: PLL_RESTARTS,
            polls: PLL_POLLS,
        });
    }

    sys::CLK_CNTL.write(on | sys::CLK_SOURCE_PLL);
    Ok(Clocks { core_hz: hz::PLL })
}
