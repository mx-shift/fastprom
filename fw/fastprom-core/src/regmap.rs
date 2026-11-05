//! The fabric's AHB slave registers (`hdl/fastprom_ahb_slave.v`).
//!
//! | offset | name   | access | meaning                                     |
//! |--------|--------|--------|---------------------------------------------|
//! | `0x00` | `CTRL` | R/W    | bit 0 READY: let the socket outputs drive   |
//! | `0x04` | `ADDR` | R/W    | load pointer, `AW` bits                     |
//! | `0x08` | `DATA` | W      | store one word at `ADDR`, then `ADDR += 1`  |
//! | `0x0C` | `ID`   | R      | `0x4650524d` (`'FPRM'`)                     |
//!
//! The slave decodes `haddr[3:2]` only, so the block repeats every 16 bytes
//! (`0x10` is `CTRL` again). Reads and writes are split into the two enums
//! below so that reading `DATA` or writing `ID` cannot be spelled.

/// What `ID` reads when the fabric is configured with FastPROM's logic.
pub const FABRIC_MAGIC: u32 = 0x4650_524D;
/// `CTRL` bit 0.
pub const CTRL_READY: u32 = 1 << 0;

/// A register that can be read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReadReg {
    Ctrl,
    Addr,
    Id,
}

/// A register that can be written.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WriteReg {
    Ctrl,
    Addr,
    Data,
}

impl ReadReg {
    /// Byte offset from the fabric's AHB base.
    pub const fn offset(self) -> usize {
        match self {
            Self::Ctrl => 0x0,
            Self::Addr => 0x4,
            Self::Id => 0xC,
        }
    }
}

impl WriteReg {
    /// Byte offset from the fabric's AHB base.
    pub const fn offset(self) -> usize {
        match self {
            Self::Ctrl => 0x0,
            Self::Addr => 0x4,
            Self::Data => 0x8,
        }
    }
}
