//! UART recovery: what a blank or bricked board is provisioned with over
//! its mask ROM's serial loader (`docs/10-updates.md`, "Recovery").
//!
//! The AG32's mask ROM, entered with BOOT0 high, speaks ST's AN3155 USART
//! boot loader protocol on UART0 (J3), 8E1: AGM's flasher is a fork of
//! `stm32flash`, and AGM's own boot loader source uses AN3155's command
//! set. Recovery asks of the mask ROM only what AGM's own tools ask of it
//! before they hand over to a loader of their own in SRAM: sync, `GET`,
//! `GID`, `READ MEMORY` and `WRITE MEMORY` to SRAM, `GO`, and `READOUT
//! UNPROTECT` for a protected chip (`DESIGN_NOTES.md`, "UART recovery",
//! has the evidence for each). It uploads FastPROM's own loader
//! (`fastprom-recovery`), which speaks the same framing, and that does the
//! flash: option bytes, sector erases, page writes, read-back.
//!
//! This module is that shared ground: the protocol's numbers and framing
//! ([`xor`]), the option bytes recovery writes ([`OPTION_BYTES`]), the
//! loader's header ([`LoaderHeader`]), and the loader's command server
//! ([`server`]), which runs on the board and in the host's simulator.

pub mod server;

use thiserror::Error;

use crate::crc32::crc32;
use crate::header::{BoardRev, Version, BOARD_REV_LEN};
use crate::memmap::{
    FABRIC_BASE, OPTION_LEN, RECOVERY_BASE, RECOVERY_ENTRY, RECOVERY_HEADER_LEN, RECOVERY_LEN,
};

/// Sent by the host first: the loader measures the baud rate on it.
pub const SYNC: u8 = 0x7F;
/// Accepted.
pub const ACK: u8 = 0x79;
/// Refused.
pub const NACK: u8 = 0x1F;

/// AN3155's commands, and AGM's extensions to it, as AGM's boot loader
/// source numbers them (`examples/dfu/src/boot.h`).
pub const CMD_GET: u8 = 0x00;
pub const CMD_GID: u8 = 0x02;
pub const CMD_READ: u8 = 0x11;
pub const CMD_GO: u8 = 0x21;
pub const CMD_WRITE: u8 = 0x31;
pub const CMD_ERASE: u8 = 0x44;
pub const CMD_READOUT_UNPROTECT: u8 = 0x92;
/// AGM: reset the chip (`BOOT_CMD_RESET`).
pub const CMD_RESET: u8 = 0xA2;
/// AGM: erase the option bytes (`BOOT_CMD_OO`).
pub const CMD_ERASE_OPTIONS: u8 = 0xA3;

/// The most bytes one read or write carries.
pub const MAX_FRAME: usize = 256;

/// The baud rate recovery runs at, 8E1, both to the mask ROM (which
/// measures it on the sync byte) and to the recovery loader (which sets
/// UART0 to it from its own clock, so the host never changes its port).
/// AN3155's own ceiling, and below the 460800 AgRV_pio passes AGM's
/// flasher for the mask ROM (`-B 460800`, docs/06-build-and-flash.md
/// "Flashing") and the 200,000 AGM's `uart_master` caps it at.
pub const BAUD: u32 = 115_200;

/// What `GID` answers on the AG32VF303 (SYS `DEVICE_ID`): AgRV_pio's
/// `boards/agrv2k_303.json` `device_id`, the ID `gen_batch -d` and the
/// flasher's device table carry.
pub const DEVICE_ID: u32 = 0x4020_0001;

/// The highest boot loader version that is the mask ROM. AGM's flasher
/// uploads its RAM booter when `GET`'s version is at most its `-X`, which
/// AgRV_pio passes as 0x10 (`builder/main.py` `boot_version`), and sends
/// the memory-mode byte only to a loader of at most 0x10
/// (`agrv32_need_mmode`); its booter answers 0x20.
pub const MASK_ROM_MAX_VERSION: u8 = 0x10;
/// What FastPROM's recovery loader answers to `GET`.
pub const LOADER_VERSION: u8 = 0x46;

