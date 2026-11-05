//! Traces of the load sequence for sim/tb_fw_load.v, which replays them on
//! hdl/fastprom_ahb_slave.v itself. The testbench checks every read value
//! the HDL returns against the one the model returned here, and the block
//! RAM's contents against the image, back to back and with idle cycles.
//! That ties the model the other tests use to the real register block.
//!
//! Format, one transfer per line, 40-bit hex for $readmemh:
//! kind (1 write, 2 read), register (offset / 4), 32-bit data or expected
//! read value; `f000000000` ends the trace.
//!
//! Written to build/fw/trace/ (FASTPROM_TRACE_DIR overrides) as
//! `<name>_aw<AW>_dw<DW>.trace` plus `.hex` (the image, one word a line).

mod common;

use std::fmt::Write as _;
use std::path::PathBuf;

use common::{build_table, pattern, MockFabric, Op};
use fastprom_core::{boot, Fabric, ReadReg, Released, SwitchSample, WriteReg};

fn trace_dir() -> PathBuf {
    let dir = std::env::var_os("FASTPROM_TRACE_DIR").map_or_else(
        || PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../build/fw/trace"),
        PathBuf::from,
    );
    std::fs::create_dir_all(&dir).expect("trace dir");
    dir
}

fn encode(ops: &[Op]) -> String {
    let mut s = String::new();
    for op in ops {
        let (kind, off, v) = match *op {
            Op::Write(r, v) => (1, r.offset(), v),
            Op::Read(r, v) => (2, r.offset(), v),
        };
        writeln!(s, "{kind:x}{:x}{v:08x}", off / 4).expect("write");
    }
    s.push_str("f000000000\n");
    s
}

fn write(name: &str, aw: u32, dw: u32, ops: &[Op], image: &[u8]) {
    let base = trace_dir().join(format!("{name}_aw{aw}_dw{dw}"));
    std::fs::write(base.with_extension("trace"), encode(ops)).expect("trace");
    // One hex digit per 4 bits, as $readmemh expects for DW-bit words.
    let digits = dw.div_ceil(4) as usize;
    let hex: String = image.iter().map(|w| format!("{w:0digits$x}\n")).collect();
    std::fs::write(base.with_extension("hex"), hex).expect("hex");
}

fn emit(name: &str, aw: u32, dw: u32, bank: u8) {
    let depth = 1u32 << aw;
    let mask = ((1u32 << dw) - 1) as u8;
    let images: Vec<Vec<u8>> = (0..4)
        .map(|i| pattern(depth as usize, aw * 10 + i, mask))
        .collect();
    let refs: Vec<(&str, &[u8])> = images.iter().map(|d| ("trace.bin", d.as_slice())).collect();
    let table = build_table("TRACE", depth, dw, &refs);

    let mut fab = MockFabric::new(aw, dw).pipelined();
    fab.ready = true; // as after a warm reset: the trace must clear it first
    let samples = [SwitchSample {
        rx: bank & 1 != 0,
        tx: bank & 2 != 0,
    }; 16];
    boot(Released::claim(&mut fab).expect("claim"), &table, &samples).expect("boot");
    let image = fab.contents();
    assert_eq!(image, images[bank as usize]);
    write(name, aw, dw, &fab.ops, &image);
}

#[test]
fn emit_load_traces() {
    emit("load", 9, 8, 0); // 512x8
    emit("load", 8, 8, 3); // 256x8, bank 3
    emit("load", 12, 4, 1); // 4096x4
    emit("load", 4, 8, 2); // the smallest depth
}

#[test]
fn emit_hazard_trace() {
    // The hazard itself, for the testbench to confirm on the HDL: a read
    // straight after a write returns the old value on a pipelined bus.
    let mut fab = MockFabric::new(9, 8).pipelined();
    fab.ready = true; // the testbench presets READY, as after a warm reset
    fab.write(WriteReg::Addr, 0x55);
    assert_eq!(fab.read(ReadReg::Addr), 0, "stale");
    assert_eq!(fab.read(ReadReg::Addr), 0x55, "fresh");
    fab.write(WriteReg::Ctrl, 0);
    assert_eq!(fab.read(ReadReg::Ctrl), 1, "stale: READY looks still set");
    assert_eq!(fab.read(ReadReg::Id), 0x4650_524D);
    assert_eq!(fab.read(ReadReg::Ctrl), 0, "fresh");
    fab.settle();
    write("hazard", 9, 8, &fab.ops, &[]);
}
