//! The load sequence against the HDL model: the success path, every
//! failure, and the invariant that matters most - READY is set only by a
//! load that passed every check, and is cleared before anything else.

mod common;

use common::{build_table, pattern, MockFabric, Op};
use fastprom_core::error::FabricError;
use fastprom_core::{
    boot, Bank, BootError, ImageError, ImageTable, LoadError, ReadReg, Released, SelectError,
    SwitchSample, WriteReg,
};

const AW: u32 = 9;
const DEPTH: u32 = 1 << AW;

fn table(count: usize) -> Vec<u8> {
    let images: Vec<Vec<u8>> = (0..count)
        .map(|i| pattern(DEPTH as usize, i as u32, 0xFF))
        .collect();
    let named: Vec<(String, &[u8])> = images
        .iter()
        .enumerate()
        .map(|(i, d)| (format!("img{i}.bin"), d.as_slice()))
        .collect();
    let refs: Vec<(&str, &[u8])> = named.iter().map(|(n, d)| (n.as_str(), *d)).collect();
    build_table("512x8", DEPTH, 8, &refs)
}

fn bank(n: u8) -> Bank {
    Bank::new(n).expect("bank")
}

fn select(n: u8) -> [SwitchSample; 16] {
    [SwitchSample {
        rx: n & 1 != 0,
        tx: n & 2 != 0,
    }; 16]
}

fn both_buses() -> [MockFabric; 2] {
    [MockFabric::new(AW, 8), MockFabric::new(AW, 8).pipelined()]
}

fn data_writes(fab: &MockFabric) -> usize {
    fab.ops
        .iter()
        .filter(|o| matches!(o, Op::Write(WriteReg::Data, _)))
        .count()
}

#[test]
fn boots_each_bank() {
    let bytes = table(4);
    let parsed = ImageTable::parse(&bytes).expect("parse");
    for n in 0..4 {
        for mut fab in both_buses() {
            let released = Released::claim(&mut fab).expect("claim");
            assert_eq!(released.depth(), DEPTH);
            let report = boot(released, &bytes, &select(n)).expect("boot");
            assert_eq!(report.loaded.bank(), bank(n));
            assert_eq!(report.name, format!("img{n}.bin"));
            assert_eq!(report.part, "512x8");
            assert_eq!(report.count, 4);
            let want = parsed.image(bank(n)).expect("image").words().to_vec();
            assert_eq!(fab.contents(), want, "bank {n}");
            assert!(fab.ready_now());
        }
    }
}

#[test]
fn ready_is_the_last_write_and_nothing_follows_it() {
    let bytes = table(1);
    for mut fab in both_buses() {
        boot(
            Released::claim(&mut fab).expect("claim"),
            &bytes,
            &select(0),
        )
        .expect("boot");
        let last_write = fab
            .ops
            .iter()
            .rposition(|o| matches!(o, Op::Write(..)))
            .expect("writes");
        assert_eq!(fab.ops[last_write], Op::Write(WriteReg::Ctrl, 1));
        // Only the read-back of READY (through ID) comes after it.
        assert_eq!(
            &fab.ops[last_write + 1..],
            &[
                Op::Read(ReadReg::Id, 0x4650_524D),
                Op::Read(ReadReg::Ctrl, 1)
            ]
        );
        // And READY is written exactly twice: cleared first, set last.
        let ctrl: Vec<u32> = fab
            .ops
            .iter()
            .filter_map(|o| match o {
                Op::Write(WriteReg::Ctrl, v) => Some(*v),
                _ => None,
            })
            .collect();
        assert_eq!(ctrl, [0, 1]);
    }
}

#[test]
fn claim_clears_ready_left_over_from_a_warm_reset() {
    // A warm reset does not reconfigure the fabric, so READY can still be
    // set; the first write must clear it.
    for mut fab in both_buses() {
        fab.ready = true;
        assert_eq!(Released::claim(&mut fab).expect("claim").depth(), DEPTH);
        assert!(!fab.ready_now());
        let first_write = fab.ops.iter().find(|o| matches!(o, Op::Write(..)));
        assert_eq!(first_write, Some(&Op::Write(WriteReg::Ctrl, 0)));
        assert_eq!(fab.ops[0], Op::Read(ReadReg::Id, 0x4650_524D));
    }
}

