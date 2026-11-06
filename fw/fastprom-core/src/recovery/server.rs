//! The recovery loader's side of the protocol: AN3155's framing, the
//! commands recovery needs, against a [`Target`].
//!
//! A state machine fed one received byte at a time ([`Server::feed`]), so
//! the board's loop and the host's simulator drive the same code: the
//! board from UART0, the simulator from what the page writes. Every reply
//! goes out through a callback before `feed` returns.
//!
//! The commands, with AN3155's framing (`cmd`, `!cmd`, ACK; an address as
//! four big-endian bytes and their XOR; a count `N` meaning `N + 1`):
//!
//! | command | does |
//! |---|---|
//! | sync `0x7F` | ACK: the host's way back in step after a failure |
//! | `GET` `0x00` | the version ([`LOADER_VERSION`]) and the commands below |
//! | `GID` `0x02` | the chip's device ID, four bytes |
//! | `READ MEMORY` `0x11` | up to 256 bytes of the main flash array or the option bytes |
//! | `WRITE MEMORY` `0x31` | up to 256 bytes into one flash page (page aligned, the rest of the page left erased), or the 128 option bytes at their base; programmed, then read back, and NACKed if it does not read back |
//! | `EXTENDED ERASE` `0x44` | a list of 4 KiB sectors by number (AGM's "page" is the sector); no mass erase |
//! | erase option bytes `0xA3` | AGM's extension: erase them, ACK when done |
//! | reset `0xA2` | AGM's extension: ACK twice, then reset the chip |
//!
//! Anything malformed (a complement, a checksum, an address or length out
//! of range) is NACKed and the server waits for the next command. A host
//! that stops part way is forgotten by [`Server::idle`], which the board
//! calls after [`IDLE_MS`] without a byte: a sync after that is answered.

use crate::memmap::{FLASH_BASE, FLASH_SIZE, OPTION_BASE, OPTION_LEN, PAGE, SECTOR};
use crate::update::{Flash, FlashError, PAGE_LEN};

use super::{
    xor, ACK, CMD_ERASE, CMD_ERASE_OPTIONS, CMD_GET, CMD_GID, CMD_READ, CMD_RESET, CMD_WRITE,
    LOADER_VERSION, MAX_FRAME, NACK, SYNC,
};

/// Without a byte for this long part way through a command, the server
/// forgets it (the board calls [`Server::idle`]).
pub const IDLE_MS: u32 = 100;

/// The commands `GET` lists, after the version.
pub const COMMANDS: [u8; 7] = [
    CMD_GET,
    CMD_GID,
    CMD_READ,
    CMD_WRITE,
    CMD_ERASE,
    CMD_ERASE_OPTIONS,
    CMD_RESET,
];

const OPTIONS: usize = OPTION_LEN as usize;
/// Sectors one erase may list: a frame's worth of two-byte numbers.
const MAX_SECTORS: usize = MAX_FRAME / 2;

/// What the loader needs of the board beyond its [`Flash`].
pub trait Target: Flash {
    /// SYS `DEVICE_ID`.
    fn device_id(&self) -> u32;

    /// The option bytes as they read now.
    fn read_options(&self, buf: &mut [u8; OPTIONS]);

    /// Erase the option bytes (flash.c `FLASH_EraseOptionByte`).
    ///
    /// # Errors
    ///
    /// When the controller reports a failure or does not finish.
    fn erase_options(&self) -> Result<(), FlashError>;

    /// Program the option bytes from `image` (words of all ones are left
    /// alone). NOR: bits only clear, so erase them first.
    ///
    /// # Errors
    ///
    /// When the controller reports a failure or does not finish.
    fn program_options(&self, image: &[u8; OPTIONS]) -> Result<(), FlashError>;

    /// Reset the chip, once the last reply has left. Does not return on
    /// the board.
    fn reset(&self);
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State {
    /// Waiting for a command (or a sync).
    Idle,
    /// Its complement.
    Complement(u8),
    /// A read's or a write's address: `got` bytes of five so far.
    Address { cmd: u8, got: usize },
    /// A read's count, then its complement.
    ReadCount { count: Option<u8> },
    /// A write's count.
    WriteCount,
    /// A write's data: `len` bytes, `got` so far, then the checksum.
    WriteData { len: usize, got: usize },
    /// An erase's count (two bytes, big-endian).
    EraseCount { high: Option<u8> },
    /// An erase's sector numbers: `count` of them, `got` bytes so far, then
    /// the checksum.
    EraseList { count: usize, got: usize },
    /// AN3155's special erase codes (mass erase and the like), which this
    /// loader does not do: their checksum, then a NACK.
    EraseSpecial,
}

/// The loader's protocol state.
#[derive(Debug)]
pub struct Server {
    state: State,
    addr: u32,
    sum: u8,
    buf: [u8; MAX_FRAME],
}

impl Default for Server {
    fn default() -> Self {
        Self::new()
    }
}

/// Where an address is.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Region {
    Flash,
    Options,
}

