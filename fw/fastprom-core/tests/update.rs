//! The update core end to end: a host drives `update::Server` with the
//! protocol's own frames (`update::proto`), packet by packet as USB carries
//! them, against a model of the AG32's 256 KiB of NOR flash (sector erase,
//! page program that only clears bits) that can lose power after any
//! operation. The second half is the protocol itself: framing, the
//! refusals of damaged, foreign and malformed transfers, and recovery.

mod common;

use std::cell::{Cell, RefCell};

use fastprom_core::crc32::crc32;
use fastprom_core::header::{
    boot_application, boot_sane, AppHeader, BoardRev, Commit, Manifest, Part, Version, APP_MAGIC,
    DFU_REQUEST, MANIFEST_MAGIC, PAGE_LEN,
};
use fastprom_core::memmap::{
    APP_BASE, APP_ENTRY, APP_LEN, DFU_BASE, FABRIC_BASE, FABRIC_LEN, FLASH_BASE, FLASH_SIZE,
    MANIFEST_BASE, OPTION_BASE, PAGE, SECTOR, TABLE_BASE, TABLE_LEN,
};
use fastprom_core::regmap::FABRIC_ID;
use fastprom_core::update::info::{
    FLAG_APP_COMMITTED, FLAG_MANIFEST_COMMITTED, FLAG_TABLE_INTACT, FLAG_TABLE_VALID, INFO_LEN,
    INFO_VERSION, OFF_APP, OFF_BANK, OFF_FLAGS, OFF_INTACT, OFF_MANIFEST, OFF_RESERVED, OFF_TABLE,
};
use fastprom_core::update::proto::{
    self, Command, FrameError, Header, HEADER_LEN, IN_TRANSFER, MAGIC, PACKET, READ_MAX,
    REQUEST_MAX, TARGET_APPLICATION, TARGET_DFU, TARGET_FABRIC, VERSION,
};
use fastprom_core::update::{
    BootTarget, Flash, FlashError, Identity, Mode, Platform, Region, ReloadError, Server, Status,
};
use fastprom_core::{Bank, ImageTable, Released, TableSummary};

use common::{build_table, pattern, MockFabric};

const VERSION_FW: Version = Version {
    major: 0,
    minor: 1,
    patch: 0,
};

fn rev(s: &str) -> BoardRev {
    BoardRev::new(s).expect("board revision")
}

// ---- the flash ----------------------------------------------------------------

/// 256 KiB of NOR flash at `FLASH_BASE`. Erase sets a sector to ones;
/// program ANDs. With a budget, power is lost at that operation: an erase
/// leaves the sector half erased, a page program leaves its word half
/// programmed, and nothing after it happens.
struct Board {
    flash: RefCell<Vec<u8>>,
    /// Flash operations (sector erases, word programs) left before power is
    /// cut; `None` for no cut.
    budget: Cell<Option<usize>>,
    /// Operations done, for sizing the cut sweep.
    ops: Cell<usize>,
    reboots: RefCell<Vec<BootTarget>>,
    bank: Option<Bank>,
    /// The block RAM, for LOAD.
    fabric: RefCell<MockFabric>,
    loads: Cell<usize>,
}

impl Board {
    fn new() -> Self {
        Self {
            flash: RefCell::new(vec![0xFF; FLASH_SIZE as usize]),
            budget: Cell::new(None),
            ops: Cell::new(0),
            reboots: RefCell::new(Vec::new()),
            bank: Bank::new(1),
            fabric: RefCell::new(MockFabric::new(9, 8)),
            loads: Cell::new(0),
        }
    }

    /// Put bytes in flash as the UART loader would (no policy).
    fn load(&self, addr: u32, data: &[u8]) {
        let at = (addr - FLASH_BASE) as usize;
        self.flash.borrow_mut()[at..at + data.len()].copy_from_slice(data);
    }

    fn bytes(&self, addr: u32, len: u32) -> Vec<u8> {
        let at = (addr - FLASH_BASE) as usize;
        self.flash.borrow()[at..at + len as usize].to_vec()
    }

    fn word(&self, addr: u32) -> u32 {
        u32::from_le_bytes(self.bytes(addr, 4).try_into().unwrap())
    }

    /// Take one operation from the budget: `Ok(true)` to do it whole,
    /// `Ok(false)` to do it half (power fails during it), `Err` if power
    /// is already gone.
    fn spend(&self) -> Result<bool, FlashError> {
        self.ops.set(self.ops.get() + 1);
        match self.budget.get() {
            None => Ok(true),
            Some(0) => Err(FlashError::Failed),
            Some(1) => {
                self.budget.set(Some(0));
                Ok(false)
            }
            Some(n) => {
                self.budget.set(Some(n - 1));
                Ok(true)
            }
        }
    }
}

impl Flash for Board {
    fn erase_sector(&self, addr: u32) -> Result<(), FlashError> {
        assert_eq!(addr % SECTOR, 0, "unaligned erase");
        let whole = self.spend()?;
        let at = (addr - FLASH_BASE) as usize;
        let mut f = self.flash.borrow_mut();
        for (i, b) in f[at..at + SECTOR as usize].iter_mut().enumerate() {
            // Half erased: every other byte.
            if whole || i % 2 == 0 {
                *b = 0xFF;
            }
        }
        if whole {
            Ok(())
        } else {
            Err(FlashError::Failed)
        }
    }

    fn program_page(&self, addr: u32, page: &[u8; PAGE_LEN]) -> Result<(), FlashError> {
        assert_eq!(addr % PAGE, 0, "unaligned program");
        let at = (addr - FLASH_BASE) as usize;
        for (w, chunk) in page.chunks(4).enumerate() {
            if chunk == [0xFF; 4] {
                continue; // programs nothing
            }
            let whole = self.spend()?;
            let mut f = self.flash.borrow_mut();
            for (i, &b) in chunk.iter().enumerate() {
                // Half programmed: the low half of each byte's bits.
                let b = if whole { b } else { b | 0xF0 };
                f[at + w * 4 + i] &= b;
            }
            if !whole {
                return Err(FlashError::Failed);
            }
        }
        Ok(())
    }

    fn read(&self, addr: u32, buf: &mut [u8]) {
        let at = (addr - FLASH_BASE) as usize;
        buf.copy_from_slice(&self.flash.borrow()[at..at + buf.len()]);
    }
}

impl Platform for Board {
    fn reload(&self) -> Result<(), ReloadError> {
        self.loads.set(self.loads.get() + 1);
        let flash = self.flash.borrow();
        let at = (TABLE_BASE - FLASH_BASE) as usize;
        let table = ImageTable::parse(&flash[at..]).map_err(|_| Status::InvalidData)?;
        let image = table
            .image(self.bank.ok_or(Status::PreconditionNotMet)?)
            .ok_or(Status::InvalidData)?;
        let mut fabric = self.fabric.borrow_mut();
        let released = Released::claim(&mut *fabric).map_err(|_| Status::PreconditionNotMet)?;
        released.load(&image).map_err(|_| Status::InvalidData)?;
        Ok(())
    }

