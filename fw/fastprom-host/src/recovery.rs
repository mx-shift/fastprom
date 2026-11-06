//! UART recovery: provisioning a blank or bricked board through the AG32's
//! mask-ROM serial loader (`docs/10-updates.md`, "Recovery";
//! `fastprom_core::recovery` has the protocol and the evidence for it).
//!
//! The user wires a 3.3 V USB-UART adapter to J3 (pad 1 TX to the
//! adapter's RX, pad 2 RX to its TX, pad 3 GND), bridges BOOT0 (pad 4) to
//! 3.3 V (pad 5) and resets the board. Then [`recover`]:
//!
//! 1. syncs and asks `GET` who is answering: the mask ROM (version at most
//!    0x10), or FastPROM's recovery loader already running (a second try
//!    after a failure); anything else is refused;
//! 2. checks `GID` is the AG32VF303's;
//! 3. with the mask ROM: probes read protection (a one-byte read of SRAM,
//!    refused when protected) and, if it is on, `READOUT UNPROTECT`s (the
//!    chip erases its flash) and syncs again; uploads the recovery loader
//!    into SRAM 256 bytes at a time, reading each back; and `GO`es to it;
//!    then syncs with the loader;
//! 4. refuses, before writing anything, a board whose flash still holds an
//!    application header or fabric manifest for another board revision;
//! 5. writes the option bytes (erase, write, read back) unless they are
//!    already right;
//! 6. erases every sector (the image table is left empty);
//! 7. writes the DFU stage, then the fabric and its manifest, then the
//!    application with its header page last, each page read back as it is
//!    written; the header and the manifest go in committed, as AGM's loader
//!    writes them, so the board boots straight to the application;
//! 8. resets the chip (with BOOT0 still bridged, back into the mask ROM).
//!
//! The user then removes the bridge and power-cycles the board: the fabric
//! configures from flash at power-on, and the board comes up on USB.
//!
//! Every command that fails part way (a NACK, a timeout) is retried, after
//! [`Loader::resync`], up to [`ATTEMPTS`] times; a write of a page that is
//! already written with the same bytes is harmless, so a retry after a lost
//! ACK is safe.

use serde::Serialize;

use fastprom_core::header::{AppHeader, Manifest, PAGE_LEN};
use fastprom_core::memmap::{
    APP_BASE, DFU_BASE, FABRIC_BASE, FLASH_SIZE, MANIFEST_BASE, OPTION_BASE, OPTION_LEN,
    RECOVERY_BASE, RECOVERY_ENTRY, SECTOR,
};
use fastprom_core::recovery::{
    LoaderHeader, CMD_GID, CMD_GO, CMD_READ, CMD_READOUT_UNPROTECT, CMD_WRITE, DEVICE_ID,
    LOADER_VERSION, MASK_ROM_MAX_VERSION, MAX_FRAME, MMODE_SRAM, OPTION_BYTES,
};

use crate::an3155::{Loader, LoaderError, OPTIONS_MS};
use crate::bundle::{self, AppPayload, Bundle, FabricPayload};
use crate::serial::{SerialError, SerialPort};
use crate::update::Progress;

/// Tries of each command before recovery gives up.
pub const ATTEMPTS: u32 = 4;
/// How long the recovery loader has to come up after `GO` (the PLL and
/// UART0), before the first sync.
pub const START_MS: u32 = 100;
/// After `READOUT UNPROTECT` the chip erases and resets.
pub const UNPROTECT_RESET_MS: u32 = 500;
/// Sectors per erase command, for the progress bar.
const ERASE_GROUP: usize = 8;
/// How long a page write may take to be acknowledged: the frame, the
/// program, the read back.
const WRITE_MS: u32 = 1_000;

/// Why recovery stopped. Each says what to do about it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RecoveryError {
    /// Nothing answers on the port.
    NoAnswer,
    /// Something answers, with another chip's ID.
    WrongDevice(u32),
    /// A boot loader that is neither the mask ROM nor FastPROM's.
    UnknownLoader(u8),
    /// The mask ROM lacks a command recovery needs.
    MissingCommand(u8),
    /// A command failed every time it was tried.
    Failed(LoaderError),
    /// Something read back is not what was written.
    Verify(String),
    /// The board or the files are not what recovery may write.
    Check(String),
    /// The serial port went away.
    Gone,
}

