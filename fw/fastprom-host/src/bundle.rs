//! The update bundle published with the page (`tools/mkbundle.py` writes
//! it): `manifest.json`, the application binary, and a fabric bitstream
//! and manifest page per part it was built for; and, for UART recovery
//! only, the DFU stage, the recovery loader and the option bytes. USB never
//! writes the DFU stage or the option bytes: only recovery does.
//!
//! ```json
//! {
//!   "format": 1,
//!   "board_rev": "A",
//!   "firmware": { "file": "fastprom-fw-0.1.0.bin", "version": "0.1.0",
//!                 "board_rev": "A", "fabric_id": 1, "size": 30976,
//!                 "crc32": "0x1234abcd", "image_len": 30720,
//!                 "image_crc32": "0x89abcdef" },
//!   "fabrics": [ { "part": "512x8", "file": "fabric-512x8.bin",
//!                  "manifest": "fabric-512x8.manifest.bin", "board_rev": "A",
//!                  "fabric_id": 1, "size": 99944, "crc32": "0x..." } ],
//!   "recovery": { "loader": { "file": "fastprom-recovery-0.1.0.bin", "size": 4372,
//!                             "crc32": "0x...", "version": "0.1.0", "board_rev": "A" },
//!                 "dfu": { "file": "fastprom-dfu-0.1.0.bin", "size": 23300,
//!                          "crc32": "0x...", "version": "0.1.0", "board_rev": "A" },
//!                 "option_bytes": { "file": "option-bytes.bin", "size": 128,
//!                                   "crc32": "0x..." } },
//!   "notes": [ "..." ]
//! }
//! ```
//!
//! `recovery` is absent from a bundle built without the firmware.
//!
//! Every file is checked against the manifest and against its own header
//! or manifest page before anything is written.

use serde::{Deserialize, Serialize};

use fastprom_core::crc32::crc32;
use fastprom_core::header::{AppHeader, Manifest as FabricManifest, PAGE_LEN};
use fastprom_core::memmap::{APP_LEN, DFU_LEN, FABRIC_LEN};
use fastprom_core::recovery::{LoaderHeader, OPTION_BYTES};

use crate::info::DeviceInfo;

/// The manifest format this page reads.
pub const FORMAT: u32 = 1;

/// The application binary.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Firmware {
    pub file: String,
    pub version: String,
    pub board_rev: String,
    pub fabric_id: u32,
    pub size: u32,
    pub crc32: String,
    pub image_len: u32,
    pub image_crc32: String,
}

/// One part's fabric.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Fabric {
    pub part: String,
    pub file: String,
    pub manifest: String,
    pub board_rev: String,
    pub fabric_id: u32,
    pub size: u32,
    pub crc32: String,
}

/// A file of the recovery set.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct File {
    pub file: String,
    pub size: u32,
    pub crc32: String,
    /// The firmware version and board revision it was built for (the
    /// loader and the DFU stage; not the option bytes).
    #[serde(default)]
    pub version: Option<String>,
    #[serde(default)]
    pub board_rev: Option<String>,
}

/// What UART recovery writes that USB never does.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Recovery {
    /// The recovery loader, uploaded into SRAM through the mask ROM.
    pub loader: File,
    /// The DFU stage, for `memmap::DFU_BASE`.
    pub dfu: File,
    /// The 128 option bytes (`fastprom_core::recovery::OPTION_BYTES`).
    pub option_bytes: File,
}

/// `manifest.json`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Bundle {
    pub format: u32,
    pub board_rev: String,
    pub firmware: Option<Firmware>,
    #[serde(default)]
    pub fabrics: Vec<Fabric>,
    #[serde(default)]
    pub recovery: Option<Recovery>,
    #[serde(default)]
    pub notes: Vec<String>,
}

fn hex32(s: &str) -> Result<u32, String> {
    let d = s.strip_prefix("0x").unwrap_or(s);
    u32::from_str_radix(d, 16).map_err(|_| format!("{s:?} is not a hex CRC"))
}

