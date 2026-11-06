//! FastPROM's USB device above the registers: a small device stack, the
//! descriptors, the update interface and a CDC-ACM port (`docs/10-updates.md`, "USB
//! device"; `docs/11-usb-driver.md`).
//!
//! Nothing here touches the chip, so all of it is host-tested against a
//! model of the controller. The register-level half - the AG32's ChipIdea
//! controller and its DMA memory - is `fastprom_hal::usb::Ag32Usb`, a
//! [`bus::Controller`]; the unique ID for the serial number is
//! `fastprom_hal::flash::Ag32Flash::unique_id`.
//!
//! - [`bus`]: the endpoints over any [`bus::Controller`], polled.
//! - [`device`]: endpoint 0's control pipe, the standard requests, and
//!   dispatch to the functions. No `usb-device`: it cannot be linked
//!   without panic paths.
//! - [`desc`], [`msos`]: every descriptor, built at compile time.
//! - [`update`]: the update interface, carrying `fastprom_core`'s update
//!   protocol server.
//! - [`cdc`]: the CDC-ACM log port, application only.
//! - [`ids`]: VID, PID, strings, interface numbers, the serial number.
//! - [`eps`]: the endpoint plan - the one place the endpoint numbers are -
//!   and the controller's endpoint numbering.
//! - [`dqh`]: the controller's queue head and transfer descriptor layout.
//!
//! Wiring it up, in the application (the DFU stage passes
//! [`device::NoFunction`] instead of a [`cdc::CdcAcm`], which makes it the
//! DFU variant):
//!
//! ```text
//! let server = update::Server::new(&board, IDENTITY);
//! let serial = Serial::from_uid(flash.unique_id());
//! let mut usb = Device::new(Ag32Usb::take()?, serial, server, CdcAcm::new());
//! loop {
//!     usb.poll();
//!     usb.function().write(b"log");
//! }
//! ```
#![no_std]

pub mod bus;
pub mod cdc;
mod constbuf;
pub mod control;
pub mod desc;
pub mod device;
pub mod dqh;
pub mod eps;
pub mod fifo;
pub mod ids;
pub mod msos;
pub mod update;

pub use bus::{Bus, Controller, Fault};
pub use cdc::CdcAcm;
pub use device::{Device, Function, NoFunction};
pub use ids::{Serial, Variant};
pub use update::{Service, UpdateClass};
