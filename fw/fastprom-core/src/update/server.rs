//! The device's side of the update protocol ([`super::proto`]): requests
//! gathered from bulk OUT packets, answered one response at a time in bulk
//! IN packets, the commands run against the board.
//!
//! No USB in here: the USB class (`fastprom-usb`'s `update.rs`) hands it
//! each OUT packet and asks it for each IN packet, so the whole protocol is
//! host-tested.
//!
//! - [`Server::receive`]: one OUT packet. A packet shorter than
//!   [`PACKET`] ends the transfer, and with it the request; the request is
//!   checked and run then, and its response queued.
//! - [`Server::next_packet`], [`Server::packet_armed`],
//!   [`Server::packet_sent`]: the response, a packet at a time, then a
//!   zero-length packet if it filled its last one. A `READ`'s bytes come
//!   from the flash as each packet is made, so no response is held whole.
//! - [`Server::ready`]: whether a packet may be taken. Not while a
//!   response is still to go: the host reads each response before its next
//!   request, and the class leaves a packet that comes early in the
//!   controller until then.
//! - [`Server::reset`]: the host's [`super::proto::REQUEST_RESET`], or a
//!   bus reset: everything half done is dropped.
//!
//! `REBOOT` reboots once the host has taken the last packet of its
//! response, so the host knows it was accepted.

use crate::crc32::Crc32;

use super::commit::{commit, feed, CommitTarget};
use super::info::{self, INFO_LEN};
use super::policy::Policy;
use super::proto::{
    self, u32_at, Command, Header, HEADER_LEN, PACKET, READ_MAX, REQUEST_MAX, TARGET_APPLICATION,
    TARGET_DFU, VERSION, WRITE_LEN,
};
use super::{BootTarget, Flash, Identity, Mode, Platform, Status, PAGE_LEN};

/// Where a response's payload comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Body {
    Empty,
    /// The `INFO` answer built when the request ran.
    Info,
    /// Flash from this address (`READ`), the header's length of it.
    Flash(u32),
}

/// A response being sent.
#[derive(Debug, Clone, Copy)]
struct Tx {
    head: [u8; HEADER_LEN],
    body: Body,
    /// Header and payload.
    total: usize,
    sent: usize,
    /// A zero-length packet is still to follow.
    zlp: bool,
    /// A packet is armed and the host has not taken it.
    in_flight: bool,
    /// Where to reboot once it has gone.
    reboot: Option<BootTarget>,
}

/// The update protocol's device, for `id.mode`, on `board`.
pub struct Server<'a, B> {
    board: &'a B,
    id: Identity,
    rx: [u8; REQUEST_MAX],
    /// Bytes of the transfer so far, stored or not.
    rx_count: usize,
    tx: Option<Tx>,
    info: [u8; INFO_LEN],
}

impl<'a, B: Flash + Platform> Server<'a, B> {
    /// Idle: nothing received, nothing to send.
    pub const fn new(board: &'a B, id: Identity) -> Self {
        Self {
            board,
            id,
            rx: [0; REQUEST_MAX],
            rx_count: 0,
            tx: None,
            info: [0; INFO_LEN],
        }
    }

    /// Who is serving.
    pub const fn identity(&self) -> Identity {
        self.id
    }

    /// Whether it takes a packet now: no response is waiting to go.
    pub const fn ready(&self) -> bool {
        self.tx.is_none()
    }

    /// Drop the request being received and the response being sent.
    pub fn reset(&mut self) {
        self.rx_count = 0;
        self.tx = None;
    }

    /// One packet from the bulk OUT endpoint. Dropped if a response is
    /// still to go ([`Server::ready`]).
    pub fn receive(&mut self, packet: &[u8]) {
        if self.tx.is_some() {
            return;
        }
        let at = self.rx_count;
        if let Some(dst) = at
            .checked_add(packet.len())
            .and_then(|end| self.rx.get_mut(at..end))
        {
            dst.copy_from_slice(packet);
        }
        self.rx_count = at.saturating_add(packet.len());
        if packet.len() < PACKET {
            self.end_of_transfer();
            self.rx_count = 0;
        }
    }