/// Read `manifest.json`.
///
/// # Errors
///
/// For JSON that is not a manifest of [`FORMAT`], or one whose entries
/// disagree with it.
pub fn parse(json: &str) -> Result<Bundle, String> {
    let b: Bundle =
        serde_json::from_str(json).map_err(|e| format!("the bundle's manifest: {e}"))?;
    if b.format != FORMAT {
        return Err(format!(
            "bundle format {}; this page reads format {FORMAT}",
            b.format
        ));
    }
    if let Some(f) = &b.firmware {
        hex32(&f.crc32)?;
        hex32(&f.image_crc32)?;
        if f.board_rev != b.board_rev {
            return Err("the bundle's firmware is for another board revision".into());
        }
    }
    for f in &b.fabrics {
        hex32(&f.crc32)?;
        if f.board_rev != b.board_rev {
            return Err(format!(
                "the bundle's {} fabric is for another board revision",
                f.part
            ));
        }
    }
    if let Some(r) = &b.recovery {
        for (what, f) in [("loader", &r.loader), ("DFU stage", &r.dfu)] {
            hex32(&f.crc32)?;
            if f.board_rev.as_deref() != Some(b.board_rev.as_str()) {
                return Err(format!(
                    "the bundle's recovery {what} is for another board revision"
                ));
            }
        }
        hex32(&r.option_bytes.crc32)?;
    }
    Ok(b)
}

fn check_file(entry: &File, data: &[u8]) -> Result<(), String> {
    if data.len() != entry.size as usize || crc32(data) != hex32(&entry.crc32)? {
        return Err(format!(
            "{}: not the file the manifest describes",
            entry.file
        ));
    }
    Ok(())
}

/// Check the recovery loader against its entry and its own header.
///
/// # Errors
///
/// A message naming the first thing that does not hold.
pub fn check_loader(entry: &File, file: &[u8]) -> Result<LoaderHeader, String> {
    check_file(entry, file)?;
    let h = LoaderHeader::check(file).map_err(|e| format!("{}: {e}", entry.file))?;
    if Some(h.fw_version.to_string()) != entry.version
        || Some(h.board_rev.as_str()) != entry.board_rev.as_deref()
    {
        return Err(format!(
            "{}: its header says {} for board {}, the manifest otherwise",
            entry.file,
            h.fw_version,
            h.board_rev.as_str()
        ));
    }
    Ok(h)
}

/// Check the DFU stage against its entry: the size and CRC (it has no
/// header; [`parse`] checked the entry's board revision).
///
/// # Errors
///
/// A message naming the first thing that does not hold.
pub fn check_dfu(entry: &File, file: &[u8]) -> Result<Vec<u8>, String> {
    check_file(entry, file)?;
    if file.is_empty() || file.len() > DFU_LEN as usize {
        return Err(format!(
            "{}: {} bytes, not 1 to the DFU stage's {DFU_LEN}",
            entry.file,
            file.len()
        ));
    }
    let mut data = file.to_vec();
    data.resize(file.len().div_ceil(PAGE_LEN) * PAGE_LEN, 0xFF);
    Ok(data)
}

/// Check the option bytes against their entry, and against what this page
/// (`fastprom-core`) says they must be: the Python tool that wrote them and
/// the Rust that reads them must agree byte for byte.
///
/// # Errors
///
/// A message naming the first thing that does not hold.
pub fn check_option_bytes(entry: &File, file: &[u8]) -> Result<(), String> {
    check_file(entry, file)?;
    if file != OPTION_BYTES {
        return Err(format!(
            "{}: not the option bytes this page writes (fastprom-core recovery::OPTION_BYTES)",
            entry.file
        ));
    }
    Ok(())
}

/// The application binary, checked: what is written at `memmap::APP_BASE`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppPayload {
    /// The file, padded with erased bytes to whole pages.
    pub data: Vec<u8>,
    pub header: AppHeader,
}

