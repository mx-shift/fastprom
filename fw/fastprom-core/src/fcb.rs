//! The fabric configuration bitstream, as the FCB takes it.
//!
//! The AG32's fabric configuration block (FCB) is fed a Supra bitstream one
//! 32-bit word at a time in its AUTO mode (`fastprom_hal::fcb`). A plain
//! bitstream is [`CONFIG_LEN`] bytes, written word by word; a *compressed*
//! one (Supra with `ON_CHIP_BITSTREAM_DECOMPRESSION ON`, which AgRV_pio's
//! `board_logic.compress = true` sets) is its first two words verbatim, the
//! IDCODE and USERID, followed by the rest LZW-coded. The DFU stage carries
//! its USB-only fabric compressed (3.2 KB instead of 98 KB) and expands it
//! into the FCB with [`decompress`].
//!
//! The format is the one AGM's decoder reads, `FCB_AutoDecompressEncrypt`
//! in AgRV_pio's `packages/framework-agrv_sdk/src/fcb.c` (there with no
//! decode callback, i.e. not encrypted): codes MSB first in a bit stream,
//! 9 bits wide to start; 0-255 literal bytes, 256 CLEAR (back to 9 bits and
//! an empty dictionary), 257 STOP, 258 on the dictionary; the width grows
//! by one bit as soon as the next code to be defined would not fit; output
//! bytes are packed into words little-endian (the first byte in bits 0-7),
//! and decoding ends after exactly [`CONFIG_LEN`] - 8 bytes. Unlike the C,
//! every index is bounded here: malformed input is an [`LzwError`], never
//! an access outside the tables, so the firmware links no panic path.

use thiserror::Error;

/// A whole configuration, in bytes: fcb.h `FCB_AUTO_WORDS` (99944 / 4) words,
/// the size of every Supra bitstream for the AGRV2K.
pub const CONFIG_LEN: usize = 99_944;
/// [`CONFIG_LEN`] in words, as the FCB's AUTO register takes them.
pub const CONFIG_WORDS: usize = CONFIG_LEN / 4;

/// fcb.c LZW_DATA_WIDTH, and the codes it implies.
const DATA_WIDTH: u32 = 8;
/// The largest literal (fcb.c MAX_DATA).
const MAX_DATA: u32 = (1 << DATA_WIDTH) - 1;
const CLEAR: u32 = MAX_DATA + 1;
const STOP: u32 = MAX_DATA + 2;
/// The first dictionary code (fcb.c MIN_INDEX).
const MIN_CODE: u32 = MAX_DATA + 3;
/// Dictionary slots: fcb.c LZW_MAX_INDEX + 1. A code `c` above
/// [`MAX_DATA`] lives in slot `c - MAX_DATA`, as in the C.
const SLOTS: usize = 1024;
const SLOT_MASK: usize = SLOTS - 1;
/// The first slot a dictionary code fills (fcb.c's initial decode_index).
const FIRST_SLOT: u32 = MIN_CODE - MAX_DATA;

/// Why a compressed bitstream could not be expanded.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum LzwError {
    #[error("the compressed bitstream is {len} bytes, shorter than its 8-byte IDCODE and USERID")]
    NoHeader { len: usize },
    #[error("the compressed bitstream ended after {bytes} of {expected} configuration bytes")]
    Truncated { bytes: usize, expected: usize },
    #[error("STOP code after {bytes} of {expected} configuration bytes")]
    Stopped { bytes: usize, expected: usize },
    #[error("code {code} is not defined yet (next is {next})")]
    UndefinedCode { code: u32, next: u32 },
    #[error("the dictionary is full and the stream did not clear it")]
    DictionaryFull,
    #[error("a dictionary entry chains longer than the dictionary")]
    Loop,
}

/// The decoder's tables: 4 KiB, fcb.c's `dict_index`, `dict_data` and
/// `decode_array` (which the C keeps on the stack).
pub struct Scratch {
    /// The code each entry extends.
    prefix: [u16; SLOTS],
    /// The byte each entry appends.
    last: [u8; SLOTS],
    /// One string, reversed, as it is unwound.
    string: [u8; SLOTS],
}

