//! FastPROM's update protocol on the wire (`docs/10-updates.md`, "The
//! update protocol"): one module that the device ([`super::Server`]) and
//! the host (`fastprom-host`'s client) both build and read frames with, so
//! that the two cannot disagree.
//!
//! # Frames
//!
//! Every request and every response is one frame: a 16-byte header, then
//! `len` bytes of payload. Little-endian throughout.
//!
//! | offset | field |
//! |---|---|
//! | 0 | magic, the four bytes `FPUP` |
//! | 4 | `u8` protocol version, [`VERSION`] |
//! | 5 | `u8` command ([`Command`]); a response echoes its request's |
//! | 6 | `u8` status ([`Status`]); 0 in a request |
//! | 7 | `u8` reserved, 0 |
//! | 8 | `u16` sequence number; a response echoes its request's |
//! | 10 | `u16` payload length |
//! | 12 | `u32` CRC-32 ([`crate::crc32`]) of bytes 0..12, then the payload |
//!
//! The header's layout, the magic and the CRC rule never change; a new
//! protocol version changes only the commands and their payloads. So any
//! device can tell a host which version it speaks, and a host can refuse a
//! device that speaks another.
//!
//! # Framing on USB
//!
//! The vendor interface's bulk OUT endpoint carries requests, its bulk IN
//! endpoint responses, in 64-byte packets. A frame is one USB transfer: its
//! packets, the last of them short (shorter than 64 bytes). A frame whose
//! length is a multiple of 64 is followed by a zero-length packet, which is
//! that short packet. The length field must then agree with the bytes the
//! transfer carried; the CRC with all of them.
//!
//! Every request transfer gets exactly one response: the device answers a
//! transfer it cannot read as a request (too short, no magic, a length
//! field that disagrees with what arrived, a CRC that does not match) with
//! [`Status::BadFrame`], echoing what it could of the header, or command
//! and sequence number 0 if it could read none. The host sends a request
//! only when it has the response to the one before.
//!
//! # Recovery
//!
//! USB's own CRCs and retries mean a packet is lost or damaged only when a
//! transfer is abandoned part way (a host gives up, a page is closed). The
//! rule is then the same for every failure the host can see - a response
//! it cannot decode, one with another sequence number or command, or
//! `BadFrame`: send [`REQUEST_RESET`], a vendor control request to the
//! interface, which empties the device's half-received request, its
//! unsent response and both endpoints' packets, and send the request again
//! under a new sequence number. Every command is idempotent (a page
//! programmed again with the same data is unchanged, an erase is an erase,
//! `COMMIT` of a committed page answers `Ok`), so a request that ran before
//! its response was lost runs again harmlessly. A host starts every session
//! with a reset as well.
//!
//! # Commands
//!
//! | command | id | request payload | response payload | from |
//! |---|---|---|---|---|
//! | `INFO` | 1 | none | [`super::info::INFO_LEN`] bytes | both |
//! | `READ` | 2 | `u32` address, `u32` length (1 to [`READ_MAX`]) | the bytes | both |
//! | `WRITE` | 3 | `u32` address, one 256-byte page | none | both (by region) |
//! | `ERASE` | 4 | `u32` address of one 4 KiB sector | none | both (by region) |
//! | `COMMIT` | 5 | `u8` target: 0 application, 1 fabric | none | DFU stage |
//! | `LOAD` | 6 | none | none | application |
//! | `REBOOT` | 7 | `u8` target: 0 application, 1 DFU mode | none | both |
//!
//! A response with a status other than `Ok` has no payload.

use crate::crc32::Crc32;

/// The first four bytes of every frame.
pub const MAGIC: [u8; 4] = *b"FPUP";
/// The protocol version this crate speaks.
pub const VERSION: u8 = 1;
/// Bytes in a header.
pub const HEADER_LEN: usize = 16;
/// The bulk endpoints' packet size (full speed).
pub const PACKET: usize = 64;
/// The most a `READ` returns.
pub const READ_MAX: usize = 4096;
/// The bytes a `WRITE` programs: one flash page.
pub const WRITE_LEN: usize = super::PAGE_LEN;
/// The longest request: `WRITE`'s.
pub const REQUEST_MAX: usize = HEADER_LEN + 4 + WRITE_LEN;
/// The longest response: `READ`'s.
pub const RESPONSE_MAX: usize = HEADER_LEN + READ_MAX;
/// What a host asks each IN transfer for: more than any response, so the
/// transfer always ends at the response's short packet.
pub const IN_TRANSFER: usize = (RESPONSE_MAX / PACKET + 1) * PACKET;
const _: () = assert!(IN_TRANSFER > RESPONSE_MAX && IN_TRANSFER.is_multiple_of(PACKET));
/// The vendor control request (to the interface, host to device, no data)
/// that empties both directions: the recovery rule above.
pub const REQUEST_RESET: u8 = 0x01;

