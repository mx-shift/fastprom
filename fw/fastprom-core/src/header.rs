//! The application header and the fabric manifest: what the DFU stage and
//! the update core know about the two payloads they cannot check at boot.
//!
//! Both are one 256-byte flash page of little-endian fields, and both start
//! with a **commit word**: the magic is the last thing programmed, by the
//! update core's `COMMIT`, after the payload's CRC-32 has been checked
//! against the page. Until then the word reads erased (`0xFFFFFFFF`), and an
//! erased or damaged commit word means "not valid", whatever the rest of the
//! page says. NOR flash programs by clearing bits, so a power cut while the
//! commit word itself is programmed leaves some of its bits still set, which
//! is not the magic either. The CRC over each page skips the commit word, so
//! programming it does not change the CRC.
//!
//! # Application header (at `memmap::APP_BASE`, version 1)
//!
//! | offset  | field                                                   |
//! |---------|---------------------------------------------------------|
//! | `0x000` | commit word: [`APP_MAGIC`] (`'FPAP'`), or erased        |
//! | `0x004` | header version, 1                                       |
//! | `0x008` | image length: bytes after the header, 1 to 48 KiB - 256 |
//! | `0x00c` | CRC-32 of those bytes                                   |
//! | `0x010` | firmware version: major, minor, patch (`u16` each), 0   |
//! | `0x018` | board revision (`hw/REVISION`), 8 bytes ASCII, NUL padded |
//! | `0x020` | fabric ID the firmware expects ([`crate::regmap::FABRIC_ID`]) |
//! | `0x024` | reserved, zero, to `0x0fc`                              |
//! | `0x0fc` | header CRC: CRC-32 of bytes `0x004..0x0fc`              |
//!
//! The application's first instruction is at `0x100`, straight after it
//! (`memmap::APP_ENTRY`). The firmware links the header with the fields it
//! knows at compile time and zero length and CRCs; `tools/fw_stamp.py`
//! fills in the length and both CRCs after linking.
//!
//! # Fabric manifest (at `memmap::MANIFEST_BASE`, version 1)
//!
//! | offset  | field                                                   |
//! |---------|---------------------------------------------------------|
//! | `0x000` | commit word: [`MANIFEST_MAGIC`] (`'FPMF'`), or erased   |
//! | `0x004` | manifest version, 1                                     |
//! | `0x008` | bitstream length, 1 to 100 KiB                          |
//! | `0x00c` | CRC-32 of the bitstream                                 |
//! | `0x010` | part (`tools/partdb.yaml` key), 16 bytes ASCII, NUL padded |
//! | `0x020` | board revision, 8 bytes ASCII, NUL padded               |
//! | `0x028` | fabric ID ([`crate::regmap::FABRIC_ID`])                |
//! | `0x02c` | reserved, zero, to `0x0fc`                              |
//! | `0x0fc` | manifest CRC: CRC-32 of bytes `0x004..0x0fc`            |
//!
//! `tools/fw_stamp.py manifest` writes one for a bitstream.

use thiserror::Error;

use crate::crc32::crc32;
use crate::memmap::{APP_HEADER_LEN, APP_LEN, FABRIC_LEN};

/// One flash page: the size of a header and of a manifest.
pub const PAGE_LEN: usize = 256;
/// What erased flash reads as.
pub const ERASED: u32 = u32::MAX;

/// `'FPAP'`: a committed application header.
pub const APP_MAGIC: u32 = 0x4650_4150;
/// The application header format this firmware reads.
pub const APP_VERSION: u32 = 1;
/// The longest application image: its region less the header.
pub const APP_IMAGE_MAX: u32 = APP_LEN - APP_HEADER_LEN;

/// `'FPMF'`: a committed fabric manifest.
pub const MANIFEST_MAGIC: u32 = 0x4650_4D46;
/// The manifest format this firmware reads.
pub const MANIFEST_VERSION: u32 = 1;
/// The longest bitstream: its region.
pub const FABRIC_IMAGE_MAX: u32 = FABRIC_LEN;

/// `mscratch` holds this across a soft reset when the application asks the
/// DFU stage to stay in DFU mode (`'DFUR'`).
pub const DFU_REQUEST: u32 = 0x4446_5552;

/// Offsets common to both pages.
pub const OFF_COMMIT: usize = 0x000;
pub const OFF_VERSION: usize = 0x004;
pub const OFF_LEN: usize = 0x008;
pub const OFF_CRC: usize = 0x00C;
pub const OFF_PAGE_CRC: usize = 0x0FC;
/// Application header fields.
pub const APP_OFF_FW_VERSION: usize = 0x010;
pub const APP_OFF_BOARD_REV: usize = 0x018;
pub const APP_OFF_FABRIC_ID: usize = 0x020;
pub const APP_OFF_RESERVED: usize = 0x024;
/// Manifest fields.
pub const MAN_OFF_PART: usize = 0x010;
pub const MAN_OFF_BOARD_REV: usize = 0x020;
pub const MAN_OFF_FABRIC_ID: usize = 0x028;
pub const MAN_OFF_RESERVED: usize = 0x02C;

