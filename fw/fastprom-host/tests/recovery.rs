//! UART recovery end to end against the simulated board: the mask ROM
//! model, the upload of the recovery loader, `fastprom-core`'s loader
//! server on the simulated flash, and then the board on USB.

use fastprom_core::crc32::crc32;
use fastprom_core::header::{BoardRev, Part, Version};
use fastprom_core::memmap::{
    APP_BASE, DFU_BASE, FABRIC_BASE, MANIFEST_BASE, OPTION_BASE, TABLE_BASE, TABLE_LEN,
};
use fastprom_core::recovery::{LoaderHeader, DEVICE_ID, OPTION_BYTES};
use fastprom_host::an3155::Loader;
use fastprom_host::bundle::{self, Bundle};
use fastprom_host::client::Client;
use fastprom_host::info::Mode;
use fastprom_host::recovery::{self, FullFiles, RecoveryError, RecoveryImage, Report};
use fastprom_host::rom::Geometry;
use fastprom_host::serial::{SerialError, SerialPort};
use fastprom_host::sim::{
    self, block_on, SimConfig, SimDevice, SimSerial, SimTransport, WireFault,
};
use fastprom_host::update;

const V: Version = Version {
    major: 0,
    minor: 3,
    patch: 1,
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
        geometry: Geometry {
            depth: 512,
            width: 8,
        },
    }
}

/// A blank board (never provisioned), BOOT0 bridged, powered.
fn blank() -> SimDevice {
    let mut d = SimDevice::blank(config("A"));
    d.set_boot0(true);
    d.power_on();
    d
}

struct Files {
    bundle: Bundle,
    loader: Vec<u8>,
    dfu: Vec<u8>,
    app: Vec<u8>,
    fabric: Vec<u8>,
    page: Vec<u8>,
}

fn files(board: &str) -> Files {
    let code = sim::pattern(3000, 91);
    let header = LoaderHeader {
        image_len: code.len() as u32,
        image_crc: crc32(&code),
        fw_version: V,
        board_rev: rev(board),
    };
    let mut loader = header.encode().to_vec();
    loader.extend_from_slice(&code);
    let dfu = sim::pattern(9000, 92);
    let app = sim::app_file(V, rev(board), 7000, 93);
    let (fabric, page) = sim::fabric_files(Part::new("512x8").unwrap(), rev(board), 20_000, 94);
    let json = format!(
        r#"{{"format": 1, "board_rev": "{board}",
            "firmware": {{"file": "fw.bin", "version": "0.3.1", "board_rev": "{board}",
                         "fabric_id": 1, "size": {}, "crc32": "{:#010x}",
                         "image_len": 7000, "image_crc32": "{:#010x}"}},
            "fabrics": [{{"part": "512x8", "file": "f.bin", "manifest": "f.manifest.bin",
                          "board_rev": "{board}", "fabric_id": 1, "size": 20000,
                          "crc32": "{:#010x}"}}],
            "recovery": {{
              "loader": {{"file": "l.bin", "size": {}, "crc32": "{:#010x}",
                          "version": "0.3.1", "board_rev": "{board}"}},
              "dfu": {{"file": "d.bin", "size": 9000, "crc32": "{:#010x}",
                       "version": "0.3.1", "board_rev": "{board}"}},
              "option_bytes": {{"file": "o.bin", "size": 128, "crc32": "{:#010x}"}}}},
            "notes": []}}"#,
        app.len(),
        crc32(&app),
        crc32(&app[256..]),
        crc32(&fabric),
        loader.len(),
        crc32(&loader),
        crc32(&dfu),
        crc32(&OPTION_BYTES),
    );
    Files {
        bundle: bundle::parse(&json).expect("manifest"),
        loader,
        dfu,
        app,
        fabric,
        page,
    }
}

fn image(f: &Files, full: bool) -> RecoveryImage {
    let app = full.then(|| FullFiles {
        part: "512x8",
        app: &f.app,
        fabric: &f.fabric,
        page: &f.page,
    });
    RecoveryImage::check(&f.bundle, &f.loader, &f.dfu, &OPTION_BYTES, app).expect("image")
}

fn recover(d: &mut SimDevice, img: &RecoveryImage) -> Result<Report, RecoveryError> {
    block_on(recovery::recover(
        &mut Loader::new(SimSerial(d)),
        img,
        &mut |_, _, _| (),
    ))
}