    fn reboot(&self, to: BootTarget) {
        self.reboots.borrow_mut().push(to);
    }

    fn switch_bank(&self) -> Option<Bank> {
        self.bank
    }

    fn table_summary(&self) -> Option<TableSummary> {
        let flash = self.flash.borrow();
        let at = (TABLE_BASE - FLASH_BASE) as usize;
        ImageTable::parse(&flash[at..]).ok().map(|t| t.summary())
    }
}

// ---- the host -------------------------------------------------------------------

/// The packets of one transfer of `bytes`: 64 bytes each, the last short,
/// a zero-length one after a transfer that fills its last packet.
fn packets(bytes: &[u8]) -> Vec<&[u8]> {
    let mut p: Vec<&[u8]> = bytes.chunks(PACKET).collect();
    if bytes.len().is_multiple_of(PACKET) {
        p.push(&[]);
    }
    p
}

/// One device under test and the host driving it.
struct Host<'a> {
    s: Server<'a, Board>,
    seq: u16,
    /// The packet lengths of the last response.
    last_packets: Vec<usize>,
}

impl<'a> Host<'a> {
    fn new(board: &'a Board, mode: Mode) -> Self {
        let id = Identity {
            mode,
            fw_version: VERSION_FW,
            board_rev: rev("A"),
        };
        Self {
            s: Server::new(board, id),
            seq: 0,
            last_packets: Vec::new(),
        }
    }

    /// Hand the device these packets, as the class does.
    fn send_packets(&mut self, packets: &[&[u8]]) {
        for p in packets {
            assert!(self.s.ready(), "a packet while a response is pending");
            self.s.receive(p);
        }
    }

    /// Take the response the device has, a packet at a time, as a host's
    /// IN transfer of `IN_TRANSFER` bytes does: up to the short packet.
    fn take_response(&mut self) -> Vec<u8> {
        let mut out = Vec::new();
        self.last_packets.clear();
        loop {
            let mut packet = [0u8; PACKET];
            let Some(n) = self.s.next_packet(&mut packet) else {
                panic!("the response ended without a short packet");
            };
            assert_eq!(
                self.s.next_packet(&mut [0; PACKET]),
                Some(n),
                "a peek changes nothing"
            );
            self.s.packet_armed();
            assert_eq!(
                self.s.next_packet(&mut [0; PACKET]),
                None,
                "one packet armed at a time"
            );
            out.extend_from_slice(&packet[..n]);
            self.last_packets.push(n);
            self.s.packet_sent();
            if n < PACKET {
                break;
            }
            assert!(out.len() < IN_TRANSFER);
        }
        assert!(self.s.ready(), "one response per transfer");
        out
    }

    /// One transfer out, its one response back.
    fn transfer(&mut self, bytes: &[u8]) -> Vec<u8> {
        self.send_packets(&packets(bytes));
        self.take_response()
    }

    fn frame(&mut self, cmd: u8, payload: &[u8]) -> Vec<u8> {
        self.seq = self.seq.wrapping_add(1);
        let mut f = vec![0u8; HEADER_LEN + payload.len()];
        proto::encode(&mut f, cmd, 0, self.seq, payload).expect("frame");
        f
    }

    /// A request and its response, checked as a host checks it.
    fn request(&mut self, cmd: u8, payload: &[u8]) -> (Status, Vec<u8>) {
        let f = self.frame(cmd, payload);
        let r = self.transfer(&f);
        let (h, p) = proto::decode(&r).expect("a response frame");
        assert_eq!(h.version, VERSION);
        assert_eq!(h.seq, self.seq, "the sequence number is echoed");
        assert_eq!(h.command, cmd, "the command is echoed");
        let status = Status::from_u8(h.status).expect("a known status");
        if status != Status::Ok {
            assert!(p.is_empty(), "a refusal carries no payload");
        }
        (status, p.to_vec())
    }

    /// ERASE each sector of `addr..addr + len`, to the first refusal.
    fn erase(&mut self, addr: u32, len: u32) -> Status {
        let mut at = addr;
        while at < addr + len {
            let s = self.request(Command::Erase as u8, &at.to_le_bytes()).0;
            if s != Status::Ok {
                return s;
            }
            at += SECTOR;
        }
        Status::Ok
    }

    /// WRITE `data` a page at a time, to the first refusal (a short last
    /// page is a malformed request).
    fn write(&mut self, addr: u32, data: &[u8]) -> Status {
        for (i, page) in data.chunks(PAGE as usize).enumerate() {
            let mut p = (addr + i as u32 * PAGE).to_le_bytes().to_vec();
            p.extend_from_slice(page);
            let s = self.request(Command::Write as u8, &p).0;
            if s != Status::Ok {
                return s;
            }
        }
        Status::Ok
    }

    /// READ, `READ_MAX` at a time.
    fn read(&mut self, addr: u32, size: u32) -> (Status, Vec<u8>) {
        let mut out = Vec::new();
        let mut done = 0;
        while done < size {
            let n = (size - done).min(READ_MAX as u32);
            let mut p = (addr + done).to_le_bytes().to_vec();
            p.extend_from_slice(&n.to_le_bytes());
            let (s, data) = self.request(Command::Read as u8, &p);
            if s != Status::Ok {
                return (s, out);
            }
            assert_eq!(data.len(), n as usize);
            out.extend_from_slice(&data);
            done += n;
        }
        (Status::Ok, out)
    }

    fn reboot(&mut self, target: u8) -> Status {
        self.request(Command::Reboot as u8, &[target]).0
    }

    fn info(&mut self) -> (Status, Vec<u8>) {
        self.request(Command::Info as u8, &[])
    }

    fn load(&mut self) -> Status {
        self.request(Command::Load as u8, &[]).0
    }

    fn commit(&mut self, target: u8) -> Status {
        self.request(Command::Commit as u8, &[target]).0
    }
}

// ---- payloads -------------------------------------------------------------------

/// An application file as `make fw-build` writes one: the stamped header
/// (committed, as the UART loader programs it), then the code, padded with
/// erased bytes to whole pages for USB.
fn app_file(seed: u32, code_len: usize, board_rev: &str) -> (Vec<u8>, Vec<u8>) {
    let code = pattern(code_len, seed, 0xFF);
    let header = AppHeader {
        image_len: code.len() as u32,
        image_crc: crc32(&code),
        fw_version: Version {
            major: 0,
            minor: 2,
            patch: seed as u16,
        },
        board_rev: rev(board_rev),
        fabric_id: FABRIC_ID,
    };
    let mut file = header.encode(Commit::Committed).to_vec();
    file.extend_from_slice(&code);
    pad(&mut file);
    (file, code)
}