/// Bytes in a board revision field.
pub const BOARD_REV_LEN: usize = 8;
/// Bytes in a part field.
pub const PART_LEN: usize = 16;

const _: () = {
    assert!(APP_HEADER_LEN as usize == PAGE_LEN);
    assert!(APP_OFF_FW_VERSION + 8 == APP_OFF_BOARD_REV);
    assert!(APP_OFF_BOARD_REV + BOARD_REV_LEN == APP_OFF_FABRIC_ID);
    assert!(APP_OFF_FABRIC_ID + 4 == APP_OFF_RESERVED);
    assert!(MAN_OFF_PART + PART_LEN == MAN_OFF_BOARD_REV);
    assert!(MAN_OFF_BOARD_REV + BOARD_REV_LEN == MAN_OFF_FABRIC_ID);
    assert!(MAN_OFF_FABRIC_ID + 4 == MAN_OFF_RESERVED);
    assert!(OFF_PAGE_CRC + 4 == PAGE_LEN);
};

/// The application half of the DFU stage's power-up check: is there an
/// application to jump to? Three words of its header, no CRC (the image
/// was verified when it was committed).
pub const fn boot_sane(commit: u32, version: u32, image_len: u32) -> bool {
    commit == APP_MAGIC && version == APP_VERSION && image_len != 0 && image_len <= APP_IMAGE_MAX
}

/// The fabric half: is there a committed bitstream for the application to
/// run on? The same three words of the manifest, no CRC (the bitstream was
/// verified when it was committed). Without it the application would come
/// up on a fabric the FCB cannot configure, which has no USB (the fabric
/// enables it): a fabric update cut part way leaves exactly that, its
/// manifest erased first.
pub const fn manifest_sane(commit: u32, version: u32, fabric_len: u32) -> bool {
    commit == MANIFEST_MAGIC
        && version == MANIFEST_VERSION
        && fabric_len != 0
        && fabric_len <= FABRIC_IMAGE_MAX
}

/// The DFU stage's whole power-up check: jump to the application only if
/// its header and the fabric manifest are both sane; otherwise DFU mode,
/// on the stage's own USB-only fabric. Each is the commit word, version
/// and length at offsets 0, 4 and 8 of its page. `fastprom-dfu`'s
/// `boot.rs` is this function in assembly, with these constants.
pub const fn boot_application(app: [u32; 3], manifest: [u32; 3]) -> bool {
    boot_sane(app[0], app[1], app[2]) && manifest_sane(manifest[0], manifest[1], manifest[2])
}

/// A firmware version, from Cargo's.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Version {
    pub major: u16,
    pub minor: u16,
    pub patch: u16,
}

impl Version {
    /// The eight bytes the header and `INFO` carry.
    pub const fn to_bytes(self) -> [u8; 8] {
        let [a, b] = self.major.to_le_bytes();
        let [c, d] = self.minor.to_le_bytes();
        let [e, f] = self.patch.to_le_bytes();
        [a, b, c, d, e, f, 0, 0]
    }

    /// Back from [`Version::to_bytes`]; `None` if the padding is not zero.
    pub const fn from_bytes(b: [u8; 8]) -> Option<Self> {
        if b[6] != 0 || b[7] != 0 {
            return None;
        }
        Some(Self {
            major: u16::from_le_bytes([b[0], b[1]]),
            minor: u16::from_le_bytes([b[2], b[3]]),
            patch: u16::from_le_bytes([b[4], b[5]]),
        })
    }
}

impl core::fmt::Display for Version {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)
    }
}

/// A short text field: 1 to `N` printable ASCII characters (no spaces),
/// NUL padded to `N` bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Text<const N: usize>([u8; N]);

/// A board revision, as `hw/REVISION` gives it (`"A"`).
pub type BoardRev = Text<BOARD_REV_LEN>;
/// A part, as `tools/partdb.yaml` names it (`"512x8"`).
pub type Part = Text<PART_LEN>;

impl<const N: usize> Text<N> {
    /// The field for `s`, if it is one.
    pub const fn new(s: &str) -> Option<Self> {
        let s = s.as_bytes();
        if s.is_empty() || s.len() > N {
            return None;
        }
        let mut out = [0u8; N];
        let mut i = 0;
        while i < s.len() {
            if !s[i].is_ascii_graphic() {
                return None;
            }
            out[i] = s[i];
            i += 1;
        }
        Some(Self(out))
    }