#[test]
fn every_failure_after_claim_leaves_ready_clear() {
    let good = table(2);
    let mut bad_crc = good.clone();
    let last = bad_crc.len() - 1;
    bad_crc[last] ^= 1;
    let mut no_table = good.clone();
    no_table[0] = 0;

    let cases: Vec<(&str, Vec<u8>, [SwitchSample; 16])> = vec![
        ("bank not in table", good.clone(), select(3)),
        ("corrupt image", bad_crc, select(1)),
        ("no table", no_table, select(0)),
    ];
    for (what, bytes, samples) in cases {
        for mut fab in both_buses() {
            fab.ready = true; // as after a warm reset
            let err =
                boot(Released::claim(&mut fab).expect("claim"), &bytes, &samples).expect_err(what);
            assert!(!fab.ready_now(), "{what}: {err}");
        }
    }
    let mut unsteady = select(0);
    unsteady[9].tx = true;
    let mut fab = MockFabric::new(AW, 8);
    let err =
        boot(Released::claim(&mut fab).expect("claim"), &good, &unsteady).expect_err("select");
    assert!(matches!(
        err,
        BootError::Select(SelectError::Unstable { index: 9, .. })
    ));
    assert!(!fab.ready_now());
}

#[test]
fn a_corrupt_image_is_never_written() {
    // Verify before serving, and before loading: not one DATA write.
    let mut bytes = table(1);
    let at = 0x100 + 77;
    bytes[at] ^= 0x40;
    let mut fab = MockFabric::new(AW, 8);
    let err = boot(
        Released::claim(&mut fab).expect("claim"),
        &bytes,
        &select(0),
    )
    .expect_err("crc");
    assert!(matches!(
        err,
        BootError::Load(LoadError::Image {
            source: ImageError::Crc { .. },
            ..
        })
    ));
    assert_eq!(data_writes(&fab), 0);
}

#[test]
fn an_unconfigured_fabric_is_not_written_at_all() {
    // An unconfigured fabric reads as zero.
    let mut fab = MockFabric::new(AW, 8);
    fab.id = 0;
    let err = Released::claim(&mut fab).expect_err("no fabric");
    assert_eq!(err, FabricError::NoFabric { found: 0 });
    assert!(fab.ops.iter().all(|o| matches!(o, Op::Read(..))));
}

fn load_with(faults: common::Faults) -> (Result<(), FabricError>, MockFabric) {
    let bytes = table(1);
    let t = ImageTable::parse(&bytes).expect("parse");
    let img = t.image(bank(0)).expect("image");
    let mut fab = MockFabric::new(AW, 8).pipelined();
    fab.faults = faults;
    let r = Released::claim(&mut fab).and_then(|r| {
        r.load(&img).map(drop).map_err(|e| match e {
            LoadError::Fabric { source, .. } => source,
            other => panic!("not a fabric error: {other}"),
        })
    });
    (r, fab)
}

#[test]
fn fabric_faults_are_caught() {
    use common::Faults;
    let (r, mut f) = load_with(Faults {
        ignore_data: true,
        ..Faults::default()
    });
    assert_eq!(
        r,
        Err(FabricError::Pointer {
            index: 0,
            expected: 1,
            found: 0
        })
    );
    assert!(!f.ready_now());

    let (r, mut f) = load_with(Faults {
        skip_increment_at: Some(300),
        ..Faults::default()
    });
    assert_eq!(
        r,
        Err(FabricError::Pointer {
            index: 300,
            expected: 301,
            found: 300
        })
    );
    assert!(!f.ready_now());

    let (r, _) = load_with(Faults {
        ready_stuck: Some(false),
        ..Faults::default()
    });
    assert_eq!(r, Err(FabricError::ReadyNotSet { ctrl: 0 }));

    let (r, _) = load_with(Faults {
        ready_stuck: Some(true),
        ..Faults::default()
    });
    assert_eq!(r, Err(FabricError::ReleaseFailed { ctrl: 1 }));

    let (r, mut f) = load_with(Faults {
        id_after: Some((200, 0)),
        ..Faults::default()
    });
    assert_eq!(r, Err(FabricError::Lost { found: 0 }));
    assert!(!f.ready_now());

    let (r, _) = load_with(Faults {
        addr_stuck_zero: true,
        ..Faults::default()
    });
    assert_eq!(r, Err(FabricError::PointerProbe { found: 0 }));
}

