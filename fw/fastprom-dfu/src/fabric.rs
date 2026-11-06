//! The DFU stage's own fabric configuration: the clocks and USB0, no other
//! fabric I/O (docs/10-updates.md, "DFU mode").
//!
//! A compressed Supra bitstream from `tools/ag32/build_dfu_fabric.sh`,
//! committed in `fabric/` because rebuilding it needs AGM's SDK; build.rs
//! refuses to build if it no longer matches its manifest or the inputs it
//! was built from. In it, no package pin has a pad cell but the three clock
//! inputs (`check_dfu_fabric.py`'s evidence, docs/06-build-and-flash.md), so
//! while it is loaded the socket's data pins are undriven and the bus
//! buffers' enable is left to its pull-up, R21: the socket is
//! high-impedance for as long as DFU mode runs.

/// `fabric/dfu_usb.bin`, for `fastprom_hal::fcb::configure_compressed`.
pub static BITSTREAM: &[u8] = include_bytes!("../fabric/dfu_usb.bin");