fn region(addr: u32) -> Option<Region> {
    if (FLASH_BASE..FLASH_BASE + FLASH_SIZE).contains(&addr) {
        Some(Region::Flash)
    } else if (OPTION_BASE..OPTION_BASE + OPTION_LEN).contains(&addr) {
        Some(Region::Options)
    } else {
        None
    }
}

impl Server {
    pub const fn new() -> Self {
        Self {
            state: State::Idle,
            addr: 0,
            sum: 0,
            buf: [0; MAX_FRAME],
        }
    }

    /// Whether a command is part way through.
    pub fn busy(&self) -> bool {
        self.state != State::Idle
    }

    /// No byte for [`IDLE_MS`]: forget the command part way through.
    pub fn idle(&mut self) {
        self.state = State::Idle;
    }

    /// One received byte; replies go to `out`.
    pub fn feed<T: Target>(&mut self, b: u8, t: &T, out: &mut impl FnMut(&[u8])) {
        let next = match self.state {
            State::Idle if b == SYNC => {
                out(&[ACK]);
                State::Idle
            }
            State::Idle => State::Complement(b),
            State::Complement(cmd) if b == !cmd => self.command(cmd, t, out),
            State::Complement(_) => {
                out(&[NACK]);
                State::Idle
            }
            State::Address { cmd, got } => self.address(cmd, got, b, out),
            State::ReadCount { count: None } => State::ReadCount { count: Some(b) },
            State::ReadCount { count: Some(n) } => {
                self.read(n, b, t, out);
                State::Idle
            }
            State::WriteCount => {
                self.sum = b;
                let len = usize::from(b) + 1;
                let fits = match region(self.addr) {
                    Some(Region::Flash) => true,
                    Some(Region::Options) => len <= OPTIONS,
                    None => false,
                };
                if fits {
                    State::WriteData { len, got: 0 }
                } else {
                    out(&[NACK]);
                    State::Idle
                }
            }
            State::WriteData { len, got } if got < len => {
                if let Some(slot) = self.buf.get_mut(got) {
                    *slot = b;
                }
                self.sum ^= b;
                State::WriteData { len, got: got + 1 }
            }
            State::WriteData { len, .. } => {
                let ok = b == self.sum && self.write(len, t).is_ok();
                out(&[if ok { ACK } else { NACK }]);
                State::Idle
            }
            State::EraseCount { high: None } => State::EraseCount { high: Some(b) },
            State::EraseCount { high: Some(h) } => {
                self.sum = h ^ b;
                let n = u16::from_be_bytes([h, b]);
                if n >= 0xFFF0 {
                    State::EraseSpecial
                } else if usize::from(n) < MAX_SECTORS {
                    State::EraseList {
                        count: usize::from(n) + 1,
                        got: 0,
                    }
                } else {
                    out(&[NACK]);
                    State::Idle
                }
            }
            State::EraseList { count, got } if got < 2 * count => {
                if let Some(slot) = self.buf.get_mut(got) {
                    *slot = b;
                }
                self.sum ^= b;
                State::EraseList {
                    count,
                    got: got + 1,
                }
            }
            State::EraseList { count, .. } => {
                let ok = b == self.sum && self.erase(count, t).is_ok();
                out(&[if ok { ACK } else { NACK }]);
                State::Idle
            }
            State::EraseSpecial => {
                out(&[NACK]);
                State::Idle
            }
        };
        self.state = next;
    }

    fn command<T: Target>(&mut self, cmd: u8, t: &T, out: &mut impl FnMut(&[u8])) -> State {
        match cmd {
            CMD_GET => {
                out(&[ACK, COMMANDS.len() as u8, LOADER_VERSION]);
                out(&COMMANDS);
                out(&[ACK]);
                State::Idle
            }
            CMD_GID => {
                let [a, b, c, d] = t.device_id().to_be_bytes();
                out(&[ACK, 3, a, b, c, d, ACK]);
                State::Idle
            }
            CMD_READ | CMD_WRITE => {
                out(&[ACK]);
                State::Address { cmd, got: 0 }
            }
            CMD_ERASE => {
                out(&[ACK]);
                State::EraseCount { high: None }
            }
            CMD_ERASE_OPTIONS => {
                out(&[ACK]);
                let ok = t.erase_options().is_ok();
                out(&[if ok { ACK } else { NACK }]);
                State::Idle
            }
            CMD_RESET => {
                out(&[ACK, ACK]);
                t.reset();
                State::Idle
            }
            _ => {
                out(&[NACK]);
                State::Idle
            }
        }
    }

