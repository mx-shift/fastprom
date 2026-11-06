//! The page's procedures end to end against the simulated board: the real
//! update core over the NOR flash model, through the client and the
//! simulated endpoints, with reboots into DFU and back and power cuts.

use fastprom_core::crc32::crc32;
use fastprom_core::header::{boot_application, BoardRev, Part, Version, APP_MAGIC};
use fastprom_core::memmap::{APP_BASE, FABRIC_BASE, MANIFEST_BASE, TABLE_BASE};
use fastprom_core::update::Status;
use fastprom_host::bundle::{self, AppPayload, Bundle, FabricPayload};
use fastprom_host::client::{Client, Error};
use fastprom_host::info::{DeviceInfo, Mode};
use fastprom_host::rom::Geometry;
use fastprom_host::sim::{
    self, block_on, SimConfig, SimDevice, SimTransport, BCD_APPLICATION, BCD_DFU,
};
use fastprom_host::table::{Plan, Slot};
use fastprom_host::{rom, update};

const G: Geometry = Geometry {
    depth: 512,
    width: 8,
};

fn rev(s: &str) -> BoardRev {
    BoardRev::new(s).unwrap()
}

fn config(board: &str) -> SimConfig {
    SimConfig {
        board_rev: rev(board),
        dfu_version: Version {
            major: 0,
            minor: 1,
            patch: 0,
        },
        part: Part::new("512x8").unwrap(),
        geometry: G,
    }
}

const OLD: Version = Version {
    major: 0,
    minor: 1,
    patch: 0,
};
const NEW: Version = Version {
    major: 0,
    minor: 2,
    patch: 7,
};

fn factory() -> SimDevice {
    SimDevice::factory(config("A"), OLD)
}

fn connect(d: &mut SimDevice) -> update::Connected {
    block_on(update::connect(&mut Client::new(SimTransport(d)))).expect("connect")
}

fn info(d: &mut SimDevice) -> DeviceInfo {
    connect(d).info
}

fn noop() -> impl FnMut(&str, u32, u32) {
    |_, _, _| ()
}

/// An update bundle as `tools/mkbundle.py` writes one, in memory.
fn bundle(board: &str) -> (Bundle, Vec<u8>, Vec<u8>, Vec<u8>) {
    let app = sim::app_file(NEW, rev(board), 9000, 77);
    let (fabric, page) = sim::fabric_files(Part::new("512x8").unwrap(), rev(board), 30_000, 78);
    let json = format!(
        r#"{{"format": 1, "board_rev": "{board}",
            "firmware": {{"file": "fw.bin", "version": "0.2.7", "board_rev": "{board}",
                         "fabric_id": 1, "size": {}, "crc32": "{:#010x}",
                         "image_len": 9000, "image_crc32": "{:#010x}"}},
            "fabrics": [{{"part": "512x8", "file": "f.bin", "manifest": "f.manifest.bin",
                          "board_rev": "{board}", "fabric_id": 1, "size": 30000,
                          "crc32": "{:#010x}"}}],
            "notes": []}}"#,
        app.len(),
        crc32(&app),
        crc32(&app[256..]),
        crc32(&fabric)
    );
    (bundle::parse(&json).expect("manifest"), app, fabric, page)
}

/// `tools/partdb.yaml`'s keys, as the page has them from `parts.json`.
fn known() -> Vec<String> {
    ["512x8", "256x8", "4096x4", "2048x4"]
        .map(String::from)
        .to_vec()
}

fn payloads(board: &str) -> (Bundle, AppPayload, FabricPayload) {
    let (b, app, fabric, page) = bundle(board);
    let a = bundle::check_firmware(b.firmware.as_ref().unwrap(), &app).expect("firmware");
    let f = bundle::check_fabric(&b.fabrics[0], &fabric, &page).expect("fabric");
    (b, a, f)
}

