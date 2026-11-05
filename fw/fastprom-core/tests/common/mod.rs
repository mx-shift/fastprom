//! Shared by the integration tests: fixtures, a model of the fabric's AHB
//! slave, and a table builder for malformed-input tests.
//!
//! Each test file uses a different part of this, so the parts it does not
//! use are dead code in that test crate.
#![allow(dead_code, reason = "each test crate uses a different subset")]

use std::path::PathBuf;

use fastprom_core::{Fabric, ReadReg, WriteReg};

// ---- fixtures ---------------------------------------------------------------

/// `build/fw/fixtures`, written by `make fw-fixtures`
/// (`fw/tools/gen_fixtures.py`). Missing fixtures fail the test unless
/// `FASTPROM_ALLOW_MISSING_FIXTURES=1`, in which case it is skipped.
pub fn fixtures_dir() -> Option<PathBuf> {
    let dir = std::env::var_os("FASTPROM_FIXTURES").map_or_else(
        || PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../build/fw/fixtures"),
        PathBuf::from,
    );
    if dir.join("crc_vectors.txt").is_file() {
        return Some(dir);
    }
    if std::env::var_os("FASTPROM_ALLOW_MISSING_FIXTURES").is_some_and(|v| v == "1") {
        eprintln!("skipping: no fixtures in {}", dir.display());
        return None;
    }
    panic!(
        "no fixtures in {} - run `make fw-fixtures` (or set FASTPROM_ALLOW_MISSING_FIXTURES=1)",
        dir.display()
    );
}

fn hex(s: &str) -> Vec<u8> {
    if s == "-" {
        return Vec::new();
    }
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).expect("hex digit"))
        .collect()
}

pub struct CrcVector {
    pub name: String,
    pub data: Vec<u8>,
    pub crc: u32,
}

/// CRC vectors from `zlib.crc32`.
pub fn crc_vectors(dir: &std::path::Path) -> Vec<CrcVector> {
    let text = std::fs::read_to_string(dir.join("crc_vectors.txt")).expect("crc_vectors.txt");
    text.lines()
        .filter(|l| !l.starts_with('#') && !l.is_empty())
        .map(|l| {
            let f: Vec<&str> = l.split(' ').collect();
            let v = CrcVector {
                name: f[0].to_string(),
                data: hex(f[3]),
                crc: u32::from_str_radix(f[2], 16).expect("crc"),
            };
            assert_eq!(
                v.data.len(),
                f[1].parse::<usize>().expect("len"),
                "{}",
                v.name
            );
            v
        })
        .collect()
}

pub struct FixtureImage {
    pub name: String,
    pub crc: u32,
    pub data: Vec<u8>,
}

pub struct TableFixture {
    pub name: String,
    pub part: String,
    pub depth: u32,
    pub width: u8,
    pub images: Vec<FixtureImage>,
    /// The table exactly as tools/mkimagetable.py wrote it.
    pub bytes: Vec<u8>,
}

/// The image tables tools/mkimagetable.py wrote, with what they must hold.
pub fn tables(dir: &std::path::Path) -> Vec<TableFixture> {
    let text = std::fs::read_to_string(dir.join("tables.txt")).expect("tables.txt");
    let mut out: Vec<TableFixture> = Vec::new();
    for line in text.lines() {
        let f: Vec<&str> = line.split(' ').collect();
        match f[0] {
            "table" => out.push(TableFixture {
                name: f[1].to_string(),
                part: f[2].to_string(),
                depth: f[3].parse().expect("depth"),
                width: f[4].parse().expect("width"),
                images: Vec::new(),
                bytes: std::fs::read(dir.join("tables").join(format!("{}.bin", f[1])))
                    .expect("table .bin"),
            }),
            "image" => {
                let t = out.last_mut().expect("image before table");
                let i: usize = f[1].parse().expect("index");
                t.images.push(FixtureImage {
                    name: f[2].to_string(),
                    crc: u32::from_str_radix(f[3], 16).expect("crc"),
                    data: std::fs::read(dir.join("tables").join(format!("{}_{i}.img", t.name)))
                        .expect(".img"),
                });
            }
            other => panic!("tables.txt: unknown record {other}"),
        }
    }
    out
}

// ---- a model of hdl/fastprom_ahb_slave.v ------------------------------------

/// One bus transfer the loader made, and for a read what it returned.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Op {
    Read(ReadReg, u32),
    Write(WriteReg, u32),
}

/// Ways to make the model misbehave.
#[derive(Debug, Default, Clone)]
pub struct Faults {
    /// DATA writes are ignored: the pointer does not move.
    pub ignore_data: bool,
    /// The pointer misses its increment on this DATA write (0-based).
    pub skip_increment_at: Option<usize>,
    /// READY reads as this regardless of what was written.
    pub ready_stuck: Option<bool>,
    /// After this many transfers, ID reads this.
    pub id_after: Option<(usize, u32)>,
    /// ADDR ignores writes and reads 0.
    pub addr_stuck_zero: bool,
}

