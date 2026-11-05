//! Loading an image into the fabric's block RAM.
//!
//! The order is the point. A bipolar PROM socket is usually on a bus with
//! other devices, and driving a wrong value onto it is worse than driving
//! nothing, so:
//!
//! 1. [`Released::claim`]: the fabric must answer with FastPROM's ID, then
//!    READY is cleared and read back clear, before anything else. A warm
//!    reset does not reconfigure the fabric, so READY can still be set from
//!    the last boot.
//! 2. The fabric's load pointer is probed (write all ones, read back the
//!    `AW`-bit mask), which gives the block RAM's depth: an image table
//!    built for another organisation is refused instead of half-filling it.
//! 3. The image's CRC is checked (and every word against the width) before
//!    a single word is written.
//! 4. Each word is written to `DATA` and the pointer read back: it must have
//!    advanced by exactly one. The HDL has no data read-back port, so this
//!    is the most the protocol can verify: every word was accepted, in
//!    order, at the address it was meant for.
//! 5. READY is set and read back set. Only then do the outputs drive.
//!
//! Any failure returns an error with READY still clear.
//!
//! # The read-after-write hazard
//!
//! `fastprom_ahb_slave.v` captures read data in the address phase, and a
//! write takes effect at the end of its data phase. On a pipelined AHB
//! master the address phase of a read can coincide with the data phase of
//! the write before it, and then the read returns the value from *before*
//! the write. So a register is never read straight after a write: an `ID`
//! read sits between them (it cannot be stale, and it re-checks that the
//! fabric is still there). `tests/loader.rs` shows the C firmware's
//! sequence failing on a pipelined bus, and `sim/tb_fw_load.v` replays this
//! sequence against the HDL itself, back to back and with idle cycles.

use crate::bank::Bank;
use crate::crc32::Crc32;
use crate::error::{FabricError, ImageError, LoadError};
use crate::fabric::Fabric;
use crate::regmap::{ReadReg, WriteReg, CTRL_READY, FABRIC_MAGIC};
use crate::table::Image;

/// A fabric that answered with FastPROM's ID and whose socket outputs are
/// released (READY read back clear). Loading needs one, so nothing can be
/// written before the outputs are down.
#[derive(Debug)]
pub struct Released<'f, F: Fabric> {
    fabric: &'f mut F,
    /// The load pointer's `AW`-bit mask: block RAM depth minus one.
    mask: u32,
}

/// Proof of a verified load: only [`Released::load`] makes one, after READY
/// read back set.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Loaded {
    bank: Bank,
    crc32: u32,
    depth: u32,
}

impl Loaded {
    /// The bank that is now being served.
    pub const fn bank(&self) -> Bank {
        self.bank
    }

    /// Its CRC-32, as computed and as the table records it.
    pub const fn crc32(&self) -> u32 {
        self.crc32
    }

    /// Words loaded.
    pub const fn depth(&self) -> u32 {
        self.depth
    }
}

impl<'f, F: Fabric> Released<'f, F> {
    /// Check the fabric is FastPROM's, release the socket outputs and probe
    /// the block RAM's depth.
    pub fn claim(fabric: &'f mut F) -> Result<Self, FabricError> {
        let found = fabric.read(ReadReg::Id);
        if found != FABRIC_MAGIC {
            return Err(FabricError::NoFabric { found });
        }

        fabric.write(WriteReg::Ctrl, 0);
        let ctrl = read_after_write(fabric, ReadReg::Ctrl)?;
        if ctrl & CTRL_READY != 0 {
            return Err(FabricError::ReleaseFailed { ctrl });
        }

        // ADDR keeps the low AW bits of what is written and reads the rest
        // as zero, so all ones reads back as depth - 1.
        fabric.write(WriteReg::Addr, u32::MAX);
        let mask = read_after_write(fabric, ReadReg::Addr)?;
        if mask == 0 || mask == u32::MAX || !mask.wrapping_add(1).is_power_of_two() {
            return Err(FabricError::PointerProbe { found: mask });
        }
        Ok(Self { fabric, mask })
    }

    /// Words of block RAM behind the load port.
    pub const fn depth(&self) -> u32 {
        // claim() refused mask == u32::MAX.
        self.mask + 1
    }

    /// Verify `image`, write it into the block RAM and set READY.
    ///
    /// Consumes the fabric: after an error the outputs are released and
    /// stay that way, and after success there is nothing left to do.
    pub fn load(mut self, image: &Image<'_>) -> Result<Loaded, LoadError> {
        let bank = image.bank();
        let crc32 = verify(image).map_err(|source| LoadError::Image { bank, source })?;
        let fabric_err = |source| LoadError::Fabric { bank, source };

        let depth = u32::try_from(image.words().len()).unwrap_or(u32::MAX);
        if depth != self.depth() {
            return Err(fabric_err(FabricError::DepthMismatch {
                fabric: self.depth(),
                image: depth,
            }));
        }
        self.write_words(image.words()).map_err(fabric_err)?;
        self.set_ready().map_err(fabric_err)?;
        Ok(Loaded { bank, crc32, depth })
    }

    fn write_words(&mut self, words: &[u8]) -> Result<(), FabricError> {
        self.fabric.write(WriteReg::Addr, 0);
        let found = read_after_write(self.fabric, ReadReg::Addr)?;
        if found != 0 {
            return Err(FabricError::PointerReset { found });
        }
        let mut expected = 0u32;
        for (index, &word) in words.iter().enumerate() {
            self.fabric.write(WriteReg::Data, u32::from(word));
            expected = expected.wrapping_add(1) & self.mask;
            let found = read_after_write(self.fabric, ReadReg::Addr)?;
            if found != expected {
                return Err(FabricError::Pointer {
                    index,
                    expected,
                    found,
                });
            }
        }
        Ok(())
    }

    fn set_ready(self) -> Result<(), FabricError> {
        self.fabric.write(WriteReg::Ctrl, CTRL_READY);
        let ctrl = read_after_write(self.fabric, ReadReg::Ctrl)?;
        if ctrl & CTRL_READY == 0 {
            // Best effort: make sure it really is released.
            self.fabric.write(WriteReg::Ctrl, 0);
            return Err(FabricError::ReadyNotSet { ctrl });
        }
        Ok(())
    }
}

/// Read `reg` after a write without tripping the read-after-write hazard
/// (see the module docs): an `ID` read goes between, and must still match.
fn read_after_write<F: Fabric>(fabric: &mut F, reg: ReadReg) -> Result<u32, FabricError> {
    let found = fabric.read(ReadReg::Id);
    if found != FABRIC_MAGIC {
        return Err(FabricError::Lost { found });
    }
    Ok(fabric.read(reg))
}

/// Check an image against its table entry: the CRC over every word, and no
/// word wider than the table's width. One pass over the data; returns the
/// CRC.
pub fn verify(image: &Image<'_>) -> Result<u32, ImageError> {
    let bank = image.bank();
    let mask = image.width().mask();
    let mut crc = Crc32::new();
    let mut too_wide = None;
    for (index, &value) in image.words().iter().enumerate() {
        crc.byte(value);
        if value & !mask != 0 && too_wide.is_none() {
            too_wide = Some((index, value));
        }
    }
    let computed = crc.finish();
    // A CRC mismatch is the likelier cause (damaged flash), so it wins.
    if computed != image.crc32() {
        return Err(ImageError::Crc {
            bank,
            computed,
            stored: image.crc32(),
        });
    }
    if let Some((index, value)) = too_wide {
        return Err(ImageError::WordTooWide {
            bank,
            index,
            value,
            width: image.width().bits(),
        });
    }
    Ok(computed)
}
