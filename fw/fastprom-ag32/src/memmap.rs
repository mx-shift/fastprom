// FastPROM's flash and RAM map on the AG32VF303KCU6: 256 KiB of flash,
// 128 KiB of SRAM. build.rs includes this file to write the linker's
// memory.x, and the firmware includes it to find the image table, so the
// linker and the code cannot disagree. Plain consts only: build.rs is std.
//
//   0x8000_0000  firmware, 28 KiB   (.text, .rodata, .data's load image)
//   0x8000_7000  fabric bitstream, 100 KiB reserved (99,944 bytes used)
//   0x8002_0000  image table, to the end of flash (128 KiB)
//   0x2000_0000  SRAM, 128 KiB      (.data, .bss, stack at the top)
//
// Sources, in AGM's AgRV_pio 1.1.0 (/opt/agm/AgRV_pio):
// - FLASH_BASE, SRAM_BASE: packages/framework-agrv_sdk/misc/devices/AgRV2K_mem.ld
// - FLASH_SIZE, SRAM_SIZE: platforms/AgRV/boards/agrv2k_303.json
//   (upload.maximum_size 262144, upload.maximum_ram_size 131072)
// - FABRIC_BASE: the --logic-address that Supra's generated af_run.tcl hands
//   gen_batch (build/ag32/<part>-bram/logic/af_run.tcl), which also points the
//   option byte at 0x8100_0030 (fcb.h FPGA_CONFIG_ADDR) there. Compressed
//   builds put the decompressor there and the image at 0x8000_8100.
// - FABRIC_LEN: platforms/AgRV/builder/common.py get_upload_info,
//   full_logic_reserve = 100 KiB; an uncompressed AGRV2K image is 99,944
//   bytes (fcb.h FCB_AUTO_WORDS)
// - TABLE_BASE: FastPROM's choice, the first 4 KiB sector after the fabric
//   reservation (flash.h FLASH_SECTOR_SIZE); see DESIGN_NOTES.md.

/// Start of on-chip flash; the mask ROM jumps here when BOOT0 is low.
pub const FLASH_BASE: u32 = 0x8000_0000;
/// On-chip flash on the KCU6 part.
pub const FLASH_SIZE: u32 = 256 * 1024;
/// The firmware: everything below the fabric bitstream.
pub const FW_BASE: u32 = FLASH_BASE;
pub const FW_LEN: u32 = FABRIC_BASE - FW_BASE;
/// The fabric bitstream, written by AGM's loader and read by the
/// configuration block at power-on; never linked into.
pub const FABRIC_BASE: u32 = 0x8000_7000;
pub const FABRIC_LEN: u32 = 100 * 1024;
/// The image table (`tools/mkimagetable.py --out <file>.bin`).
pub const TABLE_BASE: u32 = 0x8002_0000;
pub const TABLE_LEN: u32 = FLASH_BASE + FLASH_SIZE - TABLE_BASE;
/// Flash erase granule (flash.h FLASH_SECTOR_SIZE).
pub const SECTOR: u32 = 4 * 1024;
/// SRAM.
pub const SRAM_BASE: u32 = 0x2000_0000;
pub const SRAM_SIZE: u32 = 128 * 1024;

// The map must tile flash without overlaps, on sector boundaries, so that
// programming one region never erases another. Checked when build.rs and
// the firmware compile.
const _: () = {
    assert!(FW_BASE == FLASH_BASE, "the mask ROM boots from FLASH_BASE");
    assert!(
        FW_BASE + FW_LEN <= FABRIC_BASE,
        "firmware overlaps the fabric"
    );
    assert!(
        FABRIC_BASE + FABRIC_LEN <= TABLE_BASE,
        "fabric overlaps the table"
    );
    assert!(
        TABLE_BASE + TABLE_LEN <= FLASH_BASE + FLASH_SIZE,
        "table past flash"
    );
    assert!(
        FABRIC_BASE.is_multiple_of(SECTOR),
        "fabric region not sector aligned"
    );
    assert!(
        TABLE_BASE.is_multiple_of(SECTOR),
        "table region not sector aligned"
    );
};