/// The mask ROM's memory-mode byte, sent (with its complement, and
/// acknowledged) after the address of a read or write: 0 for SRAM, the
/// only memory recovery asks the mask ROM to touch. The flasher's
/// `agrv32_get_mmode` gives 0 for SRAM, 1 flash, 2 option bytes, 3 system
/// memory; AGM's `uart_master` sends 0 before every mask-ROM read and write.
pub const MMODE_SRAM: u8 = 0;

/// AN3155's checksum: the XOR of the bytes.
pub fn xor(bytes: &[u8]) -> u8 {
    bytes.iter().fold(0, |a, &b| a ^ b)
}

/// The address phase of a read, write or go: big-endian, then its XOR.
pub fn address_frame(addr: u32) -> [u8; 5] {
    let [a, b, c, d] = addr.to_be_bytes();
    [a, b, c, d, a ^ b ^ c ^ d]
}

// ---- option bytes ---------------------------------------------------------------

/// Offset of the fabric configuration address in the option bytes, and its
/// complement after it (flash.h `FLASH_OptionByteTypeDef` `FPGA_CONFIG`,
/// `FPGA_CONFIGn`; gen_batch's `--logic-address`).
pub const OFF_FABRIC_ADDRESS: usize = 0x30;

/// The option bytes recovery writes: read protection off (`RDP` 0x5AA5,
/// flash.h `RDP_WORD`), the fabric configured from [`FABRIC_BASE`],
/// uncompressed, everything else erased. Byte for byte what `make fw-batch`
/// has gen_batch write (`gen_option_bytes` with `--logic-address
/// 0x80014000` and nothing else), which `tools/mkbundle.py` writes into the
/// bundle and the page checks against this.
pub const OPTION_BYTES: [u8; OPTION_LEN as usize] = {
    let mut o = [0xFF; OPTION_LEN as usize];
    o[0] = 0xA5;
    o[1] = 0x5A;
    let a = FABRIC_BASE.to_le_bytes();
    let n = (!FABRIC_BASE).to_le_bytes();
    let mut i = 0;
    while i < 4 {
        o[OFF_FABRIC_ADDRESS + i] = a[i];
        o[OFF_FABRIC_ADDRESS + 4 + i] = n[i];
        i += 1;
    }
    o
};

/// The fabric configuration address the option bytes hold, if its
/// complement checks (flash.c `FLASH_GetFPGAConfigAddress`).
pub fn fabric_address(options: &[u8]) -> Option<u32> {
    let word = |at: usize| {
        options
            .get(at..at + 4)
            .and_then(|b| b.try_into().ok())
            .map(u32::from_le_bytes)
    };
    let (a, n) = (word(OFF_FABRIC_ADDRESS)?, word(OFF_FABRIC_ADDRESS + 4)?);
    (a == !n).then_some(a)
}

// ---- the loader's header --------------------------------------------------------

/// `'FPLD'`: FastPROM's recovery loader.
pub const LOADER_MAGIC: u32 = 0x4650_4C44;
/// The header format.
pub const LOADER_HEADER_VERSION: u32 = 1;
/// The longest loader image after its header.
pub const LOADER_IMAGE_MAX: u32 = RECOVERY_LEN - RECOVERY_HEADER_LEN;
const HEADER: usize = RECOVERY_HEADER_LEN as usize;

/// The recovery loader's header, at [`RECOVERY_BASE`], 32 bytes,
/// little-endian:
///
/// | offset | field |
/// |---|---|
/// | `0x00` | [`LOADER_MAGIC`] |
/// | `0x04` | header version, 1 |
/// | `0x08` | image length: bytes after the header |
/// | `0x0c` | CRC-32 of those bytes |
/// | `0x10` | firmware version (`Version::to_bytes`) |
/// | `0x18` | board revision, 8 bytes ASCII, NUL padded |
///
/// Its first instruction is straight after, at [`RECOVERY_ENTRY`], where
/// GO starts it. The loader links the header with a zero length and CRC;
/// `tools/fw_stamp.py loader` fills them in after linking.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LoaderHeader {
    pub image_len: u32,
    pub image_crc: u32,
    pub fw_version: Version,
    pub board_rev: BoardRev,
}

