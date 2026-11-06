//! The host side of FastPROM's USB updates (`docs/10-updates.md`), for the
//! web page (through `fastprom-web`, compiled to WebAssembly) and for tests.
//!
//! - [`transport`]: what the client needs from USB: two bulk endpoints and
//!   the vendor interface's reset request. WebUSB in the page, the [`sim`]
//!   device in tests.
//! - [`client`]: one request of FastPROM's update protocol at a time
//!   (`fastprom_core::update::proto`, the device's own module), with the
//!   protocol's recovery (reset and send again).
//! - [`info`]: FastPROM's `INFO` answer, decoded.
//! - [`rom`]: ROM files, read and checked against the part's organisation
//!   the way `tools/mkimagetable.py` reads them.
//! - [`table`]: the image table, built byte for byte as
//!   `tools/mkimagetable.py` builds it.
//! - [`bundle`]: the update bundle published with the page: its manifest,
//!   and the checks on the files it names.
//! - [`update`]: the procedures the page runs: connect, write the image
//!   table, LOAD, reboot into DFU and back, firmware and fabric updates.
//! - [`serial`], [`an3155`], [`recovery`]: UART recovery of a blank or
//!   bricked board through the mask ROM's serial loader (Web Serial in the
//!   page): the byte stream it needs, the AN3155 client, and the procedure.
//! - [`sim`]: a simulated board: `fastprom-core`'s update core over a model
//!   of the AG32's NOR flash, with its boot path (DFU stage check,
//!   `mscratch`), reboots and power cuts; and, with BOOT0 bridged, a model
//!   of the mask ROM's serial loader and `fastprom-core`'s recovery loader.
//!
//! The formats themselves (table, header, manifest, `INFO`, flash map) are
//! `fastprom-core`'s, used here as the firmware uses them, so the page and
//! the firmware cannot disagree about them.

pub mod an3155;
pub mod bundle;
pub mod client;
pub mod info;
pub mod recovery;
pub mod rom;
pub mod serial;
pub mod sim;
pub mod table;
pub mod transport;
pub mod update;

pub use client::{Client, Error};
pub use transport::{Transport, TransportError};
