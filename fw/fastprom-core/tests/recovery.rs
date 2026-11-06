//! The recovery loader's command server against a model of the flash and
//! option bytes: AN3155's framing byte by byte, every refusal, and the
//! loader header and option bytes against what the tools write.

use std::cell::{Cell, RefCell};

use fastprom_core::crc32::crc32;
use fastprom_core::header::{BoardRev, Version};
use fastprom_core::memmap::{
    FABRIC_BASE, FLASH_BASE, FLASH_SIZE, OPTION_BASE, OPTION_LEN, RECOVERY_LEN, SECTOR,
};
use fastprom_core::recovery::server::{Server, Target, COMMANDS};
use fastprom_core::recovery::{
    address_frame, fabric_address, xor, LoaderError, LoaderHeader, ACK, CMD_ERASE,
    CMD_ERASE_OPTIONS, CMD_GET, CMD_GID, CMD_READ, CMD_RESET, CMD_WRITE, DEVICE_ID, LOADER_VERSION,
    NACK, OPTION_BYTES, SYNC,
};
use fastprom_core::update::{Flash, FlashError, PAGE_LEN};

struct Chip {
    flash: RefCell<Vec<u8>>,
    options: RefCell<[u8; 128]>,
    resets: Cell<u32>,
    erases: Cell<u32>,
}

impl Chip {
    fn new() -> Self {
        Self {
            flash: RefCell::new(vec![0xFF; FLASH_SIZE as usize]),
            options: RefCell::new([0xFF; 128]),
            resets: Cell::new(0),
            erases: Cell::new(0),
        }
    }
}

impl Flash for Chip {
    fn erase_sector(&self, addr: u32) -> Result<(), FlashError> {
        let at = (addr - FLASH_BASE) as usize;
        self.flash.borrow_mut()[at..at + SECTOR as usize].fill(0xFF);
        self.erases.set(self.erases.get() + 1);
        Ok(())
    }
    fn program_page(&self, addr: u32, page: &[u8; PAGE_LEN]) -> Result<(), FlashError> {
        let at = (addr - FLASH_BASE) as usize;
        for (d, s) in self.flash.borrow_mut()[at..at + PAGE_LEN]
            .iter_mut()
            .zip(page)
        {
            *d &= s;
        }
        Ok(())
    }
    fn read(&self, addr: u32, buf: &mut [u8]) {
        let at = (addr - FLASH_BASE) as usize;
        buf.copy_from_slice(&self.flash.borrow()[at..at + buf.len()]);
    }
}

impl Target for Chip {
    fn device_id(&self) -> u32 {
        DEVICE_ID
    }
    fn read_options(&self, buf: &mut [u8; 128]) {
        *buf = *self.options.borrow();
    }
    fn erase_options(&self) -> Result<(), FlashError> {
        *self.options.borrow_mut() = [0xFF; 128];
        Ok(())
    }
    fn program_options(&self, image: &[u8; 128]) -> Result<(), FlashError> {
        for (d, s) in self.options.borrow_mut().iter_mut().zip(image) {
            *d &= s;
        }
        Ok(())
    }
    fn reset(&self) {
        self.resets.set(self.resets.get() + 1);
    }
}

/// Feed `bytes`, collect what the server says.
fn talk(s: &mut Server, chip: &Chip, bytes: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    for &b in bytes {
        s.feed(b, chip, &mut |r: &[u8]| out.extend_from_slice(r));
    }
    out
}

fn cmd(c: u8) -> [u8; 2] {
    [c, !c]
}

fn write_frame(data: &[u8]) -> Vec<u8> {
    let mut f = vec![(data.len() - 1) as u8];
    f.extend_from_slice(data);
    f.push(xor(&f));
    f
}

fn write(s: &mut Server, chip: &Chip, addr: u32, data: &[u8]) -> Vec<u8> {
    let mut all = cmd(CMD_WRITE).to_vec();
    all.extend_from_slice(&address_frame(addr));
    all.extend(write_frame(data));
    talk(s, chip, &all)
}

fn read(s: &mut Server, chip: &Chip, addr: u32, len: usize) -> Vec<u8> {
    let n = (len - 1) as u8;
    let mut all = cmd(CMD_READ).to_vec();
    all.extend_from_slice(&address_frame(addr));
    all.extend_from_slice(&[n, !n]);
    talk(s, chip, &all)
}

fn erase(s: &mut Server, chip: &Chip, sectors: &[u16]) -> Vec<u8> {
    let mut f = ((sectors.len() - 1) as u16).to_be_bytes().to_vec();
    for n in sectors {
        f.extend_from_slice(&n.to_be_bytes());
    }
    f.push(xor(&f));
    let mut all = cmd(CMD_ERASE).to_vec();
    all.extend(f);
    talk(s, chip, &all)
}

