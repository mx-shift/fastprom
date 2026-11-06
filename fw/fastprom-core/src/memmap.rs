// FastPROM's flash and RAM map on the AG32VF303KCU6: 256 KiB of flash,
// 128 KiB of SRAM. The one source of the map: both board binaries' build.rs
// include this file to write their linker's memory.x (through
// fw/fastprom-hal/linker.rs), the firmware reads it to find its regions, and
// the update policy (fastprom-core's `update`) checks every USB write
// against it, so the linker, the code and the policy cannot disagree.
// Plain consts only: build.rs is std, and include!s it.
//
//   0x8000_0000  DFU stage, 32 KiB         written over UART only
//   0x8000_8000  application, 48 KiB       a 256-byte header, then the code
//   0x8001_4000  fabric bitstream, 100 KiB (99,944 bytes used)
//   0x8002_D000  fabric manifest, 4 KiB    one sector, written last
//   0x8002_E000  image table, 72 KiB       to the end of flash
//   0x8100_0000  option bytes              written over UART only
//   0x2000_0000  SRAM, 128 KiB             (.data, .bss, stack at the top)
//   0x2001_0000  recovery loader, 5,296 B  uploaded over the UART, never in flash
//
// docs/10-updates.md ("Flash map") is the design; DESIGN_NOTES.md has the
// reasons for each size.
//
// Sources, in AGM's AgRV_pio 1.1.0 (/opt/agm/AgRV_pio):
// - FLASH_BASE, SRAM_BASE: packages/framework-agrv_sdk/misc/devices/AgRV2K_mem.ld
// - FLASH_SIZE, SRAM_SIZE: platforms/AgRV/boards/agrv2k_303.json
//   (upload.maximum_size 262144, upload.maximum_ram_size 131072)
// - SECTOR, PAGE: packages/framework-agrv_sdk/src/flash.h FLASH_SECTOR_SIZE,
//   FLASH_PAGE_SIZE
// - OPTION_BASE, OPTION_LEN: flash.h FLASH_OPTION_BASE, and the 128-byte
//   option-byte section gen_batch writes (FLASH_OptionByteTypeDef is 0x80
//   bytes); the fabric's address is the word at +0x30 (FPGA_CONFIG)
// - FABRIC_BASE: FastPROM's choice, handed to gen_batch as --logic-address
//   (Makefile LOGIC_ADDR), which points the option byte there. Supra's own
//   af_run.tcl uses 0x8000_7000, inside the DFU stage's region.
// - FABRIC_LEN: platforms/AgRV/builder/common.py get_upload_info,
//   full_logic_reserve = 100 KiB; an uncompressed AGRV2K image is 99,944
//   bytes (fcb.h FCB_AUTO_WORDS)
// - DFU_LEN, APP_LEN, MANIFEST_*, TABLE_BASE: FastPROM's choices, on 4 KiB
//   sector boundaries; see DESIGN_NOTES.md.

/// Start of on-chip flash; the mask ROM jumps here when BOOT0 is low.
pub const FLASH_BASE: u32 = 0x8000_0000;
/// On-chip flash on the KCU6 part.
pub const FLASH_SIZE: u32 = 256 * 1024;
/// Flash erase granule.
pub const SECTOR: u32 = 4 * 1024;
/// Flash program granule.
pub const PAGE: u32 = 256;

/// The DFU stage: the reset vector. Never written over USB.
pub const DFU_BASE: u32 = FLASH_BASE;
pub const DFU_LEN: u32 = 32 * 1024;
/// The application: its header, then its code.
pub const APP_BASE: u32 = 0x8000_8000;
pub const APP_LEN: u32 = 48 * 1024;
/// The application header (fastprom-core `header`), one page.
pub const APP_HEADER_LEN: u32 = PAGE;
/// Where the DFU stage jumps: the application's first instruction.
pub const APP_ENTRY: u32 = APP_BASE + APP_HEADER_LEN;
/// The fabric bitstream, read by the configuration block at power-on;
/// never linked into.
pub const FABRIC_BASE: u32 = 0x8001_4000;
pub const FABRIC_LEN: u32 = 100 * 1024;
/// The fabric manifest (fastprom-core `header`): one sector of its own, so
/// erasing it never touches the bitstream or the table.
pub const MANIFEST_BASE: u32 = 0x8002_D000;
pub const MANIFEST_LEN: u32 = SECTOR;
/// The image table (`tools/mkimagetable.py --out <file>.bin`).
pub const TABLE_BASE: u32 = 0x8002_E000;
pub const TABLE_LEN: u32 = FLASH_BASE + FLASH_SIZE - TABLE_BASE;
/// The option bytes, outside the main array.
pub const OPTION_BASE: u32 = 0x8100_0000;
pub const OPTION_LEN: u32 = 128;
/// SRAM.
pub const SRAM_BASE: u32 = 0x2000_0000;
pub const SRAM_SIZE: u32 = 128 * 1024;