/// `COMMIT`'s and `REBOOT`'s targets.
pub const TARGET_APPLICATION: u8 = 0;
/// `COMMIT`: the fabric.
pub const TARGET_FABRIC: u8 = 1;
/// `REBOOT`: DFU mode.
pub const TARGET_DFU: u8 = 1;

/// What a request asks for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum Command {
    Info = 1,
    Read = 2,
    Write = 3,
    Erase = 4,
    Commit = 5,
    Load = 6,
    Reboot = 7,
}

impl Command {
    /// The command an identifier names.
    pub const fn from_u8(v: u8) -> Option<Self> {
        Some(match v {
            1 => Self::Info,
            2 => Self::Read,
            3 => Self::Write,
            4 => Self::Erase,
            5 => Self::Commit,
            6 => Self::Load,
            7 => Self::Reboot,
            _ => return None,
        })
    }

    /// Its name, for messages.
    pub const fn name(self) -> &'static str {
        match self {
            Self::Info => "INFO",
            Self::Read => "READ",
            Self::Write => "WRITE",
            Self::Erase => "ERASE",
            Self::Commit => "COMMIT",
            Self::Load => "LOAD",
            Self::Reboot => "REBOOT",
        }
    }
}

/// How a request ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum Status {
    /// Done.
    Ok = 0,
    /// The transfer was not a whole request: too short, no magic, a length
    /// that disagrees with what arrived, or a CRC that does not match. The
    /// host resets and sends again.
    BadFrame = 1,
    /// The device does not speak the request's protocol version; the
    /// response's header carries the one it does.
    Version = 2,
    /// A command the device does not know, a payload of the wrong length,
    /// or an argument out of range.
    BadRequest = 3,
    /// Outside the flash.
    InvalidAddress = 4,
    /// Not on a page (`WRITE`) or sector (`ERASE`) boundary.
    BadAlignment = 5,
    /// Refused by the mode's policy: a region this mode may not write, the
    /// DFU stage, the option bytes, `LOAD` in DFU mode, `COMMIT` in the
    /// application, a page built for another board revision.
    NotPermitted = 6,
    /// Not now: a payload its committed page guards, a `LOAD` with the
    /// socket not released or no switch reading.
    PreconditionNotMet = 7,
    /// `COMMIT`: the payload does not match its page's CRC-32.
    CrcMismatch = 8,
    /// `COMMIT`: no well-formed page to commit; `LOAD`: no valid image.
    InvalidData = 9,
    /// The flash failed an erase or program, or read back otherwise.
    FlashFailed = 10,
}

impl Status {
    /// The status a code names.
    pub const fn from_u8(v: u8) -> Option<Self> {
        Some(match v {
            0 => Self::Ok,
            1 => Self::BadFrame,
            2 => Self::Version,
            3 => Self::BadRequest,
            4 => Self::InvalidAddress,
            5 => Self::BadAlignment,
            6 => Self::NotPermitted,
            7 => Self::PreconditionNotMet,
            8 => Self::CrcMismatch,
            9 => Self::InvalidData,
            10 => Self::FlashFailed,
            _ => return None,
        })
    }

    /// What it means, for messages.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Ok => "ok",
            Self::BadFrame => "the request arrived damaged",
            Self::Version => "another protocol version",
            Self::BadRequest => "a malformed request",
            Self::InvalidAddress => "an address outside the flash",
            Self::BadAlignment => "an address off its page or sector boundary",
            Self::NotPermitted => "not permitted in this mode",
            Self::PreconditionNotMet => "a precondition is not met",
            Self::CrcMismatch => "the payload does not match its CRC",
            Self::InvalidData => "no valid data to act on",
            Self::FlashFailed => "the flash failed",
        }
    }
}

