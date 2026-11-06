//! The application header and fabric manifest: round trips, the tool that
//! stamps them (`tools/fw_stamp.py`, through fixtures), and every way a page
//! can be refused.

mod common;

use fastprom_core::crc32::crc32;
use fastprom_core::header::{
    boot_sane, page_crc, AppHeader, BoardRev, Commit, HeaderError, Manifest, Part, Text, Version,
    APP_IMAGE_MAX, APP_MAGIC, APP_VERSION, ERASED, FABRIC_IMAGE_MAX, MANIFEST_MAGIC, OFF_PAGE_CRC,
    PAGE_LEN,
};
use fastprom_core::regmap::FABRIC_ID;

fn app() -> AppHeader {
    AppHeader {
        image_len: 19_868,
        image_crc: 0x1234_5678,
        fw_version: Version {
            major: 0,
            minor: 1,
            patch: 0,
        },
        board_rev: BoardRev::new("A").unwrap(),
        fabric_id: FABRIC_ID,
    }
}

fn manifest() -> Manifest {
    Manifest {
        fabric_len: 99_944,
        fabric_crc: 0xCAFE_F00D,
        part: Part::new("512x8").unwrap(),
        board_rev: BoardRev::new("A").unwrap(),
        fabric_id: FABRIC_ID,
    }
}

fn reseal(p: &mut [u8; PAGE_LEN]) {
    let crc = page_crc(p);
    p[OFF_PAGE_CRC..].copy_from_slice(&crc.to_le_bytes());
}

fn put32(p: &mut [u8; PAGE_LEN], at: usize, v: u32) {
    p[at..at + 4].copy_from_slice(&v.to_le_bytes());
}

#[test]
fn round_trips() {
    for commit in [Commit::Committed, Commit::Pending] {
        let page = app().encode(commit);
        assert_eq!(AppHeader::decode(&page), Ok((commit, app())));
        let page = manifest().encode(commit);
        assert_eq!(Manifest::decode(&page), Ok((commit, manifest())));
    }
    assert_eq!(
        &app().encode(Commit::Committed)[..4],
        &APP_MAGIC.to_le_bytes()
    );
    assert_eq!(&app().encode(Commit::Pending)[..4], &[0xFF; 4]);
    assert_eq!(
        &manifest().encode(Commit::Committed)[..4],
        &MANIFEST_MAGIC.to_le_bytes()
    );
    // The commit word is outside the page CRC: programming it last keeps
    // the page valid.
    assert_eq!(
        app().encode(Commit::Committed)[4..],
        app().encode(Commit::Pending)[4..]
    );
    // The linked form is the same fields with a zero page CRC.
    let mut linked = app().fields(APP_MAGIC);
    assert_eq!(&linked[OFF_PAGE_CRC..], &[0; 4]);
    reseal(&mut linked);
    assert_eq!(linked, app().encode(Commit::Committed));
}

#[test]
fn rejects_bad_magic_version_length() {
    let good = app().encode(Commit::Committed);

    for found in [0, APP_MAGIC ^ 1, MANIFEST_MAGIC, APP_MAGIC | 0xF0F0_F0F0] {
        let mut p = good;
        put32(&mut p, 0, found);
        assert_eq!(AppHeader::decode(&p), Err(HeaderError::BadMagic { found }));
    }
    let mut p = good;
    put32(&mut p, 4, 2);
    reseal(&mut p);
    assert_eq!(
        AppHeader::decode(&p),
        Err(HeaderError::Version {
            found: 2,
            expected: APP_VERSION
        })
    );
    for len in [0, APP_IMAGE_MAX + 1, u32::MAX] {
        let mut p = good;
        put32(&mut p, 8, len);
        reseal(&mut p);
        assert_eq!(
            AppHeader::decode(&p),
            Err(HeaderError::Length {
                len,
                max: APP_IMAGE_MAX
            })
        );
    }
    let mut p = good;
    put32(&mut p, 8, APP_IMAGE_MAX);
    reseal(&mut p);
    assert!(AppHeader::decode(&p).is_ok(), "the largest image fits");

    let mut p = manifest().encode(Commit::Committed);
    put32(&mut p, 8, FABRIC_IMAGE_MAX + 1);
    reseal(&mut p);
    assert!(matches!(
        Manifest::decode(&p),
        Err(HeaderError::Length { .. })
    ));
    // An application header is not a manifest, nor the other way round.
    assert!(matches!(
        Manifest::decode(&good),
        Err(HeaderError::BadMagic { .. })
    ));
}

