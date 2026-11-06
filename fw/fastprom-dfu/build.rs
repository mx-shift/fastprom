//! Writes memory.x (the DFU stage's window at the reset vector) and
//! build_info.rs, and links with riscv-rt's link.x: fw/fastprom-hal/build_support.rs.
//!
//! Then checks the USB-only fabric the stage embeds (src/fabric.rs): the
//! committed fabric/dfu_usb.bin must be the one fabric/dfu_usb.manifest
//! describes, and the manifest must have been written from the Supra inputs
//! now in the tree. The bitstream needs AGM's SDK to rebuild
//! (tools/ag32/build_dfu_fabric.sh), which CI does not have, so it is
//! committed; this is what keeps it from going stale unnoticed.

include!("../fastprom-hal/build_support.rs");

/// zlib's CRC-32, bitwise (the manifest's, from tools/ag32/check_dfu_fabric.py).
fn crc32(parts: &[&[u8]]) -> u32 {
    let mut crc = 0xFFFF_FFFFu32;
    for &b in parts.iter().flat_map(|p| p.iter()) {
        crc ^= u32::from(b);
        for _ in 0..8 {
            crc = (crc >> 1) ^ (0xEDB8_8320 & (crc & 1).wrapping_neg());
        }
    }
    !crc
}

fn fail(what: &str) -> ! {
    panic!(
        "fastprom-dfu: {what}.\n  Rebuild the DFU fabric with AGM's SDK and commit the result:\n  \
         tools/ag32/build_dfu_fabric.sh <AgRV_pio dir> --install (docs/06-build-and-flash.md)"
    )
}

fn check_fabric() {
    let manifest_dir = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").expect("cargo sets it"));
    let fabric = manifest_dir.join("fabric");
    let tools = manifest_dir.join("../../tools/ag32");
    let read = |p: PathBuf| -> Vec<u8> {
        println!("cargo:rerun-if-changed={}", p.display());
        fs::read(&p).unwrap_or_else(|e| fail(&format!("cannot read {}: {e}", p.display())))
    };
    let manifest = String::from_utf8(read(fabric.join("dfu_usb.manifest")))
        .unwrap_or_else(|_| fail("fabric/dfu_usb.manifest is not text"));
    let field = |key: &str| -> u32 {
        let v = manifest
            .lines()
            .find_map(|l| l.strip_prefix(key)?.strip_prefix(' '))
            .unwrap_or_else(|| fail(&format!("fabric/dfu_usb.manifest has no {key}")));
        let parsed = match v.trim().strip_prefix("0x") {
            Some(hex) => u32::from_str_radix(hex, 16),
            None => v.trim().parse(),
        };
        parsed.unwrap_or_else(|_| fail(&format!("fabric/dfu_usb.manifest: bad {key}")))
    };

    let bin = read(fabric.join("dfu_usb.bin"));
    if bin.len() as u32 != field("bin_len") || crc32(&[&bin]) != field("bin_crc32") {
        fail("fabric/dfu_usb.bin is not the bitstream fabric/dfu_usb.manifest records");
    }
    // The inputs, in tools/ag32/check_dfu_fabric.py's INPUTS order.
    let inputs = [
        read(fabric.join("dfu_usb.ve")),
        read(tools.join("usb0.asf")),
        read(tools.join("dfu_usb.sdc")),
        read(tools.join("dfu_usb_pre.tcl")),
        read(tools.join("dfu_usb_post.tcl")),
    ];
    let parts: Vec<&[u8]> = inputs.iter().map(Vec::as_slice).collect();
    if crc32(&parts) != field("inputs_crc32") {
        fail("the DFU fabric's Supra inputs (tools/ag32/usb0.asf, dfu_usb.sdc, dfu_usb_pre.tcl, dfu_usb_post.tcl, fabric/dfu_usb.ve) changed since fabric/dfu_usb.bin was built");
    }
}

fn main() {
    build(Binary::Dfu);
    check_fabric();
}