    /// The field as stored, if it is well formed.
    pub fn from_bytes(b: [u8; N]) -> Option<Self> {
        let len = b.iter().position(|&c| c == 0).unwrap_or(N);
        let (text, pad) = b.split_at_checked(len)?;
        let ok = !text.is_empty()
            && text.iter().all(u8::is_ascii_graphic)
            && pad.iter().all(|&c| c == 0);
        ok.then_some(Self(b))
    }

    /// The stored bytes.
    pub const fn to_bytes(self) -> [u8; N] {
        self.0
    }

    /// The text.
    pub fn as_str(&self) -> &str {
        let len = self.0.iter().position(|&c| c == 0).unwrap_or(N);
        self.0
            .get(..len)
            .and_then(|t| core::str::from_utf8(t).ok())
            .unwrap_or("")
    }
}

/// What the commit word of a well-formed header or manifest says.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Commit {
    /// The magic: the payload was checked and committed.
    Committed,
    /// Erased: written, not yet committed. Not valid.
    Pending,
}

/// A header or manifest page that is not well formed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum HeaderError {
    #[error("commit word reads {found:#010x}: neither the magic nor erased")]
    BadMagic { found: u32 },
    #[error("format version {found}; this firmware reads version {expected}")]
    Version { found: u32, expected: u32 },
    #[error("payload length {len} bytes is not 1 to {max}")]
    Length { len: u32, max: u32 },
    #[error("page CRC is {computed:#010x}, but the page stores {stored:#010x}")]
    PageCrc { computed: u32, stored: u32 },
    #[error("the {field} field is not 1 to {len} printable characters, NUL padded")]
    Text { field: &'static str, len: usize },
    #[error("the firmware version's padding is not zero")]
    FwVersion,
    #[error("reserved bytes at offset {offset:#05x} are not zero")]
    Reserved { offset: usize },
}

/// The application header's fields (all but the commit word and the CRC).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AppHeader {
    /// Bytes of code and data after the header.
    pub image_len: u32,
    /// Their CRC-32.
    pub image_crc: u32,
    /// The firmware's version.
    pub fw_version: Version,
    /// The board revision it was built for.
    pub board_rev: BoardRev,
    /// The fabric interface it expects.
    pub fabric_id: u32,
}

/// The fabric manifest's fields.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Manifest {
    /// Bytes of bitstream.
    pub fabric_len: u32,
    /// Their CRC-32.
    pub fabric_crc: u32,
    /// The part the bitstream was built for.
    pub part: Part,
    /// The board revision it was built for.
    pub board_rev: BoardRev,
    /// Its interface ([`crate::regmap::FABRIC_ID`]).
    pub fabric_id: u32,
}

impl AppHeader {
    /// The page with `commit` in the commit word and every field, but a
    /// zero page CRC. `const`, so that the firmware can link its own header
    /// (with a zero length and CRC for `tools/fw_stamp.py` to fill in).
    pub const fn fields(&self, commit: u32) -> [u8; PAGE_LEN] {
        let mut p = [0u8; PAGE_LEN];
        put32(&mut p, OFF_COMMIT, commit);
        put32(&mut p, OFF_VERSION, APP_VERSION);
        put32(&mut p, OFF_LEN, self.image_len);
        put32(&mut p, OFF_CRC, self.image_crc);
        put(&mut p, APP_OFF_FW_VERSION, &self.fw_version.to_bytes());
        put(&mut p, APP_OFF_BOARD_REV, &self.board_rev.to_bytes());
        put32(&mut p, APP_OFF_FABRIC_ID, self.fabric_id);
        p
    }

    /// The whole page, page CRC included.
    pub fn encode(&self, commit: Commit) -> [u8; PAGE_LEN] {
        seal(self.fields(commit_word(commit, APP_MAGIC)))
    }

    /// Check and decode a header page.
    pub fn decode(page: &[u8; PAGE_LEN]) -> Result<(Commit, Self), HeaderError> {
        let commit = check_page(page, APP_MAGIC, APP_VERSION)?;
        let image_len = get32(page, OFF_LEN);
        if image_len == 0 || image_len > APP_IMAGE_MAX {
            return Err(HeaderError::Length {
                len: image_len,
                max: APP_IMAGE_MAX,
            });
        }
        let fw_version = Version::from_bytes(get::<8>(page, APP_OFF_FW_VERSION))
            .ok_or(HeaderError::FwVersion)?;
        let board_rev =
            Text::from_bytes(get(page, APP_OFF_BOARD_REV)).ok_or(HeaderError::Text {
                field: "board revision",
                len: BOARD_REV_LEN,
            })?;
        reserved(page, APP_OFF_RESERVED)?;
        Ok((
            commit,
            Self {
                image_len,
                image_crc: get32(page, OFF_CRC),
                fw_version,
                board_rev,
                fabric_id: get32(page, APP_OFF_FABRIC_ID),
            },
        ))
    }
}

