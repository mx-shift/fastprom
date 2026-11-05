//! CRC-32/ISO-HDLC: reflected, polynomial `0xEDB88320`, init and xorout
//! `0xFFFFFFFF`. Identical to Python's `zlib.crc32`, which
//! `tools/mkimagetable.py` uses for both the image CRCs and the header CRC.
//!
//! Four bits at a time from a 16-entry table: 64 bytes of flash, and about
//! four times faster than the bitwise loop the C firmware used. It runs over
//! at most 4 x 64 KiB at boot, where every millisecond is time the socket is
//! dead.

const POLY: u32 = 0xEDB8_8320;

const NIBBLE: [u32; 16] = nibble_table();

const fn nibble_table() -> [u32; 16] {
    let mut table = [0u32; 16];
    let mut i = 0;
    while i < 16 {
        let mut c = i as u32;
        let mut k = 0;
        while k < 4 {
            c = if c & 1 != 0 { (c >> 1) ^ POLY } else { c >> 1 };
            k += 1;
        }
        table[i] = c;
        i += 1;
    }
    table
}

/// An incremental CRC-32.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Crc32 {
    state: u32,
}

impl Crc32 {
    /// A CRC over no bytes yet.
    pub const fn new() -> Self {
        Self { state: !0 }
    }

    /// Add one byte.
    #[inline]
    pub fn byte(&mut self, byte: u8) {
        let mut c = self.state ^ u32::from(byte);
        c = (c >> 4) ^ NIBBLE[(c & 0xF) as usize];
        c = (c >> 4) ^ NIBBLE[(c & 0xF) as usize];
        self.state = c;
    }

    /// Add a run of bytes.
    pub fn update(&mut self, data: &[u8]) {
        for &b in data {
            self.byte(b);
        }
    }

    /// The CRC of everything added.
    pub const fn finish(self) -> u32 {
        !self.state
    }
}

impl Default for Crc32 {
    fn default() -> Self {
        Self::new()
    }
}

/// The CRC-32 of `data`, as `zlib.crc32(data)`.
pub fn crc32(data: &[u8]) -> u32 {
    let mut crc = Crc32::new();
    crc.update(data);
    crc.finish()
}
