//! The update interface: FastPROM's update protocol
//! (`fastprom_core::update::proto`) on a vendor interface.
//!
//! Class 0xFF/0x00/0x00, two bulk endpoints of 64 bytes (EP1 OUT and IN),
//! interface 0 in both firmwares (`docs/10-updates.md`, "USB device").
//! The protocol itself is the update core's [`Server`]; this class only
//! moves packets between it and the endpoints:
//!
//! - **Receive.** A packet waiting on the OUT endpoint is read and handed
//!   to the server only while the server has no response still to send
//!   ([`Service::ready`]); until then it stays in the controller, which
//!   NAKs the host. A host reads each response before its next request, so
//!   nothing waits there in normal use.
//! - **Transmit.** One packet armed on the IN endpoint at a time, as the
//!   server makes it; when the host has taken it, the next.
//! - **Control.** [`REQUEST_RESET`], a vendor request to this interface
//!   with no data, resets the server and empties both endpoints: a packet
//!   armed on IN is flushed and one waiting on OUT read and dropped. A bus
//!   reset does the same. The class never halts an endpoint itself; a halt
//!   the host sets and clears is the device stack's.

use fastprom_core::update::proto::{PACKET, REQUEST_RESET};
use fastprom_core::update::{Flash, Platform, Server};

use crate::bus::{Bus, Controller};
use crate::control::{Kind, Reply, Request};
use crate::eps::{MAX_PACKET, UPDATE_IN, UPDATE_OUT};

const _: () = assert!(PACKET == MAX_PACKET as usize);

/// What the class needs of the protocol: [`Server`]'s packet interface.
/// A trait so that the stack is tested with any protocol behind it.
pub trait Service {
    /// Whether a packet may be taken now.
    fn ready(&self) -> bool;
    /// One OUT packet.
    fn receive(&mut self, packet: &[u8]);
    /// The next IN packet into `out` and its length, without consuming it.
    fn next_packet(&self, out: &mut [u8; PACKET]) -> Option<usize>;
    /// That packet is armed.
    fn packet_armed(&mut self);
    /// The host took it.
    fn packet_sent(&mut self);
    /// Drop everything half done.
    fn reset(&mut self);
}

impl<B: Flash + Platform> Service for Server<'_, B> {
    fn ready(&self) -> bool {
        Server::ready(self)
    }
    fn receive(&mut self, packet: &[u8]) {
        Server::receive(self, packet);
    }
    fn next_packet(&self, out: &mut [u8; PACKET]) -> Option<usize> {
        Server::next_packet(self, out)
    }
    fn packet_armed(&mut self) {
        Server::packet_armed(self);
    }
    fn packet_sent(&mut self) {
        Server::packet_sent(self);
    }
    fn reset(&mut self) {
        Server::reset(self);
    }
}

const OUT_BIT: u16 = 1 << (UPDATE_OUT & 0x0F);
const IN_BIT: u16 = 1 << (UPDATE_IN & 0x0F);

/// The update interface, and the protocol behind it.
pub struct UpdateClass<S> {
    iface: u8,
    service: S,
    /// The bus reported a packet on the OUT endpoint not yet read.
    out_ready: bool,
    /// A packet is armed on the IN endpoint.
    in_flight: bool,
}

impl<S: Service> UpdateClass<S> {
    /// Interface `iface`, serving `service`.
    pub const fn new(iface: u8, service: S) -> Self {
        Self {
            iface,
            service,
            out_ready: false,
            in_flight: false,
        }
    }

    /// The interface number.
    pub const fn interface(&self) -> u8 {
        self.iface
    }

    /// The protocol.
    pub fn service(&mut self) -> &mut S {
        &mut self.service
    }

    /// Move everything along, once per turn of the main loop.
    pub fn run<B: Controller>(&mut self, bus: &mut Bus<B>) {
        self.pump_rx(bus);
        self.pump_tx(bus);
    }

    /// After a bus reset: the endpoints are empty, and so is the protocol.
    pub fn bus_reset(&mut self) {
        self.service.reset();
        self.out_ready = false;
        self.in_flight = false;
    }

    /// OUT halves with a packet waiting (bit n for endpoint n).
    pub fn endpoint_out<B: Controller>(&mut self, bus: &mut Bus<B>, out: u16) {
        if out & OUT_BIT != 0 {
            self.out_ready = true;
            self.pump_rx(bus);
            self.pump_tx(bus);
        }
    }

    /// IN halves whose packet the host took.
    pub fn endpoint_in_complete<B: Controller>(&mut self, bus: &mut Bus<B>, done: u16) {
        if done & IN_BIT == 0 || !self.in_flight {
            return;
        }
        self.in_flight = false;
        self.service.packet_sent();
        // The response may be done: a request waiting can come in now.
        self.pump_rx(bus);
        self.pump_tx(bus);
    }

    /// A class or vendor request to this interface.
    pub fn control<B: Controller>(&mut self, bus: &mut Bus<B>, req: &Request) -> Reply {
        if req.kind() == Kind::Vendor
            && !req.dir_in()
            && req.request == REQUEST_RESET
            && req.length == 0
        {
            self.reset(bus);
            Reply::Ok(0)
        } else {
            Reply::Stall
        }
    }

    /// The host's reset: the protocol, and both endpoints.
    fn reset<B: Controller>(&mut self, bus: &mut Bus<B>) {
        self.service.reset();
        if self.in_flight {
            bus.flush(UPDATE_IN);
            self.in_flight = false;
        }
        // A packet the controller holds, whether or not the bus has told
        // the class of it yet (it may have arrived in the same poll as the
        // reset's SETUP): read and dropped, which arms the endpoint again.
        self.out_ready = false;
        let _ = bus.read(UPDATE_OUT, &mut [0u8; PACKET]);
    }

    fn pump_rx<B: Controller>(&mut self, bus: &mut Bus<B>) {
        while self.out_ready && self.service.ready() {
            let mut packet = [0u8; PACKET];
            self.out_ready = false;
            if let Ok(n) = bus.read(UPDATE_OUT, &mut packet) {
                self.service.receive(packet.get(..n).unwrap_or(&[]));
            }
        }
    }

    fn pump_tx<B: Controller>(&mut self, bus: &mut Bus<B>) {
        if self.in_flight {
            return;
        }
        let mut packet = [0u8; PACKET];
        let Some(n) = self.service.next_packet(&mut packet) else {
            return;
        };
        if bus.write(UPDATE_IN, packet.get(..n).unwrap_or(&[])).is_ok() {
            self.service.packet_armed();
            self.in_flight = true;
        }
    }
}