/// Every single-bit flip of a good page is refused (or, in the commit
/// word, turns it into "not committed").
#[test]
fn every_bit_flip_is_caught() {
    for good in [
        app().encode(Commit::Committed),
        manifest().encode(Commit::Committed),
    ] {
        let is_app = good[..4] == APP_MAGIC.to_le_bytes();
        for bit in 0..PAGE_LEN * 8 {
            let mut p = good;
            p[bit / 8] ^= 1 << (bit % 8);
            let r = if is_app {
                AppHeader::decode(&p).map(|(c, _)| c)
            } else {
                Manifest::decode(&p).map(|(c, _)| c)
            };
            assert!(r.is_err(), "bit {bit} flipped and still decodes: {r:?}");
        }
    }
}

#[test]
fn rejects_text_and_reserved() {
    let good = app().encode(Commit::Committed);
    for bad in [
        &b"\0\0\0\0\0\0\0\0"[..],
        b"A B\0\0\0\0\0",
        b"A\0B\0\0\0\0\0",
        b"\x7f\0\0\0\0\0\0\0",
    ] {
        let mut p = good;
        p[0x18..0x20].copy_from_slice(bad);
        reseal(&mut p);
        assert!(
            matches!(AppHeader::decode(&p), Err(HeaderError::Text { .. })),
            "{bad:?}"
        );
    }
    let mut p = good;
    p[0x80] = 1;
    reseal(&mut p);
    assert_eq!(
        AppHeader::decode(&p),
        Err(HeaderError::Reserved { offset: 0x80 })
    );
    let mut p = good;
    p[0x16] = 1; // the version's padding
    reseal(&mut p);
    assert_eq!(AppHeader::decode(&p), Err(HeaderError::FwVersion));

    assert!(Text::<8>::new("").is_none());
    assert!(Text::<8>::new("123456789").is_none());
    assert!(Text::<8>::new("a b").is_none());
    assert_eq!(Text::<8>::new("12345678").unwrap().as_str(), "12345678");
    assert_eq!(Part::new("512x8").unwrap().as_str(), "512x8");
}

