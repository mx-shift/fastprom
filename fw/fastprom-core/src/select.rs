//! The image-select switch, read on the UART0 pins.
//!
//! SW1 reaches UART0 TX (PIN_20) and RX (PIN_21) through 10 k to 3V3; a
//! 100 k pull-down on each holds it low when open
//! (`DESIGN_NOTES.md`, "Image select on the UART pins"). Closed is 1.
//! Switch 1 (pins 1-4) on RX is bit 0, switch 2 (pins 2-3) on TX is bit 1,
//! so the numbers printed on the switch name the bits
//! (`docs/06-build-and-flash.md`, "Programming").
//!
//! The firmware samples both pins several times before UART0 takes them
//! and serves an image only if every sample agrees: a switch caught moving,
//! or a pin with something else driving it, is a reason to stay off the
//! bus rather than to guess.

use crate::bank::Bank;
use crate::error::SelectError;

/// How many times the board firmware samples the switch.
pub const SAMPLES: usize = 16;

/// The two switch pins at one instant; `true` is high, switch closed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct SwitchSample {
    /// UART0 TX, PIN_20: switch 2, bit 1.
    pub tx: bool,
    /// UART0 RX, PIN_21: switch 1, bit 0.
    pub rx: bool,
}

impl SwitchSample {
    /// The bank these two levels name.
    pub const fn bank(self) -> Bank {
        Bank::from_bits(self.tx, self.rx)
    }
}

/// The bank every sample names, or why there is not one.
pub fn decide(samples: &[SwitchSample]) -> Result<Bank, SelectError> {
    let (first, rest) = samples.split_first().ok_or(SelectError::NoSamples)?;
    let bank = first.bank();
    match rest.iter().enumerate().find(|(_, s)| s.bank() != bank) {
        None => Ok(bank),
        Some((i, s)) => Err(SelectError::Unstable {
            first: bank,
            index: i + 1,
            found: s.bank(),
        }),
    }
}