/// Check the application binary against its manifest entry and its own
/// header.
///
/// # Errors
///
/// A message naming the first thing that does not hold.
pub fn check_firmware(entry: &Firmware, file: &[u8]) -> Result<AppPayload, String> {
    let what = &entry.file;
    if file.len() != entry.size as usize {
        return Err(format!(
            "{what}: {} bytes, the manifest says {}",
            file.len(),
            entry.size
        ));
    }
    if crc32(file) != hex32(&entry.crc32)? {
        return Err(format!("{what}: its CRC is not the manifest's"));
    }
    if file.len() > APP_LEN as usize {
        return Err(format!(
            "{what}: {} bytes, more than the application's {APP_LEN}",
            file.len()
        ));
    }
    let page: &[u8; PAGE_LEN] = file
        .first_chunk()
        .ok_or_else(|| format!("{what}: shorter than its header"))?;
    let (_, header) = AppHeader::decode(page).map_err(|e| format!("{what}: its header: {e}"))?;
    let end = PAGE_LEN + header.image_len as usize;
    let image = file
        .get(PAGE_LEN..end)
        .ok_or_else(|| format!("{what}: shorter than its header says"))?;
    if crc32(image) != header.image_crc {
        return Err(format!("{what}: the image does not match its header's CRC"));
    }
    if header.image_crc != hex32(&entry.image_crc32)? || header.image_len != entry.image_len {
        return Err(format!("{what}: its header is not the manifest's"));
    }
    if header.board_rev.as_str() != entry.board_rev
        || header.fw_version.to_string() != entry.version
        || header.fabric_id != entry.fabric_id
    {
        return Err(format!(
            "{what}: its header says {} for board {}, the manifest {} for board {}",
            header.fw_version,
            header.board_rev.as_str(),
            entry.version,
            entry.board_rev
        ));
    }
    let mut data = file.to_vec();
    data.resize(file.len().div_ceil(PAGE_LEN) * PAGE_LEN, 0xFF);
    Ok(AppPayload { data, header })
}

/// A fabric, checked: the bitstream for `memmap::FABRIC_BASE` and its
/// manifest page for `memmap::MANIFEST_BASE`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FabricPayload {
    /// The bitstream, padded with erased bytes to whole pages.
    pub data: Vec<u8>,
    pub page: [u8; PAGE_LEN],
    pub manifest: FabricManifest,
}

/// Check a fabric against its manifest entry and its manifest page.
///
/// # Errors
///
/// A message naming the first thing that does not hold.
pub fn check_fabric(
    entry: &Fabric,
    bitstream: &[u8],
    page: &[u8],
) -> Result<FabricPayload, String> {
    let what = &entry.file;
    if bitstream.len() != entry.size as usize || crc32(bitstream) != hex32(&entry.crc32)? {
        return Err(format!("{what}: not the file the manifest describes"));
    }
    if bitstream.len() > FABRIC_LEN as usize {
        return Err(format!("{what}: larger than the fabric's region"));
    }
    let page: [u8; PAGE_LEN] = page
        .try_into()
        .map_err(|_| format!("{}: not one {PAGE_LEN}-byte page", entry.manifest))?;
    let (_, m) = FabricManifest::decode(&page).map_err(|e| format!("{}: {e}", entry.manifest))?;
    if m.fabric_len as usize != bitstream.len() || m.fabric_crc != crc32(bitstream) {
        return Err(format!("{}: does not describe {what}", entry.manifest));
    }
    if m.part.as_str() != entry.part
        || m.board_rev.as_str() != entry.board_rev
        || m.fabric_id != entry.fabric_id
    {
        return Err(format!(
            "{}: says part {} board {}, the bundle part {} board {}",
            entry.manifest,
            m.part.as_str(),
            m.board_rev.as_str(),
            entry.part,
            entry.board_rev
        ));
    }
    let mut data = bitstream.to_vec();
    data.resize(bitstream.len().div_ceil(PAGE_LEN) * PAGE_LEN, 0xFF);
    Ok(FabricPayload {
        data,
        page,
        manifest: m,
    })
}

