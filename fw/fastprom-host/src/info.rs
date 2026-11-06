//! FastPROM's `INFO` answer, decoded (`fastprom_core::update::info` has the
//! layout, and builds it on the device).
//!
//! Formats 1 and 2 are read: a board still running firmware from before
//! format 2 answers 1, which says nothing about whether the table's images
//! are intact ([`TableInfo::intact`] is then `None`).

use serde::{Deserialize, Serialize};

use fastprom_core::header::{BoardRev, Part, Version, BOARD_REV_LEN, PART_LEN};
use fastprom_core::update::info::{
    FLAG_APP_COMMITTED, FLAG_MANIFEST_COMMITTED, FLAG_TABLE_INTACT, FLAG_TABLE_VALID, INFO_LEN,
    INFO_VERSION, OFF_APP, OFF_BANK, OFF_BOARD_REV, OFF_FABRIC_ID, OFF_FLAGS, OFF_FW_VERSION,
    OFF_INTACT, OFF_MANIFEST, OFF_MODE, OFF_TABLE,
};

/// Which binary answered.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    /// The application: the socket is served (or failed), USB alongside.
    Application,
    /// The DFU stage, in DFU mode.
    Dfu,
}

/// The application header in flash, when it is well formed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppInfo {
    pub committed: bool,
    pub version: String,
    pub board_rev: String,
    pub fabric_id: u32,
    pub image_len: u32,
    pub image_crc: u32,
}

/// The fabric manifest in flash, when it is well formed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FabricInfo {
    pub committed: bool,
    pub part: String,
    pub board_rev: String,
    pub fabric_id: u32,
    pub len: u32,
    pub crc: u32,
}

/// The image table's header, when it parses.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TableInfo {
    pub depth: u32,
    pub width: u8,
    pub count: u8,
    /// One per image in the table.
    pub crcs: Vec<u32>,
    pub part: String,
    /// One per image: its words match its CRC, as the firmware checks them
    /// before serving it. `None` from a format 1 answer, which does not
    /// say. A header written whole over images a power cut left half
    /// written parses; this is what says the images are not whole.
    pub intact: Option<Vec<bool>>,
}

impl TableInfo {
    /// Every image is known to be intact.
    pub fn all_intact(&self) -> bool {
        self.intact.as_ref().is_some_and(|v| v.iter().all(|&ok| ok))
    }
}

/// Everything `INFO` reports.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceInfo {
    pub mode: Mode,
    /// The switch's position at boot, 0 to 3, if it was read.
    pub bank: Option<u8>,
    /// The running binary's version and board revision.
    pub fw_version: String,
    pub board_rev: String,
    /// The fabric interface the running binary expects.
    pub fabric_id: u32,
    pub app: Option<AppInfo>,
    pub fabric: Option<FabricInfo>,
    pub table: Option<TableInfo>,
}

fn bytes<const N: usize>(b: &[u8], at: usize) -> [u8; N] {
    let mut out = [0u8; N];
    if let Some(s) = b.get(at..at + N) {
        out.copy_from_slice(s);
    }
    out
}

fn word(b: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(bytes(b, at))
}

fn version(b: &[u8], at: usize) -> Result<String, String> {
    Version::from_bytes(bytes(b, at))
        .map(|v| v.to_string())
        .ok_or_else(|| format!("a malformed version at {at:#x}"))
}

fn rev(b: &[u8], at: usize) -> Result<String, String> {
    BoardRev::from_bytes(bytes::<BOARD_REV_LEN>(b, at))
        .map(|r| r.as_str().to_owned())
        .ok_or_else(|| format!("a malformed board revision at {at:#x}"))
}

/// A NUL-padded text field that may also be empty.
fn text(b: &[u8]) -> String {
    let end = b.iter().position(|&c| c == 0).unwrap_or(b.len());
    String::from_utf8_lossy(&b[..end]).into_owned()
}

impl DeviceInfo {
    /// Decode an answer.
    ///
    /// # Errors
    ///
    /// A message, for an answer of another length or format version, or
    /// with a field the device would never send.
    pub fn decode(b: &[u8]) -> Result<Self, String> {
        if b.len() != INFO_LEN {
            return Err(format!("INFO answered {} bytes, not {INFO_LEN}", b.len()));
        }
        let format = b[0];
        if !(1..=INFO_VERSION).contains(&format) {
            return Err(format!(
                "INFO format {format}; this page reads formats 1 to {INFO_VERSION}"
            ));
        }
        let mode = match b[OFF_MODE] {
            0 => Mode::Application,
            1 => Mode::Dfu,
            m => return Err(format!("INFO mode {m}")),
        };
        let bank = match b[OFF_BANK] {
            0xFF => None,
            n @ 0..=3 => Some(n),
            n => return Err(format!("INFO switch position {n}")),
        };
        let flags = b[OFF_FLAGS];
        // A section the device could not decode is all zero.
        let present = |at: usize, len: usize| b[at..at + len].iter().any(|&x| x != 0);

        let app = if present(OFF_APP, 8 + 8 + 12) {
            Some(AppInfo {
                committed: flags & FLAG_APP_COMMITTED != 0,
                version: version(b, OFF_APP)?,
                board_rev: rev(b, OFF_APP + 8)?,
                fabric_id: word(b, OFF_APP + 16),
                image_len: word(b, OFF_APP + 20),
                image_crc: word(b, OFF_APP + 24),
            })
        } else {
            None
        };
        let fabric = if present(OFF_MANIFEST, PART_LEN + 8 + 12) {
            let part = Part::from_bytes(bytes::<PART_LEN>(b, OFF_MANIFEST))
                .ok_or("a malformed part in the manifest")?;
            Some(FabricInfo {
                committed: flags & FLAG_MANIFEST_COMMITTED != 0,
                part: part.as_str().to_owned(),
                board_rev: rev(b, OFF_MANIFEST + PART_LEN)?,
                fabric_id: word(b, OFF_MANIFEST + PART_LEN + 8),
                len: word(b, OFF_MANIFEST + PART_LEN + 12),
                crc: word(b, OFF_MANIFEST + PART_LEN + 16),
            })
        } else {
            None
        };
        if format >= 2 && flags & FLAG_TABLE_INTACT != 0 && flags & FLAG_TABLE_VALID == 0 {
            return Err("INFO says the table's images are intact but its header is not".into());
        }
        let table = if flags & FLAG_TABLE_VALID != 0 {
            let count = b[OFF_TABLE + 5];
            if !matches!(count, 1 | 2 | 4) {
                return Err(format!("INFO table count {count}"));
            }
            Some(TableInfo {
                depth: word(b, OFF_TABLE),
                width: b[OFF_TABLE + 4],
                count,
                crcs: (0..usize::from(count))
                    .map(|i| word(b, OFF_TABLE + 8 + 4 * i))
                    .collect(),
                part: text(&b[OFF_TABLE + 24..OFF_TABLE + 52]),
                intact: (format >= 2).then(|| {
                    let mask = b[OFF_INTACT];
                    (0..count).map(|i| mask & (1 << i) != 0).collect()
                }),
            })
        } else {
            None
        };
        Ok(Self {
            mode,
            bank,
            fw_version: version(b, OFF_FW_VERSION)?,
            board_rev: rev(b, OFF_BOARD_REV)?,
            fabric_id: word(b, OFF_FABRIC_ID),
            app,
            fabric,
            table,
        })
    }
}
