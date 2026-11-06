//! The procedures the page runs (`docs/10-updates.md`, "The update protocol" and "The
//! web page"), over any [`Transport`].
//!
//! Each takes a progress callback: `(step, done, total)`, with `step` a few
//! words for the user ("erasing", "writing", "verifying", "committing").
//!
//! Reconnecting after a reboot is the caller's: only it knows how devices
//! come and go (WebUSB's `connect` event, or the simulator).

use fastprom_core::header::{OFF_COMMIT, PAGE_LEN};
use fastprom_core::memmap::{
    APP_BASE, APP_LEN, FABRIC_BASE, FABRIC_LEN, MANIFEST_BASE, MANIFEST_LEN, SECTOR, TABLE_BASE,
    TABLE_LEN,
};
use fastprom_core::update::proto::{TARGET_APPLICATION, TARGET_DFU, TARGET_FABRIC};
use fastprom_core::ImageTable;

use crate::bundle::{AppPayload, FabricPayload};
use crate::client::{Client, Error};
use crate::info::{DeviceInfo, Mode};
use crate::transport::Transport;

/// `(step, done, total)`.
pub type Progress<'a> = &'a mut dyn FnMut(&str, u32, u32);

/// A connected device: its `INFO`, and its image table if it has a valid
/// one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Connected {
    pub info: DeviceInfo,
    pub table: Option<Vec<u8>>,
}

/// Start talking to a device: reset the interface, `INFO`, and read the
/// image table if `INFO` says it is valid.
///
/// # Errors
///
/// How the device failed to answer.
pub async fn connect<T: Transport>(c: &mut Client<T>) -> Result<Connected, Error> {
    c.reset().await?;
    let info = c.info().await?;
    let table = match &info.table {
        Some(t) => {
            let len = 256 + u32::from(t.count) * t.depth;
            if len > TABLE_LEN {
                return Err(Error::Protocol(format!("a table of {len} bytes")));
            }
            Some(c.read(TABLE_BASE, len, &mut |_| ()).await?)
        }
        None => None,
    };
    Ok(Connected { info, table })
}

fn sectors(len: usize) -> u32 {
    (len as u32).div_ceil(SECTOR) * SECTOR
}

fn padded(data: &[u8]) -> Vec<u8> {
    let mut v = data.to_vec();
    v.resize(data.len().div_ceil(PAGE_LEN) * PAGE_LEN, 0xFF);
    v
}

/// Erase, write and read back `data` at `addr`; the first `skip` bytes are
/// not compared (a commit word the device holds back).
async fn program<T: Transport>(
    c: &mut Client<T>,
    addr: u32,
    erase: u32,
    data: &[u8],
    skip: usize,
    what: &str,
    progress: Progress<'_>,
) -> Result<(), Error> {
    let total = data.len() as u32;
    if erase > 0 {
        progress(&format!("erasing {what}"), 0, erase);
        c.erase(addr, erase).await?;
        progress(&format!("erasing {what}"), erase, erase);
    }
    let step = format!("writing {what}");
    c.write(addr, data, &mut |d| progress(&step, d, total))
        .await?;
    verify(c, addr, data, skip, what, progress).await
}

async fn verify<T: Transport>(
    c: &mut Client<T>,
    addr: u32,
    data: &[u8],
    skip: usize,
    what: &str,
    progress: Progress<'_>,
) -> Result<(), Error> {
    let total = data.len() as u32;
    let step = format!("verifying {what}");
    let back = c
        .read(addr, total, &mut |d| progress(&step, d, total))
        .await?;
    match back.iter().zip(data).skip(skip).position(|(a, b)| a != b) {
        None => Ok(()),
        Some(i) => Err(Error::Check(format!(
            "{what} read back differs at byte {:#x} (flash {:#010x})",
            i + skip,
            addr as usize + i + skip
        ))),
    }
}

/// Write an image table and read it back. Leaves the socket alone: `LOAD`
/// is a separate step, with the host powered off.
///
/// # Errors
///
/// A table the firmware would refuse, or as the device answers.
pub async fn write_table<T: Transport>(
    c: &mut Client<T>,
    table: &[u8],
    progress: Progress<'_>,
) -> Result<(), Error> {
    ImageTable::parse(table)
        .map_err(|e| Error::Check(format!("the firmware would refuse this table: {e}")))?;
    if table.len() > TABLE_LEN as usize {
        return Err(Error::Check("the table does not fit its region".into()));
    }
    let data = padded(table);
    program(
        c,
        TABLE_BASE,
        sectors(data.len()),
        &data,
        0,
        "the image table",
        progress,
    )
    .await
}