#[test]
fn sync_get_and_gid() {
    let chip = Chip::new();
    let mut s = Server::new();
    assert_eq!(talk(&mut s, &chip, &[SYNC]), [ACK]);
    let mut get = vec![ACK, COMMANDS.len() as u8, LOADER_VERSION];
    get.extend_from_slice(&COMMANDS);
    get.push(ACK);
    assert_eq!(talk(&mut s, &chip, &cmd(CMD_GET)), get);
    assert_eq!(
        talk(&mut s, &chip, &cmd(CMD_GID)),
        [ACK, 3, 0x40, 0x20, 0x00, 0x01, ACK]
    );
    // A bad complement, and an unknown command.
    assert_eq!(talk(&mut s, &chip, &[CMD_GET, 0x00]), [NACK]);
    assert_eq!(talk(&mut s, &chip, &cmd(0x63)), [NACK]);
    assert!(!s.busy());
}

#[test]
fn write_programs_a_page_and_reads_back() {
    let chip = Chip::new();
    let mut s = Server::new();
    let data: Vec<u8> = (0..200u32).map(|i| (i * 7) as u8).collect();
    let addr = FLASH_BASE + 0x8000;
    assert_eq!(write(&mut s, &chip, addr, &data), [ACK, ACK, ACK]);
    let back = read(&mut s, &chip, addr, 256);
    assert_eq!(&back[..3], [ACK, ACK, ACK]);
    assert_eq!(&back[3..203], &data[..]);
    assert!(
        back[203..].iter().all(|&b| b == 0xFF),
        "the rest stays erased"
    );
    // Over programmed flash it does not read back: refused.
    let mut changed = data.clone();
    changed[0] = !data[0];
    assert_eq!(write(&mut s, &chip, addr, &changed), [ACK, ACK, NACK]);
    // The same frame again reads back: a retried write is harmless.
    assert_eq!(write(&mut s, &chip, addr, &data), [ACK, ACK, ACK]);
}

#[test]
fn refusals() {
    let chip = Chip::new();
    let mut s = Server::new();
    // Not page aligned; outside flash and option bytes; SRAM.
    for addr in [
        FLASH_BASE + 4,
        FLASH_BASE + FLASH_SIZE,
        0x2001_0000,
        OPTION_BASE + 2,
    ] {
        let mut all = cmd(CMD_WRITE).to_vec();
        all.extend_from_slice(&address_frame(addr));
        assert_eq!(talk(&mut s, &chip, &all), [ACK, NACK], "{addr:#x}");
    }
    // A bad address checksum.
    let mut all = cmd(CMD_READ).to_vec();
    let mut a = address_frame(FLASH_BASE);
    a[4] ^= 1;
    all.extend_from_slice(&a);
    assert_eq!(talk(&mut s, &chip, &all), [ACK, NACK]);
    // A bad data checksum: nothing is programmed.
    let mut all = cmd(CMD_WRITE).to_vec();
    all.extend_from_slice(&address_frame(FLASH_BASE));
    let mut f = write_frame(&[1, 2, 3]);
    *f.last_mut().unwrap() ^= 0x80;
    all.extend(f);
    assert_eq!(talk(&mut s, &chip, &all), [ACK, ACK, NACK]);
    assert!(chip.flash.borrow()[..3].iter().all(|&b| b == 0xFF));
    // A read whose count's complement is wrong, or that runs off the end.
    let mut all = cmd(CMD_READ).to_vec();
    all.extend_from_slice(&address_frame(FLASH_BASE));
    all.extend_from_slice(&[3, 3]);
    assert_eq!(talk(&mut s, &chip, &all), [ACK, ACK, NACK]);
    assert_eq!(
        read(&mut s, &chip, FLASH_BASE + FLASH_SIZE - 16, 32),
        [ACK, ACK, NACK]
    );
    assert_eq!(read(&mut s, &chip, OPTION_BASE + 120, 16), [ACK, ACK, NACK]);
    // More option bytes than there are.
    let mut all = cmd(CMD_WRITE).to_vec();
    all.extend_from_slice(&address_frame(OPTION_BASE));
    all.push(200);
    assert_eq!(talk(&mut s, &chip, &all), [ACK, ACK, NACK]);
    // A sector past the end: nothing at all is erased.
    chip.flash.borrow_mut()[0] = 0;
    assert_eq!(erase(&mut s, &chip, &[0, 64]), [ACK, NACK]);
    assert_eq!(chip.erases.get(), 0);
    // Mass erase is not offered.
    let mut all = cmd(CMD_ERASE).to_vec();
    all.extend_from_slice(&[0xFF, 0xFF, 0x00]);
    assert_eq!(talk(&mut s, &chip, &all), [ACK, NACK]);
    assert!(!s.busy());
}