/// What the bundle offers one payload of this device.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Offer {
    /// Whether the bundle can update it on this device.
    pub available: bool,
    /// What the device has, as text ("0.1.0", "512x8 0x1234abcd"), if known.
    pub current: Option<String>,
    /// What the bundle has.
    pub new: Option<String>,
    /// The device already has exactly this (and it is committed).
    pub up_to_date: bool,
    /// Why not, when not available.
    pub reason: Option<String>,
}

/// The bundle against a device.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UpdatePlan {
    /// Set when nothing may be updated, and why.
    pub refuse: Option<String>,
    pub firmware: Offer,
    pub fabric: Offer,
    /// The part whose fabric is offered.
    pub part: Option<String>,
    /// Where it came from: `"chosen"` (the user), `"manifest"` (the board's
    /// fabric manifest) or `"table"` (its image table, when the manifest
    /// is missing: an interrupted fabric update).
    pub part_source: Option<String>,
}

/// The part a board reports: its fabric manifest's, or, when there is no
/// manifest (an interrupted fabric update erases it first), its image
/// table's, if that is one of `known` (`tools/partdb.yaml`'s keys, which is
/// what the table records). With the source, as [`UpdatePlan::part_source`].
pub fn board_part(info: &DeviceInfo, known: &[String]) -> Option<(String, &'static str)> {
    if let Some(m) = &info.fabric {
        return Some((m.part.clone(), "manifest"));
    }
    info.table
        .as_ref()
        .map(|t| t.part.clone())
        .filter(|p| known.iter().any(|k| k == p))
        .map(|p| (p, "table"))
}

fn not(reason: &str) -> Offer {
    Offer {
        available: false,
        current: None,
        new: None,
        up_to_date: false,
        reason: Some(reason.to_owned()),
    }
}

/// What the bundle can do for this device. `part` is the one the user
/// chose, if any; otherwise the board's ([`board_part`], from `known`
/// parts).
pub fn plan(b: &Bundle, info: &DeviceInfo, part: Option<&str>, known: &[String]) -> UpdatePlan {
    let (part, part_source) = match part {
        Some(p) => (Some(p.to_owned()), Some("chosen")),
        None => match board_part(info, known) {
            Some((p, from)) => (Some(p), Some(from)),
            None => (None, None),
        },
    };
    let part_source = part_source.map(str::to_owned);
    if b.board_rev != info.board_rev {
        let why = format!(
            "this update is for board revision {}, and the board is revision {}: \
             nothing will be written",
            b.board_rev, info.board_rev
        );
        return UpdatePlan {
            refuse: Some(why.clone()),
            firmware: not(&why),
            fabric: not(&why),
            part,
            part_source,
        };
    }
    let firmware = match &b.firmware {
        None => not("the bundle has no firmware"),
        Some(f) => {
            let current = info.app.as_ref().map(|a| a.version.clone());
            let up_to_date = info.app.as_ref().is_some_and(|a| {
                a.committed && hex32(&f.image_crc32).is_ok_and(|c| c == a.image_crc)
            });
            Offer {
                available: true,
                current,
                new: Some(f.version.clone()),
                up_to_date,
                reason: None,
            }
        }
    };
    let fabric = match part.as_deref() {
        None => not("the board reports no part: choose the part to update its fabric"),
        Some(p) => match b.fabrics.iter().find(|f| f.part == p) {
            None => not(&format!("the bundle has no fabric for part {p}")),
            Some(f) => {
                let crc = hex32(&f.crc32).unwrap_or_default();
                Offer {
                    available: true,
                    current: info
                        .fabric
                        .as_ref()
                        .map(|m| format!("{} {:#010x}", m.part, m.crc)),
                    new: Some(format!("{p} {crc:#010x}")),
                    up_to_date: info
                        .fabric
                        .as_ref()
                        .is_some_and(|m| m.committed && m.part == p && m.crc == crc),
                    reason: None,
                }
            }
        },
    };
    UpdatePlan {
        refuse: None,
        firmware,
        fabric,
        part,
        part_source,
    }
}