impl Manifest {
    /// The page with `commit` in the commit word, but a zero page CRC.
    pub const fn fields(&self, commit: u32) -> [u8; PAGE_LEN] {
        let mut p = [0u8; PAGE_LEN];
        put32(&mut p, OFF_COMMIT, commit);
        put32(&mut p, OFF_VERSION, MANIFEST_VERSION);
        put32(&mut p, OFF_LEN, self.fabric_len);
        put32(&mut p, OFF_CRC, self.fabric_crc);
        put(&mut p, MAN_OFF_PART, &self.part.to_bytes());
        put(&mut p, MAN_OFF_BOARD_REV, &self.board_rev.to_bytes());
        put32(&mut p, MAN_OFF_FABRIC_ID, self.fabric_id);
        p
    }

    /// The whole page, page CRC included.
    pub fn encode(&self, commit: Commit) -> [u8; PAGE_LEN] {
        seal(self.fields(commit_word(commit, MANIFEST_MAGIC)))
    }

    /// Check and decode a manifest page.
    pub fn decode(page: &[u8; PAGE_LEN]) -> Result<(Commit, Self), HeaderError> {
        let commit = check_page(page, MANIFEST_MAGIC, MANIFEST_VERSION)?;
        let fabric_len = get32(page, OFF_LEN);
        if fabric_len == 0 || fabric_len > FABRIC_IMAGE_MAX {
            return Err(HeaderError::Length {
                len: fabric_len,
                max: FABRIC_IMAGE_MAX,
            });
        }
        let part = Text::from_bytes(get(page, MAN_OFF_PART)).ok_or(HeaderError::Text {
            field: "part",
            len: PART_LEN,
        })?;
        let board_rev =
            Text::from_bytes(get(page, MAN_OFF_BOARD_REV)).ok_or(HeaderError::Text {
                field: "board revision",
                len: BOARD_REV_LEN,
            })?;
        reserved(page, MAN_OFF_RESERVED)?;
        Ok((
            commit,
            Self {
                fabric_len,
                fabric_crc: get32(page, OFF_CRC),
                part,
                board_rev,
                fabric_id: get32(page, MAN_OFF_FABRIC_ID),
            },
        ))
    }
}

/// The CRC a page stores: over everything but the commit word and itself.
pub fn page_crc(page: &[u8; PAGE_LEN]) -> u32 {
    crc32(page.get(OFF_VERSION..OFF_PAGE_CRC).unwrap_or(&[]))
}

const fn commit_word(commit: Commit, magic: u32) -> u32 {
    match commit {
        Commit::Committed => magic,
        Commit::Pending => ERASED,
    }
}

fn seal(mut page: [u8; PAGE_LEN]) -> [u8; PAGE_LEN] {
    let crc = page_crc(&page);
    put32(&mut page, OFF_PAGE_CRC, crc);
    page
}

/// The commit word, version and page CRC, common to both pages.
fn check_page(page: &[u8; PAGE_LEN], magic: u32, version: u32) -> Result<Commit, HeaderError> {
    let commit = match get32(page, OFF_COMMIT) {
        m if m == magic => Commit::Committed,
        ERASED => Commit::Pending,
        found => return Err(HeaderError::BadMagic { found }),
    };
    let found = get32(page, OFF_VERSION);
    if found != version {
        return Err(HeaderError::Version {
            found,
            expected: version,
        });
    }
    let computed = page_crc(page);
    let stored = get32(page, OFF_PAGE_CRC);
    if computed != stored {
        return Err(HeaderError::PageCrc { computed, stored });
    }
    Ok(commit)
}

fn reserved(page: &[u8; PAGE_LEN], from: usize) -> Result<(), HeaderError> {
    let tail = page.get(from..OFF_PAGE_CRC).unwrap_or(&[]);
    match tail.iter().position(|&b| b != 0) {
        Some(i) => Err(HeaderError::Reserved { offset: from + i }),
        None => Ok(()),
    }
}

const fn put(p: &mut [u8; PAGE_LEN], at: usize, bytes: &[u8]) {
    let mut i = 0;
    while i < bytes.len() && at + i < PAGE_LEN {
        p[at + i] = bytes[i];
        i += 1;
    }
}

const fn put32(p: &mut [u8; PAGE_LEN], at: usize, v: u32) {
    put(p, at, &v.to_le_bytes());
}

/// `N` bytes at `at`; zeros past the end (only constant offsets reach it).
fn get<const N: usize>(p: &[u8; PAGE_LEN], at: usize) -> [u8; N] {
    p.get(at..at.saturating_add(N))
        .and_then(|s| s.first_chunk::<N>())
        .copied()
        .unwrap_or([0; N])
}

/// The little-endian word at `at`.
pub fn get32(p: &[u8; PAGE_LEN], at: usize) -> u32 {
    u32::from_le_bytes(get(p, at))
}