    /// The transfer is complete: answer it.
    fn end_of_transfer(&mut self) {
        let count = self.rx_count;
        let stored = self.rx.get(..count.min(REQUEST_MAX)).unwrap_or(&[]);
        let Some(h) = Header::parse(stored) else {
            self.respond(0, 0, Err(Status::BadFrame));
            return;
        };
        let framed = HEADER_LEN + usize::from(h.len) == count;
        if count > REQUEST_MAX {
            // A request longer than any command takes is malformed if its
            // length says it is that long, damaged if not.
            let status = if framed {
                Status::BadRequest
            } else {
                Status::BadFrame
            };
            self.respond(h.command, h.seq, Err(status));
            return;
        }
        let payload = match proto::decode(stored) {
            Ok((_, p)) => p,
            Err(_) => {
                self.respond(h.command, h.seq, Err(Status::BadFrame));
                return;
            }
        };
        if h.version != VERSION {
            self.respond(h.command, h.seq, Err(Status::Version));
            return;
        }
        // The payload is copied out so that a command may build its answer
        // in `self`.
        let mut args = [0u8; REQUEST_MAX - HEADER_LEN];
        let n = payload.len();
        if let (Some(dst), Some(src)) = (args.get_mut(..n), payload.get(..n)) {
            dst.copy_from_slice(src);
        }
        let (result, reboot) = self.run(h.command, args.get(..n).unwrap_or(&[]));
        self.respond(h.command, h.seq, result);
        if let Some(tx) = self.tx.as_mut() {
            tx.reboot = reboot;
        }
    }

    /// Run a command: its response's body (with the length for `READ`),
    /// and a reboot to do once that has gone.
    fn run(
        &mut self,
        command: u8,
        p: &[u8],
    ) -> (Result<(Body, usize), Status>, Option<BootTarget>) {
        let empty = |r: Result<(), Status>| r.map(|()| (Body::Empty, 0));
        let size = |n: usize| {
            if p.len() == n {
                Ok(())
            } else {
                Err(Status::BadRequest)
            }
        };
        let policy = Policy::new(self.board, self.id.mode);
        let Some(cmd) = Command::from_u8(command) else {
            return (Err(Status::BadRequest), None);
        };
        let result = match cmd {
            Command::Info => size(0).map(|()| {
                self.info = info::build(self.board, &self.id);
                (Body::Info, INFO_LEN)
            }),
            Command::Read => size(8).and_then(|()| {
                let addr = u32_at(p, 0).unwrap_or(u32::MAX);
                let len = u32_at(p, 4).unwrap_or(0);
                if len == 0 || len as usize > READ_MAX {
                    return Err(Status::BadRequest);
                }
                Policy::<B>::readable(addr, len)?;
                Ok((Body::Flash(addr), len as usize))
            }),
            Command::Write => empty(size(4 + WRITE_LEN).and_then(|()| {
                let addr = u32_at(p, 0).unwrap_or(u32::MAX);
                let page = p
                    .get(4..)
                    .and_then(|d| d.first_chunk::<PAGE_LEN>())
                    .ok_or(Status::BadRequest)?;
                policy.write(addr, page)
            })),
            Command::Erase => {
                empty(size(4).and_then(|()| policy.erase(u32_at(p, 0).unwrap_or(u32::MAX))))
            }
            Command::Commit => empty(size(1).and_then(|()| {
                if self.id.mode != Mode::Dfu {
                    return Err(Status::NotPermitted);
                }
                let target = p
                    .first()
                    .and_then(|&t| CommitTarget::from_arg(t))
                    .ok_or(Status::BadRequest)?;
                commit(self.board, self.id.board_rev, target)
            })),
            Command::Load => empty(size(0).and_then(|()| {
                if self.id.mode != Mode::Application {
                    return Err(Status::NotPermitted);
                }
                self.board.reload()
            })),
            Command::Reboot => {
                let to = size(1).and_then(|()| match p.first() {
                    Some(&TARGET_APPLICATION) => Ok(BootTarget::Application),
                    Some(&TARGET_DFU) => Ok(BootTarget::Dfu),
                    _ => Err(Status::BadRequest),
                });
                return (to.map(|_| (Body::Empty, 0)), to.ok());
            }
        };
        (result, None)
    }

