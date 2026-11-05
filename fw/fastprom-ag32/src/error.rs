//! The board firmware's errors, around fastprom-core's.

use fastprom_core::{BootError, FabricError, FailureCode};
use thiserror::Error;

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

/// Why the socket is not being served.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum FwError {
    #[error("releasing the socket outputs")]
    Claim(#[source] FabricError),
    #[error("starting the clocks (the block RAM's read port runs on the PLL)")]
    Clock(#[from] ClockError),
    #[error(transparent)]
    Boot(#[from] BootError),
}

impl FailureCode for ClockError {
    fn code(&self) -> u8 {
        match self {
            Self::HseNotReady { .. } => 60,
            Self::PllNotLocked { .. } => 61,
        }
    }
}

impl FailureCode for FwError {
    fn code(&self) -> u8 {
        match self {
            Self::Claim(e) => e.code(),
            Self::Clock(e) => e.code(),
            Self::Boot(e) => e.code(),
        }
    }
}
