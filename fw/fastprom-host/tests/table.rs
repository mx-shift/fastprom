//! The image-table builder against `tools/mkimagetable.py`: every table
//! `fw/tools/gen_fixtures.py` had the tool write is built again here from
//! the same source files, read by `rom::parse`, and must be the same bytes.
//! Then ROM-file validation and the page's plan.

use std::path::{Path, PathBuf};

use fastprom_core::crc32::crc32;
use fastprom_core::ImageTable;
use fastprom_host::rom::{self, Geometry};
use fastprom_host::table::{self, Plan, Slot};

/// `build/fw/fixtures` (`make fw-fixtures`), as `fastprom-core`'s tests
/// find it.
fn fixtures() -> Option<PathBuf> {
    let dir = std::env::var_os("FASTPROM_FIXTURES").map_or_else(
        || PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../build/fw/fixtures"),
        PathBuf::from,
    );
    if dir.join("tables.txt").is_file() {
        return Some(dir);
    }
    if std::env::var_os("FASTPROM_ALLOW_MISSING_FIXTURES").is_some_and(|v| v == "1") {
        eprintln!("skipping: no fixtures in {}", dir.display());
        return None;
    }
    panic!("no fixtures in {} - run `make fw-fixtures`", dir.display());
}

struct Fixture {
    name: String,
    part: String,
    geometry: Geometry,
    /// (stored name, CRC, source file)
    images: Vec<(String, u32, String)>,
    bytes: Vec<u8>,
}

fn tables(dir: &Path) -> Vec<Fixture> {
    let text = std::fs::read_to_string(dir.join("tables.txt")).expect("tables.txt");
    let mut out: Vec<Fixture> = Vec::new();
    for line in text.lines() {
        let f: Vec<&str> = line.splitn(5, ' ').collect();
        match f[0] {
            "table" => out.push(Fixture {
                name: f[1].into(),
                part: f[2].into(),
                geometry: Geometry {
                    depth: f[3].parse().unwrap(),
                    width: f[4].split(' ').next().unwrap().parse().unwrap(),
                },
                images: Vec::new(),
                bytes: std::fs::read(dir.join("tables").join(format!("{}.bin", f[1]))).unwrap(),
            }),
            "image" => out.last_mut().unwrap().images.push((
                f[2].into(),
                u32::from_str_radix(f[3], 16).unwrap(),
                f[4].into(),
            )),
            other => panic!("tables.txt: {other}"),
        }
    }
    out
}

#[test]
fn builds_every_tool_written_table_byte_for_byte() {
    let Some(dir) = fixtures() else { return };
    let all = tables(&dir);
    assert!(all.len() >= 6);
    let mut from_bin = 0;
    for t in &all {
        let roms: Vec<rom::Rom> = t
            .images
            .iter()
            .map(|(stored, crc, src)| {
                let bytes = std::fs::read(dir.join("tables").join(src)).unwrap();
                let r = rom::parse(src, &bytes, t.geometry)
                    .unwrap_or_else(|e| panic!("{}: {src}: {e}", t.name));
                assert_eq!(&r.name, stored, "{}: {src}", t.name);
                assert_eq!(r.crc, *crc, "{}: {src}", t.name);
                if !src.to_ascii_lowercase().ends_with(".hex") {
                    from_bin += 1;
                }
                r
            })
            .collect();
        let refs: Vec<(&str, &[u8])> = roms
            .iter()
            .map(|r| (r.name.as_str(), r.words.as_slice()))
            .collect();
        // The part as the tool was given it is not recorded; the stored one
        // is, and storing it again changes nothing.
        let built = table::build(&t.part, t.geometry, &refs).unwrap();
        assert_eq!(built, t.bytes, "{}", t.name);

        // The plan builds the same table from the same files.
        let mut plan = Plan::new(&t.part, t.geometry);
        for (i, r) in roms.iter().enumerate() {
            plan.slots[i] = Slot::Rom(r.clone());
        }
        assert_eq!(plan.build().unwrap(), t.bytes, "{} by plan", t.name);
    }
    assert!(from_bin >= 4, "binary sources, .BIN and .rom among them");
}

#[test]
fn reads_back_the_images_of_a_table() {
    let Some(dir) = fixtures() else { return };
    for t in tables(&dir) {
        let (g, part, images) = table::current(&t.bytes).unwrap();
        assert_eq!(g, t.geometry);
        assert_eq!(part, t.part);
        assert_eq!(images.len(), t.images.len());
        for (i, (stored, crc, _)) in images.iter().zip(&t.images) {
            assert_eq!(&i.name, stored);
            assert_eq!(i.crc, *crc);
            assert_eq!(crc32(&i.words), *crc);
        }
    }
}

const G512: Geometry = Geometry {
    depth: 512,
    width: 8,
};
const G4K4: Geometry = Geometry {
    depth: 4096,
    width: 4,
};