/// The recovery loader (`fastprom-recovery`, docs/10-updates.md
/// "Recovery"): uploaded into SRAM through the mask ROM's UART loader and
/// started with its GO, never in flash. The address is the one AGM's
/// flasher uploads its own RAM booter to (`agrv32flash -T 0x20010000`,
/// AgRV_pio builder/common.py `upload.boot_address`).
pub const RECOVERY_BASE: u32 = 0x2001_0000;
/// The most the upload may be, header included: the size of the RAM booter
/// AGM's tools upload through the mask ROM (`dfu_uart.bin`, 5,296 bytes, at
/// 0x2001_0000 by the flasher and at 0x2000_8000 by `examples/uart_master`).
/// What the mask ROM itself keeps in SRAM while it receives is not
/// published; this much there is shown not to collide with it.
pub const RECOVERY_LEN: u32 = 5296;
/// Its header (fastprom-core `recovery::LoaderHeader`).
pub const RECOVERY_HEADER_LEN: u32 = 32;
/// Where GO starts it: the first instruction after the header.
pub const RECOVERY_ENTRY: u32 = RECOVERY_BASE + RECOVERY_HEADER_LEN;
/// Its `.bss` and stack: the bottom of SRAM, which nothing uploaded uses,
/// and which the mask ROM has finished with once it has jumped.
pub const RECOVERY_RAM_BASE: u32 = SRAM_BASE;
pub const RECOVERY_RAM_LEN: u32 = 32 * 1024;

// The map must tile flash in address order without gaps or overlaps, on
// sector boundaries, so that erasing one region never erases another.
// Checked whenever build.rs or the firmware compiles.
const _: () = {
    assert!(DFU_BASE == FLASH_BASE, "the mask ROM boots from FLASH_BASE");
    assert!(DFU_BASE + DFU_LEN == APP_BASE, "DFU and application");
    assert!(APP_BASE + APP_LEN == FABRIC_BASE, "application and fabric");
    assert!(
        FABRIC_BASE + FABRIC_LEN == MANIFEST_BASE,
        "fabric and manifest"
    );
    assert!(
        MANIFEST_BASE + MANIFEST_LEN == TABLE_BASE,
        "manifest and table"
    );
    assert!(
        TABLE_BASE + TABLE_LEN == FLASH_BASE + FLASH_SIZE,
        "table end"
    );
    assert!(DFU_LEN.is_multiple_of(SECTOR), "DFU not whole sectors");
    assert!(
        APP_BASE.is_multiple_of(SECTOR),
        "application not sector aligned"
    );
    assert!(
        APP_LEN.is_multiple_of(SECTOR),
        "application not whole sectors"
    );
    assert!(
        FABRIC_BASE.is_multiple_of(SECTOR),
        "fabric not sector aligned"
    );
    assert!(
        FABRIC_LEN.is_multiple_of(SECTOR),
        "fabric not whole sectors"
    );
    assert!(
        MANIFEST_BASE.is_multiple_of(SECTOR),
        "manifest not sector aligned"
    );
    assert!(
        TABLE_BASE.is_multiple_of(SECTOR),
        "table not sector aligned"
    );
    assert!(SECTOR.is_multiple_of(PAGE), "a sector is whole pages");
    assert!(APP_HEADER_LEN == PAGE, "the header is exactly one page");
    // The largest image table: four 4096-word images behind its header.
    assert!(TABLE_LEN >= 256 + 4 * 4096, "table region too small");
    // The recovery loader's upload and its RAM are SRAM, apart.
    assert!(RECOVERY_BASE >= SRAM_BASE, "recovery loader in SRAM");
    assert!(
        RECOVERY_BASE + RECOVERY_LEN <= SRAM_BASE + SRAM_SIZE,
        "recovery loader in SRAM"
    );
    assert!(
        RECOVERY_RAM_BASE + RECOVERY_RAM_LEN <= RECOVERY_BASE,
        "the loader's RAM below its upload"
    );
    assert!(RECOVERY_HEADER_LEN < RECOVERY_LEN, "room for the loader");
};