#[test]
fn boot_check() {
    assert!(boot_sane(APP_MAGIC, APP_VERSION, 1));
    assert!(boot_sane(APP_MAGIC, APP_VERSION, APP_IMAGE_MAX));
    assert!(!boot_sane(APP_MAGIC, APP_VERSION, 0));
    assert!(!boot_sane(APP_MAGIC, APP_VERSION, APP_IMAGE_MAX + 1));
    assert!(!boot_sane(ERASED, APP_VERSION, 100));
    assert!(!boot_sane(0, APP_VERSION, 100));
    assert!(!boot_sane(APP_MAGIC, 2, 100));
    // An unstamped build (zero length) never boots; erased flash never does.
    let unstamped = app_with_len(0).fields(APP_MAGIC);
    assert!(!boot_sane(
        w(&unstamped, 0),
        w(&unstamped, 4),
        w(&unstamped, 8)
    ));
    let erased = [0xFFu8; PAGE_LEN];
    assert!(!boot_sane(w(&erased, 0), w(&erased, 4), w(&erased, 8)));

    // The fabric half, and the whole check: both, or DFU mode.
    use fastprom_core::header::{
        boot_application, manifest_sane, FABRIC_IMAGE_MAX, MANIFEST_MAGIC, MANIFEST_VERSION,
    };
    assert!(manifest_sane(MANIFEST_MAGIC, MANIFEST_VERSION, 1));
    assert!(manifest_sane(
        MANIFEST_MAGIC,
        MANIFEST_VERSION,
        FABRIC_IMAGE_MAX
    ));
    assert!(!manifest_sane(MANIFEST_MAGIC, MANIFEST_VERSION, 0));
    assert!(!manifest_sane(
        MANIFEST_MAGIC,
        MANIFEST_VERSION,
        FABRIC_IMAGE_MAX + 1
    ));
    assert!(!manifest_sane(ERASED, MANIFEST_VERSION, 100));
    assert!(
        !manifest_sane(0, MANIFEST_VERSION, 100),
        "an erase's zeroed commit word"
    );
    assert!(!manifest_sane(APP_MAGIC, MANIFEST_VERSION, 100));
    let app = [APP_MAGIC, APP_VERSION, 100];
    let man = [MANIFEST_MAGIC, MANIFEST_VERSION, 100];
    assert!(boot_application(app, man));
    assert!(!boot_application(app, [ERASED; 3]), "no fabric: DFU mode");
    assert!(
        !boot_application([ERASED; 3], man),
        "no application: DFU mode"
    );
    assert!(!boot_application([ERASED; 3], [ERASED; 3]));
}

fn app_with_len(image_len: u32) -> AppHeader {
    AppHeader { image_len, ..app() }
}

fn w(p: &[u8; PAGE_LEN], at: usize) -> u32 {
    u32::from_le_bytes(p[at..at + 4].try_into().unwrap())
}

/// The pages `tools/fw_stamp.py` writes decode to the fields it was given,
/// and encoding those fields here gives the same bytes.
#[test]
fn agrees_with_fw_stamp() {
    let Some(dir) = common::fixtures_dir() else {
        return;
    };
    let text = std::fs::read_to_string(dir.join("headers.txt")).expect("headers.txt");
    let mut n = 0;
    for line in text.lines() {
        let f: Vec<&str> = line.split(' ').collect();
        let bytes = std::fs::read(dir.join("headers").join(format!("{}.bin", f[1])))
            .expect("header fixture");
        let page: [u8; PAGE_LEN] = bytes.try_into().expect("one page");
        let len: u32 = f[2].parse().unwrap();
        let crc = u32::from_str_radix(f[3], 16).unwrap();
        let rev = BoardRev::new(f[5]).unwrap();
        let fid: u32 = f[6].parse().unwrap();
        let commit = if f[7] == "1" {
            Commit::Committed
        } else {
            Commit::Pending
        };
        match f[0] {
            "app" => {
                let v: Vec<u16> = f[4].split('.').map(|x| x.parse().unwrap()).collect();
                let want = AppHeader {
                    image_len: len,
                    image_crc: crc,
                    fw_version: Version {
                        major: v[0],
                        minor: v[1],
                        patch: v[2],
                    },
                    board_rev: rev,
                    fabric_id: fid,
                };
                assert_eq!(AppHeader::decode(&page), Ok((commit, want)), "{}", f[1]);
                assert_eq!(want.encode(commit), page, "{}", f[1]);
            }
            "manifest" => {
                let want = Manifest {
                    fabric_len: len,
                    fabric_crc: crc,
                    part: Part::new(f[4]).unwrap(),
                    board_rev: rev,
                    fabric_id: fid,
                };
                assert_eq!(Manifest::decode(&page), Ok((commit, want)), "{}", f[1]);
                assert_eq!(want.encode(commit), page, "{}", f[1]);
            }
            other => panic!("headers.txt: {other}"),
        }
        n += 1;
    }
    assert_eq!(n, 5);
    // And the CRCs are zlib's.
    assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
}