/// A loader file that is not one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum LoaderError {
    #[error("{len} bytes: not a loader of 1 to {max} bytes after its {HEADER}-byte header")]
    Length { len: usize, max: u32 },
    #[error("no loader magic (found {found:#010x})")]
    Magic { found: u32 },
    #[error("loader header version {found}; this page reads {LOADER_HEADER_VERSION}")]
    Version { found: u32 },
    #[error("the header says {stated} bytes, the file has {actual} after it")]
    Stated { stated: u32, actual: usize },
    #[error("its CRC is {computed:#010x}, the header says {stated:#010x}")]
    Crc { computed: u32, stated: u32 },
    #[error("its header's version or board revision is malformed")]
    Fields,
}

impl LoaderHeader {
    /// The header as the loader links it: its version and board revision,
    /// and a zero length and CRC for `tools/fw_stamp.py loader` to fill in.
    pub const fn unstamped(fw_version: Version, board_rev: BoardRev) -> [u8; HEADER] {
        let mut h = [0u8; HEADER];
        let magic = LOADER_MAGIC.to_le_bytes();
        let version = LOADER_HEADER_VERSION.to_le_bytes();
        let v = fw_version.to_bytes();
        let r = board_rev.to_bytes();
        let mut i = 0;
        while i < 4 {
            h[i] = magic[i];
            h[4 + i] = version[i];
            i += 1;
        }
        let mut i = 0;
        while i < 8 {
            h[16 + i] = v[i];
            i += 1;
        }
        let mut i = 0;
        while i < BOARD_REV_LEN {
            h[24 + i] = r[i];
            i += 1;
        }
        h
    }

    /// The header bytes.
    pub fn encode(&self) -> [u8; HEADER] {
        let mut h = [0u8; HEADER];
        h[0..4].copy_from_slice(&LOADER_MAGIC.to_le_bytes());
        h[4..8].copy_from_slice(&LOADER_HEADER_VERSION.to_le_bytes());
        h[8..12].copy_from_slice(&self.image_len.to_le_bytes());
        h[12..16].copy_from_slice(&self.image_crc.to_le_bytes());
        h[16..24].copy_from_slice(&self.fw_version.to_bytes());
        h[24..24 + BOARD_REV_LEN].copy_from_slice(&self.board_rev.to_bytes());
        h
    }

    /// Check a whole loader file (header and image) and return its header.
    ///
    /// # Errors
    ///
    /// The first thing about it that does not hold.
    pub fn check(file: &[u8]) -> Result<Self, LoaderError> {
        let too = LoaderError::Length {
            len: file.len(),
            max: LOADER_IMAGE_MAX,
        };
        let (head, image) = file.split_first_chunk::<HEADER>().ok_or(too)?;
        if image.is_empty() || image.len() > LOADER_IMAGE_MAX as usize {
            return Err(too);
        }
        let word = |at: usize| {
            head.get(at..at + 4)
                .and_then(|b| b.try_into().ok())
                .map_or(0, u32::from_le_bytes)
        };
        if word(0) != LOADER_MAGIC {
            return Err(LoaderError::Magic { found: word(0) });
        }
        if word(4) != LOADER_HEADER_VERSION {
            return Err(LoaderError::Version { found: word(4) });
        }
        let stated = word(8);
        if stated as usize != image.len() {
            return Err(LoaderError::Stated {
                stated,
                actual: image.len(),
            });
        }
        let computed = crc32(image);
        if computed != word(12) {
            return Err(LoaderError::Crc {
                computed,
                stated: word(12),
            });
        }
        let mut v = [0u8; 8];
        v.copy_from_slice(head.get(16..24).ok_or(LoaderError::Fields)?);
        let mut r = [0u8; BOARD_REV_LEN];
        r.copy_from_slice(
            head.get(24..24 + BOARD_REV_LEN)
                .ok_or(LoaderError::Fields)?,
        );
        Ok(Self {
            image_len: stated,
            image_crc: computed,
            fw_version: Version::from_bytes(v).ok_or(LoaderError::Fields)?,
            board_rev: BoardRev::from_bytes(r).ok_or(LoaderError::Fields)?,
        })
    }
}

const _: () = {
    assert!(RECOVERY_ENTRY == RECOVERY_BASE + HEADER as u32);
    assert!(24 + BOARD_REV_LEN == HEADER);
};
