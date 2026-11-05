//! The binary image table's layout (format version 1).
//!
//! `tools/mkimagetable.py --out <file>.bin` writes it; its docstring has the
//! same table. All integers are little-endian `u32`.
//!
//! | offset  | field                                              |
//! |---------|----------------------------------------------------|
//! | `0x000` | magic, `0x4650524d` (`'FPRM'`)                     |
//! | `0x004` | version, 1                                         |
//! | `0x008` | depth: words per image, a power of two, 16..=65536 |
//! | `0x00c` | width: bits per word, 4 or 8                       |
//! | `0x010` | image count: 1, 2 or 4                             |
//! | `0x014` | part name, 28 bytes of NUL-terminated ASCII        |
//! | `0x030` | 4 directory entries of 48 bytes (unused ones zero) |
//! | `0x0f0` | 12 reserved bytes, zero                            |
//! | `0x0fc` | header CRC: CRC-32 of bytes `0x000..0x0fc`         |
//! | `0x100` | image data: image `i` at `0x100 + i * depth`       |
//!
//! A directory entry is the image's source file name (40 bytes of
//! NUL-terminated ASCII), the CRC-32 of its `depth` data bytes, and a
//! reserved zero word. Each data byte holds one word in its low `width`
//! bits.

/// `'FPRM'`, the same magic as the fabric's ID register.
pub const MAGIC: u32 = 0x4650_524D;
/// The table format this firmware reads.
pub const VERSION: u32 = 1;

/// The fixed header, including the directory and the header CRC.
pub const HEADER_LEN: usize = 0x100;
/// Images in the directory: two switch bits.
pub const MAX_IMAGES: usize = 4;
/// Smallest depth accepted (a 32x8 PROM is the smallest bipolar part).
pub const MIN_DEPTH: u32 = 16;
/// Largest depth accepted (the AHB slave's pointer is at most 16 bits).
pub const MAX_DEPTH: u32 = 1 << 16;

pub const OFF_MAGIC: usize = 0x000;
pub const OFF_VERSION: usize = 0x004;
pub const OFF_DEPTH: usize = 0x008;
pub const OFF_WIDTH: usize = 0x00C;
pub const OFF_COUNT: usize = 0x010;
pub const OFF_PART: usize = 0x014;
pub const PART_LEN: usize = 28;
pub const OFF_DIR: usize = 0x030;
pub const ENTRY_LEN: usize = 48;
pub const ENTRY_NAME_LEN: usize = 40;
pub const ENTRY_OFF_CRC: usize = 0x28;
pub const ENTRY_OFF_RESERVED: usize = 0x2C;
pub const OFF_RESERVED: usize = 0x0F0;
pub const RESERVED_LEN: usize = 12;
pub const OFF_HEADER_CRC: usize = 0x0FC;

// The fields tile the header exactly; a layout edit that breaks this fails
// the build rather than reading the wrong bytes.
const _: () = {
    assert!(OFF_PART + PART_LEN == OFF_DIR);
    assert!(OFF_DIR + MAX_IMAGES * ENTRY_LEN == OFF_RESERVED);
    assert!(OFF_RESERVED + RESERVED_LEN == OFF_HEADER_CRC);
    assert!(OFF_HEADER_CRC + 4 == HEADER_LEN);
    assert!(ENTRY_NAME_LEN == ENTRY_OFF_CRC);
    assert!(ENTRY_OFF_RESERVED + 4 == ENTRY_LEN);
};