#[test]
fn connect_reports_the_factory_board() {
    let mut d = factory();
    assert_eq!(d.bcd_device(), BCD_APPLICATION);
    let c = connect(&mut d);
    let i = &c.info;
    assert_eq!(i.mode, Mode::Application);
    assert_eq!(i.fw_version, "0.1.0");
    assert_eq!(i.board_rev, "A");
    assert_eq!(i.bank, Some(0));
    let app = i.app.as_ref().unwrap();
    assert!(app.committed);
    assert_eq!(app.version, "0.1.0");
    let fabric = i.fabric.as_ref().unwrap();
    assert!(fabric.committed);
    assert_eq!(fabric.part, "512x8");
    let t = i.table.as_ref().unwrap();
    assert_eq!((t.depth, t.width, t.count), (512, 8, 4));
    let table = c.table.unwrap();
    assert_eq!(table.len(), 256 + 4 * 512);
    assert_eq!(table, d.board().bytes(TABLE_BASE, table.len() as u32));
    // The socket serves position 0's image.
    assert_eq!(d.status().served, Some(t.crcs[0]));
}

#[test]
fn image_table_write_verify_load() {
    let mut d = factory();
    let c = connect(&mut d);
    let old = c.table.unwrap();

    // Position 0 a new file, 1 kept, 2 and 3 empty: a table of two.
    let new = rom::parse("new.bin", &[0x42; 512], G).unwrap();
    let kept = fastprom_host::table::kept(&old, 1, G).unwrap();
    assert_eq!(kept.name, "factory-1.bin");
    let mut plan = Plan::new("512x8", G);
    plan.slots[0] = Slot::Rom(new.clone());
    plan.slots[1] = Slot::Rom(kept.clone());
    let table = plan.build().unwrap();

    let mut steps = Vec::new();
    let mut c = Client::new(SimTransport(&mut d));
    block_on(update::write_table(
        &mut c,
        &table,
        &mut |s, done, total| {
            steps.push((s.to_owned(), done, total));
        },
    ))
    .unwrap();
    assert!(steps.iter().any(|(s, ..)| s.starts_with("verifying")));
    assert_eq!(
        steps.last().unwrap().1,
        steps.last().unwrap().2,
        "progress ends full"
    );
    block_on(update::load(&mut c)).unwrap();
    assert_eq!(d.status().served, Some(new.crc));

    let i = info(&mut d);
    let t = i.table.unwrap();
    assert_eq!(t.count, 2);
    assert_eq!(t.crcs, [new.crc, kept.crc]);
}

#[test]
fn load_of_a_position_the_table_lacks_is_refused_and_releases_the_socket() {
    let mut d = factory();
    d.set_switch(3);
    d.power_off();
    d.power_on();
    let mut plan = Plan::new("512x8", G);
    plan.slots[0] = Slot::Rom(rom::parse("a.bin", &[1; 512], G).unwrap());
    let table = plan.build().unwrap();
    let mut c = Client::new(SimTransport(&mut d));
    block_on(update::write_table(&mut c, &table, &mut noop())).unwrap();
    let e = block_on(update::load(&mut c)).unwrap_err();
    assert_eq!(e.status(), Some(Status::InvalidData), "{e}");
    // The client recovered: the next command works.
    let i = block_on(c.info()).unwrap();
    assert_eq!(i.bank, Some(3));
    assert_eq!(d.status().served, None);
}

/// Into DFU, update, and back: what the page does.
fn through_dfu(d: &mut SimDevice, a: &AppPayload, f: Option<&FabricPayload>) -> Result<(), Error> {
    block_on(update::enter_dfu(&mut Client::new(SimTransport(d))))?;
    assert!(!d.connected(), "gone after REBOOT");
    assert!(d.reboot_pending());
    d.complete_reboot();
    assert_eq!(d.bcd_device(), BCD_DFU);
    let dfu = info(d);
    assert_eq!(dfu.mode, Mode::Dfu);
    let mut c = Client::new(SimTransport(d));
    if let Some(f) = f {
        block_on(update::update_fabric(&mut c, &dfu, f, &mut noop()))?;
    }
    block_on(update::update_firmware(&mut c, &dfu, a, &mut noop()))?;
    block_on(update::reboot_application(&mut c))?;
    d.complete_reboot();
    assert_eq!(d.bcd_device(), BCD_APPLICATION);
    Ok(())
}