    /// Queue the response to `command`, `seq`.
    fn respond(&mut self, command: u8, seq: u16, result: Result<(Body, usize), Status>) {
        let (status, body, len) = match result {
            Ok((body, len)) => (Status::Ok, body, len),
            Err(s) => (s, Body::Empty, 0),
        };
        let mut h = Header {
            version: VERSION,
            command,
            status: status as u8,
            seq,
            len: len as u16,
            crc: 0,
        };
        let mut crc = Crc32::new();
        crc.update(&h.head());
        match body {
            Body::Empty => {}
            Body::Info => crc.update(&self.info),
            Body::Flash(addr) => feed(self.board, &mut crc, addr, len as u32),
        }
        h.crc = crc.finish();
        let total = HEADER_LEN + len;
        self.tx = Some(Tx {
            head: h.encode(),
            body,
            total,
            sent: 0,
            zlp: total.is_multiple_of(PACKET),
            in_flight: false,
            reboot: None,
        });
    }

    /// The next packet of the response into `out`, and its length (0 for
    /// the closing zero-length packet); `None` if there is nothing to send
    /// or a packet is still armed. Nothing changes until
    /// [`Server::packet_armed`].
    pub fn next_packet(&self, out: &mut [u8; PACKET]) -> Option<usize> {
        let tx = self.tx.as_ref()?;
        if tx.in_flight {
            return None;
        }
        if tx.sent >= tx.total {
            return tx.zlp.then_some(0);
        }
        let n = (tx.total - tx.sent).min(PACKET);
        let mut filled = 0;
        while filled < n {
            let at = tx.sent + filled;
            let dst = out.get_mut(filled..n).unwrap_or(&mut []);
            let took = if at < HEADER_LEN {
                let src = tx.head.get(at..).unwrap_or(&[]);
                let k = src.len().min(dst.len());
                if let (Some(d), Some(s)) = (dst.get_mut(..k), src.get(..k)) {
                    d.copy_from_slice(s);
                }
                k
            } else {
                let off = at - HEADER_LEN;
                match tx.body {
                    Body::Empty => dst.len(),
                    Body::Info => {
                        let src = self.info.get(off..).unwrap_or(&[]);
                        let k = src.len().min(dst.len());
                        if let (Some(d), Some(s)) = (dst.get_mut(..k), src.get(..k)) {
                            d.copy_from_slice(s);
                        }
                        dst.len()
                    }
                    Body::Flash(addr) => {
                        self.board.read(addr + off as u32, dst);
                        dst.len()
                    }
                }
            };
            if took == 0 {
                break;
            }
            filled += took;
        }
        Some(n)
    }

    /// The packet [`Server::next_packet`] gave is armed on the endpoint.
    pub fn packet_armed(&mut self) {
        if let Some(tx) = self.tx.as_mut() {
            if tx.sent < tx.total {
                tx.sent += (tx.total - tx.sent).min(PACKET);
            } else {
                tx.zlp = false;
            }
            tx.in_flight = true;
        }
    }

    /// The host took the armed packet. After the last one the response is
    /// done, and a `REBOOT`'s reboot happens.
    pub fn packet_sent(&mut self) {
        let Some(tx) = self.tx.as_mut() else {
            return;
        };
        tx.in_flight = false;
        if tx.sent >= tx.total && !tx.zlp {
            let reboot = tx.reboot;
            self.tx = None;
            if let Some(to) = reboot {
                self.board.reboot(to);
            }
        }
    }

    /// Whether a response is being sent, for tests and the simulator.
    pub const fn responding(&self) -> bool {
        self.tx.is_some()
    }
}