/// A bitstream and its manifest page (committed, as the UART loader would).
fn fabric_files(seed: u32, len: usize) -> (Vec<u8>, Vec<u8>) {
    let fabric = pattern(len, seed, 0xFF);
    let m = Manifest {
        fabric_len: fabric.len() as u32,
        fabric_crc: crc32(&fabric),
        part: Part::new("512x8").unwrap(),
        board_rev: rev("A"),
        fabric_id: FABRIC_ID,
    };
    let mut padded = fabric.clone();
    pad(&mut padded);
    (padded, m.encode(Commit::Committed).to_vec())
}

fn pad(v: &mut Vec<u8>) {
    v.resize(v.len().div_ceil(PAGE as usize) * PAGE as usize, 0xFF);
}

/// A board as provisioned over the UART: a committed application, fabric
/// and manifest, and an image table.
fn provisioned() -> Board {
    let b = Board::new();
    let (app, _) = app_file(1, 5000, "A");
    b.load(APP_BASE, &app);
    let (fabric, manifest) = fabric_files(2, 3000);
    b.load(FABRIC_BASE, &fabric);
    b.load(MANIFEST_BASE, &manifest);
    b.load(TABLE_BASE, &table(3));
    b
}

fn table(seed: u32) -> Vec<u8> {
    let a = pattern(512, seed, 0xFF);
    let c = pattern(512, seed + 1, 0xFF);
    build_table("512x8", 512, 8, &[("a.bin", &a), ("c.bin", &c)])
}

/// What the DFU stage's power-up check would do now
/// (`header::boot_application`), and whether that is safe: a sane header
/// must describe the payload under it exactly, and a sane manifest the
/// bitstream under it.
fn boots_application(b: &Board) -> bool {
    let three = |at: u32| [b.word(at), b.word(at + 4), b.word(at + 8)];
    let sane = boot_application(three(APP_BASE), three(MANIFEST_BASE));
    assert_eq!(
        sane,
        boot_sane(b.word(APP_BASE), b.word(APP_BASE + 4), b.word(APP_BASE + 8))
            && fabric_committed(b),
        "the boot check is the header's and the manifest's"
    );
    if sane {
        let page: [u8; PAGE_LEN] = b.bytes(APP_BASE, PAGE).try_into().unwrap();
        let (commit, h) = AppHeader::decode(&page).expect("a sane header decodes");
        assert_eq!(commit, Commit::Committed);
        assert_eq!(
            crc32(&b.bytes(APP_ENTRY, h.image_len)),
            h.image_crc,
            "a bootable header over a payload it does not describe"
        );
    }
    sane
}

/// Whether the manifest is committed; if it is, it must describe the
/// bitstream under it exactly.
fn fabric_committed(b: &Board) -> bool {
    let page: [u8; PAGE_LEN] = b.bytes(MANIFEST_BASE, PAGE).try_into().unwrap();
    match Manifest::decode(&page) {
        Ok((Commit::Committed, m)) => {
            assert_eq!(
                crc32(&b.bytes(FABRIC_BASE, m.fabric_len)),
                m.fabric_crc,
                "a committed manifest over a bitstream it does not describe"
            );
            true
        }
        _ => {
            assert_ne!(
                b.word(MANIFEST_BASE),
                MANIFEST_MAGIC,
                "magic over a bad page"
            );
            false
        }
    }
}

// ---- the tests ------------------------------------------------------------------

#[test]
fn reads_anywhere_in_flash_only() {
    let b = provisioned();
    let mut h = Host::new(&b, Mode::Application);
    let (s, data) = h.read(APP_BASE, 300);
    assert_eq!(s, Status::Ok);
    assert_eq!(data, b.bytes(APP_BASE, 300));
    assert_eq!(
        h.read(DFU_BASE, 64).0,
        Status::Ok,
        "the DFU stage is readable"
    );
    assert_eq!(
        h.read(FLASH_BASE + FLASH_SIZE - 4, 4).0,
        Status::Ok,
        "the last word"
    );
    assert_eq!(
        h.read(FLASH_BASE + FLASH_SIZE - 4, 8).0,
        Status::InvalidAddress
    );
    assert_eq!(h.read(OPTION_BASE, 16).0, Status::InvalidAddress);
    assert_eq!(h.read(0x2000_0000, 16).0, Status::InvalidAddress);
}

/// Every region against every mode, for erase and write.
#[test]
fn range_policy_per_mode() {
    let cases = [
        (DFU_BASE, Status::NotPermitted, Status::NotPermitted),
        (APP_BASE, Status::NotPermitted, Status::Ok),
        (FABRIC_BASE, Status::NotPermitted, Status::Ok),
        (MANIFEST_BASE, Status::NotPermitted, Status::Ok),
        (TABLE_BASE, Status::Ok, Status::Ok),
    ];
    for (addr, app, dfu) in cases {
        for (mode, want) in [(Mode::Application, app), (Mode::Dfu, dfu)] {
            // On blank flash, so that no commit word is in the way.
            let b = Board::new();
            let mut h = Host::new(&b, mode);
            assert_eq!(h.erase(addr, SECTOR), want, "erase {addr:#x} in {mode:?}");
            assert_eq!(
                h.write(addr, &[0u8; 256]),
                want,
                "write {addr:#x} in {mode:?}"
            );
            if want != Status::Ok {
                assert!(
                    b.bytes(addr, SECTOR).iter().all(|&x| x == 0xFF),
                    "a refused command changed flash at {addr:#x}"
                );
            }
        }
    }
    for mode in [Mode::Application, Mode::Dfu] {
        let b = Board::new();
        let mut h = Host::new(&b, mode);
        assert_eq!(h.erase(OPTION_BASE, SECTOR), Status::NotPermitted);
        assert_eq!(h.write(OPTION_BASE, &[0; 256]), Status::NotPermitted);
        assert_eq!(h.erase(0x9000_0000, SECTOR), Status::InvalidAddress);
        assert_eq!(
            h.write(FLASH_BASE + FLASH_SIZE, &[0; 256]),
            Status::InvalidAddress
        );
        assert_eq!(h.erase(TABLE_BASE + 0x100, SECTOR), Status::BadAlignment);
        assert_eq!(h.write(TABLE_BASE + 4, &[0; 256]), Status::BadAlignment);
        // A page is the unit of WRITE, a sector of ERASE: anything else is
        // a malformed request.
        assert_eq!(h.write(TABLE_BASE, &[0; 100]), Status::BadRequest);
        let p = TABLE_BASE.to_le_bytes();
        assert_eq!(
            h.request(Command::Erase as u8, &[&p[..], &[0]].concat()).0,
            Status::BadRequest
        );
        assert_eq!(
            h.request(Command::Erase as u8, &p[..3]).0,
            Status::BadRequest
        );
        // An address that wraps.
        assert_eq!(
            h.request(Command::Erase as u8, &(u32::MAX - 0xFFF).to_le_bytes())
                .0,
            Status::InvalidAddress
        );
        assert!(b.bytes(FLASH_BASE, FLASH_SIZE).iter().all(|&x| x == 0xFF));
    }
    // No request names a range across a boundary (one page, one sector),
    // but the policy refuses one all the same: the manifest and the table,
    // and the DFU stage from the region beside it.
    assert_eq!(
        Region::of(MANIFEST_BASE, 2 * SECTOR),
        Err(Status::BadRequest)
    );
    assert_eq!(
        Region::of(APP_BASE - SECTOR, 2 * SECTOR),
        Err(Status::BadRequest)
    );
    assert_eq!(Region::of(TABLE_BASE, 0), Err(Status::BadRequest));
    assert_eq!(Region::of(TABLE_BASE, SECTOR), Ok(Region::Table));
}