/// The board as provisioned: everything where it belongs, the table
/// region erased.
fn check_flash(d: &SimDevice, f: &Files, full: bool) {
    let b = d.board();
    assert_eq!(b.bytes(DFU_BASE, f.dfu.len() as u32), f.dfu);
    assert_eq!(b.options(), OPTION_BYTES);
    if full {
        assert_eq!(b.bytes(APP_BASE, f.app.len() as u32), f.app);
        assert_eq!(b.bytes(FABRIC_BASE, f.fabric.len() as u32), f.fabric);
        assert_eq!(b.bytes(MANIFEST_BASE, 256), f.page);
    } else {
        assert!(b.bytes(APP_BASE, 256).iter().all(|&x| x == 0xFF));
    }
    assert!(b.bytes(TABLE_BASE, TABLE_LEN).iter().all(|&x| x == 0xFF));
}

/// Unbridge BOOT0 and power-cycle: then the board is on USB.
fn boot_normally(d: &mut SimDevice) -> fastprom_host::info::DeviceInfo {
    d.set_boot0(false);
    d.power_off();
    d.power_on();
    assert!(d.connected(), "the board enumerates");
    block_on(update::connect(&mut Client::new(SimTransport(d))))
        .expect("connect")
        .info
}

#[test]
fn provisions_a_blank_board_that_then_boots() {
    let f = files("A");
    let img = image(&f, true);
    let mut d = blank();
    assert!(!d.connected(), "in the mask ROM: no USB");
    assert_eq!(d.status().uart, Some("mask ROM"));
    let r = recover(&mut d, &img).expect("recovery");
    assert_eq!(r.found, "the mask ROM");
    assert!(
        r.unprotected,
        "erased option bytes leave read protection on"
    );
    assert!(r.uploaded);
    assert!(r.reset);
    assert_eq!(r.retries, 0);
    assert_eq!(
        r.wrote,
        [
            "option bytes",
            "the DFU stage",
            "the fabric",
            "the fabric manifest",
            "the firmware"
        ]
    );
    // Reset with BOOT0 still bridged: the mask ROM again.
    assert_eq!(d.status().uart, Some("mask ROM"));
    check_flash(&d, &f, true);
    let i = boot_normally(&mut d);
    assert_eq!(i.mode, Mode::Application);
    assert_eq!(i.fw_version, "0.3.1");
    assert_eq!(i.fabric.as_ref().map(|m| m.part.as_str()), Some("512x8"));
    assert!(i.app.as_ref().is_some_and(|a| a.committed));
    assert!(i.table.is_none(), "an empty image table");
}

#[test]
fn without_a_fabric_it_provisions_the_dfu_stage_alone() {
    let f = files("A");
    let img = image(&f, false);
    let mut d = blank();
    let r = recover(&mut d, &img).expect("recovery");
    assert_eq!(r.wrote, ["option bytes", "the DFU stage"]);
    check_flash(&d, &f, false);
    let i = boot_normally(&mut d);
    assert_eq!(
        i.mode,
        Mode::Dfu,
        "no application: DFU mode, on its own fabric"
    );
}

#[test]
fn faults_on_the_wire_are_retried() {
    let f = files("A");
    let img = image(&f, true);
    // A byte lost on its way to the host during the upload through the mask
    // ROM, and a byte corrupted on its way in during the loader's writes.
    for fault in [
        WireFault::Drop { after: 300 },
        WireFault::Corrupt { after: 200 },
        WireFault::Corrupt { after: 30_000 },
        WireFault::Drop { after: 25_000 },
    ] {
        let mut d = blank();
        d.inject_fault(fault);
        let r = recover(&mut d, &img).unwrap_or_else(|e| panic!("{fault:?}: {e}"));
        assert!(r.retries >= 1, "{fault:?}: {r:?}");
        check_flash(&d, &f, true);
    }
}

#[test]
fn a_wrong_device_id_is_refused_before_anything_is_written() {
    let f = files("A");
    let img = image(&f, true);
    let mut d = blank();
    d.set_device_id(0x1234_0001);
    let e = recover(&mut d, &img).unwrap_err();
    assert_eq!(e, RecoveryError::WrongDevice(0x1234_0001));
    assert!(e.to_string().contains("not the AG32VF303"), "{e}");
    assert_eq!(d.board().options(), [0xFF; 128], "nothing written");
    assert_eq!(d.status().flash_ops, 0);
    let _ = DEVICE_ID;
}

#[test]
fn nothing_answers_without_boot0() {
    let f = files("A");
    let img = image(&f, true);
    let mut d = SimDevice::blank(config("A"));
    d.power_on();
    assert!(!d.connected(), "a blank chip runs nothing: no USB either");
    let e = recover(&mut d, &img).unwrap_err();
    assert_eq!(e, RecoveryError::NoAnswer);
    assert!(e.to_string().contains("BOOT0 (pad 4)"), "{e}");
}

#[test]
fn another_board_revisions_flash_is_refused() {
    let f = files("A");
    let img = image(&f, true);
    let mut d = SimDevice::factory(config("B"), V);
    d.set_boot0(true);
    d.power_off();
    d.power_on();
    let before = d.board().bytes(APP_BASE, 256);
    let e = recover(&mut d, &img).unwrap_err();
    assert!(
        matches!(&e, RecoveryError::Check(m) if m.contains("board revision B")),
        "{e}"
    );
    assert_eq!(d.board().bytes(APP_BASE, 256), before, "nothing erased");
}