    fn address(&mut self, cmd: u8, got: usize, b: u8, out: &mut impl FnMut(&[u8])) -> State {
        if got < 4 {
            if let Some(slot) = self.buf.get_mut(got) {
                *slot = b;
            }
            return State::Address { cmd, got: got + 1 };
        }
        let a = [self.buf[0], self.buf[1], self.buf[2], self.buf[3]];
        let addr = u32::from_be_bytes(a);
        let ok = b == xor(&a)
            && match (cmd, region(addr)) {
                (CMD_READ, Some(_)) => true,
                (CMD_WRITE, Some(Region::Flash)) => addr.is_multiple_of(PAGE),
                (CMD_WRITE, Some(Region::Options)) => addr == OPTION_BASE,
                _ => false,
            };
        if !ok {
            out(&[NACK]);
            return State::Idle;
        }
        self.addr = addr;
        out(&[ACK]);
        if cmd == CMD_READ {
            State::ReadCount { count: None }
        } else {
            State::WriteCount
        }
    }

    fn read<T: Target>(&mut self, n: u8, complement: u8, t: &T, out: &mut impl FnMut(&[u8])) {
        let len = usize::from(n) + 1;
        let end = self.addr as usize + len;
        let data = self.buf.get_mut(..len);
        let ok = complement == !n
            && match region(self.addr) {
                Some(Region::Flash) => end <= (FLASH_BASE + FLASH_SIZE) as usize,
                Some(Region::Options) => end <= (OPTION_BASE + OPTION_LEN) as usize,
                None => false,
            };
        match (ok, data) {
            (true, Some(data)) => {
                if region(self.addr) == Some(Region::Flash) {
                    t.read(self.addr, data);
                } else {
                    let mut o = [0u8; OPTIONS];
                    t.read_options(&mut o);
                    let at = (self.addr - OPTION_BASE) as usize;
                    if let Some(src) = o.get(at..at + len) {
                        data.copy_from_slice(src);
                    }
                }
                out(&[ACK]);
                out(data);
            }
            _ => out(&[NACK]),
        }
    }

    /// Program what arrived and read it back.
    fn write<T: Target>(&self, len: usize, t: &T) -> Result<(), FlashError> {
        let data = self.buf.get(..len).ok_or(FlashError::Failed)?;
        match region(self.addr) {
            Some(Region::Flash) => {
                let mut page = [0xFFu8; PAGE_LEN];
                page.get_mut(..len)
                    .ok_or(FlashError::Failed)?
                    .copy_from_slice(data);
                t.program_page(self.addr, &page)?;
                let mut back = [0u8; PAGE_LEN];
                t.read(self.addr, &mut back);
                if back == page {
                    Ok(())
                } else {
                    Err(FlashError::Failed)
                }
            }
            Some(Region::Options) => {
                let mut image = [0xFFu8; OPTIONS];
                image
                    .get_mut(..len)
                    .ok_or(FlashError::Failed)?
                    .copy_from_slice(data);
                t.program_options(&image)?;
                let mut back = [0u8; OPTIONS];
                t.read_options(&mut back);
                if back == image {
                    Ok(())
                } else {
                    Err(FlashError::Failed)
                }
            }
            None => Err(FlashError::Failed),
        }
    }

    /// Erase the listed sectors, after checking every number.
    fn erase<T: Target>(&self, count: usize, t: &T) -> Result<(), FlashError> {
        let list = self.buf.get(..2 * count).ok_or(FlashError::Failed)?;
        let sectors = FLASH_SIZE / SECTOR;
        let number = |c: &[u8]| match c {
            [h, l] => u32::from(u16::from_be_bytes([*h, *l])),
            _ => u32::MAX,
        };
        if list.chunks(2).any(|c| number(c) >= sectors) {
            return Err(FlashError::Failed);
        }
        for c in list.chunks(2) {
            t.erase_sector(FLASH_BASE + number(c) * SECTOR)?;
        }
        Ok(())
    }
}