/// `fastprom_ahb_slave` at the transfer level.
///
/// Pipelined mode models an AHB master that issues the next address phase
/// during the current data phase, back to back, as the HDL sees it: a write
/// takes effect at the end of its data phase, and read data is captured in
/// the read's address phase, so a read straight after a write sees the
/// register as it was before that write. Unpipelined mode has an idle cycle
/// after every transfer, so every read is fresh. `sim/tb_fw_load.v` checks
/// the pipelined model against the HDL.
#[derive(Debug)]
pub struct MockFabric {
    pub id: u32,
    pub aw: u32,
    pub dw: u32,
    pub ready: bool,
    pub ptr: u32,
    pub bram: Vec<Option<u8>>,
    pub data_writes: usize,
    pub ops: Vec<Op>,
    pub faults: Faults,
    pipelined: bool,
    pending: Option<(WriteReg, u32)>,
}

impl MockFabric {
    /// A configured fabric with an `aw`-bit pointer and `dw`-bit words.
    pub fn new(aw: u32, dw: u32) -> Self {
        Self {
            id: fastprom_core::regmap::FABRIC_MAGIC,
            aw,
            dw,
            ready: false,
            ptr: 0,
            bram: vec![None; 1 << aw],
            data_writes: 0,
            ops: Vec::new(),
            faults: Faults::default(),
            pipelined: false,
            pending: None,
        }
    }

    /// The same, behind a pipelined master.
    pub fn pipelined(mut self) -> Self {
        self.pipelined = true;
        self
    }

    fn mask(&self) -> u32 {
        (1u32 << self.aw) - 1
    }

    fn apply(&mut self, reg: WriteReg, value: u32) {
        match reg {
            WriteReg::Ctrl => self.ready = value & 1 != 0,
            WriteReg::Addr => {
                if !self.faults.addr_stuck_zero {
                    self.ptr = value & self.mask();
                }
            }
            WriteReg::Data => {
                let index = self.data_writes;
                self.data_writes += 1;
                if self.faults.ignore_data {
                    return;
                }
                let word = (value & ((1 << self.dw) - 1)) as u8;
                self.bram[self.ptr as usize] = Some(word);
                if self.faults.skip_increment_at != Some(index) {
                    self.ptr = (self.ptr + 1) & self.mask();
                }
            }
        }
    }

    fn current(&self, reg: ReadReg) -> u32 {
        match reg {
            ReadReg::Ctrl => u32::from(self.faults.ready_stuck.unwrap_or(self.ready)),
            ReadReg::Addr => self.ptr,
            ReadReg::Id => match self.faults.id_after {
                Some((n, id)) if self.ops.len() >= n => id,
                _ => self.id,
            },
        }
    }

    /// Let a pipelined write's data phase finish (the bus goes idle).
    pub fn settle(&mut self) {
        if let Some((reg, value)) = self.pending.take() {
            self.apply(reg, value);
        }
    }

    /// What READY reads once the bus is idle.
    pub fn ready_now(&mut self) -> bool {
        self.settle();
        self.faults.ready_stuck.unwrap_or(self.ready)
    }

    /// The block RAM's contents, every word written.
    pub fn contents(&mut self) -> Vec<u8> {
        self.settle();
        self.bram
            .iter()
            .enumerate()
            .map(|(i, w)| w.unwrap_or_else(|| panic!("block RAM word {i} never written")))
            .collect()
    }
}

impl Fabric for MockFabric {
    fn read(&mut self, reg: ReadReg) -> u32 {
        let value = self.current(reg);
        self.settle();
        self.ops.push(Op::Read(reg, value));
        value
    }

    fn write(&mut self, reg: WriteReg, value: u32) {
        self.settle();
        if self.pipelined {
            self.pending = Some((reg, value));
        } else {
            self.apply(reg, value);
        }
        self.ops.push(Op::Write(reg, value));
    }
}

// ---- building tables --------------------------------------------------------

/// A binary image table, built the way tools/mkimagetable.py builds one.
/// `tests/tables.rs` checks it is byte-identical to the tool's output.
pub fn build_table(part: &str, depth: u32, width: u32, images: &[(&str, &[u8])]) -> Vec<u8> {
    fn field(s: &str, len: usize) -> Vec<u8> {
        let mut v = s.as_bytes().to_vec();
        v.truncate(len - 1);
        v.resize(len, 0);
        v
    }
    let mut h = Vec::new();
    for w in [0x4650_524D, 1, depth, width, images.len() as u32] {
        h.extend_from_slice(&u32::to_le_bytes(w));
    }
    h.extend(field(part, 28));
    for (name, data) in images {
        h.extend(field(name, 40));
        h.extend_from_slice(&fastprom_core::crc32::crc32(data).to_le_bytes());
        h.extend_from_slice(&0u32.to_le_bytes());
    }
    h.resize(0xFC, 0);
    let crc = fastprom_core::crc32::crc32(&h);
    h.extend_from_slice(&crc.to_le_bytes());
    for (_, data) in images {
        h.extend_from_slice(data);
    }
    h
}

/// Recompute the header CRC after editing a header field.
pub fn reseal(table: &mut [u8]) {
    let crc = fastprom_core::crc32::crc32(&table[..0xFC]);
    table[0xFC..0x100].copy_from_slice(&crc.to_le_bytes());
}

/// Deterministic test data.
pub fn pattern(len: usize, seed: u32, mask: u8) -> Vec<u8> {
    let mut x = seed.wrapping_mul(2_654_435_761).wrapping_add(1);
    (0..len)
        .map(|_| {
            x ^= x << 13;
            x ^= x >> 17;
            x ^= x << 5;
            (x >> 8) as u8 & mask
        })
        .collect()
}