#[test]
fn image_table_update_then_load() {
    let b = provisioned();
    let mut h = Host::new(&b, Mode::Application);

    let new = table(40);
    let len = new.len().div_ceil(SECTOR as usize) as u32 * SECTOR;
    assert_eq!(h.erase(TABLE_BASE, len), Status::Ok);
    let mut padded = new.clone();
    pad(&mut padded);
    assert_eq!(h.write(TABLE_BASE, &padded), Status::Ok);
    let (s, back) = h.read(TABLE_BASE, new.len() as u32);
    assert_eq!(s, Status::Ok);
    assert_eq!(back, new, "read back");

    assert_eq!(h.load(), Status::Ok);
    assert_eq!(b.loads.get(), 1);
    // The switch reads bank 1: the second image is in block RAM.
    assert_eq!(b.fabric.borrow_mut().contents(), pattern(512, 41, 0xFF));
    assert!(b.fabric.borrow_mut().ready_now());

    // The DFU stage has no LOAD, and the application no COMMIT.
    assert_eq!(h.commit(TARGET_APPLICATION), Status::NotPermitted);
    let mut d = Host::new(&b, Mode::Dfu);
    assert_eq!(d.load(), Status::NotPermitted);
    assert_eq!(b.loads.get(), 1);
}

#[test]
fn load_of_a_damaged_table_leaves_the_socket_released() {
    let b = provisioned();
    let mut h = Host::new(&b, Mode::Application);
    assert_eq!(h.erase(TABLE_BASE, SECTOR), Status::Ok);
    assert_eq!(h.load(), Status::InvalidData);
    assert!(!b.fabric.borrow_mut().ready_now());
}

/// The sequence the page runs for a firmware update in DFU mode.
fn firmware_update(h: &mut Host<'_>, file: &[u8]) -> Result<(), Status> {
    let ok = |s: Status| if s == Status::Ok { Ok(()) } else { Err(s) };
    ok(h.erase(APP_BASE, APP_LEN))?;
    ok(h.write(APP_BASE, file))?;
    let (s, back) = h.read(APP_BASE + 4, file.len() as u32 - 4);
    ok(s)?;
    assert_eq!(back, file[4..], "read back");
    ok(h.commit(TARGET_APPLICATION))
}

/// And for the fabric: the manifest is erased first and written last.
fn fabric_update(h: &mut Host<'_>, fabric: &[u8], manifest: &[u8]) -> Result<(), Status> {
    let ok = |s: Status| if s == Status::Ok { Ok(()) } else { Err(s) };
    ok(h.erase(MANIFEST_BASE, SECTOR))?;
    ok(h.erase(FABRIC_BASE, FABRIC_LEN))?;
    ok(h.write(FABRIC_BASE, fabric))?;
    ok(h.write(MANIFEST_BASE, manifest))?;
    ok(h.commit(TARGET_FABRIC))
}

#[test]
fn firmware_update_and_commit() {
    let b = provisioned();
    assert!(boots_application(&b));
    let (file, code) = app_file(7, 9000, "A");
    let mut h = Host::new(&b, Mode::Dfu);

    assert_eq!(h.erase(APP_BASE, APP_LEN), Status::Ok);
    assert!(!boots_application(&b), "erased: DFU mode");
    assert_eq!(h.write(APP_BASE, &file), Status::Ok);
    assert_eq!(b.word(APP_BASE), u32::MAX, "the commit word is held back");
    assert_eq!(b.bytes(APP_BASE + 4, file.len() as u32 - 4), file[4..]);
    assert!(!boots_application(&b), "written, not committed: DFU mode");

    // INFO sees the header, not committed.
    let (s, info) = h.info();
    assert_eq!(s, Status::Ok);
    assert_eq!(info.len(), INFO_LEN);
    assert_eq!(info[OFF_FLAGS] & FLAG_APP_COMMITTED, 0);
    assert_eq!(&info[OFF_APP..OFF_APP + 8], &[0, 0, 2, 0, 7, 0, 0, 0]);

    assert_eq!(h.commit(TARGET_APPLICATION), Status::Ok);
    assert_eq!(b.word(APP_BASE), APP_MAGIC);
    assert!(boots_application(&b));
    assert_eq!(b.bytes(APP_ENTRY, code.len() as u32), code);
    let (_, info) = h.info();
    assert_ne!(info[OFF_FLAGS] & FLAG_APP_COMMITTED, 0);
    // Committing again changes nothing.
    assert_eq!(h.commit(TARGET_APPLICATION), Status::Ok);

    // A committed header guards its payload: no write, and no erase that
    // does not take the header with it.
    assert_eq!(
        h.write(APP_BASE + 0x1000, &[0; 256]),
        Status::PreconditionNotMet
    );
    assert_eq!(
        h.erase(APP_BASE + SECTOR, SECTOR),
        Status::PreconditionNotMet
    );
    assert!(boots_application(&b));

    // Back to the application.
    assert_eq!(h.reboot(TARGET_APPLICATION), Status::Ok);
    assert_eq!(*b.reboots.borrow(), [BootTarget::Application]);
}

#[test]
fn fabric_update_and_commit() {
    let b = provisioned();
    assert!(fabric_committed(&b));
    let (fabric, manifest) = fabric_files(9, 20_000);
    let mut h = Host::new(&b, Mode::Dfu);
    // A committed manifest guards the bitstream.
    assert_eq!(h.erase(FABRIC_BASE, SECTOR), Status::PreconditionNotMet);
    assert_eq!(fabric_update(&mut h, &fabric, &manifest), Ok(()));
    assert!(fabric_committed(&b));
    assert_eq!(b.bytes(FABRIC_BASE, 20_000), fabric[..20_000]);
    let (_, info) = h.info();
    assert_ne!(info[OFF_FLAGS] & FLAG_MANIFEST_COMMITTED, 0);
    assert_eq!(&info[OFF_MANIFEST..OFF_MANIFEST + 6], b"512x8\0");
    // The application was not touched.
    assert!(boots_application(&b));
}

