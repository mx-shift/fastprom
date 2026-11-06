//! `INFO`'s answer: what the page shows on connecting (format version 2).
//!
//! [`INFO_LEN`] bytes, little-endian; a field that does not apply is zero.
//!
//! | offset | field |
//! |---|---|
//! | `0x00` | `u8` format version, 1 |
//! | `0x01` | `u8` mode: 0 application, 1 DFU stage |
//! | `0x02` | `u8` the switch's bank at boot, 0 to 3; `0xff` if not read |
//! | `0x03` | `u8` flags: bit 0 application header committed, bit 1 fabric manifest committed, bit 2 image table header valid, bit 3 image table valid and every image intact |
//! | `0x04` | running firmware's version: major, minor, patch (`u16`), 0 |
//! | `0x0c` | running firmware's board revision, 8 bytes |
//! | `0x14` | `u32` the fabric ID the running firmware expects |
//! | `0x18` | application header: version (8 bytes), board revision (8), fabric ID, image length, image CRC (`u32` each) |
//! | `0x34` | manifest: part (16 bytes), board revision (8), fabric ID, length, CRC (`u32` each) |
//! | `0x58` | image table: depth (`u32`), width (`u8`), count (`u8`), 0 (`u16`), four image CRCs (`u32`), part (28 bytes) |
//! | `0x8c` | `u8` images intact: bit `n` set when image `n`'s words match its CRC (the check the load at boot makes) |
//! | `0x8d` | reserved, zero |
//!
//! The header and manifest fields are shown whenever the page is well
//! formed, committed or not; the flags say which are committed.
//!
//! Bit 2 says only that the table's header parses. A power cut while a
//! table is written can leave a whole header over images half written, so
//! bit 3 and the byte at `0x8c` say what the firmware's own check at boot
//! would make of the images (`TableSummary::intact`). Format 1 had neither.

use crate::header::{AppHeader, Commit, Manifest, PAGE_LEN};
use crate::memmap::{APP_BASE, MANIFEST_BASE};
use crate::regmap::FABRIC_ID;

use super::{Flash, Identity, Mode, Platform};

/// Bytes in an `INFO` answer; a host asks for exactly this many.
pub const INFO_LEN: usize = 0x90;
/// The format above.
pub const INFO_VERSION: u8 = 2;

pub const FLAG_APP_COMMITTED: u8 = 1 << 0;
pub const FLAG_MANIFEST_COMMITTED: u8 = 1 << 1;
pub const FLAG_TABLE_VALID: u8 = 1 << 2;
pub const FLAG_TABLE_INTACT: u8 = 1 << 3;

pub const OFF_MODE: usize = 0x01;
pub const OFF_BANK: usize = 0x02;
pub const OFF_FLAGS: usize = 0x03;
pub const OFF_FW_VERSION: usize = 0x04;
pub const OFF_BOARD_REV: usize = 0x0C;
pub const OFF_FABRIC_ID: usize = 0x14;
pub const OFF_APP: usize = 0x18;
pub const OFF_MANIFEST: usize = 0x34;
pub const OFF_TABLE: usize = 0x58;
pub const OFF_INTACT: usize = 0x8C;
pub const OFF_RESERVED: usize = 0x8D;

const _: () = {
    assert!(OFF_APP + 8 + 8 + 3 * 4 == OFF_MANIFEST);
    assert!(OFF_MANIFEST + 16 + 8 + 3 * 4 == OFF_TABLE);
    assert!(OFF_TABLE + 8 + 16 + 28 == OFF_INTACT);
    assert!(OFF_INTACT + 1 == OFF_RESERVED);
    assert!(OFF_RESERVED + 3 == INFO_LEN);
};

/// Build the answer from what is in flash now.
pub fn build<B: Flash + Platform>(board: &B, id: &Identity) -> [u8; INFO_LEN] {
    let mut out = Writer::new();
    let mut flags = 0;

    out.put(&[INFO_VERSION]);
    out.put(&[match id.mode {
        Mode::Application => 0,
        Mode::Dfu => 1,
    }]);
    out.put(&[board.switch_bank().map_or(0xFF, |b| b.number())]);
    out.put(&[0]); // flags, filled in below
    out.put(&id.fw_version.to_bytes());
    out.put(&id.board_rev.to_bytes());
    out.put32(FABRIC_ID);

    let mut page = [0u8; PAGE_LEN];
    board.read(APP_BASE, &mut page);
    match AppHeader::decode(&page) {
        Ok((commit, h)) => {
            if commit == Commit::Committed {
                flags |= FLAG_APP_COMMITTED;
            }
            out.put(&h.fw_version.to_bytes());
            out.put(&h.board_rev.to_bytes());
            out.put32(h.fabric_id);
            out.put32(h.image_len);
            out.put32(h.image_crc);
        }
        Err(_) => out.skip(8 + 8 + 3 * 4),
    }

    board.read(MANIFEST_BASE, &mut page);
    match Manifest::decode(&page) {
        Ok((commit, m)) => {
            if commit == Commit::Committed {
                flags |= FLAG_MANIFEST_COMMITTED;
            }
            out.put(&m.part.to_bytes());
            out.put(&m.board_rev.to_bytes());
            out.put32(m.fabric_id);
            out.put32(m.fabric_len);
            out.put32(m.fabric_crc);
        }
        Err(_) => out.skip(16 + 8 + 3 * 4),
    }

    let mut intact = 0;
    match board.table_summary() {
        Some(t) => {
            flags |= FLAG_TABLE_VALID;
            if t.all_intact() {
                flags |= FLAG_TABLE_INTACT;
            }
            intact = t.intact;
            out.put32(t.depth);
            out.put(&[t.width.bits(), t.count.get() as u8, 0, 0]);
            for crc in t.crcs {
                out.put32(crc);
            }
            out.put(&t.part);
        }
        None => out.skip(8 + 16 + 28),
    }
    out.put(&[intact]);

    let mut info = out.0;
    if let Some(f) = info.get_mut(OFF_FLAGS) {
        *f = flags;
    }
    info
}

/// Appends fields in order; anything past the end is dropped (the offsets
/// above are checked at compile time to fit).
struct Writer([u8; INFO_LEN], usize);

impl Writer {
    const fn new() -> Self {
        Self([0; INFO_LEN], 0)
    }

    fn put(&mut self, bytes: &[u8]) {
        let at = self.1;
        if let Some(dst) = self.0.get_mut(at..at + bytes.len()) {
            dst.copy_from_slice(bytes);
        }
        self.1 = at + bytes.len();
    }

    fn put32(&mut self, v: u32) {
        self.put(&v.to_le_bytes());
    }

    fn skip(&mut self, n: usize) {
        self.1 += n;
    }
}