impl core::fmt::Display for RecoveryError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::NoAnswer => f.write_str(
                "No answer from the board's boot loader. Check the wiring: J3 pad 1 (TX) to \
                 the adapter's RX, pad 2 (RX) to its TX, pad 3 to GND; a 3.3 V adapter. Check \
                 that BOOT0 (pad 4) is bridged to 3.3 V (pad 5), then reset the board (pulse \
                 pad 6, NRST, to GND, or power it off and on) with the bridge in place, and \
                 try again.",
            ),
            Self::WrongDevice(id) => write!(
                f,
                "The chip on this port says it is device {id:#010x}, not the AG32VF303 \
                 ({DEVICE_ID:#010x}) a FastPROM board carries. Nothing was written."
            ),
            Self::UnknownLoader(v) => write!(
                f,
                "A boot loader of version {v:#04x} answered: neither the AG32's mask ROM nor \
                 FastPROM's recovery loader. Reset the board with BOOT0 bridged and try again. \
                 Nothing was written."
            ),
            Self::MissingCommand(c) => write!(
                f,
                "The mask ROM does not list command {c:#04x}, which recovery needs. Nothing \
                 was written."
            ),
            Self::Failed(e) => write!(
                f,
                "{e}, {ATTEMPTS} times. Check the wiring and the adapter, reset the board \
                 with BOOT0 bridged, and run recovery again: it starts over safely."
            ),
            Self::Verify(what) => write!(
                f,
                "{what}. The flash may be failing; run recovery again, and if it fails the \
                 same way the chip needs replacing."
            ),
            Self::Check(what) => f.write_str(what),
            Self::Gone => f.write_str(
                "The serial port went away: the adapter was unplugged. Plug it in, reset the \
                 board with BOOT0 bridged, choose the port again and run recovery again.",
            ),
        }
    }
}

impl From<LoaderError> for RecoveryError {
    fn from(e: LoaderError) -> Self {
        match e {
            LoaderError::Port(SerialError::Gone) => Self::Gone,
            e => Self::Failed(e),
        }
    }
}

/// Everything recovery writes, checked.
#[derive(Debug, Clone)]
pub struct RecoveryImage {
    pub board_rev: String,
    /// The recovery loader file, header and all, as uploaded.
    pub loader: Vec<u8>,
    pub loader_header: LoaderHeader,
    /// The DFU stage, padded to whole pages.
    pub dfu: Vec<u8>,
    /// The application, if the bundle has it and a fabric for the part.
    pub app: Option<AppPayload>,
    pub fabric: Option<FabricPayload>,
}

/// The application and a part's fabric, as fetched from the bundle: they
/// are written together or not at all, since an application without its
/// fabric would start with no USB.
#[derive(Debug, Clone, Copy)]
pub struct FullFiles<'a> {
    pub part: &'a str,
    pub app: &'a [u8],
    /// The bitstream and its manifest page.
    pub fabric: &'a [u8],
    pub page: &'a [u8],
}

impl RecoveryImage {
    /// Check the files fetched from the bundle.
    ///
    /// # Errors
    ///
    /// A message naming the first file that does not hold.
    pub fn check(
        b: &Bundle,
        loader: &[u8],
        dfu: &[u8],
        options: &[u8],
        full: Option<FullFiles<'_>>,
    ) -> Result<Self, String> {
        let r = b
            .recovery
            .as_ref()
            .ok_or("this page's bundle has no recovery files")?;
        let loader_header = bundle::check_loader(&r.loader, loader)?;
        let dfu = bundle::check_dfu(&r.dfu, dfu)?;
        bundle::check_option_bytes(&r.option_bytes, options)?;
        let (app, fabric) = match full {
            None => (None, None),
            Some(FullFiles {
                part,
                app,
                fabric: bits,
                page,
            }) => {
                let fw = b.firmware.as_ref().ok_or("the bundle has no firmware")?;
                let f = b
                    .fabrics
                    .iter()
                    .find(|f| f.part == part)
                    .ok_or_else(|| format!("the bundle has no fabric for part {part}"))?;
                (
                    Some(bundle::check_firmware(fw, app)?),
                    Some(bundle::check_fabric(f, bits, page)?),
                )
            }
        };
        Ok(Self {
            board_rev: b.board_rev.clone(),
            loader: loader.to_vec(),
            loader_header,
            dfu,
            app,
            fabric,
        })
    }
}