#[test]
fn commit_refuses_a_payload_its_page_does_not_describe() {
    // The CRC: one byte of the code differs from what the header says.
    let b = provisioned();
    let (mut file, _) = app_file(7, 4000, "A");
    file[PAGE as usize + 100] ^= 0x01;
    let mut h = Host::new(&b, Mode::Dfu);
    assert_eq!(h.erase(APP_BASE, APP_LEN), Status::Ok);
    assert_eq!(h.write(APP_BASE, &file), Status::Ok);
    assert_eq!(h.commit(TARGET_APPLICATION), Status::CrcMismatch);
    assert_eq!(b.word(APP_BASE), u32::MAX);
    assert!(!boots_application(&b));

    // A header built for another board revision.
    let (file, _) = app_file(8, 4000, "B");
    assert_eq!(h.erase(APP_BASE, APP_LEN), Status::Ok);
    assert_eq!(h.write(APP_BASE, &file), Status::Ok);
    assert_eq!(h.commit(TARGET_APPLICATION), Status::NotPermitted);
    assert!(!boots_application(&b));

    // No header at all.
    assert_eq!(h.erase(APP_BASE, APP_LEN), Status::Ok);
    assert_eq!(h.commit(TARGET_APPLICATION), Status::InvalidData);

    // A bitstream that is short of what its manifest says.
    let (fabric, manifest) = fabric_files(9, 8000);
    assert_eq!(
        fabric_update(&mut h, &fabric[..4096], &manifest),
        Err(Status::CrcMismatch)
    );
    assert!(!fabric_committed(&b));

    assert_eq!(h.commit(2), Status::BadRequest);
    assert_eq!(h.request(Command::Commit as u8, &[]).0, Status::BadRequest);
    assert_eq!(
        h.request(Command::Commit as u8, &[0, 0]).0,
        Status::BadRequest
    );
}

/// Cut power at every flash operation of an update (and in the middle of
/// it), then check what the DFU stage would boot: never a header over a
/// payload it does not describe. Then run the update again from where the
/// cut left the flash, as the page would after the next power-up, and it
/// must succeed.
#[test]
fn interrupted_firmware_update_never_boots_a_partial_image() {
    let (file, code) = app_file(11, 1500, "A");
    let total = {
        let b = provisioned();
        let mut h = Host::new(&b, Mode::Dfu);
        b.ops.set(0);
        firmware_update(&mut h, &file).expect("uninterrupted update");
        b.ops.get()
    };
    assert!(total > 400, "{total} flash operations");

    let mut booted_old = 0;
    for cut in 0..=total {
        let b = provisioned();
        let old = b.bytes(APP_BASE, APP_LEN);
        b.budget.set(Some(cut));
        {
            let mut h = Host::new(&b, Mode::Dfu);
            let r = firmware_update(&mut h, &file);
            assert!(r.is_err(), "cut {cut}: {r:?}");
        }
        if boots_application(&b) {
            // Only the old image, before anything was invalidated, or the
            // new one, once committed.
            let now = b.bytes(APP_BASE, APP_LEN);
            if now == old {
                booted_old += 1;
            } else {
                assert_eq!(b.bytes(APP_ENTRY, code.len() as u32), code, "cut {cut}");
            }
        }

        // Power back; the page starts over.
        b.budget.set(None);
        let mut h = Host::new(&b, Mode::Dfu);
        assert_eq!(
            firmware_update(&mut h, &file),
            Ok(()),
            "retry after cut {cut}"
        );
        assert!(boots_application(&b));
        assert_eq!(b.bytes(APP_ENTRY, code.len() as u32), code);
    }
    // The first operation zeroes the old commit word: only a cut before
    // it (budget 0) leaves the old image bootable.
    assert_eq!(booted_old, 1);
}

#[test]
fn interrupted_fabric_update_never_commits_a_partial_bitstream() {
    let (fabric, manifest) = fabric_files(12, 1200);
    let total = {
        let b = provisioned();
        let mut h = Host::new(&b, Mode::Dfu);
        b.ops.set(0);
        fabric_update(&mut h, &fabric, &manifest).expect("uninterrupted update");
        b.ops.get()
    };
    let mut dfu_entered = 0;
    for cut in 1..=total {
        let b = provisioned();
        b.budget.set(Some(cut));
        {
            let mut h = Host::new(&b, Mode::Dfu);
            let _ = fabric_update(&mut h, &fabric, &manifest);
        }
        // The application is untouched, but without a committed manifest
        // the board must not boot it: its fabric would not configure, and
        // it would have no USB. DFU mode, on the stage's own fabric.
        let committed = fabric_committed(&b);
        assert_eq!(
            boots_application(&b),
            committed,
            "cut {cut}: the application only over a committed fabric"
        );
        if !committed {
            dfu_entered += 1;
        }

        b.budget.set(None);
        let mut h = Host::new(&b, Mode::Dfu);
        assert_eq!(
            fabric_update(&mut h, &fabric, &manifest),
            Ok(()),
            "retry after cut {cut}"
        );
        assert!(fabric_committed(&b));
        assert!(boots_application(&b), "after the retry: the application");
    }
    // The manifest's commit word goes first: every cut, the last (during
    // COMMIT's own program, which leaves it half set) too, lands in DFU mode.
    assert_eq!(
        dfu_entered, total,
        "every cut leaves the manifest uncommitted"
    );
}

/// The commit word half programmed (power lost during COMMIT's last
/// operation) is not the magic.
#[test]
fn half_programmed_commit_word_is_not_valid() {
    let (file, _) = app_file(13, 2000, "A");
    let b = provisioned();
    let mut h = Host::new(&b, Mode::Dfu);
    assert_eq!(h.erase(APP_BASE, APP_LEN), Status::Ok);
    assert_eq!(h.write(APP_BASE, &file), Status::Ok);
    b.budget.set(Some(1));
    assert_eq!(h.commit(TARGET_APPLICATION), Status::FlashFailed);
    let w = b.word(APP_BASE);
    assert_ne!(w, APP_MAGIC);
    assert_ne!(w, u32::MAX, "something was programmed");
    assert!(!boots_application(&b));
    // Not erased, not the magic: the header must be erased again.
    b.budget.set(None);
    assert_eq!(h.commit(TARGET_APPLICATION), Status::InvalidData);
    assert_eq!(
        h.write(APP_BASE + SECTOR, &[0; 256]),
        Status::PreconditionNotMet
    );
}

#[test]
fn table_update_interrupted_leaves_a_table_the_firmware_rejects() {
    let new = table(50);
    let mut padded = new.clone();
    pad(&mut padded);
    let len = new.len().div_ceil(SECTOR as usize) as u32 * SECTOR;
    for cut in [1, 2, 5, 40] {
        let b = provisioned();
        b.budget.set(Some(cut));
        {
            let mut h = Host::new(&b, Mode::Application);
            let _ = h.erase(TABLE_BASE, len);
            let _ = h.write(TABLE_BASE, &padded);
        }
        let flash = b.bytes(TABLE_BASE, TABLE_LEN);
        assert!(ImageTable::parse(&flash).is_err(), "cut {cut}");
    }
}