#[test]
fn a_table_for_another_organisation_is_refused() {
    // A 512-word table into a 256-word fabric (and the reverse) would
    // either overrun or half-fill the block RAM.
    let bytes = table(1);
    let t = ImageTable::parse(&bytes).expect("parse");
    let img = t.image(bank(0)).expect("image");
    for aw in [8, 10] {
        let mut fab = MockFabric::new(aw, 8);
        let err = Released::claim(&mut fab)
            .expect("claim")
            .load(&img)
            .expect_err("depth");
        assert_eq!(
            err,
            LoadError::Fabric {
                bank: bank(0),
                source: FabricError::DepthMismatch {
                    fabric: 1 << aw,
                    image: DEPTH,
                },
            }
        );
        assert_eq!(data_writes(&fab), 0);
        assert!(!fab.ready_now());
    }
}

#[test]
fn the_sequence_does_not_depend_on_bus_pipelining() {
    // The same transfers and the same read values with and without idle
    // cycles between transfers: this is what lets sim/tb_fw_load.v replay
    // one trace both ways.
    let bytes = table(1);
    let [mut plain, mut piped] = both_buses();
    boot(
        Released::claim(&mut plain).expect("claim"),
        &bytes,
        &select(0),
    )
    .expect("boot");
    boot(
        Released::claim(&mut piped).expect("claim"),
        &bytes,
        &select(0),
    )
    .expect("boot");
    assert_eq!(plain.ops, piped.ops);
}

#[test]
fn no_register_is_read_straight_after_a_write() {
    let bytes = table(1);
    let mut fab = MockFabric::new(AW, 8);
    boot(
        Released::claim(&mut fab).expect("claim"),
        &bytes,
        &select(0),
    )
    .expect("boot");
    for pair in fab.ops.windows(2) {
        if let [Op::Write(..), Op::Read(reg, _)] = pair {
            assert_eq!(*reg, ReadReg::Id, "{pair:?}");
        }
    }
}

/// The retired C loader's sequence (fw/src/loader.c): write every word,
/// then read ADDR straight after the last DATA write.
fn c_style_load(fab: &mut MockFabric, words: &[u8]) -> bool {
    use fastprom_core::Fabric;
    if fab.read(ReadReg::Id) != 0x4650_524D {
        return false;
    }
    fab.write(WriteReg::Ctrl, 0);
    fab.write(WriteReg::Addr, 0);
    for &w in words {
        fab.write(WriteReg::Data, u32::from(w));
    }
    if fab.read(ReadReg::Addr) & (words.len() as u32 - 1) != 0 {
        return false; // FASTPROM_ERR_LOAD_FAILED
    }
    fab.write(WriteReg::Ctrl, 1);
    true
}

#[test]
fn the_c_loaders_read_back_was_stale_on_a_pipelined_bus() {
    // Why the ID read sits between every write and read-back. On a master
    // that pipelines, the C loader's ADDR read returns the pointer from
    // before the last DATA write and it fails every boot.
    let words = pattern(DEPTH as usize, 1, 0xFF);
    assert!(c_style_load(&mut MockFabric::new(AW, 8), &words));
    assert!(!c_style_load(
        &mut MockFabric::new(AW, 8).pipelined(),
        &words
    ));
    // And its check passes a fabric that ignores every DATA write, since
    // the pointer then never leaves 0.
    let mut deaf = MockFabric::new(AW, 8);
    deaf.faults.ignore_data = true;
    assert!(c_style_load(&mut deaf, &words));
}
