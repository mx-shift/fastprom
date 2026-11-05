//! The power-up sequence after the outputs are released.

use crate::error::{BootError, LoadError};
use crate::fabric::Fabric;
use crate::loader::{Loaded, Released};
use crate::select::{decide, SwitchSample};
use crate::table::{ImageTable, Width};

/// What was loaded, for the boot report.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Report<'a> {
    /// The verified load.
    pub loaded: Loaded,
    /// The image's source file name.
    pub name: &'a str,
    /// The part the table was built for.
    pub part: &'a str,
    /// Bits per word.
    pub width: Width,
    /// Images in the table.
    pub count: u32,
}

/// Decode the switch, parse the table and load the selected image.
///
/// `released` is the fabric with its outputs already down
/// ([`Released::claim`]), which the board firmware does first of all, so
/// that every failure here leaves the socket high-impedance.
pub fn boot<'a, F: Fabric>(
    released: Released<'_, F>,
    table: &'a [u8],
    samples: &[SwitchSample],
) -> Result<Report<'a>, BootError> {
    let bank = decide(samples)?;
    let table = ImageTable::parse(table)?;
    let image = table.image(bank).ok_or(LoadError::BankNotInTable {
        bank,
        count: table.count().get(),
    })?;
    let loaded = released.load(&image)?;
    Ok(Report {
        loaded,
        name: image.name(),
        part: table.part(),
        width: table.width(),
        count: table.count().get(),
    })
}