#[test]
fn reboot_maps_to_the_two_boot_targets() {
    let b = provisioned();
    let mut h = Host::new(&b, Mode::Application);
    assert_eq!(h.reboot(TARGET_DFU), Status::Ok, "the DFU stage");
    assert_eq!(h.reboot(TARGET_APPLICATION), Status::Ok, "the application");
    assert_eq!(h.reboot(2), Status::BadRequest);
    assert_eq!(h.reboot(0xFF), Status::BadRequest);
    assert_eq!(h.request(Command::Reboot as u8, &[]).0, Status::BadRequest);
    assert_eq!(
        h.request(Command::Reboot as u8, &[1, 0]).0,
        Status::BadRequest
    );
    assert_eq!(
        *b.reboots.borrow(),
        [BootTarget::Dfu, BootTarget::Application]
    );
    assert_ne!(DFU_REQUEST, 0, "mscratch reads zero after a power-on clear");
}

/// The reboot waits for the host to take the acknowledgement: until then
/// the device is still there to answer.
#[test]
fn reboot_follows_the_last_packet_of_its_response() {
    let b = provisioned();
    let mut h = Host::new(&b, Mode::Dfu);
    let f = h.frame(Command::Reboot as u8, &[TARGET_APPLICATION]);
    h.send_packets(&packets(&f));
    let mut p = [0u8; PACKET];
    assert_eq!(h.s.next_packet(&mut p), Some(HEADER_LEN));
    h.s.packet_armed();
    assert!(b.reboots.borrow().is_empty(), "armed, not taken");
    h.s.packet_sent();
    assert_eq!(*b.reboots.borrow(), [BootTarget::Application]);
    // A reset before the host takes it: no reboot, the host never learned
    // it was accepted.
    let f = h.frame(Command::Reboot as u8, &[TARGET_DFU]);
    h.send_packets(&packets(&f));
    assert_eq!(h.s.next_packet(&mut p), Some(HEADER_LEN));
    h.s.packet_armed();
    h.s.reset();
    assert!(h.s.ready());
    assert_eq!(h.s.next_packet(&mut p), None);
    h.s.packet_sent();
    assert_eq!(*b.reboots.borrow(), [BootTarget::Application]);
}

#[test]
fn info_reports_the_device() {
    let b = provisioned();
    let mut h = Host::new(&b, Mode::Application);
    let (s, info) = h.info();
    assert_eq!(s, Status::Ok);
    assert_eq!(info.len(), INFO_LEN);
    assert_eq!(info[0], INFO_VERSION, "format version");
    assert_eq!(INFO_VERSION, 2);
    assert_eq!(info[1], 0, "application");
    assert_eq!(info[OFF_BANK], 1);
    assert_eq!(
        info[OFF_FLAGS],
        FLAG_APP_COMMITTED | FLAG_MANIFEST_COMMITTED | FLAG_TABLE_VALID | FLAG_TABLE_INTACT
    );
    assert_eq!(info[OFF_INTACT], 0b11, "both images match their CRCs");
    assert!(info[OFF_RESERVED..].iter().all(|&x| x == 0));
    assert_eq!(&info[4..12], &VERSION_FW.to_bytes());
    assert_eq!(&info[12..20], b"A\0\0\0\0\0\0\0");
    assert_eq!(&info[OFF_TABLE..OFF_TABLE + 4], &512u32.to_le_bytes());
    assert_eq!(&info[OFF_TABLE + 4..OFF_TABLE + 6], &[8, 2]);
    let crc0 = crc32(&pattern(512, 3, 0xFF));
    assert_eq!(&info[OFF_TABLE + 8..OFF_TABLE + 12], &crc0.to_le_bytes());
    assert_eq!(&info[OFF_TABLE + 24..OFF_TABLE + 30], b"512x8\0");

    // INFO takes no payload, and unknown commands are refused.
    assert_eq!(h.request(Command::Info as u8, &[0]).0, Status::BadRequest);
    assert_eq!(h.request(Command::Load as u8, &[0]).0, Status::BadRequest);
    for cmd in [0, 8, 0x7F, 0x81, 0xFF] {
        assert_eq!(h.request(cmd, &[]).0, Status::BadRequest, "command {cmd}");
    }

    // DFU mode, blank flash: nothing committed, no table, mode 1.
    let blank = Board::new();
    let mut d = Host::new(&blank, Mode::Dfu);
    let (_, info) = d.info();
    assert_eq!(info[1], 1);
    assert_eq!(info[OFF_FLAGS], 0);
    assert!(info[OFF_APP..].iter().all(|&x| x == 0));
}

/// A table whose header was written whole over images a power cut left half
/// written: the header parses, the images do not match their CRCs, and INFO
/// says both.
#[test]
fn info_tells_a_whole_header_from_intact_images() {
    let b = provisioned();
    // Image 1's words (after the header and image 0), as an interrupted
    // write leaves them: erased from part way.
    let at = TABLE_BASE + 256 + 512 + 100;
    b.load(at, &[0xFF; 300]);
    let summary = Platform::table_summary(&b).expect("the header still parses");
    assert_eq!(summary.intact, 0b01);
    assert!(!summary.all_intact());
    let mut h = Host::new(&b, Mode::Application);
    let (s, info) = h.info();
    assert_eq!(s, Status::Ok);
    assert_ne!(info[OFF_FLAGS] & FLAG_TABLE_VALID, 0, "the header parses");
    assert_eq!(
        info[OFF_FLAGS] & FLAG_TABLE_INTACT,
        0,
        "but not every image is whole"
    );
    assert_eq!(info[OFF_INTACT], 0b01);
}

// ---- the protocol ---------------------------------------------------------------

/// The response to a transfer, decoded: its header and payload.
fn answer(h: &mut Host<'_>, bytes: &[u8]) -> (Header, Vec<u8>) {
    let r = h.transfer(bytes);
    let (hd, p) = proto::decode(&r).expect("every transfer gets a whole frame back");
    assert_eq!(hd.version, VERSION);
    (hd, p.to_vec())
}

fn status_of(hd: &Header) -> Status {
    Status::from_u8(hd.status).expect("a known status")
}

