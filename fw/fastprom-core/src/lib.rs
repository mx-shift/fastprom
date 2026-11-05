//! FastPROM firmware logic, free of any hardware.
//!
//! At power-up the AG32 firmware has one job: put the image the user picked
//! on the image-select switch into the fabric's block RAM, prove it got
//! there intact, and only then let the fabric drive the host's bus. This
//! crate is all of that except the registers:
//!
//! - [`crc32`]: CRC-32/ISO-HDLC, the `zlib.crc32` that `tools/mkimagetable.py`
//!   bakes into the image table.
//! - [`table`]: the binary image table in flash, parsed once into checked
//!   types.
//! - [`select`]: the two switch bits on the UART0 pins, decoded to a [`Bank`].
//! - [`loader`]: the load sequence against the [`Fabric`] trait, a model of
//!   `hdl/fastprom_ahb_slave.v`'s registers. [`Released`] is the typestate
//!   that proves the socket outputs are down before anything is written.
//! - [`boot`]: the whole power-up sequence in order.
//!
//! The board crate (`fastprom-ag32`) supplies the switch samples, the flash
//! bytes and a [`Fabric`] on the real AHB window, and reports the result.
#![no_std]

pub mod bank;
pub mod baud;
pub mod boot;
pub mod crc32;
pub mod error;
pub mod fabric;
pub mod layout;
pub mod loader;
pub mod regmap;
pub mod select;
pub mod table;

pub use bank::Bank;
pub use boot::{boot, Report};
pub use error::{
    BootError, FabricError, FailureCode, ImageError, LoadError, SelectError, TableError, TextField,
};
pub use fabric::Fabric;
pub use loader::{Loaded, Released};
pub use regmap::{ReadReg, WriteReg};
pub use select::SwitchSample;
pub use table::{Image, ImageCount, ImageTable, Width};