#[test]
fn a_bricked_board_is_recovered_without_unprotecting() {
    // Option bytes in place (unprotected), the application damaged.
    let f = files("A");
    let img = image(&f, true);
    let mut d = SimDevice::factory(config("A"), V);
    d.provision(APP_BASE + 300, &[0; 64]);
    d.set_boot0(true);
    d.power_off();
    d.power_on();
    let r = recover(&mut d, &img).expect("recovery");
    assert!(!r.unprotected);
    assert!(r.before.iter().any(|s| s.contains("board A")), "{r:?}");
    check_flash(&d, &f, true);
    assert_eq!(boot_normally(&mut d).fw_version, "0.3.1");
}

/// A port that goes away after `writes` writes, as an adapter pulled out.
struct Flaky<'a> {
    inner: SimSerial<'a>,
    writes: u32,
}

impl SerialPort for Flaky<'_> {
    async fn write(&mut self, data: &[u8]) -> Result<(), SerialError> {
        if self.writes == 0 {
            return Err(SerialError::Gone);
        }
        self.writes -= 1;
        self.inner.write(data).await
    }
    async fn read(&mut self, len: usize, ms: u32) -> Result<Vec<u8>, SerialError> {
        self.inner.read(len, ms).await
    }
    async fn discard(&mut self) -> Result<(), SerialError> {
        self.inner.discard().await
    }
    async fn sleep(&mut self, ms: u32) {
        self.inner.sleep(ms).await;
    }
}

#[test]
fn a_second_run_finds_the_loader_running_and_finishes() {
    let f = files("A");
    let img = image(&f, true);
    let mut d = blank();
    let e = block_on(recovery::recover(
        &mut Loader::new(Flaky {
            inner: SimSerial(&mut d),
            writes: 900,
        }),
        &img,
        &mut |_, _, _| (),
    ))
    .unwrap_err();
    assert_eq!(e, RecoveryError::Gone);
    assert!(e.to_string().contains("adapter was unplugged"));
    assert_eq!(d.status().uart, Some("recovery loader"));
    let r = recover(&mut d, &img).expect("the second run");
    assert_eq!(r.found, "FastPROM's recovery loader, already running");
    assert!(!r.uploaded);
    check_flash(&d, &f, true);
}

#[test]
fn a_power_cut_part_way_is_recovered_by_running_again() {
    let f = files("A");
    let img = image(&f, true);
    for cut in [1, 40, 70, 3000] {
        let mut d = blank();
        // Flash operations: the unprotect is not one, so these land in the
        // option bytes, the erase and the writes.
        d.cut_power_after(cut);
        assert!(recover(&mut d, &img).is_err(), "cut {cut}");
        d.power_on();
        recover(&mut d, &img).unwrap_or_else(|e| panic!("cut {cut}: {e}"));
        check_flash(&d, &f, true);
        assert_eq!(boot_normally(&mut d).fw_version, "0.3.1", "cut {cut}");
    }
}

#[test]
fn the_plan_says_what_will_be_written() {
    let f = files("A");
    let full = recovery::plan(&f.bundle, Some("512x8"));
    assert!(full.full && full.refuse.is_none());
    assert_eq!(full.board_rev, "A");
    assert!(full.writes.iter().any(|w| w.contains("firmware 0.3.1")));
    let dfu_only = recovery::plan(&f.bundle, None);
    assert!(!dfu_only.full);
    assert!(dfu_only.outcome.contains("update mode"));
    let mut none = f.bundle.clone();
    none.recovery = None;
    assert!(recovery::plan(&none, Some("512x8")).refuse.is_some());
}

#[test]
fn recovery_files_are_checked() {
    let f = files("A");
    let mut bad = f.loader.clone();
    bad[100] ^= 1;
    assert!(RecoveryImage::check(&f.bundle, &bad, &f.dfu, &OPTION_BYTES, None).is_err());
    assert!(RecoveryImage::check(&f.bundle, &f.loader, &f.dfu[1..], &OPTION_BYTES, None).is_err());
    let mut opt = OPTION_BYTES;
    opt[0x31] = 0;
    assert!(RecoveryImage::check(&f.bundle, &f.loader, &f.dfu, &opt, None).is_err());
    // A bundle whose recovery files are for another revision.
    let json = serde_json::to_string(&f.bundle).unwrap().replacen(
        r#""board_rev":"A"}"#,
        r#""board_rev":"B"}"#,
        1,
    );
    assert!(bundle::parse(&json).is_err(), "{json}");
    let _ = OPTION_BASE;
}