#[test]
fn frames_round_trip_and_every_damaged_byte_is_caught() {
    let payload: Vec<u8> = (0..=255).collect();
    let mut f = vec![0u8; HEADER_LEN + payload.len()];
    let n = proto::encode(&mut f, Command::Write as u8, 0, 0xBEEF, &payload).unwrap();
    assert_eq!(n, f.len());
    assert_eq!(&f[..4], b"FPUP");
    assert_eq!(MAGIC, *b"FPUP");
    assert_eq!(f[4], VERSION);
    assert_eq!(f[5], Command::Write as u8);
    assert_eq!(f[6], 0);
    assert_eq!(f[7], 0);
    assert_eq!(&f[8..10], &0xBEEFu16.to_le_bytes());
    assert_eq!(&f[10..12], &256u16.to_le_bytes());
    let mut covered = f[..12].to_vec();
    covered.extend_from_slice(&payload);
    assert_eq!(
        &f[12..16],
        &crc32(&covered).to_le_bytes(),
        "CRC of 0..12 and the payload"
    );
    let (hd, p) = proto::decode(&f).unwrap();
    assert_eq!((hd.command, hd.seq, hd.len), (3, 0xBEEF, 256));
    assert_eq!(p, payload);
    assert_eq!(Header::parse(&hd.encode()), Some(hd));

    for i in 0..f.len() {
        for bit in [0x01, 0x80] {
            let mut g = f.clone();
            g[i] ^= bit;
            let want = match i {
                0..4 => FrameError::NoHeader,
                10..12 => FrameError::Length,
                _ => FrameError::Crc,
            };
            assert_eq!(
                proto::decode(&g).map(|_| ()),
                Err(want),
                "byte {i} bit {bit:#x}"
            );
        }
    }
    assert_eq!(
        proto::decode(&f[..f.len() - 1]).map(|_| ()),
        Err(FrameError::Length)
    );
    assert_eq!(
        proto::decode(&f[..15]).map(|_| ()),
        Err(FrameError::NoHeader)
    );
    assert_eq!(proto::decode(&[]).map(|_| ()), Err(FrameError::NoHeader));
    // Too small a buffer, or a payload no u16 counts: nothing is written.
    assert_eq!(proto::encode(&mut [0u8; 20], 1, 0, 0, &[0; 5]), None);
    assert_eq!(
        proto::encode(&mut vec![0u8; 70_000], 1, 0, 0, &[0; 65_536]),
        None
    );
    // Every command and status has one code, and back.
    for v in 0..=255u8 {
        if let Some(c) = Command::from_u8(v) {
            assert_eq!(c as u8, v);
        }
        if let Some(s) = Status::from_u8(v) {
            assert_eq!(s as u8, v);
        }
    }
}

#[test]
fn a_transfer_that_is_not_a_request_is_answered_bad_frame() {
    let b = provisioned();
    let mut h = Host::new(&b, Mode::Dfu);
    // No magic, too short, empty: nothing of a header to echo.
    for bytes in [
        &b"XPUP\x01\x01\0\0\x01\0\0\0\0\0\0\0"[..],
        &b"FPUP\x01"[..],
        &[][..],
    ] {
        let (hd, p) = answer(&mut h, bytes);
        assert_eq!(status_of(&hd), Status::BadFrame);
        assert_eq!((hd.command, hd.seq), (0, 0));
        assert!(p.is_empty());
    }
    // Damaged on the way: a WRITE with one byte flipped writes nothing.
    let mut p = TABLE_BASE.to_le_bytes().to_vec();
    p.extend_from_slice(&[0x55; 256]);
    let mut f = h.frame(Command::Write as u8, &p);
    f[100] ^= 0x04;
    let (hd, _) = answer(&mut h, &f);
    assert_eq!(status_of(&hd), Status::BadFrame);
    assert_eq!(
        (hd.command, hd.seq),
        (Command::Write as u8, h.seq),
        "the header is echoed"
    );
    assert!(
        b.bytes(TABLE_BASE, 256) != vec![0x55; 256],
        "nothing written"
    );
    // The length field disagrees with the transfer: shorter, longer.
    let mut f = h.frame(Command::Info as u8, &[]);
    f.push(0);
    let (hd, _) = answer(&mut h, &f);
    assert_eq!(status_of(&hd), Status::BadFrame);
    let f = h.frame(Command::Write as u8, &p);
    let (hd, _) = answer(&mut h, &f[..f.len() - 10]);
    assert_eq!(status_of(&hd), Status::BadFrame);
    // And then everything works as before.
    assert_eq!(h.info().0, Status::Ok);
}

/// A packet lost from the middle of a request: what arrives is one short
/// of the length, the device says so, and the request sent again works.
#[test]
fn a_lost_packet_is_caught_and_the_request_sent_again() {
    let b = provisioned();
    let mut h = Host::new(&b, Mode::Application);
    let mut p = TABLE_BASE.to_le_bytes().to_vec();
    p.extend_from_slice(&[0x00; 256]);
    assert_eq!(h.erase(TABLE_BASE, SECTOR), Status::Ok);
    let f = h.frame(Command::Write as u8, &p);
    let mut ps = packets(&f);
    assert_eq!(ps.len(), 5, "276 bytes: four whole packets and one short");
    ps.remove(2);
    h.send_packets(&ps);
    let r = h.take_response();
    let (hd, _) = proto::decode(&r).unwrap();
    assert_eq!(status_of(&hd), Status::BadFrame);
    assert_eq!(hd.seq, h.seq);
    assert!(b.bytes(TABLE_BASE, 256).iter().all(|&x| x == 0xFF));
    // The host's rule: reset, and send it again under a new number.
    h.s.reset();
    assert_eq!(h.request(Command::Write as u8, &p).0, Status::Ok);
    assert_eq!(b.bytes(TABLE_BASE, 256), vec![0; 256]);
}

#[test]
fn another_protocol_version_is_refused_with_the_devices_own() {
    let b = provisioned();
    let mut h = Host::new(&b, Mode::Application);
    for v in [0, VERSION + 1, 0xFF] {
        let mut hd = Header::new(v, Command::Info as u8, 0, 77, &[]);
        let f = hd.encode();
        let r = h.transfer(&f);
        let (back, p) = proto::decode(&r).unwrap();
        assert_eq!(status_of(&back), Status::Version, "version {v}");
        assert_eq!(back.version, VERSION, "the device says what it speaks");
        assert_eq!((back.command, back.seq), (Command::Info as u8, 77));
        assert!(p.is_empty());
        // A newer host's ERASE does not erase.
        hd = Header::new(v, Command::Erase as u8, 0, 78, &TABLE_BASE.to_le_bytes());
        let mut f = hd.encode().to_vec();
        f.extend_from_slice(&TABLE_BASE.to_le_bytes());
        let (back, _) = answer(&mut h, &f);
        assert_eq!(status_of(&back), Status::Version);
        assert!(ImageTable::parse(&b.bytes(TABLE_BASE, TABLE_LEN)).is_ok());
    }
}

