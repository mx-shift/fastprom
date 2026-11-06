//! The image table in flash, parsed into checked types.
//!
//! The bytes come from flash the firmware did not write, so they are
//! untrusted: [`ImageTable::parse`] checks every header field once, and
//! everything past it relies on the types. Nothing here indexes with a value
//! read from the table, and nothing panics on any input (`tests/malformed.rs`
//! fuzzes it). The layout is in [`crate::layout`].

use crate::bank::Bank;
use crate::crc32::crc32;
use crate::error::{TableError, TextField};
use crate::layout::{
    ENTRY_LEN, ENTRY_NAME_LEN, ENTRY_OFF_CRC, ENTRY_OFF_RESERVED, HEADER_LEN, MAGIC, MAX_DEPTH,
    MAX_IMAGES, MIN_DEPTH, OFF_COUNT, OFF_DEPTH, OFF_DIR, OFF_HEADER_CRC, OFF_MAGIC, OFF_PART,
    OFF_RESERVED, OFF_VERSION, OFF_WIDTH, PART_LEN, RESERVED_LEN, VERSION,
};

/// Bits per word.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Width {
    Four,
    Eight,
}

impl Width {
    /// The width in bits.
    pub const fn bits(self) -> u8 {
        match self {
            Self::Four => 4,
            Self::Eight => 8,
        }
    }

    /// The bits a word may use.
    pub const fn mask(self) -> u8 {
        match self {
            Self::Four => 0x0F,
            Self::Eight => 0xFF,
        }
    }
}

/// How many images the table holds: one per value of the switch bits used.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImageCount {
    One,
    Two,
    Four,
}

impl ImageCount {
    /// The count.
    pub const fn get(self) -> u32 {
        match self {
            Self::One => 1,
            Self::Two => 2,
            Self::Four => 4,
        }
    }
}

/// A parsed, checked image table borrowing the flash it was read from.
#[derive(Debug, Clone, Copy)]
pub struct ImageTable<'a> {
    depth: u32,
    width: Width,
    count: ImageCount,
    part: &'a str,
    names: [&'a str; MAX_IMAGES],
    crcs: [u32; MAX_IMAGES],
    /// Exactly `count * depth` bytes.
    data: &'a [u8],
}

/// What a table holds, without borrowing it: for the update core's `INFO`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TableSummary {
    /// Words per image.
    pub depth: u32,
    /// Bits per word.
    pub width: Width,
    /// Images in the table.
    pub count: ImageCount,
    /// Each image's CRC-32; zero past `count`.
    pub crcs: [u32; MAX_IMAGES],
    /// The part name, NUL padded.
    pub part: [u8; PART_LEN],
    /// Bit `n` set: image `n`'s words match its CRC and its width, as the
    /// load at boot checks them ([`crate::loader::verify`]). A header
    /// written whole over images a power cut left half written parses, and
    /// this is what tells the two apart.
    pub intact: u8,
}

impl TableSummary {
    /// Every image in the table is intact.
    pub const fn all_intact(&self) -> bool {
        let all = (1u8 << self.count.get()) - 1;
        self.intact & all == all
    }
}

/// One image of the table: its directory entry and its words.
#[derive(Debug, Clone, Copy)]
pub struct Image<'a> {
    bank: Bank,
    name: &'a str,
    crc32: u32,
    width: Width,
    /// Exactly `depth` bytes.
    words: &'a [u8],
}

