//! The power-up path: `docs/10-updates.md`, "Power-up", step 2.
//!
//! This is the first code at the reset vector: the linker puts
//! `.fastprom_reset` at `memmap::DFU_BASE` (its own region, ahead of
//! riscv-rt's `_start`; fw/fastprom-hal/build_support.rs), and makes it the
//! ELF entry. On the normal path it is the whole of the DFU stage: no
//! riscv-rt startup, no stack, no `.data` or `.bss`, no clock change, no
//! flash controller, no USB. In order:
//!
//! 1. If RST_CNTL says power-on reset (system.h SYS_RSTF_POR), clear the
//!    reset flags (SYS_RST_REMOVE) and `mscratch`, which is undefined after
//!    power-on. A soft reset leaves both alone: that is how the application
//!    asks for DFU mode (`fastprom_hal::reset`).
//! 2. Read and clear `mscratch`. `header::DFU_REQUEST` there: on to
//!    riscv-rt's `_start` and `main`, which is DFU mode.
//! 3. Otherwise, if the application header's commit word, version and
//!    length are sane, and so are the fabric manifest's
//!    (`header::boot_application`, which this is in assembly), jump to the
//!    application's entry, which is its riscv-rt `_start`: exactly what the
//!    mask ROM used to jump to, so the application starts as it always
//!    did, only these instructions later. The manifest matters because
//!    the application's USB is the fabric's to enable: a fabric update cut
//!    part way (its manifest erased first) would otherwise boot the
//!    application on a fabric the FCB cannot configure, with no USB.
//! 4. Else DFU mode, through riscv-rt's `_start`, on the stage's own
//!    USB-only fabric.
//!
//! No CRC: each payload was checked against its page when it was
//! committed (`fastprom_core::update`), and a committed page is never left
//! over a payload that changed since. It runs before riscv-rt has set
//! `mtvec`, so it does nothing that could trap: six loads from flash, one
//! load and store of RST_CNTL, two CSR accesses; and it uses only `t0`-`t2`.

use fastprom_core::header::{
    boot_application, APP_MAGIC, APP_VERSION, DFU_REQUEST, FABRIC_IMAGE_MAX, MANIFEST_MAGIC,
    MANIFEST_VERSION, OFF_COMMIT, OFF_LEN, OFF_VERSION,
};
use fastprom_core::memmap::{APP_BASE, APP_ENTRY, MANIFEST_BASE};
use fastprom_hal::regs::sys;

/// The longest image `boot_sane` accepts.
const IMAGE_MAX: u32 = fastprom_core::header::APP_IMAGE_MAX;

core::arch::global_asm!(
    ".section .fastprom_reset, \"ax\"",
    ".global __fastprom_reset",
    ".p2align 2",
    "__fastprom_reset:",
    // 1. Power-on reset: clear the flags and mscratch.
    "    li   t0, {rst_cntl}",
    "    lw   t1, 0(t0)",
    "    li   t2, {rstf_por}",
    "    and  t2, t1, t2",
    "    beqz t2, 1f",
    "    li   t2, {rst_remove}",
    "    or   t1, t1, t2",
    "    sw   t1, 0(t0)",
    "    csrw mscratch, zero",
    // 2. A DFU request, consumed.
    "1:  csrrw t1, mscratch, zero",
    "    li   t2, {dfu_request}",
    "    beq  t1, t2, 2f",
    // 3. The header: commit word, version, 0 < length <= IMAGE_MAX.
    "    li   t0, {app_base}",
    "    lw   t1, {off_commit}(t0)",
    "    li   t2, {app_magic}",
    "    bne  t1, t2, 2f",
    "    lw   t1, {off_version}(t0)",
    "    li   t2, {app_version}",
    "    bne  t1, t2, 2f",
    "    lw   t1, {off_len}(t0)",
    "    beqz t1, 2f",
    "    li   t2, {image_max}",
    "    bgtu t1, t2, 2f",
    // 4. The manifest: commit word, version, 0 < length <= FABRIC_MAX.
    "    li   t0, {man_base}",
    "    lw   t1, {off_commit}(t0)",
    "    li   t2, {man_magic}",
    "    bne  t1, t2, 2f",
    "    lw   t1, {off_version}(t0)",
    "    li   t2, {man_version}",
    "    bne  t1, t2, 2f",
    "    lw   t1, {off_len}(t0)",
    "    beqz t1, 2f",
    "    li   t2, {fabric_max}",
    "    bgtu t1, t2, 2f",
    "    li   t0, {app_entry}",
    "    jr   t0",
    // 5. DFU mode: riscv-rt's startup, which initialises RAM and calls main.
    "2:  j    _start",
    rst_cntl = const sys::RST_CNTL_ADDR,
    rstf_por = const sys::RSTF_POR,
    rst_remove = const sys::RST_REMOVE,
    dfu_request = const DFU_REQUEST,
    app_base = const APP_BASE,
    off_commit = const OFF_COMMIT,
    app_magic = const APP_MAGIC,
    off_version = const OFF_VERSION,
    app_version = const APP_VERSION,
    off_len = const OFF_LEN,
    image_max = const IMAGE_MAX,
    man_base = const MANIFEST_BASE,
    man_magic = const MANIFEST_MAGIC,
    man_version = const MANIFEST_VERSION,
    fabric_max = const FABRIC_IMAGE_MAX,
    app_entry = const APP_ENTRY,
);

// The assembly is boot_application: if that changes shape, this must too.
const _: () = {
    let app = [APP_MAGIC, APP_VERSION, 1];
    let man = [MANIFEST_MAGIC, MANIFEST_VERSION, 1];
    assert!(boot_application(app, man));
    assert!(boot_application(
        app,
        [MANIFEST_MAGIC, MANIFEST_VERSION, FABRIC_IMAGE_MAX]
    ));
    assert!(!boot_application(app, [u32::MAX, MANIFEST_VERSION, 1]));
    assert!(!boot_application(
        app,
        [MANIFEST_MAGIC, MANIFEST_VERSION, 0]
    ));
    assert!(!boot_application(
        app,
        [MANIFEST_MAGIC, MANIFEST_VERSION, FABRIC_IMAGE_MAX + 1]
    ));
    assert!(!boot_application(
        app,
        [MANIFEST_MAGIC, MANIFEST_VERSION + 1, 1]
    ));
    assert!(!boot_application([u32::MAX, APP_VERSION, 1], man));
    assert!(fastprom_core::header::boot_sane(APP_MAGIC, APP_VERSION, 1));
    assert!(fastprom_core::header::boot_sane(
        APP_MAGIC,
        APP_VERSION,
        IMAGE_MAX
    ));
    assert!(!fastprom_core::header::boot_sane(APP_MAGIC, APP_VERSION, 0));
    assert!(!fastprom_core::header::boot_sane(
        APP_MAGIC,
        APP_VERSION,
        IMAGE_MAX + 1
    ));
    // Offsets that fit a load's 12-bit immediate.
    assert!(OFF_COMMIT < 2048 && OFF_VERSION < 2048 && OFF_LEN < 2048);
};
