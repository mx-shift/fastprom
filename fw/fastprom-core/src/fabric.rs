//! Access to the fabric's AHB slave.

use crate::regmap::{ReadReg, WriteReg};

/// One bus transfer to the fabric's register block per call.
///
/// The board crate implements this on the AG32's AHB window at
/// `0x60000000`; the host tests implement it with a cycle-level model of
/// `hdl/fastprom_ahb_slave.v`. Implementations must not merge, reorder or
/// drop calls: the load sequence relies on each call being exactly one
/// transfer, in order (see [`crate::loader`] on the read-after-write
/// hazard).
pub trait Fabric {
    /// Read a register.
    fn read(&mut self, reg: ReadReg) -> u32;
    /// Write a register.
    fn write(&mut self, reg: WriteReg, value: u32);
}