impl<'a> ImageTable<'a> {
    /// Parse a table from the start of `bytes`. Bytes after the table are
    /// ignored, so the whole flash region can be passed.
    pub fn parse(bytes: &'a [u8]) -> Result<Self, TableError> {
        let header: &[u8; HEADER_LEN] = bytes
            .first_chunk()
            .ok_or(TableError::TooShort { len: bytes.len() })?;

        let magic = le32(header, OFF_MAGIC);
        if magic != MAGIC {
            return Err(TableError::BadMagic { found: magic });
        }
        let version = le32(header, OFF_VERSION);
        if version != VERSION {
            return Err(TableError::Version { found: version });
        }
        let (covered, _) = header.split_at(OFF_HEADER_CRC);
        let computed = crc32(covered);
        let stored = le32(header, OFF_HEADER_CRC);
        if computed != stored {
            return Err(TableError::HeaderCrc { computed, stored });
        }

        let depth = le32(header, OFF_DEPTH);
        if !depth.is_power_of_two() || !(MIN_DEPTH..=MAX_DEPTH).contains(&depth) {
            return Err(TableError::Depth { depth });
        }
        let width = match le32(header, OFF_WIDTH) {
            4 => Width::Four,
            8 => Width::Eight,
            width => return Err(TableError::Width { width }),
        };
        let count = match le32(header, OFF_COUNT) {
            1 => ImageCount::One,
            2 => ImageCount::Two,
            4 => ImageCount::Four,
            count => return Err(TableError::Count { count }),
        };
        let part = text(field(header, OFF_PART, PART_LEN)).ok_or(TableError::Text {
            field: TextField::PartName,
            offset: OFF_PART,
        })?;

        let mut names = [""; MAX_IMAGES];
        let mut crcs = [0u32; MAX_IMAGES];
        let banks = (0u8..).map_while(Bank::new);
        for (bank, (name, crc)) in banks.zip(names.iter_mut().zip(crcs.iter_mut())) {
            let at = OFF_DIR + bank.index() * ENTRY_LEN;
            let entry = field(header, at, ENTRY_LEN);
            if u32::from(bank.number()) < count.get() {
                *name = text(field(entry, 0, ENTRY_NAME_LEN)).ok_or(TableError::Text {
                    field: TextField::ImageName(bank),
                    offset: at,
                })?;
                *crc = le32(entry, ENTRY_OFF_CRC);
                if le32(entry, ENTRY_OFF_RESERVED) != 0 {
                    return Err(TableError::Reserved {
                        offset: at + ENTRY_OFF_RESERVED,
                    });
                }
            } else if entry.iter().any(|&b| b != 0) {
                return Err(TableError::Reserved { offset: at });
            }
        }
        if field(header, OFF_RESERVED, RESERVED_LEN)
            .iter()
            .any(|&b| b != 0)
        {
            return Err(TableError::Reserved {
                offset: OFF_RESERVED,
            });
        }

        // depth <= 2^16 and count <= 4, so this cannot overflow even a
        // 32-bit usize; checked anyway, because it sizes a slice.
        let truncated = |needed| TableError::Truncated {
            count: count.get(),
            depth,
            needed,
            available: bytes.len(),
        };
        let data_len = (count.get() as usize)
            .checked_mul(depth as usize)
            .ok_or(truncated(usize::MAX))?;
        let needed = HEADER_LEN
            .checked_add(data_len)
            .ok_or(truncated(usize::MAX))?;
        let data = bytes.get(HEADER_LEN..needed).ok_or(truncated(needed))?;

        Ok(Self {
            depth,
            width,
            count,
            part,
            names,
            crcs,
            data,
        })
    }

    /// Words per image.
    pub const fn depth(&self) -> u32 {
        self.depth
    }

    /// Bits per word.
    pub const fn width(&self) -> Width {
        self.width
    }

    /// How many images the table holds.
    pub const fn count(&self) -> ImageCount {
        self.count
    }

    /// The target part name the table was built for.
    pub const fn part(&self) -> &'a str {
        self.part
    }

    /// The table's header fields, owned, and which images are intact: a
    /// CRC over each image's words (at most 16 KiB in all).
    pub fn summary(&self) -> TableSummary {
        let mut part = [0u8; PART_LEN];
        for (dst, src) in part.iter_mut().zip(self.part.bytes()) {
            *dst = src;
        }
        let mut intact = 0;
        for bank in (0u8..).map_while(Bank::new) {
            if self
                .image(bank)
                .is_some_and(|i| crate::loader::verify(&i).is_ok())
            {
                intact |= 1 << bank.index();
            }
        }
        TableSummary {
            depth: self.depth,
            width: self.width,
            count: self.count,
            crcs: self.crcs,
            part,
            intact,
        }
    }

    /// Image `bank`, or `None` if the table holds fewer images.
    pub fn image(&self, bank: Bank) -> Option<Image<'a>> {
        let depth = self.depth as usize;
        let start = bank.index().checked_mul(depth)?;
        let words = self.data.get(start..start.checked_add(depth)?)?;
        Some(Image {
            bank,
            name: self.names.get(bank.index()).copied()?,
            crc32: self.crcs.get(bank.index()).copied()?,
            width: self.width,
            words,
        })
    }
}

impl<'a> Image<'a> {
    /// Which bank this is.
    pub const fn bank(&self) -> Bank {
        self.bank
    }

    /// The source file name it was built from.
    pub const fn name(&self) -> &'a str {
        self.name
    }

    /// The CRC-32 the table records for its words.
    pub const fn crc32(&self) -> u32 {
        self.crc32
    }

    /// Bits per word.
    pub const fn width(&self) -> Width {
        self.width
    }

    /// The words, one per byte; exactly the table's depth of them.
    pub const fn words(&self) -> &'a [u8] {
        self.words
    }
}

/// `len` bytes at `at` of a fixed-size block. Only ever called with the
/// layout's constant offsets, which tile the header (`layout.rs` asserts it
/// at compile time); an offset past the end yields an empty field, which the
/// callers reject, rather than a panic.
fn field(block: &[u8], at: usize, len: usize) -> &[u8] {
    block.get(at..at.saturating_add(len)).unwrap_or(&[])
}

/// The little-endian `u32` at `at`; zero if out of range (see [`field`]).
fn le32(block: &[u8], at: usize) -> u32 {
    field(block, at, 4)
        .first_chunk::<4>()
        .map_or(0, |b| u32::from_le_bytes(*b))
}

/// A NUL-terminated, NUL-padded printable-ASCII field.
fn text(field: &[u8]) -> Option<&str> {
    let end = field.iter().position(|&b| b == 0)?;
    let (s, pad) = field.split_at_checked(end)?;
    let printable = s.iter().all(|&b| b == b' ' || b.is_ascii_graphic());
    if !printable || pad.iter().any(|&b| b != 0) {
        return None;
    }
    core::str::from_utf8(s).ok()
}
