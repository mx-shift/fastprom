//! Every way the power-up sequence can fail.
//!
//! Each variant carries the values that explain it, and each leaf maps to a
//! stable two-digit [`FailureCode`] that the board firmware prints on UART0
//! (`docs/06-build-and-flash.md` has the list). Whatever the error, the
//! socket outputs are left released: the fabric's READY bit is only ever
//! set as the last step of a load that passed every check.

use thiserror::Error;

use crate::bank::Bank;
use crate::layout::{MAGIC, MAX_DEPTH, MIN_DEPTH, VERSION};
use crate::regmap::FABRIC_MAGIC;

/// A stable number per failure, for a terminal or a blink pattern.
///
/// Tens digit: 1 image select, 2 image table, 3 bank, 4 image contents,
/// 5 fabric, 6 clocks (board crate).
pub trait FailureCode {
    /// The code, 10 to 99.
    fn code(&self) -> u8;
}

/// The image-select switch could not be read as one bank.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum SelectError {
    #[error("the image-select switch was sampled zero times")]
    NoSamples,
    #[error(
        "the image-select switch is not steady: sample 0 reads bank {first}, \
         sample {index} reads bank {found}"
    )]
    Unstable {
        first: Bank,
        index: usize,
        found: Bank,
    },
}

/// Which text field of the image table is malformed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextField {
    PartName,
    ImageName(Bank),
}

impl core::fmt::Display for TextField {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::PartName => f.write_str("the part name"),
            Self::ImageName(bank) => write!(f, "image {bank}'s name"),
        }
    }
}

/// The image table in flash is missing, damaged or not one this firmware
/// reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum TableError {
    #[error("the image table region is {len} bytes, shorter than the 256-byte header")]
    TooShort { len: usize },
    #[error(
        "no image table: magic reads {found:#010x}, expected {expected:#010x} ('FPRM'){hint}",
        expected = MAGIC,
        hint = erased_hint(.found)
    )]
    BadMagic { found: u32 },
    #[error("image table format version {found}; this firmware reads version {expected}", expected = VERSION)]
    Version { found: u32 },
    #[error("image table header CRC is {computed:#010x}, but the header stores {stored:#010x}")]
    HeaderCrc { computed: u32, stored: u32 },
    #[error(
        "image depth {depth} words is not a power of two from {min} to {max}",
        min = MIN_DEPTH,
        max = MAX_DEPTH
    )]
    Depth { depth: u32 },
    #[error("word width {width} bits; a bipolar PROM is 4 or 8 bits wide")]
    Width { width: u32 },
    #[error("{count} images; the two-bit image-select switch needs 1, 2 or 4")]
    Count { count: u32 },
    #[error("{field} at offset {offset:#05x} is not NUL-terminated, NUL-padded printable ASCII")]
    Text { field: TextField, offset: usize },
    #[error("reserved bytes at offset {offset:#05x} are not zero")]
    Reserved { offset: usize },
    #[error(
        "{count} images of {depth} words need {needed} bytes of table, \
         but the region holds {available}"
    )]
    Truncated {
        count: u32,
        depth: u32,
        needed: usize,
        available: usize,
    },
}

/// Erased flash reads as ones: say so, since it is the likely cause.
fn erased_hint(found: &u32) -> &'static str {
    if *found == u32::MAX {
        " - is the table region erased?"
    } else {
        ""
    }
}

/// The selected image's contents do not match the table.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum ImageError {
    #[error("image {bank} has CRC {computed:#010x}, but the table says {stored:#010x}")]
    Crc {
        bank: Bank,
        computed: u32,
        stored: u32,
    },
    #[error("image {bank} word {index} is {value:#04x}, wider than the table's {width} bits")]
    WordTooWide {
        bank: Bank,
        index: usize,
        value: u8,
        width: u8,
    },
}

/// The fabric is absent, not FastPROM's, or did not do what was written.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum FabricError {
    #[error(
        "the fabric is not configured with FastPROM's logic: ID reads {found:#010x}, \
         expected {expected:#010x}",
        expected = FABRIC_MAGIC
    )]
    NoFabric { found: u32 },
    #[error("the fabric stopped answering: ID reads {found:#010x} mid-sequence")]
    Lost { found: u32 },
    #[error("the fabric would not release the socket: CTRL reads {ctrl:#x} after writing 0")]
    ReleaseFailed { ctrl: u32 },
    #[error("the load-pointer probe read {found:#x}, which is not a mask of ones")]
    PointerProbe { found: u32 },
    #[error("the fabric's block RAM holds {fabric} words, but the image is {image} words")]
    DepthMismatch { fabric: u32, image: u32 },
    #[error("the load pointer reads {found:#x} after being set to 0")]
    PointerReset { found: u32 },
    #[error("the load pointer reads {found:#x} after word {index}, expected {expected:#x}")]
    Pointer {
        index: usize,
        expected: u32,
        found: u32,
    },
    #[error("READY did not latch: CTRL reads {ctrl:#x} after writing 1")]
    ReadyNotSet { ctrl: u32 },
}

/// The selected image could not be loaded.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum LoadError {
    #[error("bank {bank} is not in the image table, which holds {count} image(s)")]
    BankNotInTable { bank: Bank, count: u32 },
    #[error("checking image {bank}")]
    Image {
        bank: Bank,
        #[source]
        source: ImageError,
    },
    #[error("writing image {bank} into the fabric")]
    Fabric {
        bank: Bank,
        #[source]
        source: FabricError,
    },
}

/// The power-up sequence failed after the socket outputs were released.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum BootError {
    #[error("reading the image-select switch")]
    Select(#[from] SelectError),
    #[error("reading the image table")]
    Table(#[from] TableError),
    #[error("loading the selected image")]
    Load(#[from] LoadError),
}

impl FailureCode for SelectError {
    fn code(&self) -> u8 {
        match self {
            Self::NoSamples => 10,
            Self::Unstable { .. } => 11,
        }
    }
}

impl FailureCode for TableError {
    fn code(&self) -> u8 {
        match self {
            Self::TooShort { .. } => 20,
            Self::BadMagic { .. } => 21,
            Self::Version { .. } => 22,
            Self::HeaderCrc { .. } => 23,
            Self::Depth { .. } => 24,
            Self::Width { .. } => 25,
            Self::Count { .. } => 26,
            Self::Text { .. } => 27,
            Self::Reserved { .. } => 28,
            Self::Truncated { .. } => 29,
        }
    }
}

impl FailureCode for ImageError {
    fn code(&self) -> u8 {
        match self {
            Self::Crc { .. } => 40,
            Self::WordTooWide { .. } => 41,
        }
    }
}

impl FailureCode for FabricError {
    fn code(&self) -> u8 {
        match self {
            Self::NoFabric { .. } => 50,
            Self::Lost { .. } => 51,
            Self::ReleaseFailed { .. } => 52,
            Self::PointerProbe { .. } => 53,
            Self::DepthMismatch { .. } => 54,
            Self::PointerReset { .. } => 55,
            Self::Pointer { .. } => 56,
            Self::ReadyNotSet { .. } => 57,
        }
    }
}

impl FailureCode for LoadError {
    fn code(&self) -> u8 {
        match self {
            Self::BankNotInTable { .. } => 30,
            Self::Image { source, .. } => source.code(),
            Self::Fabric { source, .. } => source.code(),
        }
    }
}

impl FailureCode for BootError {
    fn code(&self) -> u8 {
        match self {
            Self::Select(e) => e.code(),
            Self::Table(e) => e.code(),
            Self::Load(e) => e.code(),
        }
    }
}