/// `LOAD`: serve the new table's image for the switch's position.
///
/// # Errors
///
/// As the device answers.
pub async fn load<T: Transport>(c: &mut Client<T>) -> Result<(), Error> {
    c.load().await
}

/// `REBOOT` and accept the device leaving with or after its answer.
async fn reboot<T: Transport>(c: &mut Client<T>, target: u8) -> Result<(), Error> {
    match c.reboot(target).await {
        Err(e) if e.is_gone() => Ok(()),
        r => r,
    }
}

/// Reboot into DFU mode. The device disconnects; the caller finds it again.
///
/// # Errors
///
/// As the device answers.
pub async fn enter_dfu<T: Transport>(c: &mut Client<T>) -> Result<(), Error> {
    reboot(c, TARGET_DFU).await
}

/// Reboot through the DFU stage's check into the application.
///
/// # Errors
///
/// As the device answers.
pub async fn reboot_application<T: Transport>(c: &mut Client<T>) -> Result<(), Error> {
    reboot(c, TARGET_APPLICATION).await
}

fn need_dfu(info: &DeviceInfo, board_rev: &str) -> Result<(), Error> {
    if info.mode != Mode::Dfu {
        return Err(Error::Check(
            "firmware and fabric are written in DFU mode".into(),
        ));
    }
    if info.board_rev != board_rev {
        return Err(Error::Check(format!(
            "the update is for board revision {board_rev}, the board is {}",
            info.board_rev
        )));
    }
    Ok(())
}

/// The application: erase its region, write the file, read it back, then
/// `COMMIT 0`. `info` is the DFU stage's.
///
/// # Errors
///
/// A device or board that is not the payload's, or as the device answers.
pub async fn update_firmware<T: Transport>(
    c: &mut Client<T>,
    info: &DeviceInfo,
    app: &AppPayload,
    progress: Progress<'_>,
) -> Result<(), Error> {
    need_dfu(info, app.header.board_rev.as_str())?;
    if app.data.len() > APP_LEN as usize {
        return Err(Error::Check("the firmware does not fit its region".into()));
    }
    // The commit word is the device's to program: compare after it.
    program(
        c,
        APP_BASE,
        APP_LEN,
        &app.data,
        OFF_COMMIT + 4,
        "the firmware",
        progress,
    )
    .await?;
    progress("committing the firmware", 0, 1);
    c.commit(TARGET_APPLICATION).await?;
    progress("committing the firmware", 1, 1);
    Ok(())
}

/// The fabric: erase the manifest, then the bitstream's region, write and
/// read back the bitstream, write and read back the manifest page, then
/// `COMMIT 1`. `info` is the DFU stage's.
///
/// # Errors
///
/// A device or board that is not the payload's, or as the device answers.
pub async fn update_fabric<T: Transport>(
    c: &mut Client<T>,
    info: &DeviceInfo,
    fabric: &FabricPayload,
    progress: Progress<'_>,
) -> Result<(), Error> {
    need_dfu(info, fabric.manifest.board_rev.as_str())?;
    if fabric.data.len() > FABRIC_LEN as usize {
        return Err(Error::Check("the bitstream does not fit its region".into()));
    }
    progress("erasing the fabric manifest", 0, 1);
    c.erase(MANIFEST_BASE, MANIFEST_LEN).await?;
    progress("erasing the fabric manifest", 1, 1);
    program(
        c,
        FABRIC_BASE,
        FABRIC_LEN,
        &fabric.data,
        0,
        "the fabric",
        progress,
    )
    .await?;
    program(
        c,
        MANIFEST_BASE,
        0,
        &fabric.page,
        OFF_COMMIT + 4,
        "the fabric manifest",
        progress,
    )
    .await?;
    progress("committing the fabric", 0, 1);
    c.commit(TARGET_FABRIC).await?;
    progress("committing the fabric", 1, 1);
    Ok(())
}