/// What recovery will write from a bundle, for the page to show first.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RecoveryPlan {
    /// Set when the bundle cannot recover a board, and why.
    pub refuse: Option<String>,
    /// The board revision the bundle is for: the user confirms the board's.
    pub board_rev: String,
    /// What goes into flash, in words.
    pub writes: Vec<String>,
    /// What the board does afterwards.
    pub outcome: String,
    /// Whether the application and the part's fabric are written.
    pub full: bool,
}

/// What recovery can write from `b` for `part` (the user's choice of the
/// bundle's fabrics, or none).
pub fn plan(b: &Bundle, part: Option<&str>) -> RecoveryPlan {
    let mut p = RecoveryPlan {
        refuse: None,
        board_rev: b.board_rev.clone(),
        writes: Vec::new(),
        outcome: String::new(),
        full: false,
    };
    let Some(r) = &b.recovery else {
        p.refuse = Some(
            "This page's update bundle has no recovery files (it was built without the \
             firmware), so it cannot recover a board."
                .into(),
        );
        return p;
    };
    p.writes
        .push("the option bytes: read protection off, the fabric at 0x80014000".into());
    p.writes.push(format!(
        "the DFU stage {}",
        r.dfu.version.as_deref().unwrap_or("")
    ));
    let fabric = part.and_then(|part| b.fabrics.iter().find(|f| f.part == part));
    match (&b.firmware, fabric) {
        (Some(fw), Some(f)) => {
            p.full = true;
            p.writes.push(format!("the fabric for part {}", f.part));
            p.writes.push(format!("the firmware {}", fw.version));
            p.writes.push("an empty image table".into());
            p.outcome = format!(
                "The board starts firmware {} with no ROM images: connect it over USB and \
                 write its images.",
                fw.version
            );
        }
        _ => {
            p.writes.push("no firmware and no fabric".into());
            p.outcome = "The board starts in its update mode: connect it over USB and install \
                         the firmware and fabric from an update that has the part's fabric."
                .into();
        }
    }
    p
}

/// What [`recover`] did.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Report {
    /// Who answered first: `"mask ROM 0x10"` or `"recovery loader"`.
    pub found: String,
    /// The mask ROM reported read protection, and recovery removed it.
    pub unprotected: bool,
    /// The recovery loader was uploaded (not already running).
    pub uploaded: bool,
    /// What the board's flash held before, if anything was recognisable.
    pub before: Vec<String>,
    /// What was written.
    pub wrote: Vec<String>,
    /// Commands that had to be tried again.
    pub retries: u32,
    /// The chip acknowledged the reset at the end.
    pub reset: bool,
}

struct Run<'a, 'p, P> {
    l: &'a mut Loader<P>,
    progress: Progress<'p>,
    report: Report,
}

/// Try `$op` (an expression of `$run.l`) up to [`ATTEMPTS`] times, with a
/// resync between.
macro_rules! retry {
    ($run:expr, $op:expr) => {{
        let mut tries = 1;
        loop {
            match $op.await {
                Ok(v) => break Ok(v),
                Err(e) if e.retryable() && tries < ATTEMPTS => {
                    tries += 1;
                    $run.report.retries += 1;
                    if let Err(e) = $run.l.resync().await {
                        break Err(e);
                    }
                }
                Err(e) => break Err(e),
            }
        }
    }};
}

impl<P: SerialPort> Run<'_, '_, P> {
    /// Who is answering: `Ok(true)` the mask ROM, `Ok(false)` the
    /// recovery loader.
    /// A one-byte read of SRAM through the mask ROM: `Ok(false)` if it is
    /// refused, which is read protection.
    async fn readable(&mut self) -> Result<bool, LoaderError> {
        match self.l.read(RECOVERY_BASE, 1).await {
            Ok(_) => Ok(true),
            Err(LoaderError::Nack(_)) => Ok(false),
            Err(e) => Err(e),
        }
    }

    async fn identify(&mut self) -> Result<bool, RecoveryError> {
        let get = retry!(self, self.l.get())?;
        let mask_rom = if get.version <= MASK_ROM_MAX_VERSION {
            for c in [CMD_GID, CMD_READ, CMD_WRITE, CMD_GO] {
                if !get.commands.contains(&c) {
                    return Err(RecoveryError::MissingCommand(c));
                }
            }
            true
        } else if get.version == LOADER_VERSION {
            false
        } else {
            return Err(RecoveryError::UnknownLoader(get.version));
        };
        let id = retry!(self, self.l.gid())?;
        if id != DEVICE_ID {
            return Err(RecoveryError::WrongDevice(id));
        }
        Ok(mask_rom)
    }

