//! `COMMIT`: check a payload against the header or manifest just written,
//! and only then make the page valid by programming its commit word.

use crate::crc32::Crc32;
use crate::header::{AppHeader, BoardRev, Commit, Manifest, APP_MAGIC, MANIFEST_MAGIC, OFF_COMMIT};
use crate::memmap::{APP_BASE, APP_ENTRY, FABRIC_BASE, MANIFEST_BASE};

use super::{Flash, Status, PAGE_LEN};

/// Bytes of payload read at a time for a CRC.
const CRC_CHUNK: usize = 64;

/// What `COMMIT` commits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommitTarget {
    /// The application, against its header.
    Application,
    /// The fabric bitstream, against its manifest.
    Fabric,
}

impl CommitTarget {
    /// From `COMMIT`'s argument ([`super::proto::TARGET_APPLICATION`],
    /// [`super::proto::TARGET_FABRIC`]).
    pub const fn from_arg(v: u8) -> Option<Self> {
        match v {
            super::proto::TARGET_APPLICATION => Some(Self::Application),
            super::proto::TARGET_FABRIC => Some(Self::Fabric),
            _ => None,
        }
    }
}

/// Commit `target` on `board`, a DFU stage built for `board_rev`.
pub fn commit<B: Flash>(
    board: &B,
    board_rev: BoardRev,
    target: CommitTarget,
) -> Result<(), Status> {
    let mut page = [0u8; PAGE_LEN];
    let (at, magic) = match target {
        CommitTarget::Application => (APP_BASE, APP_MAGIC),
        CommitTarget::Fabric => (MANIFEST_BASE, MANIFEST_MAGIC),
    };
    board.read(at, &mut page);
    let (commit, rev, data, len, crc) = match target {
        CommitTarget::Application => {
            let (c, h) = AppHeader::decode(&page).map_err(|_| Status::InvalidData)?;
            (c, h.board_rev, APP_ENTRY, h.image_len, h.image_crc)
        }
        CommitTarget::Fabric => {
            let (c, m) = Manifest::decode(&page).map_err(|_| Status::InvalidData)?;
            (c, m.board_rev, FABRIC_BASE, m.fabric_len, m.fabric_crc)
        }
    };
    if rev != board_rev {
        return Err(Status::NotPermitted);
    }
    if crc_of(board, data, len) != crc {
        return Err(Status::CrcMismatch);
    }
    if commit == Commit::Committed {
        return Ok(());
    }
    // Last of all: the commit word, alone in an otherwise all-ones page,
    // which programs nothing else.
    let mut word = [0xFFu8; PAGE_LEN];
    if let Some(w) = word.get_mut(OFF_COMMIT..OFF_COMMIT + 4) {
        w.copy_from_slice(&magic.to_le_bytes());
    }
    board
        .program_page(at, &word)
        .map_err(|_| Status::FlashFailed)?;
    let mut back = [0u8; 4];
    board.read(at, &mut back);
    if u32::from_le_bytes(back) == magic {
        Ok(())
    } else {
        Err(Status::FlashFailed)
    }
}

/// The CRC-32 of `len` bytes of flash at `addr` (inside its region: the
/// decoders bound `len` by it).
pub fn crc_of<B: Flash>(board: &B, addr: u32, len: u32) -> u32 {
    let mut crc = Crc32::new();
    feed(board, &mut crc, addr, len);
    crc.finish()
}

/// Run `len` bytes of flash at `addr` through `crc`.
pub fn feed<B: Flash>(board: &B, crc: &mut Crc32, addr: u32, len: u32) {
    let mut buf = [0u8; CRC_CHUNK];
    let mut done = 0u32;
    while done < len {
        let n = (len - done).min(CRC_CHUNK as u32);
        let chunk = buf.get_mut(..n as usize).unwrap_or(&mut []);
        board.read(addr + done, chunk);
        crc.update(chunk);
        done += n;
    }
}
