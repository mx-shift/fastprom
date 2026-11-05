//! The fabric's AHB slave on the AG32's AHB window.

use fastprom_core::regmap::FABRIC_MAGIC;
use fastprom_core::{Fabric, ReadReg, WriteReg};

use crate::mmio::Reg;
use crate::regs::fabric::BASE;

/// The fabric's register block at 0x6000_0000 (`hdl/fastprom_ahb_slave.v`).
/// One volatile access per call, so each call is one AHB transfer, in
/// program order.
#[derive(Debug)]
pub struct Ag32Fabric {
    _owned: (),
}

impl Ag32Fabric {
    /// The fabric. Only `main` makes one.
    pub const fn new() -> Self {
        Self { _owned: () }
    }
}

fn reg(offset: usize) -> Reg {
    // SAFETY: offsets come from ReadReg/WriteReg (0x0 to 0xC), inside the
    // fabric's AHB window; every address there reads and writes as a u32.
    // An unconfigured fabric reads as zero (it does not stall the bus).
    unsafe { Reg::new(BASE + offset) }
}

impl Fabric for Ag32Fabric {
    fn read(&mut self, r: ReadReg) -> u32 {
        reg(r.offset()).read()
    }

    fn write(&mut self, r: WriteReg, value: u32) {
        reg(r.offset()).write(value);
    }
}

/// Take the socket outputs down if the fabric is ours: for the panic and
/// trap paths, which must never leave READY set.
pub fn force_release() {
    if reg(ReadReg::Id.offset()).read() == FABRIC_MAGIC {
        reg(WriteReg::Ctrl.offset()).write(0);
    }
}
