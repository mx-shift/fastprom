//! A CDC-ACM serial port, for the boot report and the log
//! (`docs/06-build-and-flash.md`, "The boot report"). Application only.
//!
//! Interfaces 1 (communications, with the notification endpoint) and 2
//! (data), tied together by an interface association descriptor
//! (`desc.rs`). The port is written to through a FIFO which
//! [`CdcAcm::run`] drains a packet at a time; a transfer that ends on a
//! full packet is closed with a zero-length one, so a host reading more
//! than a packet at a time (Linux reads two) gets the data at once. Nothing
//! is ever sent on the notification endpoint.

use crate::bus::{Bus, Controller};
use crate::control::{Kind, Recipient, Reply, Request};
use crate::device::Function;
use crate::eps::{CDC_DATA_IN, CDC_DATA_OUT, MAX_PACKET};
use crate::fifo::Fifo;
use crate::ids::{CDC_COMM_INTERFACE, CDC_DATA_INTERFACE};

/// SET_LINE_CODING.
pub const SET_LINE_CODING: u8 = 0x20;
/// GET_LINE_CODING.
pub const GET_LINE_CODING: u8 = 0x21;
/// SET_CONTROL_LINE_STATE.
pub const SET_CONTROL_LINE_STATE: u8 = 0x22;
/// SEND_BREAK.
pub const SEND_BREAK: u8 = 0x23;

/// The transmit FIFO: a boot report's worth.
pub const TX_CAPACITY: usize = 512;
/// The receive FIFO.
pub const RX_CAPACITY: usize = 128;

const PACKET: usize = MAX_PACKET as usize;
const OUT_BIT: u16 = 1 << (CDC_DATA_OUT & 0x0F);
const IN_BIT: u16 = 1 << (CDC_DATA_IN & 0x0F);

/// The line coding until the host sets one: 115200 baud, 1 stop bit, no
/// parity, 8 data bits (the UART's boot report). It means nothing to a USB
/// port; it is stored and given back.
pub const DEFAULT_LINE_CODING: [u8; 7] = [0x00, 0xC2, 0x01, 0x00, 0, 0, 8];

/// The serial port.
#[derive(Debug, Clone)]
pub struct CdcAcm {
    line_coding: [u8; 7],
    dtr: bool,
    rts: bool,
    tx: Fifo<TX_CAPACITY>,
    rx: Fifo<RX_CAPACITY>,
    in_flight: bool,
    /// The last packet sent was full and nothing followed it yet.
    zlp_owed: bool,
    out_ready: bool,
}

impl Default for CdcAcm {
    fn default() -> Self {
        Self::new()
    }
}

impl CdcAcm {
    /// Closed, empty.
    pub const fn new() -> Self {
        Self {
            line_coding: DEFAULT_LINE_CODING,
            dtr: false,
            rts: false,
            tx: Fifo::new(),
            rx: Fifo::new(),
            in_flight: false,
            zlp_owed: false,
            out_ready: false,
        }
    }

    /// Whether the host has the port open (DTR set).
    pub fn dtr(&self) -> bool {
        self.dtr
    }

    /// RTS as the host last set it.
    pub fn rts(&self) -> bool {
        self.rts
    }

    /// The line coding the host last set.
    pub fn line_coding(&self) -> [u8; 7] {
        self.line_coding
    }

    /// Queue bytes for the host; how many fitted. They go on the next
    /// [`Function::run`].
    pub fn write(&mut self, data: &[u8]) -> usize {
        self.tx.push(data)
    }

    /// Room in the transmit queue.
    pub fn write_room(&self) -> usize {
        self.tx.room()
    }

    /// Bytes from the host; how many.
    pub fn read(&mut self, out: &mut [u8]) -> usize {
        self.rx.pop(out)
    }

    fn pump_tx<B: Controller>(&mut self, bus: &mut Bus<B>) {
        if self.in_flight {
            return;
        }
        let mut packet = [0u8; PACKET];
        let n = self.tx.peek(&mut packet);
        if n == 0 && !self.zlp_owed {
            return;
        }
        if let Ok(sent) = bus.write(CDC_DATA_IN, packet.get(..n).unwrap_or(&[])) {
            self.tx.consume(sent);
            self.zlp_owed = sent == PACKET;
            self.in_flight = true;
        }
    }

    fn pump_rx<B: Controller>(&mut self, bus: &mut Bus<B>) {
        if !self.out_ready || self.rx.room() < PACKET {
            return;
        }
        let mut packet = [0u8; PACKET];
        if let Ok(n) = bus.read(CDC_DATA_OUT, &mut packet) {
            self.rx.push(packet.get(..n).unwrap_or(&[]));
        }
        self.out_ready = false;
    }
}

impl<B: Controller> Function<B> for CdcAcm {
    const PRESENT: bool = true;

    fn owns_interface(&self, iface: u8) -> bool {
        iface == CDC_COMM_INTERFACE || iface == CDC_DATA_INTERFACE
    }

    /// The line state starts over, and what was armed is gone with the
    /// reset. What is queued for the host is kept: the boot report is
    /// queued before the host has even enumerated the device.
    fn reset(&mut self) {
        self.line_coding = DEFAULT_LINE_CODING;
        self.dtr = false;
        self.rts = false;
        self.rx.clear();
        self.in_flight = false;
        self.zlp_owed = false;
        self.out_ready = false;
    }

    fn run(&mut self, bus: &mut Bus<B>) {
        self.pump_rx(bus);
        self.pump_tx(bus);
    }

    fn endpoint_out(&mut self, bus: &mut Bus<B>, out: u16) {
        if out & OUT_BIT != 0 {
            self.out_ready = true;
            self.pump_rx(bus);
        }
    }

    fn endpoint_in_complete(&mut self, bus: &mut Bus<B>, done: u16) {
        if done & IN_BIT != 0 {
            self.in_flight = false;
            self.pump_tx(bus);
        }
    }

    fn control(&mut self, _bus: &mut Bus<B>, req: &Request, data: &[u8], buf: &mut [u8]) -> Reply {
        if req.kind() != Kind::Class
            || req.recipient() != Recipient::Interface
            || req.target() != CDC_COMM_INTERFACE
        {
            return Reply::Stall;
        }
        match (req.request, req.dir_in()) {
            (SET_LINE_CODING, false) => match data.get(..7) {
                Some(coding) => {
                    self.line_coding.copy_from_slice(coding);
                    Reply::Ok(0)
                }
                None => Reply::Stall,
            },
            (SET_CONTROL_LINE_STATE, false) => {
                self.dtr = req.value & 0x1 != 0;
                self.rts = req.value & 0x2 != 0;
                Reply::Ok(0)
            }
            (SEND_BREAK, false) => Reply::Ok(0),
            (GET_LINE_CODING, true) => match buf.get_mut(..7) {
                Some(dst) => {
                    dst.copy_from_slice(&self.line_coding);
                    Reply::Ok(7)
                }
                None => Reply::Stall,
            },
            _ => Reply::Stall,
        }
    }
}
