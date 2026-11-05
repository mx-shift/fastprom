//! Error messages, source chains and failure codes.

use std::collections::HashSet;
use std::error::Error;

use fastprom_core::error::FabricError;
use fastprom_core::{
    Bank, BootError, FailureCode, ImageError, LoadError, SelectError, TableError, TextField,
};

fn chain(e: &dyn Error) -> Vec<String> {
    let mut out = vec![e.to_string()];
    let mut cur = e.source();
    while let Some(s) = cur {
        out.push(s.to_string());
        cur = s.source();
    }
    out
}

fn b(n: u8) -> Bank {
    Bank::new(n).expect("bank")
}

#[test]
fn a_crc_failure_reads_as_a_chain() {
    let e = BootError::from(LoadError::Image {
        bank: b(2),
        source: ImageError::Crc {
            bank: b(2),
            computed: 0x1234_5678,
            stored: 0x9ABC_DEF0,
        },
    });
    assert_eq!(
        chain(&e),
        [
            "loading the selected image",
            "checking image 2",
            "image 2 has CRC 0x12345678, but the table says 0x9abcdef0",
        ]
    );
    assert_eq!(e.code(), 40);
}

#[test]
fn a_fabric_failure_keeps_its_cause() {
    let e = BootError::from(LoadError::Fabric {
        bank: b(1),
        source: FabricError::Pointer {
            index: 5,
            expected: 6,
            found: 5,
        },
    });
    let c = chain(&e);
    assert_eq!(c.len(), 3);
    assert_eq!(c[1], "writing image 1 into the fabric");
    assert_eq!(
        c[2],
        "the load pointer reads 0x5 after word 5, expected 0x6"
    );
    // The outer messages do not repeat the cause.
    assert!(!c[0].contains("pointer") && !c[1].contains("pointer"));
}

#[test]
fn messages_carry_their_values() {
    let no_fabric = FabricError::NoFabric { found: 0 }.to_string();
    assert!(
        no_fabric.contains("0x00000000") && no_fabric.contains("0x4650524d"),
        "{no_fabric}"
    );
    let unsteady = SelectError::Unstable {
        first: b(0),
        index: 3,
        found: b(2),
    }
    .to_string();
    assert!(unsteady.contains("sample 3 reads bank 2"), "{unsteady}");
    let magic = TableError::BadMagic { found: 0x1234 }.to_string();
    assert!(!magic.contains("erased"), "{magic}");
    let name = TableError::Text {
        field: TextField::ImageName(b(3)),
        offset: 0xC0,
    }
    .to_string();
    assert!(name.starts_with("image 3's name at offset 0x0c0"), "{name}");
}

#[test]
fn failure_codes_are_unique_and_in_range() {
    let bank = b(0);
    let fabric = [
        FabricError::NoFabric { found: 0 },
        FabricError::Lost { found: 0 },
        FabricError::ReleaseFailed { ctrl: 1 },
        FabricError::PointerProbe { found: 0 },
        FabricError::DepthMismatch {
            fabric: 1,
            image: 2,
        },
        FabricError::PointerReset { found: 1 },
        FabricError::Pointer {
            index: 0,
            expected: 1,
            found: 0,
        },
        FabricError::ReadyNotSet { ctrl: 0 },
    ];
    let mut all: Vec<BootError> = vec![
        SelectError::NoSamples.into(),
        SelectError::Unstable {
            first: bank,
            index: 1,
            found: bank,
        }
        .into(),
        TableError::TooShort { len: 0 }.into(),
        TableError::BadMagic { found: 0 }.into(),
        TableError::Version { found: 0 }.into(),
        TableError::HeaderCrc {
            computed: 0,
            stored: 1,
        }
        .into(),
        TableError::Depth { depth: 0 }.into(),
        TableError::Width { width: 0 }.into(),
        TableError::Count { count: 0 }.into(),
        TableError::Text {
            field: TextField::PartName,
            offset: 0,
        }
        .into(),
        TableError::Reserved { offset: 0 }.into(),
        TableError::Truncated {
            count: 1,
            depth: 16,
            needed: 1,
            available: 0,
        }
        .into(),
        LoadError::BankNotInTable { bank, count: 1 }.into(),
        LoadError::Image {
            bank,
            source: ImageError::Crc {
                bank,
                computed: 0,
                stored: 1,
            },
        }
        .into(),
        LoadError::Image {
            bank,
            source: ImageError::WordTooWide {
                bank,
                index: 0,
                value: 0x10,
                width: 4,
            },
        }
        .into(),
    ];
    all.extend(
        fabric
            .into_iter()
            .map(|source| BootError::from(LoadError::Fabric { bank, source })),
    );
    let mut seen = HashSet::new();
    for e in &all {
        let code = e.code();
        assert!((10..=99).contains(&code), "{e}: {code}");
        assert!(seen.insert(code), "code {code} used twice ({e})");
        assert!(!e.to_string().is_empty());
    }
}
