//! The application header, linked at `memmap::APP_BASE` ahead of the code.
//!
//! The DFU stage boots the application only through it
//! (`fastprom_core::header::boot_sane`). It is linked with every field
//! known at compile time, and with a zero image length and zero CRCs, which
//! the DFU stage refuses: `make fw-build` runs `tools/fw_stamp.py app`
//! after linking to fill them in, and `tools/fw_layout.py` checks that it
//! did.

use fastprom_core::header::{AppHeader, APP_MAGIC, PAGE_LEN};
use fastprom_core::regmap::FABRIC_ID;

use crate::build_info::{BOARD_REV, FW_VERSION};

#[used]
#[unsafe(link_section = ".fastprom_header")]
static HEADER: [u8; PAGE_LEN] = AppHeader {
    image_len: 0,
    image_crc: 0,
    fw_version: FW_VERSION,
    board_rev: BOARD_REV,
    fabric_id: FABRIC_ID,
}
.fields(APP_MAGIC);
