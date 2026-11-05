//! The parser against tables tools/mkimagetable.py itself wrote, and every
//! image in them loaded into the HDL model.

mod common;

use common::MockFabric;
use fastprom_core::{Bank, ImageTable, Released};

#[test]
fn parses_every_tool_written_table() {
    let Some(dir) = common::fixtures_dir() else {
        return;
    };
    let tables = common::tables(&dir);
    assert!(tables.len() >= 5);
    for t in &tables {
        let table = ImageTable::parse(&t.bytes).unwrap_or_else(|e| panic!("{}: {e}", t.name));
        assert_eq!(table.part(), t.part, "{}", t.name);
        assert_eq!(table.depth(), t.depth, "{}", t.name);
        assert_eq!(table.width().bits(), t.width, "{}", t.name);
        assert_eq!(table.count().get() as usize, t.images.len(), "{}", t.name);
        for (i, want) in t.images.iter().enumerate() {
            let bank = Bank::new(i as u8).expect("bank");
            let img = table.image(bank).expect("image");
            assert_eq!(img.name(), want.name, "{} image {i}", t.name);
            assert_eq!(img.crc32(), want.crc, "{} image {i}", t.name);
            assert_eq!(img.words(), want.data, "{} image {i}", t.name);
            assert_eq!(fastprom_core::loader::verify(&img), Ok(want.crc));
        }
        let missing = Bank::new(t.images.len() as u8);
        assert!(
            missing.is_none_or(|b| table.image(b).is_none()),
            "{}",
            t.name
        );
    }
}

#[test]
fn a_table_parses_with_flash_after_it() {
    // The firmware hands the parser the whole 128 KiB region; what follows
    // the table (erased flash, an older longer table) must not matter.
    let Some(dir) = common::fixtures_dir() else {
        return;
    };
    for t in common::tables(&dir) {
        let mut region = t.bytes.clone();
        region.resize(128 * 1024, 0xFF);
        let table = ImageTable::parse(&region).expect("with trailing flash");
        assert_eq!(table.depth(), t.depth);
    }
}

#[test]
fn the_test_builder_matches_the_tool_byte_for_byte() {
    // tests/malformed.rs mutates tables from common::build_table; this
    // proves that builder and tools/mkimagetable.py agree on the layout.
    let Some(dir) = common::fixtures_dir() else {
        return;
    };
    for t in common::tables(&dir) {
        let images: Vec<(&str, &[u8])> = t
            .images
            .iter()
            .map(|i| (i.name.as_str(), i.data.as_slice()))
            .collect();
        let built = common::build_table(&t.part, t.depth, u32::from(t.width), &images);
        assert_eq!(built, t.bytes, "{}", t.name);
    }
}

#[test]
fn every_tool_written_image_loads_into_the_hdl_model() {
    let Some(dir) = common::fixtures_dir() else {
        return;
    };
    for t in common::tables(&dir) {
        let table = ImageTable::parse(&t.bytes).expect("parse");
        let aw = t.depth.trailing_zeros();
        for (i, want) in t.images.iter().enumerate() {
            for pipelined in [false, true] {
                let mut fab = MockFabric::new(aw, u32::from(t.width));
                if pipelined {
                    fab = fab.pipelined();
                }
                let bank = Bank::new(i as u8).expect("bank");
                let img = table.image(bank).expect("image");
                let loaded = Released::claim(&mut fab)
                    .expect("claim")
                    .load(&img)
                    .unwrap_or_else(|e| panic!("{} image {i}: {e}", t.name));
                assert_eq!(loaded.bank(), bank);
                assert_eq!(loaded.crc32(), want.crc);
                assert_eq!(fab.contents(), want.data, "{} image {i}", t.name);
                assert!(fab.ready_now(), "{} image {i}: READY", t.name);
            }
        }
    }
}