#[test]
fn firmware_and_fabric_update_through_dfu() {
    let (b, a, f) = payloads("A");
    let mut d = factory();
    let before = info(&mut d);
    let plan = bundle::plan(&b, &before, None, &known());
    assert_eq!(plan.refuse, None);
    assert!(plan.firmware.available && !plan.firmware.up_to_date);
    assert!(plan.fabric.available && !plan.fabric.up_to_date);
    assert_eq!(plan.part.as_deref(), Some("512x8"));

    through_dfu(&mut d, &a, Some(&f)).unwrap();
    let after = info(&mut d);
    assert_eq!(after.mode, Mode::Application);
    assert_eq!(after.fw_version, "0.2.7");
    let m = after.fabric.as_ref().unwrap();
    assert!(m.committed);
    assert_eq!(m.crc, f.manifest.fabric_crc);
    assert_eq!(d.board().bytes(FABRIC_BASE, 30_000), f.data[..30_000]);
    let plan = bundle::plan(&b, &after, None, &known());
    assert!(plan.firmware.up_to_date && plan.fabric.up_to_date);
    // The image table was left alone, and is served again.
    assert_eq!(after.table, before.table);
    assert_eq!(d.status().served, Some(before.table.unwrap().crcs[0]));
}

#[test]
fn wrong_board_revision_is_refused() {
    let (b, a, _) = payloads("B");
    let mut d = factory();
    let i = info(&mut d);
    let plan = bundle::plan(&b, &i, None, &known());
    let why = plan.refuse.expect("refused");
    assert!(why.contains("board revision B"), "{why}");
    assert!(!plan.firmware.available && !plan.fabric.available);

    // And if the page went ahead anyway: the host refuses before writing,
    // and the device would refuse the COMMIT.
    block_on(update::enter_dfu(&mut Client::new(SimTransport(&mut d)))).unwrap();
    d.complete_reboot();
    let dfu = info(&mut d);
    let e = block_on(update::update_firmware(
        &mut Client::new(SimTransport(&mut d)),
        &dfu,
        &a,
        &mut noop(),
    ))
    .unwrap_err();
    assert!(matches!(e, Error::Check(_)), "{e}");
    assert_eq!(
        d.board().bytes(APP_BASE, 4),
        APP_MAGIC.to_le_bytes(),
        "untouched"
    );

    let mut c = Client::new(SimTransport(&mut d));
    block_on(c.erase(APP_BASE, 48 * 1024)).unwrap();
    block_on(c.write(APP_BASE, &a.data, &mut |_| ())).unwrap();
    assert_eq!(
        block_on(c.commit(0)).unwrap_err().status(),
        Some(Status::NotPermitted)
    );
}

#[test]
fn the_application_may_not_write_firmware() {
    let (_, a, _) = payloads("A");
    let mut d = factory();
    let i = info(&mut d);
    let e = block_on(update::update_firmware(
        &mut Client::new(SimTransport(&mut d)),
        &i,
        &a,
        &mut noop(),
    ))
    .unwrap_err();
    assert!(
        matches!(e, Error::Check(_)),
        "the page checks the mode: {e}"
    );
    let mut c = Client::new(SimTransport(&mut d));
    assert_eq!(
        block_on(c.erase(APP_BASE, 4096)).unwrap_err().status(),
        Some(Status::NotPermitted)
    );
    assert_eq!(
        block_on(c.commit(0)).unwrap_err().status(),
        Some(Status::NotPermitted)
    );
}

