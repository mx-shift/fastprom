//! CRC-32 against Python's zlib.crc32, which tools/mkimagetable.py uses.
//! If these disagree the firmware rejects every image and the socket stays
//! dead, so the expected values come from zlib, never from this crate.

mod common;

use fastprom_core::crc32::{crc32, Crc32};

/// The textbook bitwise CRC-32, the same loop as the retired C firmware's
/// crc32.c: an independent second implementation.
fn bitwise(data: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFFu32;
    for &b in data {
        crc ^= u32::from(b);
        for _ in 0..8 {
            let mask = (crc & 1).wrapping_neg();
            crc = (crc >> 1) ^ (0xEDB8_8320 & mask);
        }
    }
    !crc
}

#[test]
fn matches_zlib_vectors() {
    let Some(dir) = common::fixtures_dir() else {
        return;
    };
    let vectors = common::crc_vectors(&dir);
    // The C firmware's nine vectors plus the additions.
    assert!(vectors.len() >= 9 + 13, "only {} vectors", vectors.len());
    for v in &vectors {
        assert_eq!(crc32(&v.data), v.crc, "{}", v.name);
        assert_eq!(bitwise(&v.data), v.crc, "{} (bitwise)", v.name);
    }
}

#[test]
fn the_c_vectors_are_all_there() {
    let Some(dir) = common::fixtures_dir() else {
        return;
    };
    let vectors = common::crc_vectors(&dir);
    // fw/test/vectors.h, as the C tests had them.
    let c = [
        ("empty", 0x0000_0000),
        ("one_zero", 0xd202_ef8d),
        ("one_ff", 0xff00_0000),
        ("ascii", 0xfd4c_f1a9),
        ("rand16", 0x1c0e_192e),
        ("rand256", 0x5e14_229f),
        ("rand512", 0x4352_646d),
        ("rand2048", 0x7481_cc21),
        ("all_ff_512", 0xbd7b_c39f),
    ];
    for (name, crc) in c {
        let v = vectors.iter().find(|v| v.name == name).expect(name);
        assert_eq!(v.crc, crc, "{name}");
        assert_eq!(crc32(&v.data), crc, "{name}");
    }
}

#[test]
fn standard_check_value() {
    // CRC-32/ISO-HDLC's catalogued check value.
    assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
}

#[test]
fn incremental_equals_one_shot_at_every_split() {
    let data = common::pattern(300, 7, 0xFF);
    let whole = crc32(&data);
    for split in 0..=data.len() {
        let mut c = Crc32::new();
        c.update(&data[..split]);
        c.update(&data[split..]);
        assert_eq!(c.finish(), whole, "split at {split}");
    }
    let mut c = Crc32::default();
    data.iter().for_each(|&b| c.byte(b));
    assert_eq!(c.finish(), whole);
}

#[test]
fn nibble_table_equals_bitwise_on_every_byte_and_length() {
    for len in 0..64 {
        let data = common::pattern(len, len as u32, 0xFF);
        assert_eq!(crc32(&data), bitwise(&data), "len {len}");
    }
    for b in 0..=255u8 {
        assert_eq!(crc32(&[b]), bitwise(&[b]), "byte {b:#04x}");
    }
}
