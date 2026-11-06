//! What UART recovery needs from a serial port: a byte stream with
//! timeouts.
//!
//! In the page this is Web Serial (`fastprom-web`, over `web/js/serial.js`)
//! on a USB-UART adapter at 115200 8E1; in tests the simulated board's UART
//! ([`crate::sim::SimSerial`]). The port is opened, and its framing set,
//! by whoever makes it: recovery never changes it.

use core::future::Future;

/// Why the port could not be used.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SerialError {
    /// The port went away: the adapter was unplugged, or the page closed it.
    Gone,
    /// Anything else, as the platform put it.
    Other(String),
}

impl core::fmt::Display for SerialError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Gone => f.write_str("the serial port went away (the adapter was unplugged?)"),
            Self::Other(s) => f.write_str(s),
        }
    }
}

/// A serial port, open, at the framing recovery uses.
///
/// Written with `impl Future` rather than `async fn` so that the trait says
/// nothing about `Send`, which Web Serial's futures are not.
pub trait SerialPort {
    /// Send `data`.
    fn write(&mut self, data: &[u8]) -> impl Future<Output = Result<(), SerialError>>;

    /// Up to `len` bytes: as soon as that many have arrived, or what has
    /// arrived when `timeout_ms` has passed since the call (possibly
    /// nothing).
    fn read(
        &mut self,
        len: usize,
        timeout_ms: u32,
    ) -> impl Future<Output = Result<Vec<u8>, SerialError>>;

    /// Drop whatever has been received and not read.
    fn discard(&mut self) -> impl Future<Output = Result<(), SerialError>>;

    /// Wait `ms` milliseconds.
    fn sleep(&mut self, ms: u32) -> impl Future<Output = ()>;
}