#[test]
fn rom_files_are_checked_against_the_part() {
    // Exactly the part's size.
    let r = rom::parse("a.bin", &[0x5A; 512], G512).unwrap();
    assert_eq!((r.given, r.padded()), (512, 0));
    // Short: padded with blank words, as mkimagetable.py pads.
    let r = rom::parse("a.bin", &[0; 100], G512).unwrap();
    assert_eq!((r.given, r.padded()), (100, 412));
    assert!(r.words[100..].iter().all(|&w| w == 0xFF));
    let r = rom::parse("n.bin", &[1; 10], G4K4).unwrap();
    assert!(r.words[10..].iter().all(|&w| w == 0x0F));
    // Too long, too wide, empty.
    let e = rom::parse("a.bin", &[0; 513], G512).unwrap_err();
    assert!(e.contains("513 words"), "{e}");
    let e = rom::parse("n.bin", &[0x10], G4K4).unwrap_err();
    assert!(e.contains("wider than the part's 4 bits"), "{e}");
    assert!(rom::parse("a.bin", &[], G512).is_err());
    // A case or extension the tool would read as text is binary here.
    assert_eq!(
        rom::parse("A.BIN", &[7; 512], G512).unwrap().words,
        [7; 512]
    );
    assert_eq!(
        rom::parse("dump.rom", &[7; 512], G512).unwrap().words,
        [7; 512]
    );
}

#[test]
fn text_images_as_mkimage_writes_them() {
    let text = b"// a comment\n0f 0x1E\n  ff // trailing\n\n00\n";
    let r = rom::parse("x.hex", text, G512).unwrap();
    assert_eq!(&r.words[..4], &[0x0F, 0x1E, 0xFF, 0x00]);
    assert_eq!(r.given, 4);
    let e = rom::parse("x.hex", b"0f zz\n", G512).unwrap_err();
    assert!(e.contains("line 1"), "{e}");
    let e = rom::parse("x.hex", b":10000000FF\n", G512).unwrap_err();
    assert!(e.contains("Intel HEX"), "{e}");
    let e = rom::parse("x.hex", b"100\n", G512).unwrap_err();
    assert!(e.contains("wider"), "{e}");
}

#[test]
fn stored_names_are_what_the_firmware_reads() {
    assert_eq!(rom::stored_name("dir/sub/ROM.bin"), "ROM.bin");
    assert_eq!(rom::stored_name("C:\\roms\\x.bin"), "x.bin");
    assert_eq!(rom::stored_name("é.bin"), "?.bin");
    assert_eq!(rom::stored_name("tab\there.bin"), "tab?here.bin");
    assert_eq!(rom::stored_name(&"x".repeat(60)).len(), 39);
}

#[test]
fn plan_counts_and_fills_positions() {
    let a = rom::parse("a.bin", &[1; 512], G512).unwrap();
    let mut plan = Plan::new("512x8", G512);
    assert_eq!(plan.count(), 0);
    assert!(plan.build().is_err(), "no image at all");

    plan.slots[0] = Slot::Rom(a.clone());
    assert_eq!(plan.count(), 1);
    plan.slots[1] = Slot::Rom(a.clone());
    assert_eq!(plan.count(), 2);

    // Position 2 alone: a table of four, positions 0, 1 and 3 blank.
    let mut plan = Plan::new("512x8", G512);
    plan.slots[2] = Slot::Rom(a.clone());
    assert_eq!(plan.count(), 4);
    let bytes = plan.build().unwrap();
    let t = ImageTable::parse(&bytes).unwrap();
    assert_eq!(t.count().get(), 4);
    let names: Vec<&str> = (0..4)
        .map(|i| {
            t.image(fastprom_core::Bank::new(i).unwrap())
                .unwrap()
                .name()
        })
        .collect();
    assert_eq!(names, ["(blank)", "(blank)", "a.bin", "(blank)"]);
    let blank = t.image(fastprom_core::Bank::new(0).unwrap()).unwrap();
    assert!(blank.words().iter().all(|&w| w == 0xFF));
}

#[test]
fn builder_refuses_what_the_table_cannot_hold() {
    let w = vec![0u8; 512];
    assert!(table::build("p", G512, &[]).is_err());
    assert!(table::build("p", G512, &[("a", &w), ("b", &w), ("c", &w)]).is_err());
    assert!(table::build("p", G512, &[("a", &w[..100])]).is_err());
    assert!(table::build(
        "p",
        Geometry {
            depth: 500,
            width: 8
        },
        &[("a", &w)]
    )
    .is_err());
    assert!(table::build(
        "p",
        Geometry {
            depth: 512,
            width: 5
        },
        &[("a", &w)]
    )
    .is_err());
    // A control character the tool would store, and the firmware refuse.
    let e = table::build("p", G512, &[("a\tb", &w)]).unwrap_err();
    assert!(e.contains("would refuse"), "{e}");
}
