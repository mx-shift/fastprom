//! Which image to serve: one of the four the image-select switch can name.

use core::fmt;

/// An image number, 0 to 3: the two bits of the image-select switch.
///
/// The field is private, so a bank out of range cannot be spelled:
///
/// ```compile_fail,E0423
/// let _ = fastprom_core::Bank(7);
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Bank(u8);

impl Bank {
    /// How many banks two switch bits can name.
    pub const COUNT: usize = 4;

    /// Bank `index`, if it is one the switch can name (0 to 3).
    pub const fn new(index: u8) -> Option<Self> {
        if (index as usize) < Self::COUNT {
            Some(Self(index))
        } else {
            None
        }
    }

    /// The bank the two switch bits name, bit 1 high.
    pub const fn from_bits(bit1: bool, bit0: bool) -> Self {
        Self(((bit1 as u8) << 1) | bit0 as u8)
    }

    /// The bank as an index into the image table's directory.
    pub const fn index(self) -> usize {
        self.0 as usize
    }

    /// The bank number, 0 to 3.
    pub const fn number(self) -> u8 {
        self.0
    }
}

impl fmt::Display for Bank {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}