#[test]
fn erase_sectors() {
    let chip = Chip::new();
    let mut s = Server::new();
    chip.flash.borrow_mut().fill(0);
    assert_eq!(erase(&mut s, &chip, &[0, 63]), [ACK, ACK]);
    let f = chip.flash.borrow();
    assert!(f[..SECTOR as usize].iter().all(|&b| b == 0xFF));
    assert!(f[(FLASH_SIZE - SECTOR) as usize..]
        .iter()
        .all(|&b| b == 0xFF));
    assert!(f[SECTOR as usize..2 * SECTOR as usize]
        .iter()
        .all(|&b| b == 0));
}

#[test]
fn option_bytes_erase_write_read() {
    let chip = Chip::new();
    let mut s = Server::new();
    chip.options.borrow_mut().fill(0);
    assert_eq!(talk(&mut s, &chip, &cmd(CMD_ERASE_OPTIONS)), [ACK, ACK]);
    assert_eq!(*chip.options.borrow(), [0xFF; 128]);
    assert_eq!(
        write(&mut s, &chip, OPTION_BASE, &OPTION_BYTES),
        [ACK, ACK, ACK]
    );
    let back = read(&mut s, &chip, OPTION_BASE, OPTION_LEN as usize);
    assert_eq!(&back[3..], &OPTION_BYTES[..]);
    assert_eq!(fabric_address(&back[3..]), Some(FABRIC_BASE));
}

#[test]
fn reset_answers_twice_first() {
    let chip = Chip::new();
    let mut s = Server::new();
    assert_eq!(talk(&mut s, &chip, &cmd(CMD_RESET)), [ACK, ACK]);
    assert_eq!(chip.resets.get(), 1);
}

#[test]
fn a_host_that_stops_part_way_is_forgotten() {
    let chip = Chip::new();
    let mut s = Server::new();
    let mut all = cmd(CMD_WRITE).to_vec();
    all.extend_from_slice(&address_frame(FLASH_BASE));
    all.extend_from_slice(&[9, 1, 2]);
    talk(&mut s, &chip, &all);
    assert!(s.busy());
    s.idle();
    assert_eq!(talk(&mut s, &chip, &[SYNC]), [ACK]);
}

/// gen_batch's option bytes for `--logic-address 0x80014000` and nothing
/// else (`make fw-batch`), worked out from its `gen_option_bytes`: RDP's
/// 0xA5 and complement, 46 erased bytes, the address and its complement.
#[test]
fn option_bytes_are_gen_batchs() {
    let mut want = vec![0xA5, 0x5A];
    want.extend([0xFF; 46]);
    want.extend_from_slice(&0x8001_4000u32.to_le_bytes());
    want.extend_from_slice(&(0xFFFF_FFFFu32 - 0x8001_4000).to_le_bytes());
    want.resize(128, 0xFF);
    assert_eq!(OPTION_BYTES.to_vec(), want);
    assert_eq!(fabric_address(&[0xFF; 128]), None);
}

#[test]
fn loader_header() {
    let code: Vec<u8> = (0..3000u32).map(|i| (i * 13) as u8).collect();
    let h = LoaderHeader {
        image_len: code.len() as u32,
        image_crc: crc32(&code),
        fw_version: Version {
            major: 0,
            minor: 1,
            patch: 0,
        },
        board_rev: BoardRev::new("A").unwrap(),
    };
    let mut file = h.encode().to_vec();
    file.extend_from_slice(&code);
    assert_eq!(LoaderHeader::check(&file), Ok(h));

    let mut bad = file.clone();
    bad[40] ^= 1;
    assert!(matches!(
        LoaderHeader::check(&bad),
        Err(LoaderError::Crc { .. })
    ));
    assert!(matches!(
        LoaderHeader::check(&file[..file.len() - 1]),
        Err(LoaderError::Stated { .. })
    ));
    assert!(matches!(
        LoaderHeader::check(&file[..20]),
        Err(LoaderError::Length { .. })
    ));
    let mut big = file.clone();
    big.resize(RECOVERY_LEN as usize + 1, 0);
    assert!(matches!(
        LoaderHeader::check(&big),
        Err(LoaderError::Length { .. })
    ));
    let mut magic = file;
    magic[0] = 0;
    assert!(matches!(
        LoaderHeader::check(&magic),
        Err(LoaderError::Magic { .. })
    ));
}
