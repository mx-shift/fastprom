//! Untrusted flash: the table parser must reject every malformed table with
//! an error, never panic, and never accept one whose header was damaged.

mod common;

use common::{build_table, pattern, reseal};
use fastprom_core::{ImageTable, TableError, TextField};

fn good() -> Vec<u8> {
    let a = pattern(256, 1, 0xFF);
    let b = pattern(256, 2, 0xFF);
    build_table("256x8", 256, 8, &[("a.bin", &a), ("b.bin", &b)])
}

fn set_u32(t: &mut [u8], at: usize, v: u32) {
    t[at..at + 4].copy_from_slice(&v.to_le_bytes());
}

#[test]
fn erased_flash_says_so() {
    let erased = vec![0xFFu8; 128 * 1024];
    let err = ImageTable::parse(&erased).expect_err("erased");
    assert_eq!(err, TableError::BadMagic { found: u32::MAX });
    assert!(err.to_string().contains("erased"), "{err}");
}

#[test]
fn every_truncation_is_an_error() {
    let t = good();
    for len in 0..t.len() {
        let err = ImageTable::parse(&t[..len]).expect_err("truncated");
        if len < 0x100 {
            assert_eq!(err, TableError::TooShort { len });
        } else {
            assert_eq!(
                err,
                TableError::Truncated {
                    count: 2,
                    depth: 256,
                    needed: t.len(),
                    available: len,
                }
            );
        }
    }
    assert!(ImageTable::parse(&t).is_ok());
}

#[test]
fn every_header_bit_flip_is_caught() {
    // The header CRC covers every header byte, so no single flipped bit can
    // get through (and none may panic on the way).
    let t = good();
    for byte in 0..0x100 {
        for bit in 0..8 {
            let mut m = t.clone();
            m[byte] ^= 1 << bit;
            assert!(ImageTable::parse(&m).is_err(), "byte {byte:#x} bit {bit}");
        }
    }
}

#[test]
fn resealed_bad_fields_are_named() {
    type Edit = Box<dyn Fn(&mut Vec<u8>)>;
    let cases: Vec<(&str, Edit, TableError)> = vec![
        (
            "version",
            Box::new(|t| set_u32(t, 4, 2)),
            TableError::Version { found: 2 },
        ),
        (
            "depth 0",
            Box::new(|t| set_u32(t, 8, 0)),
            TableError::Depth { depth: 0 },
        ),
        (
            "depth 300",
            Box::new(|t| set_u32(t, 8, 300)),
            TableError::Depth { depth: 300 },
        ),
        (
            "depth 8",
            Box::new(|t| set_u32(t, 8, 8)),
            TableError::Depth { depth: 8 },
        ),
        (
            "depth 2^17",
            Box::new(|t| set_u32(t, 8, 1 << 17)),
            TableError::Depth { depth: 1 << 17 },
        ),
        (
            "width 6",
            Box::new(|t| set_u32(t, 12, 6)),
            TableError::Width { width: 6 },
        ),
        (
            "count 0",
            Box::new(|t| set_u32(t, 16, 0)),
            TableError::Count { count: 0 },
        ),
        (
            "count 3",
            Box::new(|t| set_u32(t, 16, 3)),
            TableError::Count { count: 3 },
        ),
        (
            "count 8",
            Box::new(|t| set_u32(t, 16, 8)),
            TableError::Count { count: 8 },
        ),
        (
            "part unterminated",
            Box::new(|t| t[0x14..0x30].fill(b'A')),
            TableError::Text {
                field: TextField::PartName,
                offset: 0x14,
            },
        ),
        (
            "part control char",
            Box::new(|t| t[0x15] = b'\n'),
            TableError::Text {
                field: TextField::PartName,
                offset: 0x14,
            },
        ),
        (
            "junk after the part's NUL",
            Box::new(|t| t[0x2E] = b'x'),
            TableError::Text {
                field: TextField::PartName,
                offset: 0x14,
            },
        ),
        (
            "name not ASCII",
            Box::new(|t| t[0x60] = 0xC3),
            TableError::Text {
                field: TextField::ImageName(fastprom_core::Bank::new(1).expect("bank")),
                offset: 0x60,
            },
        ),
        (
            "entry reserved",
            Box::new(|t| t[0x5C] = 1),
            TableError::Reserved { offset: 0x5C },
        ),
        (
            "unused entry",
            Box::new(|t| t[0x90] = 1),
            TableError::Reserved { offset: 0x90 },
        ),
        (
            "tail reserved",
            Box::new(|t| t[0xF4] = 1),
            TableError::Reserved { offset: 0xF0 },
        ),
    ];
    for (what, edit, want) in cases {
        let mut t = good();
        edit(&mut t);
        reseal(&mut t);
        assert_eq!(ImageTable::parse(&t).expect_err(what), want, "{what}");
    }
}

#[test]
fn a_header_crc_mismatch_reports_both_values() {
    let mut t = good();
    t[0x14] = b'X';
    let err = ImageTable::parse(&t).expect_err("crc");
    let TableError::HeaderCrc { computed, stored } = err else {
        panic!("{err:?}")
    };
    assert_ne!(computed, stored);
    assert_eq!(
        stored,
        u32::from_le_bytes(t[0xFC..0x100].try_into().expect("4"))
    );
}

/// xorshift: deterministic, no dependencies.
struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
}

#[test]
fn fuzz_loop() {
    // Random edits, some resealed so they get past the CRC and exercise the
    // field checks. Must never panic; whatever parses must be in bounds.
    let base = good();
    let mut rng = Rng(0x5EED_F00D);
    let mut parsed = 0;
    for round in 0..20_000 {
        let mut t = base.clone();
        for _ in 0..1 + rng.below(4) {
            match rng.below(4) {
                0 if !t.is_empty() => {
                    let at = rng.below(t.len());
                    t[at] = rng.next() as u8;
                }
                0 => {}
                1 => {
                    let at = rng.below(0x100 / 4) * 4;
                    let v = match rng.below(3) {
                        0 => rng.next() as u32,
                        1 => 1 << rng.below(32),
                        _ => rng.below(9) as u32,
                    };
                    if at + 4 <= t.len() {
                        set_u32(&mut t, at, v);
                    }
                }
                2 => t.truncate(rng.below(t.len() + 1)),
                _ => t.extend(std::iter::repeat_n(0xFF, rng.below(600))),
            }
        }
        if round % 2 == 0 && t.len() >= 0x100 {
            reseal(&mut t);
        }
        if let Ok(table) = ImageTable::parse(&t) {
            parsed += 1;
            let depth = table.depth() as usize;
            for n in 0..4 {
                let bank = fastprom_core::Bank::new(n).expect("bank");
                if let Some(img) = table.image(bank) {
                    assert_eq!(img.words().len(), depth);
                    assert!(0x100 + (n as usize + 1) * depth <= t.len());
                    let _ = fastprom_core::loader::verify(&img);
                }
            }
        }
    }
    assert!(parsed > 0, "the fuzz loop never produced a valid table");
}