    async fn sync(&mut self) -> Result<(), RecoveryError> {
        self.l.sync().await.map(|_| ()).map_err(|e| match e {
            LoaderError::Port(SerialError::Gone) => RecoveryError::Gone,
            _ => RecoveryError::NoAnswer,
        })
    }

    /// The mask ROM's part: protection, the upload, GO.
    async fn upload(&mut self, img: &RecoveryImage) -> Result<(), RecoveryError> {
        self.l.mask_rom(MMODE_SRAM);
        // A read the mask ROM refuses: read protection is on.
        if !retry!(self, self.readable())? {
            let get = retry!(self, self.l.get())?;
            if !get.commands.contains(&CMD_READOUT_UNPROTECT) {
                return Err(RecoveryError::MissingCommand(CMD_READOUT_UNPROTECT));
            }
            (self.progress)("removing read protection (the chip erases itself)", 0, 1);
            self.l.readout_unprotect().await?;
            self.l.port().sleep(UNPROTECT_RESET_MS).await;
            self.sync().await?;
            if !self.identify().await? {
                return Err(RecoveryError::Check(
                    "after READOUT UNPROTECT the mask ROM did not answer again".into(),
                ));
            }
            self.l.mask_rom(MMODE_SRAM);
            if !retry!(self, self.readable())? {
                return Err(RecoveryError::Check(
                    "the chip is still read protected after READOUT UNPROTECT".into(),
                ));
            }
            self.report.unprotected = true;
            (self.progress)("removing read protection (the chip erases itself)", 1, 1);
        }
        let total = img.loader.len() as u32;
        let step = "uploading the recovery loader";
        for (i, chunk) in img.loader.chunks(MAX_FRAME).enumerate() {
            let addr = RECOVERY_BASE + (i * MAX_FRAME) as u32;
            retry!(self, self.l.write(addr, chunk, WRITE_MS))?;
            let back = retry!(self, self.l.read(addr, chunk.len()))?;
            if back != chunk {
                return Err(RecoveryError::Verify(format!(
                    "the recovery loader read back differently from SRAM at {addr:#010x}"
                )));
            }
            (self.progress)(step, (addr - RECOVERY_BASE) + chunk.len() as u32, total);
        }
        retry!(self, self.l.go(RECOVERY_ENTRY))?;
        self.l.plain();
        self.l.port().sleep(START_MS).await;
        self.sync().await?;
        if self.identify().await? {
            return Err(RecoveryError::Check(
                "the recovery loader did not start: the mask ROM answered after GO".into(),
            ));
        }
        self.report.uploaded = true;
        Ok(())
    }

    /// What the flash holds now, and a refusal if it is another board's.
    async fn inspect(&mut self, img: &RecoveryImage) -> Result<(), RecoveryError> {
        let app = retry!(self, self.l.read(APP_BASE, PAGE_LEN))?;
        let man = retry!(self, self.l.read(MANIFEST_BASE, PAGE_LEN))?;
        let mut revs = Vec::new();
        if let Ok((_, h)) = AppHeader::decode(&page(&app)) {
            self.report.before.push(format!(
                "firmware {} for board {}",
                h.fw_version,
                h.board_rev.as_str()
            ));
            revs.push(h.board_rev.as_str().to_owned());
        }
        if let Ok((_, m)) = Manifest::decode(&page(&man)) {
            self.report.before.push(format!(
                "a fabric for part {} on board {}",
                m.part.as_str(),
                m.board_rev.as_str()
            ));
            revs.push(m.board_rev.as_str().to_owned());
        }
        if let Some(r) = revs.iter().find(|r| **r != img.board_rev) {
            return Err(RecoveryError::Check(format!(
                "This board's flash holds firmware or a fabric for board revision {r}, and \
                 this update is for revision {}: nothing was written. Use the page published \
                 for revision {r}.",
                img.board_rev
            )));
        }
        Ok(())
    }