/// Power cut at a spread of flash operations of a firmware update; at
/// power-up the board is in DFU mode or on the old firmware, never on a
/// partial one, and the page's update run again succeeds.
#[test]
fn power_cut_during_firmware_update_then_recovery() {
    let (_, a, _) = payloads("A");
    // How many flash operations the update takes.
    let total = {
        let mut d = factory();
        block_on(update::enter_dfu(&mut Client::new(SimTransport(&mut d)))).unwrap();
        d.complete_reboot();
        let dfu = info(&mut d);
        let before = d.status().flash_ops;
        block_on(update::update_firmware(
            &mut Client::new(SimTransport(&mut d)),
            &dfu,
            &a,
            &mut noop(),
        ))
        .unwrap();
        d.status().flash_ops - before
    };
    assert!(total > 2000, "{total}");
    for cut in [1, 2, 5, 13, 200, total / 2, total - 1, total] {
        let mut d = factory();
        block_on(update::enter_dfu(&mut Client::new(SimTransport(&mut d)))).unwrap();
        d.complete_reboot();
        let dfu = info(&mut d);
        d.cut_power_after(cut);
        let e = block_on(update::update_firmware(
            &mut Client::new(SimTransport(&mut d)),
            &dfu,
            &a,
            &mut noop(),
        ))
        .unwrap_err();
        assert!(e.is_gone(), "cut {cut}: {e}");
        assert!(!d.connected() && !d.status().powered);

        d.power_on();
        let b = d.board();
        let w = |at: u32| u32::from_le_bytes(b.bytes(at, 4).try_into().unwrap());
        let three = |at: u32| [w(at), w(at + 4), w(at + 8)];
        let sane = boot_application(three(APP_BASE), three(MANIFEST_BASE));
        let i = info(&mut d);
        assert_eq!(i.mode == Mode::Application, sane, "cut {cut}");
        assert_eq!(
            i.mode,
            Mode::Dfu,
            "cut {cut}: the old commit word goes first"
        );

        // The page resumes in DFU mode.
        let mut c = Client::new(SimTransport(&mut d));
        block_on(update::update_firmware(&mut c, &i, &a, &mut noop())).unwrap();
        block_on(update::reboot_application(&mut c)).unwrap();
        d.complete_reboot();
        assert_eq!(info(&mut d).fw_version, "0.2.7", "cut {cut}");
    }
}

#[test]
fn power_cut_during_table_write_leaves_the_socket_released() {
    let mut d = factory();
    let mut plan = Plan::new("512x8", G);
    plan.slots[0] = Slot::Rom(rom::parse("a.bin", &[9; 512], G).unwrap());
    let table = plan.build().unwrap();
    d.cut_power_after(100);
    let e = block_on(update::write_table(
        &mut Client::new(SimTransport(&mut d)),
        &table,
        &mut noop(),
    ))
    .unwrap_err();
    assert!(e.is_gone(), "{e}");
    d.power_on();
    let c = connect(&mut d);
    assert_eq!(
        c.info.mode,
        Mode::Application,
        "the firmware was not touched"
    );
    // The header went first and is whole: INFO's "valid" flag is the
    // header's, and its "intact" flag and mask say the image under it is
    // not, as the firmware's check at boot found...
    let ti = c.info.table.as_ref().expect("the header parses");
    assert_eq!(ti.intact, Some(vec![false]));
    assert!(!ti.all_intact());
    // ...as the page also sees in the read-back...
    let t = c.table.expect("the header parses");
    let (_, _, images) = fastprom_host::table::current(&t).unwrap();
    assert_ne!(crc32(&images[0].words), images[0].crc);
    assert!(
        fastprom_host::table::kept(&t, 0, G).is_err(),
        "a damaged image is not kept"
    );
    // ...and the firmware refuses to serve.
    assert_eq!(d.status().served, None, "the socket stays off");
    // Written again, and loaded.
    let mut c = Client::new(SimTransport(&mut d));
    block_on(update::write_table(&mut c, &table, &mut noop())).unwrap();
    block_on(update::load(&mut c)).unwrap();
    assert!(d.status().served.is_some());
    let ti = info(&mut d).table.expect("a table");
    assert_eq!(ti.intact, Some(vec![true]));
    assert!(ti.all_intact());
}

