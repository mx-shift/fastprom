//! The compressed-bitstream decoder (fastprom_core::fcb) against Supra's own
//! output and against an encoder written here from the format.
//!
//! The decisive case is the DFU stage's committed fabric: it must expand to
//! the plain bitstream Supra wrote from the same routing, whose length and
//! CRC-32 tools/ag32/check_dfu_fabric.py recorded in its manifest (and
//! checked against a second decoder, in Python).

use std::collections::HashMap;
use std::path::PathBuf;

use fastprom_core::crc32::crc32;
use fastprom_core::fcb::{decompress, LzwError, Scratch, CONFIG_LEN, CONFIG_WORDS};

fn expand(src: &[u8]) -> Result<Vec<u8>, LzwError> {
    let mut out = Vec::with_capacity(CONFIG_LEN);
    let mut scratch = Box::new(Scratch::new());
    decompress(src, &mut scratch, |w| {
        out.extend_from_slice(&w.to_le_bytes())
    })?;
    Ok(out)
}

fn manifest_field(manifest: &str, key: &str) -> u32 {
    let v = manifest
        .lines()
        .find_map(|l| l.strip_prefix(key)?.strip_prefix(' '))
        .unwrap_or_else(|| panic!("manifest has no {key}"));
    match v.trim().strip_prefix("0x") {
        Some(hex) => u32::from_str_radix(hex, 16).unwrap(),
        None => v.trim().parse().unwrap(),
    }
}

#[test]
fn dfu_fabric_expands_to_supras_plain_bitstream() {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../fastprom-dfu/fabric");
    let bin = std::fs::read(dir.join("dfu_usb.bin")).unwrap();
    let manifest = std::fs::read_to_string(dir.join("dfu_usb.manifest")).unwrap();
    assert_eq!(bin.len() as u32, manifest_field(&manifest, "bin_len"));
    assert_eq!(crc32(&bin), manifest_field(&manifest, "bin_crc32"));

    let raw = expand(&bin).unwrap();
    assert_eq!(raw.len(), CONFIG_LEN);
    assert_eq!(raw.len() as u32, manifest_field(&manifest, "raw_len"));
    assert_eq!(crc32(&raw), manifest_field(&manifest, "raw_crc32"));
    // The AGRV2K's device ID leads it (boards/agrv2k_*.json device_id).
    assert_eq!(raw[..4], [0x40, 0x20, 0x00, 0x01]);
    // And the compression is worth carrying: about 3 % of the plain size.
    assert!(bin.len() < CONFIG_LEN / 20, "{} bytes", bin.len());
}

/// An encoder for the format fcb.c decodes, independent of the decoder:
/// MSB-first codes, 9 bits to start, CLEAR = 256, entries from 258, the
/// width one bit wider as soon as the decoder's next code would not fit.
/// `limit` entries and it clears the dictionary.
fn compress(data: &[u8], limit: u32) -> Vec<u8> {
    struct Bits {
        out: Vec<u8>,
        acc: u64,
        n: u32,
    }
    impl Bits {
        fn put(&mut self, code: u32, width: u32) {
            assert!(code < 1 << width, "code {code} in {width} bits");
            self.acc = (self.acc << width) | u64::from(code);
            self.n += width;
            while self.n >= 8 {
                self.n -= 8;
                self.out.push((self.acc >> self.n) as u8);
            }
        }
    }
    let mut bits = Bits {
        out: data[..8].to_vec(),
        acc: 0,
        n: 0,
    };
    let mut dict: HashMap<(u32, u8), u32> = HashMap::new();
    let mut width = 9;
    let mut emitted = 0u32; // codes since the last CLEAR
    let emit = |bits: &mut Bits, code: u32, width: &mut u32, emitted: &mut u32| {
        bits.put(code, *width);
        *emitted += 1;
        // The decoder defines an entry on every code but the first after
        // a CLEAR, then widens if the next code to define does not fit.
        if *emitted >= 2 && 258 + (*emitted - 1) > (1 << *width) - 1 {
            *width += 1;
        }
    };
    let body = &data[8..];
    let mut w = u32::from(body[0]);
    for &c in &body[1..] {
        if let Some(&code) = dict.get(&(w, c)) {
            w = code;
            continue;
        }
        emit(&mut bits, w, &mut width, &mut emitted);
        if dict.len() as u32 + 1 >= limit {
            bits.put(256, width);
            dict.clear();
            width = 9;
            emitted = 0;
        } else {
            dict.insert((w, c), 258 + dict.len() as u32);
        }
        w = u32::from(c);
    }
    emit(&mut bits, w, &mut width, &mut emitted);
    if bits.n > 0 {
        bits.out.push((bits.acc << (8 - bits.n)) as u8);
    }
    bits.out
}