#[test]
fn oversized_and_short_requests_are_refused() {
    let b = provisioned();
    let mut h = Host::new(&b, Mode::Dfu);
    // Longer than any command takes, whole: malformed. 16 packets and a
    // zero-length one, none of it stored.
    let f = h.frame(Command::Write as u8, &[0; 1008]);
    assert_eq!(f.len() % PACKET, 0);
    let (hd, _) = answer(&mut h, &f);
    assert_eq!(status_of(&hd), Status::BadRequest);
    assert_eq!(hd.seq, h.seq);
    // Longer, with a length that does not say so: damaged.
    let mut f = h.frame(Command::Write as u8, &[0; 260]);
    f.extend_from_slice(&[0xA5; 700]);
    let (hd, _) = answer(&mut h, &f);
    assert_eq!(status_of(&hd), Status::BadFrame);
    // The most a request may be, and one past it.
    assert_eq!(REQUEST_MAX, HEADER_LEN + 4 + 256);
    let mut p = TABLE_BASE.to_le_bytes().to_vec();
    p.extend_from_slice(&[0xFF; 257]);
    assert_eq!(h.request(Command::Write as u8, &p).0, Status::BadRequest);
    // Each command's payload, one byte short and one long.
    for (cmd, len) in [
        (Command::Read, 8),
        (Command::Write, 260),
        (Command::Erase, 4),
    ] {
        for n in [len - 1, len + 1] {
            if HEADER_LEN + n <= REQUEST_MAX {
                let s = h.request(cmd as u8, &vec![0; n]).0;
                assert_eq!(s, Status::BadRequest, "{cmd:?} with {n} bytes");
            }
        }
    }
    // READ: nothing, too much, past the end.
    let read = |addr: u32, len: u32| [addr.to_le_bytes(), len.to_le_bytes()].concat();
    assert_eq!(
        h.request(Command::Read as u8, &read(APP_BASE, 0)).0,
        Status::BadRequest
    );
    assert_eq!(
        h.request(Command::Read as u8, &read(APP_BASE, READ_MAX as u32 + 1))
            .0,
        Status::BadRequest
    );
    assert_eq!(
        h.request(Command::Read as u8, &read(u32::MAX - 3, 8)).0,
        Status::InvalidAddress
    );
    assert_eq!(h.info().0, Status::Ok);
}

#[test]
fn a_long_read_spans_many_packets_and_ends_short() {
    let b = provisioned();
    let mut h = Host::new(&b, Mode::Application);
    let (s, data) = h.read(APP_BASE, READ_MAX as u32);
    assert_eq!(s, Status::Ok);
    assert_eq!(data, b.bytes(APP_BASE, READ_MAX as u32));
    // 4112 bytes: 64 whole packets and 16 bytes.
    assert_eq!(h.last_packets.len(), 65);
    assert!(h.last_packets[..64].iter().all(|&n| n == PACKET));
    assert_eq!(h.last_packets[64], 16);
    // A response that fills its last packet is closed by a zero-length one.
    let (s, data) = h.read(TABLE_BASE, 48);
    assert_eq!(s, Status::Ok);
    assert_eq!(data, b.bytes(TABLE_BASE, 48));
    assert_eq!(h.last_packets, [64, 0]);
    let (s, _) = h.read(TABLE_BASE, 112);
    assert_eq!(s, Status::Ok);
    assert_eq!(h.last_packets, [64, 64, 0]);
    // Every length from 1 to 300 reads back what the flash holds.
    for len in 1..300 {
        let (s, data) = h.read(DFU_BASE + len, len);
        assert_eq!(s, Status::Ok);
        assert_eq!(data, b.bytes(DFU_BASE + len, len), "{len}");
    }
}

#[test]
fn sequence_numbers_are_echoed_through_their_wrap() {
    let b = provisioned();
    let mut h = Host::new(&b, Mode::Application);
    h.seq = 0xFFFD;
    for _ in 0..5 {
        // request() checks the echo.
        assert_eq!(h.info().0, Status::Ok);
    }
    assert_eq!(h.seq, 2);
}

/// A reset drops half a request and a response not yet taken; what comes
/// after is a fresh exchange.
#[test]
fn reset_drops_what_is_half_done() {
    let b = provisioned();
    let mut h = Host::new(&b, Mode::Application);
    // Half a WRITE, abandoned.
    let mut p = TABLE_BASE.to_le_bytes().to_vec();
    p.extend_from_slice(&[0; 256]);
    let f = h.frame(Command::Write as u8, &p);
    h.send_packets(&packets(&f)[..2]);
    h.s.reset();
    assert_eq!(h.info().0, Status::Ok, "not merged with the half WRITE");
    // A READ's response, partly taken.
    let read = [TABLE_BASE.to_le_bytes(), 4096u32.to_le_bytes()].concat();
    let f = h.frame(Command::Read as u8, &read);
    h.send_packets(&packets(&f));
    assert!(!h.s.ready());
    // While a response waits, a packet is not taken.
    let mut q = [0u8; PACKET];
    for _ in 0..3 {
        assert!(h.s.next_packet(&mut q).is_some());
        h.s.packet_armed();
        h.s.packet_sent();
    }
    h.s.reset();
    assert!(h.s.ready());
    let (s, info) = h.info();
    assert_eq!(s, Status::Ok);
    assert_eq!(
        info.len(),
        INFO_LEN,
        "the INFO answer, not the rest of the READ"
    );
}

/// A packet that arrives while a response is still to go is dropped, not
/// taken as a request (the class leaves it in the controller until then).
#[test]
fn nothing_is_taken_while_a_response_waits() {
    let b = provisioned();
    let mut h = Host::new(&b, Mode::Application);
    let f = h.frame(Command::Info as u8, &[]);
    h.send_packets(&packets(&f));
    let erase = h.frame(Command::Erase as u8, &TABLE_BASE.to_le_bytes());
    h.s.receive(&erase);
    let r = h.take_response();
    let (hd, p) = proto::decode(&r).unwrap();
    assert_eq!(hd.command, Command::Info as u8);
    assert_eq!(p.len(), INFO_LEN);
    assert!(h.s.ready());
    assert!(
        ImageTable::parse(&b.bytes(TABLE_BASE, TABLE_LEN)).is_ok(),
        "no erase ran"
    );
}

/// Bytes off the wire, at random: every transfer is answered with one
/// frame, nothing panics, and nothing outside the table changes.
#[test]
fn random_transfers_never_panic_and_each_gets_one_answer() {
    let b = provisioned();
    let before = b.bytes(FLASH_BASE, TABLE_BASE - FLASH_BASE);
    let mut h = Host::new(&b, Mode::Application);
    let mut x = 0x1234_5678u32;
    let mut next = move || {
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        x
    };
    for _ in 0..3000 {
        let len = (next() % 400) as usize;
        let mut bytes: Vec<u8> = (0..len).map(|_| next() as u8).collect();
        match next() % 4 {
            // Mostly noise; sometimes a real header over a random payload,
            // sometimes a real frame with one byte flipped.
            0 if len >= HEADER_LEN => bytes[..4].copy_from_slice(&MAGIC),
            1 => {
                let cmd = (next() % 9) as u8;
                let n = len.min(300);
                bytes = h.frame(cmd, &bytes[..n]);
                let i = next() as usize % bytes.len();
                bytes[i] ^= 1 << (next() % 8);
            }
            _ => {}
        }
        let r = h.transfer(&bytes);
        let (hd, _) = proto::decode(&r).expect("a frame");
        assert!(Status::from_u8(hd.status).is_some());
    }
    assert_eq!(b.bytes(FLASH_BASE, TABLE_BASE - FLASH_BASE), before);
    assert_eq!(h.info().0, Status::Ok);
}