/// An interrupted fabric update erases the manifest first: the board then
/// reports no part, and the page falls back on the image table's part
/// (a `tools/partdb.yaml` key), unless that is not one.
#[test]
fn the_tables_part_stands_in_for_a_missing_manifest() {
    let (b, _, _) = payloads("A");
    let mut d = factory();
    let mut with = info(&mut d);
    let p = bundle::plan(&b, &with, None, &known());
    assert_eq!(
        (p.part.as_deref(), p.part_source.as_deref()),
        (Some("512x8"), Some("manifest"))
    );
    with.fabric = None;
    let p = bundle::plan(&b, &with, None, &known());
    assert_eq!(
        (p.part.as_deref(), p.part_source.as_deref()),
        (Some("512x8"), Some("table"))
    );
    assert!(p.fabric.available, "{p:?}");
    // A part the table records that is not a key: ask the user.
    let mut odd = with.clone();
    odd.table.as_mut().unwrap().part = "82S147".into();
    let p = bundle::plan(&b, &odd, None, &known());
    assert_eq!(p.part, None);
    assert!(!p.fabric.available);
    // Neither: ask. The user's choice wins over both.
    let mut neither = with.clone();
    neither.table = None;
    assert_eq!(bundle::plan(&b, &neither, None, &known()).part, None);
    let p = bundle::plan(&b, &neither, Some("512x8"), &known());
    assert_eq!(p.part_source.as_deref(), Some("chosen"));
}

#[test]
fn power_cut_during_fabric_update_never_commits_a_partial_bitstream() {
    let (_, _, f) = payloads("A");
    for cut in [1, 3, 40, 3000, 7000, 8000] {
        let mut d = factory();
        block_on(update::enter_dfu(&mut Client::new(SimTransport(&mut d)))).unwrap();
        d.complete_reboot();
        let dfu = info(&mut d);
        d.cut_power_after(cut);
        let r = block_on(update::update_fabric(
            &mut Client::new(SimTransport(&mut d)),
            &dfu,
            &f,
            &mut noop(),
        ));
        d.power_on();
        let i = info(&mut d);
        match (&r, &i.fabric) {
            (Ok(()), Some(m)) => assert!(m.committed && m.crc == f.manifest.fabric_crc),
            (Err(e), m) => {
                assert!(e.is_gone(), "cut {cut}: {e}");
                assert!(m.as_ref().is_none_or(|m| !m.committed), "cut {cut}: {m:?}");
            }
            (Ok(()), None) => panic!("cut {cut}: committed and no manifest"),
        }
        // The application is untouched, but over an uncommitted fabric the
        // DFU stage does not boot it (it would have no USB): DFU mode, on
        // USB, where the page finishes the update.
        let committed = i.fabric.as_ref().is_some_and(|m| m.committed);
        assert_eq!(i.mode == Mode::Application, committed, "cut {cut}");
        if r.is_err() {
            assert_eq!(i.mode, Mode::Dfu, "cut {cut}: DFU mode over USB");
            assert!(d.connected());
            let mut c = Client::new(SimTransport(&mut d));
            block_on(update::update_fabric(&mut c, &i, &f, &mut noop())).expect("finish");
            block_on(update::reboot_application(&mut c)).unwrap();
            d.complete_reboot();
            assert_eq!(info(&mut d).mode, Mode::Application, "cut {cut}: after it");
        }
    }
}

#[test]
fn bundle_files_are_checked() {
    let (b, app, fabric, page) = bundle("A");
    let fw = b.firmware.as_ref().unwrap();
    let mut bad = app.clone();
    bad[300] ^= 1;
    assert!(bundle::check_firmware(fw, &bad)
        .unwrap_err()
        .contains("CRC"));
    assert!(bundle::check_firmware(fw, &app[..app.len() - 1]).is_err());
    let mut wrong = fw.clone();
    wrong.version = "9.9.9".into();
    assert!(bundle::check_firmware(&wrong, &app).is_err());
    let mut bad = fabric.clone();
    bad[0] ^= 1;
    assert!(bundle::check_fabric(&b.fabrics[0], &bad, &page).is_err());
    assert!(bundle::check_fabric(&b.fabrics[0], &fabric, &page[..255]).is_err());
    let mut other = b.fabrics[0].clone();
    other.part = "256x8".into();
    assert!(bundle::check_fabric(&other, &fabric, &page).is_err());

    assert!(bundle::parse(r#"{"format": 2, "board_rev": "A", "firmware": null}"#).is_err());
    assert!(bundle::parse("{").is_err());
    let none = bundle::parse(r#"{"format": 1, "board_rev": "A", "firmware": null}"#).unwrap();
    let mut d = factory();
    let p = bundle::plan(&none, &info(&mut d), None, &known());
    assert!(!p.firmware.available && !p.fabric.available);
}
