//! The board firmware's errors, around fastprom-core's.

use fastprom_core::{BootError, FabricError, FailureCode};
use fastprom_hal::clock::ClockError;
use thiserror::Error;

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

impl FailureCode for FwError {
    fn code(&self) -> u8 {
        match self {
            Self::Claim(e) => e.code(),
            Self::Clock(e) => e.code(),
            Self::Boot(e) => e.code(),
        }
    }
}