/// A configuration-sized test pattern like a bitstream: long runs of 0x00
/// and 0xFF (repeated strings of one byte, the decoder's cScSc case),
/// repeated frames, and noise.
fn pattern(seed: u32) -> Vec<u8> {
    let mut x = seed;
    let mut rnd = || {
        x = x.wrapping_mul(1_103_515_245).wrapping_add(12_345);
        (x >> 16) as u8
    };
    let mut v = vec![0x40, 0x20, 0x00, 0x01, 0x00, 0x00, 0xFF, 0xFF];
    while v.len() < CONFIG_LEN {
        match rnd() % 4 {
            0 => v.extend(std::iter::repeat_n(0, usize::from(rnd()) * 4)),
            1 => v.extend(std::iter::repeat_n(0xFF, usize::from(rnd()))),
            2 => {
                let frame: Vec<u8> = (0..24).map(|_| rnd()).collect();
                for _ in 0..rnd() % 8 {
                    v.extend_from_slice(&frame);
                }
            }
            _ => v.extend((0..usize::from(rnd())).map(|_| rnd())),
        }
    }
    v.truncate(CONFIG_LEN);
    v
}

#[test]
fn round_trips_through_an_independent_encoder() {
    for (seed, limit) in [(1, 700), (2, 1020), (3, 300), (4, 1020)] {
        let data = pattern(seed);
        let packed = compress(&data, limit);
        assert_eq!(expand(&packed).unwrap(), data, "seed {seed}, limit {limit}");
    }
}

#[test]
fn writes_exactly_one_configuration_of_words() {
    let data = pattern(5);
    let mut packed = compress(&data, 1020);
    // Trailing input past the end is not read.
    packed.extend_from_slice(&[0xAA; 64]);
    let mut words = 0;
    let mut scratch = Box::new(Scratch::new());
    decompress(&packed, &mut scratch, |_| words += 1).unwrap();
    assert_eq!(words, CONFIG_WORDS);
}

#[test]
fn malformed_streams_are_errors() {
    let data = pattern(6);
    let packed = compress(&data, 1020);

    assert_eq!(expand(&packed[..5]), Err(LzwError::NoHeader { len: 5 }));
    assert!(matches!(
        expand(&packed[..packed.len() / 2]),
        Err(LzwError::Truncated { .. })
    ));

    // STOP (257) as the first code.
    let mut stop = data[..8].to_vec();
    stop.extend_from_slice(&[0x80, 0x80]); // 1_0000_0001 then padding
    assert!(matches!(
        expand(&stop),
        Err(LzwError::Stopped { bytes: 0, .. })
    ));

    // A dictionary code before any entry exists: 258 first.
    let mut undefined = data[..8].to_vec();
    undefined.extend_from_slice(&[0x81, 0x00]);
    assert_eq!(
        expand(&undefined),
        Err(LzwError::UndefinedCode {
            code: 258,
            next: 258
        })
    );

    // Codes past the next one, anywhere in the stream: an error, never a
    // read outside the tables.
    for at in [100, 1000, packed.len() - 10] {
        let mut bad = packed.clone();
        for b in &mut bad[at..at + 4] {
            *b = 0xFF;
        }
        let _ = expand(&bad); // any result, no panic
    }
}

#[test]
fn a_dictionary_never_cleared_is_an_error() {
    // Every byte different from its neighbours' pairs fills the dictionary;
    // with no CLEAR the decoder must stop at its last slot.
    let data = pattern(7);
    let packed = compress(&data, u32::MAX);
    assert_eq!(expand(&packed), Err(LzwError::DictionaryFull));
}