    async fn options(&mut self) -> Result<(), RecoveryError> {
        let step = "writing the option bytes";
        (self.progress)(step, 0, 1);
        let now = retry!(self, self.l.read(OPTION_BASE, OPTION_LEN as usize))?;
        if now != OPTION_BYTES {
            retry!(self, self.l.erase_options())?;
            retry!(self, self.l.write(OPTION_BASE, &OPTION_BYTES, OPTIONS_MS))?;
            let back = retry!(self, self.l.read(OPTION_BASE, OPTION_LEN as usize))?;
            if back != OPTION_BYTES {
                return Err(RecoveryError::Verify(
                    "the option bytes read back differently".into(),
                ));
            }
        }
        self.report.wrote.push("option bytes".into());
        (self.progress)(step, 1, 1);
        Ok(())
    }

    async fn erase_all(&mut self) -> Result<(), RecoveryError> {
        let sectors: Vec<u16> = (0..(FLASH_SIZE / SECTOR) as u16).collect();
        let total = sectors.len() as u32;
        for (i, group) in sectors.chunks(ERASE_GROUP).enumerate() {
            retry!(self, self.l.erase(group))?;
            (self.progress)("erasing the flash", ((i + 1) * ERASE_GROUP) as u32, total);
        }
        Ok(())
    }

    /// Write `data` at `addr` page by page, reading each back; the pages
    /// in `order`.
    async fn write_pages(
        &mut self,
        addr: u32,
        data: &[u8],
        order: &[usize],
        what: &str,
    ) -> Result<(), RecoveryError> {
        let total = order.len() as u32;
        let step = format!("writing {what}");
        for (n, &i) in order.iter().enumerate() {
            let Some(chunk) = data.get(i * PAGE_LEN..(i + 1) * PAGE_LEN) else {
                continue;
            };
            let at = addr + (i * PAGE_LEN) as u32;
            if chunk.iter().any(|&b| b != 0xFF) {
                retry!(self, self.l.write(at, chunk, WRITE_MS))?;
                let back = retry!(self, self.l.read(at, PAGE_LEN))?;
                if back != chunk {
                    return Err(RecoveryError::Verify(format!(
                        "{what} read back differently at {at:#010x}"
                    )));
                }
            }
            (self.progress)(&step, n as u32 + 1, total);
        }
        self.report.wrote.push(what.to_owned());
        Ok(())
    }
}

fn page(b: &[u8]) -> [u8; PAGE_LEN] {
    let mut p = [0xFFu8; PAGE_LEN];
    for (d, s) in p.iter_mut().zip(b) {
        *d = *s;
    }
    p
}

fn pages(data: &[u8]) -> Vec<usize> {
    (0..data.len().div_ceil(PAGE_LEN)).collect()
}

/// Provision the board on `l`'s port with `img`. The port is open, at
/// [`fastprom_core::recovery::BAUD`] 8E1.
///
/// # Errors
///
/// Why it stopped; [`RecoveryError`]'s text says what to do.
pub async fn recover<P: SerialPort>(
    l: &mut Loader<P>,
    img: &RecoveryImage,
    progress: Progress<'_>,
) -> Result<Report, RecoveryError> {
    let mut run = Run {
        l,
        progress,
        report: Report::default(),
    };
    run.l.plain();
    run.sync().await?;
    // The mask ROM's dialect for the identification: GET and GID take no
    // memory-mode byte, so either works.
    let mask_rom = run.identify().await?;
    if mask_rom {
        run.report.found = "the mask ROM".into();
        run.upload(img).await?;
    } else {
        run.report.found = "FastPROM's recovery loader, already running".into();
    }
    run.inspect(img).await?;
    run.options().await?;
    run.erase_all().await?;
    run.write_pages(DFU_BASE, &img.dfu, &pages(&img.dfu), "the DFU stage")
        .await?;
    if let (Some(app), Some(fabric)) = (&img.app, &img.fabric) {
        run.write_pages(
            FABRIC_BASE,
            &fabric.data,
            &pages(&fabric.data),
            "the fabric",
        )
        .await?;
        run.write_pages(MANIFEST_BASE, &fabric.page, &[0], "the fabric manifest")
            .await?;
        // The header page last: until it is written, the DFU stage's boot
        // check finds no application and stays in DFU mode.
        let mut order: Vec<usize> = pages(&app.data).into_iter().skip(1).collect();
        order.push(0);
        run.write_pages(APP_BASE, &app.data, &order, "the firmware")
            .await?;
    }
    run.report.reset = run.l.reset().await.is_ok();
    Ok(run.report)
}
