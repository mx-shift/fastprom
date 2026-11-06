//! What the client needs from USB: the update interface's two bulk
//! endpoints and its one control request.
//!
//! In the page this is WebUSB (`fastprom-web`, over `web/js/usb.js`), in
//! tests the simulated device ([`crate::sim`]). The methods are async
//! because WebUSB's are; the simulated device answers at once.

use core::future::Future;

/// Why a transfer did not happen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TransportError {
    /// The device is no longer there: unplugged, rebooting, power lost.
    Gone,
    /// Anything else, as the USB stack put it (a timeout, a halted
    /// endpoint).
    Other(String),
}

impl core::fmt::Display for TransportError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Gone => f.write_str("the device disconnected"),
            Self::Other(s) => f.write_str(s),
        }
    }
}

/// The update interface, found and claimed.
///
/// Written with `impl Future` rather than `async fn` so that the trait says
/// nothing about `Send`, which WebUSB's futures are not.
pub trait Transport {
    /// One bulk OUT transfer (split into packets by the USB stack); an
    /// empty `data` is a zero-length packet.
    fn bulk_out(&mut self, data: &[u8]) -> impl Future<Output = Result<(), TransportError>>;

    /// One bulk IN transfer of at most `len` bytes: it ends at `len` or at
    /// a short packet.
    fn bulk_in(&mut self, len: usize) -> impl Future<Output = Result<Vec<u8>, TransportError>>;

    /// The protocol's reset (`fastprom_core::update::proto::REQUEST_RESET`):
    /// a vendor control request to the interface, host to device, no data.
    fn reset(&mut self) -> impl Future<Output = Result<(), TransportError>>;
}

/// The update interface as a host finds it in a configuration descriptor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UpdateInterface {
    pub interface: u8,
    /// Endpoint addresses (the IN one with bit 7 set).
    pub ep_out: u8,
    pub ep_in: u8,
}

type Partial = Option<(u8, Option<u8>, Option<u8>)>;

fn complete(c: Partial) -> Option<UpdateInterface> {
    match c {
        Some((interface, Some(ep_out), Some(ep_in))) => Some(UpdateInterface {
            interface,
            ep_out,
            ep_in,
        }),
        _ => None,
    }
}

/// Find the update interface in a configuration descriptor: the first
/// vendor interface (`FF/00/00`) with a bulk OUT and a bulk IN endpoint.
/// It is interface 0 in both firmwares, but the number is not assumed.
/// `web/js/usb.js` applies the same rule to WebUSB's parsed configuration.
pub fn find_update_interface(config: &[u8]) -> Option<UpdateInterface> {
    let mut at = 0;
    let mut current: Partial = None;
    while let (Some(&len), Some(&kind)) = (config.get(at), config.get(at + 1)) {
        if len < 2 {
            return None;
        }
        let d = config.get(at..at + usize::from(len))?;
        match (kind, d) {
            (4, [_, _, num, _, _, class, sub, proto, ..]) => {
                if let Some(found) = complete(current) {
                    return Some(found);
                }
                current = ([*class, *sub, *proto] == [0xFF, 0, 0]).then_some((*num, None, None));
            }
            (5, [_, _, addr, attr, ..]) if attr & 3 == 2 => {
                if let Some((_, o, i)) = current.as_mut() {
                    if addr & 0x80 != 0 {
                        *i = Some(*addr);
                    } else {
                        *o = Some(*addr);
                    }
                }
            }
            _ => {}
        }
        at += usize::from(len);
    }
    complete(current)
}