impl Scratch {
    pub const fn new() -> Self {
        Self {
            prefix: [0; SLOTS],
            last: [0; SLOTS],
            string: [0; SLOTS],
        }
    }
}

impl Default for Scratch {
    fn default() -> Self {
        Self::new()
    }
}

/// Packs bytes into little-endian words and counts them to the end.
struct Words<F> {
    word: u32,
    bytes: usize,
    sink: F,
}

impl<F: FnMut(u32)> Words<F> {
    /// Add a byte; true once the configuration is complete.
    fn push(&mut self, byte: u8) -> bool {
        self.word = (self.word >> 8) | (u32::from(byte) << 24);
        self.bytes += 1;
        if self.bytes.is_multiple_of(4) {
            (self.sink)(self.word);
        }
        self.bytes == CONFIG_LEN - 8
    }
}

/// Expand a compressed bitstream into its [`CONFIG_WORDS`] words, in order,
/// through `sink` (the FCB's AUTO register, or a test buffer).
///
/// On an error `sink` has had some words; the caller ends the FCB's AUTO
/// mode either way and reads its status.
pub fn decompress(
    src: &[u8],
    scratch: &mut Scratch,
    mut sink: impl FnMut(u32),
) -> Result<(), LzwError> {
    let (Some(head), Some(body)) = (src.get(..8), src.get(8..)) else {
        return Err(LzwError::NoHeader { len: src.len() });
    };
    // IDCODE and USERID, as the C reads them: two native (little-endian)
    // words, not compressed.
    for w in head.as_chunks::<4>().0 {
        sink(u32::from_le_bytes(*w));
    }

    let mut out = Words {
        word: 0,
        bytes: 0,
        sink,
    };
    let mut width = DATA_WIDTH + 1;
    let mut slot = FIRST_SLOT;
    let mut prev: Option<u32> = None;
    let mut prev_first = 0u8;
    let mut acc = 0u32;
    let mut bits = 0u32;

    for &byte in body {
        acc = (acc << 8) | u32::from(byte);
        bits += 8;
        while bits >= width {
            bits -= width;
            let code = (acc >> bits) & ((1 << width) - 1);
            if code == CLEAR {
                width = DATA_WIDTH + 1;
                slot = FIRST_SLOT;
                prev = None;
                continue;
            }
            if code == STOP {
                return Err(LzwError::Stopped {
                    bytes: out.bytes,
                    expected: CONFIG_LEN - 8,
                });
            }
            let next = slot + MAX_DATA;
            if code > next || (code > MAX_DATA && prev.is_none()) {
                return Err(LzwError::UndefinedCode { code, next });
            }
            if code == next {
                // The entry being defined (cScSc): the previous string plus
                // its own first byte.
                if let Some(p) = prev {
                    scratch.prefix[slot as usize & SLOT_MASK] = p as u16;
                    scratch.last[slot as usize & SLOT_MASK] = prev_first;
                }
            }

            // Unwind the string, last byte first.
            let mut len = 0usize;
            let mut c = code;
            while c > MAX_DATA {
                if len >= SLOTS - 1 {
                    return Err(LzwError::Loop);
                }
                let s = (c - MAX_DATA) as usize & SLOT_MASK;
                scratch.string[len & SLOT_MASK] = scratch.last[s];
                len += 1;
                c = u32::from(scratch.prefix[s]);
            }
            let first = c as u8;
            scratch.string[len & SLOT_MASK] = first;
            prev_first = first;

            if let Some(p) = prev {
                if slot as usize >= SLOTS {
                    return Err(LzwError::DictionaryFull);
                }
                scratch.prefix[slot as usize & SLOT_MASK] = p as u16;
                scratch.last[slot as usize & SLOT_MASK] = first;
                slot += 1;
                if slot + MAX_DATA > (1 << width) - 1 {
                    width += 1;
                }
            }
            prev = Some(code);

            // Out in order: the first byte, then back down the unwinding.
            for i in (0..=len).rev() {
                if out.push(scratch.string[i & SLOT_MASK]) {
                    return Ok(());
                }
            }
        }
    }
    Err(LzwError::Truncated {
        bytes: out.bytes,
        expected: CONFIG_LEN - 8,
    })
}