impl core::fmt::Display for Status {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A frame's header, less the magic.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Header {
    pub version: u8,
    pub command: u8,
    pub status: u8,
    pub seq: u16,
    pub len: u16,
    pub crc: u32,
}

impl Header {
    /// The first 12 bytes: everything the CRC covers before the payload.
    pub fn head(&self) -> [u8; 12] {
        let [s0, s1] = self.seq.to_le_bytes();
        let [l0, l1] = self.len.to_le_bytes();
        let [m0, m1, m2, m3] = MAGIC;
        [
            m0,
            m1,
            m2,
            m3,
            self.version,
            self.command,
            self.status,
            0,
            s0,
            s1,
            l0,
            l1,
        ]
    }

    /// All 16 bytes.
    pub fn encode(&self) -> [u8; HEADER_LEN] {
        let mut out = [0u8; HEADER_LEN];
        let head = self.head();
        let crc = self.crc.to_le_bytes();
        for (o, b) in out.iter_mut().zip(head.iter().chain(crc.iter())) {
            *o = *b;
        }
        out
    }

    /// Read a header from the start of `bytes`; `None` if they are fewer
    /// than [`HEADER_LEN`] or do not start with the magic. The CRC is not
    /// checked here ([`decode`] does).
    pub fn parse(bytes: &[u8]) -> Option<Self> {
        let h = bytes.first_chunk::<HEADER_LEN>()?;
        let [m0, m1, m2, m3, version, command, status, _, s0, s1, l0, l1, c0, c1, c2, c3] = *h;
        if [m0, m1, m2, m3] != MAGIC {
            return None;
        }
        Some(Self {
            version,
            command,
            status,
            seq: u16::from_le_bytes([s0, s1]),
            len: u16::from_le_bytes([l0, l1]),
            crc: u32::from_le_bytes([c0, c1, c2, c3]),
        })
    }

    /// A header for `payload`, its length and CRC filled in.
    pub fn new(version: u8, command: u8, status: u8, seq: u16, payload: &[u8]) -> Self {
        let mut h = Self {
            version,
            command,
            status,
            seq,
            len: payload.len() as u16,
            crc: 0,
        };
        let mut crc = Crc32::new();
        crc.update(&h.head());
        crc.update(payload);
        h.crc = crc.finish();
        h
    }
}

/// Why a frame was not read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrameError {
    /// Shorter than a header, or not starting with the magic.
    NoHeader,
    /// The length field does not match the bytes.
    Length,
    /// The CRC does not match.
    Crc,
}

impl FrameError {
    /// What it means, for messages.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::NoHeader => "no header",
            Self::Length => "a length that does not match the frame",
            Self::Crc => "a CRC that does not match",
        }
    }
}

/// Check a whole frame: its header and its payload.
///
/// # Errors
///
/// As [`FrameError`] says.
pub fn decode(frame: &[u8]) -> Result<(Header, &[u8]), FrameError> {
    let h = Header::parse(frame).ok_or(FrameError::NoHeader)?;
    let payload = frame.get(HEADER_LEN..).unwrap_or(&[]);
    if payload.len() != usize::from(h.len) {
        return Err(FrameError::Length);
    }
    // The bytes as they came, the reserved one included.
    let mut crc = Crc32::new();
    crc.update(frame.get(..12).unwrap_or(&[]));
    crc.update(payload);
    if crc.finish() != h.crc {
        return Err(FrameError::Crc);
    }
    Ok((h, payload))
}

/// Write a whole frame into `out`: the length written, or `None` if `out`
/// is too short or the payload longer than a `u16` counts.
pub fn encode(out: &mut [u8], command: u8, status: u8, seq: u16, payload: &[u8]) -> Option<usize> {
    let len = HEADER_LEN.checked_add(payload.len())?;
    if payload.len() > usize::from(u16::MAX) {
        return None;
    }
    let (head, rest) = out.get_mut(..len)?.split_at_mut(HEADER_LEN);
    head.copy_from_slice(&Header::new(VERSION, command, status, seq, payload).encode());
    rest.copy_from_slice(payload);
    Some(len)
}

/// The little-endian `u32` at `at` in `p`.
pub fn u32_at(p: &[u8], at: usize) -> Option<u32> {
    let end = at.checked_add(4)?;
    p.get(at..end)?
        .first_chunk::<4>()
        .map(|b| u32::from_le_bytes(*b))
}
